use super::*;

fn developer_avatar() -> std::sync::Arc<Image> {
    static AVATAR: std::sync::OnceLock<std::sync::Arc<Image>> = std::sync::OnceLock::new();
    AVATAR
        .get_or_init(|| {
            std::sync::Arc::new(Image::from_bytes(
                ImageFormat::Jpeg,
                include_bytes!("../../../assets/devinfo-avatar.jpg").to_vec(),
            ))
        })
        .clone()
}

impl App {
    pub(super) fn developer_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let rows = developer_info_rows();

        let mut info = v_flex().flex_1().min_w_0();
        for (index, (label, value)) in rows.into_iter().enumerate() {
            if index > 0 {
                info = info.child(card_divider(&p));
            }
            let value = match index {
                2 => self
                    .developer_link(
                        "developer-repository",
                        value,
                        self.developer_repository_focus.clone(),
                        DEVELOPER_REPOSITORY_URL,
                        cx,
                    )
                    .into_any_element(),
                3 => self
                    .developer_link(
                        "developer-x",
                        value,
                        self.developer_x_focus.clone(),
                        DEVELOPER_X_URL,
                        cx,
                    )
                    .into_any_element(),
                _ => div()
                    .text_size(px(12.))
                    .text_color(rgb(p.text))
                    .child(value)
                    .into_any_element(),
            };
            info = info.child(
                v_flex()
                    .gap(px(1.))
                    .py(px(7.))
                    .when(index == 0, |this| this.pt(px(0.)))
                    .when(index == 3, |this| this.pb(px(0.)))
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(rgb(p.muted_2))
                            .child(label),
                    )
                    .child(value),
            );
        }

        glass_panel(&self.palette)
            .flex()
            .flex_col()
            .p(px(14.))
            .rounded(px(17.))
            .child(
                h_flex()
                    .items_stretch()
                    .gap(px(14.))
                    .child(
                        v_flex()
                            .w(px(148.))
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .gap(px(8.))
                            .child(
                                img(developer_avatar())
                                    .size(px(64.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(p.border))
                                    .object_fit(ObjectFit::Cover),
                            )
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_semibold()
                                    .text_color(rgb(p.text))
                                    .child("TommyZ7"),
                            ),
                    )
                    .child(div().w(px(1.)).flex_shrink_0().bg(rgba(p.line_soft)))
                    .child(info),
            )
    }

    fn developer_link(
        &self,
        id: &'static str,
        label: String,
        focus: FocusHandle,
        url: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette;
        let click_focus = focus.clone();
        let key_focus = focus.clone();
        div()
            .id(id)
            .self_start()
            .text_size(px(12.))
            .text_color(rgb(p.accent))
            .cursor_pointer()
            .track_focus(&focus)
            .tab_stop(true)
            .hover(|this| this.underline())
            .focus(|this| this.underline())
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if matches!(event, ClickEvent::Keyboard(_)) {
                    return;
                }
                window.focus(&click_focus, cx);
                if let Err(error) = open_path(Path::new(url)) {
                    tracing::warn!(%error, %url, "cannot open the developer link");
                    this.toast_error(tr!("settings.developer.open_failed"), cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if !event.is_held && (key == "enter" || key == "space") {
                    window.prevent_default();
                    cx.stop_propagation();
                    window.focus(&key_focus, cx);
                    if let Err(error) = open_path(Path::new(url)) {
                        tracing::warn!(%error, %url, "cannot open the developer link");
                        this.toast_error(tr!("settings.developer.open_failed"), cx);
                    }
                }
            }))
            .child(label)
    }
}
