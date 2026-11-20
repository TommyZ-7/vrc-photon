//! Per-concern state groups owned by [`App`](super::App).
//!
//! Each struct gathers the fields that move together, so `App` stops being a
//! flat list of two hundred unrelated values and a reader can see which parts
//! belong to the library browser, the search screen, the daemon bridge, and
//! the settings draft.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use vrc_photon_core::ai::{PhotoAi, PhotoAiTagView};
use vrc_photon_core::model::{
    DaemonError, Derivative, LibraryStats, Photo, PhotoPlayer, PhotoTextFilter,
};

use super::CardBoundsMap;
use super::ai;
use super::lifecycle::ScanProgressView;
use super::search_suggest;
use crate::state::photo_browser::{CountDebounce, PhotoBrowser};
use crate::state::selection::PhotoSelection;
use crate::ui::gallery::GalleryView;
use vrc_photon_ipc::Endpoint;

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

/// The search screen: its own browser and card bounds, the applied filter
/// state, and the suggestion popover.
pub(crate) struct SearchState {
    pub(crate) view: Rc<RefCell<PhotoBrowser>>,
    pub(crate) gallery: Entity<GalleryView>,
    pub(crate) bounds: CardBoundsMap,
    pub(crate) bounds_epoch: Rc<Cell<u64>>,
    pub(crate) grid_focus: FocusHandle,
    pub(crate) active: bool,
    pub(crate) searching: bool,
    pub(crate) request: u64,
    pub(crate) applied: Option<PhotoTextFilter>,
    pub(crate) pending_anchor: Option<i64>,
    pub(crate) refresh_pending: bool,
    /// A tag/AI edit changed the applied search while the Search screen was
    /// hidden. Refresh once when the user returns to Search.
    pub(crate) refresh_dirty: bool,
    pub(crate) favorite: bool,
    pub(crate) days: Option<i64>,
    pub(crate) ai_tags: Vec<String>,
    pub(crate) people_input: Entity<InputState>,
    pub(crate) world_input: Entity<InputState>,
    pub(crate) player_input: Entity<InputState>,
    pub(crate) tag_input: Entity<InputState>,
    pub(crate) suggestion: Option<search_suggest::SuggestionState>,
    pub(crate) suggestion_request: u64,
    pub(crate) suggestion_bounds:
        Rc<RefCell<HashMap<search_suggest::SuggestionKind, Bounds<Pixels>>>>,
    pub(crate) _suggestion_subscriptions: Vec<Subscription>,
    pub(crate) _suggestion_keys: Subscription,
}

/// The daemon bridge: connection state, the latest status snapshot, the
/// monitor selection, and the failures worth showing to the user.
pub(crate) struct IpcState {
    pub(crate) daemon_age_secs: Option<i64>,
    pub(crate) daemon_supervisor: Option<String>,
    pub(crate) daemon_pid: Option<u32>,
    pub(crate) daemon_paused: bool,
    pub(crate) daemon_active: bool,
    pub(crate) daemon_errors: Vec<DaemonError>,
    /// Version of the last daemon status snapshot applied from the event
    /// stream. Snapshots stamped with an older version are stale (read
    /// before a state change but published after it) and must not revive
    /// state the daemon has already moved past.
    pub(crate) last_status_version: u64,
    pub(crate) endpoint: Option<Endpoint>,
    pub(crate) connected: bool,
    pub(crate) error: Option<String>,
    pub(crate) scan_progress: Option<ScanProgressView>,
    pub(crate) shutdown_armed: bool,
    pub(crate) monitor_world: Option<String>,
    pub(crate) monitor_instance: Option<String>,
    pub(crate) monitor_players: Vec<String>,
}
