use super::*;

/// One AI plugin action button: its identity, effect, copy, and emphasis.
struct AiActionButton {
    id: &'static str,
    action: AiPluginAction,
    label: String,
    primary: bool,
    danger: bool,
}

impl App {
    pub(super) fn ai_plugin_card(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let view = AiPluginCardView::from(&self.ai);
        let state = view.state;
        let has_active = self.ai.active.is_some();
        let update_notice = ai_plugin_update_notice(
            self.ai.active.as_ref(),
            self.update.ai_plugin.as_ref(),
            vrc_photon_update::CURRENT_VERSION,
        );
        let mut card = glass_panel(&self.palette)
            .flex()
            .flex_col()
            .p(px(14.))
            .gap(px(12.))
            .rounded(px(17.))
            .child(
                h_flex().items_center().gap(px(10.)).child(
                    v_flex().flex_1().min_w_0().gap(px(2.)).child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(p.text))
                            .child(tr!("settings.ai.title")),
                    ),
                ),
            );

        if self.ai.install_busy {
            return card.child(card_divider(&p)).child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .child(tr!("settings.ai.installing")),
            );
        }

        if !has_active {
            card = card.child(card_divider(&p)).child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .child(tr!("settings.ai.not_installed.body")),
            );
            if let Some(error) = self
                .ai
                .error_of(AiErrorKind::Install)
                .or_else(|| self.ai.error_of(AiErrorKind::Model))
                .cloned()
            {
                card = card
                    .child(card_divider(&p))
                    .child(self.ai_operation_error_box(&error));
            }
            return card
                .child(card_divider(&p))
                .child(self.ai_primary_action_button(view.primary_action, false, cx));
        }

        let install = self
            .ai
            .active
            .clone()
            .expect("the active installation was checked above");
        let manifest = ai_manifest_view(&install.manifest_json);
        let model_size = self
            .ai
            .state
            .as_ref()
            .and_then(|state| state.model_bytes_total)
            .and_then(|bytes| u64::try_from(bytes).ok())
            .filter(|bytes| *bytes > 0)
            .or(manifest.model_size);
        let size_label = model_size
            .map(format_bytes)
            .unwrap_or_else(|| tr!("settings.ai.not_downloaded"));

        card = card.child(card_divider(&p)).child(self.ai_install_meta(
            &install,
            &size_label,
            update_notice.as_ref(),
        ));
        if !matches!(
            state,
            AiPluginCardState::Downloading | AiPluginCardState::Verifying
        ) {
            card = card.child(card_divider(&p)).child(self.ai_variant_list(cx));
        }

        if matches!(
            state,
            AiPluginCardState::Downloading | AiPluginCardState::Verifying
        ) {
            if let Some(download) = self.ai.download.clone() {
                card = card
                    .child(card_divider(&p))
                    .child(self.ai_download_box(&download));
            }
            card = card.child(card_divider(&p)).child(
                h_flex().flex_wrap().items_center().gap(px(8.)).child(
                    Button::new("ai-cancel-download")
                        .label(tr!("settings.ai.cancel"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_ai_model_download(cx))),
                ),
            );
        } else if matches!(
            state,
            AiPluginCardState::Ready | AiPluginCardState::NeedsProbe
        ) {
            card = card.child(card_divider(&p)).child(self.ai_model_path_box(
                &install,
                &manifest,
                &size_label,
            ));
            card = card.child(card_divider(&p)).child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(self.ai_primary_action_button(view.primary_action, false, cx))
                    .when(state == AiPluginCardState::Ready, |this| {
                        this.child(
                            Button::new("ai-thresholds-open")
                                .label(tr!("settings.ai.thresholds.open"))
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_ai_thresholds(window, cx)
                                })),
                        )
                    })
                    .child(self.ai_action_button(
                        AiActionButton {
                            id: "ai-import-zip",
                            action: AiPluginAction::InstallZip,
                            label: tr!("settings.ai.update"),
                            primary: false,
                            danger: false,
                        },
                        cx,
                    ))
                    .child(self.ai_action_button(
                        AiActionButton {
                            id: "ai-delete-model",
                            action: AiPluginAction::DeleteModel,
                            label: tr!("settings.ai.delete_model"),
                            primary: false,
                            danger: false,
                        },
                        cx,
                    ))
                    .child(self.ai_action_button(
                        AiActionButton {
                            id: "ai-uninstall",
                            action: AiPluginAction::Uninstall,
                            label: tr!("settings.ai.uninstall"),
                            primary: false,
                            danger: true,
                        },
                        cx,
                    )),
            );
        } else if state == AiPluginCardState::Error {
            let error = self
                .ai
                .error_of(AiErrorKind::Model)
                .cloned()
                .or_else(|| {
                    self.ai
                        .state
                        .as_ref()
                        .and_then(|state| state.model_error.clone())
                        .map(|code| AiErrorInfo::new(AiErrorKind::Model, code, None))
                })
                .unwrap_or_else(|| {
                    AiErrorInfo::new(AiErrorKind::Model, "model_download_failed", None)
                });
            card = card
                .child(card_divider(&p))
                .child(self.ai_error_box(&error, cx))
                .child(card_divider(&p))
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(self.ai_primary_action_button(view.primary_action, true, cx))
                        .child(self.ai_action_button(
                            AiActionButton {
                                id: "ai-error-import",
                                action: AiPluginAction::ImportModel,
                                label: tr!("settings.ai.import_model"),
                                primary: false,
                                danger: false,
                            },
                            cx,
                        ))
                        .child(self.ai_action_button(
                            AiActionButton {
                                id: "ai-error-recheck",
                                action: AiPluginAction::Recheck,
                                label: tr!("settings.ai.recheck"),
                                primary: false,
                                danger: false,
                            },
                            cx,
                        ))
                        .child(self.ai_action_button(
                            AiActionButton {
                                id: "ai-error-uninstall",
                                action: AiPluginAction::Uninstall,
                                label: tr!("settings.ai.uninstall"),
                                primary: false,
                                danger: true,
                            },
                            cx,
                        )),
                );
        } else {
            card = card
                .child(card_divider(&p))
                .child(self.ai_model_box(&manifest, &size_label))
                .child(card_divider(&p))
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(self.ai_primary_action_button(view.primary_action, false, cx))
                        .child(self.ai_action_button(
                            AiActionButton {
                                id: "ai-import-model",
                                action: AiPluginAction::ImportModel,
                                label: tr!("settings.ai.import_model"),
                                primary: false,
                                danger: false,
                            },
                            cx,
                        ))
                        .child(self.ai_action_button(
                            AiActionButton {
                                id: "ai-recheck",
                                action: AiPluginAction::Recheck,
                                label: tr!("settings.ai.recheck"),
                                primary: false,
                                danger: false,
                            },
                            cx,
                        ))
                        .child(self.ai_action_button(
                            AiActionButton {
                                id: "ai-import-zip",
                                action: AiPluginAction::InstallZip,
                                label: tr!("settings.ai.update"),
                                primary: false,
                                danger: false,
                            },
                            cx,
                        ))
                        .child(self.ai_action_button(
                            AiActionButton {
                                id: "ai-missing-uninstall",
                                action: AiPluginAction::Uninstall,
                                label: tr!("settings.ai.uninstall"),
                                primary: false,
                                danger: true,
                            },
                            cx,
                        )),
                );
        }

        if state != AiPluginCardState::Error
            && let Some(error) = self
                .ai
                .error_of(AiErrorKind::Install)
                .or_else(|| self.ai.error_of(AiErrorKind::Model))
                .cloned()
        {
            card = card
                .child(card_divider(&p))
                .child(self.ai_operation_error_box(&error));
        }

        card
    }

    fn ai_install_meta(
        &self,
        install: &PluginInstallation,
        size_label: &str,
        update: Option<&semver::Version>,
    ) -> impl IntoElement {
        let p = self.palette;
        v_flex()
            .gap(px(4.))
            .child(self.meta_row(tr!("settings.ai.version"), format!("v{}", install.version)))
            .when_some(update.cloned(), |this, version| {
                this.child(
                    h_flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().w(px(3.)).h(px(12.)).rounded_full().bg(rgb(p.accent)))
                        .child(
                            div()
                                .text_size(px(10.))
                                .font_medium()
                                .text_color(rgb(p.accent))
                                .child(tr_args!(
                                    "settings.ai.update_available",
                                    version = version.to_string()
                                )),
                        ),
                )
            })
            .child(self.meta_row(tr!("settings.ai.variant"), install.variant.clone()))
            .child(self.meta_row(
                tr!("settings.ai.backend"),
                variant_backend_label(&install.variant),
            ))
            .child(self.meta_row(tr!("settings.ai.model_size"), size_label.to_string()))
    }

    fn ai_variant_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let installations = self.ai.installations.clone();
        v_flex()
            .gap(px(6.))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.section_label(tr!("settings.ai.variants")))
                    .child(self.badge(installations.len().to_string(), p.muted, p.panel_2)),
            )
            .child(
                v_flex()
                    .gap(px(4.))
                    .children(installations.into_iter().map(|install| {
                        let active = is_active_installation(&install);
                        let display_variant = install.variant.clone();
                        let switch_variant = install.variant.clone();
                        let version = install.version.clone();
                        let row_id = installation_row_id(&install);
                        let remove_id = format!("{row_id}-remove");
                        let remove_target = UninstallTarget::for_install(&install);
                        h_flex()
                            .items_center()
                            .gap(px(8.))
                            .p(px(8.))
                            .rounded(px(8.))
                            .bg(rgb(if active { p.accent_soft } else { p.panel_2 }))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_medium()
                                            .text_color(rgb(if active { p.accent } else { p.text }))
                                            .child(display_variant.clone()),
                                    )
                                    .child(
                                        div().text_size(px(9.)).text_color(rgb(p.muted_2)).child(
                                            format!(
                                                "v{version} · {}",
                                                variant_backend_label(&display_variant)
                                            ),
                                        ),
                                    ),
                            )
                            .when(!active, |this| {
                                this.child(
                                    h_flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .child(
                                            Button::new(SharedString::from(row_id))
                                                .label(tr!("settings.ai.variant.switch"))
                                                .small()
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.set_ai_active_variant(
                                                        &switch_variant,
                                                        Some(&version),
                                                        cx,
                                                    );
                                                })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(remove_id))
                                                .label(tr!("settings.ai.variant.remove"))
                                                .small()
                                                .danger()
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.open_ai_uninstall_modal(
                                                            remove_target.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                        ),
                                )
                            })
                    })),
            )
    }

    fn ai_model_box(&self, manifest: &AiManifestView, size_label: &str) -> impl IntoElement {
        let p = self.palette;
        h_flex()
            .items_center()
            .gap(px(10.))
            .p(px(10.))
            .rounded(px(10.))
            .bg(rgb(p.panel_2))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div().text_size(px(11.)).text_color(rgb(p.text)).child(
                            manifest
                                .model_name
                                .clone()
                                .unwrap_or_else(|| tr!("settings.ai.model_unknown")),
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(9.))
                            .text_color(rgb(p.muted_2))
                            .child(format!(
                                "{} · {size_label}",
                                manifest
                                    .revision
                                    .clone()
                                    .unwrap_or_else(|| tr!("settings.ai.unknown"))
                            )),
                    ),
            )
    }

    fn ai_model_path_box(
        &self,
        install: &PluginInstallation,
        manifest: &AiManifestView,
        size_label: &str,
    ) -> impl IntoElement {
        let p = self.palette;
        let models_dir = vrc_photon_core::data_dir()
            .map(|dir| dir.join("plugins").join(&install.plugin_id).join("models"));
        let path = match (models_dir, manifest.revision.clone()) {
            (Some(dir), Some(revision)) => dir.join(revision).display().to_string(),
            (Some(dir), None) => dir.display().to_string(),
            (None, Some(revision)) => revision,
            (None, None) => tr!("settings.ai.unknown"),
        };
        v_flex()
            .gap(px(4.))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.section_label(tr!("settings.ai.model_path")))
                    .child(self.badge(size_label.to_string(), p.muted, p.panel_2)),
            )
            .child(
                div()
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(rgb(p.panel_2))
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .truncate()
                    .child(path),
            )
    }

    fn ai_download_box(&self, download: &ModelDownloadView) -> impl IntoElement {
        let p = self.palette;
        let ratio = model_progress_ratio(download.bytes, download.total);
        let percent = (ratio * 100.0).round() as u32;
        let size = match download.total {
            Some(total) => format!("{} / {}", format_bytes(download.bytes), format_bytes(total)),
            None => format_bytes(download.bytes),
        };
        let status = if download.state == ModelState::Verifying {
            tr!("settings.ai.status.verifying")
        } else {
            tr!("settings.ai.status.downloading")
        };
        v_flex()
            .gap(px(6.))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted))
                            .child(status),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_semibold()
                            .text_color(rgb(p.text))
                            .child(format!("{percent}%")),
                    ),
            )
            .child(status_progress_bar(&p, ratio))
            .child(
                div()
                    .text_size(px(9.))
                    .text_color(rgb(p.muted_2))
                    .child(size),
            )
    }

    fn ai_error_box(&self, error: &AiErrorInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let language = vrc_photon_i18n::current_locale();
        let summary = ai_error_summary_in(&language, error);
        let show_code = ai_error_code_is_stable(&error.code);
        let detail = ai_error_detail_in(&language, error)
            .filter(|detail| !(show_code && detail == &error.code));
        let code = error.code.clone();
        v_flex()
            .gap(px(3.))
            .p(px(10.))
            .rounded(px(8.))
            .bg(rgb(p.red_soft))
            .border_1()
            .border_color(rgb(p.red_border))
            .child(
                div()
                    .text_size(px(10.))
                    .font_medium()
                    .text_color(rgb(p.red))
                    .child(summary),
            )
            .when_some(detail, |this, detail| {
                this.child(
                    div()
                        .text_size(px(9.))
                        .text_color(rgb(p.muted))
                        .child(detail),
                )
            })
            .when(show_code, |this| {
                this.child(
                    div()
                        .text_size(px(9.))
                        .text_color(rgb(p.muted_2))
                        .child(tr_args!("settings.ai.error_code", code = code)),
                )
            })
            .child(
                Button::new("ai-error-open-logs")
                    .label(tr!("settings.ai.open_logs"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| this.open_logs_folder(cx))),
            )
    }

    fn ai_operation_error_box(&self, error: &AiErrorInfo) -> impl IntoElement {
        let p = self.palette;
        let language = vrc_photon_i18n::current_locale();
        let summary = ai_error_summary_in(&language, error);
        let detail = ai_error_detail_in(&language, error);
        v_flex()
            .gap(px(2.))
            .p(px(10.))
            .rounded(px(8.))
            .bg(rgb(p.red_soft))
            .border_1()
            .border_color(rgb(p.red_border))
            .child(
                div()
                    .text_size(px(10.))
                    .font_medium()
                    .text_color(rgb(p.red))
                    .child(summary),
            )
            .when_some(detail, |this, detail| {
                this.child(
                    div()
                        .text_size(px(9.))
                        .text_color(rgb(p.muted))
                        .child(detail),
                )
            })
    }

    fn ai_primary_action_button(
        &self,
        action: AiPluginAction,
        retry: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (label, danger) = match action {
            AiPluginAction::InstallZip => (tr!("settings.ai.install"), false),
            AiPluginAction::DownloadModel => (
                if retry {
                    tr!("settings.ai.retry")
                } else {
                    tr!("settings.ai.download")
                },
                false,
            ),
            AiPluginAction::ImportModel => (tr!("settings.ai.import_model"), false),
            AiPluginAction::Recheck => (tr!("settings.ai.recheck"), false),
            AiPluginAction::DeleteModel => (tr!("settings.ai.delete_model"), false),
            AiPluginAction::Uninstall => (tr!("settings.ai.uninstall"), true),
            AiPluginAction::None => return div().into_any_element(),
        };
        self.ai_action_button(
            AiActionButton {
                id: "ai-primary-action",
                action,
                label,
                primary: true,
                danger,
            },
            cx,
        )
    }

    fn ai_action_button(&self, button: AiActionButton, cx: &mut Context<Self>) -> AnyElement {
        let AiActionButton {
            id,
            action,
            label,
            primary,
            danger,
        } = button;
        let mut button = Button::new(id).label(label).small();
        if primary {
            button = button.primary();
        }
        if danger {
            button = button.danger();
        }
        button
            .on_click(cx.listener(move |this, _, window, cx| match action {
                AiPluginAction::InstallZip => this.install_ai_plugin(window, cx),
                AiPluginAction::DownloadModel => this.request_ai_model_download(window, cx),
                AiPluginAction::ImportModel => this.import_ai_model_files(window, cx),
                AiPluginAction::Recheck => this.probe_ai_plugin(cx),
                AiPluginAction::DeleteModel => this.delete_ai_model(cx),
                AiPluginAction::Uninstall => {
                    this.open_ai_uninstall_modal(UninstallTarget::plugin_wide(), window, cx)
                }
                AiPluginAction::None => {}
            }))
            .into_any_element()
    }

    pub(super) fn ai_license_row(&self, label: String, value: String) -> impl IntoElement {
        let p = self.palette;
        v_flex()
            .gap(px(2.))
            .child(
                div()
                    .text_size(px(9.))
                    .text_color(rgb(p.muted_2))
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text))
                    .child(value),
            )
    }
}
