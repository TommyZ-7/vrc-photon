use super::*;

pub(crate) fn enqueue_auto_jobs(
    db: &Db,
    new_ids: &[i64],
    keep_original: bool,
    immediate: bool,
) -> Result<usize> {
    let profiles = db.list_profiles()?;
    let auto_profiles: Vec<&CompressionProfile> = profiles
        .iter()
        .filter(|profile| profile.enabled && profile.auto_enabled)
        .collect();
    if auto_profiles.is_empty() {
        return Ok(0);
    }
    let policy = if keep_original { "keep" } else { "replace" };
    let status = if immediate {
        QueueStatus::Pending
    } else {
        QueueStatus::Waiting
    };
    let mut enqueued = 0;
    for photo_id in new_ids {
        for profile in &auto_profiles {
            if db.enqueue_auto_job_for_existing_profile_with_status(
                *photo_id,
                &profile.id,
                policy,
                status,
            )? {
                enqueued += 1;
            }
        }
    }
    tracing::info!(
        photos = new_ids.len(),
        profiles = auto_profiles.len(),
        jobs = enqueued,
        "auto compression jobs enqueued"
    );
    Ok(enqueued)
}

/// Enqueues pending AI tagging jobs for newly indexed photos. Automatic
/// tagging is opt-in twice (enabled and auto) and requires a ready plugin;
/// everything else is left for the manual and backlog paths.
pub(crate) fn enqueue_auto_tagging(
    db: &Db,
    new_ids: &[i64],
    settings: &AppSettings,
    plugin_ready: bool,
) -> Result<usize> {
    if new_ids.is_empty()
        || !settings.ai_tagging_enabled
        || !settings.ai_tagging_auto
        || !plugin_ready
    {
        return Ok(0);
    }
    let enqueued = db.enqueue_tagging_jobs(new_ids, AI_PLUGIN_ID, TaggingJobStatus::Pending)?;
    tracing::info!(
        photos = new_ids.len(),
        jobs = enqueued,
        "automatic AI tagging jobs enqueued"
    );
    Ok(enqueued)
}

/// Keeps a photo visible when its original file was deleted manually but a
/// completed compression output remains. Uses the same deterministic profile
/// preference as replacement jobs.
pub(crate) fn promote_display_derivative(db: &Db, photo: &Photo) -> Result<bool> {
    let derivatives = db.derivatives_for_photo(photo.id)?;
    let profiles = db.list_profiles()?;
    let Some(preferred) = preferred_display_derivative(&derivatives, &profiles) else {
        return Ok(false);
    };
    Ok(db.set_replaced(photo.id, preferred.id)?)
}

/// Picks the derivative to show for a photo. Profiles are ordered by
/// `sort_order`, so the first profile in `list_profiles()` wins. Unknown
/// profiles rank below known ones and fall back to the larger derivative id.
/// Only files that still exist are candidates.
pub(crate) fn preferred_display_derivative<'a>(
    derivatives: &'a [Derivative],
    profiles: &[CompressionProfile],
) -> Option<&'a Derivative> {
    derivatives
        .iter()
        .filter(|derivative| Path::new(&derivative.path).is_file())
        .max_by_key(|derivative| {
            (
                profiles
                    .iter()
                    .position(|profile| profile.id == derivative.profile_id)
                    .map(std::cmp::Reverse),
                derivative.id,
            )
        })
}

