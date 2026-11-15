//! First-run setup wizard.
//!
//! The wizard replaces the shell while it is open. It keeps only a minimal
//! titlebar so the window can still be moved and closed through the platform
//! controls.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vrc_photon_core::model::CompressionFormat;
use vrc_photon_core::thumbnail::THUMBNAIL_SIZES;
use vrc_photon_i18n::{tr, tr_args};

use crate::app::App;
use crate::app::compression::FeatureCompressionTarget;
use crate::app::setup::{SetupPage, SetupPathField};
use crate::ui::components::{card_divider, compression_policy_warning, dialog_panel};
use crate::ui::settings::ThemeAxis;
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
            SetupPage::Display => self.setup_display(cx).into_any_element(),
            SetupPage::Discord => self.setup_discord(cx).into_any_element(),
            SetupPage::Compression => self.setup_compression(cx),
            SetupPage::Theme => self.setup_theme(cx).into_any_element(),
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

    fn setup_display(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette;
        let left = v_flex()
            .w(px(240.))
            .flex_shrink_0()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("settings.display.title")),
            )
            .child(
                v_flex()
                    .gap(px(7.))
                    .child(setup_switch_row(
                        &p,
                        tr!("settings.display.status.label"),
                        "setup-card-status",
                        self.settings.ui_card_show_status,
                        cx,
                        |app, value, _, cx| {
                            app.settings.ui_card_show_status = value;
                            if let Err(error) = app.settings.save(&app.db) {
                                tracing::warn!(%error, "cannot save card status setting");
                            }
                            cx.notify();
                        },
                    ))
                    .child(setup_switch_row(
                        &p,
                        tr!("settings.display.world.label"),
                        "setup-card-world",
                        self.settings.ui_card_show_world,
                        cx,
                        |app, value, _, cx| {
                            app.settings.ui_card_show_world = value;
                            if let Err(error) = app.settings.save(&app.db) {
                                tracing::warn!(%error, "cannot save card world setting");
                            }
                            cx.notify();
                        },
                    ))
                    .child(setup_switch_row(
                        &p,
                        tr!("settings.display.date.label"),
                        "setup-card-date",
                        self.settings.ui_card_show_date,
                        cx,
                        |app, value, _, cx| {
                            app.settings.ui_card_show_date = value;
                            if let Err(error) = app.settings.save(&app.db) {
                                tracing::warn!(%error, "cannot save card date setting");
                            }
                            cx.notify();
                        },
                    ))
                    .child(setup_switch_row(
                        &p,
                        tr!("settings.display.size.label"),
                        "setup-card-size",
                        self.settings.ui_card_show_size,
                        cx,
                        |app, value, _, cx| {
                            app.settings.ui_card_show_size = value;
                            if let Err(error) = app.settings.save(&app.db) {
                                tracing::warn!(%error, "cannot save card size setting");
                            }
                            cx.notify();
                        },
                    )),
            )
            .child(card_divider(&p))
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        h_flex()
                            .items_baseline()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text))
                                    .child(tr!("settings.display.grid_gap.label")),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_semibold()
                                    .text_color(rgb(p.text))
                                    .child(format!("{} px", self.settings.ui_grid_gap)),
                            ),
                    )
                    .child(Slider::new(&self.grid_gap_slider).w_full()),
            )
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        h_flex()
                            .items_baseline()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text))
                                    .child(tr!("settings.display.card_radius.label")),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_semibold()
                                    .text_color(rgb(p.text))
                                    .child(format!("{} px", self.settings.ui_card_radius)),
                            ),
                    )
                    .child(Slider::new(&self.card_radius_slider).w_full()),
            )
            .child(card_divider(&p))
            .child(
                div()
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("settings.thumbnail.title")),
            )
            .child(h_flex().gap(px(4.)).children(
                THUMBNAIL_SIZES.into_iter().map(|size| {
                    let active = self.settings.thumbnail_size == size;
                    div()
                        .id(SharedString::from(format!("setup-thumbnail-{size}")))
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
                        .text_color(if active {
                            rgb(p.accent)
                        } else {
                            rgb(p.muted)
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_thumbnail_size(size, cx)
                        }))
                        .child(format!("{size}×{}", size * 9 / 16))
                }),
            ));

        h_flex()
            .items_start()
            .gap(px(14.))
            .child(left)
            .child(self.setup_display_preview())
    }

    fn setup_display_preview(&self) -> Div {
        let p = self.palette;
        let radius = f32::from(self.settings.ui_card_radius);
        let gap = f32::from(self.settings.ui_grid_gap);
        let inner_radius = inner_corner_radius(radius, 1.0);
        let worlds = ["VRChat Home", "The Black Cat"];
        let dates = ["2026/11/15", "2026/11/14"];
        let sizes = ["1.2 MB", "980 KB"];
        let cards = h_flex().gap(px(gap)).children((0..2).map(|index| {
            let image = if index == 0 {
                linear_gradient(
                    135.,
                    linear_color_stop(rgba(with_alpha(p.accent, 0x66)), 0.),
                    linear_color_stop(rgba(with_alpha(p.panel_deep, 0xff)), 1.),
                )
            } else {
                linear_gradient(
                    135.,
                    linear_color_stop(rgba(with_alpha(p.green, 0x55)), 0.),
                    linear_color_stop(rgba(with_alpha(p.panel_deep, 0xff)), 1.),
                )
            };
            v_flex()
                .w(px(100.))
                .flex_shrink_0()
                .overflow_hidden()
                .rounded(px(radius))
                .border_1()
                .border_color(rgba(with_alpha(p.text, 0x24)))
                .bg(rgb(p.panel_2))
                .child(
                    div()
                        .relative()
                        .h(px(60.))
                        .w_full()
                        .rounded_tl(px(inner_radius))
                        .rounded_tr(px(inner_radius))
                        .bg(image)
                        .when(self.settings.ui_card_show_status, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .top(px(5.))
                                    .left(px(5.))
                                    .px(px(5.))
                                    .py(px(1.))
                                    .rounded_full()
                                    .bg(rgba(with_alpha(p.bg, 0xcc)))
                                    .text_size(px(7.5))
                                    .font_semibold()
                                    .text_color(rgb(p.accent))
                                    .child(tr!("photos.badge.compressed")),
                            )
                        }),
                )
                .child(
                    v_flex()
                        .p(px(5.))
                        .gap(px(2.))
                        .when(self.settings.ui_card_show_world, |this| {
                            this.child(
                                div()
                                    .truncate()
                                    .text_size(px(8.5))
                                    .font_medium()
                                    .text_color(rgb(p.text))
                                    .child(worlds[index]),
                            )
                        })
                        .when(
                            self.settings.ui_card_show_date || self.settings.ui_card_show_size,
                            |this| {
                                this.child(
                                    h_flex()
                                        .items_center()
                                        .gap(px(3.))
                                        .text_size(px(7.5))
                                        .text_color(rgb(p.muted_2))
                                        .when(self.settings.ui_card_show_date, |this| {
                                            this.child(dates[index])
                                        })
                                        .when(
                                            self.settings.ui_card_show_date
                                                && self.settings.ui_card_show_size,
                                            |this| this.child("·"),
                                        )
                                        .when(self.settings.ui_card_show_size, |this| {
                                            this.child(sizes[index])
                                        }),
                                )
                            },
                        ),
                )
        }));

        let surface = div()
            .p(px(8.))
            .rounded(px(10.))
            .border_1()
            .border_color(rgba(with_alpha(p.text, 0x12)))
            .bg(rgb(p.panel_deep))
            .child(cards);
        let thumbnail = self.settings.thumbnail_size;
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted_2))
                    .child(tr!("setup.display.preview")),
            )
            .child(surface)
            .child(
                h_flex()
                    .items_baseline()
                    .justify_between()
                    .gap(px(6.))
                    .text_size(px(9.))
                    .text_color(rgb(p.muted_2))
                    .child(tr_args!(
                        "setup.display.preview_thumbnail",
                        size = format!("{}×{}", thumbnail, thumbnail * 9 / 16)
                    ))
                    .child(tr!("setup.display.preview_live")),
            )
    }

    fn setup_theme(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette;
        v_flex()
            .gap(px(12.))
            .child(self.theme_axis(ThemeAxis::Main, cx))
            .child(card_divider(&p))
            .child(self.theme_axis(ThemeAxis::Accent, cx))
    }

    fn setup_discord(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette;
        let body = v_flex()
            .gap(px(14.))
            .child(setup_switch_row(
                &p,
                tr!("integrations.discord.title"),
                "setup-discord-enabled",
                self.settings.discord_enabled,
                cx,
                |app, value, _, cx| app.set_discord_enabled(value, cx),
            ))
            .child(card_divider(&p))
            .child(
                v_flex()
                    .gap(px(7.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(p.text))
                            .child(tr!("setup.discord.webhook_label")),
                    )
                    .child(Input::new(&self.discord_url_input).h(px(38.))),
            )
            .when_some(self.setup.error.clone(), |this, error| {
                this.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.red))
                        .child(error),
                )
            })
            .child(
                v_flex()
                    .gap(px(7.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(p.text))
                            .child(tr!("integrations.discord.destination.label")),
                    )
                    .child(Input::new(&self.discord_destination_input).h(px(34.))),
            )
            .child(setup_switch_row(
                &p,
                tr!("integrations.discord.players.label"),
                "setup-discord-players",
                self.settings.discord_include_players,
                cx,
                |app, value, _, cx| app.set_discord_include_players(value, cx),
            ))
            .child(setup_switch_row(
                &p,
                tr!("integrations.discord.source.prefer_original"),
                "setup-discord-original",
                self.settings.discord_send_original,
                cx,
                |app, value, _, cx| app.set_discord_send_original(value, cx),
            ));
        // Keep the page useful while the toggle is off; the settings screen
        // collapses the body, but a first-run user needs to see what enabling
        // Discord will configure.
        body
    }

    fn setup_compression(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let enabled = self.settings.auto_compress;
        let options = self.settings.auto_compression.clone();

        let settings_body = v_flex()
            .gap(px(7.))
            .child(setup_switch_row(
                &p,
                tr!("settings.startup.immediate"),
                "setup-auto-immediate",
                self.settings.auto_compress_immediate,
                cx,
                |app, value, _, cx| app.set_setup_immediate(value, cx),
            ))
            .child(setup_switch_row(
                &p,
                tr!("settings.startup.keep_original"),
                "setup-auto-keep",
                self.settings.auto_compress_keep_original,
                cx,
                |app, value, _, cx| app.set_setup_keep_original(value, cx),
            ))
            .when_some(
                compression_policy_warning(
                    self.settings.auto_compress_keep_original,
                    self.settings.auto_compress,
                ),
                |this, warning| {
                    this.child(
                        div()
                            .p(px(10.))
                            .rounded(px(8.))
                            .bg(rgb(p.red_soft))
                            .border_1()
                            .border_color(rgb(p.red_border))
                            .text_size(px(10.))
                            .text_color(rgb(p.red))
                            .child(warning),
                    )
                },
            )
            .child(card_divider(&p))
            .child(
                div()
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("integrations.compression.section")),
            )
            .child(h_flex().flex_wrap().gap(px(4.)).children(
                [
                    CompressionFormat::Jpeg,
                    CompressionFormat::Webp,
                    CompressionFormat::Avif,
                    CompressionFormat::JpegXl,
                ]
                .into_iter()
                .map(|format| {
                    let active = options.format == format;
                    let selected = format.clone();
                    div()
                        .id(SharedString::from(format!(
                            "setup-format-{}",
                            format.as_str()
                        )))
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
                        .text_color(if active {
                            rgb(p.accent)
                        } else {
                            rgb(p.muted)
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.edit_feature_compression(
                                FeatureCompressionTarget::Auto,
                                |options| options.set_format(selected.clone()),
                                cx,
                            )
                        }))
                        .child(format_label(&format))
                }),
            ))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .text_color(rgb(p.text))
                            .child(tr!("compression.lossless")),
                    )
                    .child(
                        Switch::new("setup-lossless")
                            .checked(options.lossless)
                            .disabled(!options.format.supports_lossless())
                            .on_click(cx.listener(|this, value, _, cx| {
                                this.edit_feature_compression(
                                    FeatureCompressionTarget::Auto,
                                    |options| options.lossless = *value,
                                    cx,
                                );
                            })),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(setup_switch_row(
                &p,
                tr!("settings.startup.auto_compress"),
                "setup-auto-enabled",
                enabled,
                cx,
                |app, value, _, cx| app.set_auto_compress_enabled(value, cx),
            ))
            .child(card_divider(&p))
            .child(self.integration_settings_body(enabled, settings_body))
            .into_any_element()
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
