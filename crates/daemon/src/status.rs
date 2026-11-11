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

impl Daemon {
    pub(crate) fn refresh_status(&self) -> Result<()> {
        let (world_name, players, photo_count, pending_jobs, pending_sends) = {
            let db = lock_db(&self.db);
            let now = chrono::Utc::now().timestamp();
            let session = db.session_at(now)?;
            let (world_name, players) = if let Some(session) = &session {
                let world = db.world_by_id(session.world_id)?;
                let players = db.players_at(session.id, now)?.len();
                (world.and_then(|world| world.alias.or(world.name)), players)
            } else {
                (None, 0)
            };
            (
                world_name,
                players,
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
        status.watching = self.settings.log_dir.is_some();
        status.world_name = world_name;
        status.players = players;
        status.photo_count = photo_count;
        status.pending_jobs = pending_jobs;
        status.pending_sends = pending_sends;
        status.pending_tagging_jobs = pending_tagging_jobs;
        status.tagging_ready = tagging_ready;
        status.last_log = self
            .log_tail
            .as_ref()
            .map(|tail| tail.path.to_string_lossy().to_string());
        Ok(())
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
