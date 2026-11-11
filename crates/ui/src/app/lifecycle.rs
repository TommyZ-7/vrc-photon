//! App lifecycle: startup timers, daemon IPC, library reloads, and navigation.
//!
//! `App::new` stays in `app.rs`; these methods run after construction and keep
//! the live library, job queue, and daemon status in sync.

use std::time::{Duration, Instant};

use gpui_kit::*;
use vrc_photon_core::model::{CompressionProfile, LibraryStats, PhotoSort};
use vrc_photon_core::settings::AppSettings;
use vrc_photon_i18n::tr;
use vrc_photon_ipc::{
    Endpoint, Event as IpcEvent, JobStatus, Request as IpcRequest, ScanPhase as IpcScanPhase,
};

use crate::app::cli::log_timing;
use crate::app::live::{AuxSnapshot, LiveRefreshRequest, collect_jobs, collect_library_snapshot};
use crate::app::{App, Screen};
use crate::ipc_bridge;
use crate::state::photo_browser::{apply_library_index, sync_search_mirror};
use crate::ui::palette::{Palette, apply_component_theme, resolve};
use crate::ui::shell::screen_direction;
use crate::ui::theme::{MOTION_BASE, MOTION_SIDEBAR, MOTION_SLOW};

/// Current daemon scan progress, shown by the sidebar status card and the
/// dashboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScanProgressView {
    pub(crate) phase: IpcScanPhase,
    pub(crate) processed: usize,
    pub(crate) total: Option<usize>,
    pub(crate) indexed: usize,
    pub(crate) root: usize,
    pub(crate) roots: usize,
}

/// Returns the updated import auto-compression count when a finished scan
/// targets the root the import asked for. Unrelated scans (including full
/// scans and other imports) must not consume the pending target.
pub(crate) fn import_scan_enqueued_count(
    pending_target: Option<&str>,
    event_target: Option<&str>,
    current: usize,
    auto_enqueued: usize,
) -> Option<usize> {
    pending_target
        .zip(event_target)
        .filter(|(pending, event)| pending == event)
        .map(|_| current.saturating_add(auto_enqueued))
}

/// True when a live snapshot was collected before a revision the UI has
/// already applied. Revisions are monotonic; an older observation must not
/// trigger a reload or count comparison.
pub(crate) fn is_stale_revision(observed: Option<i64>, current: i64) -> bool {
    observed.is_some_and(|observed| observed < current)
}

/// Quiet period a revision or photo count must hold before the UI rebuilds the
/// whole index. Import bursts bump both several times per second.
const LIBRARY_QUIET_PERIOD: Duration = Duration::from_secs(2);

/// Collapses the per-message repaint decisions of one drained IPC batch into a
/// single notification. Progress events arrive several times per second, so
/// notifying once per message used to rebuild the whole window each time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IpcBatchNotify {
    needed: bool,
}

impl IpcBatchNotify {
    pub(crate) fn record(&mut self, needed: bool) {
        self.needed |= needed;
    }

    pub(crate) fn needed(self) -> bool {
        self.needed
    }
}

/// Pulls the first message from the bridge and then every message that is
/// already queued, preserving arrival order. The caller handles the whole
/// batch in one UI update and repaints once.
pub(crate) fn drain_ipc_batch(
    first: ipc_bridge::Incoming,
    rx: &async_channel::Receiver<ipc_bridge::Incoming>,
) -> Vec<ipc_bridge::Incoming> {
    let mut batch = Vec::new();
    batch.push(first);
    while let Ok(next) = rx.try_recv() {
        batch.push(next);
    }
    batch
}

/// What a finished scan should refresh, based on the mounted screen and
/// whether anything actually changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanRefresh {
    None,
    Snapshot,
    Search,
    Dirty,
}

fn scan_refresh_action(
    screen: Screen,
    has_applied_search: bool,
    revision: Option<i64>,
    last_revision: i64,
    library_dirty: bool,
) -> ScanRefresh {
    match screen {
        Screen::Photos => {
            if library_dirty || revision.is_some_and(|revision| revision > last_revision) {
                ScanRefresh::Snapshot
            } else {
                ScanRefresh::None
            }
        }
        Screen::Search if has_applied_search => ScanRefresh::Search,
        _ => ScanRefresh::Dirty,
    }
}

/// Keeps the live settings when a reload fails.
///
/// `AppSettings::load` already copied the malformed JSON to
/// `app_settings_corrupt`. Replacing the in-memory settings with defaults
/// here would reset every switch in the UI and make the next save persist
/// those defaults.
fn settings_after_reload(
    current: &AppSettings,
    loaded: vrc_photon_core::Result<AppSettings>,
) -> AppSettings {
    match loaded {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(%error, "cannot load app settings; keeping the current settings");
            current.clone()
        }
    }
}

