//! Photos view-mode wiring: timeline/session switching, session summary
//! loading, collapse state, and mode-aware scroll anchors.

use std::time::{Duration, Instant};

use gpui_kit::*;
use vrc_photon_core::error::CoreError;
use vrc_photon_core::model::{PhotoIndexEntry, SessionSummary};
use vrc_photon_core::settings::PhotosViewMode;

use crate::app::App;
use crate::state::photo_browser::{
    anchor_photo_id, list_scroll_top, session_scroll_top, set_session_scroll_offset,
};
use crate::state::session_rows::SessionRows;
use crate::ui::theme::{
    MOTION_BASE, SESSION_CONTINUATION_ROW_HEIGHT, SESSION_DATE_LABEL_ROW_HEIGHT,
    SESSION_HEADER_ROW_HEIGHT,
};

#[cfg(test)]
pub(crate) fn next_view_mode(mode: PhotosViewMode) -> PhotosViewMode {
    match mode {
        PhotosViewMode::Timeline => PhotosViewMode::Sessions,
        PhotosViewMode::Sessions => PhotosViewMode::Timeline,
    }
}

fn collapse_motion_duration(reduce_motion: bool) -> Option<Duration> {
    (!reduce_motion).then_some(MOTION_BASE)
}

/// Keeps a clicked header at its current viewport position, nudging it into
/// view only when it is partially clipped.
fn keep_header_viewport_y(y: f32, header_height: f32, viewport_height: f32) -> f32 {
    if y < 0.0 {
        0.0
    } else if y + header_height > viewport_height {
        (viewport_height - header_height).max(0.0)
    } else {
        y
    }
}

fn header_scroll_offset(header_offset: f32, viewport_y: f32) -> f32 {
    (header_offset - viewport_y).max(0.0)
}

/// Scroll anchor for a bulk collapse/expand: the header of the session that
/// owns the top visible photo. Unlinked photos have no session header, so
/// they keep the current offset.
fn bulk_anchor_for_scroll(
    rows: &SessionRows,
    entries: &[PhotoIndexEntry],
    scroll_top: f32,
    heights: &[f32],
    viewport_height: f32,
) -> Option<(i64, f32)> {
    let photo_index = rows.top_photo_index_with_heights(scroll_top, heights)?;
    let session_id = entries.get(photo_index)?.session_id?;
    let header_offset = rows.offset_for_session_with_heights(session_id, heights)?;
    let viewport_y = keep_header_viewport_y(
        header_offset - scroll_top,
        SESSION_HEADER_ROW_HEIGHT,
        viewport_height,
    );
    Some((session_id, viewport_y))
}

impl App {
    pub(crate) fn set_photos_view_mode(&mut self, mode: PhotosViewMode, cx: &mut Context<Self>) {
        if self.settings.ui_photos_view_mode == mode {
            return;
        }
        let anchor = self.capture_top_photo(false);
        self.settings.ui_photos_view_mode = mode;
        self.schedule_settings_save(cx);
        if mode == PhotosViewMode::Sessions {
            self.ensure_library_session_rows();
        }
        if let Some(id) = anchor {
            self.scroll_to_photo(false, id, ScrollStrategy::Top);
        } else {
            self.scroll_grid_to_top(false);
        }
        cx.notify();
    }

    /// Builds the mixed session row model with the current library layout.
    pub(crate) fn ensure_library_session_rows(&mut self) {
        let (columns, row_height) = {
            let browser = self.library.borrow();
            (browser.columns.max(1), browser.row_height)
        };
        self.library.borrow_mut().ensure_session_rows(
            columns,
            row_height,
            SESSION_HEADER_ROW_HEIGHT + self.grid_gap(),
            SESSION_DATE_LABEL_ROW_HEIGHT + self.grid_gap(),
            SESSION_CONTINUATION_ROW_HEIGHT + self.grid_gap(),
        );
    }

    /// Returns the photo at the top-left of the current viewport for the
    /// active grid. Search always uses the timeline geometry.
    pub(crate) fn capture_top_photo(&mut self, search: bool) -> Option<i64> {
        if search || self.settings.ui_photos_view_mode == PhotosViewMode::Timeline {
            let browser = if search {
                self.search_view.borrow()
            } else {
                self.library.borrow()
            };
            return anchor_photo_id(
                &browser.index,
                list_scroll_top(&browser.scroll),
                browser.columns,
                browser.row_height,
            );
        }
        let (columns, row_height) = {
            let browser = self.library.borrow();
            (browser.columns.max(1), browser.row_height)
        };
        let mut browser = self.library.borrow_mut();
        browser.settle_all_session_collapses();
        let rows = browser.ensure_session_rows(
            columns,
            row_height,
            SESSION_HEADER_ROW_HEIGHT + self.grid_gap(),
            SESSION_DATE_LABEL_ROW_HEIGHT + self.grid_gap(),
            SESSION_CONTINUATION_ROW_HEIGHT + self.grid_gap(),
        );
        let index = rows.top_photo_index(session_scroll_top(&browser.session_scroll))?;
        browser.index.entries.get(index).map(|entry| entry.id)
    }

