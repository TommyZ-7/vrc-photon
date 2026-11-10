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
use gpui_kit::component::popover::Popover;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_kit::{Size as VirtualRowSize, size as virtual_row_size};
use vrc_photon_core::model::{Photo, SessionSummary};
use vrc_photon_core::settings::PhotosViewMode;
use vrc_photon_i18n::{tr, tr_args};

use crate::app::{App, CardBoundsMap, Screen, effective_motion_fps, retain_recent_bounds};
use crate::state::photo_browser::{
    FailedPage, PAGE_SIZE, PageFailure, PhotoBrowser, anchor_photo_id, first_visible_photo,
    list_scroll_top, row_for_photo, session_scroll_top, set_session_scroll_offset, visible_month,
};
use crate::state::session_rows::{SessionRow, SessionRowKind};
use crate::state::thumbnail::ThumbnailCache;
use crate::state::viewer::ViewerOpenSource;
use crate::state::year_rail::format_captured_month;
use crate::ui::components::{card_header, chip, glass_panel, photo_scene_key, photo_world_label};
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
        });
        if self.entries.len() >= MAX_CACHED_CARD_STRINGS {
            self.entries.clear();
        }
        self.entries.insert(photo.id, strings.clone());
        self.computations = self.computations.wrapping_add(1);
        strings
    }
}

/// Player list for a session header: first three names plus the remainder.
pub(crate) fn session_player_summary(players: &[String]) -> String {
    let separator = tr!("session.player_separator");
    match players.len() {
        0 => tr!("session.players.none"),
        1..=3 => tr_args!("session.players", names = players.join(&separator)),
        _ => {
            let names = players[..3].join(&separator);
            tr_args!(
                "session.players.more",
                names = names,
                count = players.len() - 3
            )
        }
    }
}

/// Local time range for a session header.
pub(crate) fn session_time_label<Tz: TimeZone>(
    started_at: i64,
    ended_at: Option<i64>,
    tz: &Tz,
) -> String {
    let Some(start) = tz.timestamp_opt(started_at, 0).single() else {
        return tr!("date.unknown");
    };
    let start_text = format_session_moment(&start);
    let Some(end) = ended_at.and_then(|timestamp| tz.timestamp_opt(timestamp, 0).single()) else {
        return format!("{start_text} –");
    };
    if end.date_naive() == start.date_naive() {
        format!("{start_text} – {:02}:{:02}", end.hour(), end.minute())
    } else {
        format!(
            "{start_text} – {}/{} {}:{:02}",
            end.month(),
            end.day(),
            end.hour(),
            end.minute()
        )
    }
}

fn format_session_moment<Tz: TimeZone>(moment: &DateTime<Tz>) -> String {
    format!(
        "{}/{} {}:{:02}",
        moment.month(),
        moment.day(),
        moment.hour(),
        moment.minute()
    )
}

fn format_session_day(day: i32) -> String {
    if day <= 0 {
        tr!("date.unknown_day")
    } else {
        format!("{}/{}", (day / 100) % 100, day % 100)
    }
}

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
        // page cache, selection, compressed/duplicate badges, settings, and
        // palette from `App`, so a narrower subscription needs a state split
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
        .child(view_mode_toggle(app, shared.app.clone()))
        .child(filter_popover(&p))
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_size(px(10.))
                .text_color(rgb(p.muted_2))
                .child(tr!("gallery.sort_newest")),
        )
        .into_any_element()
}

/// The header's filter chips: (label, active) pairs in display order.
///
/// The chips are appearance only, as the design spec sets out for the
/// library header: nothing reads them to filter the grid. They live in a
/// popover overlay, so opening or closing it cannot change the gallery's
/// row heights or scroll position.
fn filter_chips() -> [(String, bool); 4] {
    [
        (tr!("gallery.chip.world"), true),
        (tr!("gallery.chip.player"), false),
        (tr!("gallery.chip.date"), false),
        (tr!("gallery.chip.tag"), false),
    ]
}

/// Filter button for the gallery header, opening the chip popover below
/// the button. The popover is anchored to the button's bottom-right so
/// its surface stays inside the gallery card at narrow window widths.
fn filter_popover(palette: &Palette) -> AnyElement {
    let p = *palette;
    Popover::new("gallery-filter-popover")
        .anchor(Anchor::TopRight)
        .trigger(
            Button::new("gallery-filter")
                .label(tr!("gallery.filter"))
                .small()
                .ghost(),
        )
        .content(move |_, _, _| {
            let mut row = h_flex().gap(px(6.));
            for (label, active) in filter_chips() {
                row = row.child(chip(&p, &label, active));
            }
            row
        })
        .into_any_element()
}

/// One inline error panel and the grid cell it starts at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PageErrorPanel {
    pub(crate) photo_index: usize,
    pub(crate) page: usize,
    pub(crate) span: usize,
    pub(crate) failure: PageFailure,
    pub(crate) retrying: bool,
}

/// Column span of an inline page-error panel.
///
/// Two columns when the row has a second cell to consume, one otherwise. The
/// caller skips `span` cells so every later card keeps its original column.
pub(crate) fn error_span(columns: usize, column: usize) -> usize {
    if columns >= 2 && column + 1 < columns {
        2
    } else {
        1
    }
}

/// Places at most one error panel per failed page inside a rendered photo
/// range. The panel sits at the page's first cell, or at the first visible
/// cell when the page starts above the viewport. Its span never crosses a page
/// boundary or a row end, so it cannot hide a loaded card from the next page.
pub(crate) fn page_error_panels(
    photo_start: usize,
    photo_end: usize,
    total: usize,
    columns: usize,
    failed_pages: &[FailedPage],
) -> Vec<PageErrorPanel> {
    let columns = columns.max(1);
    let photo_end = photo_end.min(total);
    if photo_end <= photo_start {
        return Vec::new();
    }
    let mut panels = Vec::new();
    for failed in failed_pages {
        let page = failed.page;
        let page_start = page * PAGE_SIZE;
        if page_start >= total {
            continue;
        }
        let page_end = (page_start + PAGE_SIZE).min(total);
        let anchor = page_start.max(photo_start);
        if anchor >= page_end || anchor >= photo_end {
            continue;
        }
        let remaining_in_page = page_end - anchor;
        let span = error_span(columns, anchor % columns)
            .min(remaining_in_page)
            .max(1);
        panels.push(PageErrorPanel {
            photo_index: anchor,
            page,
            span,
            failure: failed.failure,
            retrying: failed.retrying,
        });
    }
    panels.sort_unstable_by_key(|panel| panel.photo_index);
    panels
}

