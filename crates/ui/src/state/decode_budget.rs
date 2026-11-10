//! Process-wide reservation budget for decoded image memory in the UI.

use std::path::Path;
use std::sync::{Condvar, LazyLock, Mutex};

use vrc_photon_core::image_decode::FULL_IMAGE_MAX;

/// Total decoded-image bytes the UI may reserve at once.
pub(crate) const DECODE_BUDGET_BYTES: usize = 256 * 1024 * 1024;

static BUDGET: LazyLock<DecodeBudget> = LazyLock::new(|| DecodeBudget::new(DECODE_BUDGET_BYTES));

/// Reserves decoded-image memory for one decode, waiting for headroom if needed.
pub(crate) fn reserve(bytes: usize) -> DecodePermit<'static> {
    BUDGET.reserve(bytes)
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
pub(crate) fn full_image_bytes(width: u32, height: u32, source: FullImageSource) -> usize {
    if width == 0 || height == 0 {
        return 0;
    }
    let source_bytes = rgba_bytes(width, height);
    let mut estimate = match source {
        FullImageSource::DirectRgba => source_bytes,
        FullImageSource::Converted => source_bytes.saturating_mul(2),
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
    let (width, height) = vrc_photon_core::image_decode::image_dimensions(path).unwrap_or(fallback);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    let source = match extension.as_deref() {
        Some("jpg" | "jpeg" | "png") => FullImageSource::DirectRgba,
        _ => FullImageSource::Converted,
    };
    full_image_bytes(width, height, source)
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
        let mut reserved = self.reserved.lock().expect("decode budget lock poisoned");
        while reserved.saturating_add(charge) > self.cap {
            reserved = self
                .available
                .wait(reserved)
                .expect("decode budget lock poisoned");
        }
        *reserved += charge;
        DecodePermit {
            budget: self,
            bytes: charge,
        }
    }

    fn release(&self, bytes: usize) {
        let mut reserved = self.reserved.lock().expect("decode budget lock poisoned");
        *reserved = reserved.saturating_sub(bytes);
        self.available.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn reserved(&self) -> usize {
        *self.reserved.lock().expect("decode budget lock poisoned")
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
    fn full_image_estimate_accounts_for_the_conversion_buffer() {
        assert_eq!(
            full_image_bytes(100, 50, FullImageSource::DirectRgba),
            20_000
        );
        assert_eq!(
            full_image_bytes(100, 50, FullImageSource::Converted),
            40_000
        );
    }

    #[test]
    fn full_image_estimate_accounts_for_the_downscaled_output() {
        assert_eq!(
            full_image_bytes(10_000, 1_000, FullImageSource::DirectRgba),
            rgba_bytes(10_000, 1_000) + rgba_bytes(FULL_IMAGE_MAX, 1_000)
        );
    }

    #[test]
    fn full_image_estimate_ignores_unknown_dimensions() {
        assert_eq!(full_image_bytes(0, 0, FullImageSource::Converted), 0);
    }
}
