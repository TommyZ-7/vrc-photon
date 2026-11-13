//! Integrations & automation screen: a master list of Discord, automatic
//! compression, AI tagging, and clipboard, with the selected item's settings
//! in the detail pane (#142, #448).

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vrc_photon_ai_protocol::{TaggingJobStatus, TaggingStage};
use vrc_photon_core::model::{CompressionOptions, ResizeSettings};
use vrc_photon_i18n::{tr, tr_args};

use crate::app::App;
use crate::app::IntegrationsSection;
use crate::app::Screen;
use crate::app::ai::{
    AiErrorKind, active_job_photo_label, ai_error_summary_in, backend_options,
    backend_selection_is_disabled_when_not_ready, download_progress_bar_ratio,
    variant_backend_label,
};
use crate::app::compression::FeatureCompressionTarget;
use crate::state::motion::ToggleMotion;
use crate::ui::components::{
    card_divider, chip, compression_policy_warning, glass_panel, manual_option_row,
    status_progress_bar,
};
use crate::ui::theme::*;

/// Generous bounds for the collapsed-body height animation. The profile lists
/// scroll above these sizes, so the body never exceeds its bound.
const AUTO_BODY_MAX: f32 = 560.0;
const CLIPBOARD_BODY_MAX: f32 = 400.0;
const AI_BODY_MAX: f32 = 700.0;

/// Which runtime setting a switch row writes.
#[derive(Clone, Copy)]
enum AiToggle {
    Enabled,
    Auto,
    Backlog,
}

fn on_off_label(enabled: bool) -> String {
    if enabled {
        tr!("integrations.summary.enabled")
    } else {
        tr!("integrations.summary.disabled")
    }
}

/// One-line Discord state for the master list.
pub(crate) fn discord_summary(configured: bool, destination: &str) -> String {
    let state = if configured {
        tr!("integrations.discord.webhook.configured")
    } else {
        tr!("integrations.discord.webhook.missing_short")
    };
    tr_args!(
        "integrations.summary.discord",
        state = state,
        destination = destination
    )
}

/// One-line automatic compression state for the master list.
pub(crate) fn auto_compression_summary(
    enabled: bool,
    options: &CompressionOptions,
) -> String {
    tr_args!(
        "integrations.summary.auto",
        state = on_off_label(enabled),
        compression = options_summary(options)
    )
}

/// One-line AI tagging state for the master list.
pub(crate) fn ai_tagging_summary(enabled: bool, running: i64, waiting: i64) -> String {
    tr_args!(
        "integrations.summary.ai",
        state = on_off_label(enabled),
        running = running,
        waiting = waiting
    )
}

