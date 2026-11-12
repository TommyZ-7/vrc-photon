use super::compression::*;
use super::deletion::*;
use super::hash::*;
use super::logs::*;
use super::status::*;
use super::thumbnails::*;
use super::vrchat_process::VrchatProcessProbe;
use super::*;

#[derive(Debug, Clone, Default)]
pub struct DaemonStatus {
    pub paused: bool,
    pub watching: bool,
    pub world_name: Option<String>,
    pub players: usize,
    pub last_log: Option<String>,
    pub photo_count: i64,
    pub pending_jobs: i64,
    pub pending_sends: i64,
    /// `waiting` + `pending` + `processing` AI tagging jobs.
    pub pending_tagging_jobs: i64,
    /// Whether an install, ready model, and saved probe are all present.
    pub tagging_ready: bool,
    pub scanning: bool,
    pub scan_phase: Option<vrc_photon_ipc::ScanPhase>,
    pub scan_processed: usize,
    pub scan_total: Option<usize>,
    pub scan_indexed: usize,
    pub scan_root: usize,
    pub scan_roots: usize,
    /// Number of snapshots taken so far. Each snapshot stamps and bumps
    /// this under the status lock, so a snapshot's version orders it
    /// against every other snapshot of this status.
    pub version: u64,
    /// Supervised worker/main-loop restarts since startup. Kept out of
    /// [`vrc_photon_ipc::StatusSnapshot`] so the wire shape stays stable;
    /// the daemon exposes it through [`Daemon::status`] for diagnostics.
    pub worker_restarts: u64,
    /// The last worker panic, spawn failure, or main-loop panic, for
    /// diagnostics. Not part of the IPC status snapshot.
    pub last_worker_error: Option<String>,
}

/// A user-facing daemon error summary that can be rendered in any locale.
#[derive(Debug, Clone)]
pub(crate) struct LocalizedError {
    key: &'static str,
    args: Vec<(&'static str, String)>,
}

impl LocalizedError {
    pub(crate) fn new(key: &'static str) -> Self {
        Self {
            key,
            args: Vec::new(),
        }
    }

    pub(crate) fn arg(mut self, name: &'static str, value: impl ToString) -> Self {
        self.args.push((name, value.to_string()));
        self
    }

    pub(crate) fn message(&self) -> String {
        self.message_in(&vrc_photon_i18n::current_locale())
    }

    pub(crate) fn message_in(&self, locale: &str) -> String {
        vrc_photon_i18n::tr_args_in(locale, self.key, &self.args)
    }

    pub(crate) fn args_json(&self) -> String {
        let args: std::collections::BTreeMap<&str, &str> = self
            .args
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
    }

    pub(crate) fn signature(&self, detail: Option<&str>) -> String {
        error_signature(&format!("{}|{}", self.key, self.args_json()), detail)
    }
}

pub struct Daemon {
    pub(crate) db: Arc<Mutex<Db>>,
    /// Read-only connection for IPC reads; falls back to `db` when opening one
    /// is not possible.
    pub(crate) read_db: Arc<Mutex<Db>>,
    /// Shared transient-memory budget for the compression workers.
    pub(crate) compression_budget: Arc<MemoryBudget>,
    pub(crate) settings: AppSettings,
    pub(crate) status: Arc<Mutex<DaemonStatus>>,
    pub(crate) running: Arc<AtomicBool>,
    /// Fast pause gate read by workers at their dequeue points. Mirrors
    /// `DaemonStatus::paused`, which remains the status-snapshot source.
    pub(crate) paused: Arc<AtomicBool>,
    pub(crate) wake: Arc<WakeSignal>,
    /// Wakes supervised workers out of a restart backoff during shutdown.
    pub(crate) supervisor_wake: Arc<WakeSignal>,
    pub(crate) jobs: Arc<JobSignal>,
    pub(crate) sends: Arc<JobSignal>,
    /// Wakes the photo-hash backfill worker when a scan indexed new rows.
    pub(crate) hash_wake: Arc<JobSignal>,
    /// Wakes the world name worker when lookups are queued.
    pub(crate) world_names: Arc<JobSignal>,
    /// Live copy of `settings.world_name_resolution_enabled` for the worker.
    pub(crate) world_name_resolution_enabled: Arc<AtomicBool>,
    pub(crate) events: EventBus,
    pub(crate) ipc_endpoint: Option<Endpoint>,
    pub(crate) ipc: Option<IpcServer>,
    pub(crate) watchers: Vec<RecommendedWatcher>,
    pub(crate) watcher_rx: Option<Receiver<notify::Result<notify::Event>>>,
    /// Roots the current watchers were set up for, to skip pointless
    /// rebuilds that would drop buffered events.
    pub(crate) watcher_roots: Vec<PathBuf>,
    /// The watcher-start failure currently kept visible, if any. It is
    /// recorded under the `Scan` operation but must survive clean scans:
    /// scanning works while watching does not.
    pub(crate) watcher_error: Option<(LocalizedError, Option<String>)>,
    /// New photo ids scanned while no thumbnail worker exists (notably
    /// `--once`); the caller generates their thumbnails directly.
    pub(crate) deferred_thumbnail_ids: Vec<i64>,
    pub(crate) log_tail: Option<LogTail>,
    pub(crate) log_locator: Option<LogLocator>,
    pub(crate) vrchat_probe: VrchatProcessProbe,
    pub(crate) vrchat_missing_polls: u8,
    pub(crate) vrchat_session_id: Option<i64>,
    pub(crate) last_presence_check: Instant,
    pub(crate) workers: Vec<JoinHandle<()>>,
    pub(crate) dirty_paths: Vec<PathBuf>,
    /// Commands consumed from the database while paused. They run in order
    /// once the daemon resumes, so pausing never loses an explicit request.
    pub(crate) deferred_commands: Vec<DaemonCommand>,
    pub(crate) last_log_poll: std::time::Instant,
    pub(crate) tray: Option<Tray>,
    pub(crate) tray_enabled: bool,
    pub(crate) thumbnail_tx: Option<mpsc::Sender<ThumbnailRequest>>,
    pub(crate) thumbnail_handle: Option<JoinHandle<()>>,
    pub(crate) hash_handle: Option<JoinHandle<()>>,
    pub(crate) send_handle: Option<JoinHandle<()>>,
    pub(crate) world_name_handle: Option<JoinHandle<()>>,
    pub(crate) owns_deletion_executor: bool,
    pub(crate) deletion_handle: Option<JoinHandle<()>>,
    pub(crate) last_autostart_check: std::time::Instant,
    pub(crate) last_settings_check: std::time::Instant,
    pub(crate) last_data_version: Option<i64>,
    pub(crate) last_session_status: Instant,
    pub(crate) last_counts_status: Instant,
    pub(crate) last_published_status: Option<StatusSnapshot>,
    pub(crate) last_heartbeat: std::time::Instant,
    pub(crate) last_asset_check: Instant,
    pub(crate) thumbnail_dir: PathBuf,
    pub(crate) thumbnail_cache_limit: Arc<AtomicU64>,
    /// Packed quality and long edge for the thumbnail worker.
    pub(crate) thumbnail_spec: Arc<AtomicU32>,
    /// Bumped when the thumbnail cache is cleared so the worker resets its
    /// byte accounting and rescans on the next pass.
    pub(crate) thumbnail_cache_epoch: Arc<AtomicU64>,
    /// Bytes of thumbnails the UI generated itself, as reported over IPC.
    pub(crate) thumbnail_external_bytes: Arc<AtomicU64>,
    pub(crate) last_errors: HashMap<DaemonOperation, String>,
    /// AI plugin installs, model files, probe cache, and the lazy sidecar.
    pub(crate) plugin: PluginService,
    /// Wakes the AI tagging worker when jobs arrive or settings change.
    pub(crate) tagging: Arc<JobSignal>,
    /// In-flight tagging cancel tokens shared with IPC.
    pub(crate) tagging_control: Arc<TaggingControl>,
    pub(crate) tagging_handle: Option<JoinHandle<()>>,
    /// Root for AI plugin installs, model files, snapshots, and logs.
    pub(crate) ai_data_dir: PathBuf,
    #[cfg(test)]
    pub(crate) ai_plugin_keys: Vec<String>,
    #[cfg(test)]
    pub(crate) ai_http: Arc<dyn ModelHttpClient>,
}

/// Re-links photos whose XMP has already been read outside the database lock.
///
/// `read_xmp` must be called while no database lock is held; the daemon passes
/// a closure that reads the PNG sidecar. The database lock is taken again for
/// each short link/retry write.
impl Daemon {
    /// Opens the daemon as the deletion executor. The caller must own the
    /// single-instance lock so no other process can still be executing a
    /// `running` deletion.
    pub fn open(
        db_path: impl AsRef<Path>,
        photo_dir: Option<PathBuf>,
        log_dir: Option<PathBuf>,
    ) -> Result<Self> {
        Self::open_inner(db_path, photo_dir, log_dir, true)
    }

