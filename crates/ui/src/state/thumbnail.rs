//! Texture-array thumbnail cache and the background BC7 load helpers shared
//! by the photo grid, inspector, and viewer.
//!
//! The cache holds no decoded images. Each resident photo owns one layer of a
//! fixed 512-layer `Texture2DArray`; when all layers are in use the least
//! recently displayed, unpinned photo is evicted. Re-displaying an evicted
//! photo reads its `.bc7` file again.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{RenderImage, TextureArray};
use smallvec::SmallVec;
use vrc_photon_core::thumbnail::{self, ThumbnailSpec};

/// Layers in the single texture array. The array never grows or shrinks.
pub(crate) const THUMB_LAYERS: usize = 512;

/// Decode and generation share one slot: a thumbnail is the only display
/// derivative, so serialising them keeps the UI decode peak bounded.
const THUMB_IN_FLIGHT: usize = 1;
/// A failed decode is retried after this delay, so a transient source or IO
/// error does not hide the thumbnail until the next cache clear.
const FAILED_RETRY_AFTER: Duration = Duration::from_secs(30);

/// A flush is due once a batch reaches either bound. Small batches are flushed
/// by a timer instead so a quiet gallery still reports its generated files.
pub(crate) const THUMBNAIL_NOTICE_BATCH_FILES: u64 = 32;
pub(crate) const THUMBNAIL_NOTICE_BATCH_BYTES: u64 = 1024 * 1024;

/// A thumbnail resident in the texture array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThumbnailHandle {
    pub(crate) array: TextureArray,
    pub(crate) layer: u32,
    pub(crate) rotated: bool,
}

/// One live layer.
#[derive(Clone, Copy, Debug)]
struct LayerEntry {
    rotated: bool,
}

/// A decode request token. `generation` is bumped when the whole cache is
/// cleared; `photo_generation` is bumped when one photo is invalidated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodeTicket {
    generation: u64,
    photo_generation: u64,
}

/// A generated or loaded BC7 layer waiting for the UI thread to upload it.
#[derive(Debug)]
pub(crate) struct PendingUpload {
    pub(crate) id: i64,
    pub(crate) ticket: DecodeTicket,
    pub(crate) layer: u32,
    pub(crate) blocks: Vec<u8>,
    pub(crate) color: u32,
    pub(crate) rotated: bool,
    pub(crate) generated_bytes: Option<u64>,
}

/// Accumulates thumbnails the UI generated on its own before telling the
/// daemon about them. One notice per thumbnail would double the IPC traffic,
/// so the caller flushes on a threshold or a short timer.
#[derive(Debug, Default)]
pub(crate) struct ThumbnailNoticeBatch {
    files: u64,
    bytes: u64,
}

impl ThumbnailNoticeBatch {
    /// Records one generated file and returns whether the batch is now full.
    pub(crate) fn add(&mut self, bytes: u64) -> bool {
        self.files += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        self.files >= THUMBNAIL_NOTICE_BATCH_FILES || self.bytes >= THUMBNAIL_NOTICE_BATCH_BYTES
    }

    /// Returns the accumulated `(files, bytes)` and clears the batch.
    pub(crate) fn take(&mut self) -> Option<(u64, u64)> {
        if self.files == 0 {
            return None;
        }
        let notice = (self.files, self.bytes);
        self.files = 0;
        self.bytes = 0;
        Some(notice)
    }
}

/// The resident thumbnails and their pending decode / upload state.
#[derive(Default)]
pub(crate) struct ThumbnailCache {
    array: Option<TextureArray>,
    removed_array: Option<TextureArray>,
    layers: Vec<Option<LayerEntry>>,
    free_layers: Vec<u32>,
    by_photo: HashMap<i64, u32>,
    /// Average colour survives an eviction so the placeholder does not flash.
    colors: HashMap<i64, u32>,
    /// Monotonic access tick per resident id.
    recency: HashMap<i64, u64>,
    clock: u64,
    pending: HashMap<i64, DecodeTicket>,
    failed: HashMap<i64, Instant>,
    pinned: HashMap<i64, u32>,
    in_flight: usize,
    uploads: VecDeque<PendingUpload>,
    /// Bumped when every entry is discarded (thumbnail settings changed or the
    /// daemon cleared the cache).
    generation: u64,
    /// Per-photo invalidation epochs used by replace compression.
    photo_generations: HashMap<i64, u64>,
}

