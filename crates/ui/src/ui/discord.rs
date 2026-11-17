//! Discord integration cards: webhook editing, destination label, test send,
//! the compression settings used for uploads, and the shared queue section.
//!
//! The full webhook URL never reaches the screen: a stored URL is rendered
//! through [`mask_webhook_url`] and only the edit field shows what the user is
//! typing.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vrc_photon_core::discord::{mask_webhook_url, validate_webhook_url};
use vrc_photon_core::model::{CompressionFormat, CompressionOptions};
use vrc_photon_i18n::tr;

use crate::app::App;
use crate::app::compression::FeatureCompressionTarget;
use crate::app::discord::discord_backlog_available;
use crate::ui::auto_behavior::{auto_mode_segmented, bulk_action_block};
use crate::ui::components::{card_divider, glass_panel};

/// Warning shown when the Discord compression settings use a format Discord
/// embeds do not render. Sending is not blocked: the upload still works and
/// Discord may show a download link instead of an inline preview.
pub(crate) fn discord_format_warning(options: &CompressionOptions) -> Option<String> {
    matches!(
        options.format,
        CompressionFormat::Avif | CompressionFormat::JpegXl
    )
    .then(|| tr!("integrations.discord.compression.format_warning"))
}

/// Inline message for a stored webhook that no longer validates.
fn stored_webhook_error(url: Option<&str>) -> Option<String> {
    url.filter(|url| validate_webhook_url(url).is_err())
        .map(|_| tr!("integrations.discord.webhook.invalid_url"))
}

impl App {
    pub(crate) fn discord_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let enabled = self.settings.discord_enabled;
        let webhook = self.discord_url.clone();
        let editing = self.discord_url_editing;
        let url_error = self.discord_url_error.clone();
        let destination_value = self.discord_destination_input.read(cx).value().to_string();
        let stored_destination = self.settings.discord_destination.clone();
        let can_test = enabled
            && webhook
                .as_deref()
                .is_some_and(|url| validate_webhook_url(url).is_ok());
        let stored_webhook_error = stored_webhook_error(webhook.as_deref());
        let send_original = self.settings.discord_send_original;
        let include_players = self.settings.discord_include_players;
        let options = self.settings.discord_compression.clone();
        let format_warning = discord_format_warning(&options);

