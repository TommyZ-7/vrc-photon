//! First-run setup wizard.
//!
//! The wizard replaces the shell while it is open. It keeps only a minimal
//! titlebar so the window can still be moved and closed through the platform
//! controls.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::*;
use vrc_photon_i18n::tr;

use crate::app::App;
use crate::app::setup::{SetupPage, SetupPathField};
use crate::ui::components::dialog_panel;
use crate::ui::theme::*;

impl App {
    pub(crate) fn setup_screen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette;
        let title = setup_page_title(self.setup.page);

        let header = h_flex()
            .items_start()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(24.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(title),
            )
            .child(
                Button::new("setup-close")
                    .icon(Icon::new(IconName::Close))
                    .small()
                    .ghost()
                    .tooltip(tr!("setup.close"))
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_setup(cx))),
            );

        let body = v_flex().flex_1().min_h_0().pt(px(30.)).child(self.setup_body(cx));

        let mut footer = h_flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1());
        if self.setup.page.prev().is_some() {
            footer = footer.child(
                Button::new("setup-back")
                    .label(tr!("setup.back"))
                    .secondary()
                    .h(px(38.))
                    .px(px(18.))
                    .on_click(cx.listener(|this, _, _, cx| this.back_setup(cx))),
            );
        }
        let action = if self.setup.page.is_last() {
            tr!("setup.done")
        } else {
            tr!("setup.next")
        };
        footer = footer.child(
            Button::new("setup-next")
                .label(action)
                .primary()
                .h(px(38.))
                .px(px(18.))
                .on_click(cx.listener(|this, _, _, cx| this.advance_setup(cx))),
        );

        let panel = dialog_panel(&p)
            .id("setup-panel")
            .w(px(600.))
            .h(px(561.))
            .pt(px(36.))
            .px(px(38.))
            .pb(px(28.))
            .track_focus(&self.setup.focus)
            .focus_trap("setup", &self.setup.focus)
            .key_context("setup_modal")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.dismiss_setup(cx);
                }
            }))
            .child(header)
            .child(body)
            .child(footer);

        let mut root = v_flex()
            .size_full()
            .bg(rgba(p.glass))
            .font_family("Noto Sans JP")
            .child(self.setup_titlebar())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(24.))
                    .child(panel),
            );

        if self.ai.license_modal_open {
            root = root.child(self.ai_license_overlay(window, cx));
        }
        root
    }

    fn setup_titlebar(&self) -> AnyElement {
        let p = self.palette;
        let line = h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .gap(px(9.))
            .child(
                div()
                    .size(px(22.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .bg(rgb(p.accent_fill))
                    .child(
                        Icon::new(IconName::Asterisk)
                            .xsmall()
                            .text_color(rgb(p.on_accent)),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child("VRC PhotoN"),
            )
            .child(div().w(px(1.)).h(px(14.)).bg(rgba(p.highlight)))
            .child(div().flex_1());

        TitleBar::new()
            .h(px(46.))
            .flex_shrink_0()
            .pl(px(14.))
            .pr(px(8.))
            .border_color(rgba(p.highlight))
            .bg(rgba(0x00000000))
            .child(line)
            .into_any_element()
    }

    fn setup_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        match self.setup.page {
            SetupPage::Welcome => self.setup_welcome(cx).into_any_element(),
            SetupPage::Startup => self.setup_startup(cx).into_any_element(),
            SetupPage::Folders => self.setup_folders(cx).into_any_element(),
            SetupPage::Display => div().into_any_element(),
            SetupPage::Discord => div().into_any_element(),
            SetupPage::Compression => div().into_any_element(),
            SetupPage::Theme => div().into_any_element(),
            SetupPage::AiTagging => div().into_any_element(),
        }
    }

    fn setup_welcome(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette;
        let active = vrc_photon_i18n::normalize(&self.settings.language);
        let chips = h_flex().flex_wrap().gap(px(4.)).children(
            vrc_photon_i18n::locales().into_iter().map(|locale| {
                let selected = locale.code == active;
                let code = locale.code.clone();
                div()
                    .id(SharedString::from(format!("setup-language-{}", locale.code)))
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
        );
        v_flex()
            .gap(px(7.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(p.text))
                    .child(tr!("setup.language.label")),
            )
            .child(chips)
    }

    fn setup_startup(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette;
        v_flex()
            .gap(px(18.))
            .child(setup_switch_row(
                &p,
                tr!("settings.startup.autostart"),
                "setup-autostart",
                self.settings.autostart,
                cx,
                |app, value, _, cx| {
                    app.settings.autostart = value;
                    if let Err(error) = app.settings.save(&app.db) {
                        tracing::warn!(%error, "cannot save autostart setting");
                    }
                    cx.notify();
                },
            ))
            .child(setup_switch_row(
                &p,
                tr!("settings.startup.tray"),
                "setup-tray",
                self.settings.tray,
                cx,
                |app, value, _, cx| {
                    app.settings.tray = value;
                    if let Err(error) = app.settings.save(&app.db) {
                        tracing::warn!(%error, "cannot save tray setting");
                    }
                    cx.notify();
                },
            ))
            .child(setup_switch_row(
                &p,
                tr!("settings.startup.reduce_motion"),
                "setup-reduce-motion",
                self.settings.ui_reduce_motion,
                cx,
                |app, value, _, cx| app.set_reduce_motion(value, cx),
            ))
    }

    fn setup_folders(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette;
        v_flex()
            .gap(px(18.))
            .child(self.setup_folder_row(
                &p,
                tr!("settings.library.photo_roots"),
                "setup-photo-root",
                &self.photo_root_input,
                SetupPathField::PhotoRoot,
                cx,
            ))
            .child(self.setup_folder_row(
                &p,
                tr!("settings.library.log_dir"),
                "setup-log-dir",
                &self.log_dir_input,
                SetupPathField::LogDir,
                cx,
            ))
            .child(self.setup_folder_row(
                &p,
                tr!("settings.library.import_dir"),
                "setup-import-dir",
                &self.import_dir_input,
                SetupPathField::ImportDir,
                cx,
            ))
    }

    fn setup_folder_row(
        &self,
        p: &Palette,
        label: String,
        id: &'static str,
        input: &Entity<InputState>,
        field: SetupPathField,
        cx: &mut Context<Self>,
    ) -> Div {
        v_flex()
            .gap(px(7.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(p.text))
                    .child(label),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(Input::new(input).h(px(34.)).flex_1())
                    .child(
                        Button::new(id)
                            .label(tr!("setup.folders.change"))
                            .h(px(34.))
                            .px(px(12.))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pick_setup_folder(field, window, cx)
                            })),
                    ),
            )
    }
}

fn setup_switch_row(
    p: &Palette,
    label: String,
    id: &'static str,
    checked: bool,
    cx: &mut Context<App>,
    on_toggle: impl Fn(&mut App, bool, &mut Window, &mut Context<App>) + 'static,
) -> Div {
    h_flex()
        .items_center()
        .gap(px(12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .text_color(rgb(p.text))
                .child(label),
        )
        .child(
            Switch::new(id)
                .checked(checked)
                .on_click(cx.listener(move |app, value, window, cx| {
                    on_toggle(app, *value, window, cx)
                })),
        )
}

fn setup_page_title(page: SetupPage) -> String {
    match page {
        SetupPage::Welcome => tr!("setup.title.welcome"),
        SetupPage::Startup => tr!("settings.startup.title"),
        SetupPage::Folders => tr!("setup.folders.title"),
        SetupPage::Display => tr!("setup.display.title"),
        SetupPage::Discord => tr!("setup.discord.title"),
        SetupPage::Compression => tr!("integrations.auto.title"),
        SetupPage::Theme => tr!("settings.theme.title"),
        SetupPage::AiTagging => tr!("integrations.ai.title"),
    }
}
