use super::compression::{enqueue_auto_jobs, enqueue_auto_tagging, promote_display_derivative};
use super::lifecycle::LocalizedError;
use super::logs::missing_display_derivatives;
use super::thumbnails::ThumbnailRequest;
use super::*;

/// Re-links photos whose XMP has already been read outside the database lock.
///
/// `read_xmp` must be called while no database lock is held; the daemon passes
/// a closure that reads the PNG sidecar. The database lock is taken again for
/// each short link/retry write.
pub(crate) fn relink_photos_outside_lock<F>(
    db: &Arc<Mutex<Db>>,
    photos: Vec<Photo>,
    estimated_enabled: bool,
    now: i64,
    mut read_xmp: F,
) -> Result<usize>
where
    F: FnMut(&Photo) -> Option<XmpMetadata>,
{
    let mut linked = 0;
    for photo in photos {
        let xmp = read_xmp(&photo);
        let is_linked = {
            let db = lock_db(db);
            linker::link_photo_with_xmp(&db, &photo, xmp, estimated_enabled)?
        };
        if is_linked {
            linked += 1;
        } else {
            let db = lock_db(db);
            db.record_link_attempt(photo.id, now)?;
        }
    }
    Ok(linked)
}

pub(crate) struct ScanObserver {
    events: EventBus,
    status: Arc<Mutex<DaemonStatus>>,
    indexed_before: usize,
    root: usize,
    roots: usize,
    last_emit: Instant,
    last_phase: Option<IpcScanPhase>,
}

impl ScanObserver {
    pub(crate) fn new(
        events: EventBus,
        status: Arc<Mutex<DaemonStatus>>,
        indexed_before: usize,
        root: usize,
        roots: usize,
    ) -> Self {
        Self {
            events,
            status,
            indexed_before,
            root,
            roots,
            last_emit: Instant::now(),
            last_phase: None,
        }
    }

    pub(crate) fn observe(&mut self, progress: ScanProgress) {
        let phase = match progress.phase {
            CoreScanPhase::Walking => IpcScanPhase::Walking,
            CoreScanPhase::Indexing => IpcScanPhase::Indexing,
        };
        let final_indexing =
            progress.phase == CoreScanPhase::Indexing && progress.total == Some(progress.processed);
        let phase_changed = self.last_phase != Some(phase);
        if !phase_changed
            && !final_indexing
            && self.last_emit.elapsed() < Duration::from_millis(200)
        {
            return;
        }
        self.last_phase = Some(phase);
        self.last_emit = Instant::now();
        let indexed = self.indexed_before + progress.indexed;
        {
            let mut status = lock_status(&self.status);
            status.scan_phase = Some(phase);
            status.scan_processed = progress.processed;
            status.scan_total = progress.total;
            status.scan_indexed = indexed;
            status.scan_root = self.root;
            status.scan_roots = self.roots;
        }
        self.events.publish(Event::ScanProgress {
            phase,
            processed: progress.processed,
            total: progress.total,
            indexed,
            root: self.root,
            roots: self.roots,
        });
    }
}

