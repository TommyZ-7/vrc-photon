//! Queue pause, waiting deletion, and release actions shared by the three
//! integration cards and the dashboard.

use gpui_kit::component::WindowExt;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::*;
use vrc_photon_core::ai::AI_PLUGIN_ID;
use vrc_photon_core::queue::{QueueCounts, QueueFeature};
use vrc_photon_i18n::{tr, tr_args};
use vrc_photon_ipc::Request as IpcRequest;

use crate::app::App;

impl App {
    pub(crate) fn set_queue_paused(
        &mut self,
        feature: QueueFeature,
        paused: bool,
        cx: &mut Context<Self>,
    ) {
        match feature {
            QueueFeature::Compression => self.settings.auto_compress_queue_paused = paused,
            QueueFeature::Discord => self.settings.discord_queue_paused = paused,
            QueueFeature::Tagging => self.settings.ai_tagging_queue_paused = paused,
        }
        if let Err(error) = self.settings.save(&self.db) {
            self.toast_error(tr_args!("settings.save_failed", error = error), cx);
            return;
        }
        cx.notify();
    }

    pub(crate) fn feature_queue_counts(&self, feature: QueueFeature) -> QueueCounts {
        match feature {
            QueueFeature::Compression => self.compression_queue_counts,
            QueueFeature::Discord => self.discord_queue_counts,
            QueueFeature::Tagging => self.tagging_queue_counts,
        }
    }

    pub(crate) fn delete_waiting_entry(
        &mut self,
        feature: QueueFeature,
        job_id: &str,
        cx: &mut Context<Self>,
    ) {
        match self.db.delete_waiting_entry(feature, job_id) {
            Ok(true) => {
                self.toast_success(tr!("queue.delete_waiting_row.done"), cx);
            }
            Ok(false) => {}
            Err(error) => {
                self.toast_error(
                    tr_args!("queue.delete_waiting_row.error", error = error),
                    cx,
                );
            }
        }
        self.refresh_queue(cx);
    }

    pub(crate) fn delete_all_waiting(
        &mut self,
        feature: QueueFeature,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.feature_queue_counts(feature).waiting;
        if count <= 0 {
            self.toast_info(tr!("queue.empty"), cx);
            return;
        }
        let entity = cx.entity();
        let panel_deep = self.palette.panel_deep;
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = entity.clone();
            alert
                .title(tr!("queue.delete_waiting_all.confirm.title"))
                .bg(rgb(panel_deep))
                .description(tr_args!(
                    "queue.delete_waiting_all.confirm.body",
                    count = count
                ))
                .confirm()
                .ok_text(tr!("queue.delete_waiting_all.confirm.ok"))
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(tr!("common.cancel"))
                .on_ok(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.confirm_delete_all_waiting(feature, cx));
                    true
                })
        });
    }

    fn confirm_delete_all_waiting(&mut self, feature: QueueFeature, cx: &mut Context<Self>) {
        match self.db.delete_all_waiting_entries(feature) {
            Ok(0) => self.toast_info(tr!("queue.empty"), cx),
            Ok(count) => {
                self.toast_success(tr_args!("queue.delete_waiting_all.done", count = count), cx);
            }
            Err(error) => {
                self.toast_error(
                    tr_args!("queue.delete_waiting_all.error", error = error),
                    cx,
                );
            }
        }
        self.refresh_queue(cx);
    }

    pub(crate) fn release_waiting_queue(
        &mut self,
        feature: QueueFeature,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match feature {
            QueueFeature::Compression => self.request_release_waiting_jobs(window, cx),
            QueueFeature::Discord => self.release_waiting_sends(cx),
            QueueFeature::Tagging => self.release_waiting_ai_jobs(cx),
        }
    }

    fn release_waiting_ai_jobs(&mut self, cx: &mut Context<Self>) {
        match self.db.release_waiting_tagging_jobs(AI_PLUGIN_ID) {
            Ok(0) => self.toast_info(tr!("integrations.ai.backlog.empty"), cx),
            Ok(count) => {
                self.toast_success(tr_args!("queue.start_waiting.done", count = count), cx);
                self.wake_daemon(IpcRequest::WakeTagging, cx);
                self.refresh_ai_state(cx);
            }
            Err(error) => {
                self.toast_error(tr_args!("queue.start_waiting.error", error = error), cx);
            }
        }
        self.refresh_queue(cx);
    }

    pub(crate) fn refresh_queue(&mut self, cx: &mut Context<Self>) {
        let snapshot = crate::app::live::collect_jobs(&self.db);
        self.apply_job_snapshot(snapshot);
        cx.notify();
    }
}
