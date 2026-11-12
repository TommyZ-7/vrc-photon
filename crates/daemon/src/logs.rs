use std::time::SystemTime;

use super::compression::modified_utc;
use super::lifecycle::LocalizedError;
use super::*;

/// Picks the end timestamp for a session: log file mtime, then the mtime
/// recorded by the last log ingestion, then `now`. The result is clamped to
/// `[started_at, now]` so a future or pre-start mtime cannot keep the session
/// visible (and so a clock skew cannot create an invalid interval).
pub(crate) fn session_end_time(db: &Db, log_file: &Path, started_at: i64, now: i64) -> i64 {
    let candidate = modified_utc(log_file)
        .or_else(|| db.log_mtime(&log_file.to_string_lossy()).ok().flatten())
        .unwrap_or(now);
    candidate.clamp(started_at.min(now), now)
}

/// An open session may only survive startup when it belongs to the newest log
/// and VRChat is still running. A missing or unknown process state is treated
/// as "not running" so repair keeps its conservative close behavior.
pub(crate) fn should_resume_open_session(
    open_log: &Path,
    latest_log: Option<&Path>,
    vrchat_running: Option<bool>,
) -> bool {
    vrchat_running == Some(true) && latest_log.is_some_and(|latest| latest == open_log)
}

pub(crate) fn latest_log_file(dir: &Path) -> Result<Option<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read log folder {}", dir.display()));
        }
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    (name.starts_with("output_log_") || name == "output_log.txt")
                        && name.ends_with(".txt")
                })
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    Ok(files.pop())
}

/// Caches the newest log file and only rescans the directory when its mtime
/// changes, the cached file disappears, or the fallback interval expires.
pub(crate) struct LogLocator {
    dir: PathBuf,
    dir_mtime: Option<SystemTime>,
    latest: Option<PathBuf>,
    last_scan: Instant,
    #[cfg(test)]
    scans: usize,
}

impl LogLocator {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            dir_mtime: None,
            latest: None,
            last_scan: Instant::now(),
            #[cfg(test)]
            scans: 0,
        }
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn invalidate(&mut self) {
        self.latest = None;
        self.dir_mtime = None;
        self.last_scan = Instant::now() - LOG_DIR_RESCAN_FALLBACK;
    }

    pub(crate) fn latest(&mut self) -> Result<Option<PathBuf>> {
        let metadata = match std::fs::metadata(&self.dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.invalidate();
                return Ok(None);
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("cannot read log folder {}", self.dir.display()));
            }
        };
        let mtime = metadata.modified().ok();
        let cached_ok = self.dir_mtime.is_some()
            && mtime == self.dir_mtime
            && self.latest.as_ref().is_none_or(|path| path.is_file());
        let fallback_due = self.last_scan.elapsed() >= LOG_DIR_RESCAN_FALLBACK;
        if cached_ok && !fallback_due {
            return Ok(self.latest.clone());
        }

        #[cfg(test)]
        {
            self.scans += 1;
        }
        let latest = latest_log_file(&self.dir)?;
        self.latest = latest.clone();
        self.dir_mtime = mtime;
        self.last_scan = Instant::now();
        Ok(latest)
    }
}

/// Returns the ids of replaced photos whose display derivative is missing.
/// Roots that are not currently directories are skipped entirely so a
/// disconnected drive never marks its photos missing. Root checks are cached
/// per root, and both predicates run without the database lock held.
pub(crate) fn missing_display_derivatives(
    rows: Vec<(i64, Option<String>, String)>,
    mut root_is_dir: impl FnMut(&Path) -> bool,
    mut file_is_file: impl FnMut(&Path) -> bool,
) -> Vec<i64> {
    let mut roots: HashMap<String, bool> = HashMap::new();
    let mut missing = Vec::new();
    for (photo_id, path, root) in rows {
        let available = *roots
            .entry(root.clone())
            .or_insert_with(|| root_is_dir(Path::new(&root)));
        if !available {
            continue;
        }
        let valid = path
            .as_deref()
            .is_some_and(|path| file_is_file(Path::new(path)));
        if !valid {
            missing.push(photo_id);
        }
    }
    missing
}

