//! Shared queue rows for the three integrations and the dashboard.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vrc_photon_core::queue::{QueueCounts, QueueEntry, QueueFeature, QueueState};
use vrc_photon_i18n::{tr, tr_args};

use crate::app::App;
use crate::ui::components::{card_divider, glass_panel, status_progress_bar};
use crate::ui::theme::*;

/// Sorts the mixed queue oldest first. The DB already does this; the helper
/// keeps the UI's local ordering testable.
pub(crate) fn sort_mixed_queue(mut entries: Vec<QueueEntry>) -> Vec<QueueEntry> {
    entries.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.job_id.cmp(&b.job_id))
    });
    entries
}

pub(crate) fn waiting_delete_available(state: QueueState) -> bool {
    state == QueueState::Waiting
}

/// Which queue actions a card exposes while it is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QueueActionAvailability {
    pub(crate) pause: bool,
    pub(crate) start_waiting: bool,
    pub(crate) delete_waiting: bool,
}

pub(crate) fn queue_action_availability(
    card_enabled: bool,
    waiting: i64,
) -> QueueActionAvailability {
    QueueActionAvailability {
        pause: card_enabled,
        start_waiting: card_enabled && waiting > 0,
        delete_waiting: card_enabled && waiting > 0,
    }
}

pub(crate) fn queue_row_delete_available(card_enabled: bool, state: QueueState) -> bool {
    card_enabled && waiting_delete_available(state)
}

fn feature_label(feature: QueueFeature) -> String {
    match feature {
        QueueFeature::Compression => tr!("queue.feature.compression"),
        QueueFeature::Discord => tr!("queue.feature.discord"),
        QueueFeature::Tagging => tr!("queue.feature.tagging"),
    }
}

fn state_label(state: QueueState) -> String {
    match state {
        QueueState::Waiting => tr!("queue.state.waiting"),
        QueueState::Pending => tr!("queue.state.pending"),
        QueueState::Processing => tr!("queue.state.processing"),
    }
}

fn state_colors(state: QueueState, p: &Palette) -> (u32, u32) {
    match state {
        QueueState::Waiting => (p.amber, p.amber_soft),
        QueueState::Pending => (p.muted, p.panel_2),
        QueueState::Processing => (p.accent, p.accent_soft),
    }
}

fn feature_colors(feature: QueueFeature, p: &Palette) -> (u32, u32) {
    match feature {
        QueueFeature::Compression => (p.accent, p.accent_soft),
        QueueFeature::Discord => (p.green, p.green_soft),
        QueueFeature::Tagging => (p.amber, p.amber_soft),
    }
}

fn detail_label(entry: &QueueEntry) -> String {
    match entry.feature {
        QueueFeature::Compression if entry.detail == "auto" => {
            tr!("queue.feature.compression")
        }
        QueueFeature::Discord if entry.detail == "test" => {
            tr!("integrations.discord.test.button")
        }
        QueueFeature::Tagging => match entry.detail.as_str() {
            "presence" => tr!("integrations.ai.jobs.stage.presence"),
            "people_count" => tr!("integrations.ai.jobs.stage.people_count"),
            "place_class" => tr!("integrations.ai.jobs.stage.place_class"),
            "place_tags" => tr!("integrations.ai.jobs.stage.place_tags"),
            "atmosphere_tags" => tr!("integrations.ai.jobs.stage.atmosphere_tags"),
            "write" => tr!("integrations.ai.jobs.stage.write"),
            "" => tr!("integrations.ai.jobs.stage.queued"),
            detail => detail.to_string(),
        },
        _ => entry.detail.clone(),
    }
}

fn entry_name(entry: &QueueEntry) -> String {
    entry
        .photo_name
        .clone()
        .or_else(|| entry.photo_id.map(|id| format!("photo #{id}")))
        .unwrap_or_else(|| "—".to_string())
}

