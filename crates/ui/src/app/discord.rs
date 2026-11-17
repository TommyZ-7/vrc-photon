//! App-side Discord integration state: webhook editing, destination label,
//! integration toggles, compression settings, automatic-send mode and consent,
//! and send-history refresh.
//!
//! Rendering stays in `ui::discord`; these methods only touch App fields and
//! the send outbox.

use chrono::Utc;
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::*;
use vrc_photon_core::discord::{WebhookUrlError, validate_webhook_url};
use vrc_photon_core::model::{PhotoStatus, SendHistoryEntry, SendKind, SendOrigin, SendStatus};
use vrc_photon_core::settings::{
    AppSettings, AutoMode, AutoScope, clear_discord_webhook, save_discord_webhook,
};
use vrc_photon_i18n::{tr, tr_args};

use crate::app::{App, Screen};

/// Label used when the user leaves the destination field blank.
const DEFAULT_DESTINATION: &str = "Discord";

/// Why a manual or test send cannot start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscordSendGuardError {
    Disabled,
    MissingWebhook,
    InvalidWebhook,
}

/// Blocks a send until the feature is enabled and a valid webhook is stored.
///
/// A stored URL is revalidated: the database may have been edited directly or
/// the URL may predate a validation change, and sending to anything other than
/// a Discord webhook must fail locally.
pub(crate) fn discord_send_guard(
    settings: &AppSettings,
    webhook: Option<&str>,
) -> Result<(), DiscordSendGuardError> {
    if !settings.discord_enabled {
        return Err(DiscordSendGuardError::Disabled);
    }
    let Some(webhook) = webhook else {
        return Err(DiscordSendGuardError::MissingWebhook);
    };
    if validate_webhook_url(webhook).is_err() {
        return Err(DiscordSendGuardError::InvalidWebhook);
    }
    Ok(())
}

/// The bulk action only needs the feature and a valid webhook; an inactive
/// automatic mode does not disable it.
pub(crate) fn discord_backlog_available(settings: &AppSettings, webhook: Option<&str>) -> bool {
    discord_send_guard(settings, webhook).is_ok()
}

/// Toast text for a manual send.
///
/// `Ok(true)` means the send was queued and is shown as a success toast;
/// `Ok(false)` means an active send for the photo already exists and is shown
/// as an info toast. A guard error maps to the warning shown before the user is
/// sent to the Integrations screen.
pub(crate) fn manual_send_toast(result: Result<bool, DiscordSendGuardError>) -> String {
    match result {
        Ok(true) => tr!("photos.action.discord_queued"),
        Ok(false) => tr!("photos.action.discord_waiting"),
        Err(DiscordSendGuardError::Disabled) => tr!("photos.action.discord_disabled"),
        Err(DiscordSendGuardError::MissingWebhook) => {
            tr!("photos.action.discord_missing_webhook")
        }
        Err(DiscordSendGuardError::InvalidWebhook) => {
            tr!("integrations.discord.webhook.invalid_url")
        }
    }
}

/// Toast text after releasing the waiting queue.
///
/// `0` means nothing was waiting, so the action is reported as a warning
/// rather than a success with a count of zero.
pub(crate) fn waiting_release_toast(count: usize) -> String {
    if count == 0 {
        tr!("queue.empty")
    } else {
        tr_args!("queue.start_waiting.done", count = count)
    }
}

/// Trims the destination label and falls back to `Discord` when blank, the
/// same normalization `AppSettings` applies when it is loaded or saved.
pub(crate) fn normalized_destination(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        DEFAULT_DESTINATION.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Catalog key for an inline webhook validation error.
pub(crate) fn webhook_error_key(error: WebhookUrlError) -> &'static str {
    match error {
        WebhookUrlError::HostNotAllowed => "integrations.discord.webhook.invalid_host",
        WebhookUrlError::InvalidPath => "integrations.discord.webhook.invalid_path",
        WebhookUrlError::Empty
        | WebhookUrlError::Invalid
        | WebhookUrlError::NotHttps
        | WebhookUrlError::PortNotAllowed => "integrations.discord.webhook.invalid_url",
    }
}

/// Automatic sending needs an explicit opt-in once. The stored timestamp is
/// the only signal, so upgrades never show the dialog again after consent.
pub(crate) fn needs_discord_consent(settings: &AppSettings) -> bool {
    settings.discord_consent_at.is_none()
}