impl ThumbnailCache {
    /// Installs the array created for the current spec.
    pub(crate) fn set_array(&mut self, array: TextureArray) {
        if self.array.is_some_and(|current| current == array) {
            return;
        }
        if let Some(old) = self.array.replace(array) {
            self.removed_array = Some(old);
        }
        self.array = Some(array);
        self.reset_layers();
    }

    pub(crate) fn array(&self) -> Option<TextureArray> {
        self.array
    }

    /// Takes the array the cache dropped, so the UI can release its GPU memory.
    pub(crate) fn take_removed_array(&mut self) -> Option<TextureArray> {
        self.removed_array.take()
    }

    pub(crate) fn get(&mut self, id: i64) -> Option<ThumbnailHandle> {
        let array = self.array?;
        let layer = *self.by_photo.get(&id)?;
        let entry = self.layers.get(layer as usize).copied().flatten()?;
        self.touch(id);
        Some(ThumbnailHandle {
            array,
            layer,
            rotated: entry.rotated,
        })
    }

    pub(crate) fn color(&self, id: i64) -> Option<u32> {
        self.colors.get(&id).copied()
    }

    /// How many layers are currently resident. Test/diagnostic helper.
    #[cfg(test)]
    pub(crate) fn resident_layers(&self) -> usize {
        self.by_photo.len()
    }

    /// Reserves an in-flight slot and returns the token the decode must present
    /// when it finishes.
    pub(crate) fn begin_ticket(&mut self, id: i64) -> Option<DecodeTicket> {
        let retry_ready = self
            .failed
            .get(&id)
            .is_some_and(|failed_at| failed_at.elapsed() >= FAILED_RETRY_AFTER);
        if retry_ready {
            self.failed.remove(&id);
        }
        if self.by_photo.contains_key(&id)
            || self.pending.contains_key(&id)
            || self.failed.contains_key(&id)
            || self.in_flight >= THUMB_IN_FLIGHT
        {
            return None;
        }
        let ticket = DecodeTicket {
            generation: self.generation,
            photo_generation: self.photo_generation(id),
        };
        self.pending.insert(id, ticket);
        self.in_flight += 1;
        Some(ticket)
    }

    /// Queues a loaded layer for the next render's upload.
    pub(crate) fn queue_upload(&mut self, mut upload: PendingUpload) -> bool {
        if self.pending.get(&upload.id) != Some(&upload.ticket) || self.array.is_none() {
            return false;
        }
        let Some(layer) = self.allocate_layer() else {
            return false;
        };
        upload.layer = layer;
        self.uploads.push_back(upload);
        true
    }

    /// Takes the next layer the UI thread should upload.
    pub(crate) fn take_upload(&mut self) -> Option<PendingUpload> {
        self.uploads.pop_front()
    }

    /// Records a successfully uploaded layer. Returns false when the ticket is
    /// stale, so the caller drops the upload instead of repopulating the cache.
    pub(crate) fn insert_uploaded(
        &mut self,
        id: i64,
        ticket: DecodeTicket,
        layer: u32,
        color: u32,
        rotated: bool,
    ) -> bool {
        if self.pending.get(&id) != Some(&ticket)
            || ticket.generation != self.generation
            || ticket.photo_generation != self.photo_generation(id)
            || layer as usize >= self.layers.len()
        {
            self.free_layer_slot(layer);
            return false;
        }
        self.pending.remove(&id);
        self.in_flight = self.in_flight.saturating_sub(1);
        if let Some(old) = self.by_photo.insert(id, layer) {
            self.layers[old as usize] = None;
            self.free_layer_slot(old);
        }
        self.layers[layer as usize] = Some(LayerEntry { rotated });
        self.colors.insert(id, color);
        self.touch(id);
        true
    }

    /// Clears an explicit retry marker, for example when the user clicks the
    /// photo again.
    pub(crate) fn retry(&mut self, id: i64) {
        self.failed.remove(&id);
    }

    /// Drops one photo's layer after its display source changed. The pin is
    /// kept so the selected card is re-requested.
    pub(crate) fn invalidate(&mut self, id: i64) {
        let generation = self.photo_generation(id).wrapping_add(1);
        self.photo_generations.insert(id, generation);
        self.release_layer(id);
        self.pending.remove(&id);
        self.failed.remove(&id);
    }