/// Tracks the cache size between scans. `known_bytes` is `None` while the
/// cache has not been measured yet, which forces the first prune decision to
/// scan even when nothing was generated.
pub(crate) fn worker_loop(
    db: Arc<Mutex<Db>>,
    running: Arc<AtomicBool>,
    jobs: Arc<JobSignal>,
    events: EventBus,
    thumbnail_dir: PathBuf,
    paused: Arc<AtomicBool>,
    budget: Arc<MemoryBudget>,
) {
    let mut seen_generation = 0;
    let mut last_poll_error: Option<String> = None;
    while running.load(Ordering::Relaxed) {
        if paused.load(Ordering::Relaxed) {
            jobs.wait(&mut seen_generation, PAUSED_WORKER_POLL);
            continue;
        }
        let batch = {
            let db = lock_db(&db);
            match db.next_pending_photo_jobs() {
                Ok(batch) => {
                    last_poll_error = None;
                    batch
                }
                Err(error) => {
                    // The loop polls every 400 ms; warn once per distinct
                    // failure instead of flooding the log while it persists.
                    let message = error.to_string();
                    if last_poll_error.as_deref() != Some(&message) {
                        tracing::warn!(%error, "cannot load pending compression jobs");
                        last_poll_error = Some(message);
                    }
                    Vec::new()
                }
            }
        };
        if batch.is_empty() {
            // Poll first so a batch keeps all workers busy; the signal only
            // saves the 400 ms wait when the queue was empty.
            jobs.wait(&mut seen_generation, Duration::from_millis(400));
            continue;
        }
        for job in &batch {
            publish_compression(&events, job, JobStatus::Processing, 0.05, None);
        }
        for outcome in process_photo_batch(&db, &batch, Some(&thumbnail_dir), &budget) {
            match outcome.status {
                QueueStatus::Completed => {
                    publish_compression(&events, &outcome.job, JobStatus::Completed, 1.0, None)
                }
                QueueStatus::Cancelled => {
                    publish_compression(&events, &outcome.job, JobStatus::Cancelled, 0.0, None)
                }
                _ => publish_compression(
                    &events,
                    &outcome.job,
                    JobStatus::Failed,
                    0.0,
                    outcome.error,
                ),
            }
        }
    }
}

pub(crate) fn publish_compression(
    events: &EventBus,
    job: &CompressionJob,
    status: JobStatus,
    progress: f64,
    error: Option<String>,
) {
    events.publish(Event::CompressionProgress {
        job_id: job.id.clone(),
        photo_id: job.photo_id,
        status,
        progress,
        error,
    });
}

pub(crate) struct JobOutcome {
    pub(crate) job: CompressionJob,
    pub(crate) status: QueueStatus,
    pub(crate) error: Option<String>,
}

/// Processes every job for one photo after decoding the source once.
///
/// The source path is frozen before the first job, so a `replace` job cannot
/// feed its derivative into a later job in the same batch. Individual job
/// failures are recorded and do not stop the remaining jobs. A batch-level
/// failure only marks jobs that are still pending/processing as failed, so
/// completed and cancelled rows are never rewritten.
pub(crate) fn process_photo_batch(
    db: &Arc<Mutex<Db>>,
    jobs: &[CompressionJob],
    thumbnail_dir: Option<&Path>,
    budget: &MemoryBudget,
) -> Vec<JobOutcome> {
    match process_photo_batch_inner(db, jobs, thumbnail_dir, budget) {
        Ok(outcomes) => outcomes,
        Err(error) => {
            let message = error.to_string();
            tracing::error!(%error, "compression batch failed");
            fail_claimed_jobs(db, jobs, &message)
        }
    }
}

pub(crate) fn fail_claimed_jobs(
    db: &Arc<Mutex<Db>>,
    jobs: &[CompressionJob],
    message: &str,
) -> Vec<JobOutcome> {
    let mut outcomes = Vec::with_capacity(jobs.len());
    for job in jobs {
        let db = lock_db(db);
        let status = match db.fail_job_if_active(&job.id, message) {
            Ok(status) => status.unwrap_or(QueueStatus::Failed),
            Err(error) => {
                tracing::error!(
                    job = %job.id,
                    %error,
                    "cannot record failed compression job"
                );
                QueueStatus::Failed
            }
        };
        drop(db);
        outcomes.push(JobOutcome {
            job: job.clone(),
            error: (status == QueueStatus::Failed).then(|| message.to_string()),
            status,
        });
    }
    outcomes
}

