//! Photo grid as a child entity.
//!
//! GPUI's scroll handler notifies the view that rendered the list. Rendering
//! the grid from `App` made every wheel event re-render the sidebar, chrome,
//! inspector, and the whole grid. This entity owns the scrollable list, the
//! gallery header, and the year rail, so a scroll only re-renders this view.
//!
//! The index, page cache, selection, and thumbnails stay shared with `App`:
//! `PhotoBrowser` is a `Rc<RefCell<_>>`, and page/thumbnail loads are deferred
//! back to `App` (a page boundary is rare and may re-render App; plain
//! scrolling never does).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, TimeZone, Timelike};
use gpui_kit::base::v_virtual_list;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_kit::{Size as VirtualRowSize, size as virtual_row_size};
use vrc_photon_core::model::{Photo, SessionSummary};
use vrc_photon_core::settings::PhotosViewMode;
use vrc_photon_i18n::{tr, tr_args};

use crate::app::{App, CardBoundsMap, Screen, effective_motion_fps, retain_recent_bounds};
use crate::state::photo_browser::{
    FailedPage, PAGE_SIZE, PageFailure, PhotoBrowser, SessionCollapseSummary, anchor_photo_id,
    first_visible_photo, list_scroll_top, row_for_photo, session_scroll_top,
    set_session_scroll_offset, visible_month,
};
use crate::state::session_rows::{SessionRow, SessionRowKind};
use crate::state::thumbnail::ThumbnailCache;
use crate::state::viewer::ViewerOpenSource;
use crate::state::year_rail::format_captured_month;
use crate::ui::components::{
    card_header, glass_panel, photo_scene_key, photo_world_label, thumbnail_layer,
};
use crate::ui::theme::*;
use crate::ui::year_rail::{RailList, YearRail, YearRailLabels};

pub(crate) struct GalleryShared {
    pub(crate) app: WeakEntity<App>,
    pub(crate) browser: Rc<RefCell<PhotoBrowser>>,
    pub(crate) thumbnails: Rc<RefCell<ThumbnailCache>>,
    pub(crate) bounds: CardBoundsMap,
    /// Frame counter for card bounds. Updated here instead of App's render
    /// epoch because scrolling repaints cards without re-rendering App.
    pub(crate) bounds_epoch: Rc<Cell<u64>>,
    /// Display strings for the cards this gallery has painted. A scroll
    /// repaint reuses them instead of rebuilding the world and meta lines.
    pub(crate) card_strings: RefCell<CardStringsCache>,
    pub(crate) search: bool,
}

/// Display strings for one card. These depend only on the photo row and the
/// card display settings, never on the selection, thumbnail, or palette.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CardStrings {
    pub(crate) world: SharedString,
    pub(crate) meta: Option<SharedString>,
    /// The compressed badge, which depends only on the language.
    pub(crate) badge_compressed: SharedString,
}

/// Everything that can change a card's display strings between renders.
///
/// `generation` is the photo-browser generation: it advances whenever the
/// index or page cache is replaced, so strings cached for an old copy of a
/// photo cannot survive a reload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardStringsKey {
    pub(crate) generation: u64,
    pub(crate) language: SharedString,
    pub(crate) show_world: bool,
    pub(crate) show_date: bool,
    pub(crate) show_size: bool,
}

impl CardStringsKey {
    fn new(app: &App, generation: u64) -> Self {
        Self {
            generation,
            language: app.settings.language.clone().into(),
            show_world: app.settings.ui_card_show_world,
            show_date: app.settings.ui_card_show_date,
            show_size: app.settings.ui_card_show_size,
        }
    }
}

/// Upper bound on cards kept per gallery. A session that scrolls the whole
/// library would otherwise retain one string pair per visited photo.
const MAX_CACHED_CARD_STRINGS: usize = 4096;

#[derive(Default)]
pub(crate) struct CardStringsCache {
    key: Option<CardStringsKey>,
    entries: HashMap<i64, Rc<CardStrings>>,
    badge_compressed: SharedString,
    computations: u64,
}

