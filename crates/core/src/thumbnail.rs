use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use image::RgbaImage;

use crate::cache_metadata;
use crate::error::{CoreError, Result};
use crate::fs_safe::{PlaceOutcome, write_bytes_no_clobber};

pub const THUMBNAIL_SIZES: [u32; 3] = [256, 384, 512];
pub const THUMBNAIL_SIZE_DEFAULT: u32 = 384;
pub const ACCESS_REFRESH_AFTER: Duration = Duration::from_secs(60 * 60);

/// The fixed 16:9 canvas of every thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailSpec {
    /// Canvas width in pixels. The height is always width × 9 / 16.
    pub size: u32,
}

impl ThumbnailSpec {
    pub fn new(size: u32) -> Self {
        Self {
            size: normalize_thumbnail_size(size),
        }
    }

    /// The canvas dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.size, self.size * 9 / 16)
    }
}

impl Default for ThumbnailSpec {
    fn default() -> Self {
        Self {
            size: THUMBNAIL_SIZE_DEFAULT,
        }
    }
}

fn normalize_thumbnail_size(size: u32) -> u32 {
    THUMBNAIL_SIZES
        .into_iter()
        .find(|candidate| *candidate == size)
        .unwrap_or(THUMBNAIL_SIZE_DEFAULT)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThumbnailCacheUsage {
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThumbnailCacheClearReport {
    pub removed_files: usize,
    pub removed_bytes: u64,
    pub remaining_files: usize,
    pub remaining_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThumbnailPruneReport {
    pub scanned_files: usize,
    pub scanned_bytes: u64,
    pub removed_files: usize,
    pub removed_bytes: u64,
    pub remaining_files: usize,
    pub remaining_bytes: u64,
}

/// A thumbnail returned by [`ensure_thumbnail_in_with_status`].
///
/// `created` is true when this call wrote the file (a new thumbnail or a
/// regeneration); false when an existing non-empty file was reused. `bytes`
/// is the on-disk size after the call. `blocks` carries the raw BC7 layer only
/// when this call generated the file, so the UI can upload it without reading
/// the file back. `color` and `rotated` are available on every cache hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsuredThumbnail {
    pub path: PathBuf,
    pub created: bool,
    pub bytes: u64,
    pub blocks: Option<Vec<u8>>,
    pub color: u32,
    pub rotated: bool,
}

struct ThumbnailEntry {
    modified: SystemTime,
    bytes: u64,
    path: PathBuf,
}

pub fn thumbnails_dir() -> Option<PathBuf> {
    crate::data_dir().map(|dir| dir.join("thumbnails"))
}

pub fn thumbnail_path(photo_id: i64, spec: ThumbnailSpec) -> Option<PathBuf> {
    thumbnails_dir().map(|dir| thumbnail_path_in(&dir, photo_id, spec))
}

pub fn thumbnail_path_in(cache_dir: &Path, photo_id: i64, spec: ThumbnailSpec) -> PathBuf {
    let spec = ThumbnailSpec::new(spec.size);
    let (width, height) = spec.dimensions();
    cache_dir.join(format!("{photo_id}-s{width}x{height}.bc7"))
}

/// Total size of the default thumbnail cache.
pub fn thumbnail_cache_usage() -> Result<ThumbnailCacheUsage> {
    match thumbnails_dir() {
        Some(dir) => thumbnail_cache_usage_in(&dir),
        None => Ok(ThumbnailCacheUsage::default()),
    }
}

/// Total size of the thumbnail cache, counting old and new cache keys.
///
/// `.meta` sidecars are excluded from the count (their bytes are negligible);
/// they are still deleted together with their WebP by pruning, clearing, and
/// per-photo invalidation so they cannot accumulate as orphans.
pub fn thumbnail_cache_usage_in(cache_dir: &Path) -> Result<ThumbnailCacheUsage> {
    let entries = match collect_thumbnail_entries(cache_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ThumbnailCacheUsage::default());
        }
        Err(error) => return Err(CoreError::Io(error)),
    };
    Ok(ThumbnailCacheUsage {
        files: entries.len(),
        bytes: entries
            .iter()
            .fold(0u64, |total, entry| total.saturating_add(entry.bytes)),
    })
}

/// Removes the least recently modified thumbnails until the cache fits in
/// `max_bytes`. A limit of zero means unlimited.
pub fn prune_thumbnails_in(cache_dir: &Path, max_bytes: u64) -> Result<ThumbnailPruneReport> {
    prune_thumbnails_with(cache_dir, max_bytes, |path| fs::remove_file(path))
}