pub(crate) fn process_photo_batch_inner(
    db: &Arc<Mutex<Db>>,
    jobs: &[CompressionJob],
    thumbnail_dir: Option<&Path>,
    budget: &MemoryBudget,
) -> Result<Vec<JobOutcome>> {
    let Some(first) = jobs.first() else {
        return Ok(Vec::new());
    };
    debug_assert!(jobs.iter().all(|job| job.photo_id == first.photo_id));

    let photo = {
        let db = lock_db(db);
        db.photo_by_id(first.photo_id)?
    };
    let Some(photo) = photo else {
        return fail_batch(db, jobs, "photo no longer exists");
    };
    let source_path = if Path::new(&photo.abs_path).is_file() {
        PathBuf::from(&photo.abs_path)
    } else {
        PathBuf::from(photo.effective_path())
    };
    if photo.blake3.is_none()
        && source_path == Path::new(&photo.abs_path)
        && let Ok(Some(hash)) = hashing::file_blake3(&source_path)
    {
        lock_db(db).set_photo_blake3(photo.id, &hash)?;
    }

    // Hold the shared reservation for the whole batch: the decoded image is
    // reused by every profile while resized copies and encoder output are
    // created one after another.
    let _memory_permit = budget.reserve(compression_peak_estimate(&source_path));
    let Some(image) = vrc_photon_core::image_decode::decode_image(&source_path) else {
        return fail_batch(
            db,
            jobs,
            &format!("cannot decode {}", source_path.display()),
        );
    };

    let mut outcomes = Vec::with_capacity(jobs.len());
    for (index, job) in jobs.iter().enumerate() {
        if index > 0 && lock_db(db).photo_has_forced_deletion(photo.id)? {
            for remaining in &jobs[index..] {
                lock_db(db).update_job(&remaining.id, QueueStatus::Cancelled, 0.0, None, None)?;
                outcomes.push(JobOutcome {
                    job: remaining.clone(),
                    status: QueueStatus::Cancelled,
                    error: None,
                });
            }
            break;
        }
        lock_db(db).update_job(&job.id, QueueStatus::Processing, 0.05, None, None)?;
        match process_decoded_job(db, &photo, &source_path, &image, job, thumbnail_dir) {
            Ok(()) => outcomes.push(JobOutcome {
                job: job.clone(),
                status: QueueStatus::Completed,
                error: None,
            }),
            Err(error) => {
                let message = error.to_string();
                tracing::error!(job = %job.id, %error, "compression failed");
                if let Err(update_error) =
                    lock_db(db).update_job(&job.id, QueueStatus::Failed, 0.0, Some(&message), None)
                {
                    tracing::error!(
                        job = %job.id,
                        %update_error,
                        "cannot record failed compression job"
                    );
                }
                outcomes.push(JobOutcome {
                    job: job.clone(),
                    status: QueueStatus::Failed,
                    error: Some(message),
                });
            }
        }
    }
    Ok(outcomes)
}

pub(crate) fn fail_batch(
    db: &Arc<Mutex<Db>>,
    jobs: &[CompressionJob],
    error: &str,
) -> Result<Vec<JobOutcome>> {
    let mut outcomes = Vec::with_capacity(jobs.len());
    for job in jobs {
        tracing::error!(job = %job.id, %error, "compression failed");
        lock_db(db).update_job(&job.id, QueueStatus::Failed, 0.0, Some(error), None)?;
        outcomes.push(JobOutcome {
            job: job.clone(),
            status: QueueStatus::Failed,
            error: Some(error.to_string()),
        });
    }
    Ok(outcomes)
}