    /// Scrolls to `id` in the active grid. Timeline rows use the uniform-list
    /// row; session rows use the mixed row model's photo offset.
    pub(crate) fn scroll_to_photo(&mut self, search: bool, id: i64, strategy: ScrollStrategy) {
        if search || self.settings.ui_photos_view_mode == PhotosViewMode::Timeline {
            let browser = if search {
                self.search_view.borrow()
            } else {
                self.library.borrow()
            };
            let Some(position) = browser.index.row_of_id(id) else {
                return;
            };
            let row = position / browser.columns.max(1);
            browser.scroll.scroll_to_item_strict(row, strategy);
            return;
        }

        let (columns, row_height) = {
            let browser = self.library.borrow();
            (browser.columns.max(1), browser.row_height)
        };
        let mut browser = self.library.borrow_mut();
        browser.settle_all_session_collapses();
        let rows = browser.ensure_session_rows(
            columns,
            row_height,
            SESSION_HEADER_ROW_HEIGHT + self.grid_gap(),
            SESSION_DATE_LABEL_ROW_HEIGHT + self.grid_gap(),
            SESSION_CONTINUATION_ROW_HEIGHT + self.grid_gap(),
        );
        let Some(position) = browser.index.row_of_id(id) else {
            return;
        };
        if strategy == ScrollStrategy::Nearest
            && let Some(row) = rows.row_for_photo(position)
        {
            browser.session_scroll.scroll_to_item(row, strategy);
            return;
        }
        let Some(offset) = rows.anchor_offset_for_photo(position) else {
            return;
        };
        set_session_scroll_offset(&browser, offset);
    }

    /// Returns the active grid to the first row without clearing selection.
    pub(crate) fn scroll_grid_to_top(&mut self, search: bool) {
        if search || self.settings.ui_photos_view_mode == PhotosViewMode::Timeline {
            let browser = if search {
                self.search_view.borrow()
            } else {
                self.library.borrow()
            };
            browser.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
            return;
        }
        let browser = self.library.borrow();
        browser.session_scroll.set_offset(point(px(0.), px(0.)));
    }

    /// Expands the session that contains `id` when the session is collapsed.
    /// Callers scroll to `id` afterwards.
    pub(crate) fn reveal_photo_session(&mut self, id: i64) {
        if self.settings.ui_photos_view_mode != PhotosViewMode::Sessions {
            return;
        }
        let session_id = {
            let browser = self.library.borrow();
            browser
                .index
                .row_of_id(id)
                .and_then(|position| browser.index.entries.get(position))
                .and_then(|entry| entry.session_id)
        };
        let Some(session_id) = session_id else {
            return;
        };
        let collapsed = self
            .library
            .borrow()
            .collapsed_sessions
            .contains(&session_id);
        if collapsed {
            let mut browser = self.library.borrow_mut();
            browser.toggle_session_collapsed(session_id);
            browser.settle_session_collapse(session_id);
            drop(browser);
            self.ensure_library_session_rows();
        }
    }

    pub(crate) fn toggle_session_collapsed(
        &mut self,
        session_id: i64,
        viewport_height: f32,
        cx: &mut Context<Self>,
    ) {
        let now = Instant::now();
        let (columns, row_height) = {
            let browser = self.library.borrow();
            (browser.columns.max(1), browser.row_height)
        };
        let mut browser = self.library.borrow_mut();
        let before = browser.session_layout(
            columns,
            row_height,
            SESSION_HEADER_ROW_HEIGHT + self.grid_gap(),
            SESSION_DATE_LABEL_ROW_HEIGHT + self.grid_gap(),
            SESSION_CONTINUATION_ROW_HEIGHT + self.grid_gap(),
            now,
        );
        let header_offset = before
            .rows
            .offset_for_session_with_heights(session_id, &before.heights)
            .unwrap_or(0.0);
        let viewport_y = keep_header_viewport_y(
            header_offset - session_scroll_top(&browser.session_scroll),
            SESSION_HEADER_ROW_HEIGHT,
            viewport_height,
        );

        let collapsed = browser.toggle_session_collapsed(session_id);
        let animating = if let Some(duration) = collapse_motion_duration(cx.reduce_motion()) {
            browser.animate_session_collapse(session_id, collapsed, duration, now);
            true
        } else {
            browser.settle_session_collapse(session_id);
            false
        };

        let after = browser.session_layout(
            columns,
            row_height,
            SESSION_HEADER_ROW_HEIGHT + self.grid_gap(),
            SESSION_DATE_LABEL_ROW_HEIGHT + self.grid_gap(),
            SESSION_CONTINUATION_ROW_HEIGHT + self.grid_gap(),
            now,
        );
        let header_offset = after
            .rows
            .offset_for_session_with_heights(session_id, &after.heights)
            .unwrap_or(header_offset);
        set_session_scroll_offset(&browser, header_scroll_offset(header_offset, viewport_y));
        if animating {
            browser.set_session_scroll_anchor(session_id, viewport_y);
        } else {
            browser.cancel_session_scroll_anchor();
        }
        drop(browser);
        cx.notify();
    }

