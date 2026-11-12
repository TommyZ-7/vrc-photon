use vrc_photon_ipc::StatusSnapshot;

use super::*;

pub(crate) fn ipc_operation(operation: DaemonOperation) -> Operation {
    match operation {
        DaemonOperation::Scan => Operation::Scan,
        DaemonOperation::Log => Operation::Log,
        DaemonOperation::Link => Operation::Link,
    }
}

pub(crate) fn error_signature(message: &str, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("{message}\n{detail}"),
        None => message.to_string(),
    }
}

pub(crate) fn supervisor_label(
    invocation_id: Option<&std::ffi::OsStr>,
    exec_pid: Option<&std::ffi::OsStr>,
    current_pid: u32,
) -> &'static str {
    let exec_pid = exec_pid
        .and_then(|pid| pid.to_str())
        .and_then(|pid| pid.parse::<u32>().ok());
    if invocation_id.is_some() && exec_pid == Some(current_pid) {
        "systemd"
    } else {
        "app"
    }
}

pub(crate) fn cgroup_has_unit(cgroup: &str, unit: &str) -> bool {
    cgroup
        .lines()
        .flat_map(|line| line.split('/'))
        .any(|part| part == unit)
}

/// Compares the observable status fields. `version` is stamped on every
/// snapshot and must not make an unchanged status look different.
fn same_status_fields(a: &StatusSnapshot, b: &StatusSnapshot) -> bool {
    a.paused == b.paused
        && a.watching == b.watching
        && a.scanning == b.scanning
        && a.scan_phase == b.scan_phase
        && a.scan_processed == b.scan_processed
        && a.scan_total == b.scan_total
        && a.scan_indexed == b.scan_indexed
        && a.scan_root == b.scan_root
        && a.scan_roots == b.scan_roots
        && a.photo_count == b.photo_count
        && a.pending_jobs == b.pending_jobs
        && a.pending_sends == b.pending_sends
        && a.pending_tagging_jobs == b.pending_tagging_jobs
        && a.tagging_ready == b.tagging_ready
}

impl Daemon {
    /// Refreshes the cheap session view: world, players, watching, last log.
    pub(crate) fn refresh_session_status(&self) -> Result<()> {
        let (world_name, players) = {
            let db = lock_db(&self.db);
            let now = chrono::Utc::now().timestamp();
            match db.session_at(now)? {
                Some(session) => {
                    let world = db.world_by_id(session.world_id)?;
                    let players = db.players_at(session.id, now)?.len();
                    (world.and_then(|world| world.alias.or(world.name)), players)
                }
                None => (None, 0),
            }
        };
        let mut status = lock_status(&self.status);
        status.watching = self.settings.log_dir.is_some();
        status.world_name = world_name;
        status.players = players;
        status.last_log = self
            .log_tail
            .as_ref()
            .map(|tail| tail.path.to_string_lossy().to_string());
        Ok(())
    }

    /// Refreshes the expensive library and queue aggregates.
    pub(crate) fn refresh_counts(&self) -> Result<()> {
        let (photo_count, pending_jobs, pending_sends) = {
            let db = lock_db(&self.db);
            (
                db.count_photos()?,
                db.count_jobs(QueueStatus::Pending)?,
                db.count_pending_sends()?,
            )
        };
        // The readiness check reads the database, so it must run after the
        // block above released the lock. The cached values keep IPC snapshots
        // cheap and lock-order safe.
        let (pending_tagging_jobs, tagging_ready) = tagging_status(&self.db, &self.plugin);
        let mut status = lock_status(&self.status);
        status.photo_count = photo_count;
        status.pending_jobs = pending_jobs;
        status.pending_sends = pending_sends;
        status.pending_tagging_jobs = pending_tagging_jobs;
        status.tagging_ready = tagging_ready;
        Ok(())
    }

    /// Refreshes both halves. Used by startup and command execution, where a
    /// precise snapshot matters more than saving one aggregate query.
    pub(crate) fn refresh_status(&self) -> Result<()> {
        self.refresh_session_status()?;
        self.refresh_counts()
    }

    /// Marks the aggregate counts for a refresh on the next loop iteration.
    pub(crate) fn invalidate_counts(&mut self) {
        self.last_counts_status = Instant::now() - COUNTS_STATUS_INTERVAL;
    }

    /// Publishes a status event only when the observable fields changed.
    pub(crate) fn publish_status_if_changed(&mut self) {
        let snapshot = {
            let mut status = lock_status(&self.status);
            status_snapshot(&mut status)
        };
        let changed = self
            .last_published_status
            .as_ref()
            .is_none_or(|previous| !same_status_fields(previous, &snapshot));
        if changed {
            self.last_published_status = Some(snapshot.clone());
            self.events.publish(Event::StatusChanged { status: snapshot });
        }
    }

    pub(crate) fn write_heartbeat(&mut self) {

        let db = lock_db(&self.db);
        if let Err(error) = db.set_setting(
            "daemon_heartbeat",
            &chrono::Utc::now().timestamp().to_string(),
        ) {
            tracing::warn!(%error, "cannot write daemon heartbeat");
        }
        drop(db);
        self.last_heartbeat = std::time::Instant::now();
    }

    pub(crate) fn record_supervisor(&self) {
        let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok();
        let supervisor = if cgroup
            .as_deref()
            .is_some_and(|cgroup| cgroup_has_unit(cgroup, "vrc-photon-daemon.service"))
        {
            "systemd"
        } else {
            supervisor_label(
                std::env::var_os("INVOCATION_ID").as_deref(),
                std::env::var_os("SYSTEMD_EXEC_PID").as_deref(),
                std::process::id(),
            )
        };
        let db = lock_db(&self.db);
        if let Err(error) = db.set_setting("daemon_supervisor", supervisor) {
            tracing::warn!(%error, "cannot record daemon supervisor");
        }
    }
}
