//! Process-wide reservation budget for decoded image memory in the UI.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::Duration;

use image::metadata::Orientation;
use vrc_photon_core::image_decode::{FULL_IMAGE_MAX, image_info};

/// Total decoded-image bytes the UI may reserve at once.
pub(crate) const DECODE_BUDGET_BYTES: usize = 256 * 1024 * 1024;

static BUDGET: LazyLock<DecodeBudget> = LazyLock::new(|| DecodeBudget::new(DECODE_BUDGET_BYTES));

/// How often a cancellable reservation re-checks its cancellation token while
/// it waits for headroom.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Reserves decoded-image memory for one decode, waiting for headroom if needed.
pub(crate) fn reserve(bytes: usize) -> DecodePermit<'static> {
    BUDGET.reserve(bytes)
}

/// Reserves decoded-image memory for one decode, giving up with `None` when
/// `cancel` is set while waiting for headroom.
#[allow(dead_code)] // Consumed by the full-image decode queue (tasks 7–8).
pub(crate) fn reserve_cancellable(
    bytes: usize,
    cancel: &AtomicBool,
) -> Option<DecodePermit<'static>> {
    BUDGET.reserve_cancellable(bytes, cancel)
}

/// Estimates the RGBA bytes needed to decode an image of `width` x `height`.
pub(crate) fn rgba_bytes(width: u32, height: u32) -> usize {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    usize::try_from(pixels.saturating_mul(4)).unwrap_or(usize::MAX)
}

/// How the full-image decoder produces RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FullImageSource {
    /// JPEG and PNG decode straight into one RGBA buffer.
    DirectRgba,
    /// Other formats decode through the image crate, which may hold the
    /// source and the converted RGBA buffer at the same time.
    Converted,
}

/// Estimates the peak bytes for one full-image decode.
///
/// `rotated` accounts for the extra RGBA copy made while applying EXIF
/// orientation, and `compressed_bytes` is the on-disk size of an AVIF/JXL
/// source that the image crate decodes through a conversion buffer.
pub(crate) fn full_image_bytes(
    width: u32,
    height: u32,
    source: FullImageSource,
    rotated: bool,
    compressed_bytes: usize,
) -> usize {
    if width == 0 || height == 0 {
        return 0;
    }
    let source_bytes = rgba_bytes(width, height);
    let mut estimate = match source {
        FullImageSource::DirectRgba if rotated => source_bytes.saturating_mul(2),
        FullImageSource::DirectRgba => source_bytes,
        FullImageSource::Converted => source_bytes
            .saturating_mul(2)
            .saturating_add(compressed_bytes),
    };
    if width.max(height) > FULL_IMAGE_MAX {
        let scaled = rgba_bytes(width.min(FULL_IMAGE_MAX), height.min(FULL_IMAGE_MAX));
        estimate = estimate.max(source_bytes.saturating_add(scaled));
    }
    estimate
}

/// Estimates the full-image peak from the file header, falling back to the
/// database dimensions when the format cannot be read cheaply.
pub(crate) fn full_image_estimate(path: &Path, fallback: (u32, u32)) -> usize {
    let (width, height, rotated) = image_info(path)
        .map(|info| {
            (
                info.width,
                info.height,
                info.orientation != Orientation::NoTransforms,
            )
        })
        .unwrap_or((fallback.0, fallback.1, false));
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    let source = match extension.as_deref() {
        Some("jpg" | "jpeg" | "png") => FullImageSource::DirectRgba,
        _ => FullImageSource::Converted,
    };
    let compressed_bytes = if matches!(extension.as_deref(), Some("avif" | "jxl")) {
        std::fs::metadata(path)
            .map(|metadata| metadata.len() as usize)
            .unwrap_or(0)
    } else {
        0
    };
    full_image_bytes(width, height, source, rotated, compressed_bytes)
}

pub(crate) struct DecodeBudget {
    cap: usize,
    reserved: Mutex<usize>,
    available: Condvar,
}

impl DecodeBudget {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            cap,
            reserved: Mutex::new(0),
            available: Condvar::new(),
        }
    }

    pub(crate) fn reserve(&self, bytes: usize) -> DecodePermit<'_> {
        // A single decode larger than the cap still runs, but only while no
        // other decode holds a reservation: account it as the whole cap.
        let charge = bytes.min(self.cap);
        let mut reserved = self.lock_reserved();
        while reserved.saturating_add(charge) > self.cap {
            reserved = self
                .available
                .wait(reserved)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *reserved += charge;
        DecodePermit {
            budget: self,
            bytes: charge,
        }
    }

    /// Like [`DecodeBudget::reserve`], but returns `None` as soon as `cancel`
    /// is set: a viewer that moved on must not keep blocking its decode queue
    /// on headroom it no longer needs.
    pub(crate) fn reserve_cancellable(
        &self,
        bytes: usize,
        cancel: &AtomicBool,
    ) -> Option<DecodePermit<'_>> {
        let charge = bytes.min(self.cap);
        let mut reserved = self.lock_reserved();
        while reserved.saturating_add(charge) > self.cap {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            reserved = self
                .available
                .wait_timeout(reserved, CANCEL_POLL_INTERVAL)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        *reserved += charge;
        Some(DecodePermit {
            budget: self,
            bytes: charge,
        })
    }

    fn release(&self, bytes: usize) {
        let mut reserved = self.lock_reserved();
        *reserved = reserved.saturating_sub(bytes);
        self.available.notify_all();
    }

    /// A panic while holding the reservation mutex must not stop every later
    /// image decode; recover the guarded count and continue.
    fn lock_reserved(&self) -> std::sync::MutexGuard<'_, usize> {
        self.reserved
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn reserved(&self) -> usize {
        *self.lock_reserved()
    }
}