pub(crate) fn process_decoded_job(
    db: &Arc<Mutex<Db>>,
    photo: &Photo,
    source_path: &Path,
    image: &DynamicImage,
    job: &CompressionJob,
    thumbnail_dir: Option<&Path>,
) -> Result<()> {
    let mut profile = {
        let db = lock_db(db);
        db.list_profiles()?
            .into_iter()
            .find(|profile| profile.id == job.profile_id)
            .context("compression profile no longer exists")?
    };
    if let Some(output_dir) = job
        .output_dir
        .as_deref()
        .filter(|dir| !dir.trim().is_empty())
    {
        profile.output_dir = Some(output_dir.to_string());
    }

    let result = compressor::compress_decoded(image, source_path, &profile)?;
    if !result.output_path.is_file()
        || std::fs::metadata(&result.output_path)
            .map(|meta| meta.len() == 0)
            .unwrap_or(true)
    {
        anyhow::bail!("compressed output is missing or empty");
    }
    let preferred_derivative = {
        let db = lock_db(db);
        db.add_derivative(
            photo.id,
            &result.output_path.to_string_lossy(),
            result.format.as_str(),
            result.width,
            result.height,
            result.compressed_size,
            &profile.id,
            &job.policy,
        )?;
        let current_status = db.photo_by_id(photo.id)?.map(|row| row.status);
        if job.policy == "replace" && current_status != Some(PhotoStatus::Deleting) {
            let derivatives = db.derivatives_for_photo(photo.id)?;
            let profiles = db.list_profiles()?;
            Some(
                preferred_display_derivative(&derivatives, &profiles)
                    .context("no displayable derivative after compression")?
                    .id,
            )
        } else {
            None
        }
    };
    if let Some(preferred_id) = preferred_derivative {
        // The displayed file is about to change. Drop every cached thumbnail
        // for this photo before publishing `replaced`, so the next display
        // request regenerates from the derivative instead of showing the old
        // original. The file removal runs without the database lock; failures
        // are non-fatal because the source stamp still invalidates a stale
        // entry on the next read.
        if let Some(cache_dir) = thumbnail_dir
            && let Err(error) = thumbnail::remove_thumbnail_files_in(cache_dir, photo.id)
        {
            tracing::warn!(photo_id = photo.id, %error, "cannot invalidate thumbnail cache");
        }
        let db = lock_db(db);
        db.complete_replacement(photo.id, preferred_id)?;
        drop(db);
    }
    let db = lock_db(db);
    db.update_job(
        &job.id,
        QueueStatus::Completed,
        1.0,
        None,
        Some(&result.output_path.to_string_lossy()),
    )?;
    if let Err(error) = db.resume_blocked_deletions_for_photo(photo.id) {
        tracing::warn!(photo_id = photo.id, %error, "cannot resume blocked deletion");
    }
    db.bump_library_revision()?;
    tracing::info!(
        photo = %source_path.display(),
        output = %result.output_path.display(),
        saved = result.compressed_size,
        "compressed"
    );
    Ok(())
}

#[cfg(test)]
pub(crate) fn process_job(db: &Arc<Mutex<Db>>, job: &CompressionJob) -> Result<()> {
    process_job_with_thumbnail_dir(db, job, None)
}

#[cfg(test)]
pub(crate) fn process_job_with_thumbnail_dir(
    db: &Arc<Mutex<Db>>,
    job: &CompressionJob,
    thumbnail_dir: Option<&Path>,
) -> Result<()> {
    let budget = MemoryBudget::new(COMPRESSION_MEMORY_BUDGET_BYTES);
    let mut outcomes = process_photo_batch(db, std::slice::from_ref(job), thumbnail_dir, &budget);
    let outcome = outcomes.pop().context("compression produced no outcome")?;
    match outcome.status {
        QueueStatus::Completed => Ok(()),
        _ => Err(anyhow::anyhow!(
            outcome
                .error
                .unwrap_or_else(|| "compression failed".to_string())
        )),
    }
}

pub(crate) fn modified_utc(path: &Path) -> Option<i64> {
    path.metadata()
        .ok()?
        .modified()
        .ok()
        .map(vrc_photon_core::time::from_system_time)
        .map(|stamp| stamp.utc)
}