/// Reads complete lines from `offset`, leaving a trailing partial line for the
/// next poll. Each line is returned with its end byte offset.
pub(crate) fn read_new_lines(path: &Path, offset: i64) -> Result<(Vec<(String, i64)>, i64)> {
    read_new_lines_bounded(path, offset, LOG_READ_MAX_LINES, LOG_READ_MAX_BYTES)
}

/// Bounded variant used by tests and [`read_new_lines`]. It never reads past
/// the byte budget, so a partial line at the budget boundary is retried on the
/// next poll instead of being treated as complete.
pub(crate) fn read_new_lines_bounded(
    path: &Path,
    offset: i64,
    max_lines: usize,
    max_bytes: u64,
) -> Result<(Vec<(String, i64)>, i64)> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset.max(0) as u64))?;
    let mut reader = BufReader::new(file).take(max_bytes);
    let mut buffer = Vec::new();
    let mut lines = Vec::new();
    let mut complete_lines = 0usize;
    let mut consumed = 0u64;
    while complete_lines < max_lines {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 || buffer.last() != Some(&b'\n') {
            break;
        }
        complete_lines += 1;
        consumed += read as u64;
        let text = String::from_utf8_lossy(&buffer[..buffer.len() - 1]);
        let line = text.trim_end_matches('\r');
        if !line.is_empty() {
            lines.push((line.to_string(), offset + consumed as i64));
        }
    }
    Ok((lines, offset + consumed as i64))
}

#[derive(Debug)]
pub(crate) struct LogTail {
    pub(crate) path: PathBuf,
    pub(crate) ingestor: LogIngestor,
    pub(crate) offset: i64,
    pub(crate) failed_offset: Option<i64>,
    pub(crate) failed_attempts: u8,
}

/// Remembers a failed line at `offset`. The caller retries the line until it
/// has failed three times, then this returns `true` so the poison line can be
/// skipped.
pub(crate) fn record_line_failure(
    failed_offset: &mut Option<i64>,
    failed_attempts: &mut u8,
    offset: i64,
) -> bool {
    if *failed_offset != Some(offset) {
        *failed_offset = Some(offset);
        *failed_attempts = 1;
        return false;
    }
    *failed_attempts = failed_attempts.saturating_add(1);
    if *failed_attempts >= 3 {
        *failed_offset = None;
        *failed_attempts = 0;
        return true;
    }
    false
}

#[derive(Debug, Default)]
pub(crate) struct LogPass {
    pub(crate) processed: usize,
    pub(crate) skipped: usize,
    pub(crate) failure: Option<String>,
    /// One sentence per skipped poison line, capped at [`MAX_SKIPPED_DETAILS`].
    pub(crate) skipped_details: Vec<String>,
}

impl LogPass {
    /// Records one poison line that was skipped, including its path and byte
    /// range so the reported error changes even when the parse error repeats.
    pub(crate) fn record_skip(&mut self, path: &Path, start: i64, end: i64) {
        self.skipped += 1;
        if self.skipped_details.len() < MAX_SKIPPED_DETAILS {
            self.skipped_details.push(format!(
                "{}: オフセット {}〜{} の行は3回失敗したためスキップしました",
                path.display(),
                start,
                end
            ));
        }
    }