impl App {
    pub(crate) fn start_ipc(&mut self, cx: &mut Context<Self>) {
        let Some(data_dir) = vrc_photon_core::data_dir() else {
            return;
        };
        let endpoint = match Endpoint::for_data_dir(&data_dir) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                tracing::warn!(%error, "cannot resolve daemon IPC endpoint");
                return;
            }
        };
        self.ipc_endpoint = Some(endpoint.clone());
        let (tx, rx) = async_channel::bounded(ipc_bridge::INCOMING_QUEUE_CAPACITY);
        ipc_bridge::start(endpoint, tx);
        cx.spawn(async move |this, cx| {
            while let Ok(incoming) = rx.recv().await {
                let batch = drain_ipc_batch(incoming, &rx);
                if this
                    .update(cx, |this, cx| this.handle_ipc_batch(batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Handles one drained batch and repaints at most once, after every
    /// message in it has been applied.
    fn handle_ipc_batch(&mut self, batch: Vec<ipc_bridge::Incoming>, cx: &mut Context<Self>) {
        let mut notify = IpcBatchNotify::default();
        for incoming in batch {
            notify.record(self.handle_ipc_incoming(incoming, cx));
        }
        if notify.needed() {
            cx.notify();
        }
    }

    /// Applies one bridge message. Returns whether the window needs a repaint;
    /// the batch caller folds those decisions into one notification.
    fn handle_ipc_incoming(
        &mut self,
        incoming: ipc_bridge::Incoming,
        cx: &mut Context<Self>,
    ) -> bool {
        match incoming {
            ipc_bridge::Incoming::Connected { status, daemon_pid } => {
                self.ipc_connected = true;
                self.daemon_pid = Some(daemon_pid);
                self.ipc_error = None;
                self.daemon_shutdown_armed = false;
                // A new connection may belong to a restarted daemon whose
                // version counter started over; adopt its lineage.
                self.last_status_version = status.version;
                self.refresh_monitor();
                self.apply_daemon_status(status);
                // A restarted daemon may have finished an install/probe while
                // the link was down. Reload the cards and the inspector.
                self.refresh_ai_state(cx);
                self.refresh_selected_ai(cx);
                // A batch held while disconnected can now be reported.
                self.flush_thumbnail_notice(cx);
                true
            }
            ipc_bridge::Incoming::Disconnected(error) => {
                let was_connected = self.ipc_connected;
                self.ipc_connected = false;
                // Keep the last PID: the daemon may still be alive with a dead
                // IPC server. The update helper will wait for it (or time out)
                // instead of replacing files under a running process.
                self.ipc_error = Some(error);
                self.daemon_paused = false;
                self.daemon_shutdown_armed = false;
                self.scan_progress = None;
                self.import_scan_target = None;
                // No daemon event can finish an in-flight install, probe, or
                // download while the link is down. Drop the transient state so
                // the cards cannot stay on "Installing…".
                self.ai.reset_connection();
                if was_connected {
                    self.refresh_monitor();
                }
                true
            }
            ipc_bridge::Incoming::Event(event) => self.handle_ipc_event(event, cx),
        }
    }

    fn handle_ipc_event(&mut self, event: IpcEvent, cx: &mut Context<Self>) -> bool {
        match event {
            IpcEvent::ScanStarted { roots, .. } => {
                self.scan_progress = Some(ScanProgressView {
                    phase: IpcScanPhase::Walking,
                    processed: 0,
                    total: None,
                    indexed: 0,
                    root: usize::from(roots > 0),
                    roots,
                });
                true
            }
            IpcEvent::ScanProgress {
                phase,
                processed,
                total,
                indexed,
                root,
                roots,
            } => {
                self.scan_progress = Some(ScanProgressView {
                    phase,
                    processed,
                    total,
                    indexed,
                    root,
                    roots,
                });
                true
            }
            IpcEvent::ScanFinished {
                auto_enqueued,
                target,
                ..
            } => {
                if let Some(count) = import_scan_enqueued_count(
                    self.import_scan_target.as_deref(),
                    target.as_deref(),
                    self.import_auto_compress_enqueued,
                    auto_enqueued,
                ) {
                    self.import_scan_target = None;
                    self.import_auto_compress_enqueued = count;
                }
                self.scan_progress = None;
                self.refresh_library_after_scan(cx);
                true
            }
            IpcEvent::CompressionProgress {
                photo_id, status, ..
            } => {
                let completed = matches!(&status, JobStatus::Completed);
                let library_changed =
                    completed || matches!(&status, JobStatus::Failed | JobStatus::Cancelled);
                if completed {
                    // A `replace` job changed the displayed file. Only that
                    // terminal state can publish a new derivative; failed and
                    // cancelled jobs leave every cache entry valid. Check the
                    // database because `keep` jobs also complete.
                    let replaced = self
                        .db
                        .photo_by_id(photo_id)
                        .ok()
                        .flatten()
                        .is_some_and(|photo| photo.status.is_replaced());
                    if replaced {
                        self.invalidate_photo_media(photo_id, cx);
                    }
                }
                self.schedule_compression_refresh(library_changed, cx);
                true
            }
            IpcEvent::DaemonErrorChanged { .. } => {
                // Daemon errors are part of the background live snapshot; the
                // failure toast transition runs when it is applied.
                self.refresh_live_async(cx);
                true
            }
            IpcEvent::SendProgress { .. } => {
                // A send moved between waiting / pending / processing /
                // terminal; the history page and the waiting badge follow it.
                self.reload_send_history(cx);
                true
            }
            IpcEvent::StatusChanged { status } => {
                if crate::app::daemon::status_snapshot_is_newer(
                    status.version,
                    self.last_status_version,
                ) {
                    self.last_status_version = self.last_status_version.max(status.version);
                    self.apply_daemon_status(status);
                    self.refresh_monitor();
                    true
                } else {
                    false
                }
            }
            // AI tagging state is owned by `app::ai`. Events for other
            // plugins do not repaint this window.
            event @ (IpcEvent::TaggingProgress { .. }
            | IpcEvent::PluginInstallProgress { .. }
            | IpcEvent::PluginStateChanged { .. }
            | IpcEvent::ModelDownloadProgress { .. }) => self.handle_ai_event(event, cx),
        }
    }

    pub(crate) fn wake_daemon(&self, request: IpcRequest, cx: &mut Context<Self>) {
        if !self.ipc_connected {
            return;
        }
        let Some(endpoint) = self.ipc_endpoint.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { ipc_bridge::request(endpoint, request) })
                .await;
            if let Err(error) = result {
                tracing::debug!(%error, "daemon IPC wake failed");
            }
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }

    pub(crate) fn send_ipc_request(
        &mut self,
        request: IpcRequest,
        success: Option<String>,
        failure: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.ipc_connected {
            if let Some(message) = failure {
                self.toast_error(message, cx);
            }
            cx.notify();
            return;
        }
        let Some(endpoint) = self.ipc_endpoint.clone() else {
            self.toast_error(tr!("daemon.ipc.resolve_error"), cx);
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { ipc_bridge::request(endpoint, request) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        if let Some(message) = success {
                            this.toast_success(message, cx);
                        }
                    }
                    Err(error) => {
                        this.toast_error(
                            failure.unwrap_or_else(|| ipc_bridge::error_message(&error)),
                            cx,
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Coalesces compression progress into one refresh per 250 ms. The daemon
    /// reports once at job start and once at completion; a batch of 1,000 jobs
    /// used to rebuild the 100k index once per completion.
    fn schedule_compression_refresh(&mut self, library_changed: bool, cx: &mut Context<Self>) {
        self.compression_library_refresh_pending |= library_changed;
        if self.compression_refresh_scheduled {
            return;
        }
        self.compression_refresh_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            this.update(cx, |this, cx| {
                this.compression_refresh_scheduled = false;
                if this.compression_library_refresh_pending {
                    this.compression_library_refresh_pending = false;
                    this.refresh_after_compression(cx);
                } else {
                    // Progress-only ticks move the job rows; collect them in
                    // the same background pass as the terminal path.
                    this.refresh_live_async(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Compression changes photo metadata (display derivative, replaced
    /// status) but not the id set or order, so the index snapshot stays valid.
    /// Invalidate the page cache and refresh the side panels instead of
    /// re-querying the 100k-row index.
    fn refresh_after_compression(&mut self, cx: &mut Context<Self>) {
        // Claim the revision before the (background) metadata refresh so a
        // poll that lands mid-refresh does not start a full index reload.
        if let Ok(revision) = self.db.library_revision() {
            self.last_library_revision = revision;
        }
        self.revision_debounce.reset();
        self.count_debounce.reset();
        self.library.borrow_mut().invalidate_pages();
        self.search_view.borrow_mut().invalidate_pages();
        if let Some(id) = self.selected {
            self.load_selected_meta(id);
            if let Some(photo) = self.selected_photo.clone() {
                // A replace invalidation drops the pinned thumbnail; request
                // it again now that the display derivative is known.
                self.request_thumbnail(&photo, cx);
            }
        }
        if self.screen != Screen::Photos {
            self.mark_library_dirty();
        }
        // Stats, the compressed-id scan, and the job counters moved to the
        // background snapshot; this path used to run them on the UI thread
        // after every 250 ms compression burst.
        self.refresh_live_async(cx);
        cx.notify();
    }

    /// Synchronous live refresh used only by in-memory/demo databases, which
    /// have no second handle for the background snapshot.
    pub(crate) fn refresh_live(&mut self, cx: &mut Context<Self>) {
        let count = self.db.count_photos();
        let revision = self.db.library_revision();
        let revision_value = revision.as_ref().ok().copied();
        if let Ok(ai_revision) = self.db.ai_revision()
            && ai_revision > self.last_ai_revision
        {
            self.last_ai_revision = ai_revision;
            self.refresh_search_after_ai_tag_edit(cx);
            self.refresh_selected_ai(cx);
        }
        if let Ok(session_revision) = self.db.session_revision()
            && session_revision > self.last_session_revision
        {
            self.last_session_revision = session_revision;
            self.library.borrow_mut().invalidate_session_summaries();
        }
        if let Some(revision) = revision_value
            && revision != self.stats_revision
        {
            if let Ok(stats) = self.db.stats() {
                self.stats = stats;
                self.stats_revision = revision;
            }
            if let Ok(ids) = self.db.compressed_photo_ids() {
                self.compressed_ids = ids.into_iter().collect();
            }
            if let Ok(ids) = self.db.duplicate_photo_ids() {
                self.duplicate_ids = ids.into_iter().collect();
            }
        }
        self.reload_job_state(cx);
        self.refresh_monitor();
        if let Some(id) = self.selected {
            self.load_selected_meta(id);
        }

        // Path rebinds and replacement status changes keep the same photo
        // count. The daemon bumps this revision so the UI can invalidate its
        // page cache even when `count_photos` is unchanged.
        self.apply_library_change(revision_value, count.ok(), cx);
        cx.notify();
    }

    /// Applies a revision/count observation from the live poller. Returns true
    /// when the visible library was rebuilt or a search was re-run.
    pub(crate) fn apply_library_change(
        &mut self,
        revision: Option<i64>,
        count: Option<i64>,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut changed = false;
        // A snapshot can be collected just before the UI bumps the revision
        // (trash, compression). Revisions are monotonic, so ignore an older
        // observation instead of reacting to it as a new change.
        if is_stale_revision(revision, self.last_library_revision) {
            return false;
        }
        let now = Instant::now();
        // A terminal compression refresh was already scheduled for this
        // revision. Its light path invalidates pages and reloads the side
        // panels without re-querying the 100k index; let it consume the
        // revision instead of racing it with a full reload.
        let compression_refresh_pending = self.compression_library_refresh_pending;
        if let Some(revision) = revision
            && revision > self.last_library_revision
            && !compression_refresh_pending
        {
            let expensive = self.screen == Screen::Photos
                || (self.screen == Screen::Search && self.applied_search.is_some());
            if expensive {
                // A burst of revisions must not rebuild the 100k-entry index
                // on every poll tick. Wait until the revision has been stable
                // for one quiet period.
                if self
                    .revision_debounce
                    .observe(revision, now, LIBRARY_QUIET_PERIOD)
                {
                    if self.screen == Screen::Photos {
                        if self.apply_library_snapshot(Some(revision), cx) {
                            changed = true;
                        }
                    } else {
                        self.last_library_revision = revision;
                        self.revision_debounce.reset();
                        if self.applied_search.is_some() {
                            self.refresh_search_keeping_anchor(cx);
                            changed = true;
                        }
                    }
                } else {
                    self.mark_library_dirty();
                }
            } else {
                self.last_library_revision = revision;
                self.mark_library_dirty();
                if let Some(id) = self.selected {
                    self.load_selected_meta(id);
                }
            }
        }

        // Only compare counts when the query succeeds; treating a transient
        // error as zero used to blank the whole library.
        if let Some(count) = count {
            if count != self.library.borrow().index.total as i64 {
                // A burst of files arriving from the daemon must not rebuild
                // the 100k-entry index on every three-second tick. Wait until
                // the observed count has been stable for a quiet period.
                self.mark_library_dirty();
                if self.screen == Screen::Photos
                    && self
                        .count_debounce
                        .observe(count, now, LIBRARY_QUIET_PERIOD)
                    && self.apply_library_snapshot(revision, cx)
                {
                    changed = true;
                }
            } else {
                self.count_debounce.reset();
            }
        }
        changed
    }

    /// Rebuilds the visible library snapshot and records the revision that
    /// triggered it. `observed_revision` must be read before the snapshot
    /// query so a concurrent commit cannot be swallowed. File-backed
    /// databases apply the revision when the background snapshot lands.
    pub(crate) fn apply_library_snapshot(
        &mut self,
        observed_revision: Option<i64>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_library_snapshot_with_anchor(observed_revision, true, cx)
    }

    /// `apply_library_snapshot` with control over the scroll anchor. A root
    /// change resets to the first row; every other caller keeps the anchor.
    pub(crate) fn apply_library_snapshot_with_anchor(
        &mut self,
        observed_revision: Option<i64>,
        preserve_anchor: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.reload_library(preserve_anchor, cx) {
            self.mark_library_dirty();
            return false;
        }
        // A background rebuild records the observed revision when its
        // snapshot lands. Claiming it here would swallow a failed rebuild and
        // suppress the retry.
        if !self.library_loading {
            if let Some(revision) = observed_revision {
                self.last_library_revision = self.last_library_revision.max(revision);
            }
            self.count_debounce.reset();
            self.revision_debounce.reset();
        }
        true
    }

    /// A finished scan is a natural end to a revision burst, so show its
    /// result once immediately. Watcher batches after this point still go
    /// through the quiet-period debounce, and a no-op scan does not rebuild
    /// the index at all.
    fn refresh_library_after_scan(&mut self, cx: &mut Context<Self>) {
        let revision = self.db.library_revision().ok();
        let action = scan_refresh_action(
            self.screen,
            self.applied_search.is_some(),
            revision,
            self.last_library_revision,
            self.library_dirty,
        );
        match action {
            ScanRefresh::Snapshot => {
                // A full index rebuild carries the stats, jobs, and monitor
                // in the same background snapshot; scheduling a second live
                // pass here would duplicate those queries. A failed rebuild
                // refreshes the side panels from the live path instead.
                if !self.apply_library_snapshot(revision, cx) {
                    self.refresh_live_async(cx);
                }
                return;
            }
            ScanRefresh::Search => {
                if let Some(revision) = revision {
                    self.last_library_revision = self.last_library_revision.max(revision);
                }
                self.revision_debounce.reset();
                self.count_debounce.reset();
                self.refresh_search_keeping_anchor(cx);
            }
            ScanRefresh::Dirty => {
                if let Some(revision) = revision {
                    self.last_library_revision = self.last_library_revision.max(revision);
                }
                self.mark_library_dirty();
            }
            ScanRefresh::None => {
                self.revision_debounce.reset();
                self.count_debounce.reset();
            }
        }
        // The index did not need a rebuild, but the scan can still have moved
        // job counters and the monitor. Collect those off the UI thread.
        if let Some(id) = self.selected {
            self.load_selected_meta(id);
        }
        self.refresh_live_async(cx);
    }

    pub(crate) fn reload_job_state(&mut self, cx: &mut Context<Self>) {
        let job_apply = self.apply_job_snapshot(collect_jobs(&self.db));
        let notified =
            self.apply_failure_transition(job_apply.failed_count, job_apply.daemon_count, cx);
        if job_apply.changed || notified {
            cx.notify();
        }
    }

    pub(crate) fn ensure_profiles(&mut self) {
        self.profiles = self.db.ensure_default_profiles().unwrap_or_else(|error| {
            tracing::warn!(%error, "cannot ensure default compression profiles");
            Vec::new()
        });
    }

    /// Reloads the compression profiles, keeping the current list when the
    /// read fails so a transient error cannot empty the UI.
    pub(crate) fn reload_profiles(&mut self) {
        match self.db.list_profiles() {
            Ok(profiles) => self.profiles = profiles,
            Err(error) => {
                tracing::warn!(%error, "cannot load compression profiles");
            }
        }
    }

    /// Refreshes every side panel. Used by the synchronous demo path and by
    /// actions that need both the aggregates and the settings/theme refresh.
    pub(crate) fn reload_aux(&mut self, cx: &mut Context<Self>) {
        self.reload_aux_live(cx);
        self.reload_aux_ui(cx);
    }

    /// Recomputes the expensive library aggregates on the UI thread.
    /// File-backed rebuilds collect these on the library reader instead.
    fn reload_aux_live(&mut self, cx: &mut Context<Self>) {
        match self.db.stats() {
            Ok(stats) => {
                self.stats = stats;
                self.stats_revision = self.db.library_revision().unwrap_or(-1);
            }
            Err(error) => {
                tracing::warn!(%error, "cannot load library stats");
                self.stats = LibraryStats::default();
                self.stats_revision = -1;
            }
        }
        self.compressed_ids = self
            .db
            .compressed_photo_ids()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "cannot load compressed photo ids");
                Vec::new()
            })
            .into_iter()
            .collect();
        self.duplicate_ids = self
            .db
            .duplicate_photo_ids()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "cannot load duplicate photo ids");
                Vec::new()
            })
            .into_iter()
            .collect();
        self.reload_job_state(cx);
    }

    /// Refreshes profiles, settings, Discord history, the daemon monitor, and
    /// the selected photo. No library-wide aggregate runs here.
    pub(crate) fn reload_aux_ui(&mut self, cx: &mut Context<Self>) {
        self.reload_profiles();
        if self.profile_index >= self.profiles.len() {
            self.profile_index = 0;
        }
        let settings = settings_after_reload(&self.settings, AppSettings::load(&self.db));
        self.apply_loaded_settings(settings, cx);
        self.reload_send_history(cx);
        self.refresh_monitor();
        if let Some(id) = self.selected {
            self.load_selected_meta(id);
        }
    }

    /// Applies the side-panel part of a full background rebuild.
    pub(crate) fn apply_aux_snapshot(&mut self, aux: AuxSnapshot, cx: &mut Context<Self>) {
        let mut notify = false;
        if let Some(profiles) = aux.profiles {
            self.apply_profiles(profiles);
            notify = true;
        }
        if let Some(settings) = aux.settings {
            self.apply_loaded_settings(settings, cx);
            notify = true;
        }
        if aux.send_history.is_some() || aux.send_waiting_count.is_some() {
            self.apply_send_history(aux.send_history, aux.send_waiting_count, cx);
            notify = true;
        }
        if let Some(selected) = aux.selected
            && self.selected == Some(selected.id)
        {
            self.apply_selected_meta(selected);
            notify = true;
        }
        if notify {
            cx.notify();
        }
    }

    fn apply_profiles(&mut self, profiles: Vec<CompressionProfile>) {
        self.profiles = profiles;
        if self.profile_index >= self.profiles.len() {
            self.profile_index = 0;
        }
    }

    fn apply_loaded_settings(&mut self, mut settings: AppSettings, cx: &mut Context<Self>) {
        let previous_language = self.settings.language.clone();
        let previous_auto_compress = self.settings.auto_compress;
        let previous_ai_tagging_enabled = self.settings.ai_tagging_enabled;
        let previous_clipboard_enabled = self.settings.clipboard_enabled;
        let previous_discord_enabled = self.settings.discord_enabled;
        let previous_thumbnail_spec = self.settings.thumbnail_spec();
        settings.language = vrc_photon_i18n::set_locale(&settings.language);
        let language_changed = settings.language != previous_language;
        self.settings = settings;
        if self.settings.thumbnail_spec() != previous_thumbnail_spec {
            self.thumbnails.borrow_mut().clear();
        }
        if self.settings.auto_compress != previous_auto_compress {
            self.integrations_auto_motion =
                crate::state::motion::ToggleMotion::settled(if self.settings.auto_compress {
                    0.0
                } else {
                    1.0
                });
        }
        if self.settings.ai_tagging_enabled != previous_ai_tagging_enabled {
            self.integrations_ai_motion =
                crate::state::motion::ToggleMotion::settled(if self.settings.ai_tagging_enabled {
                    0.0
                } else {
                    1.0
                });
        }
        if self.settings.clipboard_enabled != previous_clipboard_enabled {
            self.integrations_clipboard_motion =
                crate::state::motion::ToggleMotion::settled(if self.settings.clipboard_enabled {
                    0.0
                } else {
                    1.0
                });
        }
        if self.settings.discord_enabled != previous_discord_enabled {
            self.integrations_discord_motion =
                crate::state::motion::ToggleMotion::settled(if self.settings.discord_enabled {
                    0.0
                } else {
                    1.0
                });
        }
        if language_changed {
            self.library.borrow_mut().invalidate_labels();
            self.search_view.borrow_mut().invalidate_labels();
        }
        cx.set_reduce_motion(self.settings.ui_reduce_motion);
        let palette = resolve(&self.settings.ui_theme_main, &self.settings.ui_theme_accent);
        if palette != self.palette {
            self.palette = palette;
            apply_component_theme(self.palette, cx);
        }
    }

    /// Applies a resolved palette to the app, the component widgets, and the
    /// caches that bake colours into shaped text or morph state.
    pub(crate) fn apply_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        self.palette = palette;
        apply_component_theme(palette, cx);
        self.library.borrow_mut().labels = None;
        self.search_view.borrow_mut().labels = None;
        self.viewer_morph_view
            .update(cx, |view, cx| view.set_palette(palette, cx));
        cx.notify();
    }

    /// Rebuilds the ordered library snapshot and its page cache.
    ///
    /// File-backed databases collect the index and every side-panel read on
    /// the dedicated library reader; the UI thread only applies the finished
    /// snapshot. In-memory demo databases keep the synchronous path because
    /// they have no second handle and only hold a handful of rows.
    ///
    /// With `preserve_anchor`, the photo at the top-left of the first visible
    /// row stays at the top after the rebuild. A root change or an explicit
    /// reset passes `false` to return the grid to the first row.
    ///
    /// Returns `true` when a rebuild ran synchronously or was scheduled. On a
    /// query error the previous snapshot, pages, and scroll offset stay
    /// mounted and `library_error` exposes the retry action.
    pub(crate) fn reload_library(&mut self, preserve_anchor: bool, cx: &mut Context<Self>) -> bool {
        if self.library_reader.is_none() {
            return self.reload_library_sync(preserve_anchor, cx);
        }
        self.request_library_reload(preserve_anchor, cx);
        true
    }

    fn request_library_reload(&mut self, preserve_anchor: bool, cx: &mut Context<Self>) {
        self.pending_library_reload = Some(preserve_anchor);
        if self.library_reload.request() == LiveRefreshRequest::StartNow {
            self.spawn_library_reload(cx);
        }
    }

    fn spawn_library_reload(&mut self, cx: &mut Context<Self>) {
        let Some(preserve_anchor) = self.pending_library_reload.take() else {
            return;
        };
        let Some(reader) = self.library_reader.clone() else {
            self.reload_library_sync(preserve_anchor, cx);
            return;
        };
        let anchor = preserve_anchor
            .then(|| self.capture_top_photo(false))
            .flatten();
        let stats_revision = self.stats_revision;
        let selected_id = self.selected;
        self.library_loading = true;
        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move {
                    reader
                        .query(|db| Ok(collect_library_snapshot(db, stats_revision, selected_id)))
                        .ok()
                })
                .await;
            this.update(cx, |this, cx| {
                let follow_up = this.library_reload.finish();
                match snapshot {
                    Some(snapshot) => this.apply_library_snapshot_from_background(
                        snapshot,
                        preserve_anchor,
                        anchor,
                        cx,
                    ),
                    None => {
                        this.library_loading = false;
                        this.note_library_error("photos.index_error", cx);
                        this.refresh_live_async(cx);
                    }
                }
                if follow_up {
                    this.spawn_library_reload(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn reload_library_sync(&mut self, preserve_anchor: bool, cx: &mut Context<Self>) -> bool {
        let anchor = preserve_anchor
            .then(|| self.capture_top_photo(false))
            .flatten();

        let index_start = Instant::now();
        let result = self.db.photo_index_snapshot(PhotoSort::Newest);
        log_timing("photo_index", index_start);
        let applied = apply_library_index(&mut self.library.borrow_mut(), result);
        if let Some(error) = applied.error {
            self.note_library_error(error, cx);
            // Leave the mounted view and side panels untouched; the next
            // successful reload repopulates them.
            return false;
        }

        self.note_library_truncation(applied.truncated, cx);
        self.library_error = None;
        self.library_dirty = false;
        let anchor_present =
            anchor.is_some_and(|id| self.library.borrow().index.row_of_id(id).is_some());
        if let Some(id) = anchor.filter(|_| anchor_present) {
            self.scroll_to_photo(false, id, ScrollStrategy::Top);
        } else {
            self.scroll_grid_to_top(false);
        }

        // An active result set stays mounted until the user clears it or runs
        // a new search; the inactive mirror only needs the fresh snapshot.
        if !self.search_active {
            let index = self.library.borrow().index.clone();
            self.search_view.borrow_mut().replace_index(index);
        }
        self.reload_aux(cx);
        true
    }

    /// Full rebuild used at startup.
    pub(crate) fn reload_all(&mut self, reset_scroll: bool, cx: &mut Context<Self>) {
        self.reload_library(!reset_scroll, cx);
    }

    fn mark_library_dirty(&mut self) {
        self.library_dirty = true;
    }

    /// Changes screens without rebuilding the library unless Photos was marked
    /// dirty while another screen was mounted.
    pub(crate) fn switch_screen(&mut self, screen: Screen, cx: &mut Context<Self>) {
        if self.screen != screen {
            self.close_suggestion(cx);
            self.nav_previous = self.screen;
            self.nav_epoch = self.nav_epoch.wrapping_add(1);
            self.screen_direction = screen_direction(self.screen, screen);
            self.screen_motion = crate::state::motion::ToggleMotion::settled(0.0);
            self.screen_motion.animate_to(1.0, MOTION_BASE);
        }
        // The two photo grids have separate snapshots, so a selection made in
        // one tab cannot stay meaningful in the other. Keep it when visiting
        // Compression/Settings so the manual-compression action can use it.
        if self.screen != screen
            && matches!(self.screen, Screen::Photos | Screen::Search)
            && matches!(screen, Screen::Photos | Screen::Search)
        {
            self.clear_selection(cx);
        }
        self.screen = screen;
        if screen == Screen::Settings {
            self.refresh_thumbnail_cache_usage(cx);
        }
        if screen == Screen::Search && self.search_refresh_dirty {
            // Tag edits made while another screen was mounted are applied with
            // one query when Search becomes visible again.
            self.search_refresh_dirty = false;
            self.refresh_search_keeping_anchor(cx);
        } else if screen == Screen::Photos && self.library_dirty {
            let revision = self.db.library_revision().ok();
            self.apply_library_snapshot(revision, cx);
        } else if screen == Screen::Search && !self.search_active {
            // The no-query mirror only needs a refresh when the library
            // snapshot changed while another screen was mounted. A dirty
            // library is rebuilt first so the mirror also sees new photos.
            if self.library_dirty {
                let revision = self.db.library_revision().ok();
                self.apply_library_snapshot(revision, cx);
            } else {
                let mut search = self.search_view.borrow_mut();
                let library = self.library.borrow();
                sync_search_mirror(&mut search, &library);
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.overlay_open() {
            return;
        }
        self.sidebar_collapsed = !self.sidebar_collapsed;
        // The width animation recreates the nav subtree, so its animations must
        // render at the current tab instead of replaying the last tab change.
        self.nav_previous = self.screen;
        let target = if self.sidebar_collapsed { 1.0 } else { 0.0 };
        if cx.reduce_motion() {
            self.sidebar_motion = crate::state::motion::ToggleMotion::settled(target);
        } else {
            self.sidebar_motion.animate_to(target, MOTION_SIDEBAR);
        }
        self.hovered_nav = None;
        self.settings.ui_sidebar_collapsed = self.sidebar_collapsed;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save sidebar state");
        }
        cx.notify();
    }

    pub(crate) fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        if self.overlay_open() {
            return;
        }
        self.inspector_collapsed = !self.inspector_collapsed;
        let target = if self.inspector_collapsed { 1.0 } else { 0.0 };
        if cx.reduce_motion() {
            self.inspector_motion = crate::state::motion::ToggleMotion::settled(target);
        } else {
            self.inspector_motion.animate_to(target, MOTION_SLOW);
        }
        self.settings.ui_inspector_collapsed = self.inspector_collapsed;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save inspector state");
        }
        cx.notify();
    }

    pub(crate) fn toggle_ribbon_collapsed(&mut self, cx: &mut Context<Self>) {
        self.set_ribbon_collapsed(!self.ribbon_collapsed, cx);
    }

    fn set_ribbon_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.ribbon_collapsed = collapsed;
        let target = if collapsed { 1.0 } else { 0.0 };
        if cx.reduce_motion() {
            self.ribbon_motion = crate::state::motion::ToggleMotion::settled(target);
        } else {
            self.ribbon_motion.animate_to(target, MOTION_BASE);
        }
        self.settings.ui_ribbon_collapsed = collapsed;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save ribbon state");
        }
        cx.notify();
    }

    pub(crate) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_open() {
            return;
        }
        self.switch_screen(Screen::Search, cx);
        let focus_handle = self.world_input.read(cx).focus_handle(cx);
        window.focus(&focus_handle, cx);
        // The search screen may not be mounted yet when the action fires, so
        // retry after the deferred update has attached the input to the tree.
        window.defer(cx, move |window, cx| {
            window.focus(&focus_handle, cx);
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IpcBatchNotify, ScanRefresh, drain_ipc_batch, import_scan_enqueued_count,
        is_stale_revision, scan_refresh_action, settings_after_reload,
    };
    use crate::app::Screen;
    use crate::ipc_bridge::Incoming;
    use vrc_photon_core::settings::AppSettings;

    #[test]
    fn settings_reload_keeps_the_live_values_when_the_row_is_malformed() {
        let current = AppSettings {
            language: "en".to_string(),
            auto_compress: false,
            ..Default::default()
        };
        let error = serde_json::from_str::<AppSettings>("{").unwrap_err().into();

        let kept = settings_after_reload(&current, Err(error));

        assert_eq!(kept.language, "en");
        assert!(!kept.auto_compress);
    }

    #[test]
    fn settings_reload_uses_the_loaded_values_when_the_row_is_valid() {
        let current = AppSettings::default();
        let loaded = AppSettings {
            language: "en".to_string(),
            ..Default::default()
        };

        let applied = settings_after_reload(&current, Ok(loaded));

        assert_eq!(applied.language, "en");
    }

    #[test]
    fn import_auto_compress_count_only_accepts_the_requested_target() {
        assert_eq!(
            import_scan_enqueued_count(Some("/imported"), Some("/imported"), 2, 3),
            Some(5)
        );
        assert_eq!(
            import_scan_enqueued_count(Some("/imported"), Some("/other"), 2, 3),
            None
        );
        assert_eq!(
            import_scan_enqueued_count(Some("/imported"), None, 2, 3),
            None
        );
        assert_eq!(
            import_scan_enqueued_count(None, Some("/imported"), 2, 3),
            None
        );
        assert_eq!(
            import_scan_enqueued_count(Some("/imported"), Some("/imported"), usize::MAX, 1),
            Some(usize::MAX)
        );
    }

    #[test]
    fn only_older_live_revisions_are_stale() {
        assert!(is_stale_revision(Some(4), 5));
        assert!(!is_stale_revision(Some(5), 5));
        assert!(!is_stale_revision(Some(6), 5));
        assert!(!is_stale_revision(None, 5));
    }

    #[test]
    fn ipc_batch_drains_every_queued_message_in_order() {
        let (tx, rx) = async_channel::bounded(8);
        for id in 0..4 {
            tx.try_send(Incoming::Disconnected(id.to_string())).unwrap();
        }

        let first = rx.try_recv().unwrap();
        let batch = drain_ipc_batch(first, &rx);

        assert_eq!(batch.len(), 4);
        for (id, incoming) in batch.iter().enumerate() {
            assert_eq!(incoming, &Incoming::Disconnected(id.to_string()));
        }
        assert!(rx.is_empty());
    }

    #[test]
    fn an_ipc_batch_with_several_visible_events_repaints_once() {
        let (tx, rx) = async_channel::bounded(8);
        for id in 0..4 {
            tx.try_send(Incoming::Disconnected(id.to_string())).unwrap();
        }
        let first = rx.try_recv().unwrap();
        let batch = drain_ipc_batch(first, &rx);

        let mut notify = IpcBatchNotify::default();
        for _ in &batch {
            notify.record(true);
        }

        assert!(notify.needed());
        // The batch caller emits one `cx.notify()` for the whole batch, not
        // one per message.
        assert_eq!(usize::from(notify.needed()), 1);
    }

    #[test]
    fn an_ipc_batch_with_no_visible_change_skips_the_repaint() {
        let mut notify = IpcBatchNotify::default();
        notify.record(false);
        notify.record(false);

        assert!(!notify.needed());
    }

    #[test]
    fn scan_finished_refresh_is_screen_aware_and_skips_no_ops() {
        assert_eq!(
            scan_refresh_action(Screen::Photos, false, Some(5), 5, false),
            ScanRefresh::None
        );
        assert_eq!(
            scan_refresh_action(Screen::Photos, false, Some(6), 5, false),
            ScanRefresh::Snapshot
        );
        assert_eq!(
            scan_refresh_action(Screen::Photos, false, Some(5), 5, true),
            ScanRefresh::Snapshot
        );
        assert_eq!(
            scan_refresh_action(Screen::Search, true, Some(5), 5, false),
            ScanRefresh::Search
        );
        assert_eq!(
            scan_refresh_action(Screen::Search, false, Some(5), 5, false),
            ScanRefresh::Dirty
        );
        assert_eq!(
            scan_refresh_action(Screen::Settings, false, Some(6), 5, false),
            ScanRefresh::Dirty
        );
    }
}
