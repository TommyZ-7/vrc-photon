use super::*;

/// Player list for a session header: first three names plus the remainder.
pub(super) fn session_player_summary(players: &[String]) -> String {
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
pub(super) fn session_time_label<Tz: TimeZone>(
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

pub(super) fn format_session_moment<Tz: TimeZone>(moment: &DateTime<Tz>) -> String {
    format!(
        "{}/{} {}:{:02}",
        moment.month(),
        moment.day(),
        moment.hour(),
        moment.minute()
    )
}

pub(super) fn format_session_day(day: i32) -> String {
    if day <= 0 {
        tr!("date.unknown_day")
    } else {
        format!("{}/{}", (day / 100) % 100, day % 100)
    }
}

/// Minimum header content width that still fits the bulk actions, the mode
/// toggle, and the sort label. Below it, the sort label yields first.
pub(super) const SESSION_BULK_HEADER_MIN_WIDTH: f32 = 640.0;

pub(super) fn shows_session_bulk_actions(mode: PhotosViewMode) -> bool {
    mode == PhotosViewMode::Sessions
}

pub(super) fn shows_sort_label(mode: PhotosViewMode, header_width: f32) -> bool {
    mode != PhotosViewMode::Sessions || header_width >= SESSION_BULK_HEADER_MIN_WIDTH
}

pub(super) fn session_bulk_actions(
    summary: SessionCollapseSummary,
    app_entity: WeakEntity<App>,
    viewport_height: f32,
) -> AnyElement {
    h_flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(2.))
        .child(bulk_button(
            "photos-collapse-all",
            tr!("session.collapse_all"),
            !summary.can_collapse(),
            true,
            viewport_height,
            app_entity.clone(),
        ))
        .child(bulk_button(
            "photos-expand-all",
            tr!("session.expand_all"),
            !summary.can_expand(),
            false,
            viewport_height,
            app_entity,
        ))
        .into_any_element()
}

pub(super) fn bulk_button(
    id: &'static str,
    label: String,
    disabled: bool,
    collapsed: bool,
    viewport_height: f32,
    app_entity: WeakEntity<App>,
) -> Button {
    Button::new(SharedString::from(id))
        .label(label)
        .small()
        .ghost()
        .disabled(disabled)
        .on_click(move |_, _, cx| {
            if let Some(app_entity) = app_entity.upgrade() {
                app_entity.update(cx, |app, cx| {
                    app.set_all_sessions_collapsed(collapsed, viewport_height, cx);
                });
            }
        })
}

/// The per-row values a session header displays.
pub(super) struct SessionHeaderInput<'a> {
    pub(super) session_id: i64,
    pub(super) photo_count: usize,
    pub(super) collapsed: bool,
    pub(super) summary: Option<&'a SessionSummary>,
    pub(super) failed: bool,
}

pub(super) fn render_session_header(
    app: &App,
    shared: &GalleryShared,
    header: SessionHeaderInput<'_>,
    missing_thumbnails: &mut Vec<Arc<Photo>>,
) -> AnyElement {
    let SessionHeaderInput {
        session_id,
        photo_count,
        collapsed,
        summary,
        failed,
    } = header;
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
        let dimensions = cover
            .map(|photo| photo.effective_dimensions())
            .unwrap_or((16, 9));
        cover_el = cover_el.child(thumbnail_layer(image, dimensions, ObjectFit::Cover, 8.0));
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
        .text()
        .h(px(SESSION_TOGGLE_SIZE))
        .w(px(SESSION_TOGGLE_SIZE))
        .flex_shrink_0()
        .justify_center()
        .text_size(px(14.))
        .accessibility_label(if collapsed {
            tr!("session.expand")
        } else {
            tr!("session.collapse")
        })
        .toggled(!collapsed)
        .tooltip(if collapsed {
            tr!("session.expand")
        } else {
            tr!("session.collapse")
        })
        .on_click(move |_, window, cx| {
            // The card below is also clickable. Stop here so one press toggles once.
            cx.stop_propagation();
            let viewport_height = session_grid_viewport_height(window);
            if let Some(app_entity) = app_weak.upgrade() {
                app_entity.update(cx, |app, cx| {
                    app.toggle_session_collapsed(session_id, viewport_height, cx);
                });
            }
        });

    let mut controls = h_flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(8.))
        .mr(px(SESSION_HEADER_CONTROL_INSET));
    if ongoing {
        controls = controls.child(
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
    controls = controls.child(toggle);

    let card_id = SharedString::from(format!("session-header-{session_id}"));
    let app_card = shared.app.clone();
    let header = h_flex()
        .id(card_id)
        .cursor_pointer()
        .hover(move |this| this.bg(rgb(p.panel_2)))
        .on_click(move |_, window, cx| {
            let viewport_height = session_grid_viewport_height(window);
            if let Some(app_entity) = app_card.upgrade() {
                app_entity.update(cx, |app, cx| {
                    app.toggle_session_collapsed(session_id, viewport_height, cx);
                });
            }
        })
        .w_full()
        .h(px(SESSION_HEADER_ROW_HEIGHT))
        .items_center()
        .gap(px(13.))
        .pl(px(SESSION_HEADER_PAD_X))
        .pr(px(SESSION_HEADER_PAD_X))
        .rounded(px(12.))
        .border_1()
        .border_color(rgba(p.line_soft))
        .bg(rgb(p.panel_inner))
        .child(cover_el)
        .child(body)
        .child(controls);

    div()
        .w_full()
        .pr(px(RAIL_WIDTH))
        .child(header)
        .into_any_element()
}

pub(super) fn render_session_continuation(
    app: &App,
    summary: Option<&SessionSummary>,
    photo_count: usize,
) -> AnyElement {
    let p = app.palette;
    let world = summary
        .and_then(|summary| summary.world_name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| tr!("session.unknown_world"));
    let world = if summary.is_none() {
        tr!("session.loading")
    } else {
        world
    };

    h_flex()
        .h(px(SESSION_CONTINUATION_ROW_HEIGHT))
        .w_full()
        .pr(px(RAIL_WIDTH))
        .pl(px(SESSION_HEADER_PAD_X))
        .items_center()
        .gap(px(9.))
        .border_l_2()
        .border_color(rgba(p.line_soft))
        .text_size(px(11.))
        .text_color(rgb(p.muted))
        .child(div().min_w_0().truncate().child(world))
        .child(
            div()
                .flex_shrink_0()
                .px(px(6.))
                .py(px(1.))
                .rounded_full()
                .bg(rgba(with_alpha(p.accent, 0x12)))
                .text_color(rgb(p.accent))
                .text_size(px(8.5))
                .child(tr!("session.continued")),
        )
        .child(
            div()
                .flex_shrink_0()
                .font_family("monospace")
                .child(tr_args!(
                    "session.continued_count",
                    count = format_count(photo_count as i64)
                )),
        )
        .into_any_element()
}

pub(super) fn render_session_date_label(app: &App, day: i32, photo_count: usize) -> AnyElement {
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