/// One-line clipboard state for the master list.
pub(crate) fn clipboard_summary(enabled: bool, resize: &ResizeSettings) -> String {
    tr_args!(
        "integrations.summary.clipboard",
        state = on_off_label(enabled),
        resize = resize_summary(resize)
    )
}

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
            IntegrationsSection::Discord => detail_body
                .child(self.discord_card(window, cx))
                .child(self.send_history_card(window, cx)),
            IntegrationsSection::AutoCompression => {
                detail_body.child(self.auto_compression_card(window, cx))
            }
            IntegrationsSection::AiTagging => detail_body.child(self.ai_tagging_card(window, cx)),
            IntegrationsSection::Clipboard => detail_body.child(self.clipboard_card(window, cx)),
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
    /// same title as its detail card, a status dot, and a one-line summary.
    fn integrations_list(
        &self,
        selected: IntegrationsSection,
        stacked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        let running = self.ai.job_counts.pending + self.ai.job_counts.processing;
        let configured = self.discord_url.is_some();

        let items = [
            (
                IntegrationsSection::Discord,
                "integrations-list-discord",
                tr!("integrations.discord.title"),
                discord_summary(configured, &self.settings.discord_destination),
                configured,
            ),
            (
                IntegrationsSection::AutoCompression,
                "integrations-list-auto",
                tr!("integrations.auto.title"),
                auto_compression_summary(self.settings.auto_compress, &self.settings.auto_compression),
                self.settings.auto_compress,
            ),
            (
                IntegrationsSection::AiTagging,
                "integrations-list-ai",
                tr!("integrations.ai.title"),
                ai_tagging_summary(
                    self.settings.ai_tagging_enabled,
                    running,
                    self.ai.job_counts.waiting,
                ),
                self.settings.ai_tagging_enabled,
            ),
            (
                IntegrationsSection::Clipboard,
                "integrations-list-clipboard",
                tr!("integrations.clipboard.title"),
                clipboard_summary(
                    self.settings.clipboard_enabled,
                    &self.settings.clipboard_compression,
                ),
                self.settings.clipboard_enabled,
            ),
        ];

        let rows = v_flex().gap(px(4.)).children(items.into_iter().map(
            |(section, id, name, summary, on)| {
                let active = section == selected;
                v_flex()
                    .id(SharedString::from(id))
                    .gap(px(3.))
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
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(p.muted_2))
                            .child(summary),
                    )
                    .into_any_element()
            },
        ));

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

    fn auto_compression_card(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let enabled = self.settings.auto_compress;
        let motion = self.integrations_auto_motion;
        let options = self.settings.auto_compression.clone();

        let body = v_flex()
            .pt(px(12.))
            .gap(px(12.))
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.startup.immediate"),
                "",
                self.settings.auto_compress_immediate,
                false,
                cx,
                |settings, value| settings.auto_compress_immediate = value,
            ))
            .child(card_divider(&p))
            .child(self.setting_switch(
                tr!("settings.startup.keep_original"),
                tr!("settings.startup.keep_original_hint"),
                self.settings.auto_compress_keep_original,
                false,
                cx,
                |settings, value| settings.auto_compress_keep_original = value,
            ))
            .when_some(
                compression_policy_warning(
                    self.settings.auto_compress_keep_original,
                    self.settings.auto_compress,
                ),
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
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted_2))
                    .child(tr!("integrations.compression.output_note.auto")),
            );

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
            .child(self.collapsible_body(
                "integrations-auto-body",
                motion,
                AUTO_BODY_MAX,
                window,
                cx,
                body,
            ))
            .into_any_element()
    }

    fn ai_tagging_card(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let ready = self.ai.ready();
        let enabled = self.settings.ai_tagging_enabled;
        let motion = self.integrations_ai_motion;

        let card = glass_panel(&self.palette)
            .flex()
            .flex_col()
            .p(px(14.))
            .rounded(px(17.))
            .child(
                h_flex().items_center().gap(px(12.)).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_semibold()
                                .text_color(rgb(p.text))
                                .child(tr!("integrations.ai.title")),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.muted_2))
                                .child(tr!("integrations.ai.hint")),
                        ),
                ),
            );

        if !ready {
            return card
                .child(
                    v_flex()
                        .pt(px(12.))
                        .gap(px(10.))
                        .child(card_divider(&p))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.muted))
                                .child(tr!("integrations.ai.not_ready")),
                        )
                        .child(card_divider(&p))
                        .child(
                            Button::new("integrations-ai-open-settings-not-ready")
                                .label(tr!("integrations.ai.open_settings"))
                                .small()
                                .primary()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.switch_screen(Screen::Settings, cx);
                                })),
                        ),
                )
                .into_any_element();
        }

        let auto = self.settings.ai_tagging_auto;
        let backlog = self.settings.ai_tagging_backlog;
        let counts = self.ai.job_counts;
        let language = vrc_photon_i18n::current_locale();
        let job_names = self.ai.active_job_names.clone();
        let job_error = self.ai.error_of(AiErrorKind::Job).cloned();
        let active_jobs: Vec<_> = self
            .ai
            .active_jobs
            .iter()
            .filter(|job| {
                matches!(
                    job.status,
                    TaggingJobStatus::Pending | TaggingJobStatus::Processing
                )
            })
            .take(3)
            .cloned()
            .collect();
        let backend_selection_disabled =
            backend_selection_is_disabled_when_not_ready(ready, enabled);
        let backends = self
            .ai
            .probe
            .as_ref()
            .map(|probe| backend_options(probe, &self.settings.ai_tagging_backend))
            .unwrap_or_default();

        let details =
            v_flex()
                .gap(px(10.))
                .child(
                    h_flex()
                        .gap(px(14.))
                        .child(ai_job_stat(
                            &p,
                            tr!("integrations.ai.jobs.waiting"),
                            counts.waiting,
                        ))
                        .child(ai_job_stat(
                            &p,
                            tr!("integrations.ai.jobs.processing"),
                            counts.pending + counts.processing,
                        ))
                        .child(ai_job_stat(
                            &p,
                            tr!("integrations.ai.jobs.failed"),
                            counts.failed,
                        )),
                )
                .when_some(job_error, |this, error| {
                    this.child(
                        div()
                            .px(px(10.))
                            .text_size(px(10.))
                            .text_color(rgb(p.red))
                            .child(tr_args!(
                                "integrations.ai.jobs.latest_failure",
                                error = ai_error_summary_in(&language, &error)
                            )),
                    )
                })
                .child(if active_jobs.is_empty() {
                    v_flex()
                        .gap(px(2.))
                        .p(px(10.))
                        .rounded(px(10.))
                        .bg(rgb(p.panel_2))
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_medium()
                                .text_color(rgb(p.muted))
                                .child(tr!("integrations.ai.jobs.empty")),
                        )
                        .into_any_element()
                } else {
                    v_flex()
                        .id("integrations-ai-jobs")
                        .gap(px(6.))
                        .children(active_jobs.into_iter().map(|job| {
                            let job_id = job.id.clone();
                            let photo_label = active_job_photo_label(
                                job_names.get(&job.photo_id).map(String::as_str),
                                job.photo_id,
                                &language,
                            );
                            let ratio = download_progress_bar_ratio(job.progress);
                            let percent = (ratio * 100.0).round() as u32;
                            v_flex()
                                .gap(px(5.))
                                .p(px(10.))
                                .rounded(px(10.))
                                .bg(rgb(p.panel_2))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_size(px(11.))
                                                .font_medium()
                                                .text_color(rgb(p.text))
                                                .truncate()
                                                .child(photo_label),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(rgb(p.accent))
                                                .child(format!("{percent}%")),
                                        ),
                                )
                                .child(status_progress_bar(&p, ratio))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_size(px(10.))
                                                .text_color(rgb(p.muted_2))
                                                .truncate()
                                                .child(ai_stage_label(job.stage)),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "integrations-ai-cancel-{job_id}"
                                            )))
                                            .label(tr!("integrations.ai.cancel_job"))
                                            .small()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.cancel_ai_job(&job_id, cx);
                                            })),
                                        ),
                                )
                                .into_any_element()
                        }))
                        .into_any_element()
                })
                .when(counts.waiting > 0, |this| {
                    this.child(
                        v_flex()
                            .gap(px(6.))
                            .p(px(10.))
                            .rounded(px(10.))
                            .bg(rgb(p.panel_2))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_medium()
                                    .text_color(rgb(p.text))
                                    .child(tr!("integrations.ai.start_backlog")),
                            )
                            .child(
                                Button::new("integrations-ai-start-backlog")
                                    .label(tr!("integrations.ai.start_backlog"))
                                    .small()
                                    .primary()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_ai_backlog(cx);
                                    })),
                            ),
                    )
                });

        let settings_row = h_flex().items_center().gap(px(8.)).child(
            Button::new("integrations-ai-open-settings-ready")
                .label(tr!("integrations.ai.open_settings"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.switch_screen(Screen::Settings, cx);
                })),
        );

        let body =
            v_flex()
                .pt(px(12.))
                .gap(px(12.))
                .child(card_divider(&p))
                .child(self.ai_switch_row(
                    tr!("integrations.ai.enabled"),
                    enabled,
                    true,
                    AiToggle::Enabled,
                    cx,
                ))
                .child(card_divider(&p))
                .child(self.ai_switch_row(
                    tr!("integrations.ai.auto"),
                    auto,
                    enabled,
                    AiToggle::Auto,
                    cx,
                ))
                .child(card_divider(&p))
                .child(self.ai_switch_row(
                    tr!("integrations.ai.backlog"),
                    backlog,
                    enabled,
                    AiToggle::Backlog,
                    cx,
                ))
                .child(card_divider(&p))
                .child(
                    v_flex()
                        .gap(px(6.))
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(p.text))
                                        .child(tr!("integrations.ai.backend")),
                                )
                                .child(div().flex_1()),
                        )
                        .child(h_flex().flex_wrap().gap(px(4.)).children(
                            backends.into_iter().map(|(id, selected)| {
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
                                                this.save_ai_runtime_settings(
                                                    cx,
                                                    move |settings| {
                                                        settings.ai_tagging_backend = backend;
                                                    },
                                                );
                                            },
                                        ))
                                    })
                                    .into_any_element()
                            }),
                        )),
                )
                .child(card_divider(&p))
                .child(self.collapsible_body(
                    "integrations-ai-body",
                    motion,
                    AI_BODY_MAX,
                    window,
                    cx,
                    details,
                ))
                .child(card_divider(&p))
                .child(settings_row);

        card.child(body).into_any_element()
    }

    fn ai_switch_row(
        &self,
        label: String,
        checked: bool,
        enabled: bool,
        toggle: AiToggle,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette;
        let id = match toggle {
            AiToggle::Enabled => "integrations-ai-enabled",
            AiToggle::Auto => "integrations-ai-auto",
            AiToggle::Backlog => "integrations-ai-backlog",
        };
        h_flex()
            .items_center()
            .gap(px(12.))
            .py(px(2.))
            .child(
                v_flex().flex_1().min_w_0().gap(px(2.)).child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(if enabled { p.text } else { p.muted_2 }))
                        .child(label),
                ),
            )
            .child(
                Switch::new(id)
                    .checked(checked)
                    .disabled(!enabled)
                    .on_click(cx.listener(move |this, value, _, cx| {
                        if !enabled {
                            return;
                        }
                        match toggle {
                            AiToggle::Enabled => this.set_ai_tagging_enabled(*value, cx),
                            AiToggle::Auto => this.save_ai_runtime_settings(cx, |settings| {
                                settings.ai_tagging_auto = *value;
                            }),
                            AiToggle::Backlog => this.save_ai_runtime_settings(cx, |settings| {
                                settings.ai_tagging_backlog = *value;
                            }),
                        }
                    })),
            )
    }

    fn set_ai_tagging_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.settings.ai_tagging_enabled == enabled {
            return;
        }
        self.save_ai_runtime_settings(cx, move |settings| {
            settings.ai_tagging_enabled = enabled;
        });
        // A failed save rolls the setting back; animate to the persisted
        // value so the runtime section cannot claim a state it did not save.
        let target = if self.settings.ai_tagging_enabled {
            0.0
        } else {
            1.0
        };
        if cx.reduce_motion() {
            self.integrations_ai_motion = ToggleMotion::settled(target);
        } else {
            self.integrations_ai_motion.animate_to(target, MOTION_FAST);
        }
        cx.notify();
    }

    fn clipboard_card(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let enabled = self.settings.clipboard_enabled;
        let motion = self.integrations_clipboard_motion;
        let profiles = self.profiles.clone();

        let body = v_flex()
            .pt(px(12.))
            .gap(px(8.))
            .child(card_divider(&p))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted_2))
                    .child(tr!("integrations.clipboard.profiles_hint")),
            )
            .child(card_divider(&p))
            .child(if profiles.is_empty() {
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted_2))
                    .child(tr!("integrations.clipboard.profiles_empty"))
                    .into_any_element()
            } else {
                v_flex()
                    .id("integrations-clipboard-profiles")
                    .max_h(px(180.))
                    .overflow_y_scrollbar()
                    .gap(px(4.))
                    .children(profiles.into_iter().map(|profile| {
                        let id = profile.id.clone();
                        let selected = profile.clipboard;
                        manual_option_row(
                            &p,
                            SharedString::from(format!("clipboard-profile-{}", profile.id)),
                            profile.name.clone(),
                            profile_summary(&profile),
                            selected,
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_clipboard_profile(&id, !selected, cx);
                        }))
                        .into_any_element()
                    }))
                    .into_any_element()
            });

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
                                .child(tr!("integrations.clipboard.title")),
                        ),
                    )
                    .child(
                        Switch::new("integrations-clipboard-enabled")
                            .checked(enabled)
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.set_clipboard_enabled(*checked, cx);
                            })),
                    ),
            )
            .child(self.collapsible_body(
                "integrations-clipboard-body",
                motion,
                CLIPBOARD_BODY_MAX,
                window,
                cx,
                body,
            ))
            .into_any_element()
    }

    pub(crate) fn collapsible_body(
        &self,
        id: &'static str,
        motion: ToggleMotion,
        max_height: f32,
        window: &Window,
        cx: &Context<Self>,
        body: impl IntoElement + 'static,
    ) -> AnyElement {
        div()
            .overflow_hidden()
            .with_animation(
                (id, motion.epoch()),
                Animation::new(motion.driver_duration())
                    .with_easing(ease_motion)
                    .with_max_fps(self.motion_fps(window, cx)),
                move |el, _| {
                    let collapsed = motion.value();
                    el.max_h(px((1.0 - collapsed) * max_height))
                        .opacity(1.0 - collapsed)
                },
            )
            .child(body)
            .into_any_element()
    }

    fn set_auto_compress_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.settings.auto_compress == enabled {
            return;
        }
        self.settings.auto_compress = enabled;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save automatic compression setting");
        }
        let target = if enabled { 0.0 } else { 1.0 };
        if cx.reduce_motion() {
            self.integrations_auto_motion = ToggleMotion::settled(target);
        } else {
            self.integrations_auto_motion
                .animate_to(target, MOTION_FAST);
        }
        cx.notify();
    }

    fn set_clipboard_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.settings.clipboard_enabled == enabled {
            return;
        }
        self.settings.clipboard_enabled = enabled;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save clipboard setting");
        }
        let target = if enabled { 0.0 } else { 1.0 };
        if cx.reduce_motion() {
            self.integrations_clipboard_motion = ToggleMotion::settled(target);
        } else {
            self.integrations_clipboard_motion
                .animate_to(target, MOTION_FAST);
        }
        cx.notify();
    }
}

