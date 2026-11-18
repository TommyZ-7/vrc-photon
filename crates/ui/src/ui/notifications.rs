//! Transient toasts and the action-required notification modal.
//!
//! Toasts are delivered through gpui-component's `NotificationList` so
//! stacking, auto-hide, and enter/exit animations stay consistent across the
//! app. The modal is app-owned because it hosts the import report summary and
//! the failure actions that are specific to this application.

use gpui_kit::assets::IconName;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use semver::Version;
use vrc_photon_core::queue::{FailedJobEntry, QueueFeature};
use vrc_photon_i18n::{tr, tr_args};

use crate::app::App;
use crate::app::Screen;
use crate::app::import::ImportReportView;
use crate::ui::components::{ButtonState, ButtonTone};
use crate::ui::import::{has_retryable_sources, import_mode_label};
use crate::ui::theme::*;

#[derive(Clone, Debug)]
pub(crate) struct ToastSpec {
    pub(crate) tone: NotificationType,
    pub(crate) message: SharedString,
    pub(crate) opens_notifications: bool,
}

/// Counts used to decide which failures crossed their last-seen values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FailureCounts {
    pub(crate) compression: i64,
    pub(crate) discord: i64,
    pub(crate) tagging: i64,
    pub(crate) daemon: usize,
}

/// The failed rows and counts shown by the notification modal.
#[derive(Clone, Debug, Default)]
pub(crate) struct NotificationSnapshot {
    pub(crate) compression_failed: i64,
    pub(crate) discord_failed: i64,
    pub(crate) tagging_failed: i64,
    pub(crate) entries: Vec<FailedJobEntry>,
}

impl NotificationSnapshot {
    pub(crate) fn total_jobs(&self) -> i64 {
        self.compression_failed + self.discord_failed + self.tagging_failed
    }

    pub(crate) fn failure_counts(&self, daemon: usize) -> FailureCounts {
        FailureCounts {
            compression: self.compression_failed,
            discord: self.discord_failed,
            tagging: self.tagging_failed,
            daemon,
        }
    }
}

/// One failure domain that crossed its last-seen count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureNotice {
    Compression { count: i64 },
    Discord { count: i64 },
    Tagging { count: i64 },
    Daemon { count: usize },
}

/// Returns the failures that appeared since `previous`. `None` means the
/// baseline has not been seeded yet, so existing failures stay silent.
pub(crate) fn failure_transition(
    previous: Option<FailureCounts>,
    next: FailureCounts,
) -> Vec<FailureNotice> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let mut notices = Vec::new();
    if next.compression > previous.compression {
        notices.push(FailureNotice::Compression {
            count: next.compression,
        });
    }
    if next.discord > previous.discord {
        notices.push(FailureNotice::Discord {
            count: next.discord,
        });
    }
    if next.tagging > previous.tagging {
        notices.push(FailureNotice::Tagging {
            count: next.tagging,
        });
    }
    if next.daemon > previous.daemon {
        notices.push(FailureNotice::Daemon { count: next.daemon });
    }
    notices
}

fn job_feature_label(feature: QueueFeature) -> String {
    match feature {
        QueueFeature::Compression => tr!("queue.feature.compression"),
        QueueFeature::Discord => tr!("queue.feature.discord"),
        QueueFeature::Tagging => tr!("queue.feature.tagging"),
    }
}

/// Newest failed rows, capped at `limit`.
pub(crate) fn latest_failed_entries(
    mut entries: Vec<FailedJobEntry>,
    limit: usize,
) -> Vec<FailedJobEntry> {
    entries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.job_id.cmp(&a.job_id))
    });
    entries.truncate(limit);
    entries
}

/// Returns the blocked-deletion count when it grew since `previous`. The
/// first snapshot seeds the baseline silently, like `failure_transition`.
pub(crate) fn deletion_blocked_transition(
    previous: Option<(i64, i64)>,
    next: (i64, i64),
) -> Option<i64> {
    let (_, previous_blocked) = previous?;
    (next.1 > previous_blocked).then_some(next.1)
}

/// The action-required blocks shown by the notification modal, in display
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotificationSection {
    AppUpdate,
    Import,
    Library,
    JobFailures,
    Daemon,
}

