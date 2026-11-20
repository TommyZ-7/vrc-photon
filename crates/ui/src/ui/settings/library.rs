use super::*;

impl App {
    pub(super) fn library_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(tr!("settings.library.title")),
            )
            .child(card_divider(&p))
            .child(self.filter_field(
                tr!("settings.library.photo_roots"),
                Input::new(&self.photo_root_input),
            ))
            .child(card_divider(&p))
            .child(self.filter_field(
                tr!("settings.library.log_dir"),
                Input::new(&self.log_dir_input),
            ))
            .child(card_divider(&p))
            .child(self.filter_field(
                tr!("settings.library.import_dir"),
                Input::new(&self.import_dir_input),
            ))
            .child(card_divider(&p))
            .child(
                h_flex().gap(px(8.)).items_center().child(
                    Button::new("save-paths")
                        .label(tr!("settings.library.save_paths"))
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.save_library_paths(cx) {
                                this.toast_success(tr!("settings.library.saved"), cx);
                            }
                        })),
                ),
            )
    }

    pub(super) fn world_name_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(tr!("settings.world_name.title")),
            )
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.world_name.enabled"),
                tr!("settings.world_name.hint"),
                self.settings.current.world_name_resolution_enabled,
                false,
                cx,
                |settings, value| settings.world_name_resolution_enabled = value,
            ))
    }

    /// Saves the three library path fields and refreshes the library/daemon.
    /// Returns false when the database write failed and the settings were
    /// rolled back.
    pub(crate) fn save_library_paths(&mut self, cx: &mut Context<Self>) -> bool {
        let root = self.photo_root_input.read(cx).value().to_string();
        let log = self.log_dir_input.read(cx).value().to_string();
        let imported = self.import_dir_input.read(cx).value().to_string();
        let previous = self.settings.current.clone();
        let changes = apply_library_path_inputs(&mut self.settings.current, &root, &log, &imported);
        if let Err(error) = self.settings.current.save(&self.db) {
            self.settings.current = previous;
            let message = tr_args!("settings.language.save_failed", error = error);
            self.toast_error(message.clone(), cx);
            if self.setup.open {
                // The wizard queues toasts until it closes, so a failed
                // page-leave must also surface inline or Next looks dead.
                self.setup.error = Some(message.into());
            }
            tracing::warn!(%error, "cannot save path settings");
            cx.notify();
            return false;
        }
        if changes.root {
            let revision = self.db.library_revision().ok();
            self.apply_library_snapshot_with_anchor(revision, false, cx);
        }
        if changes.any {
            if let Err(error) = self.db.request_daemon_command(DaemonCommand::Rescan) {
                tracing::warn!(%error, "cannot request daemon rescan");
            } else {
                self.send_wake_commands(cx);
            }
        }
        cx.notify();
        true
    }
}
