use super::*;

impl App {
    pub(super) fn display_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        glass_panel(&self.palette)
            .flex()
            .flex_col()
            .p(px(14.))
            .gap(px(12.))
            .rounded(px(17.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("settings.display.title")),
            )
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.display.status.label"),
                "",
                self.settings.ui_card_show_status,
                true,
                cx,
                |settings, value| settings.ui_card_show_status = value,
            ))
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.display.world.label"),
                "",
                self.settings.ui_card_show_world,
                true,
                cx,
                |settings, value| settings.ui_card_show_world = value,
            ))
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.display.date.label"),
                "",
                self.settings.ui_card_show_date,
                true,
                cx,
                |settings, value| settings.ui_card_show_date = value,
            ))
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.display.size.label"),
                "",
                self.settings.ui_card_show_size,
                true,
                cx,
                |settings, value| settings.ui_card_show_size = value,
            ))
            .child(div().h(px(1.)).bg(rgba(p.line_soft)))
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text))
                                    .child(tr!("settings.display.grid_gap.label")),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_semibold()
                                    .text_color(rgb(p.text))
                                    .child(format!("{} px", self.settings.ui_grid_gap)),
                            ),
                    )
                    .child(Slider::new(&self.grid_gap_slider).w_full())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted_2))
                            .child(tr!("settings.display.grid_gap.hint")),
                    ),
            )
            .child(card_divider(&p))
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text))
                                    .child(tr!("settings.display.card_radius.label")),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_semibold()
                                    .text_color(rgb(p.text))
                                    .child(format!("{} px", self.settings.ui_card_radius)),
                            ),
                    )
                    .child(Slider::new(&self.card_radius_slider).w_full())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted_2))
                            .child(tr!("settings.display.card_radius.hint")),
                    ),
            )
    }

    pub(super) fn performance_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        glass_panel(&self.palette)
            .flex()
            .flex_col()
            .p(px(14.))
            .gap(px(12.))
            .rounded(px(17.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("settings.performance.title")),
            )
            .child(card_divider(&p))
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.text))
                            .child(tr!("settings.performance.max_fps.label")),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted_2))
                            .child(tr!("settings.performance.max_fps.hint")),
                    )
                    .child(
                        h_flex().flex_wrap().gap(px(4.)).children(
                            [
                                (30u32, "30".to_string()),
                                (60, "60".to_string()),
                                (120, "120".to_string()),
                                (144, "144".to_string()),
                                (240, "240".to_string()),
                                (0, tr!("settings.performance.max_fps.unlimited")),
                            ]
                            .into_iter()
                            .map(|(fps, label)| {
                                let active = self.settings.ui_max_fps == fps;
                                div()
                                    .id(SharedString::from(format!("max-fps-{fps}")))
                                    .px(px(10.))
                                    .py(px(5.))
                                    .rounded(px(8.))
                                    .bg(if active {
                                        rgb(p.accent_soft)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .text_size(px(11.))
                                    .font_medium()
                                    .text_color(if active { rgb(p.accent) } else { rgb(p.muted) })
                                    .cursor_pointer()
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.set_max_fps(fps, cx)
                                        }),
                                    )
                                    .child(label)
                            }),
                        ),
                    ),
            )
    }

    pub(super) fn thumbnail_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        glass_panel(&self.palette)
            .flex()
            .flex_col()
            .p(px(14.))
            .gap(px(12.))
            .rounded(px(17.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("settings.thumbnail.title")),
            )
            .child(card_divider(&p))
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.text))
                            .child(tr!("settings.thumbnail.size")),
                    )
                    .child(h_flex().gap(px(4.)).children(
                        thumbnail::THUMBNAIL_SIZES.into_iter().map(|size| {
                            let active = self.settings.thumbnail_size == size;
                            div()
                                .id(SharedString::from(format!("thumbnail-size-{size}")))
                                .px(px(10.))
                                .py(px(5.))
                                .rounded(px(8.))
                                .bg(if active {
                                    rgb(p.accent_soft)
                                } else {
                                    rgba(0x00000000)
                                })
                                .text_size(px(11.))
                                .font_medium()
                                .text_color(if active { rgb(p.accent) } else { rgb(p.muted) })
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_thumbnail_size(size, cx)
                                }))
                                .child({
                                    let height = size * 9 / 16;
                                    format!("{size}×{height}")
                                })
                        }),
                    )),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .child(tr!("settings.thumbnail.regenerate_hint")),
            )
            .child(card_divider(&p))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(2.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted))
                            .child(match self.thumbnail_cache_usage {
                                Some(usage) => tr_args!(
                                    "settings.thumbnail.usage",
                                    files = usage.files,
                                    size = format_bytes(usage.bytes)
                                ),
                                None => tr!("settings.thumbnail.usage_loading"),
                            }),
                    )
                    .child(
                        Button::new("clear-thumbnail-cache")
                            .label(tr!("settings.thumbnail.clear"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.clear_thumbnail_cache(cx))),
                    ),
            )
    }

    pub(super) fn startup_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        glass_panel(&self.palette)
            .flex()
            .flex_col()
            .p(px(14.))
            .gap(px(12.))
            .rounded(px(17.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("settings.startup.title")),
            )
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.startup.autostart"),
                "",
                self.settings.autostart,
                false,
                cx,
                |settings, value| settings.autostart = value,
            ))
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.startup.tray"),
                "",
                self.settings.tray,
                false,
                cx,
                |settings, value| settings.tray = value,
            ))
            .child(card_divider(&p))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(12.))
                    .py(px(2.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.text))
                            .child(tr!("settings.startup.reduce_motion")),
                    )
                    .child(div().flex_1())
                    .child(
                        Switch::new("reduce-motion")
                            .checked(self.settings.ui_reduce_motion)
                            .on_click(cx.listener(|this, value, _, cx| {
                                this.set_reduce_motion(*value, cx);
                            })),
                    ),
            )
    }

    pub(crate) fn set_max_fps(&mut self, fps: u32, cx: &mut Context<Self>) {
        if self.settings.ui_max_fps == fps {
            return;
        }
        self.settings.ui_max_fps = fps;
        self.schedule_settings_save(cx);
        cx.notify();
    }

    pub(crate) fn set_card_size(&mut self, size: CardSize, cx: &mut Context<Self>) {
        if self.settings.ui_card_size == size {
            return;
        }
        self.capture_display_layout_anchors();
        self.settings.ui_card_size = size;
        self.schedule_settings_save(cx);
        cx.notify();
    }

    /// Remembers the top-left photo before a display change alters columns or
    /// row height. The next render of each grid restores that photo.
    pub(crate) fn capture_display_layout_anchors(&mut self) {
        if self.settings.ui_photos_view_mode == vrc_photon_core::settings::PhotosViewMode::Sessions
        {
            self.ensure_library_session_rows();
        }
        let library_anchor = self.capture_top_photo(false);
        let search_anchor = self.capture_top_photo(true);
        let mut library = self.library.browser.borrow_mut();
        if library.pending_anchor.is_none() {
            library.pending_anchor = library_anchor;
        }
        drop(library);
        let mut search = self.search.view.borrow_mut();
        if search.pending_anchor.is_none() {
            search.pending_anchor = search_anchor;
        }
    }

    /// Persists a grid-gap or card-radius change once the slider is released.
    pub(crate) fn save_grid_display_settings(&mut self, cx: &mut Context<Self>) {
        self.schedule_settings_save(cx);
        cx.notify();
    }

    /// Persists a thumbnail quality or size change. The in-memory images were
    /// generated with the previous spec, so drop them and let the grid request
    /// the new variant on its next frame.
    pub(crate) fn save_thumbnail_settings(&mut self, cx: &mut Context<Self>) {
        self.schedule_settings_save(cx);
        self.thumbnails.borrow_mut().clear();
        cx.notify();
    }

    pub(crate) fn set_thumbnail_size(&mut self, size: u32, cx: &mut Context<Self>) {
        if self.settings.thumbnail_size == size {
            return;
        }
        self.settings.thumbnail_size = size;
        self.save_thumbnail_settings(cx);
    }

    pub(crate) fn refresh_thumbnail_cache_usage(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = thumbnail::thumbnails_dir() else {
            self.thumbnail_cache_usage = Some(thumbnail::ThumbnailCacheUsage::default());
            return;
        };
        cx.spawn(async move |this, cx| {
            let usage = cx
                .background_executor()
                .spawn(async move { thumbnail::thumbnail_cache_usage_in(&dir).ok() })
                .await;
            this.update(cx, |this, cx| {
                this.thumbnail_cache_usage = usage;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn clear_thumbnail_cache(&mut self, cx: &mut Context<Self>) {
        if !self.ipc.connected {
            self.toast_error(tr!("settings.thumbnail.clear_daemon_required"), cx);
            cx.notify();
            return;
        }
        let Some(endpoint) = self.ipc.endpoint.clone() else {
            self.toast_error(tr!("daemon.ipc.resolve_error"), cx);
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let (result, usage) = cx
                .background_executor()
                .spawn(async move {
                    let result =
                        ipc_bridge::request(endpoint, vrc_photon_ipc::Request::ClearThumbnailCache);
                    let usage = thumbnail::thumbnail_cache_usage().ok();
                    (result, usage)
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(Response::ThumbnailCacheCleared {
                        removed_files,
                        removed_bytes,
                        ..
                    }) => {
                        this.thumbnails.borrow_mut().clear();
                        this.thumbnail_cache_usage = usage;
                        this.toast_success(
                            tr_args!(
                                "settings.thumbnail.cleared",
                                files = removed_files,
                                size = format_bytes(removed_bytes)
                            ),
                            cx,
                        );
                    }
                    Ok(Response::Error { message, .. }) => {
                        this.toast_error(message, cx);
                    }
                    Ok(_) => {
                        this.toast_error(tr!("settings.thumbnail.clear_error"), cx);
                    }
                    Err(error) => {
                        this.toast_error(ipc_bridge::error_message(&error), cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn set_reduce_motion(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.ui_reduce_motion = enabled;
        self.schedule_settings_save(cx);
        cx.set_reduce_motion(enabled);
        cx.notify();
    }
}
