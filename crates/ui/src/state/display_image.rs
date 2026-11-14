//! Display-image identity and the single-decode queue used by Quick Look.

use vrc_photon_core::model::Photo;

/// Identifies a display file by photo and displayed derivative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DisplayKey {
    pub(crate) photo_id: i64,
    pub(crate) variant: i64,
}

pub(crate) fn display_key(photo: &Photo) -> DisplayKey {
    DisplayKey {
        photo_id: photo.id,
        variant: photo.display_derivative_id.unwrap_or(0),
    }
}

/// Keeps one full-image decode in flight and remembers the latest request.
///
/// A selection change while a decode is running only replaces `wanted`; the
/// running decode finishes and is dropped by the caller when stale, then the
/// latest key starts. This bounds background CPU during fast navigation.
#[derive(Debug, Default)]
pub(crate) struct DecodeQueue {
    in_flight: Option<DisplayKey>,
    wanted: Option<DisplayKey>,
}

impl DecodeQueue {
    /// Records `key` as the latest request. Returns true when the caller
    /// should start a decode immediately.
    pub(crate) fn want(&mut self, key: DisplayKey) -> bool {
        if self.in_flight == Some(key) {
            self.wanted = None;
            return false;
        }
        self.wanted = Some(key);
        self.in_flight.is_none()
    }

    /// Takes the queued key and marks it in flight. Returns None while another
    /// decode is still running.
    pub(crate) fn begin(&mut self) -> Option<DisplayKey> {
        if self.in_flight.is_some() {
            return None;
        }
        let key = self.wanted.take()?;
        self.in_flight = Some(key);
        Some(key)
    }

    /// Clears `key` if it is still the in-flight decode.
    pub(crate) fn finish(&mut self, key: DisplayKey) {
        if self.in_flight == Some(key) {
            self.in_flight = None;
        }
    }

    pub(crate) fn cancel_wanted(&mut self) {
        self.wanted = None;
    }

    pub(crate) fn pending_for(&self, photo_id: i64) -> bool {
        self.in_flight.is_some_and(|key| key.photo_id == photo_id)
            || self.wanted.is_some_and(|key| key.photo_id == photo_id)
    }
}

/// Whether a finished decode for `key` should be applied.
///
/// The result is stale when the selection has moved on (or is gone) by the
/// time the decode completes; stale results are dropped by the caller.
pub(crate) fn should_apply_image(selected: Option<DisplayKey>, key: DisplayKey) -> bool {
    selected == Some(key)
}

/// Whether a decoded full image should be parked for upload.
///
/// Closing Quick Look or moving the selection while a decode runs makes the
/// result stale; it must be dropped instead of uploaded.
pub(crate) fn should_apply_pending_full_image(
    open: bool,
    selected: Option<DisplayKey>,
    key: DisplayKey,
) -> bool {
    open && should_apply_image(selected, key)
}

pub(crate) fn thumbnail_placeholder_label(pending: bool) -> String {
    if pending {
        vrc_photon_i18n::tr!("thumbnail.loading")
    } else {
        vrc_photon_i18n::tr!("thumbnail.error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrc_photon_core::model::PhotoStatus;

    fn key(photo_id: i64) -> DisplayKey {
        DisplayKey {
            photo_id,
            variant: 0,
        }
    }

    #[test]
    fn queue_starts_one_decode_and_holds_the_latest_request() {
        let mut queue = DecodeQueue::default();
        assert!(queue.want(key(1)));
        assert_eq!(queue.begin(), Some(key(1)));
        // A second request while one is in flight is held, not started.
        assert!(!queue.want(key(2)));
        assert_eq!(queue.begin(), None);
        queue.finish(key(1));
        assert_eq!(queue.begin(), Some(key(2)));
    }

    #[test]
    fn cancel_wanted_drops_the_queued_request() {
        // The reset path (selection clear / viewer close / removal) relies
        // on this: after a cancel, no decode starts for the old selection.
        let mut queue = DecodeQueue::default();
        assert!(queue.want(key(1)));
        assert_eq!(queue.begin(), Some(key(1)));
        assert!(!queue.want(key(2)));
        queue.cancel_wanted();
        queue.finish(key(1));
        assert_eq!(queue.begin(), None);
        assert!(!queue.pending_for(2));
    }

    fn test_photo(id: i64) -> Photo {
        Photo {
            id,
            root_id: 1,
            rel_path: format!("p/{id}.png"),
            abs_path: format!("/p/{id}.png"),
            file_name: format!("{id}.png"),
            captured_at_utc: id,
            captured_at_local: "2026-10-21 21:14:03".to_string(),
            tz_offset_sec: 0,
            width: 1,
            height: 1,
            file_size: 1,
            mtime: id,
            blake3: None,
            status: PhotoStatus::Present,
            is_favorite: false,
            note: None,
            display_derivative_id: None,
            display_path: None,
            display_format: None,
            display_file_size: None,
            display_width: None,
            display_height: None,
            world_name: None,
            world_id: None,
            instance_id: None,
            link_method: None,
            confidence: None,
            author: None,
        }
    }

    #[test]
    fn decode_queue_re_requesting_the_in_flight_key_drops_the_queued_key() {
        let mut queue = DecodeQueue::default();
        assert!(queue.want(key(1)));
        queue.begin();
        assert!(!queue.want(key(2)));
        assert!(!queue.want(key(1)));
        queue.finish(key(1));
        assert_eq!(queue.begin(), None);
    }

    #[test]
    fn decode_queue_pending_for_covers_queued_and_in_flight() {
        let mut queue = DecodeQueue::default();
        queue.want(key(1));
        assert!(queue.pending_for(1));
        queue.begin();
        assert!(queue.pending_for(1));
        queue.finish(key(1));
        assert!(!queue.pending_for(1));
    }

    #[test]
    fn display_key_uses_the_display_derivative() {
        let mut photo = test_photo(7);
        assert_eq!(
            display_key(&photo),
            DisplayKey {
                photo_id: 7,
                variant: 0
            }
        );
        photo.display_derivative_id = Some(42);
        assert_eq!(
            display_key(&photo),
            DisplayKey {
                photo_id: 7,
                variant: 42
            }
        );
    }

    #[test]
    fn applies_a_finished_decode_only_for_the_current_selection() {
        let key = DisplayKey {
            photo_id: 7,
            variant: 0,
        };
        let other = DisplayKey {
            photo_id: 7,
            variant: 42,
        };
        assert!(should_apply_image(Some(key), key));
        assert!(!should_apply_image(Some(other), key));
        assert!(!should_apply_image(None, key));
    }

    #[test]
    fn pending_full_image_only_applies_to_the_current_selection() {
        let selected = key(1);
        assert!(should_apply_pending_full_image(
            true,
            Some(selected),
            selected
        ));
        assert!(!should_apply_pending_full_image(
            false,
            Some(selected),
            selected
        ));
        assert!(!should_apply_pending_full_image(
            true,
            Some(key(2)),
            selected
        ));
        assert!(!should_apply_pending_full_image(true, None, selected));
    }

    #[test]
    fn thumbnail_placeholder_label_distinguishes_loading_from_failure() {
        assert_eq!(thumbnail_placeholder_label(true), "サムネイルを読み込み中…");
        assert_eq!(
            thumbnail_placeholder_label(false),
            "サムネイルを読み込めません"
        );
    }
}
