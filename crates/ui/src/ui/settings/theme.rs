use super::*;

/// One theme swatch: its axis and position plus the values the swatch renders.
struct ThemeSwatch {
    axis: ThemeAxis,
    index: usize,
    id: &'static str,
    label: String,
    swatch: u32,
    selected: bool,
}

impl App {
    pub(super) fn theme_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(tr!("settings.theme.title")),
            )
            .child(card_divider(&p))
            .child(self.theme_axis(ThemeAxis::Main, cx))
            .child(card_divider(&p))
            .child(self.theme_axis(ThemeAxis::Accent, cx))
    }

    pub(crate) fn theme_axis(&self, axis: ThemeAxis, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let selected_main = find_main(&self.settings.current.ui_theme_main);
        let selected_accent = find_accent(&self.settings.current.ui_theme_accent);
        let selected_label = match axis {
            ThemeAxis::Main => selected_main.label(),
            ThemeAxis::Accent => selected_accent.label(),
        };
        let items: Vec<(&'static str, String, u32, bool)> = match axis {
            ThemeAxis::Main => main_colors()
                .iter()
                .map(|color| {
                    (
                        color.id,
                        color.label(),
                        preview_main(color),
                        selected_main.id == color.id,
                    )
                })
                .collect(),
            ThemeAxis::Accent => accent_colors()
                .iter()
                .map(|color| {
                    (
                        color.id,
                        color.label(),
                        preview_accent(color),
                        selected_accent.id == color.id,
                    )
                })
                .collect(),
        };
        // GPUI tooltips only appear on hover, so while a swatch in this
        // axis holds keyboard focus its color name is repeated in the
        // heading, in the accent color, next to the selected color name.
        let focused_label = self
            .theme_focused
            .filter(|(focused_axis, _)| *focused_axis == axis)
            .and_then(|(_, index)| items.get(index))
            .map(|item| item.1.clone())
            .filter(|label| *label != selected_label);
        let mut rows = v_flex().gap(px(8.));
        for (chunk_index, chunk) in items.chunks(6).enumerate() {
            let mut row = h_flex().items_center().gap(px(8.));
            for (offset, item) in chunk.iter().enumerate() {
                let (id, label, swatch, selected) = item;
                let index = chunk_index * 6 + offset;
                row = row.child(self.theme_swatch(
                    ThemeSwatch {
                        axis,
                        index,
                        id,
                        label: label.clone(),
                        swatch: *swatch,
                        selected: *selected,
                    },
                    cx,
                ));
            }
            rows = rows.child(row);
        }
        v_flex()
            .gap(px(8.))
            .child(
                h_flex()
                    .items_baseline()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_medium()
                            .text_color(rgb(p.text))
                            .child(axis.title()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted))
                            .child(format!("· {selected_label}")),
                    )
                    .when_some(focused_label, |this, label| {
                        this.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.accent))
                                .child(format!("· {label}")),
                        )
                    }),
            )
            .child(rows)
    }

    fn theme_swatch(&self, swatch: ThemeSwatch, cx: &mut Context<Self>) -> impl IntoElement {
        let ThemeSwatch {
            axis,
            index,
            id,
            label,
            swatch,
            selected,
        } = swatch;
        let p = self.palette;
        let focus = match axis {
            ThemeAxis::Main => self.theme_main_focus[index].clone(),
            ThemeAxis::Accent => self.theme_accent_focus[index].clone(),
        };
        let click_focus = focus.clone();
        let key_focus = focus;
        div()
            .id(SharedString::from(format!("theme-{}-{id}", axis.key())))
            .size(px(30.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_2()
            .border_color(if selected {
                rgb(p.accent_border)
            } else {
                rgba(0x00000000)
            })
            .cursor_pointer()
            .track_focus(&click_focus)
            .tab_stop(true)
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .hover(move |this| this.bg(rgb(p.panel_inner)))
            .focus(move |this| this.border_color(rgb(p.accent)).bg(rgb(p.panel_inner)))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                // Keyboard activation is handled on key down; ignore the
                // click GPUI synthesizes on key up so the theme is not
                // applied twice.
                if matches!(event, ClickEvent::Keyboard(_)) {
                    return;
                }
                window.focus(&click_focus, cx);
                match axis {
                    ThemeAxis::Main => this.set_main_theme(id, cx),
                    ThemeAxis::Accent => this.set_accent_theme(id, cx),
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if !event.is_held && (key == "enter" || key == "space") {
                    window.prevent_default();
                    cx.stop_propagation();
                    window.focus(&key_focus, cx);
                    match axis {
                        ThemeAxis::Main => this.set_main_theme(id, cx),
                        ThemeAxis::Accent => this.set_accent_theme(id, cx),
                    }
                }
            }))
            .child(
                div()
                    .size(px(24.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rgb(swatch))
                    .border_1()
                    .border_color(rgba(with_alpha(p.text, 0x24)))
                    .when(selected, |this| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .font_semibold()
                                .text_color(rgb(readable_on(swatch)))
                                .child("✓"),
                        )
                    }),
            )
    }

    pub(crate) fn set_main_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = find_main(id);
        if self.settings.current.ui_theme_main == id.id {
            return;
        }
        let previous = (self.settings.current.ui_theme_main.clone(), self.palette);
        self.settings.current.ui_theme_main = id.id.to_string();
        let palette = resolve(
            &self.settings.current.ui_theme_main,
            &self.settings.current.ui_theme_accent,
        );
        self.palette = palette;
        if let Err(error) = self.settings.current.save(&self.db) {
            self.settings.current.ui_theme_main = previous.0;
            self.palette = previous.1;
            self.toast_error(tr_args!("settings.language.save_failed", error = error), cx);
            tracing::warn!(%error, "cannot save theme setting");
            cx.notify();
            return;
        }
        self.apply_palette(self.palette, cx);
    }

    pub(crate) fn set_accent_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = find_accent(id);
        if self.settings.current.ui_theme_accent == id.id {
            return;
        }
        let previous = (self.settings.current.ui_theme_accent.clone(), self.palette);
        self.settings.current.ui_theme_accent = id.id.to_string();
        let palette = resolve(
            &self.settings.current.ui_theme_main,
            &self.settings.current.ui_theme_accent,
        );
        self.palette = palette;
        if let Err(error) = self.settings.current.save(&self.db) {
            self.settings.current.ui_theme_accent = previous.0;
            self.palette = previous.1;
            self.toast_error(tr_args!("settings.language.save_failed", error = error), cx);
            tracing::warn!(%error, "cannot save theme setting");
            cx.notify();
            return;
        }
        self.apply_palette(self.palette, cx);
    }
}