/// One error panel per failed page across a mixed session row range.
fn session_error_panels(
    rows: &[SessionRow],
    range: std::ops::Range<usize>,
    total: usize,
    columns: usize,
    failed_pages: &[FailedPage],
) -> HashMap<usize, PageErrorPanel> {
    let mut photo_start = usize::MAX;
    let mut photo_end = 0;
    for row in rows.get(range).unwrap_or(&[]) {
        if let SessionRowKind::Photos { first_photo, count } = row.kind() {
            photo_start = photo_start.min(first_photo);
            photo_end = photo_end.max(first_photo + count);
        }
    }
    if photo_start >= photo_end {
        return HashMap::new();
    }
    page_error_panels(
        photo_start,
        photo_end.min(total),
        total,
        columns,
        failed_pages,
    )
    .into_iter()
    .map(|panel| (panel.photo_index, panel))
    .collect()
}

fn render_grid(
    app: &App,
    shared: &Rc<GalleryShared>,
    frame: u64,
    window: &mut Window,
) -> AnyElement {
    let p = app.palette;
    let search = shared.search;
    let width = f32::from(content_viewport(window).width);
    let metrics = if search {
        search_grid_metrics(
            width,
            app.sidebar_collapsed,
            app.settings.ui_card_size,
            app.settings.ui_card_show_world,
            app.settings.ui_card_show_date || app.settings.ui_card_show_size,
        )
    } else {
        library_grid_metrics(
            width,
            app.sidebar_collapsed,
            app.inspector_collapsed,
            app.settings.ui_card_size,
            app.settings.ui_card_show_world,
            app.settings.ui_card_show_date || app.settings.ui_card_show_size,
        )
    };
    let image_height = metrics.image_height;
    let card_width = Some(metrics.card_width);
    let grid_gap = GRID_GAP;

    let (index, scroll, columns, row_height, generation, labels, rail_drag, rail_hover) = {
        let mut browser = shared.browser.borrow_mut();
        let pending = browser.pending_anchor.take();
        let layout_changed = metrics.columns != browser.columns
            || (metrics.row_height - browser.row_height).abs() > 0.5;
        // A resize or pane toggle changes the metrics mid-scroll. Capture the
        // old top photo before replacing them so `uniform_list` does not keep
        // a pixel offset that now points at a different row.
        let anchor = pending.or_else(|| {
            layout_changed
                .then(|| {
                    anchor_photo_id(
                        &browser.index,
                        list_scroll_top(&browser.scroll),
                        browser.columns,
                        browser.row_height,
                    )
                })
                .flatten()
        });
        browser.columns = metrics.columns;
        browser.row_height = metrics.row_height;
        if let Some(id) = anchor {
            let row = row_for_photo(&browser.index, id, browser.columns);
            browser
                .scroll
                .scroll_to_item_strict(row, ScrollStrategy::Top);
        }
        let labels = if browser.labels.is_none() || browser.labels_generation != browser.generation
        {
            // Shaping is expensive, so do it once per index snapshot instead
            // of on every render or paint frame.
            let labels = Arc::new(YearRailLabels::build(&browser.index, window, &p));
            browser.labels = Some(labels.clone());
            browser.labels_generation = browser.generation;
            labels
        } else {
            browser.labels.clone().expect("year rail labels")
        };
        (
            browser.index.clone(),
            browser.scroll.clone(),
            browser.columns,
            browser.row_height,
            browser.generation,
            labels,
            browser.rail_drag.clone(),
            browser.rail_hover.clone(),
        )
    };
    let rows = index.total.div_ceil(columns);

    if rows == 0 {
        let empty = if search {
            if app.search_active {
                glass_panel(&p)
                    .p(px(20.))
                    .text_size(px(12.))
                    .text_color(rgb(p.muted))
                    .child(tr!("gallery.no_match"))
                    .into_any_element()
            } else {
                // Before a search runs the browser mirrors the library, so an
                // empty library is not an empty result set.
                div().into_any_element()
            }
        } else if let Some(error) = app.library_error {
            div()
                .mt(px(18.))
                .max_w(px(560.))
                .p(px(20.))
                .rounded(px(18.))
                .bg(rgb(p.panel))
                .border_1()
                .border_color(rgb(p.border))
                .text_size(px(12.))
                .text_color(rgb(p.red))
                .child(tr(error))
                .child(
                    div()
                        .pt(px(4.))
                        .text_size(px(10.))
                        .text_color(rgb(p.muted_2))
                        .child(tr!("gallery.reload_hint")),
                )
                .into_any_element()
        } else {
            div()
                .mt(px(18.))
                .max_w(px(560.))
                .p(px(20.))
                .rounded(px(18.))
                .bg(rgb(p.panel))
                .border_1()
                .border_color(rgb(p.border))
                .text_size(px(12.))
                .text_color(rgb(p.muted))
                .child(tr!("gallery.empty"))
                .into_any_element()
        };
        let container = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .items_stretch()
            .pl(px(GALLERY_PAD_X))
            .pr(px(GALLERY_PAD_X))
            .pt(px(GALLERY_PAD_TOP))
            .pb(px(GALLERY_PAD_BOTTOM));
        return container.child(empty).into_any_element();
    }

    let card_strings_key = CardStringsKey::new(app, generation);
    let list_index = index.clone();
    let list_shared = shared.clone();
    let list = uniform_list(
        // Keep the list ID stable so uniform_list retains its scroll offset
        // when the column count changes; the row height is constant for the
        // current layout and only changes on resize.
        SharedString::from(if search {
            "search-grid-list"
        } else {
            "photo-grid-list"
        }),
        rows,
        move |range: std::ops::Range<usize>, _window, cx| {
            let Some(app_entity) = list_shared.app.upgrade() else {
                return Vec::new();
            };
            let (rendered, missing, missing_thumbnails) = {
                let app = app_entity.read(cx);
                let mut browser = list_shared.browser.borrow_mut();
                let photo_start = range.start * columns;
                let photo_end = (range.end * columns).min(list_index.total);
                let missing = browser.plan_pages(photo_start, photo_end, 2);
                let failed_pages = browser.pages.failed_pages_in_range(photo_start, photo_end);
                let error_panels: HashMap<usize, PageErrorPanel> = page_error_panels(
                    photo_start,
                    photo_end,
                    list_index.total,
                    columns,
                    &failed_pages,
                )
                .into_iter()
                .map(|panel| (panel.photo_index, panel))
                .collect();
                let mut missing_thumbnails = Vec::new();
                let mut rendered = Vec::with_capacity(range.len());
                for row in range {
                    let mut line = h_flex()
                        .h(px(row_height))
                        .w_full()
                        .min_w_0()
                        .items_start()
                        .gap(px(grid_gap))
                        // Keep cards out from under the year rail. The list
                        // itself stays full width so wheel events over the
                        // gutter still reach its scroll handle.
                        .pr(px(RAIL_WIDTH));
                    let mut column = 0;
                    while column < columns {
                        let photo_index = row * columns + column;
                        if photo_index >= list_index.total {
                            line = line.child(div().flex_1());
                            column += 1;
                            continue;
                        }
                        match browser.photo_at(photo_index) {
                            Some(photo) => {
                                line = line.child(render_card(
                                    app,
                                    list_shared.as_ref(),
                                    frame,
                                    &photo,
                                    &metrics,
                                    &card_strings_key,
                                    &mut missing_thumbnails,
                                ));
                                column += 1;
                            }
                            None => {
                                if let Some(panel) = error_panels.get(&photo_index) {
                                    line = line.child(render_page_error(
                                        app,
                                        list_shared.as_ref(),
                                        *panel,
                                        generation,
                                        list_index.total,
                                        &metrics,
                                        grid_gap,
                                    ));
                                    // The panel owns the consumed columns, so
                                    // later cards do not shift into them.
                                    column += panel.span;
                                } else {
                                    line =
                                        line.child(grid_placeholder(&p, image_height, card_width));
                                    column += 1;
                                }
                            }
                        }
                    }
                    rendered.push(line);
                }
                (rendered, missing, missing_thumbnails)
            };

            if !missing.is_empty() {
                let app = list_shared.app.clone();
                cx.defer(move |cx| {
                    if let Some(app_entity) = app.upgrade() {
                        app_entity.update(cx, |app, cx| {
                            app.load_pages(search, generation, missing, cx);
                        });
                    }
                });
            }
            if !missing_thumbnails.is_empty() {
                let app = list_shared.app.clone();
                cx.defer(move |cx| {
                    if let Some(app_entity) = app.upgrade() {
                        app_entity.update(cx, |app, cx| {
                            for photo in missing_thumbnails {
                                app.request_thumbnail(photo.as_ref(), cx);
                            }
                        });
                    }
                });
            }
            // Cards read the memory cache during the render above; hand the
            // ids that crossed the throttle window to one background batch.
            let access_refreshes = list_shared.thumbnails.borrow_mut().take_access_refreshes();
            if !access_refreshes.is_empty() {
                let app = list_shared.app.clone();
                cx.defer(move |cx| {
                    if let Some(app_entity) = app.upgrade() {
                        app_entity.update(cx, |app, cx| {
                            app.refresh_thumbnail_access(access_refreshes, cx);
                        });
                    }
                });
            }
            rendered
        },
    )
    .on_scroll({
        let app = shared.app.clone();
        move |_, window, cx| {
            if let Some(app_entity) = app.upgrade() {
                app_entity.update(cx, |app, cx| {
                    app.close_photo_context_menu_for_lifecycle(window, cx);
                });
            }
        }
    })
    .track_scroll(&scroll)
    .flex_1()
    .w_full()
    .min_w_0()
    .min_h_0();

    // The wrapper stays mounted (and relative) so the year rail can overlay
    // the list without remounting it.
    let mut wrapper = v_flex()
        .id(if search { "search-grid" } else { "photo-grid" })
        .relative()
        .flex_1()
        .w_full()
        .min_w_0()
        .min_h_0();
    if search {
        wrapper = wrapper
            .track_focus(&app.search_grid_focus)
            .key_context("search_grid")
            .tab_stop(true);
    } else {
        wrapper = wrapper
            .track_focus(&app.photo_grid_focus)
            .key_context("photo_grid")
            .tab_stop(true);
    }
    let mut grid = wrapper.child(list);
    if index.total > 0 {
        // Added after the list so the rail owns the reserved gutter instead
        // of the cards underneath.
        grid = grid.child(
            div()
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .w(px(RAIL_WIDTH))
                .child(YearRail::new(
                    if search {
                        "search-year-rail"
                    } else {
                        "year-rail"
                    },
                    labels,
                    index,
                    RailList::Timeline {
                        scroll: scroll.clone(),
                        columns,
                        row_height,
                    },
                    rail_drag,
                    rail_hover,
                    p,
                )),
        );
    }
    v_flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .items_stretch()
        .pl(px(GALLERY_PAD_X))
        .pr(px(GALLERY_PAD_X))
        .pt(px(GALLERY_PAD_TOP))
        .pb(px(GALLERY_PAD_BOTTOM))
        .child(grid)
        .into_any_element()
}

