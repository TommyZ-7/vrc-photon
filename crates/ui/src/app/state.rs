//! Per-concern state groups owned by [`App`](super::App).
//!
//! Each struct gathers the fields that move together, so `App` stops being a
//! flat list of two hundred unrelated values and a reader can see which parts
//! belong to the library browser, the search screen, the daemon bridge, and
//! the settings draft.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::*;
use vrc_photon_core::ai::{PhotoAi, PhotoAiTagView};
use vrc_photon_core::model::{Derivative, LibraryStats, Photo, PhotoPlayer};

use super::CardBoundsMap;
use super::ai;
use crate::state::photo_browser::{CountDebounce, PhotoBrowser};
use crate::state::selection::PhotoSelection;
use crate::ui::gallery::GalleryView;

/// The photo library browser: index, pages, selection, and the metadata the
/// inspector shows for the current selection.
pub(crate) struct LibraryState {
    pub(crate) browser: Rc<RefCell<PhotoBrowser>>,
    pub(crate) gallery: Entity<GalleryView>,
    pub(crate) bounds: CardBoundsMap,
    pub(crate) bounds_epoch: Rc<Cell<u64>>,
    pub(crate) dirty: bool,
    pub(crate) error: Option<&'static str>,
    pub(crate) truncated_notified: bool,
    pub(crate) count_debounce: CountDebounce,
    pub(crate) revision_debounce: CountDebounce,
    pub(crate) selected: Option<i64>,
    pub(crate) selected_ids: PhotoSelection,
    pub(crate) selection_anchor: Option<i64>,
    pub(crate) selected_photo: Option<Photo>,
    pub(crate) selected_players: Vec<PhotoPlayer>,
    pub(crate) selected_tags: Vec<String>,
    pub(crate) selected_ai: Option<PhotoAi>,
    pub(crate) selected_ai_tags: Vec<PhotoAiTagView>,
    pub(crate) ai_meta_revision: u64,
    pub(crate) ai_meta_cache: ai::AiMetaCache,
    pub(crate) selected_derivatives: Vec<Derivative>,
    pub(crate) compressed_ids: HashSet<i64>,
    pub(crate) duplicate_ids: HashSet<i64>,
    pub(crate) last_revision: i64,
    pub(crate) last_ai_revision: i64,
    pub(crate) last_session_revision: i64,
    pub(crate) loading: bool,
    pub(crate) pending_reload: Option<bool>,
    pub(crate) pending_initial_select: bool,
    pub(crate) stats: LibraryStats,
    pub(crate) stats_revision: i64,
}