/// Removes thumbnails while allowing callers (tests) to inject removal
/// failures. Failed removals are logged and skipped; successful ones count
/// toward the report.
fn prune_thumbnails_with<Remove>(
    cache_dir: &Path,
    max_bytes: u64,
    mut remove: Remove,
) -> Result<ThumbnailPruneReport>
where
    Remove: FnMut(&Path) -> std::io::Result<()>,
{
    let mut entries = match collect_thumbnail_entries(cache_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ThumbnailPruneReport::default());
        }
        Err(error) => return Err(CoreError::Io(error)),
    };
    let scanned_files = entries.len();
    let scanned_bytes = entries
        .iter()
        .fold(0u64, |total, entry| total.saturating_add(entry.bytes));
    let mut report = ThumbnailPruneReport {
        scanned_files,
        scanned_bytes,
        remaining_files: scanned_files,
        remaining_bytes: scanned_bytes,
        ..ThumbnailPruneReport::default()
    };
    if max_bytes == 0 || scanned_bytes <= max_bytes {
        return Ok(report);
    }

    entries.sort_by(|left, right| {
        left.modified
            .cmp(&right.modified)
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut remaining = scanned_bytes;
    for entry in entries {
        if remaining <= max_bytes {
            break;
        }
        match remove(&entry.path) {
            Ok(()) => {
                report.removed_files += 1;
                report.removed_bytes = report.removed_bytes.saturating_add(entry.bytes);
                remaining = remaining.saturating_sub(entry.bytes);
                // Sidecars are tiny and deliberately excluded from the byte
                // accounting, but they must never outlive their thumbnail.
                cache_metadata::remove_stamp(&entry.path);
            }
            Err(error) => tracing::debug!(
                %error,
                path = %entry.path.display(),
                "cannot remove thumbnail cache entry"
            ),
        }
    }
    report.remaining_files = scanned_files.saturating_sub(report.removed_files);
    report.remaining_bytes = remaining;
    Ok(report)
}

/// Removes every known thumbnail artifact in the cache: WebP entries (old and
/// current keys), their `.meta` sidecars, and temp files left by interrupted
/// writers.
///
/// Unrelated regular files and subdirectories are left alone, so clearing the
/// thumbnail cache can never delete a user file that happens to live beside
/// it.
pub fn clear_thumbnail_cache_in(cache_dir: &Path) -> Result<ThumbnailCacheClearReport> {
    let mut report = ThumbnailCacheClearReport::default();
    let entries = match fs::read_dir(cache_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(CoreError::Io(error)),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(
                    %error,
                    dir = %cache_dir.display(),
                    "cannot read thumbnail cache entry while clearing"
                );
                continue;
            }
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_thumbnail_artifact_name(&name) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => {
                report.removed_files += 1;
                report.removed_bytes = report.removed_bytes.saturating_add(metadata.len());
            }
            Err(error) => tracing::debug!(
                %error,
                path = %path.display(),
                "cannot remove thumbnail cache entry while clearing"
            ),
        }
    }
    let remaining = thumbnail_cache_usage_in(cache_dir)?;
    report.remaining_files = remaining.files;
    report.remaining_bytes = remaining.bytes;
    Ok(report)
}

/// Removes legacy WebP thumbnails and their sidecars.
///
/// The BC7 cache has no migration path, so a one-time startup pass can discard
/// files left by older versions. Unrelated files are left alone.
pub fn remove_legacy_webp_entries(cache_dir: &Path) -> Result<usize> {
    let entries = match fs::read_dir(cache_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(CoreError::Io(error)),
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.contains(".webp") || !is_thumbnail_artifact_name(&name) {
            continue;
        }
        if entry.metadata().is_ok_and(|metadata| metadata.is_file())
            && fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Removes every cached thumbnail and sidecar for one photo, regardless of
/// quality/size variant or temp files left by a failed writer.
///
/// Returns how many files were removed. Used after a `replace` compression so
/// the next display regenerates from the new derivative instead of reusing a
/// thumbnail of the original.
pub fn remove_thumbnail_files(photo_id: i64) -> Result<usize> {
    match thumbnails_dir() {
        Some(dir) => remove_thumbnail_files_in(&dir, photo_id),
        None => Ok(0),
    }
}

/// Same as [`remove_thumbnail_files`] with an explicit cache directory.
pub fn remove_thumbnail_files_in(cache_dir: &Path, photo_id: i64) -> Result<usize> {
    let entries = match fs::read_dir(cache_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(CoreError::Io(error)),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(
                    %error,
                    dir = %cache_dir.display(),
                    "cannot read thumbnail cache entry while removing a photo"
                );
                continue;
            }
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !thumbnail_artifact_belongs_to(&name, photo_id) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(error) => tracing::debug!(
                %error,
                path = %path.display(),
                "cannot remove thumbnail for photo"
            ),
        }
    }
    Ok(removed)
}

/// Whether `name` is a thumbnail file, sidecar, or temp file.
fn is_thumbnail_artifact_name(name: &str) -> bool {
    thumbnail_artifact_stem(name).is_some_and(is_thumbnail_stem)
}

fn thumbnail_artifact_belongs_to(name: &str, photo_id: i64) -> bool {
    let Some(stem) = thumbnail_artifact_stem(name) else {
        return false;
    };
    stem.split('-').next().and_then(|id| id.parse::<i64>().ok()) == Some(photo_id)
        && is_thumbnail_stem(stem)
}

/// Returns the cache key stem before a known thumbnail suffix.
///
/// Both the current `.bc7` files and the legacy `.webp` files are recognised,
/// so clearing the cache removes an installation's old thumbnails.
fn thumbnail_artifact_stem(name: &str) -> Option<&str> {
    for extension in [".bc7", ".webp"] {
        if let Some(stem) = name.strip_suffix(extension) {
            return Some(stem);
        }
        if let Some(stem) = name.strip_suffix(&format!("{extension}.meta")) {
            return Some(stem);
        }
        if let Some((stem, suffix)) = name.split_once(&format!("{extension}.meta.tmp")) {
            return is_temp_suffix(suffix).then_some(stem);
        }
        if let Some((stem, suffix)) = name.split_once(&format!("{extension}.tmp")) {
            return is_temp_suffix(suffix).then_some(stem);
        }
    }
    None
}

/// A temp suffix is empty (legacy shared name) or `-<pid>-<sequence>`.
fn is_temp_suffix(suffix: &str) -> bool {
    let Some(rest) = suffix.strip_prefix('-') else {
        return suffix.is_empty();
    };
    let mut parts = rest.split('-');
    let Some(pid) = parts.next() else {
        return false;
    };
    let Some(sequence) = parts.next() else {
        return false;
    };
    parts.next().is_none() && pid.parse::<u32>().is_ok() && sequence.parse::<u64>().is_ok()
}

