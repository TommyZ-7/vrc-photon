//! Integrations & automation screen: a master list of Discord, automatic
//! compression, AI tagging, and clipboard, with the selected item's settings
//! in the detail pane (#142, #448).

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vrc_photon_ai_protocol::TaggingStage;
use vrc_photon_i18n::tr;

use crate::app::App;
use crate::app::IntegrationsSection;
use crate::app::ai::{
    backend_options, backend_selection_is_disabled_when_not_ready, effective_ai_tagging_enabled,
    variant_backend_label,
};
use crate::app::compression::FeatureCompressionTarget;
use vrc_photon_core::queue::QueueFeature;

use crate::ui::auto_behavior::{auto_mode_segmented, bulk_action_available, bulk_action_block};
use crate::ui::components::{card_divider, chip, compression_policy_warning, glass_panel};
use crate::ui::queue::queue_section;
use crate::ui::theme::*;

impl App {
    pub(crate) fn integrations_screen(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let viewport = content_viewport(window);
        let stacked = integrations_stacks(f32::from(viewport.width), self.sidebar_collapsed);
        let selected = self.integrations_selected;

        let list = self.integrations_list(selected, stacked, cx);

        let detail_body = v_flex()
            .id("integrations-detail")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.integrations_scroll)
            .gap(px(CARD_GAP));
        let detail_body = match selected {
            IntegrationsSection::Discord => detail_body.child(self.discord_card(cx)),
            IntegrationsSection::AutoCompression => {
                detail_body.child(self.auto_compression_card(cx))
            }
            IntegrationsSection::AiTagging => detail_body.child(self.ai_tagging_card(cx)),
            IntegrationsSection::Clipboard => detail_body.child(self.clipboard_card(cx)),
        };
        let detail = h_flex()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .items_stretch()
            .child(detail_body)
            .vertical_scrollbar(&self.integrations_scroll);