fn render_session_grid(
    app: &App,
    shared: &Rc<GalleryShared>,
    frame: u64,
    window: &mut Window,
    motion_fps: f32,
) -> AnyElement {
    let p = app.palette;
    let width = f32::from(content_viewport(window).width);
    let metrics = library_grid_metrics(
        width,
        app.sidebar_collapsed,
        app.inspector_collapsed,
        app.settings.ui_card_size,
        app.settings.ui_card_show_world,
        app.settings.ui_card_show_date || app.settings.ui_card_show_size,
    );

    let now = Instant::now();
    let (
        scroll,
        columns,
        rows,
        generation,
        index,
        total,
        labels,
        rail_drag,
        rail_hover,
        animated_heights,
        driver_duration,
        driver_epoch,
    ) = {
        let mut browser = shared.browser.borrow_mut();
        let pending = browser.pending_anchor.take();
        let layout_changed = metrics.columns != browser.columns
            || (metrics.row_height - browser.row_height).abs() > 0.5;
        let anchor = pending.or_else(|| {
            layout_changed
                .then(|| {
                    let rows = browser.session_rows.as_ref()?;
                    let index =
                        rows.top_photo_index(session_scroll_top(&browser.session_scroll))?;
                    browser.index.entries.get(index).map(|entry| entry.id)
                })
                .flatten()
        });
        browser.columns = metrics.columns;
        browser.row_height = metrics.row_height;
        let layout = browser.session_layout(
            metrics.columns,
            metrics.row_height,
            SESSION_HEADER_ROW_HEIGHT + CARD_ROW_GAP,
            SESSION_DATE_LABEL_ROW_HEIGHT + CARD_ROW_GAP,
            now,
        );
        let rows = layout.rows.clone();
        if let Some(id) = anchor
            && let Some(position) = browser.index.row_of_id(id)
            && let Some(offset) = rows.anchor_offset_for_photo(position)
        {
            set_session_scroll_offset(&browser, offset);
        }
        let labels = if browser.labels.is_none() || browser.labels_generation != browser.generation
        {
            let labels = Arc::new(YearRailLabels::build(&browser.index, window, &p));
            browser.labels = Some(labels.clone());
            browser.labels_generation = browser.generation;
            labels
        } else {
            browser.labels.clone().expect("year rail labels")
        };
        (
            browser.session_scroll.clone(),
            browser.columns,
            rows,
            browser.generation,
            browser.index.clone(),
            browser.index.total,
            labels,
            browser.rail_drag.clone(),
            browser.rail_hover.clone(),
            layout.heights,
            layout.driver_duration,
            layout.driver_epoch,
        )
    };

    if total == 0 {
        return render_grid(app, shared, frame, window);
    }

    let Some(app_entity) = shared.app.upgrade() else {
        return div().into_any_element();
    };
    let animated_heights = Rc::new(animated_heights);
    let item_sizes: Rc<Vec<VirtualRowSize<Pixels>>> = Rc::new(
        animated_heights
            .iter()
            .map(|height| virtual_row_size(px(0.), px(*height)))
            .collect(),
    );
    let card_strings_key = CardStringsKey::new(app, generation);
    let list_shared = shared.clone();
    let rail_rows = rows.clone();
    let list_rows = rows;
    let list_index = index.clone();
    let list_columns = columns;
    let list_heights = animated_heights.clone();
    let grid_gap = GRID_GAP;
    let list = v_virtual_list(
        app_entity,
        "session-photo-grid",
        item_sizes,
        move |app, range, _window, cx| {
            let mut rendered = Vec::with_capacity(range.len());
            let mut missing_pages = Vec::new();
            let mut missing_summaries = Vec::new();
            let mut missing_thumbnails: Vec<Arc<Photo>> = Vec::new();
            {
                let mut browser = list_shared.browser.borrow_mut();
                let mut visible_start = usize::MAX;
                let mut visible_end = 0;
                for row_index in range.clone() {
                    if let Some(row) = list_rows.rows().get(row_index)
                        && let SessionRowKind::Photos { first_photo, count } = row.kind()
                    {
                        visible_start = visible_start.min(first_photo);
                        visible_end = visible_end.max(first_photo + count);
                    }
                }
                let failed_pages = if visible_start < visible_end {
                    browser
                        .pages
                        .failed_pages_in_range(visible_start, visible_end)
                } else {
                    Vec::new()
                };
                let error_panels = session_error_panels(
                    list_rows.rows(),
                    range.clone(),
                    list_index.total,
                    list_columns,
                    &failed_pages,
                );
                for row_index in range {
                    let Some(row) = list_rows.rows().get(row_index) else {
                        continue;
                    };
                    match row.kind() {
                        SessionRowKind::Header {
                            session_id,
                            photo_count,
                            collapsed,
                        } => {
                            let summary = browser.session_summary(session_id);
                            let failed = browser.session_summary_failed.contains(&session_id);
                            if summary.is_none() && !failed {
                                missing_summaries.push(session_id);
                            }
                            rendered.push(render_session_header(
                                app,
                                list_shared.as_ref(),
                                session_id,
                                photo_count,
                                collapsed,
                                summary.as_deref(),
                                failed,
                                &mut missing_thumbnails,
                            ));
                        }
                        SessionRowKind::DateLabel { day, photo_count } => {
                            rendered.push(render_session_date_label(app, day, photo_count));
                        }
                        SessionRowKind::Photos { first_photo, count } => {
                            let start = first_photo;
                            let end = (first_photo + count).min(list_index.total);
                            missing_pages.extend(browser.plan_pages(start, end, 2));
                            let full_height = list_rows
                                .heights()
                                .get(row_index)
                                .copied()
                                .unwrap_or(metrics.row_height);
                            let row_height =
                                list_heights.get(row_index).copied().unwrap_or(full_height);
                            let opacity = if full_height > 0.0 {
                                (row_height / full_height).clamp(0.0, 1.0)
                            } else {
                                1.0
                            };
                            let mut line = h_flex()
                                .h(px(row_height))
                                .opacity(opacity)
                                .w_full()
                                .min_w_0()
                                .items_start()
                                .gap(px(grid_gap))
                                .pr(px(RAIL_WIDTH));
                            if row_height + 0.01 < full_height {
                                line = line.overflow_hidden();
                            }
                            let mut column = 0;
                            let mut photo_index = start;
                            while column < list_columns {
                                if photo_index >= end {
                                    line = line.child(div().flex_1());
                                    column += 1;
                                    continue;
                                }
                                match browser.photo_at(photo_index) {
                                    Some(photo) => {
                                        line = line.child(render_card(
                                            app,
                                            list_shared.as_ref(),
                                            frame,
                                            &photo,
                                            &metrics,
                                            &card_strings_key,
                                            &mut missing_thumbnails,
                                        ));
                                        column += 1;
                                        photo_index += 1;
                                    }
                                    None => {
                                        if let Some(panel) = error_panels.get(&photo_index) {
                                            let mut panel = *panel;
                                            panel.span = panel.span.min(end - photo_index);
                                            line = line.child(render_page_error(
                                                app,
                                                list_shared.as_ref(),
                                                panel,
                                                generation,
                                                list_index.total,
                                                &metrics,
                                                grid_gap,
                                            ));
                                            column += panel.span;
                                            photo_index += panel.span;
                                        } else {
                                            line = line.child(grid_placeholder(
                                                &p,
                                                metrics.image_height,
                                                Some(metrics.card_width),
                                            ));
                                            column += 1;
                                            photo_index += 1;
                                        }
                                    }
                                }
                            }
                            rendered.push(line.into_any_element());
                        }
                    }
                }
                if !missing_summaries.is_empty() {
                    // One background query per visible batch instead of two
                    // IDs per frame. The core query caps the list at 256.
                    let planned = browser.plan_session_summaries(missing_summaries, 32);
                    if !planned.is_empty() {
                        let app_weak = list_shared.app.clone();
                        cx.defer(move |cx| {
                            if let Some(app_entity) = app_weak.upgrade() {
                                app_entity.update(cx, |app, cx| {
                                    app.load_session_summaries(false, generation, planned, cx);
                                });
                            }
                        });
                    }
                }
            }

            if !missing_pages.is_empty() {
                let app = list_shared.app.clone();
                cx.defer(move |cx| {
                    if let Some(app_entity) = app.upgrade() {
                        app_entity.update(cx, |app, cx| {
                            app.load_pages(false, generation, missing_pages, cx);
                        });
                    }
                });
            }
            if !missing_thumbnails.is_empty() {
                let app = list_shared.app.clone();
                cx.defer(move |cx| {
                    if let Some(app_entity) = app.upgrade() {
                        app_entity.update(cx, |app, cx| {
                            for photo in missing_thumbnails {
                                app.request_thumbnail(photo.as_ref(), cx);
                            }
                        });
                    }
                });
            }
            let access_refreshes = list_shared.thumbnails.borrow_mut().take_access_refreshes();
            if !access_refreshes.is_empty() {
                let app = list_shared.app.clone();
                cx.defer(move |cx| {
                    if let Some(app_entity) = app.upgrade() {
                        app_entity.update(cx, |app, cx| {
                            app.refresh_thumbnail_access(access_refreshes, cx);
                        });
                    }
                });
            }
            rendered
        },
    )
    .track_scroll(&scroll)
    .flex_1()
    .w_full()
    .min_w_0()
    .min_h_0();

    let mut wrapper = v_flex()
        .id("session-grid")
        .relative()
        .flex_1()
        .w_full()
        .min_w_0()
        .min_h_0()
        .track_focus(&app.photo_grid_focus)
        .key_context("photo_grid")
        .tab_stop(true)
        .on_scroll({
            let app = shared.app.clone();
            move |_, window, cx| {
                if let Some(app_entity) = app.upgrade() {
                    app_entity.update(cx, |app, cx| {
                        app.close_photo_context_menu_for_lifecycle(window, cx);
                    });
                }
            }
        })
        .child(list);
    if total > 0 {
        wrapper = wrapper.child(
            div()
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .w(px(RAIL_WIDTH))
                .child(YearRail::new(
                    "session-year-rail",
                    labels,
                    index,
                    RailList::Sessions {
                        scroll: scroll.clone(),
                        rows: rail_rows,
                        browser: shared.browser.clone(),
                    },
                    rail_drag,
                    rail_hover,
                    p,
                )),
        );
    }

    // Keep the driver alive just past the motion so the next render can drop
    // the collapsed session's zero-height photo rows from the row model.
    let driver_duration = if driver_duration.is_zero() {
        Duration::ZERO
    } else {
        driver_duration + Duration::from_millis(50)
    };
    let driver = div().size(px(0.)).with_animation(
        ("session-collapse-driver", driver_epoch),
        Animation::new(driver_duration)
            .with_easing(ease_motion)
            .with_max_fps(motion_fps),
        |el, _| el,
    );

    v_flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .items_stretch()
        .pl(px(GALLERY_PAD_X))
        .pr(px(GALLERY_PAD_X))
        .pt(px(GALLERY_PAD_TOP))
        .pb(px(GALLERY_PAD_BOTTOM))
        .child(wrapper)
        .child(driver)
        .into_any_element()
}