    /// Drops every trace of a deleted photo.
    pub(crate) fn remove_photo(&mut self, id: i64) {
        self.release_layer(id);
        self.colors.remove(&id);
        self.pending.remove(&id);
        self.failed.remove(&id);
        self.photo_generations.remove(&id);
        self.pinned.remove(&id);
    }

    /// Discards every layer, upload, and retry marker, and drops the array.
    pub(crate) fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(old) = self.array.take() {
            self.removed_array = Some(old);
        }
        self.reset_layers();
        self.colors.clear();
        self.pending.clear();
        self.failed.clear();
        self.uploads.clear();
        self.in_flight = 0;
        self.pinned.clear();
    }

    pub(crate) fn fail_if_ticket(&mut self, ticket: DecodeTicket, id: i64) {
        if self.pending.get(&id) != Some(&ticket) {
            return;
        }
        if ticket.generation != self.generation
            || ticket.photo_generation != self.photo_generation(id)
        {
            self.pending.remove(&id);
            self.in_flight = self.in_flight.saturating_sub(1);
            return;
        }
        self.pending.remove(&id);
        self.in_flight = self.in_flight.saturating_sub(1);
        self.failed.insert(id, Instant::now());
    }

    pub(crate) fn pending_for(&self, id: i64) -> bool {
        self.pending.contains_key(&id)
    }

    pub(crate) fn pin(&mut self, id: i64) {
        *self.pinned.entry(id).or_insert(0) += 1;
    }

    pub(crate) fn unpin(&mut self, id: i64) {
        if let Some(count) = self.pinned.get_mut(&id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.pinned.remove(&id);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn is_pinned(&self, id: i64) -> bool {
        self.pinned.contains_key(&id)
    }

    pub(crate) fn pinned_ids(&self) -> Vec<i64> {
        self.pinned.keys().copied().collect()
    }

    fn reset_layers(&mut self) {
        self.layers = vec![None; THUMB_LAYERS];
        self.free_layers = (0..THUMB_LAYERS as u32).rev().collect();
        self.by_photo.clear();
        self.recency.clear();
        self.clock = 0;
        self.uploads.clear();
        self.pending.clear();
        self.in_flight = 0;
    }

    fn allocate_layer(&mut self) -> Option<u32> {
        if let Some(layer) = self.free_layers.pop() {
            return Some(layer);
        }
        let (&victim, _) = self
            .recency
            .iter()
            .filter(|(id, _)| !self.pinned.contains_key(id))
            .min_by_key(|(_, tick)| **tick)?;
        self.release_layer(victim);
        self.free_layers.pop()
    }

    /// Returns a layer reserved for an upload that never landed.
    pub(crate) fn free_layer_slot(&mut self, layer: u32) {
        if layer as usize >= self.layers.len() {
            return;
        }
        if self.layers[layer as usize].is_none() && !self.free_layers.contains(&layer) {
            self.free_layers.push(layer);
        }
    }

    fn release_layer(&mut self, id: i64) {
        let Some(layer) = self.by_photo.remove(&id) else {
            return;
        };
        if let Some(entry) = self.layers.get_mut(layer as usize) {
            *entry = None;
        }
        self.recency.remove(&id);
        self.free_layer_slot(layer);
    }

    fn photo_generation(&self, id: i64) -> u64 {
        self.photo_generations.get(&id).copied().unwrap_or(0)
    }

    fn touch(&mut self, id: i64) {
        self.clock = self.clock.wrapping_add(1);
        self.recency.insert(id, self.clock);
    }
}

/// A thumbnail loaded on a background thread.
#[derive(Debug)]
pub(crate) struct DecodedThumbnail {
    /// Average colour in RGB.
    pub(crate) color: u32,
    /// Raw BC7 blocks for the texture array layer.
    pub(crate) blocks: Vec<u8>,
    /// On-disk size when this load wrote the file; `None` for a cache hit.
    pub(crate) generated_bytes: Option<u64>,
    /// Whether the canvas was rotated when the source was portrait.
    pub(crate) rotated: bool,
}

/// Loads or generates a thumbnail on a background thread.
///
/// With `cache_dir`, the core resolves or regenerates the file. Without it,
/// `cache_path` is read-only (demo mode or an explicit asset path).
pub(crate) fn decode_thumbnail(
    id: i64,
    cache_path: &Path,
    source: &Path,
    cache_dir: Option<&Path>,
    spec: ThumbnailSpec,
) -> Option<DecodedThumbnail> {
    let Some(cache_dir) = cache_dir else {
        let blocks = thumbnail::read_thumbnail_blocks(cache_path, spec).ok()?;
        let (color, rotated) =
            thumbnail::read_thumbnail_color_rotation(cache_path).unwrap_or((0, false));
        return Some(DecodedThumbnail {
            color,
            blocks,
            generated_bytes: None,
            rotated,
        });
    };

    let ensured = match thumbnail::ensure_thumbnail_in_with_status(id, source, cache_dir, spec) {
        Ok(Some(ensured)) => ensured,
        Ok(None) => {
            tracing::debug!(
                photo_id = id,
                path = %source.display(),
                "thumbnail source is missing or cannot be decoded"
            );
            return None;
        }
        Err(error) => {
            tracing::warn!(
                %error,
                photo_id = id,
                path = %source.display(),
                "cannot generate thumbnail"
            );
            return None;
        }
    };
    let generated_bytes = ensured.created.then_some(ensured.bytes);
    let blocks = match ensured.blocks {
        Some(blocks) => blocks,
        None => match thumbnail::read_thumbnail_blocks(&ensured.path, spec) {
            Ok(blocks) => blocks,
            Err(error) => {
                tracing::warn!(
                    %error,
                    photo_id = id,
                    path = %ensured.path.display(),
                    "cannot read thumbnail cache"
                );
                return None;
            }
        },
    };
    Some(DecodedThumbnail {
        color: ensured.color,
        blocks,
        generated_bytes,
        rotated: ensured.rotated,
    })
}

/// Builds a GPUI image from a decoded full-image buffer. Must run on the UI
/// thread. Thumbnails use the texture array instead.
pub(crate) fn make_render_image(rgba: image::RgbaImage) -> Arc<RenderImage> {
    let frame = image::Frame::new(rgba);
    Arc::new(RenderImage::new(SmallVec::from_buf([frame])))
}

/// Swaps RGBA to the BGRA order GPUI's `RenderImage` expects. Call this on the
/// background executor so a full image does not spend its pixel loop on the
/// UI thread.
pub(crate) fn rgba_to_bgra(mut rgba: image::RgbaImage) -> image::RgbaImage {
    for pixel in rgba.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{DevicePixels, size};

    fn array() -> TextureArray {
        // The unit tests only need a handle; the platform API is exercised by
        // the fork and the app's smoke test.
        TextureArray::new_for_test(size(DevicePixels(384), DevicePixels(216)), 512)
    }

    fn ticket(cache: &mut ThumbnailCache, id: i64) -> DecodeTicket {
        cache.begin_ticket(id).expect("decode slot")
    }

    fn upload(cache: &mut ThumbnailCache, id: i64, ticket: DecodeTicket, color: u32) -> bool {
        let layer = cache.allocate_layer().expect("layer");
        cache.insert_uploaded(id, ticket, layer, color, false)
    }

    fn fill(cache: &mut ThumbnailCache, count: usize) {
        for id in 0..count as i64 {
            let ticket = ticket(cache, id);
            assert!(upload(cache, id, ticket, id as u32));
        }
    }

    #[test]
    fn allocates_up_to_512_layers_and_evicts_the_oldest() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        fill(&mut cache, THUMB_LAYERS);
        assert_eq!(cache.resident_layers(), THUMB_LAYERS);
        assert!(cache.get(0).is_some());

        let ticket = ticket(&mut cache, 999);
        assert!(upload(&mut cache, 999, ticket, 0));
        assert_eq!(cache.resident_layers(), THUMB_LAYERS);
        assert!(cache.get(1).is_none());
        assert!(cache.get(0).is_some());
        assert!(cache.get(999).is_some());
    }

    #[test]
    fn pinned_layers_are_never_evicted() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        cache.pin(0);
        fill(&mut cache, THUMB_LAYERS);

        let ticket = ticket(&mut cache, 999);
        assert!(upload(&mut cache, 999, ticket, 0));
        assert!(cache.get(0).is_some());
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn all_pinned_layers_refuse_a_new_upload() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        for id in 0..THUMB_LAYERS as i64 {
            cache.pin(id);
            let ticket = ticket(&mut cache, id);
            assert!(upload(&mut cache, id, ticket, 0));
        }
        let ticket = ticket(&mut cache, 99_999);
        let upload = PendingUpload {
            id: 99_999,
            ticket,
            layer: 0,
            blocks: vec![0; 384 * 216],
            color: 0,
            rotated: false,
            generated_bytes: None,
        };
        assert!(!cache.queue_upload(upload));
    }

    #[test]
    fn invalidation_releases_the_layer_and_rejects_the_old_ticket() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        let old = ticket(&mut cache, 7);
        assert!(upload(&mut cache, 7, old, 0x112233));
        cache.pin(7);
        cache.invalidate(7);
        assert!(cache.get(7).is_none());
        assert!(cache.is_pinned(7));
        let new = ticket(&mut cache, 7);
        assert!(!upload(&mut cache, 7, old, 0x112233));
        assert!(upload(&mut cache, 7, new, 0x445566));
        assert!(cache.get(7).is_some());
    }

    #[test]
    fn clear_drops_the_array_and_all_layers() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        fill(&mut cache, 2);
        cache.clear();
        assert!(cache.array().is_none());
        assert!(cache.get(0).is_none());
        assert_eq!(cache.resident_layers(), 0);
    }

    #[test]
    fn a_stale_upload_is_rejected_after_clear() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        let ticket = ticket(&mut cache, 1);
        cache.clear();
        assert!(!upload(&mut cache, 1, ticket, 0));
    }

    #[test]
    fn failed_thumbnail_is_retried_after_the_ttl() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        let ticket = ticket(&mut cache, 1);
        cache.fail_if_ticket(ticket, 1);
        assert!(cache.begin_ticket(1).is_none());
        cache.failed.insert(
            1,
            Instant::now() - FAILED_RETRY_AFTER - Duration::from_secs(1),
        );
        assert!(cache.begin_ticket(1).is_some());
    }

    #[test]
    fn explicit_retry_clears_a_recent_failure() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        let ticket = ticket(&mut cache, 1);
        cache.fail_if_ticket(ticket, 1);
        cache.retry(1);
        assert!(cache.begin_ticket(1).is_some());
    }

    #[test]
    fn remove_photo_releases_the_layer_and_colour() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        let ticket = ticket(&mut cache, 1);
        assert!(upload(&mut cache, 1, ticket, 0x112233));
        assert_eq!(cache.color(1), Some(0x112233));
        cache.remove_photo(1);
        assert!(cache.get(1).is_none());
        assert_eq!(cache.color(1), None);
    }

    #[test]
    fn upload_without_an_array_is_rejected() {
        let mut cache = ThumbnailCache::default();
        let ticket = ticket(&mut cache, 1);
        assert!(!cache.insert_uploaded(1, ticket, 0, 0, false));
    }

    #[test]
    fn thumbnail_color_survives_eviction() {
        let mut cache = ThumbnailCache::default();
        cache.set_array(array());
        fill(&mut cache, THUMB_LAYERS + 1);
        assert!(cache.get(0).is_none());
        assert!(cache.color(0).is_some());
    }

    #[test]
    fn make_render_image_moves_the_buffer_without_changing_its_bytes() {
        let rgba = image::RgbaImage::from_pixel(1, 1, image::Rgba([30, 20, 10, 40]));
        let image = make_render_image(rgba);
        assert_eq!(image.frame_count(), 1);
        assert_eq!(image.as_bytes(0), Some([30, 20, 10, 40].as_slice()));
    }

    #[test]
    fn notice_batch_flushes_at_file_and_byte_thresholds() {
        let mut batch = ThumbnailNoticeBatch::default();
        for _ in 0..(THUMBNAIL_NOTICE_BATCH_FILES - 1) {
            assert!(!batch.add(1));
        }
        assert!(batch.add(1));
        assert_eq!(
            batch.take(),
            Some((THUMBNAIL_NOTICE_BATCH_FILES, THUMBNAIL_NOTICE_BATCH_FILES))
        );
        assert_eq!(batch.take(), None);
    }
}