impl CardStringsCache {
    /// Returns the strings for `photo`, computing them only on a miss.
    ///
    /// A changed key (language, display flags, or browser generation) drops
    /// every entry, so a reloaded photo row is never shown with stale text.
    pub(crate) fn strings_for(&mut self, key: &CardStringsKey, photo: &Photo) -> Rc<CardStrings> {
        if self.key.as_ref() != Some(key) {
            self.key = Some(key.clone());
            self.entries.clear();
            self.badge_compressed = tr!("gallery.badge.compressed").into();
        }
        if let Some(strings) = self.entries.get(&photo.id) {
            return strings.clone();
        }
        let strings = Rc::new(CardStrings {
            world: photo_world_label(photo).into(),
            meta: card_meta_line(
                key.show_date,
                key.show_size,
                &photo.captured_at_local,
                if key.show_size {
                    format_bytes(photo.effective_file_size())
                } else {
                    String::new()
                },
            )
            .map(Into::into),
            badge_compressed: self.badge_compressed.clone(),
        });
        if self.entries.len() >= MAX_CACHED_CARD_STRINGS {
            self.entries.clear();
        }
        self.entries.insert(photo.id, strings.clone());
        self.computations = self.computations.wrapping_add(1);
        strings
    }
}

mod card;
mod grid;
mod session;

// The three rendering modules call each other (the grid renders cards and
// session headers) and the tests here reach their helpers, so pull the shared
// items into this module's scope.
use card::render_card;
use grid::{focus_grid_or_search, render_grid, render_session_grid, session_grid_viewport_height};
use session::{
    SessionHeaderInput, render_session_continuation, render_session_date_label,
    render_session_header, session_bulk_actions, shows_session_bulk_actions, shows_sort_label,
};

pub(crate) struct GalleryView {
    shared: Rc<GalleryShared>,
    _observe: Subscription,
}

impl GalleryView {
    pub(crate) fn new(
        app: Entity<App>,
        search: bool,
        browser: Rc<RefCell<PhotoBrowser>>,
        thumbnails: Rc<RefCell<ThumbnailCache>>,
        bounds: CardBoundsMap,
        bounds_epoch: Rc<Cell<u64>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let shared = Rc::new(GalleryShared {
            app: app.downgrade(),
            browser,
            thumbnails,
            bounds,
            bounds_epoch,
            card_strings: RefCell::new(CardStringsCache::default()),
            search,
        });
        // P09 keeps the App-wide observation: the gallery reads the index,
        // page cache, selection, compressed badges, settings, and palette
        // from `App`, so a narrower subscription needs a state split
        // that is out of scope here. IPC bursts now coalesce into one App
        // notify per drained batch (and the three-second live pass notifies
        // only on an actual change), so the gallery repaints once per batch
        // instead of once per message. A dedicated gallery revision remains a
        // follow-up.
        let observe = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            shared,
            _observe: observe,
        }
    }
}

impl Render for GalleryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app_entity) = self.shared.app.upgrade() else {
            return div().into_any_element();
        };
        let app = app_entity.read(cx);
        let motion_fps = effective_motion_fps(app.settings.ui_max_fps, window.refresh_interval(cx));
        let frame = self.shared.bounds_epoch.get().wrapping_add(1);
        self.shared.bounds_epoch.set(frame);
        retain_recent_bounds(&mut self.shared.bounds.borrow_mut(), frame);

        if self.shared.search {
            render_grid(app, &self.shared, frame, window)
        } else {
            let header = render_header(app, &self.shared, window);
            let grid = if app.settings.ui_photos_view_mode == PhotosViewMode::Sessions {
                render_session_grid(app, &self.shared, frame, window, motion_fps)
            } else {
                render_grid(app, &self.shared, frame, window)
            };
            render_panel(app, header, grid)
        }
    }
}

fn render_panel(app: &App, header: AnyElement, grid: AnyElement) -> AnyElement {
    let p = app.palette;
    v_flex()
        .relative()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .overflow_hidden()
        .rounded(px(17.))
        .bg(rgb(p.panel_card))
        .border_1()
        .border_color(rgba(p.line_soft))
        .shadow(vec![
            BoxShadow::new(px(0.), px(18.), rgba(p.shadow).into()).blur_radius(px(44.)),
            BoxShadow::new(px(0.), px(1.), rgba(p.highlight).into())
                .inset()
                .blur_radius(px(0.)),
        ])
        .child(header)
        .child(grid)
        .into_any_element()
}