/// Approximate height of the session list viewport inside the gallery card.
/// Used only to keep a partially clipped header inside the visible area.
fn session_grid_viewport_height(window: &Window) -> f32 {
    let content = content_viewport(window);
    (f32::from(content.height)
        - GALLERY_HEADER_HEIGHT
        - GALLERY_PAD_TOP
        - GALLERY_PAD_BOTTOM)
        .max(SESSION_HEADER_ROW_HEIGHT)
}

#[allow(clippy::too_many_arguments)]
fn render_session_header(
    app: &App,
    shared: &GalleryShared,
    session_id: i64,
    photo_count: usize,
    collapsed: bool,
    summary: Option<&SessionSummary>,
    failed: bool,
    missing_thumbnails: &mut Vec<Arc<Photo>>,
) -> AnyElement {
    let p = app.palette;
    let (world_name, count, time, players, ongoing, cover) = if let Some(summary) = summary {
        (
            summary
                .world_name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| tr!("session.unknown_world")),
            summary.photo_count,
            session_time_label(
                summary.session.started_at,
                summary.session.ended_at,
                &chrono::Local,
            ),
            session_player_summary(&summary.players),
            summary.session.ended_at.is_none(),
            summary.cover_photo.as_ref(),
        )
    } else {
        (
            if failed {
                tr!("session.summary_failed")
            } else {
                tr!("session.loading")
            },
            photo_count as i64,
            String::new(),
            String::new(),
            false,
            None,
        )
    };

    let (cover_color, cover_image) = if let Some(photo) = cover {
        let image = shared.thumbnails.borrow_mut().get(photo.id);
        if image.is_none() {
            missing_thumbnails.push(Arc::new(photo.clone()));
        }
        let color = shared
            .thumbnails
            .borrow()
            .color(photo.id)
            .unwrap_or_else(|| p.scene_color(photo_scene_key(photo)));
        (color, image)
    } else {
        (p.panel_deep, None)
    };

    let mut cover_el = div()
        .w(px(104.))
        .h(px(58.))
        .flex_shrink_0()
        .rounded(px(8.))
        .overflow_hidden()
        .border_1()
        .border_color(rgba(p.line_soft))
        .bg(rgb(cover_color));
    if let Some(image) = cover_image {
        cover_el = cover_el.child(
            gpui_kit::img(image)
                .object_fit(ObjectFit::Cover)
                .size_full()
                .rounded(px(8.)),
        );
    }

    let mut meta = h_flex()
        .min_w_0()
        .items_center()
        .gap(px(14.))
        .overflow_hidden()
        .text_size(px(10.))
        .text_color(rgb(p.muted));
    if !time.is_empty() {
        meta = meta.child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .font_family("monospace")
                .child(time),
        );
    }
    meta = meta.child(
        div()
            .flex_shrink_0()
            .whitespace_nowrap()
            .child(tr_args!("session.photo_count", count = format_count(count))),
    );
    if !players.is_empty() {
        meta = meta.child(div().min_w_0().truncate().child(players));
    }

    let body = v_flex()
        .min_w_0()
        .flex_1()
        .gap(px(4.))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .font_semibold()
                .text_color(rgb(p.text))
                .child(world_name),
        )
        .child(meta);

    let app_weak = shared.app.clone();
    let toggle = Button::new(SharedString::from(format!("session-toggle-{session_id}")))
        .label(if collapsed { "▸" } else { "▾" })
        .small()
        .ghost()
        .tooltip(if collapsed {
            tr!("session.expand")
        } else {
            tr!("session.collapse")
        })
        .on_click(move |_, window, cx| {
            let viewport_height = session_grid_viewport_height(window);
            if let Some(app_entity) = app_weak.upgrade() {
                app_entity.update(cx, |app, cx| {
                    app.toggle_session_collapsed(session_id, viewport_height, cx);
                });
            }
        });

    let mut header = h_flex()
        .w_full()
        .h(px(SESSION_HEADER_ROW_HEIGHT))
        .items_center()
        .gap(px(13.))
        .pl(px(14.))
        .pr(px(RAIL_WIDTH + 14.))
        .rounded(px(12.))
        .border_1()
        .border_color(rgba(p.line_soft))
        .bg(rgb(p.panel_inner))
        .child(cover_el)
        .child(body);
    if ongoing {
        header = header.child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap(px(5.))
                .px(px(8.))
                .py(px(2.))
                .rounded_full()
                .bg(rgba(with_alpha(p.green, 0x1a)))
                .border_1()
                .border_color(rgba(with_alpha(p.green, 0x33)))
                .text_size(px(9.))
                .font_semibold()
                .text_color(rgb(p.green))
                .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(p.green)))
                .child(tr!("session.ongoing")),
        );
    }
    header.child(toggle).into_any_element()
}