    /// The original error plus the skipped-line sentences. Skips beyond the
    /// cap are summarized so one pass cannot grow the detail without bound.
    pub(crate) fn failure_detail(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(failure) = &self.failure {
            parts.push(failure.clone());
        }
        parts.extend(self.skipped_details.iter().cloned());
        let omitted = self.skipped.saturating_sub(self.skipped_details.len());
        if omitted > 0 {
            parts.push(format!("ほか {omitted} 行を同じ理由でスキップしました"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n"))
        }
    }

    /// A pass can only clear a previous error when it processed real log data
    /// without skipping or failing on any line.
    pub(crate) fn is_clean(&self) -> bool {
        self.processed > 0 && self.skipped == 0 && self.failure.is_none()
    }
}

impl Daemon {
    pub(crate) fn repair_open_sessions(&mut self) -> Result<usize> {
        self.repair_open_sessions_with_running(vrchat_process::is_running())
    }

    pub(crate) fn repair_open_sessions_with_running(
        &mut self,
        vrchat_running: Option<bool>,
    ) -> Result<usize> {
        if !self.owns_deletion_executor {
            return Ok(0);
        }
        let latest_log = self
            .settings
            .log_dir()
            .and_then(|dir| match latest_log_file(&dir) {
                Ok(path) => path,
                Err(error) => {
                    tracing::warn!(%error, "cannot resolve the latest log for session repair");
                    None
                }
            });
        let db = lock_db(&self.db);
        let sessions = db.open_sessions()?;
        let now = chrono::Utc::now().timestamp();
        let mut repaired = 0;
        for session in sessions {
            if should_resume_open_session(
                Path::new(&session.log_file),
                latest_log.as_deref(),
                vrchat_running,
            ) {
                tracing::info!(
                    session_id = session.id,
                    log_file = %session.log_file,
                    "resuming open session at startup"
                );
                continue;
            }
            let ended_at =
                session_end_time(&db, Path::new(&session.log_file), session.started_at, now);
            db.close_session(session.id, ended_at)?;
            tracing::info!(
                session_id = session.id,
                log_file = %session.log_file,
                ended_at,
                "closed stale session at startup"
            );
            repaired += 1;
        }
        Ok(repaired)
    }

    pub(crate) fn poll_vrchat_presence(&mut self) -> Result<()> {
        let has_session = self
            .log_tail
            .as_ref()
            .and_then(|tail| tail.ingestor.session_id())
            .is_some();
        if !has_session {
            self.vrchat_missing_polls = 0;
            self.vrchat_session_id = None;
            return Ok(());
        }
        let running = self.vrchat_probe.poll();
        self.observe_vrchat_presence(running)
    }

    pub(crate) fn observe_vrchat_presence(&mut self, running: Option<bool>) -> Result<()> {
        let Some(session_id) = self
            .log_tail
            .as_ref()
            .and_then(|tail| tail.ingestor.session_id())
        else {
            self.vrchat_missing_polls = 0;
            self.vrchat_session_id = None;
            return Ok(());
        };
        if self.vrchat_session_id != Some(session_id) {
            self.vrchat_session_id = Some(session_id);
            self.vrchat_missing_polls = 0;
        }
        let Some(running) = running else {
            self.vrchat_missing_polls = 0;
            return Ok(());
        };
        if running {
            self.vrchat_missing_polls = 0;
            return Ok(());
        }
        self.vrchat_missing_polls = self.vrchat_missing_polls.saturating_add(1);
        if self.vrchat_missing_polls < VRCHAT_MISSING_POLLS_BEFORE_CLOSE {
            return Ok(());
        }

        let now = chrono::Utc::now().timestamp();
        let (ended_at, closed) = {
            let db = lock_db(&self.db);
            let ended_at = self
                .log_tail
                .as_ref()
                .map(|tail| {
                    let started_at = tail.ingestor.session_started_at().unwrap_or(now);
                    session_end_time(&db, &tail.path, started_at, now)
                })
                .unwrap_or(now);
            let closed = self
                .log_tail
                .as_mut()
                .map(|tail| tail.ingestor.close_current_session(&db, ended_at))
                .transpose()?
                .unwrap_or(false);
            (ended_at, closed)
        };
        self.vrchat_missing_polls = 0;
        self.vrchat_session_id = None;
        if closed {
            tracing::info!(ended_at, "closed session after VRChat exited");
            self.refresh_status()?;
        }
        Ok(())
    }

    pub(crate) fn ingest_logs(&mut self) -> Result<()> {
        let Some(log_dir) = self.settings.log_dir() else {
            self.clear_daemon_error(DaemonOperation::Log)?;
            return Ok(());
        };
        if !log_dir.is_dir() {
            self.report_localized_error(
                DaemonOperation::Log,
                LocalizedError::new("daemon.log_dir_missing").arg("path", log_dir.display()),
                None,
            )?;
            return Ok(());
        }
        if self
            .log_locator
            .as_ref()
            .is_none_or(|locator| locator.dir() != log_dir.as_path())
        {
            self.log_locator = Some(LogLocator::new(log_dir.clone()));
        }
        let path = match self.log_locator.as_mut().unwrap().latest() {
            Ok(Some(path)) => path,
            Ok(None) => return Ok(()),
            Err(error) => {
                self.report_localized_error(
                    DaemonOperation::Log,
                    LocalizedError::new("daemon.log_unreadable"),
                    Some(&error.to_string()),
                )?;
                return Ok(());
            }
        };
        let pass = self.ingest_log_pass(&path)?;
        if let Some(failure) = pass.failure_detail() {
            self.report_localized_error(
                DaemonOperation::Log,
                LocalizedError::new("daemon.log_ingest_failed"),
                Some(&failure),
            )?;
        } else if pass.is_clean() {
            self.clear_daemon_error(DaemonOperation::Log)?;
        }
        Ok(())
    }

    pub(crate) fn ingest_log_pass(&mut self, path: &Path) -> Result<LogPass> {
        let mut pass = LogPass::default();
        let path_string = path.to_string_lossy().to_string();
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                pass.failure = Some(format!("{}: {error}", path.display()));
                return Ok(pass);
            }
        };
        let size = metadata.len() as i64;
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);