fn view_mode_toggle(app: &App, app_entity: WeakEntity<App>) -> AnyElement {
    let p = app.palette;
    let current = app.settings.ui_photos_view_mode;
    let timeline = view_mode_button(
        "photos-view-mode-timeline",
        tr!("session.mode.timeline"),
        current == PhotosViewMode::Timeline,
        app_entity.clone(),
        PhotosViewMode::Timeline,
    );
    let sessions = view_mode_button(
        "photos-view-mode-sessions",
        tr!("session.mode.sessions"),
        current == PhotosViewMode::Sessions,
        app_entity,
        PhotosViewMode::Sessions,
    );
    h_flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(p.line_soft))
        .bg(rgb(p.panel_deep))
        .child(timeline)
        .child(sessions)
        .into_any_element()
}

fn view_mode_button(
    id: &'static str,
    label: String,
    active: bool,
    app_entity: WeakEntity<App>,
    mode: PhotosViewMode,
) -> Button {
    let button = Button::new(SharedString::from(id)).label(label).small();
    let button = if active {
        button.primary()
    } else {
        button.ghost()
    };
    button.on_click(move |_, _, cx| {
        if let Some(app_entity) = app_entity.upgrade() {
            app_entity.update(cx, |app, cx| app.set_photos_view_mode(mode, cx));
        }
    })
}