pub(crate) fn notification_sections(
    update: bool,
    import: bool,
    library_error: bool,
    compression_failed: i64,
    discord_failed: i64,
    tagging_failed: i64,
    daemon_errors: usize,
) -> Vec<NotificationSection> {
    let mut sections = Vec::new();
    if update {
        sections.push(NotificationSection::AppUpdate);
    }
    if import {
        sections.push(NotificationSection::Import);
    }
    if library_error {
        sections.push(NotificationSection::Library);
    }
    if compression_failed > 0 || discord_failed > 0 || tagging_failed > 0 {
        sections.push(NotificationSection::JobFailures);
    }
    if daemon_errors > 0 {
        sections.push(NotificationSection::Daemon);
    }
    sections
}

/// Whether the open notification modal has lost its last visible section.
#[allow(clippy::too_many_arguments)]
pub(crate) fn notification_modal_should_close(
    open: bool,
    update: bool,
    import: bool,
    library_error: bool,
    compression_failed: i64,
    discord_failed: i64,
    tagging_failed: i64,
    daemon_errors: usize,
) -> bool {
    open && notification_sections(
        update,
        import,
        library_error,
        compression_failed,
        discord_failed,
        tagging_failed,
        daemon_errors,
    )
    .is_empty()
}

/// Whether a render should schedule the deferred toast flush.
pub(crate) fn toast_flush_needed(pending: usize, scheduled: bool) -> bool {
    pending > 0 && !scheduled
}

/// Label for the import result badge; zero jobs stay hidden.
fn auto_compress_badge_label(count: usize) -> Option<String> {
    (count > 0).then(|| tr_args!("notifications.auto_compress_badge", count = count))
}

/// The notification modal must not stack over another modal.
pub(crate) fn notification_modal_allowed(import_open: bool, manual_open: bool) -> bool {
    !import_open && !manual_open
}

/// Whether the bell should show its attention dot and open the modal.
pub(crate) fn notification_attention_for(
    update: bool,
    import: bool,
    library_error: bool,
    failed_jobs: i64,
    daemon_errors: usize,
) -> bool {
    update || import || library_error || failed_jobs > 0 || daemon_errors > 0
}

fn notification_for(spec: &ToastSpec, entity: WeakEntity<App>) -> Notification {
    let mut note = Notification::new()
        .message(spec.message.clone())
        .with_type(spec.tone);
    if spec.opens_notifications {
        note = note.on_click(move |_, window, cx| {
            entity
                .update(cx, |this, cx| this.open_notification_modal(window, cx))
                .ok();
        });
    }
    note
}

impl App {
    pub(crate) fn toast(
        &mut self,
        tone: NotificationType,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.enqueue_toast(
            ToastSpec {
                tone,
                message: message.into(),
                opens_notifications: false,
            },
            cx,
        );
    }

    pub(crate) fn toast_info(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast(NotificationType::Info, message, cx);
    }

    pub(crate) fn toast_success(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.toast(NotificationType::Success, message, cx);
    }

    pub(crate) fn toast_warning(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.toast(NotificationType::Warning, message, cx);
    }