        let body = v_flex()
            .pt(px(12.))
            .gap(px(12.))
            .child(card_divider(&p))
            .child(self.webhook_section(webhook.as_deref(), editing, url_error, cx))
            .when_some(stored_webhook_error, |this, error| {
                this.child(card_divider(&p)).child(
                    div()
                        .px(px(4.))
                        .text_size(px(10.))
                        .text_color(rgb(p.red))
                        .child(error),
                )
            })
            .child(card_divider(&p))
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.text))
                            .child(tr!("integrations.discord.destination.label")),
                    )
                    .child(Input::new(&self.discord_destination_input).small())
                    // Enter/blur commit through the Task 11 subscription, which
                    // has no window to rewrite the field. Blank input is saved
                    // as the default, so show that normalized value instead of
                    // leaving the field looking unsaved.
                    .when(destination_value.trim().is_empty(), |this| {
                        this.child(div().text_size(px(10.)).text_color(rgb(p.muted_2)).child(
                            format!(
                                "{}: {stored_destination}",
                                tr!("integrations.discord.destination.label")
                            ),
                        ))
                    }),
            )
            .child(card_divider(&p))
            .child(
                h_flex().items_center().gap(px(8.)).child(
                    Button::new("discord-test-send")
                        .label(tr!("integrations.discord.test.button"))
                        .small()
                        .disabled(!can_test)
                        .on_click(cx.listener(|this, _, _, cx| this.send_test_message(cx))),
                ),
            )
            .child(card_divider(&p))
            .child(self.discord_toggle_row(
                "discord-send-original",
                tr!("integrations.discord.source.prefer_original"),
                send_original,
                cx,
                App::set_discord_send_original,
            ))
            .child(card_divider(&p))
            .child(self.discord_toggle_row(
                "discord-include-players",
                tr!("integrations.discord.players.label"),
                include_players,
                cx,
                App::set_discord_include_players,
            ))
            .child(card_divider(&p))
            .child(self.section_label(tr!("integrations.compression.section")))
            .child(self.compression_options_editor(
                "discord-compression",
                &options,
                FeatureCompressionTarget::Discord,
                cx,
            ))
            .when_some(format_warning, |this, warning| {
                this.child(
                    div()
                        .p(px(8.))
                        .rounded(px(8.))
                        .bg(rgb(p.amber_soft))
                        .text_size(px(10.))
                        .text_color(rgb(p.amber))
                        .child(warning),
                )
            })
            .child(card_divider(&p))
            .child(self.discord_auto_section(cx))
            .child(crate::ui::queue::queue_section(
                self,
                vrc_photon_core::queue::QueueFeature::Discord,
                &self.discord_queue_entries,
                self.discord_queue_counts,
                enabled,
                self.settings.discord_queue_paused,
                cx,
            ));

        glass_panel(&self.palette)
            .flex()
            .flex_col()
            .p(px(14.))
            .rounded(px(17.))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        v_flex().flex_1().min_w_0().gap(px(2.)).child(
                            div()
                                .text_size(px(13.))
                                .font_semibold()
                                .text_color(rgb(p.text))
                                .child(tr!("integrations.discord.title")),
                        ),
                    )
                    .child(
                        Switch::new("integrations-discord-enabled")
                            .checked(enabled)
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.set_discord_enabled(*checked, cx);
                            })),
                    ),
            )
            .child(self.integration_settings_body(enabled, body))
            .into_any_element()
    }

    fn webhook_section(
        &self,
        webhook: Option<&str>,
        editing: bool,
        error: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;

        if editing {
            return v_flex()
                .gap(px(6.))
                .child(Input::new(&self.discord_url_input).small())
                .when_some(error, |this, error| {
                    this.child(div().text_size(px(10.)).text_color(rgb(p.red)).child(error))
                })
                .child(
                    h_flex()
                        .gap(px(8.))
                        .child(
                            Button::new("discord-webhook-save")
                                .label(tr!("integrations.discord.webhook.save"))
                                .small()
                                .primary()
                                .on_click(cx.listener(|this, _, _, cx| this.save_discord_url(cx))),
                        )
                        .child(
                            Button::new("discord-webhook-cancel")
                                .label(tr!("integrations.discord.webhook.cancel"))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.discord_url_editing = false;
                                    this.discord_url_error = None;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element();
        }

        match webhook {
            Some(url) => h_flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(8.))
                        .bg(rgb(p.panel_2))
                        .text_size(px(11.))
                        .text_color(rgb(p.muted))
                        .child(mask_webhook_url(url)),
                )
                .child(
                    Button::new("discord-webhook-change")
                        .label(tr!("integrations.discord.webhook.change"))
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.begin_discord_url_edit(window, cx);
                        })),
                )
                .child(
                    Button::new("discord-webhook-remove")
                        .label(tr!("integrations.discord.webhook.remove"))
                        .small()
                        .danger()
                        .on_click(cx.listener(|this, _, _, cx| this.clear_discord_url(cx))),
                )
                .into_any_element(),
            None => h_flex()
                .items_center()
                .gap(px(8.))
                .child(
                    Button::new("discord-webhook-add")
                        .label(tr!("integrations.discord.webhook.add"))
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.begin_discord_url_edit(window, cx);
                        })),
                )
                .into_any_element(),
        }
    }

    fn discord_toggle_row(
        &self,
        id: &'static str,
        label: String,
        checked: bool,
        cx: &mut Context<Self>,
        toggle: fn(&mut App, bool, &mut Context<App>),
    ) -> AnyElement {
        let p = self.palette;
        h_flex()
            .items_center()
            .gap(px(12.))
            .py(px(2.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .text_color(rgb(p.text))
                    .child(label),
            )
            .child(Switch::new(id).checked(checked).on_click(cx.listener(
                move |this, value, _, cx| {
                    toggle(this, *value, cx);
                },
            )))
            .into_any_element()
    }

    /// Off / Immediate / Waiting plus the independent bulk action. The bulk
    /// action stays available while the automatic mode is off as long as the
    /// integration and webhook are valid.
    fn discord_auto_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let behavior = self.settings.discord_auto;
        let bulk_enabled = discord_backlog_available(&self.settings, self.discord_url.as_deref());
        v_flex()
            .gap(px(12.))
            .child(auto_mode_segmented(
                &p,
                "integrations-discord",
                behavior.mode,
                cx,
                App::set_discord_auto_mode,
            ))
            .child(card_divider(&p))
            .child(bulk_action_block(
                &p,
                "integrations-discord",
                behavior,
                bulk_enabled,
                tr!("integrations.discord.backlog.button"),
                cx,
                App::set_discord_auto_scope,
                App::send_discord_backlog,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{discord_format_warning, mask_webhook_url, stored_webhook_error};
    use vrc_photon_core::model::{CompressionFormat, CompressionOptions};
    use vrc_photon_i18n::tr;

    #[test]
    fn stored_webhook_never_exposes_the_token() {
        let masked = mask_webhook_url("https://discord.com/api/webhooks/1/secret");
        assert!(masked.contains("••••"), "{masked}");
        assert!(!masked.contains("secret"), "{masked}");
    }

    #[test]
    fn discord_format_warning_uses_the_discord_options() {
        let avif = CompressionOptions {
            format: CompressionFormat::Avif,
            ..CompressionOptions::discord_default()
        };
        assert!(discord_format_warning(&avif).is_some());
        let jxl = CompressionOptions {
            format: CompressionFormat::JpegXl,
            ..CompressionOptions::discord_default()
        };
        assert!(discord_format_warning(&jxl).is_some());
        let webp = CompressionOptions::discord_default();
        assert!(discord_format_warning(&webp).is_none());
    }

    #[test]
    fn stored_webhooks_that_no_longer_validate_show_the_inline_error() {
        assert!(stored_webhook_error(None).is_none());
        assert!(stored_webhook_error(Some("https://discord.com/api/webhooks/1/token")).is_none());
        assert_eq!(
            stored_webhook_error(Some("http://discord.com/api/webhooks/1/token")),
            Some(tr!("integrations.discord.webhook.invalid_url"))
        );
    }
}