fn render_session_date_label(app: &App, day: i32, photo_count: usize) -> AnyElement {
    let p = app.palette;
    let line = || div().h(px(1.)).flex_1().bg(rgba(p.line_soft));
    h_flex()
        .h(px(SESSION_DATE_LABEL_ROW_HEIGHT))
        .w_full()
        .items_center()
        .gap(px(10.))
        .pr(px(RAIL_WIDTH))
        .child(line())
        .child(
            div()
                .whitespace_nowrap()
                .text_size(px(10.))
                .text_color(rgb(p.muted_2))
                .child(tr_args!(
                    "session.unlinked_date",
                    date = format_session_day(day),
                    count = format_count(photo_count as i64)
                )),
        )
        .child(line())
        .into_any_element()
}

/// Inline error panel for one failed grid page.
///
/// It keeps the card's total height so the uniform list row does not resize:
/// the image area plus the metadata area is exactly the panel box. The panel
/// replaces only the failed page's first visible cell (and the neighbour it
/// spans) and leaves every loaded card where it was.
fn render_page_error(
    app: &App,
    shared: &GalleryShared,
    panel: PageErrorPanel,
    generation: u64,
    total: usize,
    metrics: &GridMetrics,
    grid_gap: f32,
) -> AnyElement {
    let p = app.palette;
    let search = shared.search;
    let page = panel.page;
    let page_start = page * PAGE_SIZE;
    let page_end = (page_start + PAGE_SIZE).min(total);
    let count = page_end.saturating_sub(page_start);
    let reason = match panel.failure {
        PageFailure::Timeout => tr!("photos.page_load_failed_timeout"),
        PageFailure::ReadError => tr!("photos.page_load_failed_error"),
    };
    let panel_height = (metrics.row_height - CARD_ROW_GAP).max(metrics.image_height);
    // Small card presets with hidden metadata cannot fit the full mock copy.
    // Drop the explanation line first so the range, reason, and actions never
    // spill out of the fixed row.
    let compact = panel_height < 120.0;

    let retry_app = shared.app.clone();
    let retry_button = Button::new(SharedString::from(format!(
        "page-error-retry-{search}-{page}"
    )))
    .label(if panel.retrying {
        tr!("photos.page_load_retrying")
    } else {
        tr!("common.retry")
    })
    .small()
    .primary()
    .loading(panel.retrying)
    .on_click(move |_, _, cx| {
        if let Some(app_entity) = retry_app.upgrade() {
            app_entity.update(cx, |app, cx| {
                app.retry_failed_page(search, page, generation, cx);
            });
        }
    });

    let first_app = shared.app.clone();
    let first_page_button = Button::new(SharedString::from(format!(
        "page-error-first-{search}-{page}"
    )))
    .label(tr!("photos.first_page"))
    .small()
    .ghost()
    .on_click(move |_, _, cx| {
        if let Some(app_entity) = first_app.upgrade() {
            app_entity.update(cx, |app, cx| app.scroll_grid_to_first_page(search, cx));
        }
    });

    let header = h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(rgb(p.red))
                .child("⚠"),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(10.))
                .font_semibold()
                .text_color(rgb(p.red))
                .child(tr!("photos.page_load_failed_title")),
        );

    let body = div()
        .w_full()
        .min_w_0()
        .truncate()
        .text_size(px(10.))
        .text_color(rgb(p.muted))
        .child(tr!("photos.page_load_failed_body"));

    let mut panel_el = v_flex()
        .id(SharedString::from(format!("page-error-{search}-{page}")))
        .justify_center()
        .gap(px(if compact { 3. } else { 4. }))
        .h(px(panel_height))
        .min_w_0()
        .overflow_hidden()
        .px(px(if compact { 10. } else { 14. }))
        .py(px(if compact { 5. } else { 9. }))
        .rounded(px(13.))
        .border_1()
        .border_color(rgba(with_alpha(p.red, 0x59)))
        .bg(linear_gradient(
            135.,
            linear_color_stop(rgba(with_alpha(p.red, 0x12)), 0.),
            linear_color_stop(rgb(p.panel_deep), 1.),
        ))
        .child(header)
        .child(
            div()
                .w_full()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .font_semibold()
                .text_color(rgb(p.text))
                .child(tr_args!(
                    "photos.page_load_failed_range",
                    start = page_start + 1,
                    end = page_end
                )),
        )
        .when(!compact, |this| this.child(body))
        .child(
            div()
                .w_full()
                .min_w_0()
                .truncate()
                .text_size(px(10.))
                .text_color(rgb(p.muted_2))
                .child(tr_args!(
                    "photos.page_load_failed_detail",
                    page = page + 1,
                    count = count,
                    reason = reason
                )),
        )
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .child(retry_button)
                .child(first_page_button),
        );

    let span = panel.span.max(1);
    if metrics.card_width > 0.0 {
        let width = metrics.card_width * span as f32 + grid_gap * span.saturating_sub(1) as f32;
        panel_el = panel_el.w(px(width)).flex_shrink_0();
    } else {
        panel_el = panel_el.flex_grow(span as f32);
    }
    panel_el.into_any_element()
}