/// One queue row. `show_feature` adds the feature badge used by the dashboard.
/// `card_enabled` keeps row actions inert while the owning card is disabled.
pub(crate) fn queue_row(
    app: &App,
    entry: &QueueEntry,
    show_feature: bool,
    card_enabled: bool,
    cx: &mut Context<App>,
) -> AnyElement {
    let p = app.palette;
    let (state_fg, state_bg) = state_colors(entry.state, &p);
    let (feature_fg, feature_bg) = feature_colors(entry.feature, &p);
    let job_id = entry.job_id.clone();
    let feature = entry.feature;
    let name = entry_name(entry);
    let detail = detail_label(entry);
    let state = entry.state;
    let progress = entry.progress.clamp(0.0, 1.0);

    h_flex()
        .w_full()
        .min_h(px(30.))
        .gap(px(8.))
        .px(px(4.))
        .py(px(4.))
        .rounded(px(7.))
        .when(show_feature, |this| {
            this.child(app.badge(feature_label(feature), feature_fg, feature_bg))
        })
        .child(app.badge(state_label(state), state_fg, state_bg))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.))
                .text_color(rgb(p.text))
                .child(name),
        )
        .when(state == QueueState::Processing, |this| {
            this.child(div().w(px(80.)).child(status_progress_bar(&p, progress)))
                .child(
                    div()
                        .w(px(34.))
                        .text_size(px(9.))
                        .text_color(rgb(p.accent))
                        .child(format!("{}%", (progress * 100.0).round() as u32)),
                )
        })
        .child(
            div()
                .max_w(px(180.))
                .truncate()
                .text_size(px(9.))
                .text_color(rgb(p.muted_2))
                .child(detail),
        )
        .when(waiting_delete_available(state), |this| {
            let delete_enabled = queue_row_delete_available(card_enabled, state);
            this.child(
                Button::new(SharedString::from(format!(
                    "queue-delete-{}-{job_id}",
                    feature.as_str()
                )))
                .label(tr!("queue.delete_waiting_row"))
                .small()
                .ghost()
                .disabled(!delete_enabled)
                .tab_stop(delete_enabled)
                .when(delete_enabled, |button| {
                    button.on_click(cx.listener(move |this, _, _, cx| {
                        this.delete_waiting_entry(feature, &job_id, cx);
                    }))
                }),
            )
        })
        .into_any_element()
}

/// Queue section for one integration card. It is deliberately a plain
/// section, not a nested card.
pub(crate) fn queue_section(
    app: &App,
    feature: QueueFeature,
    entries: &[QueueEntry],
    counts: QueueCounts,
    card_enabled: bool,
    paused: bool,
    cx: &mut Context<App>,
) -> AnyElement {
    let p = app.palette;
    let prefix = match feature {
        QueueFeature::Compression => "integrations-auto-queue",
        QueueFeature::Discord => "integrations-discord-queue",
        QueueFeature::Tagging => "integrations-ai-queue",
    };
    let actions = queue_action_availability(card_enabled, counts.waiting);

    v_flex()
        .gap(px(8.))
        .child(card_divider(&p))
        .child(
            h_flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_semibold()
                        .text_color(rgb(p.text))
                        .child(tr!("queue.title")),
                )
                .child(div().flex_1())
                .when(paused, |this| {
                    this.child(app.badge(tr!("queue.paused"), p.amber, p.amber_soft))
                })
                .child(
                    Button::new(SharedString::from(format!("{prefix}-pause")))
                        .label(if paused {
                            tr!("queue.resume")
                        } else {
                            tr!("queue.pause")
                        })
                        .small()
                        .disabled(!actions.pause)
                        .tab_stop(actions.pause)
                        .when(actions.pause, |button| {
                            button.on_click(cx.listener(move |this, _, _, cx| {
                                this.set_queue_paused(feature, !paused, cx);
                            }))
                        }),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap(px(5.))
                .flex_wrap()
                .child(app.badge(
                    tr_args!("queue.count.waiting", count = counts.waiting),
                    p.amber,
                    p.amber_soft,
                ))
                .child(app.badge(
                    tr_args!("queue.count.pending", count = counts.pending),
                    p.muted,
                    p.panel_2,
                ))
                .child(app.badge(
                    tr_args!("queue.count.processing", count = counts.processing),
                    p.accent,
                    p.accent_soft,
                )),
        )
        .child(
            h_flex()
                .items_center()
                .gap(px(5.))
                .flex_wrap()
                .child(
                    Button::new(SharedString::from(format!("{prefix}-start-waiting")))
                        .label(tr!("queue.start_waiting"))
                        .small()
                        .primary()
                        .disabled(!actions.start_waiting)
                        .tab_stop(actions.start_waiting)
                        .when(actions.start_waiting, |button| {
                            button.on_click(cx.listener(move |this, _, window, cx| {
                                this.release_waiting_queue(feature, window, cx);
                            }))
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("{prefix}-delete-waiting")))
                        .label(tr!("queue.delete_waiting_all"))
                        .small()
                        .ghost()
                        .disabled(!actions.delete_waiting)
                        .tab_stop(actions.delete_waiting)
                        .when(actions.delete_waiting, |button| {
                            button.on_click(cx.listener(move |this, _, window, cx| {
                                this.delete_all_waiting(feature, window, cx);
                            }))
                        }),
                ),
        )
        .child(if entries.is_empty() {
            div()
                .px(px(4.))
                .py(px(8.))
                .text_size(px(10.))
                .text_color(rgb(p.muted_2))
                .child(tr!("queue.empty"))
                .into_any_element()
        } else {
            v_flex()
                .gap(px(2.))
                .children(
                    entries
                        .iter()
                        .map(|entry| queue_row(app, entry, false, card_enabled, cx)),
                )
                .when(counts.total() > entries.len() as i64, |this| {
                    this.child(
                        div()
                            .pt(px(4.))
                            .text_size(px(9.))
                            .text_color(rgb(p.muted_2))
                            .child(tr_args!("queue.limit", count = entries.len())),
                    )
                })
                .into_any_element()
        })
        .into_any_element()
}