    /// Collapses or expands every session in the current library index. The
    /// top visible session header stays at the top of the viewport; unlinked
    /// photos leave the scroll offset untouched.
    pub(crate) fn set_all_sessions_collapsed(
        &mut self,
        collapsed: bool,
        viewport_height: f32,
        cx: &mut Context<Self>,
    ) {
        let now = Instant::now();
        let (columns, row_height) = {
            let browser = self.library.borrow();
            (browser.columns.max(1), browser.row_height)
        };
        let mut browser = self.library.borrow_mut();
        let before = browser.session_layout(
            columns,
            row_height,
            SESSION_HEADER_ROW_HEIGHT + self.grid_gap(),
            SESSION_DATE_LABEL_ROW_HEIGHT + self.grid_gap(),
            SESSION_CONTINUATION_ROW_HEIGHT + self.grid_gap(),
            now,
        );
        let anchor = bulk_anchor_for_scroll(
            &before.rows,
            &browser.index.entries,
            session_scroll_top(&browser.session_scroll),
            &before.heights,
            viewport_height,
        );

        let changed = browser.set_displayed_sessions_collapsed(collapsed);
        if changed.is_empty() {
            return;
        }
        let animating = if let Some(duration) = collapse_motion_duration(cx.reduce_motion()) {
            for id in &changed {
                browser.animate_session_collapse(*id, collapsed, duration, now);
            }
            true
        } else {
            for id in &changed {
                browser.settle_session_collapse(*id);
            }
            false
        };

        let after = browser.session_layout(
            columns,
            row_height,
            SESSION_HEADER_ROW_HEIGHT + self.grid_gap(),
            SESSION_DATE_LABEL_ROW_HEIGHT + self.grid_gap(),
            SESSION_CONTINUATION_ROW_HEIGHT + self.grid_gap(),
            now,
        );
        if let Some((session_id, viewport_y)) = anchor {
            let header_offset = after
                .rows
                .offset_for_session_with_heights(session_id, &after.heights)
                .unwrap_or(0.0);
            set_session_scroll_offset(&browser, header_scroll_offset(header_offset, viewport_y));
            if animating {
                browser.set_session_scroll_anchor(session_id, viewport_y);
            } else {
                browser.cancel_session_scroll_anchor();
            }
        } else if animating {
            browser.cancel_session_scroll_anchor();
        }
        drop(browser);
        cx.notify();
    }

    /// Loads session metadata for the visible header rows. The browser has
    /// already marked these IDs pending, so a duplicate frame cannot plan the
    /// same set again.
    pub(crate) fn load_session_summaries(
        &mut self,
        search: bool,
        generation: u64,
        ids: Vec<i64>,
        cx: &mut Context<Self>,
    ) {
        if search || ids.is_empty() {
            return;
        }
        if self.library.borrow().generation != generation {
            return;
        }

        let Some(reader) = self.background_reader.clone() else {
            match self.db.session_summaries(&ids) {
                Ok(summaries) => self.apply_session_summaries(generation, &ids, summaries, cx),
                Err(error) => self.fail_session_summary_load(generation, &ids, error, cx),
            }
            return;
        };

        cx.spawn(async move |this, cx| {
            let background_ids = ids.clone();
            let result = cx
                .background_executor()
                .spawn(async move { reader.query(|db| db.session_summaries(&background_ids)) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(summaries) => this.apply_session_summaries(generation, &ids, summaries, cx),
                Err(_) => match this.db.session_summaries(&ids) {
                    Ok(summaries) => this.apply_session_summaries(generation, &ids, summaries, cx),
                    Err(error) => this.fail_session_summary_load(generation, &ids, error, cx),
                },
            })
            .ok();
        })
        .detach();
    }

    fn apply_session_summaries(
        &mut self,
        generation: u64,
        requested: &[i64],
        summaries: Vec<SessionSummary>,
        cx: &mut Context<Self>,
    ) {
        self.library
            .borrow_mut()
            .insert_session_summaries(generation, requested, summaries);
        cx.notify();
    }