fn render_header(app: &App, shared: &Rc<GalleryShared>, window: &Window) -> AnyElement {
    let p = app.palette;
    let browser = shared.browser.borrow();
    let entries = &browser.index.entries;
    let total = entries.len();
    let width = f32::from(content_viewport(window).width);
    let metrics = library_grid_metrics(
        width,
        app.sidebar_collapsed,
        app.inspector_collapsed,
        app.settings.ui_card_size,
        app.settings.ui_card_show_world,
        app.settings.ui_card_show_date || app.settings.ui_card_show_size,
        app.grid_gap(),
    );
    let first_visible = if app.settings.ui_photos_view_mode == PhotosViewMode::Sessions {
        browser
            .top_photo_index(session_scroll_top(&browser.session_scroll))
            .unwrap_or(0)
    } else {
        first_visible_photo(
            list_scroll_top(&browser.scroll),
            metrics.columns,
            metrics.row_height,
            total,
        )
    };
    let (month, month_count) = if total == 0 {
        (tr!("gallery.no_photos"), 0)
    } else {
        let (month, count) = visible_month(&browser.index, first_visible);
        (format_captured_month(month), count)
    };
    let mode = app.settings.ui_photos_view_mode;
    let header_width = gallery_header_width(width, app.sidebar_collapsed, app.inspector_collapsed);
    let summary = browser.session_collapse_summary();

    card_header(&p)
        .child(
            h_flex()
                .min_w_0()
                .items_baseline()
                .gap(px(7.))
                .child(
                    div()
                        .whitespace_nowrap()
                        .text_size(px(12.))
                        .font_semibold()
                        .text_color(rgb(p.text))
                        .child(month),
                )
                .child(
                    div()
                        .whitespace_nowrap()
                        .font_family("monospace")
                        .text_size(px(10.))
                        .text_color(rgb(p.muted_2))
                        .child(tr_args!(
                            "gallery.month_count",
                            count = format_count(month_count as i64),
                            total = format_count(total as i64)
                        )),
                ),
        )
        .child(div().flex_1())
        .when(shows_session_bulk_actions(mode), |this| {
            this.child(session_bulk_actions(
                summary,
                shared.app.clone(),
                session_grid_viewport_height(window),
            ))
        })
        .child(view_mode_toggle(app, shared.app.clone()))
        .when(shows_sort_label(mode, header_width), |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted_2))
                    .child(tr!("gallery.sort_newest")),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::grid::{PageErrorPanel, error_span, page_error_panels, session_error_panels};
    use super::session::{
        session_player_summary, session_time_label, shows_session_bulk_actions, shows_sort_label,
    };
    use super::{CardStringsCache, CardStringsKey, FailedPage, PAGE_SIZE, PageFailure};
    use crate::state::session_rows::SessionRows;
    use crate::state::test_support::test_photo;
    use chrono::{FixedOffset, TimeZone};
    use std::collections::HashSet;
    use vrc_photon_core::model::PhotoIndexEntry;
    use vrc_photon_core::settings::PhotosViewMode;
    use vrc_photon_i18n::tr;

    fn ts(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        FixedOffset::east_opt(9 * 3600)
            .unwrap()
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp()
    }

    fn session_entry(id: i64, day: i32, session: Option<i64>) -> PhotoIndexEntry {
        PhotoIndexEntry {
            id,
            captured_at_utc: id,
            captured_month: day / 100,
            captured_day: day,
            session_id: session,
        }
    }

    #[test]
    fn session_bulk_actions_are_only_shown_in_session_mode() {
        assert!(!shows_session_bulk_actions(PhotosViewMode::Timeline));
        assert!(shows_session_bulk_actions(PhotosViewMode::Sessions));
    }

    #[test]
    fn the_sort_label_yields_before_bulk_actions_in_a_narrow_session_header() {
        assert!(shows_sort_label(PhotosViewMode::Timeline, 400.0));
        assert!(!shows_sort_label(PhotosViewMode::Sessions, 600.0));
        assert!(shows_sort_label(PhotosViewMode::Sessions, 700.0));
    }

    #[test]
    fn session_error_panels_render_once_per_failed_page() {
        let entries: Vec<_> = (0..4)
            .map(|id| session_entry(4 - id, 20261109, None))
            .collect();
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            44.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        let failed = [failed_page(0, PageFailure::ReadError)];
        let panels =
            session_error_panels(rows.rows(), 0..rows.rows().len(), entries.len(), 2, &failed);
        assert_eq!(panels.len(), 1);
        assert!(panels.contains_key(&0));
    }

    #[test]
    fn session_player_summary_keeps_three_names_and_counts_the_rest() {
        assert_eq!(session_player_summary(&[]), "同席 なし（ソロ）");
        assert_eq!(session_player_summary(&["Aoi".into()]), "同席 Aoi");
        assert_eq!(
            session_player_summary(&["Aoi".into(), "Ren".into()]),
            "同席 Aoi、Ren"
        );
        assert_eq!(
            session_player_summary(&["Aoi".into(), "Ren".into(), "Mika".into(), "Sora".into()]),
            "同席 Aoi、Ren、Mika 他1名"
        );
    }

    #[test]
    fn session_time_label_handles_ongoing_and_cross_midnight() {
        let offset = FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(
            session_time_label(ts(2026, 11, 9, 21, 14), None, &offset),
            "11/9 21:14 –"
        );
        assert_eq!(
            session_time_label(
                ts(2026, 11, 8, 22, 3),
                Some(ts(2026, 11, 8, 23, 47)),
                &offset
            ),
            "11/8 22:03 – 23:47"
        );
        assert_eq!(
            session_time_label(
                ts(2026, 11, 7, 23, 41),
                Some(ts(2026, 11, 8, 0, 26)),
                &offset
            ),
            "11/7 23:41 – 11/8 0:26"
        );
    }

    fn failed_page(page: usize, failure: PageFailure) -> FailedPage {
        FailedPage {
            page,
            failure,
            retrying: false,
        }
    }

    #[test]
    fn error_span_uses_two_columns_only_when_the_row_has_room() {
        assert_eq!(error_span(4, 0), 2);
        assert_eq!(error_span(4, 2), 2);
        assert_eq!(error_span(4, 3), 1);
        assert_eq!(error_span(2, 0), 2);
        assert_eq!(error_span(2, 1), 1);
        assert_eq!(error_span(1, 0), 1);
        assert_eq!(error_span(0, 0), 1);
    }

    #[test]
    fn page_error_panel_sits_at_the_page_start_and_spans_two_cells() {
        let failed = [failed_page(1, PageFailure::Timeout)];
        let panels = page_error_panels(0, PAGE_SIZE * 2, PAGE_SIZE * 3, 4, &failed);
        assert_eq!(
            panels,
            vec![PageErrorPanel {
                photo_index: PAGE_SIZE,
                page: 1,
                span: 2,
                failure: PageFailure::Timeout,
                retrying: false,
            }]
        );
    }

    #[test]
    fn page_error_panel_anchors_at_the_first_visible_cell() {
        let failed = [failed_page(1, PageFailure::ReadError)];
        let panels = page_error_panels(PAGE_SIZE + 2, PAGE_SIZE + 6, PAGE_SIZE * 3, 4, &failed);
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].photo_index, PAGE_SIZE + 2);
        assert_eq!(panels[0].span, 2);
    }

    #[test]
    fn page_error_panel_does_not_cross_a_page_boundary_or_hide_a_card() {
        // The only photo on the last page gets a one-column panel.
        let failed = [failed_page(1, PageFailure::ReadError)];
        let panels = page_error_panels(PAGE_SIZE, PAGE_SIZE + 1, PAGE_SIZE + 1, 4, &failed);
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].span, 1);

        // A page starting in the last column cannot span into the next row,
        // so the panel takes one column and leaves the next page's card in
        // place.
        let failed = [failed_page(4, PageFailure::Timeout)];
        let start = PAGE_SIZE * 4;
        let panels = page_error_panels(start, start + 8, PAGE_SIZE * 6, 5, &failed);
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].photo_index, start);
        assert_eq!(panels[0].span, 1);

        // A failure outside the rendered range produces no panel.
        let failed = [failed_page(1, PageFailure::ReadError)];
        assert!(page_error_panels(0, 8, PAGE_SIZE * 3, 4, &failed).is_empty());
    }

    #[test]
    fn page_error_panel_keeps_the_retrying_state_for_the_loading_label() {
        let failed = [FailedPage {
            page: 1,
            failure: PageFailure::Timeout,
            retrying: true,
        }];
        let panels = page_error_panels(0, PAGE_SIZE * 2, PAGE_SIZE * 3, 4, &failed);
        assert_eq!(panels.len(), 1);
        assert!(panels[0].retrying);
    }

    fn card_key(
        generation: u64,
        language: &str,
        show_world: bool,
        show_date: bool,
        show_size: bool,
    ) -> CardStringsKey {
        CardStringsKey {
            generation,
            language: language.into(),
            show_world,
            show_date,
            show_size,
        }
    }

    #[test]
    fn card_strings_cache_translates_the_compressed_badge_once_per_key() {
        let mut cache = CardStringsCache::default();
        let photo = test_photo(1);
        let key = card_key(0, "ja", true, true, true);

        let first = cache.strings_for(&key, &photo);
        assert_eq!(first.badge_compressed, tr!("gallery.badge.compressed"));
        let second = cache.strings_for(&key, &photo);
        assert!(std::rc::Rc::ptr_eq(&first, &second));
        assert_eq!(cache.computations, 1);
    }

    #[test]
    fn card_strings_cache_reuses_the_same_inputs() {
        let mut cache = CardStringsCache::default();
        let mut photo = test_photo(1);
        photo.world_name = Some("Midnight Rooftop".to_string());
        let key = card_key(0, "ja", true, true, true);

        let first = cache.strings_for(&key, &photo);
        let second = cache.strings_for(&key, &photo);
        assert!(std::rc::Rc::ptr_eq(&first, &second));
        assert_eq!(cache.computations, 1);
        assert_eq!(first.world, "Midnight Rooftop");
        assert_eq!(first.meta.as_deref(), Some("10-21 21:14:03 · 1.0 B"));
    }

    #[test]
    fn card_strings_cache_invalidates_on_language_or_show_flags() {
        let mut cache = CardStringsCache::default();
        let photo = test_photo(1);
        let key = card_key(0, "ja", true, true, true);

        let first = cache.strings_for(&key, &photo);
        assert_eq!(cache.computations, 1);

        let language = card_key(0, "en", true, true, true);
        let second = cache.strings_for(&language, &photo);
        assert_eq!(cache.computations, 2);
        assert_eq!(first.world, second.world);

        let flags = card_key(0, "en", true, true, false);
        let third = cache.strings_for(&flags, &photo);
        assert_eq!(cache.computations, 3);
        assert_eq!(third.meta.as_deref(), Some("10-21 21:14:03"));
    }

    #[test]
    fn card_strings_cache_does_not_collide_between_photo_ids() {
        let mut cache = CardStringsCache::default();
        let mut first = test_photo(1);
        first.world_name = Some("World A".to_string());
        let mut second = test_photo(2);
        second.world_name = Some("World B".to_string());
        let key = card_key(0, "ja", true, true, true);

        assert_eq!(cache.strings_for(&key, &first).world, "World A");
        assert_eq!(cache.strings_for(&key, &second).world, "World B");
        assert_eq!(cache.computations, 2);
    }

    #[test]
    fn card_strings_cache_clears_when_the_browser_generation_changes() {
        let mut cache = CardStringsCache::default();
        let photo = test_photo(1);
        let key = card_key(0, "ja", true, true, true);
        cache.strings_for(&key, &photo);
        assert_eq!(cache.computations, 1);

        let reloaded = card_key(1, "ja", true, true, true);
        cache.strings_for(&reloaded, &photo);
        assert_eq!(cache.computations, 2);
    }
}