fn render_card(
    app: &App,
    shared: &GalleryShared,
    bounds_frame: u64,
    photo: &Arc<Photo>,
    metrics: &GridMetrics,
    card_strings_key: &CardStringsKey,
    missing_thumbnails: &mut Vec<Arc<Photo>>,
) -> AnyElement {
    let image_height = metrics.image_height;
    let card_width = Some(metrics.card_width);
    let p = app.palette;
    let id = photo.id;
    let selected = app.selected_ids.contains(id);
    let primary = app.selected == Some(id);
    let compressed = app.compressed_ids.contains(&id);
    let duplicate = app.duplicate_ids.contains(&id);
    let replaced = photo.is_replaced();
    let show_status = app.settings.ui_card_show_status;
    let show_world = app.settings.ui_card_show_world;
    let card_strings = shared
        .card_strings
        .borrow_mut()
        .strings_for(card_strings_key, photo);
    let thumbnail = shared.thumbnails.borrow_mut().get(id);
    if thumbnail.is_none() {
        missing_thumbnails.push(photo.clone());
    }
    let color = shared
        .thumbnails
        .borrow()
        .color(id)
        .unwrap_or_else(|| p.scene_color(photo_scene_key(photo)));
    let card_radius = 13.0;
    let image_radius = inner_corner_radius(card_radius, 1.0);
    let meta_line = card_strings.meta.clone();
    let has_info = show_world || meta_line.is_some();

    let mut thumb = div()
        .relative()
        .h(px(image_height))
        .w_full()
        .overflow_hidden()
        .bg(rgb(color))
        .rounded_tl(px(image_radius))
        .rounded_tr(px(image_radius))
        .when(!has_info, |this| {
            this.rounded_bl(px(image_radius))
                .rounded_br(px(image_radius))
        });
    if let Some(image) = thumbnail {
        let image = gpui_kit::img(image)
            .object_fit(ObjectFit::Cover)
            .size_full()
            // GPUI clips overflow to a rectangle, so the image has to carry
            // the rounded silhouette itself.
            .rounded_tl(px(image_radius))
            .rounded_tr(px(image_radius))
            .when(!has_info, |this| {
                this.rounded_bl(px(image_radius))
                    .rounded_br(px(image_radius))
            });
        thumb = thumb.child(image);
    }
    let bounds_map = shared.bounds.clone();
    thumb = thumb.child(
        canvas(
            move |bounds, _, _| {
                bounds_map.borrow_mut().insert(id, (bounds_frame, bounds));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    );
    if show_status {
        let status_color = if replaced {
            p.accent
        } else if duplicate {
            p.muted
        } else if compressed {
            p.green
        } else {
            p.amber
        };
        thumb = thumb.child(
            h_flex()
                .absolute()
                .left(px(9.))
                .bottom(px(8.))
                .px(px(6.))
                .py(px(3.))
                .rounded(px(6.))
                .bg(rgb(p.panel_deep))
                .border_1()
                .border_color(rgba(p.line_soft))
                .gap(px(5.))
                .items_center()
                .child(div().size(px(5.)).rounded_full().bg(rgb(status_color)))
                .child(
                    div()
                        .text_size(px(9.))
                        .font_medium()
                        .text_color(rgb(status_color))
                        .child(if replaced {
                            tr!("gallery.badge.compressed_only")
                        } else if duplicate {
                            tr!("gallery.badge.duplicate")
                        } else if compressed {
                            tr!("gallery.badge.compressed")
                        } else {
                            tr!("gallery.badge.uncompressed")
                        }),
                ),
        );
    }
    thumb = thumb.when(selected, |this| {
        this.child(
            div()
                .absolute()
                .top(px(9.))
                .left(px(9.))
                .size(px(19.))
                .rounded_full()
                .bg(rgb(p.accent_fill))
                .border_2()
                .border_color(rgb(p.bg))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .font_semibold()
                .text_color(rgb(p.on_accent))
                .child("✓"),
        )
    });
    thumb = thumb.when(photo.is_favorite, |this| {
        this.child(
            div()
                .absolute()
                .top(px(9.))
                .right(px(9.))
                .size(px(7.))
                .rounded_full()
                .bg(rgb(p.amber))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(0.), rgba(p.shadow).into()).blur_radius(px(3.)),
                ]),
        )
    });

    let mut info = v_flex()
        .min_w_0()
        .px(px(9.))
        .pt(px(CARD_BODY_GAP))
        .gap(px(CARD_TEXT_GAP));
    if show_world {
        info = info.child(
            div()
                .min_w_0()
                .text_size(px(12.))
                .line_height(relative(1.5))
                .font_semibold()
                .text_color(rgb(p.text))
                .truncate()
                .child(card_strings.world.clone()),
        );
    }
    if let Some(meta) = meta_line {
        info = info.child(
            div()
                .min_w_0()
                .text_size(px(10.))
                .line_height(relative(1.5))
                .text_color(rgb(p.muted_2))
                .truncate()
                .child(meta),
        );
    }

    let mut card = v_flex()
        .id(SharedString::from(format!("photo-{id}")))
        .relative()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .rounded(px(card_radius))
        .border_1()
        .border_color(if primary {
            rgb(p.accent)
        } else if selected {
            rgb(p.accent_border)
        } else {
            rgba(p.line_soft)
        })
        .bg(rgb(p.panel_deep))
        .cursor_pointer()
        .when(!primary && !selected, |this| {
            this.hover(|this| this.border_color(rgba(with_alpha(p.text, 0x30))))
        })
        .child(thumb);
    if selected {
        card = card.shadow(vec![
            BoxShadow::new(px(0.), px(8.), rgba(with_alpha(p.accent, 0x4d)).into())
                .blur_radius(px(24.)),
        ]);
    }
    if has_info {
        card = card.child(info);
    }

    let app_down = shared.app.clone();
    let app_right = shared.app.clone();
    let app_click = shared.app.clone();
    let card = card
        .on_mouse_down(
            MouseButton::Left,
            move |event: &MouseDownEvent, window, cx| {
                if let Some(app_entity) = app_down.upgrade() {
                    app_entity.update(cx, |app, cx| {
                        app.handle_photo_press(id, event.modifiers, cx);
                        focus_grid_or_search(app, window, cx);
                    });
                }
            },
        )
        .on_mouse_down(
            MouseButton::Right,
            move |event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if let Some(app_entity) = app_right.upgrade() {
                    app_entity.update(cx, |app, cx| {
                        app.open_photo_context_menu_at(id, event.position, window, cx);
                    });
                }
            },
        )
        .on_click(move |event: &ClickEvent, window, cx| {
            if let Some(app_entity) = app_click.upgrade() {
                app_entity.update(cx, |app, cx| {
                    focus_grid_or_search(app, window, cx);
                    if let ClickEvent::Mouse(click) = event
                        && click.down.click_count >= 2
                        && !event.modifiers().modified()
                    {
                        app.open_quick_look(window, cx, ViewerOpenSource::Card);
                    }
                });
            }
        });

    // Library and Search cells use the computed fixed width so a preset never
    // stretches a card beyond the width its image was sized for.
    let cell = div().relative().min_w_0().child(card);
    let cell = match card_width {
        Some(width) => cell.w(px(width)).flex_shrink_0(),
        None => cell.flex_1(),
    };

    cell.into_any_element()
}

