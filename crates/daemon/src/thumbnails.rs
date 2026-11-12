use super::*;

pub(crate) struct ThumbnailCacheBudget {
    pub(crate) limit_bytes: u64,
    pub(crate) known_bytes: Option<u64>,
}

impl ThumbnailCacheBudget {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            limit_bytes: limit,
            known_bytes: None,
        }
    }

    pub(crate) fn set_limit(&mut self, limit: u64) {
        if limit == self.limit_bytes {
            return;
        }
        self.limit_bytes = limit;
        self.known_bytes = None;
    }

    pub(crate) fn note_generated(&mut self, bytes: u64) {
        self.add(bytes);
    }

    pub(crate) fn note_external(&mut self, bytes: u64) {
        self.add(bytes);
    }

    pub(crate) fn note_pruned(&mut self, report: &thumbnail::ThumbnailPruneReport) {
        self.known_bytes = (self.limit_bytes > 0).then_some(report.remaining_bytes);
    }

    /// The cache was cleared behind the worker's back; rescan before the next
    /// prune decision instead of trusting the previous byte count.
    pub(crate) fn note_cleared(&mut self) {
        self.known_bytes = None;
    }

    pub(crate) fn needs_prune(&self) -> bool {
        self.limit_bytes > 0
            && self
                .known_bytes
                .is_none_or(|known| known > self.limit_bytes)
    }

    pub(crate) fn add(&mut self, bytes: u64) {
        if let Some(known) = self.known_bytes {
            self.known_bytes = Some(known.saturating_add(bytes));
        }
    }
}

pub(crate) struct ThumbnailPruneSchedule {
    batch: usize,
    generated: usize,
}

impl ThumbnailPruneSchedule {
    pub(crate) fn new(batch: usize) -> Self {
        Self {
            batch,
            generated: 0,
        }
    }

    pub(crate) fn note_generated(&mut self) -> bool {
        self.generated += 1;
        if self.generated >= self.batch {
            self.note_pruned();
            true
        } else {
            false
        }
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.generated > 0
    }

    pub(crate) fn note_pruned(&mut self) {
        self.generated = 0;
    }
}

pub(crate) fn thumbnail_cache_limit_bytes(megabytes: u64) -> u64 {
    megabytes.saturating_mul(1024 * 1024)
}

pub(crate) fn pack_thumbnail_spec(spec: thumbnail::ThumbnailSpec) -> u32 {
    spec.size
}

pub(crate) fn unpack_thumbnail_spec(packed: u32) -> thumbnail::ThumbnailSpec {
    thumbnail::ThumbnailSpec::new(packed)
}

/// Prunes the cache when a limit is set. Returns the measurement from the
/// scan so the worker can update its budget; `None` means the cache was not
/// scanned (unlimited) or the scan failed.
pub(crate) fn prune_thumbnail_cache(
    cache_dir: &Path,
    limit_bytes: u64,
) -> Option<thumbnail::ThumbnailPruneReport> {
    if limit_bytes == 0 {
        return None;
    }
    match thumbnail::prune_thumbnails_in(cache_dir, limit_bytes) {
        Ok(report) => {
            if report.removed_files > 0 {
                tracing::info!(
                    removed_files = report.removed_files,
                    removed_bytes = report.removed_bytes,
                    remaining_bytes = report.remaining_bytes,
                    limit_bytes,
                    dir = %cache_dir.display(),
                    "pruned thumbnail cache"
                );
            }
            Some(report)
        }
        Err(error) => {
            tracing::warn!(
                %error,
                dir = %cache_dir.display(),
                "cannot prune thumbnail cache"
            );
            None
        }
    }
}

