use super::*;

/// One inline error panel and the grid cell it starts at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PageErrorPanel {
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
pub(super) fn error_span(columns: usize, column: usize) -> usize {
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
pub(super) fn page_error_panels(
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
pub(super) fn session_error_panels(
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

pub(super) fn render_grid(
    app: &App,
    shared: &Rc<GalleryShared>,
    frame: u64,
    window: &mut Window,
) -> AnyElement {
    let p = app.palette;
    let search = shared.search;
    let width = f32::from(content_viewport(window).width);
    let grid_gap = app.grid_gap();
    let metrics = if search {
        search_grid_metrics(
            width,
            app.sidebar_collapsed,
            app.settings.ui_card_size,
            app.settings.ui_card_show_world,
            app.settings.ui_card_show_date || app.settings.ui_card_show_size,
            grid_gap,
        )
    } else {
        library_grid_metrics(
            width,
            app.sidebar_collapsed,
            app.inspector_collapsed,
            app.settings.ui_card_size,
            app.settings.ui_card_show_world,
            app.settings.ui_card_show_date || app.settings.ui_card_show_size,
            grid_gap,
        )
    };
    let image_height = metrics.image_height;
    let card_width = Some(metrics.card_width);

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
            if app.search.active {
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
        } else if let Some(error) = app.library.error {
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
            rendered
        },
    )
    .on_scroll({
        let app = shared.app.clone();
        move |_, window, cx| {
            if let Some(app_entity) = app.upgrade() {
                app_entity.update(cx, |app, cx| {
                    app.close_photo_context_menu_for_lifecycle(window, cx);
                    app.schedule_allocator_collect(cx);
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
            .track_focus(&app.search.grid_focus)
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

pub(super) fn render_session_grid(
    app: &App,
    shared: &Rc<GalleryShared>,
    frame: u64,
    window: &mut Window,
    motion_fps: f32,
) -> AnyElement {
    let p = app.palette;
    let width = f32::from(content_viewport(window).width);
    let grid_gap = app.grid_gap();
    let metrics = library_grid_metrics(
        width,
        app.sidebar_collapsed,
        app.inspector_collapsed,
        app.settings.ui_card_size,
        app.settings.ui_card_show_world,
        app.settings.ui_card_show_date || app.settings.ui_card_show_size,
        grid_gap,
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
            SESSION_HEADER_ROW_HEIGHT + grid_gap,
            SESSION_DATE_LABEL_ROW_HEIGHT + grid_gap,
            SESSION_CONTINUATION_ROW_HEIGHT + grid_gap,
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
                                SessionHeaderInput {
                                    session_id,
                                    photo_count,
                                    collapsed,
                                    summary: summary.as_deref(),
                                    failed,
                                },
                                &mut missing_thumbnails,
                            ));
                        }
                        SessionRowKind::DateLabel { day, photo_count } => {
                            rendered.push(render_session_date_label(app, day, photo_count));
                        }
                        SessionRowKind::Continuation {
                            session_id,
                            photo_count,
                        } => {
                            let summary = browser.session_summary(session_id);
                            let failed = browser.session_summary_failed.contains(&session_id);
                            if summary.is_none() && !failed {
                                missing_summaries.push(session_id);
                            }
                            rendered.push(render_session_continuation(
                                app,
                                summary.as_deref(),
                                photo_count,
                            ));
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
            let browser = shared.browser.clone();
            move |_, window, cx| {
                browser.borrow_mut().note_session_scroll();
                if let Some(app_entity) = app.upgrade() {
                    app_entity.update(cx, |app, cx| {
                        app.close_photo_context_menu_for_lifecycle(window, cx);
                        app.schedule_allocator_collect(cx);
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
pub(super) fn session_grid_viewport_height(window: &Window) -> f32 {
    let content = content_viewport(window);
    (f32::from(content.height) - GALLERY_HEADER_HEIGHT - GALLERY_PAD_TOP - GALLERY_PAD_BOTTOM)
        .max(SESSION_HEADER_ROW_HEIGHT)
}

/// Inline error panel for one failed grid page.
///
/// It keeps the card's total height so the uniform list row does not resize:
/// the image area plus the metadata area is exactly the panel box. The panel
/// replaces only the failed page's first visible cell (and the neighbour it
/// spans) and leaves every loaded card where it was.
pub(super) fn render_page_error(
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
    let panel_height = (metrics.row_height - grid_gap).max(metrics.image_height);
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

pub(super) fn focus_grid_or_search(app: &App, window: &mut Window, cx: &mut Context<App>) {
    if app.screen == Screen::Photos {
        window.focus(&app.photo_grid_focus, cx);
    } else {
        // Search cards reuse the library card widget, but its grid is
        // Photos-only. Focus the search grid so Menu / Shift+F10 can open
        // the same context menu without stealing focus from filter inputs
        // until a result card is clicked.
        window.focus(&app.search.grid_focus, cx);
    }
}

/// Stands in for a photo whose page has not loaded yet. Matches the card's
/// current image height so the uniform row never changes height.
pub(super) fn grid_placeholder(
    palette: &Palette,
    image_height: f32,
    card_width: Option<f32>,
) -> AnyElement {
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