    fn fail_session_summary_load(
        &mut self,
        generation: u64,
        ids: &[i64],
        error: CoreError,
        cx: &mut Context<Self>,
    ) {
        tracing::warn!(%error, "session_summaries failed");
        self.library
            .borrow_mut()
            .fail_session_summaries(generation, ids);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bulk_anchor_for_scroll, collapse_motion_duration, header_scroll_offset,
        keep_header_viewport_y, next_view_mode,
    };
    use crate::state::session_rows::SessionRows;
    use crate::ui::theme::MOTION_BASE;
    use std::collections::HashSet;
    use vrc_photon_core::model::PhotoIndexEntry;
    use vrc_photon_core::settings::PhotosViewMode;

    fn session_entry(id: i64, day: i32, session: Option<i64>) -> PhotoIndexEntry {
        PhotoIndexEntry {
            id,
            captured_at_utc: id,
            captured_month: day / 100,
            captured_day: day,
            session_id: session,
        }
    }

    fn rows_for(entries: &[PhotoIndexEntry]) -> SessionRows {
        SessionRows::build(
            entries,
            2,
            200.0,
            92.0,
            46.0,
            44.0,
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    #[test]
    fn bulk_anchor_uses_the_top_visible_session_header() {
        let entries = vec![
            session_entry(4, 20261109, Some(7)),
            session_entry(3, 20261109, Some(7)),
            session_entry(2, 20261108, Some(8)),
            session_entry(1, 20261108, Some(8)),
        ];
        let rows = rows_for(&entries);
        let heights = rows.heights().to_vec();

        assert_eq!(
            bulk_anchor_for_scroll(&rows, &entries, 320.0, &heights, 700.0),
            Some((8, 0.0))
        );
    }

    #[test]
    fn bulk_anchor_clamps_a_partially_hidden_header_into_view() {
        let entries = vec![
            session_entry(4, 20261109, Some(7)),
            session_entry(3, 20261109, Some(7)),
            session_entry(2, 20261108, Some(8)),
            session_entry(1, 20261108, Some(8)),
        ];
        let rows = rows_for(&entries);
        let heights = rows.heights().to_vec();

        // The session 8 header starts at 292 px; this scroll top hides it by
        // 8 px, so the anchor clamps it back to the top of the viewport.
        assert_eq!(
            bulk_anchor_for_scroll(&rows, &entries, 300.0, &heights, 700.0),
            Some((8, 0.0))
        );
    }

    #[test]
    fn bulk_anchor_resolves_a_photo_row_to_its_session_header() {
        let entries = vec![
            session_entry(4, 20261109, Some(7)),
            session_entry(3, 20261109, Some(7)),
            session_entry(2, 20261108, Some(8)),
            session_entry(1, 20261108, Some(8)),
        ];
        let rows = rows_for(&entries);
        let heights = rows.heights().to_vec();

        // Session 8's photo row starts at 384 px; this scroll top is inside
        // that row, so the anchor must still resolve to session 8's header.
        assert_eq!(
            bulk_anchor_for_scroll(&rows, &entries, 450.0, &heights, 700.0),
            Some((8, 0.0))
        );
    }

    #[test]
    fn bulk_anchor_is_none_for_unlinked_photos() {
        let entries = vec![
            session_entry(2, 20261109, None),
            session_entry(1, 20261109, None),
        ];
        let rows = rows_for(&entries);

        assert_eq!(
            bulk_anchor_for_scroll(&rows, &entries, 100.0, rows.heights(), 700.0),
            None
        );
    }

    #[test]
    fn a_visible_session_header_keeps_its_viewport_position() {
        assert_eq!(keep_header_viewport_y(320.0, 78.0, 700.0), 320.0);
    }

    #[test]
    fn a_partially_hidden_session_header_is_scrolled_minimally_into_view() {
        assert_eq!(keep_header_viewport_y(-12.0, 78.0, 700.0), 0.0);
        assert_eq!(keep_header_viewport_y(650.0, 78.0, 700.0), 622.0);
    }

    #[test]
    fn the_header_offset_uses_the_anchor_viewport_y() {
        assert_eq!(header_scroll_offset(500.0, 320.0), 180.0);
    }

    #[test]
    fn view_mode_toggle_alternates_between_timeline_and_sessions() {
        assert_eq!(
            next_view_mode(PhotosViewMode::Timeline),
            PhotosViewMode::Sessions
        );
        assert_eq!(
            next_view_mode(PhotosViewMode::Sessions),
            PhotosViewMode::Timeline
        );
    }

    #[test]
    fn reduce_motion_skips_the_collapse_animation() {
        assert_eq!(collapse_motion_duration(true), None);
        assert_eq!(collapse_motion_duration(false), Some(MOTION_BASE));
    }
}