    /// Opens the daemon without deletion-executor ownership: `running`
    /// deletions are not reset and [`Self::process_deletions`] is a no-op.
    /// Maintenance commands use this when they do not own the single-instance
    /// lock, because the live daemon may still be executing those requests.
    pub fn open_preserving_running_deletions(
        db_path: impl AsRef<Path>,
        photo_dir: Option<PathBuf>,
        log_dir: Option<PathBuf>,
    ) -> Result<Self> {
        Self::open_inner(db_path, photo_dir, log_dir, false)
    }

    pub(crate) fn open_inner(
        db_path: impl AsRef<Path>,
        photo_dir: Option<PathBuf>,
        log_dir: Option<PathBuf>,
        owns_deletion_executor: bool,
    ) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        let db = Db::open(&db_path).context("cannot open library database")?;
        if owns_deletion_executor {
            db.reset_running_deletions()
                .context("cannot recover interrupted deletions")?;
            let resumed = db
                .resume_interrupted_tagging_jobs()
                .context("cannot recover interrupted tagging jobs")?;
            if resumed > 0 {
                tracing::info!(jobs = resumed, "resumed interrupted tagging jobs");
            }
            let resumed_compression = db
                .resume_interrupted_compression_jobs()
                .context("cannot recover interrupted compression jobs")?;
            if resumed_compression > 0 {
                tracing::info!(
                    jobs = resumed_compression,
                    "resumed interrupted compression jobs"
                );
            }
        } else {
            tracing::debug!(
                "preserving running deletions and tagging jobs because this process does not own the executor lock"
            );
        }
        let mut settings = AppSettings::load(&db).context("cannot load settings")?;
        settings.language = vrc_photon_i18n::set_locale(&settings.language);
        if let Some(path) = photo_dir {
            settings.photo_roots = vec![path.to_string_lossy().to_string()];
        }
        if settings.photo_roots.is_empty() {
            let detected = detect_vrchat_paths();
            settings.photo_roots = detected
                .photo_roots
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect();
            if settings.log_dir.is_none() {
                settings.log_dir = detected
                    .log_dir
                    .map(|path| path.to_string_lossy().to_string());
            }
        }
        if let Some(path) = log_dir {
            settings.log_dir = Some(path.to_string_lossy().to_string());
        }
        settings.save(&db).context("cannot save settings")?;
        let tray_enabled = settings.tray;
        let world_name_resolution_enabled =
            Arc::new(AtomicBool::new(settings.world_name_resolution_enabled));
        let thumbnail_dir = vrc_photon_core::thumbnail::thumbnails_dir()
            .unwrap_or_else(|| PathBuf::from("thumbnails"));
        let thumbnail_cache_limit = Arc::new(AtomicU64::new(thumbnail_cache_limit_bytes(
            settings.thumbnail_cache_mb,
        )));
        let thumbnail_spec = Arc::new(AtomicU32::new(pack_thumbnail_spec(
            settings.thumbnail_spec(),
        )));
        let errors = db.daemon_errors().context("cannot load daemon errors")?;
        let watcher_error = errors
            .iter()
            .find(|error| {
                error.operation == DaemonOperation::Scan
                    && error.message_key.as_deref() == Some("daemon.watcher_failed")
            })
            .map(|error| {
                (
                    LocalizedError::new("daemon.watcher_failed"),
                    error.detail.clone(),
                )
            });
        let last_errors: std::collections::HashMap<DaemonOperation, String> = errors
            .into_iter()
            .map(|error| {
                let signature_source = match error.message_key.as_deref() {
                    Some(key) => {
                        format!("{}|{}", key, error.message_args.as_deref().unwrap_or("{}"))
                    }
                    None => error.message.clone(),
                };
                (
                    error.operation,
                    error_signature(&signature_source, error.detail.as_deref()),
                )
            })
            .collect();
        let ipc_endpoint =
            vrc_photon_core::data_dir().and_then(|dir| match Endpoint::for_data_dir(&dir) {
                Ok(endpoint) => Some(endpoint),
                Err(error) => {
                    tracing::warn!(%error, "cannot resolve IPC endpoint");
                    None
                }
            });