/// One queue counter in the AI runtime card.
fn ai_job_stat(p: &Palette, label: String, value: i64) -> Div {
    h_flex()
        .items_baseline()
        .gap(px(5.))
        .child(
            div()
                .text_size(px(9.5))
                .text_color(rgb(p.muted_2))
                .child(label),
        )
        .child(
            div()
                .text_size(px(12.))
                .font_semibold()
                .text_color(rgb(p.text))
                .child(value.to_string()),
        )
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
    use super::{
        ai_stage_label, ai_tagging_summary, auto_compression_summary, clipboard_summary,
        discord_summary,
    };
    use vrc_photon_ai_protocol::TaggingStage;
    use vrc_photon_core::model::{CompressionOptions, ResizeSettings};

    #[test]
    fn section_summaries_follow_the_existing_card_values() {
        assert_eq!(
            discord_summary(true, "VRChat写真部"),
            "設定済み ・ 送信先：VRChat写真部"
        );
        assert_eq!(
            discord_summary(false, "Discord"),
            "未設定 ・ 送信先：Discord"
        );
        assert_eq!(
            auto_compression_summary(true, &CompressionOptions::auto_default()),
            "有効 ・ WebP · Lossless"
        );
        assert_eq!(
            auto_compression_summary(false, &CompressionOptions::discord_default()),
            "無効 ・ WebP · Lossless"
        );
        assert_eq!(
            ai_tagging_summary(true, 1, 12),
            "有効 ・ 実行中 1 件 / 待機 12 件"
        );
        assert_eq!(
            clipboard_summary(true, &ResizeSettings::clipboard_default()),
            "有効 ・ 2560×1440（大きい場合のみ）"
        );
        assert_eq!(
            clipboard_summary(false, &ResizeSettings::default()),
            "無効 ・ リサイズなし"
        );
    }

    #[test]
    fn auto_compression_summary_uses_the_auto_options() {
        let options = CompressionOptions::discord_default();
        assert!(auto_compression_summary(true, &options).contains("WebP"));
    }

    #[test]
    fn atmosphere_stage_has_a_label() {
        assert_eq!(
            ai_stage_label(Some(TaggingStage::AtmosphereTags)),
            "雰囲気タグを生成中"
        );
    }
}