/// The dashboard's mixed queue. It fills the left column and has no pause
/// controls; those stay in the feature details.
pub(crate) fn mixed_queue_panel(app: &App, cx: &mut Context<App>) -> AnyElement {
    let p = app.palette;
    let counts = app.mixed_queue_counts;
    let entries = sort_mixed_queue(app.mixed_queue_entries.clone());
    glass_panel(&app.palette)
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .rounded(px(17.))
        .child(
            h_flex()
                .flex_shrink_0()
                .min_h(px(GALLERY_HEADER_HEIGHT))
                .px(px(GALLERY_PAD_X))
                .py(px(8.))
                .gap(px(8.))
                .flex_wrap()
                .items_center()
                .border_b_1()
                .border_color(rgba(p.highlight))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_semibold()
                        .text_color(rgb(p.text))
                        .child(tr!("queue.title")),
                )
                .child(div().flex_1())
                .child(app.badge(
                    tr_args!("queue.count.waiting", count = counts.waiting),
                    p.amber,
                    p.amber_soft,
                ))
                .child(app.badge(
                    tr_args!("queue.count.pending", count = counts.pending),
                    p.muted,
                    p.panel_2,
                ))
                .child(app.badge(
                    tr_args!("queue.count.processing", count = counts.processing),
                    p.accent,
                    p.accent_soft,
                )),
        )
        .child(
            v_flex()
                .id("dashboard-mixed-queue")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p(px(8.))
                .gap(px(2.))
                .when(entries.is_empty(), |this| {
                    this.child(
                        div()
                            .p(px(12.))
                            .text_size(px(11.))
                            .text_color(rgb(p.muted_2))
                            .child(tr!("queue.empty")),
                    )
                })
                .children(
                    entries
                        .iter()
                        .map(|entry| queue_row(app, entry, true, true, cx)),
                )
                .when(counts.total() > 100, |this| {
                    this.child(
                        div()
                            .pt(px(6.))
                            .text_size(px(9.))
                            .text_color(rgb(p.muted_2))
                            .child(tr_args!("queue.limit", count = 100)),
                    )
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        QueueEntry, QueueFeature, QueueState, queue_action_availability,
        queue_row_delete_available, sort_mixed_queue, waiting_delete_available,
    };

    fn entry(feature: QueueFeature, created_at: &str) -> QueueEntry {
        QueueEntry {
            feature,
            job_id: created_at.to_string(),
            photo_id: Some(1),
            photo_name: Some("photo.png".to_string()),
            state: QueueState::Waiting,
            progress: 0.0,
            detail: String::new(),
            created_at: created_at.to_string(),
        }
    }

    #[test]
    fn mixed_queue_orders_features_by_created_at() {
        let entries = vec![
            entry(QueueFeature::Tagging, "3"),
            entry(QueueFeature::Compression, "1"),
            entry(QueueFeature::Discord, "2"),
        ];
        let sorted = sort_mixed_queue(entries);
        assert_eq!(sorted[0].created_at, "1");
        assert_eq!(sorted[2].created_at, "3");
    }

    #[test]
    fn waiting_rows_can_be_deleted_but_pending_and_processing_cannot() {
        assert!(waiting_delete_available(QueueState::Waiting));
        assert!(!waiting_delete_available(QueueState::Pending));
        assert!(!waiting_delete_available(QueueState::Processing));
    }

    #[test]
    fn queue_actions_are_disabled_while_the_card_is_disabled() {
        let actions = queue_action_availability(false, 2);
        assert!(!actions.pause);
        assert!(!actions.start_waiting);
        assert!(!actions.delete_waiting);

        let actions = queue_action_availability(true, 0);
        assert!(actions.pause);
        assert!(!actions.start_waiting);
        assert!(!actions.delete_waiting);

        let actions = queue_action_availability(true, 2);
        assert!(actions.pause);
        assert!(actions.start_waiting);
        assert!(actions.delete_waiting);
    }

    #[test]
    fn queue_row_delete_is_disabled_while_the_card_is_disabled() {
        assert!(queue_row_delete_available(true, QueueState::Waiting));
        assert!(!queue_row_delete_available(false, QueueState::Waiting));
        assert!(!queue_row_delete_available(true, QueueState::Pending));
        assert!(!queue_row_delete_available(true, QueueState::Processing));
    }
}