/// Reloads the live cache limit immediately before a prune decision. A limit
/// of zero always wins, so a pending prune cannot delete files after the user
/// selected unlimited while the worker was blocked or generating.
pub(crate) fn refresh_thumbnail_budget_and_prune(
    cache_dir: &Path,
    cache_limit: &AtomicU64,
    budget: &mut ThumbnailCacheBudget,
) {
    let limit = cache_limit.load(Ordering::Relaxed);
    budget.set_limit(limit);
    if limit == 0 || !budget.needs_prune() {
        return;
    }
    if let Some(report) = prune_thumbnail_cache(cache_dir, limit) {
        budget.note_pruned(&report);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn thumbnail_loop(
    db: Arc<Mutex<Db>>,
    running: Arc<AtomicBool>,
    rx: &Receiver<i64>,
    cache_dir: PathBuf,
    cache_limit: Arc<AtomicU64>,
    spec: Arc<AtomicU32>,
    cache_epoch: Arc<AtomicU64>,
    external_bytes: Arc<AtomicU64>,
) {
    let mut budget = ThumbnailCacheBudget::new(cache_limit.load(Ordering::Relaxed));
    let mut schedule = ThumbnailPruneSchedule::new(THUMBNAIL_PRUNE_BATCH);
    let mut seen_epoch = cache_epoch.load(Ordering::Relaxed);
    // Notifications that arrived before this worker started describe a cache
    // that the startup scan is about to measure anyway.
    external_bytes.swap(0, Ordering::Relaxed);
    refresh_thumbnail_budget_and_prune(&cache_dir, &cache_limit, &mut budget);

    while running.load(Ordering::Relaxed) {
        let epoch = cache_epoch.load(Ordering::Relaxed);
        if epoch != seen_epoch {
            seen_epoch = epoch;
            external_bytes.swap(0, Ordering::Relaxed);
            budget.note_cleared();
            schedule.note_pruned();
            refresh_thumbnail_budget_and_prune(&cache_dir, &cache_limit, &mut budget);
        }
        let limit = cache_limit.load(Ordering::Relaxed);
        if limit != budget.limit_bytes {
            schedule.note_pruned();
            refresh_thumbnail_budget_and_prune(&cache_dir, &cache_limit, &mut budget);
        }
        match rx.recv_timeout(THUMBNAIL_PRUNE_IDLE) {
            Ok(id) => {
                let external = external_bytes.swap(0, Ordering::Relaxed);
                budget.note_external(external);
                let path = {
                    let db = lock_db(&db);
                    match db.photo_effective_path(id) {
                        Ok(path) => path,
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                photo_id = id,
                                "cannot load the photo path for thumbnail"
                            );
                            None
                        }
                    }
                };
                let mut batch_due = false;
                if let Some(path) = path {
                    let thumbnail_spec = unpack_thumbnail_spec(spec.load(Ordering::Relaxed));
                    match thumbnail::ensure_thumbnail_in_with_status(
                        id,
                        Path::new(&path),
                        &cache_dir,
                        thumbnail_spec,
                    ) {
                        Ok(Some(ensured)) => {
                            if ensured.created {
                                budget.note_generated(ensured.bytes);
                                batch_due = schedule.note_generated();
                            }
                        }
                        Ok(None) => tracing::debug!(
                            photo_id = id,
                            path = %path,
                            "thumbnail source is missing"
                        ),
                        Err(error) => tracing::warn!(
                            %error,
                            photo_id = id,
                            path = %path,
                            "cannot generate thumbnail"
                        ),
                    }
                }
                // The notice may describe a thumbnail the UI already wrote, so
                // it must prune even when this id generated nothing (or was
                // unknown). The helper rechecks the live limit first.
                if batch_due || external > 0 {
                    refresh_thumbnail_budget_and_prune(&cache_dir, &cache_limit, &mut budget);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let external = external_bytes.swap(0, Ordering::Relaxed);
                budget.note_external(external);
                if schedule.has_pending() || external > 0 {
                    schedule.note_pruned();
                    refresh_thumbnail_budget_and_prune(&cache_dir, &cache_limit, &mut budget);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}
