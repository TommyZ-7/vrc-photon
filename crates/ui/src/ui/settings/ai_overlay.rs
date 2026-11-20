use super::*;

impl App {
    pub(crate) fn ai_license_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.ai_modal_focus_pending {
            self.ai_modal_focus_pending = false;
            let focus = self.ai_license_focus.clone();
            cx.defer_in(window, move |this, window, cx| {
                if this.ai.license_modal_open {
                    window.focus(&focus, cx);
                }
            });
        }
        let p = self.palette;
        let manifest = self
            .ai
            .active
            .as_ref()
            .map(|install| ai_manifest_view(&install.manifest_json))
            .unwrap_or_default();
        let license = manifest
            .license
            .clone()
            .unwrap_or_else(|| tr!("settings.ai.license.unknown"));
        let license_url = manifest.license_url.clone();
        let license_file = manifest
            .license_file
            .clone()
            .unwrap_or_else(|| "—".to_string());
        let install_line = self
            .ai
            .active
            .as_ref()
            .map(|install| {
                format!(
                    "{} · {} · v{}",
                    install.plugin_id, install.variant, install.version
                )
            })
            .unwrap_or_default();

        let panel = dialog_panel(&self.palette)
            .id("ai-license-panel")
            .w(px(520.))
            .gap(px(14.))
            .p(px(20.))
            .track_focus(&self.ai_license_focus)
            .focus_trap("ai-license", &self.ai_license_focus)
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_semibold()
                                    .text_color(rgb(p.text))
                                    .child(tr!("settings.ai.license.title")),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted_2))
                                    .child(install_line),
                            ),
                    )
                    .child(
                        Button::new("ai-license-close")
                            .icon(Icon::new(IconName::Close))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_ai_license_modal(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(tr!("settings.ai.license.body")),
            )
            .child(
                v_flex()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(px(10.))
                    .bg(rgb(p.panel_2))
                    .child(self.ai_license_row(
                        tr!("settings.ai.license.metadata.license"),
                        license,
                    ))
                    .child(self.ai_license_row(
                        tr!("settings.ai.license.metadata.bundled"),
                        license_file,
                    ))
                    .when_some(license_url.clone(), |this, url| {
                        this.child(self.ai_license_row(
                            tr!("settings.ai.license.metadata.url"),
                            url,
                        ))
                    })
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted_2))
                            .child(tr!("settings.ai.license.metadata.review")),
                    )
                    .when_some(license_url, |this, url| {
                        this.child(
                            Button::new("ai-license-open")
                                .label(tr!("settings.ai.license.open"))
                                .small()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Err(error) = open_path(Path::new(&url)) {
                                        tracing::warn!(%error, %url, "cannot open the model license");
                                        this.toast_error(
                                            tr!("settings.ai.license.open_failed"),
                                            cx,
                                        );
                                    }
                                })),
                        )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(9.))
                            .text_color(rgb(p.muted_2))
                            .child(tr!("settings.ai.license.consent_note")),
                    )
                    .child(
                        Button::new("ai-license-cancel")
                            .label(tr!("common.cancel"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_ai_license_modal(window, cx)
                            })),
                    )
                    .child(
                        Button::new("ai-license-agree")
                            .label(tr!("settings.ai.license.agree_download"))
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.accept_ai_model_license(window, cx)
                            })),
                    ),
            );

        div()
            .id("ai-license-scrim")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .p(px(DIALOG_VIEWPORT_MARGIN))
            .bg(rgba(p.scrim))
            .key_context("ai_license_modal")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_ai_license_modal(window, cx);
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| this.close_ai_license_modal(window, cx)))
            .child(panel)
            .into_any_element()
    }

    pub(crate) fn ai_uninstall_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.ai_modal_focus_pending {
            self.ai_modal_focus_pending = false;
            let focus = self.ai_uninstall_focus.clone();
            cx.defer_in(window, move |this, window, cx| {
                if this.ai.uninstall_modal_open {
                    window.focus(&focus, cx);
                }
            });
        }
        let p = self.palette;
        let target = self
            .ai
            .uninstall_target
            .clone()
            .unwrap_or_else(UninstallTarget::plugin_wide);
        let variant_only = !target.is_plugin_wide();
        let install = if variant_only {
            self.ai
                .installations
                .iter()
                .find(|install| {
                    Some(&install.variant) == target.variant.as_ref()
                        && Some(&install.version) == target.version.as_ref()
                })
                .cloned()
        } else {
            self.ai.active.clone()
        };
        let manifest = install
            .as_ref()
            .map(|install| ai_manifest_view(&install.manifest_json))
            .unwrap_or_default();
        let size_label = if variant_only {
            manifest.model_size
        } else {
            self.ai
                .state
                .as_ref()
                .and_then(|state| state.model_bytes_total)
                .and_then(|bytes| u64::try_from(bytes).ok())
                .filter(|bytes| *bytes > 0)
                .or(manifest.model_size)
        }
        .map(format_bytes)
        .unwrap_or_else(|| "—".to_string());
        let install_line = install
            .as_ref()
            .map(|install| {
                format!(
                    "{} · {} · v{}",
                    install.plugin_id, install.variant, install.version
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "{} · {} · v{}",
                    AI_PLUGIN_ID,
                    target.variant.as_deref().unwrap_or("—"),
                    target.version.as_deref().unwrap_or("—")
                )
            });
        let delete_tags = self.ai.uninstall_delete_tags && !variant_only;

        let panel = dialog_panel(&self.palette)
            .id("ai-uninstall-panel")
            .w(px(520.))
            .gap(px(14.))
            .p(px(20.))
            .track_focus(&self.ai_uninstall_focus)
            .focus_trap("ai-uninstall", &self.ai_uninstall_focus)
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_semibold()
                                    .text_color(rgb(p.text))
                                    .child(tr!("settings.ai.uninstall.title")),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted_2))
                                    .child(install_line),
                            ),
                    )
                    .child(
                        Button::new("ai-uninstall-close")
                            .icon(Icon::new(IconName::Close))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_ai_uninstall_modal(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(tr!("settings.ai.uninstall.body")),
            )
            .child(self.meta_row(tr!("settings.ai.model_size"), size_label))
            .when(variant_only, |this| {
                this.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.muted_2))
                        .child(tr!("settings.ai.uninstall.variant_note")),
                )
            })
            .when(!variant_only, |this| {
                this.child(
                    v_flex()
                        .gap(px(2.))
                        .child(
                            Checkbox::new("ai-uninstall-delete-tags")
                                .checked(delete_tags)
                                .label(tr!("settings.ai.uninstall.delete_tags"))
                                .on_click(cx.listener(|this, checked, _, cx| {
                                    this.ai.uninstall_delete_tags = *checked;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(rgb(p.muted_2))
                                .child(tr!("settings.ai.uninstall.delete_tags_hint")),
                        )
                        .when(!delete_tags, |this| {
                            this.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted_2))
                                    .child(tr!("settings.ai.uninstall.keep_note")),
                            )
                        }),
                )
            })
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(9.))
                            .text_color(rgb(p.muted_2))
                            .child(tr!("settings.ai.uninstall.stopping")),
                    )
                    .child(
                        Button::new("ai-uninstall-cancel")
                            .label(tr!("common.cancel"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_ai_uninstall_modal(window, cx)
                            })),
                    )
                    .child(
                        Button::new("ai-uninstall-confirm")
                            .label(tr!("settings.ai.uninstall.confirm"))
                            .small()
                            .danger()
                            .on_click(cx.listener(|this, _, window, cx| {
                                let target = this
                                    .ai
                                    .uninstall_target
                                    .clone()
                                    .unwrap_or_else(UninstallTarget::plugin_wide);
                                let delete_tags = this.ai.uninstall_delete_tags;
                                this.close_ai_uninstall_modal(window, cx);
                                this.uninstall_ai_plugin(&target, delete_tags, cx);
                            })),
                    ),
            );

        div()
            .id("ai-uninstall-scrim")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .p(px(DIALOG_VIEWPORT_MARGIN))
            .bg(rgba(p.scrim))
            .key_context("ai_uninstall_modal")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_ai_uninstall_modal(window, cx);
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| this.close_ai_uninstall_modal(window, cx)))
            .child(panel)
            .into_any_element()
    }
}