    pub(crate) fn toast_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast(NotificationType::Error, message, cx);
    }

    /// A toast that opens the notification modal when clicked.
    pub(crate) fn toast_actionable(
        &mut self,
        tone: NotificationType,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.enqueue_toast(
            ToastSpec {
                tone,
                message: message.into(),
                opens_notifications: true,
            },
            cx,
        );
    }

    fn enqueue_toast(&mut self, spec: ToastSpec, cx: &mut Context<Self>) {
        // Always queue. Pushing directly from an event handler would re-enter
        // the window update that is currently dispatching the event.
        self.pending_toasts.push(spec);
        cx.notify();
    }

    /// Drains queued toasts after the current window update has finished.
    pub(crate) fn flush_pending_toasts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.pending_toasts);
        for spec in pending {
            let entity = cx.entity().downgrade();
            let note = notification_for(&spec, entity);
            window.push_notification(note, cx);
        }
    }

    pub(crate) fn notification_attention(&self) -> bool {
        notification_attention_for(
            self.update.pending_version().is_some(),
            self.import_report.is_some(),
            self.library_error.is_some(),
            self.job_failures.total_jobs(),
            self.daemon_errors.len(),
        )
    }

    pub(crate) fn open_notification_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notification_modal_open {
            return;
        }
        if !self.notification_attention() {
            return;
        }
        if !notification_modal_allowed(self.import_dialog.is_some(), self.manual_compress.is_some())
            || self.ai_tag_picker.open
            || self.ai_thresholds.open
        {
            // Actionable toasts are painted above the import dialog. Do not
            // stack a second modal over it; keep focus in the import trap.
            if self.import_dialog.is_some() {
                window.focus(&self.import_dialog_focus, cx);
            }
            return;
        }
        self.notification_return_focus = window.focused(cx);
        self.notification_modal_open = true;
        self.notification_modal_epoch = self.notification_modal_epoch.wrapping_add(1);
        self.import_report_expanded = false;
        let focus = self.notification_modal_focus.clone();
        cx.defer_in(window, move |this, window, cx| {
            if this.notification_modal_open {
                window.focus(&focus, cx);
            }
        });
        cx.notify();
    }

    pub(crate) fn close_notification_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.notification_modal_open {
            return;
        }
        self.notification_modal_open = false;
        self.import_report_expanded = false;
        let focus = self
            .notification_return_focus
            .take()
            .unwrap_or_else(|| self.photo_grid_focus.clone());
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(crate) fn notification_button(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let attention = self.notification_attention();
        div()
            .relative()
            .flex_shrink_0()
            .child(
                Button::new("titlebar-notifications")
                    .track_focus(&self.notification_bell_focus)
                    .ghost()
                    .icon(IconName::Bell)
                    .tooltip(tr!("notifications.title"))
                    .size(px(26.))
                    .rounded(px(6.))
                    .text_color(rgb(p.muted_2))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_notification_modal(window, cx)),
                    ),
            )
            .when(attention, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(5.))
                        .right(px(5.))
                        .size(px(4.))
                        .rounded_full()
                        .bg(rgb(p.amber))
                        .shadow(vec![
                            BoxShadow::new(px(0.), px(0.), rgba(with_alpha(p.amber, 0xcc)).into())
                                .blur_radius(px(6.)),
                        ]),
                )
            })
            .into_any_element()
    }

    pub(crate) fn notification_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        if !self.notification_modal_open {
            return div().into_any_element();
        }
        let update_version = self.update.pending_version().cloned();
        if notification_modal_should_close(
            self.notification_modal_open,
            update_version.is_some(),
            self.import_report.is_some(),
            self.library_error.is_some(),
            self.job_failures.compression_failed,
            self.job_failures.discord_failed,
            self.job_failures.tagging_failed,
            self.daemon_errors.len(),
        ) {
            cx.defer_in(window, |this, window, cx| {
                if this.notification_modal_open && !this.notification_attention() {
                    this.close_notification_modal(window, cx);
                }
            });
            return div().into_any_element();
        }
        let sections = notification_sections(
            update_version.is_some(),
            self.import_report.is_some(),
            self.library_error.is_some(),
            self.job_failures.compression_failed,
            self.job_failures.discord_failed,
            self.job_failures.tagging_failed,
            self.daemon_errors.len(),
        );
        // `App` stores the report behind an Rc so this is a pointer clone, not
        // a copy of every issue. The import card only needs a shared borrow
        // while it builds the sections.
        let import_report = self.import_report.clone();
        let library_error = self.library_error;
        let job_failures = self.job_failures.clone();
        let daemon_errors = self.daemon_errors.len();

        let mut items = Vec::new();
        for section in sections {
            let item = match section {
                NotificationSection::AppUpdate => update_version
                    .as_ref()
                    .map(|version| self.app_update_notification(version, cx)),
                NotificationSection::Import => import_report
                    .as_ref()
                    .map(|report| self.import_notification(report, cx)),
                NotificationSection::Library => library_error
                    .as_ref()
                    .map(|error| self.library_notification(error, cx)),
                NotificationSection::JobFailures => {
                    Some(self.job_failures_notification(&job_failures, cx))
                }
                NotificationSection::Daemon => Some(self.daemon_notification(daemon_errors, cx)),
            };
            if let Some(item) = item {
                items.push(item);
            }
        }

        let max_height = (f32::from(window.viewport_size().height) * 0.8).max(240.0);
        let close = self.focusable_text_button(
            "notification-close",
            tr!("viewer.close"),
            self.notification_close_focus.clone(),
            ButtonState {
                tone: ButtonTone::Neutral,
                enabled: true,
            },
            cx,
            |this, window, cx| this.close_notification_modal(window, cx),
        );

        div()
            .id("notification-modal")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(24.))
            .bg(rgba(p.scrim))
            .key_context("notification_modal")
            .occlude()
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_notification_modal(window, cx);
            }))
            .child(
                v_flex()
                    .id("notification-modal-card")
                    .track_focus(&self.notification_modal_focus)
                    .focus_trap("notification-modal", &self.notification_modal_focus)
                    .w(px(720.))
                    .max_w(relative(1.0))
                    .max_h(px(max_height))
                    .overflow_hidden()
                    .rounded(px(20.))
                    .border_1()
                    .border_color(rgba(p.line_soft))
                    .bg(rgb(p.panel_deep))
                    .shadow(vec![
                        BoxShadow::new(px(0.), px(24.), rgba(p.shadow).into()).blur_radius(px(56.)),
                        BoxShadow::new(px(0.), px(1.), rgba(with_alpha(p.text, 0x10)).into())
                            .inset()
                            .blur_radius(px(0.)),
                    ])
                    .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                    .child(
                        h_flex()
                            .h(px(52.))
                            .flex_shrink_0()
                            .px(px(18.))
                            .gap(px(9.))
                            .items_center()
                            .border_b_1()
                            .border_color(rgba(p.highlight))
                            .child(
                                Icon::new(IconName::Bell)
                                    .with_size(px(16.))
                                    .text_color(rgb(p.accent)),
                            )
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_semibold()
                                    .text_color(rgb(p.text))
                                    .child(tr!("notifications.title")),
                            )
                            .child(div().flex_1())
                            .child(close),
                    )
                    .child(
                        v_flex()
                            .id("notification-modal-body")
                            .flex_1()
                            .min_h_0()
                            .p(px(16.))
                            .gap(px(12.))
                            .overflow_y_scrollbar()
                            .children(items),
                    ),
            )
            .with_animation(
                ("notification-modal", self.notification_modal_epoch),
                Animation::new(MOTION_FAST)
                    .with_easing(ease_motion)
                    .with_max_fps(self.motion_fps(window, cx)),
                |el, t| el.opacity(t),
            )
            .into_any_element()
    }

    fn import_notification(
        &mut self,
        report: &ImportReportView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        let (failed, partial) = report.issue_counts();
        let retryable = has_retryable_sources(&report.issues);
        let scan_failed = report.scan_failures.len() + report.scan_errors.len();
        let mode = import_mode_label(report.mode);
        let dest = report.dest.to_string_lossy().to_string();
        let expanded = self.import_report_expanded;

        let mut badges = h_flex().flex_wrap().gap(px(6.));
        badges = badges
            .child(self.badge(
                tr_args!("notifications.import.imported", count = report.imported),
                p.green,
                p.green_soft,
            ))
            .child(self.badge(
                tr_args!("notifications.import.skipped", count = report.skipped),
                p.muted,
                p.panel_2,
            ));
        if failed > 0 {
            badges = badges.child(self.badge(
                tr_args!("notifications.import.failed", count = failed),
                p.red,
                p.red_soft,
            ));
        }
        if partial > 0 {
            badges = badges.child(self.badge(
                tr_args!("notifications.import.partial", count = partial),
                p.amber,
                p.amber_soft,
            ));
        }
        if !report.renamed.is_empty() {
            badges = badges.child(self.badge(
                tr_args!("notifications.import.renamed", count = report.renamed.len()),
                p.accent,
                p.accent_soft,
            ));
        }
        if !report.remaining.is_empty() {
            badges = badges.child(self.badge(
                tr_args!(
                    "notifications.import.pending",
                    count = report.remaining.len()
                ),
                p.amber,
                p.amber_soft,
            ));
        }
        if scan_failed > 0 {
            badges = badges.child(self.badge(
                tr_args!("notifications.import.scan_failed", count = scan_failed),
                p.red,
                p.red_soft,
            ));
        }
        if let Some(label) = auto_compress_badge_label(self.import_auto_compress_enqueued) {
            badges = badges.child(self.badge(label, p.green, p.green_soft));
        }

        let mut actions = h_flex().flex_wrap().gap(px(6.));
        if retryable {
            actions = actions.child(
                Button::new("notification-import-retry")
                    .label(tr!("notifications.import.retry_failed"))
                    .small()
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_notification_modal(window, cx);
                        this.open_retry_dialog(window, cx);
                    })),
            );
        }
        if !report.remaining.is_empty() {
            actions = actions.child(
                Button::new("notification-import-resume")
                    .label(tr_args!(
                        "notifications.import.resume",
                        count = report.remaining.len()
                    ))
                    .small()
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_notification_modal(window, cx);
                        this.open_resume_dialog(window, cx);
                    })),
            );
        }
        actions = actions
            .child(
                Button::new("notification-import-details")
                    .label(if expanded {
                        tr!("viewer.details_close")
                    } else {
                        tr!("viewer.details")
                    })
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.import_report_expanded = !this.import_report_expanded;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("notification-import-dismiss")
                    .label(tr!("notifications.import.close_result"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.import_report = None;
                        this.import_report_expanded = false;
                        this.import_auto_compress_enqueued = 0;
                        this.import_scan_target = None;
                        cx.notify();
                    })),
            );

        v_flex()
            .p(px(14.))
            .gap(px(10.))
            .rounded(px(14.))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .child(
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(rgb(p.text))
                            .child(tr!("notifications.import.title")),
                    )
                    .child(self.badge(mode, p.accent, p.accent_soft))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted_2))
                            .child(dest),
                    ),
            )
            .child(badges)
            .child(actions)
            .when(expanded, |this| {
                this.child(self.import_report_details(report))
            })
            .into_any_element()
    }

    fn app_update_notification(&self, version: &Version, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        v_flex()
            .p(px(14.))
            .gap(px(10.))
            .rounded(px(14.))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .text_size(px(12.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("update.title")),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(tr_args!("update.available", version = version)),
            )
            .child(
                h_flex().flex_wrap().gap(px(6.)).child(
                    Button::new("notification-update-open-settings")
                        .label(tr!("notifications.update.open_settings"))
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_notification_modal(window, cx);
                            this.switch_screen(Screen::Settings, cx);
                            this.settings_scroll.scroll_to_bottom();
                        })),
                ),
            )
            .into_any_element()
    }

    fn library_notification(&self, error: &str, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        v_flex()
            .p(px(14.))
            .gap(px(10.))
            .rounded(px(14.))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .text_size(px(12.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("notifications.section.library")),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.red))
                    .child(error.to_string()),
            )
            .child(
                Button::new("notification-library-reload")
                    .label(tr!("notifications.reload"))
                    .small()
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| {
                        let revision = this.db.library_revision().ok();
                        this.apply_library_snapshot(revision, cx);
                    })),
            )
            .into_any_element()
    }

    fn job_failures_notification(
        &self,
        snapshot: &NotificationSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        let mut counts = h_flex().flex_wrap().gap(px(5.));
        if snapshot.compression_failed > 0 {
            counts = counts.child(self.badge(
                tr_args!(
                    "notifications.job_failures.count_feature",
                    feature = job_feature_label(QueueFeature::Compression),
                    count = snapshot.compression_failed
                ),
                p.accent,
                p.accent_soft,
            ));
        }
        if snapshot.discord_failed > 0 {
            counts = counts.child(self.badge(
                tr_args!(
                    "notifications.job_failures.count_feature",
                    feature = job_feature_label(QueueFeature::Discord),
                    count = snapshot.discord_failed
                ),
                p.green,
                p.green_soft,
            ));
        }
        if snapshot.tagging_failed > 0 {
            counts = counts.child(self.badge(
                tr_args!(
                    "notifications.job_failures.count_feature",
                    feature = job_feature_label(QueueFeature::Tagging),
                    count = snapshot.tagging_failed
                ),
                p.amber,
                p.amber_soft,
            ));
        }

        let entries = latest_failed_entries(snapshot.entries.clone(), 5);
        let rows =
            entries.into_iter().map(|entry| {
                let feature = entry.feature;
                let job_id = entry.job_id.clone();
                let name = entry
                    .photo_name
                    .clone()
                    .or_else(|| entry.photo_id.map(|id| format!("photo #{id}")))
                    .unwrap_or_else(|| "—".to_string());
                let error = if entry.error.trim().is_empty() {
                    tr!("notifications.job_failures.error_generic")
                } else {
                    entry.error
                };
                let (feature_fg, feature_bg) = match feature {
                    QueueFeature::Compression => (p.accent, p.accent_soft),
                    QueueFeature::Discord => (p.green, p.green_soft),
                    QueueFeature::Tagging => (p.amber, p.amber_soft),
                };
                h_flex()
                    .gap(px(7.))
                    .p(px(7.))
                    .rounded(px(8.))
                    .bg(rgb(p.panel_2))
                    .child(self.badge(job_feature_label(feature), feature_fg, feature_bg))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(1.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.text))
                                    .child(name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(9.))
                                    .text_color(rgb(p.muted_2))
                                    .child(error),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "notification-job-retry-{}-{job_id}",
                            feature.as_str()
                        )))
                        .label(tr!("notifications.job_failures.retry"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| match feature {
                            QueueFeature::Compression => this.retry_job(job_id.clone(), cx),
                            QueueFeature::Discord => {
                                if let Ok(id) = job_id.parse::<i64>() {
                                    this.retry_failed_send(id, cx);
                                }
                            }
                            QueueFeature::Tagging => {
                                this.retry_failed_tagging_job(&job_id, cx);
                            }
                        })),
                    )
                    .into_any_element()
            });

        v_flex()
            .p(px(14.))
            .gap(px(10.))
            .rounded(px(14.))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .text_size(px(12.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("notifications.job_failures.title")),
            )
            .child(counts)
            .child(v_flex().gap(px(4.)).children(rows))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(5.))
                    .when(snapshot.compression_failed > 0, |this| {
                        this.child(
                            Button::new("notification-job-retry-all-compression")
                                .label(tr_args!(
                                    "notifications.job_failures.retry_all_feature",
                                    feature = job_feature_label(QueueFeature::Compression)
                                ))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.retry_all_failed_jobs(cx);
                                })),
                        )
                    })
                    .when(snapshot.discord_failed > 0, |this| {
                        this.child(
                            Button::new("notification-job-retry-all-discord")
                                .label(tr_args!(
                                    "notifications.job_failures.retry_all_feature",
                                    feature = job_feature_label(QueueFeature::Discord)
                                ))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.retry_all_failed_sends(cx);
                                })),
                        )
                    })
                    .when(snapshot.tagging_failed > 0, |this| {
                        this.child(
                            Button::new("notification-job-retry-all-tagging")
                                .label(tr_args!(
                                    "notifications.job_failures.retry_all_feature",
                                    feature = job_feature_label(QueueFeature::Tagging)
                                ))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.retry_all_failed_tagging_jobs(cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    fn daemon_notification(&self, count: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        v_flex()
            .p(px(14.))
            .gap(px(10.))
            .rounded(px(14.))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .text_size(px(12.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("notifications.section.daemon")),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.amber))
                    .child(tr_args!("notifications.daemon.errors", count = count)),
            )
            .child(
                Button::new("notification-daemon-dashboard")
                    .label(tr!("notifications.open_dashboard"))
                    .small()
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_notification_modal(window, cx);
                        this.switch_screen(Screen::Dashboard, cx);
                    })),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FailedJobEntry, NotificationSection, QueueFeature, auto_compress_badge_label,
        deletion_blocked_transition, latest_failed_entries, notification_attention_for,
        notification_modal_allowed, notification_modal_should_close, notification_sections,
        toast_flush_needed,
    };

    #[test]
    fn auto_compress_badge_is_hidden_for_zero_and_shows_the_count() {
        assert_eq!(auto_compress_badge_label(0), None);
        assert_eq!(auto_compress_badge_label(3).as_deref(), Some("自動圧縮 3"));
    }

    #[test]
    fn blocked_deletion_transition_notifies_only_on_increase() {
        assert_eq!(deletion_blocked_transition(None, (2, 1)), None);
        assert_eq!(deletion_blocked_transition(Some((2, 1)), (3, 1)), None);
        assert_eq!(deletion_blocked_transition(Some((3, 1)), (3, 2)), Some(2));
        assert_eq!(deletion_blocked_transition(Some((3, 2)), (3, 1)), None);
    }

    #[test]
    fn notification_sections_follow_the_action_order() {
        assert_eq!(
            notification_sections(false, true, true, 2, 1, 1, 3),
            vec![
                NotificationSection::Import,
                NotificationSection::Library,
                NotificationSection::JobFailures,
                NotificationSection::Daemon,
            ]
        );
        assert!(notification_sections(false, false, false, 0, 0, 0, 0).is_empty());
    }

    #[test]
    fn update_section_is_first_and_keeps_the_modal_open() {
        use NotificationSection::{AppUpdate, Daemon, Import, JobFailures, Library};

        assert_eq!(
            notification_sections(true, true, true, 2, 1, 1, 3),
            vec![AppUpdate, Import, Library, JobFailures, Daemon]
        );
        assert!(notification_sections(false, false, false, 0, 0, 0, 0).is_empty());
        assert!(!notification_modal_should_close(
            true, true, false, false, 0, 0, 0, 0
        ));
        assert!(notification_modal_should_close(
            true, false, false, false, 0, 0, 0, 0
        ));
    }

    #[test]
    fn an_unapplied_update_alone_raises_attention() {
        assert!(notification_attention_for(true, false, false, 0, 0));
        assert!(!notification_attention_for(false, false, false, 0, 0));
    }

    #[test]
    fn job_failures_section_appears_only_when_a_feature_has_failures() {
        assert!(
            !notification_sections(false, false, false, 0, 0, 0, 0)
                .contains(&NotificationSection::JobFailures)
        );
        assert!(
            notification_sections(false, false, false, 0, 0, 1, 0)
                .contains(&NotificationSection::JobFailures)
        );
    }

    #[test]
    fn failed_job_entries_are_newest_first_and_capped_at_five() {
        let entries = (0..7)
            .map(|index| FailedJobEntry {
                feature: QueueFeature::Compression,
                job_id: index.to_string(),
                photo_id: Some(index),
                photo_name: Some(format!("photo-{index}.png")),
                error: "boom".to_string(),
                created_at: index.to_string(),
            })
            .collect();
        let shown = latest_failed_entries(entries, 5);
        assert_eq!(shown.len(), 5);
        assert!(shown.windows(2).all(|w| w[0].created_at >= w[1].created_at));
        assert_eq!(shown[0].created_at, "6");
        assert_eq!(shown[4].created_at, "2");
    }

    #[test]
    fn toast_flush_is_scheduled_once_for_a_non_empty_queue() {
        assert!(!toast_flush_needed(0, false));
        assert!(toast_flush_needed(1, false));
        assert!(!toast_flush_needed(1, true));
    }

    #[test]
    fn notification_modal_never_stacks_over_another_modal() {
        assert!(notification_modal_allowed(false, false));
        assert!(!notification_modal_allowed(true, false));
        assert!(!notification_modal_allowed(false, true));
    }

    #[test]
    fn notification_modal_closes_when_the_last_section_disappears() {
        assert!(notification_modal_should_close(
            true, false, false, false, 0, 0, 0, 0
        ));
        assert!(!notification_modal_should_close(
            false, false, false, false, 0, 0, 0, 0
        ));
        assert!(!notification_modal_should_close(
            true, false, true, false, 0, 0, 0, 0
        ));
        assert!(!notification_modal_should_close(
            true, false, false, false, 1, 0, 0, 0
        ));
        assert!(!notification_modal_should_close(
            true, false, false, false, 0, 0, 0, 1
        ));
    }
}