        let new_tail = self
            .log_tail
            .as_ref()
            .map(|tail| tail.path != path)
            .unwrap_or(true);
        if new_tail {
            if let Some(mut previous) = self.log_tail.take() {
                let db = lock_db(&self.db);
                let now = chrono::Utc::now().timestamp();
                let started_at = previous.ingestor.session_started_at().unwrap_or(now);
                let finish_at = session_end_time(&db, &previous.path, started_at, now);
                previous.ingestor.finish(&db, finish_at)?;
            }
            let (offset, ingestor) = {
                let db = lock_db(&self.db);
                let offset = db.log_offset(&path_string)?.unwrap_or(0);
                let open_session = db
                    .open_sessions()?
                    .into_iter()
                    .find(|session| session.log_file == path_string);
                let ingestor = match open_session {
                    Some(session) => LogIngestor::resume_from_db(&db, &path_string, session.id)?
                        .unwrap_or_else(|| LogIngestor::new(&path_string)),
                    None => LogIngestor::new(&path_string),
                };
                (offset, ingestor)
            };
            self.log_tail = Some(LogTail {
                path: path.to_path_buf(),
                ingestor,
                offset,
                failed_offset: None,
                failed_attempts: 0,
            });
        }

        let tail = self.log_tail.as_mut().unwrap();
        if size < tail.offset {
            tail.offset = 0;
            tail.ingestor = LogIngestor::new(&path_string);
            tail.failed_offset = None;
            tail.failed_attempts = 0;
        }
        let (lines, new_offset) = match read_new_lines(path, tail.offset) {
            Ok(result) => result,
            Err(error) => {
                pass.failure = Some(format!("{}: {error}", path.display()));
                return Ok(pass);
            }
        };
        let mut next = 0;
        while next < lines.len() {
            let chunk_end = (next + LOG_INGEST_CHUNK_LINES).min(lines.len());
            let chunk = &lines[next..chunk_end];
            let chunk_start = tail.offset;
            // Work on a copy so a failed transaction leaves both the database
            // and the in-memory session state at the last committed offset.
            let mut ingestor = tail.ingestor.clone();
            let mut line_start = chunk_start;
            let mut committed_offset = chunk_start;
            let mut failed_offset = tail.failed_offset;
            let mut failed_attempts = tail.failed_attempts;
            let mut processed = 0;
            let mut skipped = Vec::new();
            let mut failure = None;
            let mut stop = false;
            let result = {
                let db = lock_db(&self.db);
                db.in_transaction(|db| {
                    for (line, line_end) in chunk {
                        match ingestor.ingest_line(db, line) {
                            Ok(_) => {
                                committed_offset = *line_end;
                                line_start = *line_end;
                                processed += 1;
                            }
                            Err(error) => {
                                tracing::debug!(%error, "cannot parse log line");
                                failure
                                    .get_or_insert_with(|| format!("{}: {error}", path.display()));
                                if !record_line_failure(
                                    &mut failed_offset,
                                    &mut failed_attempts,
                                    line_start,
                                ) {
                                    stop = true;
                                    break;
                                }
                                skipped.push((line_start, *line_end));
                                committed_offset = *line_end;
                                line_start = *line_end;
                            }
                        }
                    }
                    // The ingest and its offset commit atomically. A crash
                    // between chunks re-reads only lines that were rolled back.
                    if committed_offset != chunk_start {
                        db.set_log_offset(
                            &path_string,
                            size,
                            mtime,
                            committed_offset,
                            PARSER_VERSION,
                        )?;
                    }
                    Ok(())
                })
            };
            if let Err(error) = result {
                return Err(error.into());
            }
            tail.ingestor = ingestor;
            tail.offset = committed_offset;
            tail.failed_offset = failed_offset;
            tail.failed_attempts = failed_attempts;
            pass.processed += processed;
            if let Some(failure) = failure {
                pass.failure.get_or_insert(failure);
            }
            for (start, end) in skipped {
                pass.record_skip(path, start, end);
            }
            if stop {
                break;
            }
            next = chunk_end;
        }
        if pass.failure.is_none() && tail.offset != new_offset {
            let db = lock_db(&self.db);
            db.set_log_offset(&path_string, size, mtime, new_offset, PARSER_VERSION)?;
            drop(db);
            tail.offset = new_offset;
        }
        self.refresh_status()?;
        self.link_unlinked()?;
        Ok(pass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_log(dir: &Path, name: &str) {
        fs::write(dir.join(name), "log line\n").unwrap();
    }

    #[test]
    fn locator_reuses_the_cached_latest_without_rescanning() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        write_log(&dir, "output_log_2026-01-01_00-00-00.txt");

        let mut locator = LogLocator::new(dir);
        assert!(locator.latest().unwrap().is_some());
        assert_eq!(locator.scans, 1);

        assert!(locator.latest().unwrap().is_some());
        assert_eq!(locator.scans, 1, "unchanged directory must not be rescanned");
    }

