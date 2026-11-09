//! Photos view-mode wiring: timeline/session switching, session summary
//! loading, collapse state, and mode-aware scroll anchors.

use std::time::{Duration, Instant};

use gpui_kit::*;
use vrc_photon_core::error::CoreError;
use vrc_photon_core::model::SessionSummary;
use vrc_photon_core::settings::PhotosViewMode;

use crate::app::App;
use crate::state::photo_browser::{
    anchor_photo_id, list_scroll_top, session_scroll_top, set_session_scroll_offset,
};
use crate::ui::theme::{
    CARD_ROW_GAP, MOTION_BASE, SESSION_DATE_LABEL_ROW_HEIGHT, SESSION_HEADER_ROW_HEIGHT,
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

impl App {
    pub(crate) fn set_photos_view_mode(&mut self, mode: PhotosViewMode, cx: &mut Context<Self>) {
        if self.settings.ui_photos_view_mode == mode {
            return;
        }
        let anchor = self.capture_top_photo(false);
        self.settings.ui_photos_view_mode = mode;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save photos view mode");
        }
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
            SESSION_HEADER_ROW_HEIGHT + CARD_ROW_GAP,
            SESSION_DATE_LABEL_ROW_HEIGHT + CARD_ROW_GAP,
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
            SESSION_HEADER_ROW_HEIGHT + CARD_ROW_GAP,
            SESSION_DATE_LABEL_ROW_HEIGHT + CARD_ROW_GAP,
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
            SESSION_HEADER_ROW_HEIGHT + CARD_ROW_GAP,
            SESSION_DATE_LABEL_ROW_HEIGHT + CARD_ROW_GAP,
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

    pub(crate) fn toggle_session_collapsed(&mut self, session_id: i64, cx: &mut Context<Self>) {
        let (columns, row_height, old_offset) = {
            let browser = self.library.borrow();
            let offset = browser
                .session_rows
                .as_ref()
                .and_then(|rows| rows.offset_for_session(session_id))
                .unwrap_or(0.0);
            (browser.columns.max(1), browser.row_height, offset)
        };
        let mut browser = self.library.borrow_mut();
        let collapsed = browser.toggle_session_collapsed(session_id);
        if let Some(duration) = collapse_motion_duration(cx.reduce_motion()) {
            browser.animate_session_collapse(session_id, collapsed, duration, Instant::now());
        } else {
            browser.settle_session_collapse(session_id);
        }
        let layout = browser.session_layout(
            columns,
            row_height,
            SESSION_HEADER_ROW_HEIGHT + CARD_ROW_GAP,
            SESSION_DATE_LABEL_ROW_HEIGHT + CARD_ROW_GAP,
            Instant::now(),
        );
        let offset = layout
            .rows
            .offset_for_session_with_heights(session_id, &layout.heights)
            .unwrap_or(old_offset);
        set_session_scroll_offset(&browser, offset);
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
    use super::{collapse_motion_duration, next_view_mode};
    use crate::ui::theme::MOTION_BASE;
    use vrc_photon_core::settings::PhotosViewMode;

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