        if stacked {
            v_flex()
                .flex_1()
                .min_h_0()
                .gap(px(CARD_GAP))
                .child(list)
                .child(detail)
                .into_any_element()
        } else {
            h_flex()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .gap(px(INTEGRATIONS_PANE_GAP))
                .child(list)
                .child(detail)
                .into_any_element()
        }
    }

    /// Master list of the four integration sections. Each row carries the
    /// same title as its detail card and a status dot.
    fn integrations_list(
        &self,
        selected: IntegrationsSection,
        stacked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        let ai_enabled =
            effective_ai_tagging_enabled(self.settings.ai_tagging_enabled, self.ai.installed());

        let items = [
            (
                IntegrationsSection::Discord,
                "integrations-list-discord",
                tr!("integrations.discord.title"),
                self.discord_url.is_some(),
            ),
            (
                IntegrationsSection::AutoCompression,
                "integrations-list-auto",
                tr!("integrations.auto.title"),
                self.settings.auto_compress_enabled,
            ),
            (
                IntegrationsSection::AiTagging,
                "integrations-list-ai",
                tr!("integrations.ai.title"),
                ai_enabled,
            ),
            (
                IntegrationsSection::Clipboard,
                "integrations-list-clipboard",
                tr!("integrations.clipboard.title"),
                true,
            ),
        ];

        let rows =
            v_flex()
                .gap(px(4.))
                .children(items.into_iter().map(|(section, id, name, on)| {
                    let active = section == selected;
                    v_flex()
                        .id(SharedString::from(id))
                        .p(px(11.))
                        .rounded(px(9.))
                        .border_1()
                        .border_color(if active {
                            rgba(with_alpha(p.accent, 0x66))
                        } else {
                            rgba(0x00000000)
                        })
                        .when(active, |this| this.bg(rgb(p.accent_soft)))
                        .cursor_pointer()
                        .when(!active, |this| this.hover(|this| this.bg(rgb(p.panel_2))))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.integrations_selected = section;
                            this.integrations_scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                        }))
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().size(px(8.)).rounded_full().bg(rgb(if on {
                                    p.green
                                } else {
                                    p.muted_2
                                })))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_semibold()
                                        .text_color(rgb(p.text))
                                        .child(name),
                                ),
                        )
                        .into_any_element()
                }));

        glass_panel(&p)
            .id("integrations-list")
            .flex_shrink_0()
            .p(px(8.))
            .rounded(px(17.))
            .when(!stacked, |this| this.w(px(INTEGRATIONS_LIST_WIDTH)))
            .when(stacked, |this| this.w_full())
            .child(rows)
            .into_any_element()
    }

    fn auto_compression_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let enabled = self.settings.auto_compress_enabled;
        let options = self.settings.auto_compression.clone();
        let behavior = self.settings.auto_compress;

        let body = v_flex()
            .pt(px(12.))
            .gap(px(12.))
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.startup.keep_original"),
                "",
                self.settings.auto_compress_keep_original,
                false,
                cx,
                |settings, value| settings.auto_compress_keep_original = value,
            ))
            .when_some(
                compression_policy_warning(self.settings.auto_compress_keep_original, enabled),
                |this, warning| {
                    this.child(card_divider(&p)).child(
                        div()
                            .p(px(12.))
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
            .child(self.section_label(tr!("integrations.compression.section")))
            .child(self.compression_options_editor(
                "auto-compression",
                &options,
                FeatureCompressionTarget::Auto,
                cx,
            ))
            .child(card_divider(&p))
            .child(auto_mode_segmented(
                &p,
                "integrations-auto",
                behavior.mode,
                cx,
                App::set_auto_compress_mode,
            ))
            .child(card_divider(&p))
            .child(bulk_action_block(
                &p,
                "integrations-auto",
                behavior,
                enabled,
                tr!("integrations.auto.process_uncompressed"),
                cx,
                App::set_auto_compress_scope,
                App::process_uncompressed_compression_action,
            ))
            .child(queue_section(
                self,
                QueueFeature::Compression,
                &self.compression_queue_entries,
                self.compression_queue_counts,
                self.settings.auto_compress_queue_paused,
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
                        v_flex().flex_1().min_w_0().child(
                            div()
                                .text_size(px(13.))
                                .font_semibold()
                                .text_color(rgb(p.text))
                                .child(tr!("integrations.auto.title")),
                        ),
                    )
                    .child(
                        Switch::new("integrations-auto-enabled")
                            .checked(enabled)
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.set_auto_compress_enabled(*checked, cx);
                            })),
                    ),
            )
            .child(self.integration_settings_body(enabled, body))
            .into_any_element()
    }

    fn ai_tagging_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let ready = self.ai.ready();
        let installed = self.ai.installed();
        let enabled =
            effective_ai_tagging_enabled(self.settings.ai_tagging_enabled, installed);
        let behavior = self.settings.ai_tagging_auto;
        let backend_selection_disabled =
            backend_selection_is_disabled_when_not_ready(ready, enabled);
        let backends = self
            .ai
            .probe
            .as_ref()
            .map(|probe| backend_options(probe, &self.settings.ai_tagging_backend))
            .unwrap_or_default();

        let backend_section = v_flex()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(p.text))
                    .child(tr!("integrations.ai.backend")),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .children(backends.into_iter().map(|(id, selected)| {
                        let click_id = id.clone();
                        let label = if id == "auto" {
                            tr!("integrations.ai.backend.auto")
                        } else {
                            variant_backend_label(&id)
                        };
                        chip(&p, &label, selected)
                            .id(SharedString::from(format!("integrations-ai-backend-{id}")))
                            .opacity(if backend_selection_disabled { 0.5 } else { 1.0 })
                            .when(!backend_selection_disabled, |this| {
                                this.cursor_pointer().on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        let backend = click_id.clone();
                                        this.save_ai_runtime_settings(cx, move |settings| {
                                            settings.ai_tagging_backend = backend;
                                        });
                                    },
                                ))
                            })
                            .into_any_element()
                    })),
            );

        let body = v_flex()
            .pt(px(12.))
            .gap(px(12.))
            .when(ready, |this| {
                this.child(card_divider(&p)).child(backend_section)
            })
            .child(card_divider(&p))
            .child(auto_mode_segmented(
                &p,
                "integrations-ai",
                behavior.mode,
                cx,
                App::set_ai_auto_mode,
            ))
            .child(card_divider(&p))
            .child(bulk_action_block(
                &p,
                "integrations-ai",
                behavior,
                bulk_action_available(enabled, ready),
                tr!("integrations.ai.process_untagged"),
                cx,
                App::set_ai_auto_scope,
                App::process_untagged_ai_action,
            ))
            .child(queue_section(
                self,
                QueueFeature::Tagging,
                &self.tagging_queue_entries,
                self.tagging_queue_counts,
                self.settings.ai_tagging_queue_paused,
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
                        v_flex().flex_1().min_w_0().child(
                            div()
                                .text_size(px(13.))
                                .font_semibold()
                                .text_color(rgb(p.text))
                                .child(tr!("integrations.ai.title")),
                        ),
                    )
                    .when(installed, |this| {
                        this.child(
                            Switch::new("integrations-ai-enabled")
                                .checked(enabled)
                                .on_click(cx.listener(|this, checked, _, cx| {
                                    this.set_ai_tagging_enabled(*checked, cx);
                                })),
                        )
                    }),
            )
            .child(self.integration_settings_body(enabled, body))
            .into_any_element()
    }

    fn clipboard_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let resize = self.settings.clipboard_compression;

        let body = v_flex()
            .pt(px(12.))
            .gap(px(8.))
            .child(card_divider(&p))
            .child(self.section_label(tr!("compression.section.resize")))
            .child(self.resize_settings_editor("clipboard-resize", &resize, cx));

        glass_panel(&self.palette)
            .flex()
            .flex_col()
            .p(px(14.))
            .rounded(px(17.))
            .child(
                h_flex().items_center().gap(px(12.)).child(
                    v_flex().flex_1().min_w_0().gap(px(2.)).child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(p.text))
                            .child(tr!("integrations.clipboard.title")),
                    ),
                ),
            )
            .child(body)
            .into_any_element()
    }

    /// Shows a card's settings even while its integration is disabled, muted
    /// and inert instead of collapsing it out of view.
    pub(crate) fn integration_settings_body(
        &self,
        enabled: bool,
        body: impl IntoElement + 'static,
    ) -> AnyElement {
        div()
            .relative()
            .opacity(if enabled { 1.0 } else { 0.5 })
            .child(body)
            .when(!enabled, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .block_mouse_except_scroll(),
                )
            })
            .into_any_element()
    }
}

/// The localized stage description shown under one job's progress bar.
pub(crate) fn ai_stage_label(stage: Option<TaggingStage>) -> String {
    match stage {
        Some(TaggingStage::Presence) => tr!("integrations.ai.jobs.stage.presence"),
        Some(TaggingStage::PeopleCount) => tr!("integrations.ai.jobs.stage.people_count"),
        Some(TaggingStage::PlaceClass) => tr!("integrations.ai.jobs.stage.place_class"),
        Some(TaggingStage::PlaceTags) => tr!("integrations.ai.jobs.stage.place_tags"),
        Some(TaggingStage::AtmosphereTags) => {
            tr!("integrations.ai.jobs.stage.atmosphere_tags")
        }
        Some(TaggingStage::Write) => tr!("integrations.ai.jobs.stage.write"),
        None => tr!("integrations.ai.jobs.stage.queued"),
    }
}

#[cfg(test)]
mod tests {
    use super::ai_stage_label;
    use vrc_photon_ai_protocol::TaggingStage;

    #[test]
    fn atmosphere_stage_has_a_label() {
        assert_eq!(
            ai_stage_label(Some(TaggingStage::AtmosphereTags)),
            "雰囲気タグを生成中"
        );
    }
}
