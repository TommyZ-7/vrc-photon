//! Shared peak-memory budget for compression workers.
//!
//! A compression worker holds the decoded image, a resized copy, and the
//! encoder output at the same time. With up to four workers the daemon could
//! otherwise allocate several gigabytes of transient memory. This budget caps
//! the combined reservation; a worker waits when the next decode would exceed
//! it, and a single decode larger than the cap runs alone.

use std::path::Path;
use std::sync::{Condvar, Mutex};

use vrc_photon_core::image_decode;

/// Total transient bytes the compression workers may reserve at once.
pub(crate) const COMPRESSION_MEMORY_BUDGET_BYTES: usize = 1024 * 1024 * 1024;

/// A process-wide reservation budget shared by every compression worker.
pub(crate) struct MemoryBudget {
    cap: usize,
    reserved: Mutex<usize>,
    available: Condvar,
}

impl MemoryBudget {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            cap,
            reserved: Mutex::new(0),
            available: Condvar::new(),
        }
    }

    /// Reserves memory for one photo batch, waiting for headroom if needed.
    pub(crate) fn reserve(&self, bytes: usize) -> MemoryPermit<'_> {
        // A single decode larger than the cap still runs, but only while no
        // other decode holds a reservation: account it as the whole cap.
        let charge = bytes.min(self.cap);
        let mut reserved = self
            .reserved
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while reserved.saturating_add(charge) > self.cap {
            reserved = self
                .available
                .wait(reserved)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *reserved += charge;
        MemoryPermit {
            budget: self,
            bytes: charge,
        }
    }

    fn release(&self, bytes: usize) {
        let mut reserved = self
            .reserved
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *reserved = reserved.saturating_sub(bytes);
        self.available.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn reserved(&self) -> usize {
        *self
            .reserved
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) struct MemoryPermit<'a> {
    budget: &'a MemoryBudget,
    bytes: usize,
}

impl Drop for MemoryPermit<'_> {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

/// Estimates the transient bytes one photo batch needs.
///
/// The decoded image stays alive while each profile builds a resized copy and
/// an encoded output, so three RGBA buffers bound the working set. The source
/// file size is added because AVIF/JXL decoders read the whole compressed file
/// into memory. When the header cannot be read, fall back to the full
/// [`image_decode::FULL_IMAGE_MAX`] square so a corrupt file cannot bypass the
/// budget.
pub(crate) fn compression_peak_estimate(path: &Path) -> usize {
    let (width, height) = image_decode::image_dimensions(path).unwrap_or((0, 0));
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    let rgba = usize::try_from(pixels.saturating_mul(4)).unwrap_or(usize::MAX);
    let fallback = (image_decode::FULL_IMAGE_MAX as usize)
        .saturating_mul(image_decode::FULL_IMAGE_MAX as usize)
        .saturating_mul(4);
    let source = if rgba == 0 { fallback } else { rgba };
    let file_bytes = std::fs::metadata(path)
        .ok()
        .and_then(|meta| usize::try_from(meta.len()).ok())
        .unwrap_or(0);
    source.saturating_mul(3).saturating_add(file_bytes)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn reserve_accounts_and_releases_bytes() {
        let budget = MemoryBudget::new(100);
        let permit = budget.reserve(60);
        assert_eq!(budget.reserved(), 60);
        drop(permit);
        assert_eq!(budget.reserved(), 0);
    }

    #[test]
    fn waits_until_headroom_is_available() {
        let budget = Arc::new(MemoryBudget::new(100));
        let first = budget.reserve(60);
        let (tx, rx) = mpsc::channel();
        let waiting = {
            let budget = Arc::clone(&budget);
            thread::spawn(move || {
                let _permit = budget.reserve(60);
                tx.send(()).expect("notify waiter");
            })
        };
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "second reservation must wait for headroom"
        );
        drop(first);
        rx.recv_timeout(Duration::from_secs(2))
            .expect("second reservation runs after the first releases");
        waiting.join().expect("waiter thread");
        assert_eq!(budget.reserved(), 0);
    }

    #[test]
    fn an_oversized_decode_runs_alone() {
        let budget = Arc::new(MemoryBudget::new(100));
        let first = budget.reserve(80);
        let (tx, rx) = mpsc::channel();
        let waiting = {
            let budget = Arc::clone(&budget);
            thread::spawn(move || {
                let permit = budget.reserve(150);
                tx.send(()).expect("notify waiter");
                drop(permit);
            })
        };
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "an oversized decode waits until the current decode releases"
        );
        drop(first);
        rx.recv_timeout(Duration::from_secs(2))
            .expect("oversized decode runs alone");
        waiting.join().expect("waiter thread");
        assert_eq!(budget.reserved(), 0);
    }

    #[test]
    fn compression_estimate_covers_three_rgba_buffers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.png");
        image::RgbaImage::from_pixel(16, 10, image::Rgba([1, 2, 3, 255]))
            .save(&path)
            .unwrap();
        let rgba = 16usize * 10 * 4;
        assert!(
            compression_peak_estimate(&path) >= rgba * 3,
            "estimate must cover the decoded image, resized copy, and encoded output"
        );
    }

    #[test]
    fn unknown_dimensions_use_the_conservative_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-an-image.png");
        std::fs::write(&path, b"not an image").unwrap();
        let fallback = (image_decode::FULL_IMAGE_MAX as usize)
            .saturating_mul(image_decode::FULL_IMAGE_MAX as usize)
            .saturating_mul(4);
        assert!(compression_peak_estimate(&path) >= fallback * 3);
    }
}