impl Daemon {
    pub(crate) fn index_dirty_paths(&mut self) -> Result<()> {
        let mut paths = std::mem::take(&mut self.dirty_paths);
        if paths.is_empty() {
            return Ok(());
        }
        self.refresh_compression_settings();
        let tagging_ready = self.plugin.is_ready(AI_PLUGIN_ID);
        paths.sort_by_cached_key(|path| !path.is_file());
        paths.dedup();
        let roots = self.settings.photo_roots();
        let mut new_ids = Vec::new();
        let mut thumbnail_ids = Vec::new();
        let mut missing_ids = Vec::new();
        let mut changed = false;
        for path in paths {
            let Some(root) = roots.iter().find(|root| path.starts_with(root)) else {
                continue;
            };
            let root_id = {
                let db = lock_db(&self.db);
                db.add_root(&root.to_string_lossy(), "reference")?
            };
            if path.is_file() {
                let Some(scanned) = scan_one(root, &path) else {
                    continue;
                };
                let mut new_photo = scanned.into_new_photo(root_id);
                let abs_path = new_photo.abs_path.clone();
                let existing = {
                    let db = lock_db(&self.db);
                    db.photo_by_path(&abs_path)?
                };
                let existed = existing.is_some();
                new_photo.blake3 = match existing.as_ref() {
                    Some(existing)
                        if existing.blake3.is_some()
                            && existing.file_size == new_photo.file_size
                            && existing.mtime == new_photo.mtime =>
                    {
                        existing.blake3.clone()
                    }
                    _ => hashing::file_blake3(Path::new(&new_photo.abs_path))
                        .ok()
                        .flatten(),
                };

                if !existed {
                    let candidate = {
                        let db = lock_db(&self.db);
                        db.find_rebind_candidate(&new_photo)?
                    };
                    if let Some(candidate) = candidate {
                        let old_path_exists = Path::new(&candidate.abs_path).exists();
                        let rebound = if old_path_exists {
                            false
                        } else {
                            let db = lock_db(&self.db);
                            db.rebind_photo(candidate.id, &new_photo)?
                        };
                        if rebound {
                            thumbnail_ids.push(candidate.id);
                            changed = true;
                            continue;
                        }
                    }
                }

                let id = {
                    let db = lock_db(&self.db);
                    db.upsert_photo(&new_photo)?
                };
                changed = true;
                if !existed {
                    new_ids.push(id);
                    thumbnail_ids.push(id);
                }
            } else {
                let photo = {
                    let db = lock_db(&self.db);
                    db.photo_by_path(&path.to_string_lossy())?
                };
                let Some(photo) = photo else {
                    continue;
                };
                if photo.status == PhotoStatus::Present {
                    let marked = {
                        let db = lock_db(&self.db);
                        db.mark_missing_if_path(photo.id, &photo.abs_path)?
                    };
                    if marked {
                        missing_ids.push(photo.id);
                        changed = true;
                    }
                }
            }
        }
        // A remove event and its matching create event can arrive in one
        // batch. Only after every addition has had a chance to rebind do we
        // promote a derivative for rows that are still missing.
        {
            let db = lock_db(&self.db);
            for photo_id in missing_ids {
                if let Some(photo) = db.photo_by_id(photo_id)?
                    && photo.status == PhotoStatus::Missing
                    && promote_display_derivative(&db, &photo)?
                {
                    changed = true;
                }
            }
            if changed {
                db.bump_library_revision()?;
            }
        }
        if self.settings.auto_compress && !new_ids.is_empty() {
            {
                let db = lock_db(&self.db);
                enqueue_auto_jobs(
                    &db,
                    &new_ids,
                    self.settings.auto_compress_keep_original,
                    self.settings.auto_compress_immediate,
                )?;
            }
            self.jobs.notify();
        }
        if !new_ids.is_empty() {
            let tagging_enqueued = {
                let db = lock_db(&self.db);
                enqueue_auto_tagging(&db, &new_ids, &self.settings, tagging_ready)?
            };
            if tagging_enqueued > 0 {
                self.tagging.notify();
            }
        }
        if let Some(tx) = &self.thumbnail_tx {
            for id in &thumbnail_ids {
                if let Err(error) = tx.send(ThumbnailRequest::Photo(*id)) {
                    tracing::debug!(
                        %error,
                        photo_id = *id,
                        "thumbnail worker is unavailable; dropping request"
                    );
                }
            }
        }
        if !new_ids.is_empty() {
            self.link_unlinked()?;
            let enqueued = {
                let db = lock_db(&self.db);
                match discord::enqueue_auto_sends(&db, &new_ids) {
                    Ok(count) => count,
                    Err(error) => {
                        // Automatic sending is best effort: an outbox error
                        // must not fail or roll back the photo scan.
                        tracing::warn!(%error, "cannot enqueue automatic Discord sends");
                        0
                    }
                }
            };
            if enqueued > 0 {
                self.sends.notify();
            }
        }
        self.hash_wake.notify();
        self.invalidate_counts();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn scan_photos(&mut self) -> Result<ScanReport> {
        self.scan_photos_with_initial(None)
    }

    pub(crate) fn scan_photos_with_initial(
        &mut self,
        initial_scan: Option<bool>,
    ) -> Result<ScanReport> {
        let roots = self.settings.photo_roots();
        self.scan_roots_with_initial(roots, true, initial_scan)
    }

    pub(crate) fn scan_root_photos(&mut self, root: PathBuf) -> Result<ScanReport> {
        self.scan_roots_with_initial(vec![root], false, None)
    }

    pub(crate) fn scan_roots_with_initial(
        &mut self,
        roots: Vec<PathBuf>,
        full_scan: bool,
        initial_scan: Option<bool>,
    ) -> Result<ScanReport> {
        let target =
            (!full_scan && roots.len() == 1).then(|| roots[0].to_string_lossy().to_string());
        self.begin_scan(roots.len(), target.clone());
        let result = self.scan_roots_inner(roots, full_scan, initial_scan);
        let (report_result, auto_enqueued) = match result {
            Ok((report, auto_enqueued)) => (Ok(report), auto_enqueued),
            Err(error) => (Err(error), 0),
        };
        self.finish_scan(&report_result, auto_enqueued, target);
        // Covers both the full scan and ScanRoot commands. A no-op when the
        // scan found nothing; the hash worker also has a long fallback.
        self.hash_wake.notify();
        self.invalidate_counts();
        report_result
    }

    pub(crate) fn begin_scan(&self, roots: usize, target: Option<String>) {
        {
            let mut status = lock_status(&self.status);
            status.scanning = true;
            status.scan_phase = Some(IpcScanPhase::Walking);
            status.scan_processed = 0;
            status.scan_total = None;
            status.scan_indexed = 0;
            status.scan_root = usize::from(roots > 0);
            status.scan_roots = roots;
        }
        self.events.publish(Event::ScanStarted { roots, target });
    }

    pub(crate) fn finish_scan(
        &self,
        result: &Result<ScanReport>,
        auto_enqueued: usize,
        target: Option<String>,
    ) {
        let (found, indexed, skipped, failures, failed) = match result {
            Ok(report) => (
                report.found,
                report.indexed,
                report.skipped,
                report.failures.len(),
                false,
            ),
            Err(_) => (0, 0, 0, 0, true),
        };
        {
            let mut status = lock_status(&self.status);
            status.scanning = false;
            status.scan_phase = None;
            status.scan_processed = found;
            status.scan_total = Some(found);
            status.scan_indexed = indexed;
        }
        self.events.publish(Event::ScanFinished {
            found,
            indexed,
            skipped,
            failures,
            failed,
            auto_enqueued,
            target,
        });
    }

    pub(crate) fn scan_roots_inner(
        &mut self,
        roots: Vec<PathBuf>,
        full_scan: bool,
        initial_scan_override: Option<bool>,
    ) -> Result<(ScanReport, usize)> {
        self.refresh_compression_settings();
        let tagging_ready = self.plugin.is_ready(AI_PLUGIN_ID);
        let mut report = ScanReport::default();
        let mut auto_enqueued = 0;
        let root_count = roots.len();
        if !roots.is_empty() {
            let initial_scan = {
                let db = lock_db(&self.db);
                initial_scan_override.unwrap_or(full_scan && db.count_photos()? == 0)
            };
            let mut changed = false;
            let mut missing_ids = Vec::new();
            for (index, root) in roots.into_iter().enumerate() {
                if self.stopping() {
                    report.cancelled = true;
                    break;
                }
                let root_number = index + 1;
                {
                    let mut status = lock_status(&self.status);
                    status.scan_root = root_number;
                    status.scan_roots = root_count;
                    status.scan_phase = Some(IpcScanPhase::Walking);
                    status.scan_processed = 0;
                    status.scan_total = None;
                }
                if !root.is_dir() {
                    tracing::warn!(path = %root.display(), "photo root does not exist");
                    report.failures.push(ScanFailure {
                        path: root,
                        message: vrc_photon_i18n::tr!("daemon.photo_root_missing"),
                    });
                    continue;
                }
                let root_id = {
                    let db = lock_db(&self.db);
                    db.add_root(&root.to_string_lossy(), "reference")?
                };
                let existing = {
                    let db = lock_db(&self.db);
                    db.photos_for_root(root_id)?
                };
                let indexed_before = report.indexed;
                let mut observer = ScanObserver::new(
                    self.events.clone(),
                    self.status.clone(),
                    indexed_before,
                    root_number,
                    root_count,
                );
                let root_report = match scan_and_index_shared(
                    &self.db,
                    root_id,
                    &root,
                    Some(self.running.as_ref()),
                    |progress| observer.observe(progress),
                ) {
                    Ok(root_report) => root_report,
                    Err(error) => {
                        tracing::error!(path = %root.display(), %error, "photo scan failed");
                        report.failures.push(ScanFailure {
                            path: root,
                            message: error.to_string(),
                        });
                        continue;
                    }
                };
                if root_report.failures.is_empty() && !root_report.cancelled {
                    let seen: std::collections::HashSet<&str> =
                        root_report.seen_paths.iter().map(String::as_str).collect();
                    let rebound: std::collections::HashSet<i64> =
                        root_report.rebound_ids.iter().copied().collect();
                    // Keep the write lock bounded even when a large library
                    // contains many deleted photos.
                    for chunk in existing.chunks(500) {
                        let db = lock_db(&self.db);
                        for (id, path) in chunk {
                            if !seen.contains(path.as_str()) && !rebound.contains(id) {
                                match db.mark_missing_if_path(*id, path) {
                                    Ok(true) => {
                                        changed = true;
                                        missing_ids.push(*id);
                                    }
                                    Ok(false) => {}
                                    Err(error) => {
                                        tracing::warn!(
                                            %error,
                                            photo_id = id,
                                            "cannot mark missing photo"
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
                changed |= root_report.modified > 0;
                report.found += root_report.found;
                report.indexed += root_report.indexed;
                report.modified += root_report.modified;
                report.skipped += root_report.skipped;
                report.new_ids.extend(root_report.new_ids);
                report.rebound_ids.extend(root_report.rebound_ids);
                report.failures.extend(root_report.failures);
                if root_report.cancelled {
                    report.cancelled = true;
                    break;
                }
            }
            // Give every root a chance to rebind a moved file before falling
            // back to a compressed derivative for a missing original.
            for chunk in missing_ids.chunks(500) {
                let db = lock_db(&self.db);
                for photo_id in chunk {
                    if let Some(photo) = db.photo_by_id(*photo_id)?
                        && photo.status == PhotoStatus::Missing
                        && promote_display_derivative(&db, &photo)?
                    {
                        changed = true;
                    }
                }
            }
            if changed {
                let db = lock_db(&self.db);
                db.bump_library_revision()?;
            }
            if self.settings.auto_compress
                && !report.new_ids.is_empty()
                && (!initial_scan || self.settings.auto_compress_backlog)
            {
                let enqueued = {
                    let db = lock_db(&self.db);
                    enqueue_auto_jobs(
                        &db,
                        &report.new_ids,
                        self.settings.auto_compress_keep_original,
                        self.settings.auto_compress_immediate,
                    )?
                };
                auto_enqueued = enqueued;
                self.jobs.notify();
            }
            if !report.new_ids.is_empty() {
                let tagging_enqueued = {
                    let db = lock_db(&self.db);
                    enqueue_auto_tagging(&db, &report.new_ids, &self.settings, tagging_ready)?
                };
                if tagging_enqueued > 0 {
                    self.tagging.notify();
                }
            }
        }
        self.verify_replaced_assets()?;
        self.report_scan_outcome(&report, full_scan)?;
        if report.modified > 0 {
            self.link_unlinked()?;
        }
        if !report.new_ids.is_empty() {
            let enqueued = {
                let db = lock_db(&self.db);
                match discord::enqueue_auto_sends(&db, &report.new_ids) {
                    Ok(count) => count,
                    Err(error) => {
                        // Automatic sending is best effort: an outbox error
                        // must not fail or roll back the photo scan.
                        tracing::warn!(%error, "cannot enqueue automatic Discord sends");
                        0
                    }
                }
            };
            if enqueued > 0 {
                self.sends.notify();
            }
        }
        if let Some(tx) = &self.thumbnail_tx {
            for id in &report.new_ids {
                if let Err(error) = tx.send(ThumbnailRequest::Photo(*id)) {
                    tracing::debug!(
                        %error,
                        photo_id = *id,
                        "thumbnail worker is unavailable; dropping request"
                    );
                }
            }
        } else {
            // No thumbnail worker (notably `--once`): keep the ids so the
            // caller can generate directly instead of dropping them.
            self.deferred_thumbnail_ids
                .extend_from_slice(&report.new_ids);
        }
        Ok((report, auto_enqueued))
    }

    /// Records one `Scan` error summarizing the current failures, or clears a
    /// stale error when a full scan had none. A targeted scan (`ScanRoot`)
    /// without failures leaves any recorded error alone: it says nothing
    /// about the roots it did not scan.
    ///
    /// A watcher-start failure shares the `Scan` slot but is not a scan
    /// failure: a clean scan keeps it visible (or restores it after clearing
    /// a scan failure) until the watchers start successfully.
    pub(crate) fn report_scan_outcome(
        &mut self,
        report: &ScanReport,
        full_scan: bool,
    ) -> Result<()> {
        if report.cancelled {
            return Ok(());
        }
        if report.failures.is_empty() {
            if full_scan {
                if let Some((localized, detail)) = self.watcher_error.clone() {
                    if self.last_errors.get(&DaemonOperation::Scan)
                        == Some(&localized.signature(detail.as_deref()))
                    {
                        return Ok(());
                    }
                    self.clear_daemon_error(DaemonOperation::Scan)?;
                    return self.report_localized_error(
                        DaemonOperation::Scan,
                        localized,
                        detail.as_deref(),
                    );
                }
                return self.clear_daemon_error(DaemonOperation::Scan);
            }
            return Ok(());
        }
        let detail = report
            .failures
            .iter()
            .take(5)
            .map(|failure| format!("{}: {}", failure.path.display(), failure.message))
            .collect::<Vec<_>>()
            .join("\n");
        self.report_localized_error(
            DaemonOperation::Scan,
            LocalizedError::new("daemon.scan_failed").arg("count", report.failures.len()),
            Some(&detail),
        )
    }

    /// Closes sessions left open by a previous daemon, except the session that
    /// still belongs to the latest log while VRChat is running: that one is
    /// resumed by the log ingestor instead. Only the owner of the
    /// single-instance lock may repair sessions: a maintenance run must not
    /// close the live daemon's active session.
    pub(crate) fn make_unlinked_photos_due(&self) -> Result<usize> {
        let changed = {
            let db = lock_db(&self.db);
            db.make_unlinked_photos_due()?
        };
        if changed > 0 {
            tracing::debug!(
                photos = changed,
                "cleared link retry backoff for an explicit retry"
            );
        }
        Ok(changed)
    }

    pub(crate) fn link_unlinked(&mut self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let (photos, estimated_enabled) = {
            let db = lock_db(&self.db);
            (
                db.unlinked_photos_due(2000, now)?,
                linker::estimated_link_enabled(&db),
            )
        };
        let result =
            relink_photos_outside_lock(&self.db, photos, estimated_enabled, now, |photo| {
                metadata::read_xmp(Path::new(&photo.abs_path))
                    .ok()
                    .flatten()
            });
        match result {
            Ok(linked) => {
                if linked > 0 {
                    tracing::info!(linked, "linked photos to log context");
                    if self.world_name_resolution_enabled.load(Ordering::Relaxed) {
                        self.enqueue_world_name_backfill();
                        self.world_names.notify();
                    }
                }
                self.clear_daemon_error(DaemonOperation::Link)
            }
            Err(error) => {
                tracing::error!(%error, "cannot link photos to log context");
                self.report_localized_error(
                    DaemonOperation::Link,
                    LocalizedError::new("daemon.link_failed"),
                    Some(&error.to_string()),
                )?;
                Err(error)
            }
        }
    }

    /// Marks replaced photos missing when their compressed display file was
    /// deleted or moved away. The notify watcher intentionally ignores the
    /// `compressed` directory, so the daemon checks these paths periodically.
    pub(crate) fn verify_replaced_assets(&mut self) -> Result<()> {
        let rows = {
            let db = lock_db(&self.db);
            db.display_derivative_paths(100_000)?
        };
        // Stat every asset without the database lock so a slow or disconnected
        // volume cannot block scans, IPC, or the heartbeat.
        let missing =
            missing_display_derivatives(rows, |path| path.is_dir(), |path| path.is_file());
        if missing.is_empty() {
            return Ok(());
        }
        let db = lock_db(&self.db);
        db.in_transaction(|db| {
            for photo_id in missing {
                db.mark_missing(photo_id)?;
                db.clear_display_derivative(photo_id)?;
            }
            db.bump_library_revision()?;
            Ok(())
        })?;
        Ok(())
    }
}
