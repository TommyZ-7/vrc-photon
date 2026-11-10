//! Photo browser index snapshot, page cache, and query helpers.
//!
//! The `uniform_list` page loading and rendering stay in the UI layer; this
//! module owns only the index snapshot, the page cache, and the pure helpers
//! that describe scroll and anchor state.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::{ScrollStrategy, UniformListScrollHandle, point, px};
use vrc_photon_core::error::CoreError;
use vrc_photon_core::model::{Photo, PhotoIndexEntry, SessionSummary};

use crate::state::motion::SessionCollapseMotions;
use crate::state::session_rows::SessionRows;
use crate::state::year_rail::{YearBucket, build_year_buckets};
use crate::ui::year_rail::YearRailLabels;

pub(crate) const PAGE_SIZE: usize = 256;
pub(crate) const MAX_PAGES: usize = 24;

/// Scroll distance from the top of a uniform list, in pixels.
pub(crate) fn list_scroll_top(handle: &UniformListScrollHandle) -> f32 {
    let offset = handle.0.borrow().base_handle.offset().y;
    (-f32::from(offset)).max(0.0)
}

/// Scroll distance from the top of a `v_virtual_list`, in pixels.
pub(crate) fn session_scroll_top(handle: &VirtualListScrollHandle) -> f32 {
    let offset = handle.base_handle().offset().y;
    (-f32::from(offset)).max(0.0)
}

/// Sets the session list offset. The virtual list clamps it against the
/// freshly laid-out content, so an offset requested before the first session
/// render is not discarded by a zero `max_offset`.
pub(crate) fn set_session_scroll_offset(browser: &PhotoBrowser, offset: f32) {
    let y = (-offset).min(0.0);
    browser.session_scroll.set_offset(point(px(0.), px(y)));
}

/// Ordered snapshot of the library: only IDs and capture timestamps, so the
/// full 100k-entry list stays cheap while photo rows load in pages.
pub(crate) struct PhotoIndex {
    pub(crate) entries: Vec<PhotoIndexEntry>,
    pub(crate) total: usize,
    pub(crate) years: Vec<YearBucket>,
    /// Photo count per local capture month. The gallery header needs the
    /// count for the month at the top of the viewport on every scroll-driven
    /// render; scanning the 100k-entry index there was measurable work.
    months: HashMap<i32, usize>,
    /// Row of every photo id, built with the snapshot. Arrow-key repeats ask
    /// for this on every key event, so it must not scan the 100k-entry list.
    rows: HashMap<i64, usize>,
}

impl PhotoIndex {
    pub(crate) fn new(entries: Vec<PhotoIndexEntry>) -> Self {
        let total = entries.len();
        let years = build_year_buckets(&entries);
        let mut months: HashMap<i32, usize> = HashMap::new();
        let mut rows: HashMap<i64, usize> = HashMap::with_capacity(entries.len());
        for (row, entry) in entries.iter().enumerate() {
            *months.entry(entry.captured_month).or_insert(0) += 1;
            // The linear lookup this replaces returned the first occurrence,
            // so a duplicated id keeps its earliest row.
            rows.entry(entry.id).or_insert(row);
        }
        Self {
            entries,
            total,
            years,
            months,
            rows,
        }
    }

    pub(crate) fn month_count(&self, month: i32) -> usize {
        self.months.get(&month).copied().unwrap_or(0)
    }

    /// Entries between `anchor` and `target` in index order, inclusive.
    /// Returns `None` when either id is no longer in the snapshot.
    pub(crate) fn range_between(&self, anchor: i64, target: i64) -> Option<&[PhotoIndexEntry]> {
        let anchor = self.row_of_id(anchor)?;
        let target = self.row_of_id(target)?;
        let (start, end) = if anchor <= target {
            (anchor, target)
        } else {
            (target, anchor)
        };
        Some(&self.entries[start..=end])
    }

    /// Position of `id` in the ordered snapshot, if it is still present.
    pub(crate) fn row_of_id(&self, id: i64) -> Option<usize> {
        self.rows.get(&id).copied()
    }
}

/// First photo in the top visible row.
///
/// `scroll_top` is the non-negative scroll distance from the top of the list.
/// The row is floored before multiplying by the column count: halfway into
/// row 2 the top row is still row 2, not the first card of row 3.
pub(crate) fn first_visible_photo(
    scroll_top: f32,
    columns: usize,
    row_height: f32,
    total: usize,
) -> usize {
    if total == 0 {
        return 0;
    }
    let columns = columns.max(1);
    let row = (scroll_top.max(0.0) / row_height).floor() as usize;
    row.saturating_mul(columns).min(total - 1)
}

/// Month and count for the top visible row.
///
/// The gallery header follows the scroll position, not the first entry in the
/// index, so the month label always describes the cards at the top of the
/// viewport.
pub(crate) fn visible_month(index: &PhotoIndex, first_visible: usize) -> (i32, usize) {
    let Some(entry) = index.entries.get(first_visible) else {
        return (0, 0);
    };
    let month = entry.captured_month;
    (month, index.month_count(month))
}

/// Photo at the top-left of the first visible row.
///
/// `offset_y` is the non-negative scroll distance from the top of the list;
/// callers convert the scroll handle's negative offset before calling.
pub(crate) fn anchor_photo_id(
    index: &PhotoIndex,
    offset_y: f32,
    columns: usize,
    row_height: f32,
) -> Option<i64> {
    let total = index.total;
    if total == 0 {
        return None;
    }
    let first_photo = first_visible_photo(offset_y, columns, row_height, total);
    index.entries.get(first_photo).map(|entry| entry.id)
}

/// List row that keeps `id` as the first photo of the top visible row under
/// `columns`. Missing ids anchor at the top.
pub(crate) fn row_for_photo(index: &PhotoIndex, id: i64, columns: usize) -> usize {
    index
        .row_of_id(id)
        .map(|position| position / columns.max(1))
        .unwrap_or(0)
}

/// The photo the next search result should be anchored to. Keep the requested
/// anchor when it survived the query; otherwise fall back to the first photo
/// (the top of the grid) or `None` for an empty result.
pub(crate) fn search_anchor_after_result(
    requested: Option<i64>,
    index: &PhotoIndex,
) -> Option<i64> {
    requested
        .filter(|id| index.row_of_id(*id).is_some())
        .or_else(|| index.entries.first().map(|entry| entry.id))
}

/// Why a grid page failed, in a form the UI can present without leaking
/// internal paths or database error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageFailure {
    Timeout,
    ReadError,
}

impl PageFailure {
    pub(crate) fn from_core_error(error: &CoreError) -> Self {
        match error {
            CoreError::Io(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Self::Timeout
            }
            _ => Self::ReadError,
        }
    }
}

/// One failed page in the rendered range, including whether an explicit retry
/// is already in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FailedPage {
    pub(crate) page: usize,
    pub(crate) failure: PageFailure,
    pub(crate) retrying: bool,
}

/// Bounded access-order LRU cache for grid photo pages.
///
/// `get_page` moves the page to the back of the order; `insert_page` evicts
/// from the front, so a page the user scrolled back to is not displaced by a
/// backlog of page loads that finish later.
#[derive(Default)]
pub(crate) struct PhotoPageCache {
    pages: HashMap<usize, Arc<Vec<Arc<Photo>>>>,
    order: VecDeque<usize>,
    pending: HashSet<usize>,
    failed: HashMap<usize, PageFailure>,
    /// Pages whose inline retry button was pressed. Their failure marker stays
    /// in place until the reload succeeds so the panel does not blink back to
    /// placeholders and no automatic reload loop can pick them up.
    retrying: HashSet<usize>,
}

impl PhotoPageCache {
    fn new() -> Self {
        Self::default()
    }

    fn clear(&mut self) {
        self.pages.clear();
        self.order.clear();
        self.pending.clear();
        self.failed.clear();
        self.retrying.clear();
    }

    pub(crate) fn get_page(&mut self, page: usize) -> Option<Arc<Vec<Arc<Photo>>>> {
        let page_photos = self.pages.get(&page).cloned()?;
        self.touch(page);
        Some(page_photos)
    }