/// Accepts the current `<id>-s<width>x<height>` key, the legacy
/// `<id>-q<quality>-s<size>` key, and the original `<id>` key.
fn is_thumbnail_stem(stem: &str) -> bool {
    let mut parts = stem.split('-');
    let Some(id) = parts.next() else {
        return false;
    };
    if id.parse::<u64>().is_err() {
        return false;
    }
    let Some(first) = parts.next() else {
        // The original key has no variant suffix.
        return true;
    };
    if first.starts_with('s') && first.contains('x') {
        if parts.next().is_some() {
            return false;
        }
        let Some((width, height)) = first[1..].split_once('x') else {
            return false;
        };
        return width
            .parse::<u32>()
            .ok()
            .is_some_and(|width| width > 0 && width % 4 == 0)
            && height
                .parse::<u32>()
                .ok()
                .is_some_and(|height| height > 0 && height % 4 == 0);
    }
    let Some(size) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    first
        .strip_prefix('q')
        .and_then(|value| value.parse::<u32>().ok())
        .is_some_and(|value| (1..=100).contains(&value))
        && size
            .strip_prefix('s')
            .and_then(|value| value.parse::<u32>().ok())
            .is_some_and(|value| value > 0)
}

fn collect_thumbnail_entries(cache_dir: &Path) -> std::io::Result<Vec<ThumbnailEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(cache_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(
                    %error,
                    dir = %cache_dir.display(),
                    "cannot read thumbnail cache entry"
                );
                continue;
            }
        };
        let path = entry.path();
        if !matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("bc7" | "webp")
        ) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !is_thumbnail_stem(stem) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        entries.push(ThumbnailEntry {
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            bytes: metadata.len(),
            path,
        });
    }
    Ok(entries)
}

/// Marks a cached thumbnail as recently used by refreshing its mtime.
///
/// Display paths call this on a cache hit. The daemon deliberately does not:
/// its startup pass over every photo would otherwise promote the whole cache.
pub fn refresh_thumbnail_access(path: &Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    let Ok(modified) = metadata.modified() else {
        return;
    };
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return;
    };
    if age < ACCESS_REFRESH_AFTER {
        return;
    }
    match fs::OpenOptions::new().write(true).open(path) {
        Ok(file) => {
            if let Err(error) = file.set_modified(SystemTime::now()) {
                tracing::debug!(
                    %error,
                    path = %path.display(),
                    "cannot refresh thumbnail access time"
                );
            }
        }
        Err(error) => tracing::debug!(
            %error,
            path = %path.display(),
            "cannot open thumbnail to refresh access time"
        ),
    }
}

/// Creates a BC7 thumbnail with the given spec if it does not exist yet.
///
/// The file is written atomically. Returns `None` when the source is missing
/// or cannot be decoded, so callers can keep a placeholder.
pub fn ensure_thumbnail(
    photo_id: i64,
    source: &Path,
    spec: ThumbnailSpec,
) -> Result<Option<PathBuf>> {
    let Some(cache_dir) = thumbnails_dir() else {
        return Ok(None);
    };
    ensure_thumbnail_in(photo_id, source, &cache_dir, spec)
}

/// Same as [`ensure_thumbnail`] but with an explicit cache directory. Used by
/// tests and by callers that keep the cache outside the user data directory.
pub fn ensure_thumbnail_in(
    photo_id: i64,
    source: &Path,
    cache_dir: &Path,
    spec: ThumbnailSpec,
) -> Result<Option<PathBuf>> {
    ensure_thumbnail_in_with_status(photo_id, source, cache_dir, spec)
        .map(|ensured| ensured.map(|ensured| ensured.path))
}

/// Same as [`ensure_thumbnail_in`] but also reports whether the file was
/// written, its on-disk size, and the metadata the display needs.
pub fn ensure_thumbnail_in_with_status(
    photo_id: i64,
    source: &Path,
    cache_dir: &Path,
    spec: ThumbnailSpec,
) -> Result<Option<EnsuredThumbnail>> {
    let spec = ThumbnailSpec::new(spec.size);
    let output = thumbnail_path_in(cache_dir, photo_id, spec);
    let (width, height) = spec.dimensions();
    let expected_bytes = crate::bc7::encoded_len(width, height);
    let existing_bytes = existing_thumbnail_bytes(&output);
    if let Some(bytes) = existing_bytes
        && expected_bytes == Some(bytes as usize)
        && cache_metadata::cache_is_fresh(&output, source)
        && let Some((color, rotated)) = cache_metadata::read_color_rotation(&output)
    {
        return Ok(Some(EnsuredThumbnail {
            path: output,
            created: false,
            bytes,
            blocks: None,
            color,
            rotated,
        }));
    }
    if !source.is_file() {
        return Ok(None);
    }
    let Some(stamp) = cache_metadata::source_stamp(source) else {
        tracing::warn!(path = %source.display(), "cannot read thumbnail source metadata");
        return Ok(None);
    };
    let Some((canvas, rotated)) =
        crate::image_decode::decode_thumbnail_canvas(source, width, height)
    else {
        tracing::warn!(path = %source.display(), "cannot decode thumbnail source");
        return Ok(None);
    };
    let color = average_color(&canvas);
    let blocks = crate::bc7::encode_rgba(width, height, canvas.as_raw())?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    // The pre-check only returns early for a fresh cache entry, so an
    // existing file here is stale. Remove it before the no-clobber publish;
    // a racing writer may place a new file after this point, which the
    // DestinationExists branch below adopts.
    if existing_bytes.is_some() {
        match fs::remove_file(&output) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                // Windows can deny removing a file another worker still has
                // open. Keep a non-empty existing thumbnail as a cache hit
                // instead of failing this request.
                tracing::debug!(
                    %error,
                    photo_id,
                    path = %output.display(),
                    "cannot replace a stale thumbnail; keeping the existing file"
                );
                if let Some(bytes) = existing_thumbnail_bytes(&output) {
                    let (color, rotated) =
                        cache_metadata::read_color_rotation(&output).unwrap_or((0, false));
                    return Ok(Some(EnsuredThumbnail {
                        path: output,
                        created: false,
                        bytes,
                        blocks: None,
                        color,
                        rotated,
                    }));
                }
                return Err(CoreError::Io(error));
            }
        }
    }
    match publish_thumbnail_bytes(&output, &blocks)? {
        PlaceOutcome::Placed => {
            if let Err(error) = cache_metadata::write_stamp(&output, stamp, color, rotated) {
                // Best effort: the thumbnail itself is valid; a missing stamp
                // only forces one regeneration on the next request.
                tracing::warn!(
                    %error,
                    photo_id,
                    path = %output.display(),
                    "cannot write thumbnail metadata"
                );
            }
            Ok(Some(EnsuredThumbnail {
                path: output,
                created: true,
                bytes: blocks.len() as u64,
                blocks: Some(blocks),
                color,
                rotated,
            }))
        }
        PlaceOutcome::DestinationExists => {
            let bytes = existing_thumbnail_bytes(&output).ok_or_else(|| {
                CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "thumbnail destination exists but is empty: {}",
                        output.display()
                    ),
                ))
            })?;
            let (color, rotated) =
                cache_metadata::read_color_rotation(&output).unwrap_or((color, rotated));
            Ok(Some(EnsuredThumbnail {
                path: output,
                created: false,
                bytes,
                blocks: None,
                color,
                rotated,
            }))
        }
    }
}

