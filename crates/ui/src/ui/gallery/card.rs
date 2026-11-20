use super::*;

pub(super) fn render_card(
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
    let selected = app.library.selected_ids.contains(id);
    let primary = app.library.selected == Some(id);
    let compressed = app.library.compressed_ids.contains(&id);
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
    let card_radius = f32::from(app.settings.ui_card_radius);
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
        thumb = thumb.child(thumbnail_layer(
            image,
            photo.effective_dimensions(),
            ObjectFit::Cover,
            image_radius,
        ));
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
    if show_status && compressed {
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
                .child(div().size(px(5.)).rounded_full().bg(rgb(p.green)))
                .child(
                    div()
                        .text_size(px(9.))
                        .font_medium()
                        .text_color(rgb(p.green))
                        .child(card_strings.badge_compressed.clone()),
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
        .id(("photo", id as u64))
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