fn focus_grid_or_search(app: &App, window: &mut Window, cx: &mut Context<App>) {
    if app.screen == Screen::Photos {
        window.focus(&app.photo_grid_focus, cx);
    } else {
        // Search cards reuse the library card widget, but its grid is
        // Photos-only. Focus the search grid so Menu / Shift+F10 can open
        // the same context menu without stealing focus from filter inputs
        // until a result card is clicked.
        window.focus(&app.search_grid_focus, cx);
    }
}

/// Stands in for a photo whose page has not loaded yet. Matches the card's
/// current image height so the uniform row never changes height.
fn grid_placeholder(palette: &Palette, image_height: f32, card_width: Option<f32>) -> AnyElement {
    let p = palette;
    let mut placeholder = div()
        .min_w_0()
        .h(px(image_height))
        .rounded(px(13.))
        .bg(rgb(p.panel_deep))
        .border_1()
        .border_color(rgba(p.line_soft));
    placeholder = match card_width {
        Some(width) => placeholder.w(px(width)).flex_shrink_0(),
        None => placeholder.flex_1(),
    };
    placeholder.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        CardStringsCache, CardStringsKey, FailedPage, PAGE_SIZE, PageFailure, error_span,
        filter_chips, page_error_panels, session_error_panels, session_player_summary,
        session_time_label,
    };
    use crate::state::session_rows::SessionRows;
    use crate::state::test_support::test_photo;
    use chrono::{FixedOffset, TimeZone};
    use std::collections::HashSet;
    use vrc_photon_core::model::PhotoIndexEntry;
    use vrc_photon_i18n::MISSING;

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
            vec![super::PageErrorPanel {
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

    #[test]
    fn filter_chips_match_the_design_set() {
        let chips = filter_chips();
        assert_eq!(chips.len(), 4);

        // Exactly one chip is active: the world chip, first in the row.
        let active: Vec<usize> = chips
            .iter()
            .enumerate()
            .filter(|(_, (_, active))| *active)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(active, vec![0]);

        // Every label resolves in the catalog (never the missing marker)
        // and the four labels are distinct.
        let mut labels: Vec<&str> = chips.iter().map(|(label, _)| label.as_str()).collect();
        for label in &labels {
            assert!(!label.is_empty());
            assert_ne!(*label, MISSING);
        }
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), 4);
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