    #[test]
    fn locator_detects_a_new_log_file() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        write_log(&dir, "output_log_2026-01-01_00-00-00.txt");
        let mut locator = LogLocator::new(dir.clone());
        assert_eq!(
            locator.latest().unwrap().unwrap().file_name().unwrap(),
            "output_log_2026-01-01_00-00-00.txt"
        );
        assert_eq!(locator.scans, 1);

        write_log(&dir, "output_log_2026-01-02_00-00-00.txt");
        assert_eq!(
            locator.latest().unwrap().unwrap().file_name().unwrap(),
            "output_log_2026-01-02_00-00-00.txt"
        );
        assert_eq!(locator.scans, 2);
    }

    #[test]
    fn locator_rescans_when_the_cached_file_disappears() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        let path = dir.join("output_log_2026-01-01_00-00-00.txt");
        write_log(&dir, "output_log_2026-01-01_00-00-00.txt");

        let mut locator = LogLocator::new(dir);
        assert_eq!(locator.latest().unwrap().as_deref(), Some(path.as_path()));
        assert_eq!(locator.scans, 1);

        fs::remove_file(&path).unwrap();
        assert!(locator.latest().unwrap().is_none());
        assert_eq!(locator.scans, 2);
    }

    #[test]
    fn locator_rescans_after_the_fallback_interval() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        write_log(&dir, "output_log_2026-01-01_00-00-00.txt");

        let mut locator = LogLocator::new(dir);
        assert!(locator.latest().unwrap().is_some());
        assert_eq!(locator.scans, 1);

        locator.last_scan = Instant::now() - LOG_DIR_RESCAN_FALLBACK - Duration::from_secs(1);
        assert!(locator.latest().unwrap().is_some());
        assert_eq!(locator.scans, 2);
    }
}
