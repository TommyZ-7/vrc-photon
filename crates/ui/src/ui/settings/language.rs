use super::*;

impl App {
    pub(super) fn language_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let active = vrc_photon_i18n::normalize(&self.settings.current.language);
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
                    .child(tr!("settings.language.title")),
            )
            .child(card_divider(&p))
            .child(h_flex().flex_wrap().gap(px(4.)).children(
                vrc_photon_i18n::locales().into_iter().map(|locale| {
                    let selected = locale.code == active;
                    let code = locale.code.clone();
                    div()
                        .id(SharedString::from(format!("language-{}", locale.code)))
                        .px(px(10.))
                        .py(px(5.))
                        .rounded(px(8.))
                        .bg(if selected {
                            rgb(p.accent_soft)
                        } else {
                            rgba(0x00000000)
                        })
                        .text_size(px(11.))
                        .font_medium()
                        .text_color(if selected {
                            rgb(p.accent)
                        } else {
                            rgb(p.muted)
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_language(&code, window, cx)
                        }))
                        .child(locale.name)
                }),
            ))
    }
}