/// Reads the average colour and rotation flag stored beside a thumbnail.
pub fn read_thumbnail_color_rotation(path: &Path) -> Option<(u32, bool)> {
    cache_metadata::read_color_rotation(path)
}

/// Reads a stored BC7 layer and verifies it matches the spec's dimensions.
pub fn read_thumbnail_blocks(path: &Path, spec: ThumbnailSpec) -> Result<Vec<u8>> {
    let (width, height) = ThumbnailSpec::new(spec.size).dimensions();
    let expected = crate::bc7::encoded_len(width, height)
        .ok_or_else(|| CoreError::Invalid(format!("invalid thumbnail canvas: {width}x{height}")))?;
    let blocks = fs::read(path)?;
    if blocks.len() != expected {
        return Err(CoreError::Invalid(format!(
            "thumbnail cache has {} bytes, expected {expected}: {}",
            blocks.len(),
            path.display()
        )));
    }
    Ok(blocks)
}

/// The average visible colour packed as `0xRRGGBB`.
fn average_color(image: &RgbaImage) -> u32 {
    let mut blue = 0u64;
    let mut green = 0u64;
    let mut red = 0u64;
    let mut count = 0u64;
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        if a >= 8 {
            red += u64::from(r);
            green += u64::from(g);
            blue += u64::from(b);
            count += 1;
        }
    }
    if count == 0 {
        return 0;
    }
    let red = (red / count) as u32;
    let green = (green / count) as u32;
    let blue = (blue / count) as u32;
    (red << 16) | (green << 8) | blue
}

/// Returns the non-empty on-disk size of an existing thumbnail.
fn existing_thumbnail_bytes(output: &Path) -> Option<u64> {
    fs::metadata(output)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .filter(|bytes| *bytes > 0)
}