        let read_db = match Db::open_read_only(&db_path) {
            Ok(read_db) => Some(read_db),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "cannot open read-only library database; IPC reads will use the writer"
                );
                None
            }
        };
        let db = Arc::new(Mutex::new(db));
        let read_db = read_db
            .map(|read_db| Arc::new(Mutex::new(read_db)))
            .unwrap_or_else(|| db.clone());
        let events = EventBus::default();
        let ai_data_dir = vrc_photon_core::data_dir().unwrap_or_else(|| PathBuf::from("."));
        let ai_plugin_keys = vrc_photon_ai_host::effective_plugin_keys();
        let ai_http: Arc<dyn ModelHttpClient> = Arc::new(UreqModelClient::new());
        let plugin = PluginService::new(
            db.clone(),
            ai_data_dir.clone(),
            events.clone(),
            ai_plugin_keys.clone(),
            ai_http.clone(),
        );
        if owns_deletion_executor && let Err(error) = plugin.recover_interrupted_model_state() {
            tracing::warn!(%error, "cannot recover interrupted AI model state");
        }
        let tagging = Arc::new(JobSignal::default());
        let tagging_control = Arc::new(TaggingControl::new(tagging.clone()));

        Ok(Self {
            db,
            read_db,
            compression_budget: Arc::new(MemoryBudget::new(COMPRESSION_MEMORY_BUDGET_BYTES)),
            settings,
            status: Arc::new(Mutex::new(DaemonStatus::default())),
            running: Arc::new(AtomicBool::new(true)),
            paused: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(WakeSignal::default()),
            supervisor_wake: Arc::new(WakeSignal::default()),
            jobs: Arc::new(JobSignal::default()),
            sends: Arc::new(JobSignal::default()),
            hash_wake: Arc::new(JobSignal::default()),
            world_names: Arc::new(JobSignal::default()),
            world_name_resolution_enabled,
            events,
            ipc_endpoint,
            ipc: None,
            watchers: Vec::new(),
            watcher_rx: None,
            watcher_roots: Vec::new(),
            watcher_error,
            deferred_thumbnail_ids: Vec::new(),
            log_tail: None,
            log_locator: None,
            vrchat_probe: VrchatProcessProbe::new(),
            vrchat_missing_polls: 0,
            vrchat_session_id: None,
            last_presence_check: Instant::now() - PRESENCE_INTERVAL,
            workers: Vec::new(),
            dirty_paths: Vec::new(),
            deferred_commands: Vec::new(),
            last_log_poll: std::time::Instant::now() - LOG_INTERVAL,
            tray: None,
            tray_enabled,
            thumbnail_tx: None,
            thumbnail_handle: None,
            hash_handle: None,
            send_handle: None,
            world_name_handle: None,
            owns_deletion_executor,
            deletion_handle: None,
            last_autostart_check: std::time::Instant::now(),
            last_settings_check: Instant::now() - Duration::from_secs(2),
            last_data_version: None,
            last_session_status: Instant::now() - SESSION_STATUS_INTERVAL,
            last_counts_status: Instant::now() - COUNTS_STATUS_INTERVAL,
            last_published_status: None,
            last_heartbeat: std::time::Instant::now() - Duration::from_secs(10),
            last_asset_check: Instant::now() - Duration::from_secs(30),
            thumbnail_dir,
            thumbnail_cache_limit,
            thumbnail_spec,
            thumbnail_cache_epoch: Arc::new(AtomicU64::new(0)),
            thumbnail_external_bytes: Arc::new(AtomicU64::new(0)),
            last_errors,
            plugin,
            tagging,
            tagging_control,
            tagging_handle: None,
            ai_data_dir,
            #[cfg(test)]
            ai_plugin_keys,
            #[cfg(test)]
            ai_http,
        })
    }

    pub fn status(&self) -> Arc<Mutex<DaemonStatus>> {
        self.status.clone()
    }

    pub fn set_thumbnail_dir(&mut self, path: PathBuf) {
        self.thumbnail_dir = path;
    }

    pub fn set_tray_enabled(&mut self, enabled: bool) {
        self.tray_enabled = enabled;
        if !enabled {
            self.tray = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn set_ai_data_dir(&mut self, path: PathBuf) {
        self.ai_data_dir = path;
        self.rebuild_plugin_service();
    }

    #[cfg(test)]
    pub(crate) fn set_ai_plugin_keys(&mut self, keys: Vec<String>) {
        self.ai_plugin_keys = keys;
        self.rebuild_plugin_service();
    }

    #[cfg(test)]
    pub(crate) fn set_ai_http_client(&mut self, client: Arc<dyn ModelHttpClient>) {
        self.ai_http = client;
        self.rebuild_plugin_service();
    }

    #[cfg(test)]
    pub(crate) fn rebuild_plugin_service(&mut self) {
        self.plugin = PluginService::new(
            self.db.clone(),
            self.ai_data_dir.clone(),
            self.events.clone(),
            self.ai_plugin_keys.clone(),
            self.ai_http.clone(),
        );
    }

    #[cfg(test)]
    pub(crate) fn plugin_service(&self) -> &PluginService {
        &self.plugin
    }

    /// Overrides the resolved IPC endpoint. Tests disable it with `None`.
    pub fn set_ipc_endpoint(&mut self, endpoint: Option<Endpoint>) {
        self.ipc_endpoint = endpoint;
    }

    /// Pauses or resumes heavy/queued work: photo scanning and indexing,
    /// replaced-asset verification, and the compression, tagging, world-name,
    /// deletion, and hash workers. Observation keeps running while paused:
    /// heartbeat writes, status snapshots, tray updates, IPC, watcher event
    /// collection, log ingestion, and periodic settings checks. Scan progress
    /// and recorded daemon errors are left untouched. Thumbnail generation and
    /// Discord delivery stay active: the former serves UI browsing and the
    /// latter is an outbound queue independent of library processing.
    pub fn set_paused(&self, paused: bool) {
        let mut status = lock_status(&self.status);
        let changed = if status.paused == paused {
            false
        } else {
            status.paused = paused;
            true
        };
        drop(status);
        self.paused.store(paused, Ordering::Relaxed);
        if changed {
            self.notify_workers();
            self.wake.notify();
            self.publish_status();
        }
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Wakes every queued-work worker so a pause or resume takes effect at the
    /// next dequeue instead of after the worker's poll timeout.
    pub(crate) fn notify_workers(&self) {
        self.jobs.notify();
        self.sends.notify();
        self.world_names.notify();
        self.tagging.notify();
        self.hash_wake.notify();
        self.notify_thumbnail_worker();
    }

    fn notify_thumbnail_worker(&self) {
        if let Some(tx) = &self.thumbnail_tx {
            let _ = tx.send(ThumbnailRequest::Refresh);
        }
    }

    /// Records a supervised restart without changing the IPC status shape.
    pub(crate) fn record_supervised_restart(&self, worker: &str, message: &str) {
        supervisor::record_restart(&self.status, worker, message);
    }

    pub(crate) fn publish_status(&self) {
        let mut status = lock_status(&self.status);
        let snapshot = status_snapshot(&mut status);
        self.events
            .publish(Event::StatusChanged { status: snapshot });
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// Persists a technical daemon failure without a catalog key. Kept for
    /// errors whose detail text has no user-facing translation.
    #[allow(dead_code)]
    pub(crate) fn report_daemon_error(
        &mut self,
        operation: DaemonOperation,
        message: &str,
        detail: Option<&str>,
    ) -> Result<()> {
        self.report_daemon_error_fields(operation, message, None, None, detail, message)
    }

    /// Records a user-facing failure with a catalog key and JSON args.
    pub(crate) fn report_localized_error(
        &mut self,
        operation: DaemonOperation,
        error: LocalizedError,
        detail: Option<&str>,
    ) -> Result<()> {
        let args_json = error.args_json();
        let signature = format!("{}|{}", error.key, args_json);
        self.report_daemon_error_fields(
            operation,
            &error.message(),
            Some(error.key),
            Some(&args_json),
            detail,
            &signature,
        )
    }

    pub(crate) fn report_daemon_error_fields(
        &mut self,
        operation: DaemonOperation,
        message: &str,
        message_key: Option<&str>,
        message_args: Option<&str>,
        detail: Option<&str>,
        signature_source: &str,
    ) -> Result<()> {
        let signature = error_signature(signature_source, detail);
        if self.last_errors.get(&operation) == Some(&signature) {
            return Ok(());
        }
        {
            let db = lock_db(&self.db);
            db.record_daemon_error_i18n(operation, message, message_key, message_args, detail)?;
        }
        self.last_errors.insert(operation, signature);
        self.events.publish(Event::DaemonErrorChanged {
            operation: ipc_operation(operation),
        });
        Ok(())
    }

    /// Removes a previously reported failure once the operation recovers.
    pub(crate) fn clear_daemon_error(&mut self, operation: DaemonOperation) -> Result<()> {
        if !self.last_errors.contains_key(&operation) {
            return Ok(());
        }
        {
            let db = lock_db(&self.db);
            db.clear_daemon_error(operation)?;
        }
        self.last_errors.remove(&operation);
        self.events.publish(Event::DaemonErrorChanged {
            operation: ipc_operation(operation),
        });
        Ok(())
    }

    /// Consumes queued UI requests once. See
    /// [`Self::poll_commands_with_initial`].
    pub(crate) fn poll_commands(&mut self) -> Result<()> {
        self.poll_commands_with_initial(None)
    }

    /// Consumes queued UI requests once: reloads settings, rebuilds the photo
    /// watchers for the new roots, runs each command in request order, then
    /// refreshes the status view. Commands are consumed even when they fail so
    /// a broken request cannot loop forever; the failure is recorded under its
    /// operation instead.
    ///
    /// When `initial_scan` is `Some(true)`, targeted `ScanRoot` commands run
    /// before full rescans. An import into an empty library must be indexed as
    /// new work before the startup scan can classify it as backlog.
    pub(crate) fn poll_commands_with_initial(&mut self, initial_scan: Option<bool>) -> Result<()> {
        let mut commands = std::mem::take(&mut self.deferred_commands);
        {
            let db = lock_db(&self.db);
            if commands.is_empty() && !db.has_daemon_commands()? {
                return Ok(());
            }
            commands.extend(db.take_daemon_commands()?);
        }
        if commands.is_empty() {
            return Ok(());
        }
        self.reload_settings()?;
        self.restart_watchers();
        if self.is_paused() {
            // Settings and watchers above still apply while paused, but the
            // scan/relink work these commands represent waits for resume.
            self.deferred_commands = commands;
            return Ok(());
        }
        if initial_scan == Some(true) {
            commands.sort_by_key(|command| !matches!(command, DaemonCommand::ScanRoot { .. }));
        }
        for command in commands {
            let kind = command.as_str();
            if let Err(error) = self.run_command_with_initial(&command, initial_scan) {
                tracing::error!(command = kind, %error, "daemon command failed");
                let (operation, message) = match &command {
                    DaemonCommand::Rescan => (
                        DaemonOperation::Scan,
                        LocalizedError::new("daemon.rescan_failed"),
                    ),
                    DaemonCommand::Relog => (
                        DaemonOperation::Log,
                        LocalizedError::new("daemon.log_reimport_failed"),
                    ),
                    DaemonCommand::Relink => (
                        DaemonOperation::Link,
                        LocalizedError::new("daemon.relink_failed"),
                    ),
                    DaemonCommand::ScanRoot { .. } => (
                        DaemonOperation::Scan,
                        LocalizedError::new("daemon.rescan_failed"),
                    ),
                };
                if let Err(report_error) =
                    self.report_localized_error(operation, message, Some(&error.to_string()))
                {
                    tracing::error!(%report_error, "cannot record daemon command error");
                }
            }
        }
        self.refresh_status()
    }

    /// Replaces the in-memory settings with the current database values so a
    /// corrected path or switch takes effect without restarting the daemon.
    pub(crate) fn reload_settings(&mut self) -> Result<()> {
        let mut settings = {
            let db = lock_db(&self.db);
            AppSettings::load(&db)?
        };
        settings.language = vrc_photon_i18n::normalize(&settings.language);
        let language_changed = settings.language != self.settings.language;
        if settings.autostart != self.settings.autostart {
            self.apply_autostart(settings.autostart);
        }
        let old_limit = self.thumbnail_cache_limit.load(Ordering::Relaxed);
        let old_spec = self.thumbnail_spec.load(Ordering::Relaxed);
        let new_limit = thumbnail_cache_limit_bytes(settings.thumbnail_cache_mb);
        let new_spec = pack_thumbnail_spec(settings.thumbnail_spec());
        self.thumbnail_cache_limit.store(new_limit, Ordering::Relaxed);
        self.thumbnail_spec.store(new_spec, Ordering::Relaxed);
        let log_dir_changed = self.settings.log_dir != settings.log_dir;
        let thumbnail_size_changed = self.settings.thumbnail_size != settings.thumbnail_size;
        if thumbnail_size_changed {
            if let Err(error) = thumbnail::clear_thumbnail_cache_in(&self.thumbnail_dir) {
                tracing::warn!(%error, "cannot clear thumbnails after a resolution change");
            }
            self.thumbnail_cache_epoch.fetch_add(1, Ordering::Relaxed);
        }
        if old_limit != new_limit || old_spec != new_spec || thumbnail_size_changed {
            self.notify_thumbnail_worker();
        }
        let world_name_enabled = settings.world_name_resolution_enabled;
        self.settings = settings;
        if log_dir_changed {
            self.log_locator = None;
        }
        let was_world_name_enabled = self
            .world_name_resolution_enabled
            .swap(world_name_enabled, Ordering::Relaxed);
        if world_name_enabled && !was_world_name_enabled {
            self.enqueue_world_name_backfill();
            self.world_names.notify();
        }
        if language_changed {
            vrc_photon_i18n::set_locale(&self.settings.language);
            if let Some(tray) = self.tray.as_mut() {
                tray.set_language();
            }
        }
        Ok(())
    }

    /// Stops the current watchers and sets up new ones for the reloaded roots.
    /// A failure leaves them stopped so the next command or daemon start can
    /// retry the setup.
    ///
    /// Buffered events are drained first, and when the existing roots are
    /// already watched the channel (and its buffer) is kept as is: a
    /// command batch must not lose photo events that arrived while it ran.
    pub(crate) fn rebuild_watchers(&mut self) -> Result<()> {
        self.poll_watchers();
        let wanted: Vec<PathBuf> = self
            .settings
            .photo_roots()
            .into_iter()
            .filter(|root| root.is_dir())
            .collect();
        if self.watcher_rx.is_some() && self.watcher_roots == wanted {
            return Ok(());
        }
        self.watchers.clear();
        self.watcher_rx = None;
        self.watcher_roots.clear();
        let result = self.setup_watchers();
        if result.is_err() {
            self.watchers.clear();
            self.watcher_roots.clear();
        }
        result
    }

    /// Rebuilds the watchers, recording a `Scan` failure instead of aborting
    /// the daemon when the operating system refuses a watch. The failure is
    /// tracked separately from scan failures so a clean scan cannot erase
    /// it; it clears when the watchers start successfully.
    pub(crate) fn restart_watchers(&mut self) {
        match self.rebuild_watchers() {
            Ok(()) => {
                if let Some((localized, detail)) = self.watcher_error.take() {
                    // Clear only when the Scan slot still shows the watcher
                    // failure; a scan failure recorded since supersedes it.
                    if self.last_errors.get(&DaemonOperation::Scan)
                        == Some(&localized.signature(detail.as_deref()))
                        && let Err(error) = self.clear_daemon_error(DaemonOperation::Scan)
                    {
                        tracing::error!(%error, "cannot clear watcher error");
                    }
                }
            }
            Err(error) => {
                tracing::error!(%error, "cannot start photo watchers");
                let detail = error.to_string();
                let localized = LocalizedError::new("daemon.watcher_failed");
                self.watcher_error = Some((localized.clone(), Some(detail.clone())));
                if let Err(report_error) =
                    self.report_localized_error(DaemonOperation::Scan, localized, Some(&detail))
                {
                    tracing::error!(%report_error, "cannot record watcher error");
                }
            }
        }
    }

    /// Runs one queued command with an optional startup initial-scan flag.
    /// `ScanRoot` always indexes a targeted root and therefore never treats
    /// its photos as backlog.
    pub(crate) fn run_command_with_initial(
        &mut self,
        command: &DaemonCommand,
        initial_scan: Option<bool>,
    ) -> Result<()> {
        match command {
            DaemonCommand::Rescan => {
                self.make_unlinked_photos_due()?;
                self.scan_photos_with_initial(initial_scan)?;
                self.link_unlinked()?;
            }
            DaemonCommand::Relog => {
                self.make_unlinked_photos_due()?;
                self.ingest_logs()?;
                self.link_unlinked()?;
            }
            DaemonCommand::Relink => self.link_unlinked()?,
            DaemonCommand::ScanRoot { path } => {
                self.scan_root_photos(PathBuf::from(path))?;
                self.link_unlinked()?;
            }
        }
        Ok(())
    }

    pub fn run_once(&mut self) -> Result<()> {
        self.repair_open_sessions()?;
        let initial_scan = {
            let db = lock_db(&self.db);
            db.count_photos()? == 0
        };
        self.poll_commands_with_initial(Some(initial_scan))?;
        self.write_heartbeat();
        let report = self.scan_photos_with_initial(Some(initial_scan))?;
        // Photos indexed by the queued commands above were scanned while
        // no thumbnail worker exists in `--once` mode, so their ids were
        // deferred; generate for the union of both batches.
        let mut thumbnail_ids = std::mem::take(&mut self.deferred_thumbnail_ids);
        for id in &report.new_ids {
            if !thumbnail_ids.contains(id) {
                thumbnail_ids.push(*id);
            }
        }
        for id in &thumbnail_ids {
            let path = {
                let db = lock_db(&self.db);
                match db.photo_by_id(*id) {
                    Ok(photo) => photo.map(|photo| photo.effective_path().to_string()),
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            photo_id = *id,
                            "cannot load photo for thumbnail"
                        );
                        None
                    }
                }
            };
            if let Some(path) = path {
                match thumbnail::ensure_thumbnail_in(
                    *id,
                    Path::new(&path),
                    &self.thumbnail_dir,
                    self.settings.thumbnail_spec(),
                ) {
                    Ok(Some(_)) => {}
                    Ok(None) => tracing::debug!(
                        photo_id = *id,
                        path = %path,
                        "thumbnail source is missing"
                    ),
                    Err(error) => tracing::warn!(
                        %error,
                        photo_id = *id,
                        path = %path,
                        "cannot generate thumbnail"
                    ),
                }
            }
        }
        let _ = prune_thumbnail_cache(
            &self.thumbnail_dir,
            self.thumbnail_cache_limit.load(Ordering::Relaxed),
        );
        // The scan above ran before this first log pass; clear the backoff it
        // recorded so the sessions just ingested can link those photos.
        self.make_unlinked_photos_due()?;
        self.ingest_logs()?;
        self.poll_vrchat_presence()?;
        self.link_unlinked()?;
        self.refresh_status()?;
        self.process_deletions()?;
        Ok(())
    }

    pub fn compress_pending(&mut self) -> Result<()> {
        loop {
            let jobs = {
                let db = lock_db(&self.db);
                db.next_pending_photo_jobs()?
            };
            if jobs.is_empty() {
                break;
            }
            process_photo_batch(
                &self.db,
                &jobs,
                Some(&self.thumbnail_dir),
                &self.compression_budget,
            );
        }
        self.process_deletions()?;
        Ok(())
    }

    /// Moves the AI backlog from `waiting` to `pending` and wakes the worker.
    /// The settings UI performs the same database update directly; this
    /// method exists for in-process callers and tests.
    pub fn release_ai_backlog(&mut self) -> Result<usize> {
        let released = {
            let db = lock_db(&self.db);
            db.release_waiting_tagging_jobs(AI_PLUGIN_ID)?
        };
        self.tagging.notify();
        Ok(released)
    }

    /// Executes every deletion that has no remaining job blocker. The
    /// background thread calls the same routine on a timer. A process that
    /// does not own the instance lock must leave deletion execution to the
    /// owner so an interrupted maintenance run cannot strand a `running` row.
    pub fn process_deletions(&mut self) -> Result<()> {
        if !self.owns_deletion_executor {
            return Ok(());
        }
        run_deletions_once(&self.db)?;
        Ok(())
    }

    /// Prepares the initial scan: load roots, run queued import scans before
    /// the first full scan, then scan the library. The empty-library decision
    /// is taken before commands run so imported photos are new work while
    /// pre-existing photos stay backlog.
    pub(crate) fn startup_scan(&mut self) -> Result<()> {
        self.ensure_roots()?;
        if self.stopping() {
            return Ok(());
        }
        let initial_scan = {
            let db = lock_db(&self.db);
            db.count_photos()? == 0
        };
        self.poll_commands_with_initial(Some(initial_scan))?;
        if self.stopping() {
            return Ok(());
        }
        self.scan_photos_with_initial(Some(initial_scan))?;
        Ok(())
    }

    pub fn run(&mut self) -> Result<()> {
        let _ = prune_thumbnail_cache(
            &self.thumbnail_dir,
            self.thumbnail_cache_limit.load(Ordering::Relaxed),
        );
        self.repair_open_sessions()?;
        self.start_ipc();
        if self.stopping() {
            self.shutdown();
            return Ok(());
        }
        self.record_supervisor();
        // Start workers before the initial scan so a long scan does not delay
        // already queued compression, tagging, or deletion work. The scan now
        // takes the database mutex only for short per-photo operations.
        self.start_workers();
        self.startup_scan()?;
        if self.stopping() {
            self.shutdown();
            return Ok(());
        }
        // The startup scan runs before logs are ingested, so its link pass can
        // record a retry before the sessions needed to link are available.
        // Clear that schedule once at startup; the periodic loop keeps the
        // backoff and only retries due photos.
        self.make_unlinked_photos_due()?;
        self.ingest_logs()?;
        self.poll_vrchat_presence()?;
        self.link_unlinked()?;
        if self.stopping() {
            self.shutdown();
            return Ok(());
        }
        self.write_heartbeat();
        self.restart_watchers();
        self.apply_autostart(self.settings.autostart);
        self.ensure_tray();

        tracing::info!(
            photos = self.settings.photo_roots.len(),
            log_dir = ?self.settings.log_dir,
            ai_data_dir = %self.ai_data_dir.display(),
            "daemon running"
        );

        let mut backoff = supervisor::RestartBackoff::new();
        while self.running.load(Ordering::Relaxed) {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.run_loop_iteration()
            })) {
                Ok(()) => backoff.reset(),
                Err(payload) => {
                    let message = supervisor::panic_message(payload.as_ref());
                    self.record_supervised_restart("daemon-loop", &message);
                    let delay = backoff.next_delay();
                    tracing::error!(
                        panic = %message,
                        restart_in_ms = delay.as_millis(),
                        "daemon main loop panicked; continuing after backoff"
                    );
                    supervisor::wait_while_running(&self.running, delay);
                }
            }
            if self.running.load(Ordering::Relaxed) {
                self.wake.wait(SCAN_INTERVAL);
            }
        }
        self.shutdown();
        Ok(())
    }

    /// One supervised iteration of the background loop. Pause gates only the
    /// heavy work below; heartbeat, status, tray, log ingestion, watcher event
    /// collection, settings refreshes, and IPC keep running so a paused daemon
    /// stays observable and resumes without losing queued events.
    pub(crate) fn run_loop_iteration(&mut self) {
        if let Err(error) = self.poll_commands() {
            tracing::error!(%error, "cannot handle daemon commands");
        }
        self.refresh_periodic_settings();
        self.poll_watchers();

        let paused = self.is_paused();
        if !paused
            && !self.dirty_paths.is_empty()
            && let Err(error) = self.index_dirty_paths()
        {
            tracing::error!(%error, "incremental photo scan failed");
            if let Err(report_error) = self.report_localized_error(
                DaemonOperation::Scan,
                LocalizedError::new("daemon.diff_scan_failed"),
                Some(&error.to_string()),
            ) {
                tracing::error!(%report_error, "cannot record scan error");
            }
        }
        if self.last_log_poll.elapsed() >= LOG_INTERVAL {
            if let Err(error) = self.ingest_logs() {
                tracing::error!(%error, "log ingestion failed");
            }
            self.last_log_poll = std::time::Instant::now();
        }
        if self.last_presence_check.elapsed() >= PRESENCE_INTERVAL {
            if let Err(error) = self.poll_vrchat_presence() {
                tracing::error!(%error, "VRChat process check failed");
            }
            self.last_presence_check = std::time::Instant::now();
        }
        if self.last_session_status.elapsed() >= SESSION_STATUS_INTERVAL {
            if let Err(error) = self.refresh_session_status() {
                tracing::warn!(%error, "cannot refresh daemon session status");
            }
            self.last_session_status = Instant::now();
            self.publish_status_if_changed();
        }
        if self.last_counts_status.elapsed() >= COUNTS_STATUS_INTERVAL {
            if let Err(error) = self.refresh_counts() {
                tracing::warn!(%error, "cannot refresh daemon counts");
            }
            self.last_counts_status = Instant::now();
            self.publish_status_if_changed();
        }
        if !paused && self.last_asset_check.elapsed() >= Duration::from_secs(30) {
            if let Err(error) = self.verify_replaced_assets() {
                tracing::warn!(%error, "cannot verify replaced photo assets");
            }
            self.last_asset_check = Instant::now();
        }
        if self.last_heartbeat.elapsed() >= Duration::from_secs(10) {
            self.write_heartbeat();
        }
        self.poll_tray();
    }

    pub(crate) fn stopping(&self) -> bool {
        !self.running.load(Ordering::Relaxed)
    }

    pub(crate) fn start_ipc(&mut self) {
        if self.ipc.is_some() {
            return;
        }
        let Some(endpoint) = self.ipc_endpoint.clone() else {
            return;
        };
        let control = IpcControl {
            running: self.running.clone(),
            status: self.status.clone(),
            paused: self.paused.clone(),
            wake: self.wake.clone(),
            jobs: self.jobs.clone(),
            sends: self.sends.clone(),
            hash: self.hash_wake.clone(),
            world_names: self.world_names.clone(),
            events: self.events.clone(),
            thumbnail_dir: self.thumbnail_dir.clone(),
            thumbnail_cache_epoch: self.thumbnail_cache_epoch.clone(),
            thumbnail_external_bytes: self.thumbnail_external_bytes.clone(),
            plugin: self.plugin.clone(),
            tagging: self.tagging.clone(),
            tagging_control: self.tagging_control.clone(),
            db: self.db.clone(),
            read_db: self.read_db.clone(),
        };
        match IpcServer::start(endpoint, control) {
            Ok(server) => self.ipc = Some(server),
            Err(error) => {
                tracing::warn!(%error, "cannot start IPC server; continuing without IPC")
            }
        }
    }

    pub(crate) fn ensure_roots(&mut self) -> Result<()> {
        if self.settings.photo_roots.is_empty() {
            let detected = detect_vrchat_paths();
            self.settings.photo_roots = detected
                .photo_roots
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect();
            if self.settings.log_dir.is_none() {
                self.settings.log_dir = detected
                    .log_dir
                    .map(|path| path.to_string_lossy().to_string());
            }
            if self.settings.photo_roots.is_empty() {
                tracing::warn!(
                    "no VRChat photo folder detected; set VRC_PHOTON_DIR or configure settings"
                );
            } else {
                let db = lock_db(&self.db);
                self.settings.save(&db)?;
            }
        }
        Ok(())
    }

    /// Runs the timed settings refreshes. Called on every main-loop
    /// iteration, including while paused, so a settings change the UI saved
    /// is applied promptly instead of waiting until the daemon resumes.
    pub(crate) fn refresh_periodic_settings(&mut self) {
        if self.last_settings_check.elapsed() >= SETTINGS_INTERVAL {
            let version = {
                let db = lock_db(&self.db);
                db.data_version().ok()
            };
            let changed = version.is_none() || version != self.last_data_version;
            self.last_data_version = version;
            self.last_settings_check = Instant::now();
            if changed {
                self.refresh_compression_settings();
            }
        }
        if self.last_autostart_check.elapsed() >= Duration::from_secs(10) {
            self.refresh_autostart_setting();
            self.last_autostart_check = std::time::Instant::now();
        }
    }

    pub(crate) fn refresh_autostart_setting(&mut self) {
        let enabled = {
            let db = lock_db(&self.db);
            match AppSettings::load(&db) {
                Ok(settings) => settings.autostart,
                Err(error) => {
                    tracing::warn!(%error, "cannot reload settings for autostart");
                    self.settings.autostart
                }
            }
        };
        if enabled != self.settings.autostart {
            self.settings.autostart = enabled;
            self.apply_autostart(enabled);
        }
    }

    /// Applies the compression switches and thumbnail cache limit without
    /// touching paths or restarting watchers. The UI saves these immediately;
    /// a running daemon must not keep enqueuing with the previous policy or
    /// prune with a stale cache limit. This also mirrors
    /// `world_name_resolution_enabled` into the worker flag and runs the
    /// backfill when the switch turns on.
    pub(crate) fn refresh_compression_settings(&mut self) {
        let loaded = {
            let db = lock_db(&self.db);
            match AppSettings::load(&db) {
                Ok(settings) => Some(settings),
                Err(error) => {
                    tracing::warn!(%error, "cannot reload compression settings");
                    None
                }
            }
        };
        let Some(settings) = loaded else {
            return;
        };
        self.settings.auto_compress = settings.auto_compress;
        self.settings.auto_compress_backlog = settings.auto_compress_backlog;
        self.settings.auto_compress_immediate = settings.auto_compress_immediate;
        self.settings.auto_compress_keep_original = settings.auto_compress_keep_original;
        self.settings.ai_tagging_enabled = settings.ai_tagging_enabled;
        self.settings.ai_tagging_auto = settings.ai_tagging_auto;
        self.settings.ai_tagging_backlog = settings.ai_tagging_backlog;
        self.settings.ai_tagging_idle_timeout_sec = settings.ai_tagging_idle_timeout_sec;
        let thumbnail_size_changed = self.settings.thumbnail_size != settings.thumbnail_size;
        self.settings.thumbnail_size = settings.thumbnail_size;
        if thumbnail_size_changed {
            let _ = thumbnail::clear_thumbnail_cache_in(&self.thumbnail_dir);
            self.thumbnail_cache_epoch.fetch_add(1, Ordering::Relaxed);
        }
        let old_limit = self.thumbnail_cache_limit.load(Ordering::Relaxed);
        let old_spec = self.thumbnail_spec.load(Ordering::Relaxed);
        let new_limit = thumbnail_cache_limit_bytes(settings.thumbnail_cache_mb);
        let new_spec = pack_thumbnail_spec(settings.thumbnail_spec());
        self.thumbnail_cache_limit.store(new_limit, Ordering::Relaxed);
        self.thumbnail_spec.store(new_spec, Ordering::Relaxed);
        if old_limit != new_limit || old_spec != new_spec || thumbnail_size_changed {
            self.notify_thumbnail_worker();
        }
        let world_name_enabled = settings.world_name_resolution_enabled;
        let was_enabled = self
            .world_name_resolution_enabled
            .swap(world_name_enabled, Ordering::Relaxed);
        self.settings.world_name_resolution_enabled = world_name_enabled;
        if world_name_enabled && !was_enabled {
            self.enqueue_world_name_backfill();
            self.world_names.notify();
        } else if !world_name_enabled && was_enabled {
            self.world_names.notify();
        }
    }

    /// Fills photo contexts from stored world names, then queues every world
    /// that still has no name. Lookup failures stay out of `daemon_errors`:
    /// the feature is best-effort and must not surface as a user-facing fault.
    pub(crate) fn enqueue_world_name_backfill(&self) {
        let db = lock_db(&self.db);
        if let Err(error) = db.backfill_photo_context_world_names() {
            tracing::warn!(%error, "cannot backfill photo context world names");
        }
        if let Err(error) = db.enqueue_missing_world_name_lookups() {
            tracing::warn!(%error, "cannot enqueue missing world name lookups");
        }
    }

    pub(crate) fn apply_autostart(&self, enabled: bool) {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let exe = exe.to_string_lossy();
        let mut builder = auto_launch::AutoLaunchBuilder::new();
        builder.set_app_name("VRC PhotoN").set_app_path(&exe);
        match builder.build() {
            Ok(auto) => {
                let result = if enabled {
                    auto.enable()
                } else {
                    auto.disable()
                };
                if let Err(error) = result {
                    tracing::warn!(%error, enabled, "cannot update autostart");
                }
            }
            Err(error) => tracing::warn!(%error, "cannot build autostart entry"),
        }
    }

    pub(crate) fn ensure_tray(&mut self) {
        if !self.tray_enabled || self.tray.is_some() {
            return;
        }
        match Tray::new() {
            Ok(tray) => self.tray = Some(tray),
            Err(error) => {
                tracing::warn!(%error, "tray unavailable; continuing headless");
                self.tray_enabled = false;
            }
        }
    }

    pub(crate) fn poll_tray(&mut self) {
        self.ensure_tray();
        let status = self
            .status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default();
        let Some(tray) = self.tray.as_mut() else {
            return;
        };
        match tray.poll(&status) {
            TrayAction::Noop => {}
            TrayAction::TogglePause(paused) => self.set_paused(paused),
            TrayAction::Quit => {
                self.running.store(false, Ordering::Relaxed);
                self.wake.notify();
            }
            TrayAction::OpenUi => tray::open_ui(),
        }
    }

    pub(crate) fn setup_watchers(&mut self) -> Result<()> {
        if self.watcher_rx.is_some() {
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        let mut watched = Vec::new();
        for root in self.settings.photo_roots() {
            if !root.is_dir() {
                continue;
            }
            let tx = tx.clone();
            let mut watcher = notify::recommended_watcher(move |event| {
                if let Err(error) = tx.send(event) {
                    tracing::debug!(%error, "photo watcher receiver closed; dropping event");
                }
            })
            .context("cannot create photo watcher")?;
            watcher
                .watch(&root, RecursiveMode::Recursive)
                .with_context(|| format!("cannot watch {}", root.display()))?;
            self.watchers.push(watcher);
            watched.push(root);
        }
        self.watcher_rx = Some(rx);
        self.watcher_roots = watched;
        Ok(())
    }

    pub(crate) fn poll_watchers(&mut self) {
        let Some(rx) = &self.watcher_rx else {
            return;
        };
        while let Ok(result) = rx.try_recv() {
            match result {
                Ok(event) => {
                    if matches!(
                        event.kind,
                        notify::EventKind::Create(_)
                            | notify::EventKind::Modify(_)
                            | notify::EventKind::Remove(_)
                    ) {
                        for path in event.paths {
                            if path
                                .components()
                                .any(|part| part.as_os_str() == "compressed")
                            {
                                continue;
                            }
                            if is_source_photo(&path) {
                                self.dirty_paths.push(path);
                            }
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "photo watcher error"),
            }
        }
    }

    pub(crate) fn spawn_supervised_worker<F>(&self, name: &str, task: F) -> Option<JoinHandle<()>>
    where
        F: FnMut() + Send + 'static,
    {
        match supervisor::spawn(
            name.to_string(),
            self.running.clone(),
            self.supervisor_wake.clone(),
            self.status.clone(),
            task,
        ) {
            Ok(handle) => Some(handle),
            Err(error) => {
                tracing::error!(
                    worker = name,
                    %error,
                    "cannot spawn long-lived worker; daemon continues without it"
                );
                let mut status = lock_status(&self.status);
                status.last_worker_error = Some(format!("{name}: {error}"));
                drop(status);
                None
            }
        }
    }

    pub(crate) fn start_tagging_worker(&mut self) {
        if self.tagging_handle.is_some() {
            return;
        }
        let config = TaggingWorkerConfig {
            idle_timeout: Duration::from_secs(self.settings.ai_tagging_idle_timeout_sec),
            ..TaggingWorkerConfig::default()
        };
        let worker = TaggingWorker::new(
            self.db.clone(),
            self.ai_data_dir.clone(),
            self.plugin.clone(),
            self.events.clone(),
            self.running.clone(),
            self.tagging.clone(),
            self.tagging_control.clone(),
            config,
        )
        .with_paused(self.paused.clone());
        self.tagging_handle =
            self.spawn_supervised_worker("tagging", move || tagging::run_worker_loop(&worker));
    }

    pub(crate) fn start_workers(&mut self) {
        if self.thumbnail_tx.is_none() {
            match thumbnail::remove_legacy_webp_entries(&self.thumbnail_dir) {
                Ok(0) => {}
                Ok(removed) => tracing::info!(removed, "removed legacy WebP thumbnails"),
                Err(error) => tracing::warn!(%error, "cannot remove legacy WebP thumbnails"),
            }
            let (tx, rx) = mpsc::channel();
            let db = self.db.clone();
            let running = self.running.clone();
            let cache_dir = self.thumbnail_dir.clone();
            let cache_limit = self.thumbnail_cache_limit.clone();
            let spec = self.thumbnail_spec.clone();
            let cache_epoch = self.thumbnail_cache_epoch.clone();
            let external_bytes = self.thumbnail_external_bytes.clone();
            let handle = self.spawn_supervised_worker("thumbnails", move || {
                thumbnail_loop(
                    db.clone(),
                    running.clone(),
                    &rx,
                    cache_dir.clone(),
                    cache_limit.clone(),
                    spec.clone(),
                    cache_epoch.clone(),
                    external_bytes.clone(),
                )
            });
            if let Some(handle) = handle {
                self.thumbnail_tx = Some(tx.clone());
                self.thumbnail_handle = Some(handle);
                let db = lock_db(&self.db);
                if let Ok(photos) =
                    db.list_photos(5000, 0, vrc_photon_core::model::PhotoSort::Newest)
                {
                    for photo in photos {
                        if let Err(error) = tx.send(ThumbnailRequest::Photo(photo.id)) {
                            tracing::debug!(
                                %error,
                                photo_id = photo.id,
                                "thumbnail worker is unavailable; dropping request"
                            );
                        }
                    }
                }
                drop(db);
                for id in std::mem::take(&mut self.deferred_thumbnail_ids) {
                    if let Err(error) = tx.send(ThumbnailRequest::Photo(id)) {
                        tracing::debug!(
                            %error,
                            photo_id = id,
                            "thumbnail worker is unavailable; dropping deferred request"
                        );
                    }
                }
            }
        }
        if self.hash_handle.is_none() {
            let db = self.db.clone();
            let running = self.running.clone();
            let paused = self.paused.clone();
            let wake = self.hash_wake.clone();
            self.hash_handle = self.spawn_supervised_worker("photo-hashes", move || {
                hash_loop(db.clone(), running.clone(), paused.clone(), wake.clone())
            });
        }
        if self.owns_deletion_executor && self.deletion_handle.is_none() {
            let db = self.db.clone();
            let running = self.running.clone();
            let jobs = self.jobs.clone();
            let paused = self.paused.clone();
            self.deletion_handle = self.spawn_supervised_worker("deletion-executor", move || {
                deletion_loop(db.clone(), running.clone(), jobs.clone(), paused.clone())
            });
        }
        if self.send_handle.is_none() {
            let db = self.db.clone();
            let running = self.running.clone();
            let sends = self.sends.clone();
            let events = self.events.clone();
            self.send_handle = self.spawn_supervised_worker("discord-send", move || {
                discord::worker_loop(
                    db.clone(),
                    running.clone(),
                    sends.clone(),
                    events.clone(),
                    Arc::new(UreqTransport::new()),
                )
            });
        }
        if self.world_name_handle.is_none() {
            if self.world_name_resolution_enabled.load(Ordering::Relaxed) {
                self.enqueue_world_name_backfill();
            }
            let db = self.db.clone();
            let running = self.running.clone();
            let enabled = self.world_name_resolution_enabled.clone();
            let wake = self.world_names.clone();
            let paused = self.paused.clone();
            let transport = Arc::new(UreqWorldNameTransport::new());
            self.world_name_handle = self.spawn_supervised_worker("world-name", move || {
                world_names::worker_loop(
                    db.clone(),
                    running.clone(),
                    enabled.clone(),
                    wake.clone(),
                    paused.clone(),
                    transport.clone(),
                )
            });
        }
        self.start_tagging_worker();
        let count = self.settings.compression_workers.clamp(1, 4);
        while self.workers.len() < count {
            let index = self.workers.len();
            let db = self.db.clone();
            let running = self.running.clone();
            let jobs = self.jobs.clone();
            let events = self.events.clone();
            let thumbnail_dir = self.thumbnail_dir.clone();
            let paused = self.paused.clone();
            let budget = self.compression_budget.clone();
            let name = format!("compress-{index}");
            let Some(handle) = self.spawn_supervised_worker(&name, move || {
                worker_loop(
                    db.clone(),
                    running.clone(),
                    jobs.clone(),
                    events.clone(),
                    thumbnail_dir.clone(),
                    paused.clone(),
                    budget.clone(),
                )
            }) else {
                break;
            };
            self.workers.push(handle);
        }
    }

    pub(crate) fn stop_workers(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // Interrupt an in-flight sidecar request before joining the tagging
        // worker; otherwise shutdown waits for the whole inference.
        self.tagging_control.cancel_all();
        self.jobs.notify();
        self.sends.notify();
        self.world_names.notify();
        self.tagging.notify();
        self.hash_wake.notify();
        self.supervisor_wake.notify();
        self.thumbnail_tx = None;
        if let Some(handle) = self.thumbnail_handle.take()
            && handle.join().is_err()
        {
            tracing::error!("thumbnail worker panicked");
        }
        if let Some(handle) = self.hash_handle.take()
            && handle.join().is_err()
        {
            tracing::error!("photo hash worker panicked");
        }
        if let Some(handle) = self.deletion_handle.take()
            && handle.join().is_err()
        {
            tracing::error!("deletion worker panicked");
        }
        if let Some(handle) = self.send_handle.take()
            && handle.join().is_err()
        {
            tracing::error!("Discord send worker panicked");
        }
        if let Some(handle) = self.world_name_handle.take()
            && handle.join().is_err()
        {
            tracing::error!("world name worker panicked");
        }
        if let Some(handle) = self.tagging_handle.take()
            && handle.join().is_err()
        {
            tracing::error!("tagging worker panicked");
        }
        for (index, handle) in self.workers.drain(..).enumerate() {
            if handle.join().is_err() {
                tracing::error!(worker = index, "compression worker panicked");
            }
        }
    }

    /// Puts commands consumed while paused back into the database so a
    /// graceful shutdown does not drop an explicit user request.
    pub(crate) fn persist_deferred_commands(&mut self) {
        if self.deferred_commands.is_empty() {
            return;
        }
        let commands = std::mem::take(&mut self.deferred_commands);
        let db = lock_db(&self.db);
        for command in commands {
            if let Err(error) = db.request_daemon_command(command) {
                tracing::warn!(%error, "cannot requeue daemon command deferred while paused");
            }
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.persist_deferred_commands();
        self.ipc = None;
        self.running.store(false, Ordering::Relaxed);
        self.wake.notify();
        self.sends.notify();
        self.world_names.notify();
        if let Some(mut tail) = self.log_tail.take() {
            let db = lock_db(&self.db);
            let now = chrono::Utc::now().timestamp();
            let started_at = tail.ingestor.session_started_at().unwrap_or(now);
            let ended_at = session_end_time(&db, &tail.path, started_at, now);
            if let Err(error) = tail.ingestor.finish(&db, ended_at) {
                tracing::warn!(%error, "cannot finish log ingestion on shutdown");
            }
        }
        self.stop_workers();
        self.plugin.shutdown(plugin::SHUTDOWN_TIMEOUT);
        self.tray = None;
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.persist_deferred_commands();
        self.ipc = None;
        self.running.store(false, Ordering::Relaxed);
        self.jobs.notify();
        self.sends.notify();
        self.world_names.notify();
        self.tagging.notify();
        self.hash_wake.notify();
        self.supervisor_wake.notify();
        self.wake.notify();
    }
}