/// Applies an automatic-send mode in place.
///
/// `was_off` is the mode observed before the click. The capture-time cutoff
/// only moves when automatic sending leaves `Off`, so switching between
/// `Immediate` and `Waiting` keeps the backlog boundary, and turning the mode
/// off never clears it.
pub(crate) fn apply_discord_auto_mode(
    settings: &mut AppSettings,
    mode: AutoMode,
    now: i64,
    _was_off: bool,
) {
    settings.discord_auto.set_mode(mode, now);
}

/// Restarts the bulk-action activation window at `now`.
///
/// The feature and the first webhook are feature-level resumes: they move the
/// boundary even when the automatic mode is off, because the bulk action still
/// works in that state.
fn refresh_auto_activation(settings: &mut AppSettings, now: i64) {
    settings.discord_auto.touch_activation(now);
}

/// Applies the master switch resuming from `false` to `true`.
pub(crate) fn auto_activation_for_feature_resume(
    settings: &mut AppSettings,
    was_enabled: bool,
    enabled: bool,
    now: i64,
) -> bool {
    if was_enabled || !enabled {
        return false;
    }
    refresh_auto_activation(settings, now);
    true
}

/// Applies the first webhook arriving after the integration had none.
pub(crate) fn auto_activation_for_webhook_resume(
    settings: &mut AppSettings,
    had_webhook: bool,
    has_webhook: bool,
    now: i64,
) -> bool {
    if had_webhook || !has_webhook {
        return false;
    }
    refresh_auto_activation(settings, now);
    true
}