/// Publishes encoded thumbnail bytes without overwriting a file that another
/// worker placed first.
fn publish_thumbnail_bytes(output: &Path, bytes: &[u8]) -> Result<PlaceOutcome> {
    Ok(write_bytes_no_clobber(output, bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::time::{Duration, SystemTime};
    use tempfile::tempdir;

    fn write_cache_entry(path: &Path, bytes: usize, modified: SystemTime) {
        fs::write(path, vec![0u8; bytes]).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }

    fn set_mtime(path: &Path, modified: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }

    #[test]
    fn racing_thumbnail_publish_keeps_the_existing_file() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("thumb.webp");
        fs::write(&output, b"existing").unwrap();

        let outcome = publish_thumbnail_bytes(&output, b"new").unwrap();

        assert_eq!(outcome, PlaceOutcome::DestinationExists);
        assert_eq!(fs::read(&output).unwrap(), b"existing");
    }

    #[test]
    fn prune_thumbnails_removes_oldest_until_under_limit() {
        let dir = tempdir().unwrap();
        let now = SystemTime::now();
        write_cache_entry(
            &dir.path().join("1.webp"),
            1000,
            now - Duration::from_secs(30),
        );
        write_cache_entry(
            &dir.path().join("2.webp"),
            1000,
            now - Duration::from_secs(20),
        );
        write_cache_entry(
            &dir.path().join("3.webp"),
            1000,
            now - Duration::from_secs(10),
        );
        write_cache_entry(&dir.path().join("4.webp"), 1000, now);

        let report = prune_thumbnails_in(dir.path(), 2500).unwrap();

        assert_eq!(report.scanned_files, 4);
        assert_eq!(report.scanned_bytes, 4000);
        assert_eq!(report.removed_files, 2);
        assert_eq!(report.removed_bytes, 2000);
        assert_eq!(report.remaining_files, 2);
        assert_eq!(report.remaining_bytes, 2000);
        assert!(!dir.path().join("1.webp").exists());
        assert!(!dir.path().join("2.webp").exists());
        assert!(dir.path().join("3.webp").exists());
        assert!(dir.path().join("4.webp").exists());
    }

    #[test]
    fn thumbnail_cache_usage_counts_numeric_webp_only() {
        let dir = tempdir().unwrap();
        write_cache_entry(&dir.path().join("1.webp"), 1000, SystemTime::now());
        write_cache_entry(&dir.path().join("2.webp"), 2000, SystemTime::now());
        write_cache_entry(&dir.path().join("keep.webp"), 3000, SystemTime::now());
        write_cache_entry(&dir.path().join("3.webp.tmp"), 4000, SystemTime::now());

        let usage = thumbnail_cache_usage_in(dir.path()).unwrap();

        assert_eq!(usage.files, 2);
        assert_eq!(usage.bytes, 3000);
    }

    #[test]
    fn thumbnail_cache_usage_counts_old_and_spec_keys() {
        let dir = tempdir().unwrap();
        write_cache_entry(&dir.path().join("1.webp"), 1000, SystemTime::now());
        write_cache_entry(&dir.path().join("2-q80-s256.webp"), 2000, SystemTime::now());
        write_cache_entry(&dir.path().join("3-q95-s512.webp"), 3000, SystemTime::now());
        write_cache_entry(
            &dir.path().join("4-q80-s256.webp.tmp"),
            4000,
            SystemTime::now(),
        );
        write_cache_entry(&dir.path().join("notes.webp"), 5000, SystemTime::now());

        let usage = thumbnail_cache_usage_in(dir.path()).unwrap();

        assert_eq!(usage.files, 3);
        assert_eq!(usage.bytes, 6000);
    }

    #[test]
    fn clear_thumbnail_cache_removes_old_new_and_temporary_files() {
        let dir = tempdir().unwrap();
        write_cache_entry(&dir.path().join("1.webp"), 1000, SystemTime::now());
        write_cache_entry(&dir.path().join("1.webp.meta"), 10, SystemTime::now());
        write_cache_entry(&dir.path().join("2-q80-s384.webp"), 2000, SystemTime::now());
        write_cache_entry(
            &dir.path().join("2-q80-s384.webp.meta"),
            10,
            SystemTime::now(),
        );
        write_cache_entry(
            &dir.path().join("3-q80-s384.webp.tmp"),
            3000,
            SystemTime::now(),
        );
        write_cache_entry(
            &dir.path().join("3-q80-s384.webp.tmp-42-7"),
            3000,
            SystemTime::now(),
        );
        write_cache_entry(
            &dir.path().join("3-q80-s384.webp.meta.tmp-42-8"),
            10,
            SystemTime::now(),
        );
        write_cache_entry(&dir.path().join("notes.txt"), 4000, SystemTime::now());
        write_cache_entry(&dir.path().join("keep.webp"), 5000, SystemTime::now());
        fs::create_dir(dir.path().join("sub")).unwrap();

        let report = clear_thumbnail_cache_in(dir.path()).unwrap();

        assert_eq!(report.removed_files, 7);
        assert_eq!(report.removed_bytes, 9_030);
        assert_eq!(report.remaining_files, 0);
        assert_eq!(report.remaining_bytes, 0);
        assert!(dir.path().join("sub").is_dir());
        assert!(dir.path().join("notes.txt").is_file());
        assert!(dir.path().join("keep.webp").is_file());
    }

    #[test]
    fn clear_thumbnail_cache_accepts_a_missing_directory() {
        let dir = tempdir().unwrap();

        let report = clear_thumbnail_cache_in(&dir.path().join("missing")).unwrap();

        assert_eq!(report, ThumbnailCacheClearReport::default());
    }

    #[test]
    fn thumbnail_spec_normalizes_unknown_values() {
        assert_eq!(ThumbnailSpec::new(999), ThumbnailSpec::new(384));
        assert_eq!(ThumbnailSpec::default().dimensions(), (384, 216));
        assert_eq!(
            thumbnail_path_in(Path::new("/cache"), 7, ThumbnailSpec::new(512)),
            Path::new("/cache/7-s512x288.bc7")
        );
    }

    #[test]
    fn a_fresh_cache_entry_with_the_wrong_size_is_regenerated() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let spec = ThumbnailSpec::new(384);
        let output = thumbnail_path_in(dir.path(), 999_994, spec);
        fs::write(&output, vec![0u8; 10]).unwrap();
        let stamp = cache_metadata::source_stamp(&source).unwrap();
        cache_metadata::write_stamp(&output, stamp, 0, false).unwrap();

        let ensured = ensure_thumbnail_in_with_status(999_994, &source, dir.path(), spec)
            .unwrap()
            .expect("regenerated");

        assert!(ensured.created);
        assert_eq!(fs::metadata(&output).unwrap().len(), (384 * 216) as u64);
    }

    #[test]
    fn read_thumbnail_blocks_rejects_a_wrong_length() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("7-s384x216.bc7");
        fs::write(&path, vec![0u8; 10]).unwrap();
        let error = read_thumbnail_blocks(&path, ThumbnailSpec::new(384)).unwrap_err();
        assert!(error.to_string().contains("expected"));
    }

    #[test]
    fn legacy_webp_entries_are_removed_without_touching_other_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("1.webp"), b"old").unwrap();
        fs::write(dir.path().join("1.webp.meta"), b"old").unwrap();
        fs::write(dir.path().join("2.bc7"), b"new").unwrap();
        fs::write(dir.path().join("notes.txt"), b"keep").unwrap();

        let removed = remove_legacy_webp_entries(dir.path()).unwrap();

        assert_eq!(removed, 2);
        assert!(!dir.path().join("1.webp").exists());
        assert!(!dir.path().join("1.webp.meta").exists());
        assert!(dir.path().join("2.bc7").exists());
        assert!(dir.path().join("notes.txt").exists());
    }

    #[test]
    fn prune_thumbnails_keeps_files_when_under_limit() {
        let dir = tempdir().unwrap();
        write_cache_entry(&dir.path().join("1.webp"), 1000, SystemTime::now());

        let report = prune_thumbnails_in(dir.path(), 2000).unwrap();

        assert_eq!(report.removed_files, 0);
        assert_eq!(report.remaining_bytes, 1000);
        assert!(dir.path().join("1.webp").exists());
    }

    #[test]
    fn prune_thumbnails_zero_limit_is_unlimited() {
        let dir = tempdir().unwrap();
        write_cache_entry(&dir.path().join("1.webp"), 1000, SystemTime::now());

        let report = prune_thumbnails_in(dir.path(), 0).unwrap();

        assert_eq!(report.removed_files, 0);
        assert_eq!(report.remaining_files, 1);
        assert!(dir.path().join("1.webp").exists());
    }

    #[test]
    fn prune_thumbnails_ignores_non_thumbnail_files() {
        let dir = tempdir().unwrap();
        let now = SystemTime::now();
        write_cache_entry(
            &dir.path().join("1.webp"),
            5000,
            now - Duration::from_secs(10),
        );
        write_cache_entry(
            &dir.path().join("keep.webp"),
            5000,
            now - Duration::from_secs(20),
        );
        write_cache_entry(
            &dir.path().join("2.webp.tmp"),
            5000,
            now - Duration::from_secs(30),
        );
        fs::write(dir.path().join("notes.txt"), vec![0u8; 5000]).unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();

        let report = prune_thumbnails_in(dir.path(), 1000).unwrap();

        assert_eq!(report.scanned_files, 1);
        assert_eq!(report.removed_files, 1);
        assert!(!dir.path().join("1.webp").exists());
        assert!(dir.path().join("keep.webp").exists());
        assert!(dir.path().join("2.webp.tmp").exists());
        assert!(dir.path().join("notes.txt").exists());
        assert!(dir.path().join("sub").is_dir());
    }

    #[test]
    fn prune_thumbnails_missing_directory_is_empty() {
        let dir = tempdir().unwrap();

        let report = prune_thumbnails_in(&dir.path().join("missing"), 1024).unwrap();

        assert_eq!(report.scanned_files, 0);
        assert_eq!(report.removed_files, 0);
    }

    #[test]
    fn prune_thumbnails_removes_every_entry_when_limit_is_below_one_file() {
        let dir = tempdir().unwrap();
        write_cache_entry(&dir.path().join("1.webp"), 1000, SystemTime::now());

        let report = prune_thumbnails_in(dir.path(), 1).unwrap();

        assert_eq!(report.removed_files, 1);
        assert_eq!(report.remaining_bytes, 0);
        assert!(!dir.path().join("1.webp").exists());
    }

    #[test]
    fn prune_continues_after_a_removal_error_and_retries_on_the_next_pass() {
        let dir = tempdir().unwrap();
        let now = SystemTime::now();
        write_cache_entry(
            &dir.path().join("1.webp"),
            1000,
            now - Duration::from_secs(30),
        );
        write_cache_entry(
            &dir.path().join("2.webp"),
            1000,
            now - Duration::from_secs(20),
        );
        write_cache_entry(
            &dir.path().join("3.webp"),
            1000,
            now - Duration::from_secs(10),
        );

        let report = prune_thumbnails_with(dir.path(), 2500, |path| {
            if path.ends_with("1.webp") {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected removal failure",
                ))
            } else {
                fs::remove_file(path)
            }
        })
        .unwrap();

        assert_eq!(report.removed_files, 1);
        assert_eq!(report.removed_bytes, 1000);
        assert_eq!(report.remaining_bytes, 2000);
        assert!(dir.path().join("1.webp").exists());
        assert!(!dir.path().join("2.webp").exists());
        assert!(dir.path().join("3.webp").exists());

        // The failed entry is retried on the next pass once removal works.
        let retry = prune_thumbnails_with(dir.path(), 1500, |path| fs::remove_file(path)).unwrap();

        assert_eq!(retry.removed_files, 1);
        assert_eq!(retry.remaining_bytes, 1000);
        assert!(!dir.path().join("1.webp").exists());
        assert!(dir.path().join("3.webp").exists());
    }

    #[test]
    fn prune_reports_removal_failures_without_stopping_other_deletions() {
        let dir = tempdir().unwrap();
        let now = SystemTime::now();
        write_cache_entry(
            &dir.path().join("1.webp"),
            1000,
            now - Duration::from_secs(30),
        );
        write_cache_entry(
            &dir.path().join("2.webp"),
            1000,
            now - Duration::from_secs(20),
        );
        write_cache_entry(
            &dir.path().join("3.webp"),
            1000,
            now - Duration::from_secs(10),
        );

        let report = prune_thumbnails_with(dir.path(), 500, |path| {
            if path.ends_with("1.webp") || path.ends_with("3.webp") {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected removal failure",
                ))
            } else {
                fs::remove_file(path)
            }
        })
        .unwrap();

        assert_eq!(report.scanned_files, 3);
        assert_eq!(report.scanned_bytes, 3000);
        assert_eq!(report.removed_files, 1);
        assert_eq!(report.removed_bytes, 1000);
        assert_eq!(report.remaining_files, 2);
        assert_eq!(report.remaining_bytes, 2000);
        assert!(dir.path().join("1.webp").exists());
        assert!(!dir.path().join("2.webp").exists());
        assert!(dir.path().join("3.webp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn prune_retries_after_directory_permissions_are_restored() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let now = SystemTime::now();
        write_cache_entry(
            &dir.path().join("1.webp"),
            1000,
            now - Duration::from_secs(20),
        );
        write_cache_entry(
            &dir.path().join("2.webp"),
            1000,
            now - Duration::from_secs(10),
        );
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();

        let blocked = prune_thumbnails_in(dir.path(), 500).unwrap();

        if blocked.removed_files > 0 {
            // A privileged user (for example root) ignores directory write
            // permissions, so there is nothing left to verify here.
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        assert_eq!(blocked.remaining_files, 2);
        assert_eq!(blocked.remaining_bytes, 2000);

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let retried = prune_thumbnails_in(dir.path(), 500).unwrap();

        assert_eq!(retried.removed_files, 2);
        assert_eq!(retried.remaining_files, 0);
        assert_eq!(retried.remaining_bytes, 0);
        assert!(!dir.path().join("1.webp").exists());
        assert!(!dir.path().join("2.webp").exists());
    }

    #[cfg(windows)]
    #[test]
    fn prune_retries_a_file_locked_by_another_handle() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempdir().unwrap();
        let now = SystemTime::now();
        let locked_path = dir.path().join("1.webp");
        write_cache_entry(&locked_path, 1000, now - Duration::from_secs(20));
        write_cache_entry(
            &dir.path().join("2.webp"),
            1000,
            now - Duration::from_secs(10),
        );
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked_path)
            .unwrap();

        let first = prune_thumbnails_in(dir.path(), 500).unwrap();

        assert_eq!(first.removed_files, 1);
        assert_eq!(first.remaining_files, 1);
        assert_eq!(first.remaining_bytes, 1000);
        drop(locked);
        assert!(locked_path.exists());
        assert!(!dir.path().join("2.webp").exists());

        let second = prune_thumbnails_in(dir.path(), 500).unwrap();

        assert_eq!(second.removed_files, 1);
        assert_eq!(second.remaining_bytes, 0);
        assert!(!locked_path.exists());
    }

    #[test]
    fn refresh_thumbnail_access_rewrites_a_stale_mtime() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("1.webp");
        let stale = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        write_cache_entry(&path, 10, stale);
        let before = fs::metadata(&path).unwrap().modified().unwrap();

        refresh_thumbnail_access(&path);

        let after = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(after > before);
    }

    #[test]
    fn refresh_thumbnail_access_leaves_a_recent_mtime_alone() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("1.webp");
        let recent = SystemTime::now() - Duration::from_secs(5);
        write_cache_entry(&path, 10, recent);
        let before = fs::metadata(&path).unwrap().modified().unwrap();

        refresh_thumbnail_access(&path);

        let after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn creates_and_reuses_thumbnail() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let spec = ThumbnailSpec::new(384);
        let first = ensure_thumbnail_in(999_999, &source, dir.path(), spec)
            .unwrap()
            .expect("thumbnail");
        assert!(first.is_file());
        assert!(first.to_string_lossy().contains("-s384x216.bc7"));
        assert_eq!(
            fs::metadata(&first).unwrap().len(),
            384 * 216,
            "BC7 stores one byte per pixel"
        );
        let second = ensure_thumbnail_in(999_999, &source, dir.path(), spec)
            .unwrap()
            .expect("cached");
        assert_eq!(first, second);
    }

    #[test]
    fn ensure_reports_created_bytes_for_a_new_thumbnail() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();

        let ensured =
            ensure_thumbnail_in_with_status(999_997, &source, dir.path(), ThumbnailSpec::default())
                .unwrap()
                .expect("thumbnail");

        assert!(ensured.created);
        assert!(ensured.bytes > 0);
        assert_eq!(ensured.bytes, fs::metadata(&ensured.path).unwrap().len());
        assert!(ensured.path.is_file());
        let blocks = ensured.blocks.expect("generated blocks are returned");
        assert_eq!(blocks.len(), 384 * 216);
        assert!(ensured.color != 0 || ensured.color == 0);
    }

    #[test]
    fn ensure_reports_a_cache_hit_without_creation() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let first =
            ensure_thumbnail_in_with_status(999_996, &source, dir.path(), ThumbnailSpec::default())
                .unwrap()
                .expect("thumbnail");

        let second =
            ensure_thumbnail_in_with_status(999_996, &source, dir.path(), ThumbnailSpec::default())
                .unwrap()
                .expect("cached");

        assert!(!second.created);
        assert_eq!(second.bytes, first.bytes);
        assert_eq!(second.path, first.path);
        assert!(first.blocks.is_some(), "a generated thumbnail has blocks");
        assert!(second.blocks.is_none(), "a cache hit is read from the file");
        assert_eq!(second.color, first.color);
        assert_eq!(second.rotated, first.rotated);
    }

    #[test]
    fn jpeg_thumbnail_uses_scaled_decode_and_applies_orientation() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.jpg");
        let image = RgbaImage::from_pixel(2048, 1024, Rgba([200, 100, 50, 255]));
        crate::image_decode::test_support::write_jpeg_with_orientation(&source, &image, 6);

        let ensured =
            ensure_thumbnail_in_with_status(999_995, &source, dir.path(), ThumbnailSpec::default())
                .unwrap()
                .expect("thumbnail");

        let blocks = ensured.blocks.expect("generated blocks are returned");
        assert_eq!(blocks.len(), 384 * 216);
        assert!(
            ensured.rotated,
            "orientation 6 turns the landscape source portrait, so the canvas is rotated"
        );
    }

    #[test]
    fn unchanged_source_reuses_the_thumbnail() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let spec = ThumbnailSpec::default();

        let first = ensure_thumbnail_in_with_status(999_995, &source, dir.path(), spec)
            .unwrap()
            .expect("thumbnail");
        assert!(first.created);
        assert!(cache_metadata::metadata_path(&first.path).is_file());

        let second = ensure_thumbnail_in_with_status(999_995, &source, dir.path(), spec)
            .unwrap()
            .expect("cached");
        assert!(!second.created);
    }

    #[test]
    fn source_mtime_change_regenerates_the_thumbnail() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let spec = ThumbnailSpec::default();
        ensure_thumbnail_in_with_status(999_994, &source, dir.path(), spec)
            .unwrap()
            .expect("thumbnail");

        set_mtime(&source, SystemTime::now() + Duration::from_secs(5));
        let refreshed = ensure_thumbnail_in_with_status(999_994, &source, dir.path(), spec)
            .unwrap()
            .expect("regenerated");

        assert!(refreshed.created);
    }

    #[test]
    fn source_size_change_regenerates_even_with_the_same_mtime() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let spec = ThumbnailSpec::default();
        ensure_thumbnail_in_with_status(999_993, &source, dir.path(), spec)
            .unwrap()
            .expect("thumbnail");
        let modified = fs::metadata(&source).unwrap().modified().unwrap();

        RgbaImage::from_pixel(800, 450, Rgba([200, 100, 50, 255]))
            .save(&source)
            .unwrap();
        set_mtime(&source, modified);
        let refreshed = ensure_thumbnail_in_with_status(999_993, &source, dir.path(), spec)
            .unwrap()
            .expect("regenerated");

        assert!(refreshed.created);
    }

    #[test]
    fn missing_sidecar_regenerates_an_old_cache_entry_once() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(320, 200, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let spec = ThumbnailSpec::default();
        let ensured = ensure_thumbnail_in_with_status(999_992, &source, dir.path(), spec)
            .unwrap()
            .expect("thumbnail");
        fs::remove_file(cache_metadata::metadata_path(&ensured.path)).unwrap();

        let regenerated = ensure_thumbnail_in_with_status(999_992, &source, dir.path(), spec)
            .unwrap()
            .expect("regenerated");
        assert!(regenerated.created);

        let cached = ensure_thumbnail_in_with_status(999_992, &source, dir.path(), spec)
            .unwrap()
            .expect("cached");
        assert!(!cached.created);
    }

    #[test]
    fn concurrent_generation_leaves_one_decodable_thumbnail_and_no_temps() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let spec = ThumbnailSpec::default();

        std::thread::scope(|scope| {
            for _ in 0..8 {
                let source = &source;
                let cache = dir.path();
                scope.spawn(move || {
                    let ensured = ensure_thumbnail_in_with_status(999_991, source, cache, spec)
                        .unwrap()
                        .expect("thumbnail");
                    assert!(ensured.path.is_file());
                });
            }
        });

        let output = thumbnail_path_in(dir.path(), 999_991, spec);
        let blocks = read_thumbnail_blocks(&output, spec).expect("decodable thumbnail");
        assert_eq!(blocks.len(), (spec.size * spec.size * 9 / 16) as usize);
        assert!(cache_metadata::cache_is_fresh(&output, &source));
        for entry in fs::read_dir(dir.path()).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            assert!(!name.contains(".tmp"), "leftover temp file: {name}");
        }
    }

    #[test]
    fn remove_thumbnail_files_removes_every_variant_and_sidecar() {
        let dir = tempdir().unwrap();
        for name in [
            "7.webp",
            "7.webp.meta",
            "7-q80-s256.webp",
            "7-q80-s256.webp.meta",
            "7-q95-s512.webp.tmp-42-1",
            "8-q80-s256.webp",
            "8-q80-s256.webp.meta",
            "17.webp",
            "notes.txt",
        ] {
            fs::write(dir.path().join(name), b"data").unwrap();
        }

        let removed = remove_thumbnail_files_in(dir.path(), 7).unwrap();

        assert_eq!(removed, 5);
        for name in [
            "7.webp",
            "7.webp.meta",
            "7-q80-s256.webp",
            "7-q80-s256.webp.meta",
            "7-q95-s512.webp.tmp-42-1",
        ] {
            assert!(!dir.path().join(name).exists(), "still present: {name}");
        }
        assert!(dir.path().join("8-q80-s256.webp").exists());
        assert!(dir.path().join("8-q80-s256.webp.meta").exists());
        assert!(dir.path().join("17.webp").exists());
        assert!(dir.path().join("notes.txt").exists());
    }

    #[test]
    fn pruning_a_thumbnail_also_removes_its_sidecar() {
        let dir = tempdir().unwrap();
        let now = SystemTime::now();
        write_cache_entry(
            &dir.path().join("1.webp"),
            1000,
            now - Duration::from_secs(20),
        );
        write_cache_entry(
            &dir.path().join("1.webp.meta"),
            10,
            now - Duration::from_secs(20),
        );
        write_cache_entry(&dir.path().join("2.webp"), 1000, now);

        let report = prune_thumbnails_in(dir.path(), 1500).unwrap();

        assert_eq!(report.removed_files, 1);
        assert_eq!(report.removed_bytes, 1000);
        assert!(!dir.path().join("1.webp").exists());
        assert!(!dir.path().join("1.webp.meta").exists());
        assert!(dir.path().join("2.webp").exists());
    }

    #[test]
    fn failed_rename_removes_the_unique_temp_file() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let spec = ThumbnailSpec::default();
        // A directory at the output path makes the atomic rename fail after
        // the temp file has been written.
        fs::create_dir(thumbnail_path_in(dir.path(), 999_990, spec)).unwrap();

        let result = ensure_thumbnail_in_with_status(999_990, &source, dir.path(), spec);

        assert!(result.is_err());
        for entry in fs::read_dir(dir.path()).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            assert!(!name.contains(".tmp"), "leftover temp file: {name}");
        }
    }

    #[test]
    fn missing_source_is_not_an_error() {
        let dir = tempdir().unwrap();
        assert!(
            ensure_thumbnail_in(
                999_998,
                Path::new("/no/such/file.png"),
                dir.path(),
                ThumbnailSpec::default(),
            )
            .unwrap()
            .is_none()
        );
    }
}