    fn insert_page(&mut self, page: usize, photos: Vec<Photo>) {
        self.pending.remove(&page);
        self.failed.remove(&page);
        self.retrying.remove(&page);
        let page_photos = photos.into_iter().map(Arc::new).collect::<Vec<_>>();
        self.pages.insert(page, Arc::new(page_photos));
        self.touch(page);
        while self.pages.len() > MAX_PAGES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.pages.remove(&oldest);
        }
    }

    fn touch(&mut self, page: usize) {
        if let Some(index) = self.order.iter().position(|entry| *entry == page) {
            self.order.remove(index);
        }
        self.order.push_back(page);
    }

    fn plan_pages(&mut self, start: usize, end: usize, limit: usize) -> Vec<usize> {
        if end <= start || limit == 0 {
            return Vec::new();
        }
        let first = start / PAGE_SIZE;
        let last = (end - 1) / PAGE_SIZE;
        let mut planned = Vec::new();
        for page in first..=last {
            if planned.len() >= limit {
                break;
            }
            if self.pages.contains_key(&page)
                || self.pending.contains(&page)
                || self.failed.contains_key(&page)
            {
                continue;
            }
            self.pending.insert(page);
            planned.push(page);
        }
        planned
    }

    fn mark_pending(&mut self, page: usize) {
        self.pending.insert(page);
    }

    fn fail_page(&mut self, page: usize, reason: PageFailure) {
        self.pending.remove(&page);
        self.retrying.remove(&page);
        self.failed.insert(page, reason);
    }

    pub(crate) fn is_failed(&self, page: usize) -> bool {
        self.failed.contains_key(&page)
    }

    /// Failed pages whose photo range overlaps `[start, end)`, in page order.
    pub(crate) fn failed_pages_in_range(&self, start: usize, end: usize) -> Vec<FailedPage> {
        if end <= start {
            return Vec::new();
        }
        let first = start / PAGE_SIZE;
        let last = (end - 1) / PAGE_SIZE;
        let mut failed: Vec<FailedPage> = self
            .failed
            .iter()
            .filter(|(page, _)| **page >= first && **page <= last)
            .map(|(page, reason)| FailedPage {
                page: *page,
                failure: *reason,
                retrying: self.retrying.contains(page),
            })
            .collect();
        failed.sort_unstable_by_key(|state| state.page);
        failed
    }

    /// Marks exactly one failed page for an explicit retry. The failure marker
    /// is kept until the reload reports success or failure, so the inline panel
    /// stays mounted with its loading label and `plan_pages` cannot schedule a
    /// duplicate load. Returns `false` for an unknown or already-retrying page.
    pub(crate) fn retry_page(&mut self, page: usize) -> bool {
        if !self.is_failed(page) || self.retrying.contains(&page) {
            return false;
        }
        self.retrying.insert(page);
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SessionRowsKey {
    generation: u64,
    columns: usize,
    photo_row_height: f32,
    header_height: f32,
    date_label_height: f32,
    continuation_height: f32,
    collapsed: Vec<i64>,
    animating: Vec<i64>,
}

/// Cached rows plus the frame-driver window for one session render.
pub(crate) struct SessionLayout {
    pub(crate) rows: Arc<SessionRows>,
    pub(crate) heights: Vec<f32>,
    pub(crate) driver_duration: Duration,
    pub(crate) driver_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SessionScrollAnchor {
    pub(crate) session_id: i64,
    pub(crate) viewport_y: f32,
    pub(crate) expected_offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SessionLayoutParams {
    pub(crate) columns: usize,
    pub(crate) photo_row_height: f32,
    pub(crate) header_height: f32,
    pub(crate) date_label_height: f32,
    pub(crate) continuation_height: f32,
}

/// Snapshot plus page cache for one grid (library or search).
pub(crate) struct PhotoBrowser {
    pub(crate) index: Arc<PhotoIndex>,
    pub(crate) pages: PhotoPageCache,
    pub(crate) scroll: UniformListScrollHandle,
    pub(crate) columns: usize,
    /// Row height of the current card layout, used when clamping after a
    /// removal. The gallery refreshes it from the display settings.
    pub(crate) row_height: f32,
    /// Top photo the gallery should keep anchored when the layout changes.
    pub(crate) pending_anchor: Option<i64>,
    pub(crate) generation: u64,
    pub(crate) labels_generation: u64,
    pub(crate) labels: Option<Arc<YearRailLabels>>,
    /// Shared with the mounted rail so a re-render cannot drop an in-flight
    /// drag or let it outlive the browser.
    pub(crate) rail_drag: Rc<Cell<bool>>,
    /// Whether the pointer is currently inside the rail gutter.
    pub(crate) rail_hover: Rc<Cell<bool>>,
    /// Scroll handle for the mixed header/photo rows in session mode.
    pub(crate) session_scroll: VirtualListScrollHandle,
    pub(crate) session_rows: Option<Arc<SessionRows>>,
    session_rows_key: Option<SessionRowsKey>,
    /// Row heights most recently painted by `session_layout`, including the
    /// per-frame collapse animation. The gallery header and year rail use
    /// these instead of the settled origins while rows are moving.
    session_last_heights: Vec<f32>,
    pub(crate) collapsed_sessions: HashSet<i64>,
    pub(crate) session_collapse_motions: SessionCollapseMotions,
    /// Header kept at a fixed viewport position while collapse motions settle.
    session_scroll_anchor: Option<SessionScrollAnchor>,
    /// Last layout passed to `session_layout`, used by rail jumps that need to
    /// rebuild rows after expanding a session.
    session_layout_params: Option<SessionLayoutParams>,
    /// Photo row restored after the active collapse motions settle.
    pending_photo_anchor: Option<usize>,
    /// Scroll offset written by the rail jump that set `pending_photo_anchor`.
    pending_photo_expected_offset: Option<f32>,
    pub(crate) session_summaries: HashMap<i64, Arc<SessionSummary>>,
    pub(crate) session_summary_pending: HashSet<i64>,
    pub(crate) session_summary_failed: HashSet<i64>,
}

impl PhotoBrowser {
    /// `initial_row_height` only stands in until the first grid render
    /// replaces it with the current layout's row height; the caller (the
    /// app layer) computes it so this module stays free of theme types.
    pub(crate) fn new(index: Arc<PhotoIndex>, initial_row_height: f32) -> Self {
        Self {
            index,
            pages: PhotoPageCache::new(),
            scroll: UniformListScrollHandle::new(),
            columns: 1,
            row_height: initial_row_height,
            pending_anchor: None,
            generation: 0,
            labels_generation: 0,
            labels: None,
            rail_drag: Rc::new(Cell::new(false)),
            rail_hover: Rc::new(Cell::new(false)),
            session_scroll: VirtualListScrollHandle::new(),
            session_rows: None,
            session_rows_key: None,
            session_last_heights: Vec::new(),
            collapsed_sessions: HashSet::new(),
            session_collapse_motions: SessionCollapseMotions::default(),
            session_scroll_anchor: None,
            session_layout_params: None,
            pending_photo_anchor: None,
            pending_photo_expected_offset: None,
            session_summaries: HashMap::new(),
            session_summary_pending: HashSet::new(),
            session_summary_failed: HashSet::new(),
        }
    }

    pub(crate) fn replace_index(&mut self, index: Arc<PhotoIndex>) {
        self.index = index;
        let surviving_sessions: HashSet<i64> = self
            .index
            .entries
            .iter()
            .filter_map(|entry| entry.session_id)
            .collect();
        self.collapsed_sessions
            .retain(|session_id| surviving_sessions.contains(session_id));
        self.generation = self.generation.wrapping_add(1);
        self.pages.clear();
        self.labels = None;
        self.pending_anchor = None;
        self.rail_drag.set(false);
        self.rail_hover.set(false);
        self.session_rows = None;
        self.session_rows_key = None;
        self.session_collapse_motions.clear();
        self.session_scroll_anchor = None;
        self.pending_photo_anchor = None;
        self.pending_photo_expected_offset = None;
        self.session_summaries.clear();
        self.session_summary_pending.clear();
        self.session_summary_failed.clear();
    }

    /// Builds or reuses the mixed session row model for the current index,
    /// layout, and collapsed set. The virtual list uses the returned heights
    /// directly.
    pub(crate) fn ensure_session_rows(
        &mut self,
        columns: usize,
        photo_row_height: f32,
        header_height: f32,
        date_label_height: f32,
        continuation_height: f32,
    ) -> Arc<SessionRows> {
        self.ensure_session_rows_at(
            columns,
            photo_row_height,
            header_height,
            date_label_height,
            continuation_height,
            Instant::now(),
        )
    }

    fn ensure_session_rows_at(
        &mut self,
        columns: usize,
        photo_row_height: f32,
        header_height: f32,
        date_label_height: f32,
        continuation_height: f32,
        now: Instant,
    ) -> Arc<SessionRows> {
        let animating: HashSet<i64> = self
            .session_collapse_motions
            .factors(now)
            .into_keys()
            .collect();
        self.ensure_session_rows_with_animating(
            columns,
            photo_row_height,
            header_height,
            date_label_height,
            continuation_height,
            &animating,
        )
    }

    fn ensure_session_rows_with_animating(
        &mut self,
        columns: usize,
        photo_row_height: f32,
        header_height: f32,
        date_label_height: f32,
        continuation_height: f32,
        animating: &HashSet<i64>,
    ) -> Arc<SessionRows> {
        let mut collapsed: Vec<i64> = self.collapsed_sessions.iter().copied().collect();
        collapsed.sort_unstable();
        let mut animating_ids: Vec<i64> = animating.iter().copied().collect();
        animating_ids.sort_unstable();
        let key = SessionRowsKey {
            generation: self.generation,
            columns,
            photo_row_height,
            header_height,
            date_label_height,
            continuation_height,
            collapsed,
            animating: animating_ids,
        };
        if self.session_rows_key.as_ref() == Some(&key)
            && let Some(rows) = &self.session_rows
        {
            return rows.clone();
        }
        let rows = Arc::new(SessionRows::build(
            &self.index.entries,
            columns,
            photo_row_height,
            header_height,
            date_label_height,
            continuation_height,
            &self.collapsed_sessions,
            animating,
        ));
        self.session_rows_key = Some(key);
        self.session_rows = Some(rows.clone());
        rows
    }

    /// Settled rows plus the animated heights and frame-driver window for one
    /// session render. `now` is captured once so the factors and the row model
    /// agree on the animation state.
    pub(crate) fn session_layout(
        &mut self,
        columns: usize,
        photo_row_height: f32,
        header_height: f32,
        date_label_height: f32,
        continuation_height: f32,
        now: Instant,
    ) -> SessionLayout {
        self.session_layout_params = Some(SessionLayoutParams {
            columns,
            photo_row_height,
            header_height,
            date_label_height,
            continuation_height,
        });
        let factors = self.session_collapse_motions.factors(now);
        let animating: HashSet<i64> = factors.keys().copied().collect();
        let rows = self.ensure_session_rows_with_animating(
            columns,
            photo_row_height,
            header_height,
            date_label_height,
            continuation_height,
            &animating,
        );
        let heights = rows.heights_with_collapse(&factors);
        self.session_last_heights = heights.clone();
        if animating.is_empty() {
            self.session_scroll_anchor = None;
            // A rail jump is restored once every motion has settled, using the
            // settled row origins.
            if let Some(photo_index) = self.pending_photo_anchor.take() {
                self.pending_photo_expected_offset = None;
                let offset = rows
                    .offset_for_photo(photo_index)
                    .or_else(|| rows.anchor_offset_for_photo(photo_index));
                if let Some(offset) = offset {
                    set_session_scroll_offset(self, offset);
                }
            }
        } else if let Some(anchor) = self.session_scroll_anchor {
            let current = session_scroll_top(&self.session_scroll);
            if (current - anchor.expected_offset).abs() > 1.5 {
                // A manual scroll supersedes the header anchor.
                self.session_scroll_anchor = None;
            } else if let Some(header_offset) =
                rows.offset_for_session_with_heights(anchor.session_id, &heights)
            {
                let target = (header_offset - anchor.viewport_y).max(0.0);
                set_session_scroll_offset(self, target);
                if let Some(anchor) = &mut self.session_scroll_anchor {
                    anchor.expected_offset = session_scroll_top(&self.session_scroll);
                }
            }
        }
        SessionLayout {
            rows,
            heights,
            driver_duration: self.session_collapse_motions.max_duration(),
            driver_epoch: self.session_collapse_motions.epoch(),
        }
    }

    pub(crate) fn animate_session_collapse(
        &mut self,
        session_id: i64,
        collapsed: bool,
        duration: Duration,
        now: Instant,
    ) {
        self.session_collapse_motions
            .animate(session_id, collapsed, duration, now);
    }

    pub(crate) fn settle_session_collapse(&mut self, session_id: i64) {
        self.session_collapse_motions.settle(session_id);
        if self
            .session_scroll_anchor
            .is_some_and(|anchor| anchor.session_id == session_id)
        {
            self.session_scroll_anchor = None;
        }
    }

    /// Finishes every in-flight collapse before an explicit navigation. The
    /// pending anchors are discarded because the new navigation target
    /// supersedes them.
    pub(crate) fn settle_all_session_collapses(&mut self) {
        if self.session_collapse_motions.is_empty()
            && self.session_scroll_anchor.is_none()
            && self.pending_photo_anchor.is_none()
        {
            return;
        }
        self.session_collapse_motions.clear();
        self.session_scroll_anchor = None;
        self.pending_photo_anchor = None;
        self.pending_photo_expected_offset = None;
        self.session_rows = None;
        self.session_rows_key = None;
    }

    /// Restores this photo after the active collapse motions settle. Used by
    /// the year rail, whose jump is computed from the settled row geometry.
    pub(crate) fn set_pending_photo_anchor(&mut self, photo_index: usize) {
        self.pending_photo_anchor = Some(photo_index);
        self.pending_photo_expected_offset = None;
        self.session_scroll_anchor = None;
    }

    /// Cancels anchors when the user scrolls by hand. A programmatic scroll
    /// (rail jump or header click) records its expected offset first, so only
    /// a mismatch is treated as a manual scroll.
    pub(crate) fn note_session_scroll(&mut self) {
        let top = session_scroll_top(&self.session_scroll);
        if let Some(anchor) = &self.session_scroll_anchor
            && (top - anchor.expected_offset).abs() > 1.5
        {
            self.session_scroll_anchor = None;
        }
        if let Some(expected) = self.pending_photo_expected_offset
            && (top - expected).abs() > 1.5
        {
            self.pending_photo_anchor = None;
            self.pending_photo_expected_offset = None;
        }
    }

    pub(crate) fn note_programmatic_scroll(&mut self) {
        self.pending_photo_expected_offset = Some(session_scroll_top(&self.session_scroll));
    }

    pub(crate) fn set_session_scroll_anchor(&mut self, session_id: i64, viewport_y: f32) {
        self.session_scroll_anchor = Some(SessionScrollAnchor {
            session_id,
            viewport_y,
            expected_offset: session_scroll_top(&self.session_scroll),
        });
        self.pending_photo_anchor = None;
    }

    pub(crate) fn cancel_session_scroll_anchor(&mut self) {
        self.session_scroll_anchor = None;
    }

    /// Expands a collapsed session before a rail jump measures its photo row.
    /// Returns the rebuilt rows, or `None` before the first layout.
    pub(crate) fn expand_session_for_jump(&mut self, session_id: i64) -> Option<Arc<SessionRows>> {
        if self.collapsed_sessions.remove(&session_id) {
            self.settle_session_collapse(session_id);
            self.session_rows = None;
            self.session_rows_key = None;
        }
        let params = self.session_layout_params?;
        Some(self.ensure_session_rows(
            params.columns,
            params.photo_row_height,
            params.header_height,
            params.date_label_height,
            params.continuation_height,
        ))
    }

    pub(crate) fn top_photo_index(&self, scroll_top: f32) -> Option<usize> {
        self.session_rows.as_ref().and_then(|rows| {
            if self.session_last_heights.len() == rows.heights().len() {
                rows.top_photo_index_with_heights(scroll_top, &self.session_last_heights)
            } else {
                rows.top_photo_index(scroll_top)
            }
        })
    }

    /// Marks up to `limit` uncached session IDs as pending and returns them.
    pub(crate) fn plan_session_summaries(
        &mut self,
        ids: impl IntoIterator<Item = i64>,
        limit: usize,
    ) -> Vec<i64> {
        let mut planned = Vec::new();
        for id in ids {
            if planned.len() >= limit {
                break;
            }
            if self.session_summaries.contains_key(&id)
                || self.session_summary_pending.contains(&id)
                || self.session_summary_failed.contains(&id)
                || planned.contains(&id)
            {
                continue;
            }
            planned.push(id);
        }
        for id in &planned {
            self.session_summary_pending.insert(*id);
        }
        planned
    }

    pub(crate) fn insert_session_summaries(
        &mut self,
        generation: u64,
        requested: &[i64],
        summaries: Vec<SessionSummary>,
    ) {
        if generation != self.generation {
            return;
        }
        let returned: HashSet<i64> = summaries.iter().map(|summary| summary.session.id).collect();
        for id in requested {
            self.session_summary_pending.remove(id);
            if !returned.contains(id) {
                self.session_summary_failed.insert(*id);
            }
        }
        for summary in summaries {
            self.session_summary_pending.remove(&summary.session.id);
            self.session_summary_failed.remove(&summary.session.id);
            self.session_summaries
                .insert(summary.session.id, Arc::new(summary));
        }
    }

    pub(crate) fn fail_session_summaries(&mut self, generation: u64, ids: &[i64]) {
        if generation != self.generation {
            return;
        }
        for id in ids {
            self.session_summary_pending.remove(id);
            self.session_summary_failed.insert(*id);
        }
    }

    pub(crate) fn session_summary(&self, id: i64) -> Option<Arc<SessionSummary>> {
        self.session_summaries.get(&id).cloned()
    }

    /// Drops one cached summary so a changed cover (for example after a
    /// favorite toggle) is fetched again on the next render.
    pub(crate) fn invalidate_session_summary(&mut self, session_id: i64) {
        self.session_summaries.remove(&session_id);
        self.session_summary_pending.remove(&session_id);
        self.session_summary_failed.remove(&session_id);
    }

    /// Drops every summary after a session lifecycle change. Visible headers
    /// are re-planned by the next render; the row model and scroll stay put.
    pub(crate) fn invalidate_session_summaries(&mut self) {
        self.session_summaries.clear();
        self.session_summary_pending.clear();
        self.session_summary_failed.clear();
    }

    pub(crate) fn toggle_session_collapsed(&mut self, id: i64) -> bool {
        let collapsed = if self.collapsed_sessions.remove(&id) {
            false
        } else {
            self.collapsed_sessions.insert(id);
            true
        };
        self.session_scroll_anchor = None;
        self.pending_photo_anchor = None;
        self.pending_photo_expected_offset = None;
        self.session_rows = None;
        self.session_rows_key = None;
        collapsed
    }

    /// Drops the shaped year-rail labels so the next render rebuilds them in
    /// the current language.
    pub(crate) fn invalidate_labels(&mut self) {
        self.labels = None;
    }

    /// Invalidates the decoded page cache without replacing the index. Used
    /// when photo rows changed (compression replacement, rebinds) but the set
    /// and order of ids did not.
    pub(crate) fn invalidate_pages(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pages.clear();
        self.session_rows = None;
        self.session_rows_key = None;
        self.session_summaries.clear();
        self.session_summary_pending.clear();
        self.session_summary_failed.clear();
    }

    /// Removes a batch of photos with a single index rebuild and page-cache
    /// clear. Bulk trash used to rebuild the whole snapshot once per photo.
    pub(crate) fn remove_photos(&mut self, ids: &HashSet<i64>) {
        if ids.is_empty() {
            return;
        }
        let entries: Vec<PhotoIndexEntry> = self
            .index
            .entries
            .iter()
            .filter(|entry| !ids.contains(&entry.id))
            .copied()
            .collect();
        if entries.len() == self.index.total {
            return;
        }
        self.replace_index(Arc::new(PhotoIndex::new(entries)));
        self.clamp_scroll_after_removal();
    }

    /// Removes photos and asks the next gallery render to restore `anchor`
    /// when that photo is still present. `replace_index` clears the session
    /// scroll, so deletion in session mode would otherwise jump to the top.
    pub(crate) fn remove_photos_with_anchor(&mut self, ids: &HashSet<i64>, anchor: Option<i64>) {
        let anchor = anchor.filter(|id| !ids.contains(id));
        self.remove_photos(ids);
        if self.pending_anchor.is_none() {
            self.pending_anchor = anchor;
        }
    }

    fn clamp_scroll_after_removal(&mut self) {
        let columns = self.columns.max(1);
        let rows = self.index.total.div_ceil(columns);
        if rows == 0 {
            self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
            return;
        }
        let current_row = (list_scroll_top(&self.scroll) / self.row_height).floor() as usize;
        if current_row >= rows {
            self.scroll
                .scroll_to_item_strict(rows - 1, ScrollStrategy::Nearest);
        }
    }

    pub(crate) fn photo_at(&mut self, photo_index: usize) -> Option<Arc<Photo>> {
        let page = photo_index / PAGE_SIZE;
        let offset = photo_index % PAGE_SIZE;
        self.pages.get_page(page)?.get(offset).cloned()
    }

    pub(crate) fn plan_pages(&mut self, start: usize, end: usize, limit: usize) -> Vec<usize> {
        let end = end.min(self.index.total);
        self.pages.plan_pages(start, end, limit)
    }

    pub(crate) fn insert_page(&mut self, page: usize, photos: Vec<Photo>, generation: u64) {
        if generation != self.generation {
            return;
        }
        self.pages.insert_page(page, photos);
    }

    pub(crate) fn fail_page(&mut self, page: usize, reason: PageFailure, generation: u64) {
        if generation != self.generation {
            return;
        }
        self.pages.fail_page(page, reason);
    }

    /// Marks a page as requested immediately before `load_pages` starts, so a
    /// render between an explicit retry and the page result cannot request a
    /// duplicate load.
    pub(crate) fn mark_page_pending(&mut self, page: usize) {
        self.pages.mark_pending(page);
    }
}

/// Marks one failed page as retrying for an explicit user retry.
///
/// `expected_generation` comes from the render that displayed the failure. A
/// stale render must not revive a failure recorded against a newer index/page
/// cache. The failure marker stays until [`PhotoBrowser::insert_page`] or
/// [`PhotoBrowser::fail_page`] settles the reload. Returns the generation/page
/// the caller should load.
pub(crate) fn plan_page_retry(
    browser: &mut PhotoBrowser,
    expected_generation: u64,
    page: usize,
) -> Option<PageRetry> {
    if browser.generation != expected_generation {
        return None;
    }
    let start = page.checked_mul(PAGE_SIZE)?;
    if start >= browser.index.total {
        return None;
    }
    if !browser.pages.retry_page(page) {
        return None;
    }
    Some(PageRetry {
        generation: browser.generation,
        page,
    })
}

/// A retry request that has already claimed its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PageRetry {
    pub(crate) generation: u64,
    pub(crate) page: usize,
}

/// Keeps the no-query search mirror on the current library snapshot.
///
/// Returns `true` when the mirror was replaced (and its page cache cleared).
pub(crate) fn sync_search_mirror(search: &mut PhotoBrowser, library: &PhotoBrowser) -> bool {
    if Arc::ptr_eq(&search.index, &library.index) {
        return false;
    }
    search.replace_index(library.index.clone());
    search.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
    true
}

/// Replaces the library snapshot with a fresh index result.
///
/// On failure the previously mounted snapshot, page cache, and scroll offset
/// stay untouched so a transient locked/corrupt/I-O error cannot blank the
/// visible library. Returns the user-facing error in that case.
pub(crate) fn apply_library_index(
    browser: &mut PhotoBrowser,
    result: vrc_photon_core::error::Result<Vec<PhotoIndexEntry>>,
) -> Option<&'static str> {
    match result {
        Ok(entries) => {
            browser.replace_index(Arc::new(PhotoIndex::new(entries)));
            None
        }
        Err(error) => {
            tracing::error!(%error, "photo_index failed");
            Some("photos.index_error")
        }
    }
}

/// Tracks a value that must stay unchanged for one quiet period before the
/// UI rebuilds the library index. Used for both the live photo count and
/// `library_revision` in `app::lifecycle`: a burst of imports or revisions
/// restarts the period, and a stable value releases one rebuild.
#[derive(Default)]
pub(crate) struct CountDebounce {
    pending: Option<i64>,
    since: Option<Instant>,
}

impl CountDebounce {
    pub(crate) fn observe(&mut self, count: i64, now: Instant, quiet: Duration) -> bool {
        match self.pending {
            Some(pending) if pending == count => {
                if self
                    .since
                    .is_some_and(|since| now.duration_since(since) >= quiet)
                {
                    self.reset();
                    true
                } else {
                    false
                }
            }
            _ => {
                self.pending = Some(count);
                self.since = Some(now);
                false
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        self.pending = None;
        self.since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::session_rows::SessionRowKind;
    use crate::state::test_support::{index_entry, test_photo};
    use crate::ui::theme::{GRID_GAP, library_grid_metrics};
    use crate::ui::year_rail::YearRailLabels;
    use gpui_kit::{ScrollStrategy, UniformListScrollHandle, point, px};
    use vrc_photon_core::error::CoreError;
    use vrc_photon_core::model::{Session, SessionSummary};
    use vrc_photon_core::settings::CardSize;

    fn summary(id: i64) -> SessionSummary {
        SessionSummary {
            session: Session {
                id,
                log_file: "log.txt".to_string(),
                started_at: id,
                ended_at: Some(id + 10),
                world_id: 1,
                instance_id: None,
            },
            world_name: Some("Test World".to_string()),
            world_external_id: Some("wrld_test".to_string()),
            photo_count: 1,
            players: Vec::new(),
            cover_photo: None,
        }
    }

    /// Arbitrary non-196 row height so these tests prove the maths reads the
    /// supplied layout instead of a fixed constant.
    const TEST_ROW_HEIGHT: f32 = 180.0;

    #[test]
    fn invalidate_labels_drops_cached_rail_labels() {
        let mut browser = PhotoBrowser::new(Arc::new(PhotoIndex::new(Vec::new())), TEST_ROW_HEIGHT);
        browser.labels = Some(Arc::new(YearRailLabels::empty_for_test()));
        browser.invalidate_labels();
        assert!(browser.labels.is_none());
    }

    #[test]
    fn page_cache_evicts_the_oldest_page() {
        let mut cache = PhotoPageCache::new();
        for page in 0..MAX_PAGES {
            cache.insert_page(page, vec![test_photo(page as i64)]);
        }
        cache.insert_page(MAX_PAGES, vec![test_photo(999)]);
        assert!(cache.get_page(0).is_none());
        assert!(cache.get_page(MAX_PAGES).is_some());
    }

    #[test]
    fn page_cache_keeps_recently_touched_pages() {
        let mut cache = PhotoPageCache::new();
        for page in 0..MAX_PAGES {
            cache.insert_page(page, vec![test_photo(page as i64)]);
        }
        // Touch page 0 so it is no longer the least recently used entry.
        assert!(cache.get_page(0).is_some());
        cache.insert_page(MAX_PAGES, vec![test_photo(999)]);
        assert!(cache.get_page(0).is_some());
        assert!(cache.get_page(1).is_none());
        assert!(cache.get_page(MAX_PAGES).is_some());
    }

    #[test]
    fn page_cache_plans_only_missing_pages_and_marks_them_pending() {
        let mut cache = PhotoPageCache::new();
        let first = cache.plan_pages(0, PAGE_SIZE * 3, 2);
        assert_eq!(first, vec![0, 1]);
        let second = cache.plan_pages(0, PAGE_SIZE * 3, 2);
        assert_eq!(second, vec![2]);
        cache.insert_page(0, vec![test_photo(0)]);
        assert!(cache.get_page(0).is_some());
        let third = cache.plan_pages(0, PAGE_SIZE, 2);
        assert!(third.is_empty());
    }

    #[test]
    fn failed_page_is_not_planned_again_until_its_retry_succeeds() {
        let mut cache = PhotoPageCache::new();
        cache.fail_page(0, PageFailure::Timeout);

        assert!(cache.is_failed(0));
        assert!(cache.plan_pages(0, PAGE_SIZE, 4).is_empty());
        assert!(cache.retry_page(0));
        // A double click cannot requeue the page while the first retry runs.
        assert!(!cache.retry_page(0));
        // The panel stays mounted (and `plan_pages` stays away) until the
        // reload settles.
        assert!(cache.is_failed(0));
        assert!(cache.plan_pages(0, PAGE_SIZE, 4).is_empty());
        assert_eq!(
            cache.failed_pages_in_range(0, PAGE_SIZE),
            vec![FailedPage {
                page: 0,
                failure: PageFailure::Timeout,
                retrying: true,
            }]
        );

        cache.insert_page(0, vec![test_photo(0)]);
        assert!(!cache.is_failed(0));
        assert!(cache.get_page(0).is_some());
        assert!(cache.plan_pages(0, PAGE_SIZE, 4).is_empty());
        assert!(cache.failed_pages_in_range(0, PAGE_SIZE).is_empty());
    }

    #[test]
    fn a_failed_retry_keeps_the_panel_and_reenables_the_action() {
        let mut cache = PhotoPageCache::new();
        cache.fail_page(0, PageFailure::Timeout);
        assert!(cache.retry_page(0));

        cache.fail_page(0, PageFailure::ReadError);

        assert_eq!(
            cache.failed_pages_in_range(0, PAGE_SIZE),
            vec![FailedPage {
                page: 0,
                failure: PageFailure::ReadError,
                retrying: false,
            }]
        );
        assert!(cache.retry_page(0), "a failed retry can be retried again");
    }

    #[test]
    fn retrying_one_page_leaves_other_failures_and_reports_their_range() {
        let mut cache = PhotoPageCache::new();
        cache.fail_page(1, PageFailure::Timeout);
        cache.fail_page(3, PageFailure::ReadError);

        assert_eq!(
            cache.failed_pages_in_range(0, PAGE_SIZE * 2),
            vec![FailedPage {
                page: 1,
                failure: PageFailure::Timeout,
                retrying: false,
            }]
        );
        assert_eq!(
            cache.failed_pages_in_range(PAGE_SIZE, PAGE_SIZE * 4),
            vec![
                FailedPage {
                    page: 1,
                    failure: PageFailure::Timeout,
                    retrying: false,
                },
                FailedPage {
                    page: 3,
                    failure: PageFailure::ReadError,
                    retrying: false,
                },
            ]
        );
        assert!(cache.failed_pages_in_range(0, 0).is_empty());
        assert!(
            cache
                .failed_pages_in_range(PAGE_SIZE * 4, PAGE_SIZE * 5)
                .is_empty()
        );

        assert!(cache.retry_page(1));
        assert!(cache.is_failed(1));
        assert!(cache.is_failed(3));
        assert_eq!(
            cache.failed_pages_in_range(0, PAGE_SIZE * 2),
            vec![FailedPage {
                page: 1,
                failure: PageFailure::Timeout,
                retrying: true,
            }]
        );

        cache.insert_page(1, vec![test_photo(1)]);
        assert!(!cache.is_failed(1));
        assert_eq!(
            cache.failed_pages_in_range(0, PAGE_SIZE * 4),
            vec![FailedPage {
                page: 3,
                failure: PageFailure::ReadError,
                retrying: false,
            }]
        );
    }

    #[test]
    fn a_successful_page_insert_clears_its_failure_and_retry_state() {
        let mut cache = PhotoPageCache::new();
        cache.fail_page(2, PageFailure::ReadError);
        assert!(cache.retry_page(2));
        assert!(cache.is_failed(2));

        cache.insert_page(2, vec![test_photo(2)]);
        assert!(!cache.is_failed(2));
        assert!(cache.failed_pages_in_range(0, PAGE_SIZE * 3).is_empty());
        assert!(cache.get_page(2).is_some());
    }

    #[test]
    fn page_failure_classifies_timeouts_without_leaking_error_text() {
        let timeout = CoreError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "private path /home/user/secret.db",
        ));
        assert_eq!(PageFailure::from_core_error(&timeout), PageFailure::Timeout);

        let read_error = CoreError::Invalid("/home/user/secret.db is corrupt".to_string());
        assert_eq!(
            PageFailure::from_core_error(&read_error),
            PageFailure::ReadError
        );
    }

    #[test]
    fn retry_plan_ignores_a_stale_generation_and_keeps_other_pages_failed() {
        let entries = (1..=PAGE_SIZE as i64 * 3)
            .map(|id| index_entry(id, 202601))
            .collect();
        let mut browser = PhotoBrowser::new(Arc::new(PhotoIndex::new(entries)), TEST_ROW_HEIGHT);
        let generation = browser.generation;
        browser.fail_page(1, PageFailure::Timeout, generation);
        browser.fail_page(2, PageFailure::ReadError, generation);

        assert_eq!(
            plan_page_retry(&mut browser, generation.wrapping_add(1), 1),
            None
        );
        assert!(browser.pages.is_failed(1));

        assert_eq!(
            plan_page_retry(&mut browser, generation, 1),
            Some(PageRetry {
                generation,
                page: 1
            })
        );
        assert!(browser.pages.is_failed(1));
        assert_eq!(
            browser
                .pages
                .failed_pages_in_range(PAGE_SIZE, PAGE_SIZE + 1),
            vec![FailedPage {
                page: 1,
                failure: PageFailure::Timeout,
                retrying: true,
            }]
        );
        assert!(browser.pages.is_failed(2));
        assert_eq!(
            plan_page_retry(&mut browser, generation, 1),
            None,
            "a page that is already retrying must not be requested twice"
        );

        // A successful retry clears that page and leaves the other failure.
        browser.insert_page(1, vec![test_photo(1)], generation);
        assert!(!browser.pages.is_failed(1));
        assert!(browser.pages.is_failed(2));

        // A page beyond the snapshot has no photos to load.
        assert_eq!(plan_page_retry(&mut browser, generation, 99), None);
        assert_eq!(
            plan_page_retry(&mut browser, generation, 0),
            None,
            "a page without a failure is not a retry target"
        );
    }

    #[test]
    fn row_of_id_tracks_rebuilds_and_photo_removal() {
        let mut browser = PhotoBrowser::new(
            Arc::new(PhotoIndex::new(vec![
                index_entry(10, 202601),
                index_entry(11, 202512),
                index_entry(12, 202511),
            ])),
            TEST_ROW_HEIGHT,
        );
        assert_eq!(browser.index.row_of_id(11), Some(1));

        browser.remove_photos(&HashSet::from([11]));
        assert_eq!(browser.index.row_of_id(11), None);
        assert_eq!(browser.index.row_of_id(12), Some(1));

        browser.replace_index(Arc::new(PhotoIndex::new(vec![
            index_entry(12, 202511),
            index_entry(10, 202601),
        ])));
        assert_eq!(browser.index.row_of_id(12), Some(0));
        assert_eq!(browser.index.row_of_id(10), Some(1));
        assert_eq!(browser.index.row_of_id(11), None);
    }

    #[test]
    fn row_of_id_keeps_the_first_row_for_a_duplicated_id() {
        let index = PhotoIndex::new(vec![index_entry(7, 202601), index_entry(7, 202512)]);
        assert_eq!(index.row_of_id(7), Some(0));
    }

    #[test]
    fn count_debounce_waits_for_a_quiet_period() {
        let mut debounce = CountDebounce::default();
        let quiet = Duration::from_secs(2);
        let start = Instant::now();
        assert!(!debounce.observe(100, start, quiet));
        // The three-second tick re-observes the same count after the quiet
        // period and releases the rebuild.
        assert!(debounce.observe(100, start + Duration::from_secs(3), quiet));
        // A successful reload starts a fresh period.
        assert!(!debounce.observe(101, start + Duration::from_secs(6), quiet));
    }

    #[test]
    fn count_debounce_restarts_while_imports_keep_arriving() {
        let mut debounce = CountDebounce::default();
        let quiet = Duration::from_secs(2);
        let start = Instant::now();
        assert!(!debounce.observe(100, start, quiet));
        assert!(!debounce.observe(101, start + Duration::from_secs(3), quiet));
        assert!(!debounce.observe(102, start + Duration::from_secs(6), quiet));
        // Only after the count stays at 102 for a full tick does it reload.
        assert!(debounce.observe(102, start + Duration::from_secs(9), quiet));
    }

    #[test]
    fn revision_debounce_coalesces_a_burst() {
        let mut debounce = CountDebounce::default();
        let quiet = Duration::from_secs(2);
        let start = Instant::now();
        assert!(!debounce.observe(10, start, quiet));
        assert!(!debounce.observe(11, start + Duration::from_secs(3), quiet));
        assert!(!debounce.observe(12, start + Duration::from_secs(6), quiet));
        assert!(debounce.observe(12, start + Duration::from_secs(9), quiet));
        assert!(!debounce.observe(13, start + Duration::from_secs(12), quiet));
    }

    #[test]
    fn stale_generation_cannot_insert_or_fail_pages() {
        let mut browser = PhotoBrowser::new(
            Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)])),
            TEST_ROW_HEIGHT,
        );
        let stale = browser.generation;
        browser.replace_index(Arc::new(PhotoIndex::new(vec![index_entry(2, 202601)])));

        browser.insert_page(0, vec![test_photo(1)], stale);
        assert!(browser.pages.get_page(0).is_none());
        browser.fail_page(0, PageFailure::ReadError, stale);
        assert!(!browser.pages.is_failed(0));

        let current = browser.generation;
        browser.fail_page(0, PageFailure::ReadError, current);
        assert!(browser.pages.is_failed(0));
        browser.insert_page(0, vec![test_photo(2)], current);
        assert!(browser.pages.get_page(0).is_some());
        assert!(!browser.pages.is_failed(0));
    }

    #[test]
    fn failed_library_index_keeps_the_mounted_snapshot_and_pages() {
        let mut browser = PhotoBrowser::new(
            Arc::new(PhotoIndex::new(vec![
                index_entry(1, 202601),
                index_entry(2, 202512),
            ])),
            TEST_ROW_HEIGHT,
        );
        let generation = browser.generation;
        browser.insert_page(0, vec![test_photo(1)], generation);
        let mounted = browser.index.clone();

        let error = apply_library_index(
            &mut browser,
            Err(CoreError::Invalid("database is locked".to_string())),
        );

        assert_eq!(error, Some("photos.index_error"));
        assert!(Arc::ptr_eq(&browser.index, &mounted));
        assert_eq!(browser.index.total, 2);
        assert!(browser.pages.get_page(0).is_some());
    }

    #[test]
    fn fresh_library_index_replaces_the_snapshot() {
        let mut browser = PhotoBrowser::new(
            Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)])),
            TEST_ROW_HEIGHT,
        );
        let generation = browser.generation;
        browser.insert_page(0, vec![test_photo(1)], generation);

        let error = apply_library_index(
            &mut browser,
            Ok(vec![index_entry(2, 202512), index_entry(3, 202511)]),
        );

        assert!(error.is_none());
        assert_eq!(browser.index.total, 2);
        assert_eq!(browser.index.row_of_id(2), Some(0));
        assert!(browser.pages.get_page(0).is_none());
    }

    #[test]
    fn search_mirror_refreshes_only_when_the_library_snapshot_changed() {
        let library = PhotoBrowser::new(
            Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)])),
            TEST_ROW_HEIGHT,
        );
        let mut search = PhotoBrowser::new(library.index.clone(), TEST_ROW_HEIGHT);
        let generation = search.generation;
        search.insert_page(0, vec![test_photo(1)], generation);

        assert!(!sync_search_mirror(&mut search, &library));
        assert!(search.pages.get_page(0).is_some());

        let updated = PhotoBrowser::new(
            Arc::new(PhotoIndex::new(vec![
                index_entry(2, 202601),
                index_entry(1, 202601),
            ])),
            TEST_ROW_HEIGHT,
        );
        assert!(sync_search_mirror(&mut search, &updated));
        assert!(Arc::ptr_eq(&search.index, &updated.index));
        assert_eq!(search.index.total, 2);
        assert!(search.pages.get_page(0).is_none());
    }

    #[test]
    fn anchor_photo_id_uses_the_first_visible_row() {
        let entries = vec![
            index_entry(1, 202601),
            index_entry(2, 202512),
            index_entry(3, 202411),
        ];
        let index = PhotoIndex::new(entries);
        assert_eq!(anchor_photo_id(&index, 0.0, 3, TEST_ROW_HEIGHT), Some(1));
        assert_eq!(
            anchor_photo_id(&index, TEST_ROW_HEIGHT + 1.0, 1, TEST_ROW_HEIGHT),
            Some(2)
        );
        assert_eq!(
            anchor_photo_id(&index, TEST_ROW_HEIGHT * 2.0 + 1.0, 1, TEST_ROW_HEIGHT),
            Some(3)
        );
    }

    #[test]
    fn row_for_photo_keeps_an_old_layout_anchor_on_the_new_grid() {
        let index = PhotoIndex::new((1..=12).map(|id| index_entry(id, 202601)).collect());
        // Under the old two-column layout the top visible row starts at id 7.
        let old_anchor = anchor_photo_id(&index, 3.0 * 100.0, 2, 100.0);
        assert_eq!(old_anchor, Some(7));

        // After the metrics change (three columns) the same photo must still
        // be the first card of the anchored row.
        let row = row_for_photo(&index, old_anchor.expect("anchor"), 3);
        assert_eq!(row, 2);
        assert_eq!(index.entries[row * 3].id, 7);

        // A removed photo falls back to the top instead of a stale row.
        assert_eq!(row_for_photo(&index, 999, 3), 0);
    }

    #[test]
    fn anchor_photo_id_handles_empty_and_overscrolled_indexes() {
        assert_eq!(
            anchor_photo_id(&PhotoIndex::new(Vec::new()), 0.0, 3, TEST_ROW_HEIGHT),
            None
        );
        let index = PhotoIndex::new(vec![index_entry(1, 202601), index_entry(2, 202512)]);
        assert_eq!(
            anchor_photo_id(&index, TEST_ROW_HEIGHT * 50.0, 4, TEST_ROW_HEIGHT),
            Some(2)
        );
    }

    #[test]
    fn first_visible_photo_uses_the_3_2_library_row_height() {
        let metrics =
            library_grid_metrics(1280.0, false, false, CardSize::Medium, true, true, GRID_GAP);
        assert_eq!(metrics.columns, 3);
        assert_eq!(
            first_visible_photo(
                metrics.row_height * 2.5,
                metrics.columns,
                metrics.row_height,
                100,
            ),
            6
        );
        assert_eq!(
            first_visible_photo(0.0, metrics.columns, metrics.row_height, 100),
            0
        );
    }

    #[test]
    fn first_visible_photo_floors_the_row_before_applying_columns() {
        // Halfway into row 2 with 3 columns: the top row starts at entry 6.
        // Truncating after multiplying (`2.5 * 3 = 7.5`) would pick entry 7.
        assert_eq!(
            first_visible_photo(TEST_ROW_HEIGHT * 2.5, 3, TEST_ROW_HEIGHT, 100),
            6
        );
        assert_eq!(first_visible_photo(0.0, 3, TEST_ROW_HEIGHT, 100), 0);
        assert_eq!(first_visible_photo(-20.0, 3, TEST_ROW_HEIGHT, 100), 0);
        assert_eq!(
            first_visible_photo(TEST_ROW_HEIGHT * 50.0, 3, TEST_ROW_HEIGHT, 10),
            9
        );
        assert_eq!(first_visible_photo(0.0, 3, TEST_ROW_HEIGHT, 0), 0);
    }

    #[test]
    fn visible_month_follows_the_first_visible_row() {
        let index = PhotoIndex::new(vec![
            index_entry(1, 202610),
            index_entry(2, 202610),
            index_entry(3, 202609),
            index_entry(4, 202609),
        ]);
        assert_eq!(index.month_count(202610), 2);
        assert_eq!(index.month_count(202609), 2);
        assert_eq!(index.month_count(202501), 0);
        assert_eq!(visible_month(&index, 0), (202610, 2));
        assert_eq!(visible_month(&index, 2), (202609, 2));
        assert_eq!(visible_month(&index, 3), (202609, 2));
        assert_eq!(visible_month(&index, 99), (0, 0));
    }

    #[test]
    fn range_between_returns_entries_in_index_order() {
        let index = PhotoIndex::new((1..=5).map(|id| index_entry(id, 202601)).collect());
        let range = index.range_between(4, 2).expect("range");
        assert_eq!(
            range.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert!(index.range_between(4, 99).is_none());
    }

    #[test]
    fn anchor_row_remaps_after_an_insertion_before_it() {
        let original = PhotoIndex::new(vec![index_entry(10, 202601), index_entry(11, 202512)]);
        assert_eq!(original.row_of_id(11), Some(1));
        let updated = PhotoIndex::new(vec![
            index_entry(12, 202601),
            index_entry(10, 202601),
            index_entry(11, 202512),
        ]);
        assert_eq!(updated.row_of_id(11), Some(2));
        assert_eq!(
            anchor_photo_id(&updated, TEST_ROW_HEIGHT * 2.0 + 1.0, 1, TEST_ROW_HEIGHT),
            Some(11)
        );
    }

    #[test]
    fn removing_a_photo_clears_pages_and_rebuilds_year_counts() {
        let entries = vec![
            index_entry(1, 202601),
            index_entry(2, 202601),
            index_entry(3, 202412),
        ];
        let mut browser = PhotoBrowser::new(Arc::new(PhotoIndex::new(entries)), TEST_ROW_HEIGHT);
        let generation = browser.generation;
        browser.insert_page(0, vec![test_photo(1)], generation);
        browser.remove_photos(&HashSet::from([2]));
        assert_eq!(browser.index.total, 2);
        assert_eq!(browser.index.years[0].count, 1);
        assert_eq!(browser.index.years[1].count, 1);
        assert!(browser.pages.get_page(0).is_none());
        assert_eq!(browser.index.row_of_id(1), Some(0));
        assert_eq!(browser.index.row_of_id(2), None);
        // Removing an id that is already gone is a no-op.
        browser.remove_photos(&HashSet::from([2]));
        assert_eq!(browser.index.total, 2);
    }

    #[test]
    fn invalidating_pages_keeps_the_index_and_rejects_stale_loads() {
        let mut browser = PhotoBrowser::new(
            Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)])),
            TEST_ROW_HEIGHT,
        );
        let generation = browser.generation;
        browser.insert_page(0, vec![test_photo(1)], generation);
        assert!(browser.pages.get_page(0).is_some());

        browser.invalidate_pages();
        assert_eq!(browser.index.total, 1);
        assert!(browser.pages.get_page(0).is_none());
        assert_ne!(browser.generation, generation);

        // A page load that started before the invalidation must not resurrect.
        browser.insert_page(0, vec![test_photo(1)], generation);
        assert!(browser.pages.get_page(0).is_none());
    }

    #[test]
    fn removing_several_photos_rebuilds_the_index_once() {
        let entries = vec![
            index_entry(1, 202601),
            index_entry(2, 202601),
            index_entry(3, 202512),
            index_entry(4, 202512),
        ];
        let mut browser = PhotoBrowser::new(Arc::new(PhotoIndex::new(entries)), TEST_ROW_HEIGHT);
        let generation = browser.generation;
        browser.insert_page(0, vec![test_photo(1)], generation);

        let removed: HashSet<i64> = [1, 3].into_iter().collect();
        browser.remove_photos(&removed);

        assert_eq!(browser.index.total, 2);
        assert_eq!(browser.index.row_of_id(2), Some(0));
        assert_eq!(browser.index.row_of_id(4), Some(1));
        assert_eq!(browser.index.month_count(202601), 1);
        assert_eq!(browser.index.month_count(202512), 1);
        assert!(browser.pages.get_page(0).is_none());

        // Removing ids that are already gone is a no-op.
        browser.remove_photos(&removed);
        assert_eq!(browser.index.total, 2);
    }

    #[test]
    fn removing_photos_keeps_a_surviving_anchor_and_drops_a_removed_one() {
        let entries = vec![
            index_entry(1, 202601),
            index_entry(2, 202601),
            index_entry(3, 202512),
        ];
        let mut browser = PhotoBrowser::new(Arc::new(PhotoIndex::new(entries)), TEST_ROW_HEIGHT);
        browser.remove_photos_with_anchor(&HashSet::from([1]), Some(2));
        assert_eq!(browser.pending_anchor, Some(2));

        let entries = vec![index_entry(1, 202601), index_entry(2, 202601)];
        let mut browser = PhotoBrowser::new(Arc::new(PhotoIndex::new(entries)), TEST_ROW_HEIGHT);
        browser.remove_photos_with_anchor(&HashSet::from([1]), Some(1));
        assert_eq!(browser.pending_anchor, None);
    }

    #[test]
    fn removing_a_photo_clamps_a_scroll_past_the_last_row() {
        let entries = (1..=10).map(|id| index_entry(id, 202601)).collect();
        let mut browser = PhotoBrowser::new(Arc::new(PhotoIndex::new(entries)), TEST_ROW_HEIGHT);
        browser.columns = 2;
        browser
            .scroll
            .0
            .borrow_mut()
            .base_handle
            .set_offset(point(px(0.), px(-TEST_ROW_HEIGHT * 10.0)));
        browser.remove_photos(&HashSet::from([1]));

        let state = browser.scroll.0.borrow();
        let deferred = state.deferred_scroll_to_item.expect("scroll clamp");
        // 9 photos over 2 columns is 5 rows; the last row index is 4.
        assert_eq!(deferred.item_index, 4);
        assert_eq!(deferred.strategy, ScrollStrategy::Nearest);
    }

    #[test]
    fn list_scroll_top_reads_the_uniform_list_offset() {
        let handle = UniformListScrollHandle::new();
        handle
            .0
            .borrow_mut()
            .base_handle
            .set_offset(point(px(0.), px(-392.)));
        assert_eq!(list_scroll_top(&handle), 392.0);
        handle
            .0
            .borrow_mut()
            .base_handle
            .set_offset(point(px(0.), px(12.)));
        assert_eq!(list_scroll_top(&handle), 0.0);
    }

    #[test]
    fn search_anchor_after_result_keeps_existing_or_falls_back_to_first() {
        let index = PhotoIndex::new(vec![index_entry(1, 202601), index_entry(2, 202512)]);
        assert_eq!(search_anchor_after_result(Some(2), &index), Some(2));
        assert_eq!(search_anchor_after_result(Some(9), &index), Some(1));
        assert_eq!(search_anchor_after_result(None, &index), Some(1));
        assert_eq!(
            search_anchor_after_result(Some(1), &PhotoIndex::new(Vec::new())),
            None
        );
    }

    #[test]
    fn session_offset_is_not_clamped_before_the_first_layout() {
        let index = Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)]));
        let browser = PhotoBrowser::new(index, TEST_ROW_HEIGHT);
        assert_eq!(browser.session_scroll.base_handle().max_offset().y, px(0.));
        set_session_scroll_offset(&browser, 500.0);
        assert_eq!(session_scroll_top(&browser.session_scroll), 500.0);
    }

    #[test]
    fn session_rows_rebuild_only_when_the_key_changes() {
        let index = Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)]));
        let mut browser = PhotoBrowser::new(index, TEST_ROW_HEIGHT);
        let first = browser.ensure_session_rows(4, 200.0, 92.0, 46.0, 44.0);
        let second = browser.ensure_session_rows(4, 200.0, 92.0, 46.0, 44.0);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(browser.top_photo_index(0.0), Some(0));
        let changed = browser.ensure_session_rows(3, 200.0, 92.0, 46.0, 44.0);
        assert!(!Arc::ptr_eq(&first, &changed));
        browser.toggle_session_collapsed(7);
        let collapsed = browser.ensure_session_rows(3, 200.0, 92.0, 46.0, 44.0);
        assert!(!Arc::ptr_eq(&changed, &collapsed));
    }

    #[test]
    fn session_summaries_ignore_stale_generations_and_do_not_replan() {
        let index = Arc::new(PhotoIndex::new(Vec::new()));
        let mut browser = PhotoBrowser::new(index, TEST_ROW_HEIGHT);
        let generation = browser.generation;
        let planned = browser.plan_session_summaries([7, 7, 8], 2);
        assert_eq!(planned, vec![7, 8]);
        assert!(browser.plan_session_summaries([7, 8], 2).is_empty());
        browser.insert_session_summaries(generation + 1, &[7], vec![summary(7)]);
        assert!(browser.session_summary(7).is_none());
        browser.insert_session_summaries(generation, &[7], vec![summary(7)]);
        assert!(browser.session_summary(7).is_some());
        assert_eq!(browser.plan_session_summaries([9], 1), vec![9]);
        browser.insert_session_summaries(generation, &[9], Vec::new());
        assert!(browser.session_summary_failed.contains(&9));
        assert!(!browser.session_summary_pending.contains(&9));
        browser.fail_session_summaries(generation, &[8]);
        assert!(browser.session_summary_failed.contains(&8));
        assert!(!browser.session_summary_pending.contains(&8));
        assert!(browser.plan_session_summaries([8], 2).is_empty());
    }

    #[test]
    fn invalidating_one_session_summary_allows_a_reload() {
        let index = Arc::new(PhotoIndex::new(Vec::new()));
        let mut browser = PhotoBrowser::new(index, TEST_ROW_HEIGHT);
        let generation = browser.generation;
        browser.insert_session_summaries(generation, &[7], vec![summary(7)]);
        assert!(browser.session_summary(7).is_some());
        browser.invalidate_session_summary(7);
        assert!(browser.session_summary(7).is_none());
        assert_eq!(browser.plan_session_summaries([7], 1), vec![7]);
    }

    #[test]
    fn invalidating_pages_clears_session_summary_state() {
        let index = Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)]));
        let mut browser = PhotoBrowser::new(index, TEST_ROW_HEIGHT);
        let generation = browser.generation;
        browser.insert_session_summaries(generation, &[7], vec![summary(7)]);
        browser.plan_session_summaries([8], 1);
        browser.ensure_session_rows(4, 200.0, 92.0, 46.0, 44.0);
        browser.invalidate_pages();
        assert!(browser.session_summaries.is_empty());
        assert!(browser.session_summary_pending.is_empty());
        assert!(browser.session_summary_failed.is_empty());
        assert!(browser.session_rows.is_none());
    }

    #[test]
    fn replace_index_keeps_collapsed_sessions_that_still_have_photos() {
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        browser.toggle_session_collapsed(7);
        browser.toggle_session_collapsed(8);

        browser.replace_index(Arc::new(PhotoIndex::new(vec![
            index_entry_with_session(1, 202601, Some(7)),
            index_entry_with_session(2, 202601, Some(7)),
            index_entry_with_session(9, 202511, Some(9)),
        ])));

        assert!(browser.collapsed_sessions.contains(&7));
        assert!(!browser.collapsed_sessions.contains(&8));
        assert!(!browser.collapsed_sessions.contains(&9));
    }

    #[test]
    fn replace_index_drops_collapsed_sessions_without_visible_photos() {
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        browser.toggle_session_collapsed(7);
        browser.replace_index(Arc::new(PhotoIndex::new(vec![index_entry_with_session(
            3,
            202512,
            Some(8),
        )])));
        assert!(browser.collapsed_sessions.is_empty());
    }

    #[test]
    fn replace_index_rebuilds_session_rows_and_summaries() {
        let index = Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)]));
        let mut browser = PhotoBrowser::new(index, TEST_ROW_HEIGHT);
        let generation = browser.generation;
        browser.toggle_session_collapsed(7);
        browser.ensure_session_rows(4, 200.0, 92.0, 46.0, 44.0);
        browser.insert_session_summaries(generation, &[7], vec![summary(7)]);
        browser.plan_session_summaries([8], 1);

        browser.replace_index(Arc::new(PhotoIndex::new(vec![index_entry_with_session(
            1,
            202601,
            Some(7),
        )])));

        assert!(browser.collapsed_sessions.contains(&7));
        assert!(browser.session_rows.is_none());
        assert!(browser.session_summaries.is_empty());
        assert!(browser.session_summary_pending.is_empty());
        assert!(browser.session_summary_failed.is_empty());
    }

    fn index_entry_with_session(id: i64, month: i32, session_id: Option<i64>) -> PhotoIndexEntry {
        let mut entry = index_entry(id, month);
        entry.session_id = session_id;
        entry
    }

    fn session_index() -> Arc<PhotoIndex> {
        Arc::new(PhotoIndex::new(vec![
            index_entry_with_session(1, 202601, Some(7)),
            index_entry_with_session(2, 202601, Some(7)),
            index_entry_with_session(3, 202512, Some(8)),
        ]))
    }

    #[test]
    fn session_layout_scales_the_animating_session_heights() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        browser.animate_session_collapse(7, true, Duration::from_millis(100), now);

        let layout =
            browser.session_layout(4, 200.0, 92.0, 46.0, 44.0, now + Duration::from_millis(50));
        assert_eq!(layout.rows.rows().len(), 4);
        assert!(layout.heights[1] > 0.0 && layout.heights[1] < 200.0);
        assert_eq!(layout.heights[0], 92.0);
        assert_eq!(layout.driver_duration, Duration::from_millis(100));
        assert_eq!(layout.driver_epoch, 1);
    }

    #[test]
    fn session_rows_rebuild_when_the_animating_set_changes() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        let settled = browser.ensure_session_rows(4, 200.0, 92.0, 46.0, 44.0);
        browser.animate_session_collapse(7, true, Duration::from_millis(100), now);
        let animating = browser.ensure_session_rows(4, 200.0, 92.0, 46.0, 44.0);
        assert!(!Arc::ptr_eq(&settled, &animating));
        assert_eq!(animating.rows().len(), 4);
    }

    #[test]
    fn replacing_the_browser_drops_collapse_motions() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        browser.animate_session_collapse(7, true, Duration::from_millis(100), now);
        browser.replace_index(session_index());
        assert!(browser.session_collapse_motions.is_empty());
    }

    #[test]
    fn settling_a_collapse_motion_keeps_the_collapsed_target() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        assert!(browser.toggle_session_collapsed(7));
        browser.animate_session_collapse(7, true, Duration::from_millis(100), now);
        browser.settle_session_collapse(7);
        assert!(browser.collapsed_sessions.contains(&7));
        let rows = browser.ensure_session_rows(4, 200.0, 92.0, 46.0, 44.0);
        assert!(!rows.rows().iter().any(|row| matches!(
            row.kind(),
            SessionRowKind::Photos {
                first_photo: 0..=1,
                ..
            }
        )));
    }

    #[test]
    fn a_header_anchor_keeps_its_viewport_y_while_another_session_moves() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        let layout = browser.session_layout(4, 200.0, 92.0, 46.0, 44.0, now);
        let header_offset = layout.rows.offset_for_session(8).unwrap();
        set_session_scroll_offset(&browser, header_offset - 100.0);
        browser.set_session_scroll_anchor(8, 100.0);
        browser.animate_session_collapse(7, true, Duration::from_millis(100), now);

        let mid =
            browser.session_layout(4, 200.0, 92.0, 46.0, 44.0, now + Duration::from_millis(50));
        let expected = mid
            .rows
            .offset_for_session_with_heights(8, &mid.heights)
            .unwrap()
            - 100.0;
        assert_eq!(
            session_scroll_top(&browser.session_scroll),
            expected.max(0.0)
        );
    }

    #[test]
    fn session_layout_reanchors_a_pending_photo_after_expiry() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        assert!(browser.toggle_session_collapsed(7));
        browser.animate_session_collapse(7, true, Duration::from_millis(100), now);
        browser.set_pending_photo_anchor(2);

        let mid =
            browser.session_layout(4, 200.0, 92.0, 46.0, 44.0, now + Duration::from_millis(50));
        assert_eq!(mid.rows.rows().len(), 4);
        assert_eq!(session_scroll_top(&browser.session_scroll), 0.0);

        let done =
            browser.session_layout(4, 200.0, 92.0, 46.0, 44.0, now + Duration::from_millis(100));
        assert_eq!(done.rows.rows().len(), 3);
        let expected = done.rows.offset_for_photo(2).unwrap();
        assert_eq!(session_scroll_top(&browser.session_scroll), expected);
        assert!(browser.pending_photo_anchor.is_none());
    }

    #[test]
    fn scrolling_by_hand_cancels_the_pending_photo_anchor() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        browser.animate_session_collapse(7, true, Duration::from_millis(100), now);
        browser.set_pending_photo_anchor(2);
        set_session_scroll_offset(&browser, 0.0);
        browser.note_programmatic_scroll();
        set_session_scroll_offset(&browser, 120.0);
        browser.note_session_scroll();
        assert!(browser.pending_photo_anchor.is_none());
        assert!(browser.pending_photo_expected_offset.is_none());
    }

    #[test]
    fn a_manual_scroll_cancels_the_header_anchor() {
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        browser.set_session_scroll_anchor(8, 100.0);
        set_session_scroll_offset(&browser, 50.0);
        browser.note_session_scroll();
        assert!(browser.session_scroll_anchor.is_none());
    }

    #[test]
    fn a_rail_jump_expands_a_collapsed_session_before_measuring() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        browser.toggle_session_collapsed(7);
        let _ = browser.session_layout(4, 200.0, 92.0, 46.0, 44.0, now);
        let rows = browser.expand_session_for_jump(7).unwrap();
        assert!(!browser.collapsed_sessions.contains(&7));
        assert!(rows.offset_for_photo(0).is_some());
    }

    #[test]
    fn session_layout_reuses_rows_while_the_animating_set_is_stable() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        browser.animate_session_collapse(7, true, Duration::from_millis(100), now);
        let first =
            browser.session_layout(4, 200.0, 92.0, 46.0, 44.0, now + Duration::from_millis(10));
        let second =
            browser.session_layout(4, 200.0, 92.0, 46.0, 44.0, now + Duration::from_millis(60));
        assert!(Arc::ptr_eq(&first.rows, &second.rows));
        assert_ne!(first.heights, second.heights);
    }

    #[test]
    fn settling_all_motions_drops_rows_and_pending_anchor() {
        let now = Instant::now();
        let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
        assert!(browser.toggle_session_collapsed(7));
        browser.animate_session_collapse(7, true, Duration::from_millis(100), now);
        browser.set_session_scroll_anchor(8, 100.0);
        let animating = browser.ensure_session_rows(4, 200.0, 92.0, 46.0, 44.0);
        assert_eq!(animating.rows().len(), 4);
        browser.settle_all_session_collapses();
        assert!(browser.session_scroll_anchor.is_none());
        let settled = browser.ensure_session_rows(4, 200.0, 92.0, 46.0, 44.0);
        assert!(!Arc::ptr_eq(&animating, &settled));
        assert_eq!(settled.rows().len(), 3);
    }
}