pub(crate) struct DecodePermit<'a> {
    budget: &'a DecodeBudget,
    bytes: usize,
}

impl Drop for DecodePermit<'_> {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
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
        let budget = DecodeBudget::new(100);
        let permit = budget.reserve(60);
        assert_eq!(budget.reserved(), 60);
        drop(permit);
        assert_eq!(budget.reserved(), 0);
    }

    #[test]
    fn reserve_recovers_from_a_poisoned_lock() {
        let budget = Arc::new(DecodeBudget::new(100));
        let poisoned = Arc::clone(&budget);
        let _ = thread::spawn(move || {
            let _guard = poisoned.reserved.lock().expect("hold the budget lock");
            panic!("poison the decode budget");
        })
        .join();

        let permit = budget.reserve(10);
        assert_eq!(budget.reserved(), 10);
        drop(permit);
        assert_eq!(budget.reserved(), 0);
    }

    #[test]
    fn waits_until_headroom_is_available() {
        let budget = Arc::new(DecodeBudget::new(100));
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
    fn cancelled_reserve_returns_none_immediately() {
        let budget = DecodeBudget::new(100);
        let cancel = std::sync::atomic::AtomicBool::new(true);
        assert!(budget.reserve_cancellable(10, &cancel).is_none());
    }

    #[test]
    fn cancelled_waiter_returns_none_without_taking_budget() {
        let budget = Arc::new(DecodeBudget::new(100));
        let first = budget.reserve(60);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let waiter_budget = Arc::clone(&budget);
        let waiter_cancel = Arc::clone(&cancel);
        let waiter = thread::spawn(move || {
            let result = waiter_budget.reserve_cancellable(60, &waiter_cancel);
            tx.send(result.is_none()).expect("send result");
        });
        thread::sleep(Duration::from_millis(50));
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            rx.recv_timeout(Duration::from_secs(2))
                .expect("waiter result")
        );
        drop(first);
        waiter.join().expect("waiter thread");
        assert_eq!(budget.reserved(), 0);
    }

    #[test]
    fn an_oversized_decode_runs_alone() {
        let budget = Arc::new(DecodeBudget::new(100));
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
    fn rgba_bytes_uses_four_bytes_per_pixel() {
        assert_eq!(rgba_bytes(10, 5), 200);
    }

    #[test]
    fn rgba_bytes_saturates_for_extreme_dimensions() {
        assert_eq!(rgba_bytes(u32::MAX, u32::MAX), usize::MAX);
    }

    #[test]
    fn converted_estimate_includes_the_compressed_file_size() {
        assert_eq!(
            full_image_bytes(100, 50, FullImageSource::Converted, false, 1_000),
            40_000 + 1_000
        );
    }

    #[test]
    fn avif_estimate_reads_the_compressed_file_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.avif");
        std::fs::write(&path, vec![0u8; 1_000]).unwrap();

        assert_eq!(full_image_estimate(&path, (100, 50)), 41_000);
    }

    #[test]
    fn rotated_direct_rgba_counts_the_orientation_copy() {
        assert_eq!(
            full_image_bytes(100, 50, FullImageSource::DirectRgba, true, 0),
            40_000
        );
    }

    #[test]
    fn full_image_estimate_accounts_for_the_conversion_buffer() {
        assert_eq!(
            full_image_bytes(100, 50, FullImageSource::DirectRgba, false, 0),
            20_000
        );
        assert_eq!(
            full_image_bytes(100, 50, FullImageSource::Converted, false, 0),
            40_000
        );
    }

    #[test]
    fn full_image_estimate_accounts_for_the_downscaled_output() {
        assert_eq!(
            full_image_bytes(10_000, 1_000, FullImageSource::DirectRgba, false, 0),
            rgba_bytes(10_000, 1_000) + rgba_bytes(FULL_IMAGE_MAX, 1_000)
        );
    }

    #[test]
    fn full_image_estimate_ignores_unknown_dimensions() {
        assert_eq!(
            full_image_bytes(0, 0, FullImageSource::Converted, false, 0),
            0
        );
    }
}