impl App {
    pub(crate) fn set_discord_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.settings.discord_enabled == enabled {
            return;
        }
        let was_enabled = self.settings.discord_enabled;
        self.settings.discord_enabled = enabled;
        // Enabling again after a pause restarts the automatic window, so
        // photos captured while the feature was off are not posted.
        auto_activation_for_feature_resume(
            &mut self.settings,
            was_enabled,
            enabled,
            Utc::now().timestamp(),
        );
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save Discord integration setting");
        }
        cx.notify();
    }

    pub(crate) fn begin_discord_url_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.discord_url_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.discord_url_error = None;
        self.discord_url_editing = true;
        cx.notify();
    }

    pub(crate) fn save_discord_url(&mut self, cx: &mut Context<Self>) {
        let raw = self.discord_url_input.read(cx).value().trim().to_string();
        match validate_webhook_url(&raw) {
            Ok(parsed) => {
                let url = parsed.as_str().to_string();
                let had_webhook = self.discord_url.is_some();
                if let Err(error) = save_discord_webhook(&self.db, &url) {
                    tracing::warn!(%error, "cannot save the Discord webhook URL");
                    self.toast_error(tr_args!("settings.language.save_failed", error = error), cx);
                    cx.notify();
                    return;
                }
                self.discord_url = Some(url);
                self.discord_url_editing = false;
                self.discord_url_error = None;
                // The first webhook resumes automatic sending; the cutoff must
                // move so photos captured while it was missing stay behind.
                if auto_activation_for_webhook_resume(
                    &mut self.settings,
                    had_webhook,
                    true,
                    Utc::now().timestamp(),
                ) && let Err(error) = self.settings.save(&self.db)
                {
                    tracing::warn!(%error, "cannot save the Discord automatic-send cutoff");
                }
                self.toast_success(tr!("integrations.discord.webhook.saved"), cx);
            }
            Err(error) => {
                // Never echo the URL itself: it contains the webhook token.
                self.discord_url_error = Some(tr(webhook_error_key(error)).into());
            }
        }
        cx.notify();
    }

    pub(crate) fn clear_discord_url(&mut self, cx: &mut Context<Self>) {
        match clear_discord_webhook(&self.db) {
            Ok(()) => {
                self.discord_url = None;
                self.discord_url_editing = false;
                self.discord_url_error = None;
                self.toast_success(tr!("integrations.discord.webhook.removed"), cx);
            }
            Err(error) => {
                tracing::warn!(%error, "cannot clear the Discord webhook URL");
                self.toast_error(tr_args!("settings.language.save_failed", error = error), cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn save_discord_destination(&mut self, cx: &mut Context<Self>) {
        let raw = self.discord_destination_input.read(cx).value();
        let destination = normalized_destination(&raw);
        if self.settings.discord_destination != destination {
            self.settings.discord_destination = destination;
            if let Err(error) = self.settings.save(&self.db) {
                tracing::warn!(%error, "cannot save the Discord destination");
            }
        }
        cx.notify();
    }

    pub(crate) fn set_discord_send_original(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.settings.discord_send_original == value {
            return;
        }
        self.settings.discord_send_original = value;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save the Discord original-image setting");
        }
        cx.notify();
    }

    pub(crate) fn set_discord_include_players(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.settings.discord_include_players == value {
            return;
        }
        self.settings.discord_include_players = value;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save the Discord player-name setting");
        }
        cx.notify();
    }

    /// Switches the automatic-send mode.
    ///
    /// Turning it off also cancels queued waiting rows, because those are the
    /// automatic sends the user no longer wants. The first switch away from
    /// `Off` asks for consent and only applies the mode when the dialog is
    /// accepted.
    pub(crate) fn set_discord_auto_mode(
        &mut self,
        mode: AutoMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_off = self.settings.discord_auto.mode == AutoMode::Off;

        if mode == AutoMode::Off {
            self.settings.discord_auto.mode = AutoMode::Off;
            if let Err(error) = self.settings.save(&self.db) {
                tracing::warn!(%error, "cannot save the Discord automatic-send mode");
            }
            match self.db.cancel_waiting_auto_sends() {
                Ok(0) => {}
                Ok(count) => {
                    tracing::info!(count, "cancelled waiting automatic Discord sends");
                }
                Err(error) => {
                    tracing::warn!(%error, "cannot cancel waiting automatic Discord sends");
                }
            }
            self.reload_send_history(cx);
            self.refresh_queue(cx);
            return;
        }

        if needs_discord_consent(&self.settings) {
            let entity = cx.entity();
            let panel_deep = self.palette.panel_deep;
            window.open_alert_dialog(cx, move |alert, _, _| {
                let entity = entity.clone();
                alert
                    .title(tr!("integrations.discord.auto.consent.title"))
                    .bg(rgb(panel_deep))
                    .description(tr!("integrations.discord.auto.consent.body"))
                    .confirm()
                    .ok_text(tr!("integrations.discord.auto.consent.ok"))
                    .ok_variant(ButtonVariant::Primary)
                    .cancel_text(tr!("common.cancel"))
                    .on_ok(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.confirm_discord_auto_mode(mode, cx));
                        true
                    })
            });
            return;
        }

        self.save_discord_auto_mode(mode, was_off, cx);
    }

    pub(crate) fn set_discord_auto_scope(
        &mut self,
        scope: AutoScope,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.discord_auto.scope = scope;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save the Discord bulk scope");
        }
        cx.notify();
    }

    /// Records consent and applies the mode chosen in the consent dialog.
    pub(crate) fn confirm_discord_auto_mode(&mut self, mode: AutoMode, cx: &mut Context<Self>) {
        let was_off = self.settings.discord_auto.mode == AutoMode::Off;
        self.settings.discord_consent_at = Some(Utc::now().timestamp());
        self.save_discord_auto_mode(mode, was_off, cx);
    }

    fn save_discord_auto_mode(&mut self, mode: AutoMode, was_off: bool, cx: &mut Context<Self>) {
        apply_discord_auto_mode(&mut self.settings, mode, Utc::now().timestamp(), was_off);
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save the Discord automatic-send mode");
        }
        cx.notify();
    }

    /// Refreshes the history page and the waiting-queue badge. A queued test
    /// send reports its result once its row reaches a terminal state.
    pub(crate) fn reload_send_history(&mut self, cx: &mut Context<Self>) {
        let history = match self.db.send_history(100) {
            Ok(history) => Some(history),
            Err(error) => {
                tracing::warn!(%error, "cannot load the Discord send history");
                None
            }
        };
        let waiting = match self.db.count_waiting_sends() {
            Ok(count) => Some(count),
            Err(error) => {
                tracing::warn!(%error, "cannot count waiting Discord sends");
                None
            }
        };
        self.apply_send_history(history, waiting, cx);
    }

    /// Applies send rows that were read on a background connection.
    pub(crate) fn apply_send_history(
        &mut self,
        history: Option<Vec<SendHistoryEntry>>,
        waiting: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        if let Some(history) = history {
            self.send_history = history;
        }
        if let Some(count) = waiting {
            self.send_waiting_count = count;
        }
        if let Some(id) = self.pending_test_send {
            let status = self
                .send_history
                .iter()
                .find(|entry| entry.entry.id == id)
                .map(|entry| entry.entry.status);
            match status {
                Some(SendStatus::Sent) => {
                    self.pending_test_send = None;
                    self.toast_success(tr!("integrations.discord.test.success"), cx);
                }
                Some(SendStatus::Failed) => {
                    self.pending_test_send = None;
                    self.toast_error(tr!("integrations.discord.test.failed"), cx);
                }
                // A cancelled test send is not a webhook failure; the cancel
                // action already reported success, so clear it silently.
                Some(SendStatus::Cancelled) => {
                    self.pending_test_send = None;
                }
                _ => {}
            }
        }
        cx.notify();
    }

    pub(crate) fn send_test_message(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = discord_send_guard(&self.settings, self.discord_url.as_deref()) {
            self.toast_warning(manual_send_toast(Err(error)), cx);
            cx.notify();
            return;
        }
        match self.db.enqueue_test_send() {
            Ok(id) => {
                self.pending_test_send = Some(id);
                self.reload_send_history(cx);
                self.refresh_queue(cx);
                self.send_wake_sends(cx);
                self.toast_info(tr!("integrations.discord.test.queued"), cx);
            }
            Err(error) => {
                tracing::warn!(%error, "cannot enqueue a Discord test send");
                self.toast_error(tr!("integrations.discord.test.enqueue_failed"), cx);
            }
        }
        cx.notify();
    }

    /// Promotes every waiting send (manual or automatic) so the daemon picks
    /// them up. An empty queue is reported as a warning: the button is
    /// disabled at zero, so only a stale count reaches this path.
    pub(crate) fn release_waiting_sends(&mut self, cx: &mut Context<Self>) {
        match self.db.release_waiting_sends() {
            Ok(0) => self.toast_warning(waiting_release_toast(0), cx),
            Ok(count) => {
                self.reload_send_history(cx);
                self.refresh_queue(cx);
                self.send_wake_sends(cx);
                self.toast_success(waiting_release_toast(count), cx);
            }
            Err(error) => {
                tracing::warn!(%error, "cannot release waiting Discord sends");
                self.toast_error(tr_args!("queue.start_waiting.error", error = error), cx);
            }
        }
        cx.notify();
    }

    /// Sends every never-sent photo that has no active send. The count is
    /// shown in a count-only confirmation before anything is queued.
    pub(crate) fn send_discord_backlog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let since = self.settings.discord_auto.bulk_since();
        match self.db.count_unsent_photo_backlog(since) {
            Ok(0) => {
                self.toast_info(tr!("integrations.discord.backlog.empty"), cx);
            }
            Ok(count) => {
                let entity = cx.entity();
                let panel_deep = self.palette.panel_deep;
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let entity = entity.clone();
                    alert
                        .title(tr!("integrations.discord.backlog.confirm.title"))
                        .bg(rgb(panel_deep))
                        .description(tr_args!(
                            "integrations.discord.backlog.confirm.body",
                            count = count
                        ))
                        .confirm()
                        .ok_text(tr!("integrations.discord.backlog.confirm.ok"))
                        .ok_variant(ButtonVariant::Primary)
                        .cancel_text(tr!("common.cancel"))
                        .on_ok(move |_, _, cx| {
                            entity.update(cx, |this, cx| this.confirm_discord_backlog(cx));
                            true
                        })
                });
            }
            Err(error) => {
                tracing::warn!(%error, "cannot count the unsent Discord backlog");
                self.toast_error(
                    tr_args!("integrations.discord.backlog.error", error = error),
                    cx,
                );
            }
        }
        cx.notify();
    }

    pub(crate) fn confirm_discord_backlog(&mut self, cx: &mut Context<Self>) {
        let since = self.settings.discord_auto.bulk_since();
        match self.db.enqueue_unsent_photo_backlog(since) {
            Ok(0) => self.toast_info(tr!("integrations.discord.backlog.empty"), cx),
            Ok(count) => {
                if self.settings.discord_consent_at.is_none() {
                    self.settings.discord_consent_at = Some(Utc::now().timestamp());
                    if let Err(error) = self.settings.save(&self.db) {
                        tracing::warn!(%error, "cannot save Discord consent");
                    }
                }
                self.reload_send_history(cx);
                self.refresh_queue(cx);
                self.send_wake_sends(cx);
                self.toast_success(
                    tr_args!("integrations.discord.backlog.started", count = count),
                    cx,
                );
            }
            Err(error) => {
                tracing::warn!(%error, "cannot enqueue the unsent Discord backlog");
                self.toast_error(
                    tr_args!("integrations.discord.backlog.error", error = error),
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// Queues one photo for a manual Discord send.
    ///
    /// Returns `true` when the integration guard rejected the send and the app
    /// navigated to the Integrations screen. A modal viewer uses that to close
    /// itself and reveal the screen; every other outcome returns `false`.
    pub(crate) fn enqueue_photo_send(&mut self, photo_id: i64, cx: &mut Context<Self>) -> bool {
        if let Err(error) = discord_send_guard(&self.settings, self.discord_url.as_deref()) {
            self.toast_warning(manual_send_toast(Err(error)), cx);
            self.switch_screen(Screen::Integrations, cx);
            return true;
        }
        match self.db.photo_by_id(photo_id) {
            // A missing photo has no source to read, so it shares the rejection
            // path with a photo that is being deleted.
            Ok(Some(photo))
                if !matches!(photo.status, PhotoStatus::Deleting | PhotoStatus::Missing) => {}
            Ok(Some(photo)) => {
                let message = match photo.status {
                    PhotoStatus::Deleting => tr!("photos.action.discord_deleting"),
                    _ => tr!("photos.action.discord_error"),
                };
                self.toast_error(message, cx);
                cx.notify();
                return false;
            }
            Ok(None) => {
                self.toast_error(tr!("photos.action.discord_error"), cx);
                cx.notify();
                return false;
            }
            Err(error) => {
                tracing::warn!(%error, photo_id, "cannot load a photo for a Discord send");
                self.toast_error(
                    format!("{}: {error}", tr!("photos.action.discord_error")),
                    cx,
                );
                cx.notify();
                return false;
            }
        }
        match self.db.enqueue_send(
            photo_id,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        ) {
            Ok(Some(_)) => {
                self.reload_send_history(cx);
                self.refresh_queue(cx);
                self.send_wake_sends(cx);
                self.toast_success(manual_send_toast(Ok(true)), cx);
            }
            // An active row for this photo already exists; start it when it is
            // still waiting, otherwise the worker already owns it.
            Ok(None) => match self.db.start_waiting_send(photo_id) {
                Ok(true) => {
                    self.reload_send_history(cx);
                    self.refresh_queue(cx);
                    self.send_wake_sends(cx);
                    self.toast_success(manual_send_toast(Ok(true)), cx);
                }
                Ok(false) => self.toast_info(manual_send_toast(Ok(false)), cx),
                Err(error) => {
                    tracing::warn!(%error, photo_id, "cannot start a waiting Discord send");
                    self.toast_error(
                        format!("{}: {error}", tr!("photos.action.discord_error")),
                        cx,
                    );
                }
            },
            Err(error) => {
                tracing::warn!(%error, photo_id, "cannot enqueue a Discord photo send");
                self.toast_error(
                    format!("{}: {error}", tr!("photos.action.discord_error")),
                    cx,
                );
            }
        }
        cx.notify();
        false
    }

    pub(crate) fn retry_failed_send(&mut self, id: i64, cx: &mut Context<Self>) {
        self.retry_send(id, cx);
    }

    /// Returns a failed row to the queue. The database rejects the retry when
    /// the photo already has an active send, which the warning toast covers.
    fn retry_send(&mut self, id: i64, cx: &mut Context<Self>) {
        match self.db.retry_send(id) {
            Ok(true) => {
                self.send_wake_sends(cx);
                self.toast_success(
                    tr_args!("notifications.job_failures.retry_queued", count = 1),
                    cx,
                );
            }
            Ok(false) => {
                self.toast_warning(tr!("notifications.job_failures.retry_none"), cx);
            }
            Err(error) => {
                tracing::warn!(%error, id, "cannot retry a Discord send");
                self.toast_error(
                    tr_args!("notifications.job_failures.error", error = error),
                    cx,
                );
            }
        }
        self.refresh_queue(cx);
    }

    pub(crate) fn retry_all_failed_sends(&mut self, cx: &mut Context<Self>) {
        match self.db.retry_all_failed_sends() {
            Ok(0) => self.toast_info(tr!("notifications.job_failures.retry_none"), cx),
            Ok(count) => {
                self.send_wake_sends(cx);
                self.toast_success(
                    tr_args!("notifications.job_failures.retry_queued", count = count),
                    cx,
                );
            }
            Err(error) => {
                tracing::warn!(%error, "cannot retry failed Discord sends");
                self.toast_error(
                    tr_args!("notifications.job_failures.error", error = error),
                    cx,
                );
            }
        }
        self.refresh_queue(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DiscordSendGuardError, apply_discord_auto_mode, auto_activation_for_feature_resume,
        auto_activation_for_webhook_resume, discord_backlog_available, discord_send_guard,
        manual_send_toast, needs_discord_consent, normalized_destination, waiting_release_toast,
        webhook_error_key,
    };
    use vrc_photon_core::discord::{WebhookUrlError, mask_webhook_url, validate_webhook_url};
    use vrc_photon_core::settings::{AppSettings, AutoBehavior, AutoMode};
    use vrc_photon_i18n::{tr, tr_args};

    #[test]
    fn waiting_release_result_messages() {
        assert_eq!(waiting_release_toast(0), tr!("queue.empty"));
        assert_eq!(
            waiting_release_toast(3),
            tr_args!("queue.start_waiting.done", count = 3)
        );
    }

    #[test]
    fn enabling_auto_requires_consent_only_the_first_time() {
        let mut settings = AppSettings::default();
        assert!(needs_discord_consent(&settings));
        settings.discord_consent_at = Some(1);
        assert!(!needs_discord_consent(&settings));
    }

    #[test]
    fn changing_mode_sets_activation_only_when_leaving_off() {
        let mut settings = AppSettings::default();
        settings.discord_auto.mode = AutoMode::Off;
        apply_discord_auto_mode(&mut settings, AutoMode::Waiting, 100, true);
        assert_eq!(settings.discord_auto.activated_at, Some(100));
        apply_discord_auto_mode(&mut settings, AutoMode::Immediate, 200, false);
        assert_eq!(settings.discord_auto.activated_at, Some(100));
    }

    #[test]
    fn disabling_auto_keeps_the_activation() {
        let mut settings = AppSettings {
            discord_auto: AutoBehavior {
                activated_at: Some(42),
                ..AutoBehavior::immediate()
            },
            ..AppSettings::default()
        };
        apply_discord_auto_mode(&mut settings, AutoMode::Off, 99, false);
        assert_eq!(settings.discord_auto.mode, AutoMode::Off);
        assert_eq!(settings.discord_auto.activated_at, Some(42));
    }

    #[test]
    fn enabling_the_feature_updates_the_activation_only_when_resuming() {
        let mut settings = AppSettings {
            discord_auto: AutoBehavior {
                activated_at: Some(42),
                ..AutoBehavior::immediate()
            },
            ..AppSettings::default()
        };
        assert!(!auto_activation_for_feature_resume(
            &mut settings,
            true,
            true,
            99
        ));
        assert_eq!(settings.discord_auto.activated_at, Some(42));
        assert!(!auto_activation_for_feature_resume(
            &mut settings,
            true,
            false,
            99
        ));
        assert_eq!(settings.discord_auto.activated_at, Some(42));
        assert!(auto_activation_for_feature_resume(
            &mut settings,
            false,
            true,
            99
        ));
        assert_eq!(settings.discord_auto.activated_at, Some(99));
    }

    #[test]
    fn saving_the_first_webhook_updates_the_activation_only_when_resuming() {
        let mut settings = AppSettings {
            discord_auto: AutoBehavior {
                activated_at: Some(42),
                ..AutoBehavior::waiting()
            },
            ..AppSettings::default()
        };
        assert!(!auto_activation_for_webhook_resume(
            &mut settings,
            true,
            true,
            99
        ));
        assert_eq!(settings.discord_auto.activated_at, Some(42));
        assert!(!auto_activation_for_webhook_resume(
            &mut settings,
            true,
            false,
            99
        ));
        assert_eq!(settings.discord_auto.activated_at, Some(42));
        assert!(auto_activation_for_webhook_resume(
            &mut settings,
            false,
            true,
            99
        ));
        assert_eq!(settings.discord_auto.activated_at, Some(99));
    }

    #[test]
    fn resume_updates_the_activation_even_when_auto_mode_is_off() {
        let mut settings = AppSettings {
            discord_auto: AutoBehavior {
                activated_at: Some(42),
                ..AutoBehavior::off()
            },
            ..AppSettings::default()
        };
        assert!(auto_activation_for_feature_resume(
            &mut settings,
            false,
            true,
            99
        ));
        assert_eq!(settings.discord_auto.activated_at, Some(99));
        assert!(auto_activation_for_webhook_resume(
            &mut settings,
            false,
            true,
            100
        ));
        assert_eq!(settings.discord_auto.activated_at, Some(100));
    }

    #[test]
    fn discord_backlog_button_does_not_require_an_active_auto_mode() {
        let mut settings = AppSettings {
            discord_enabled: true,
            discord_auto: AutoBehavior::off(),
            ..AppSettings::default()
        };
        assert!(discord_backlog_available(
            &settings,
            Some("https://discord.com/api/webhooks/1/token")
        ));
        settings.discord_enabled = false;
        assert!(!discord_backlog_available(
            &settings,
            Some("https://discord.com/api/webhooks/1/token")
        ));
    }

    #[test]
    fn manual_send_reports_queued_and_duplicate_states() {
        assert_eq!(
            manual_send_toast(Ok(true)),
            tr!("photos.action.discord_queued")
        );
        assert_eq!(
            manual_send_toast(Ok(false)),
            tr!("photos.action.discord_waiting")
        );
        assert_eq!(
            manual_send_toast(Err(DiscordSendGuardError::Disabled)),
            tr!("photos.action.discord_disabled")
        );
        assert_eq!(
            manual_send_toast(Err(DiscordSendGuardError::MissingWebhook)),
            tr!("photos.action.discord_missing_webhook")
        );
    }

    #[test]
    fn webhook_mask_and_validation_feed_the_editor() {
        assert!(validate_webhook_url("https://discord.com/api/webhooks/1/t").is_ok());
        assert!(!mask_webhook_url("https://discord.com/api/webhooks/1/secret").contains("secret"));
    }

    #[test]
    fn save_destination_falls_back_to_discord_for_blank_input() {
        assert_eq!(normalized_destination("   "), "Discord");
        assert_eq!(normalized_destination(" My server "), "My server");
    }

    #[test]
    fn discord_send_guard_requires_enabled_and_a_valid_webhook() {
        let webhook = "https://discord.com/api/webhooks/1/t";
        let mut settings = AppSettings::default();
        assert_eq!(
            discord_send_guard(&settings, None),
            Err(DiscordSendGuardError::Disabled)
        );
        settings.discord_enabled = true;
        assert_eq!(
            discord_send_guard(&settings, None),
            Err(DiscordSendGuardError::MissingWebhook)
        );
        assert_eq!(
            discord_send_guard(&settings, Some("http://discord.com/api/webhooks/1/t")),
            Err(DiscordSendGuardError::InvalidWebhook)
        );
        assert_eq!(discord_send_guard(&settings, Some(webhook)), Ok(()));
    }

    #[test]
    fn an_invalid_stored_webhook_maps_to_the_inline_error_toast() {
        assert_eq!(
            manual_send_toast(Err(DiscordSendGuardError::InvalidWebhook)),
            tr!("integrations.discord.webhook.invalid_url")
        );
    }

    #[test]
    fn webhook_errors_map_to_catalog_keys() {
        assert_eq!(
            webhook_error_key(WebhookUrlError::NotHttps),
            "integrations.discord.webhook.invalid_url"
        );
        assert_eq!(
            webhook_error_key(WebhookUrlError::HostNotAllowed),
            "integrations.discord.webhook.invalid_host"
        );
        assert_eq!(
            webhook_error_key(WebhookUrlError::InvalidPath),
            "integrations.discord.webhook.invalid_path"
        );
    }
}
