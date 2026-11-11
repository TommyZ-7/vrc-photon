use super::compression::preferred_display_derivative;
use super::*;

pub(crate) fn deletion_loop(
    db: Arc<Mutex<Db>>,
    running: Arc<AtomicBool>,
    jobs: Arc<JobSignal>,
    paused: Arc<AtomicBool>,
) {
    let mut seen_generation = 0;
    while running.load(Ordering::Relaxed) {
        if paused.load(Ordering::Relaxed) {
            jobs.wait(&mut seen_generation, PAUSED_WORKER_POLL);
            continue;
        }
        if let Err(error) = run_deletions_once(&db) {
            tracing::error!(%error, "deletion executor failed");
        }
        jobs.wait(&mut seen_generation, Duration::from_millis(500));
    }
}

/// A forced request may be recorded directly with `force = 1` instead of
/// through `force_deletion`, which cancels tagging jobs itself. Requests with
/// active tagging jobs are hidden from `pending_deletion_requests`, so cancel
/// those jobs first in one bulk statement; the late sidecar result is rejected
/// by the cancelled job state. `claim_deletion` then only waits for runs that
/// were not forced.
pub(crate) fn cancel_tagging_for_forced_deletions(db: &Arc<Mutex<Db>>) -> Result<()> {
    let cancelled = {
        let db = lock_db(db);
        db.cancel_tagging_for_forced_deletions()?
    };
    if cancelled > 0 {
        tracing::info!(cancelled, "cancelled tagging jobs for forced deletions");
    }
    Ok(())
}

/// Claims and executes every pending deletion whose jobs are terminal. A
/// dependency failure blocks the request instead of busy-polling it; a file
/// failure blocks it until the user retries or forces.
pub(crate) fn run_deletions_once(db: &Arc<Mutex<Db>>) -> Result<usize> {
    cancel_tagging_for_forced_deletions(db)?;
    let requests = {
        let db = lock_db(db);
        db.pending_deletion_requests(100)?
    };
    let mut executed = 0;
    for pending in requests {
        if pending.reason == DeletionReason::Unknown {
            // The snapshot may be stale: the UI can upgrade the request to
            // manual after collection. Only block when the stored reason is
            // still unknown.
            let blocked = {
                let db = lock_db(db);
                db.block_deletion_if_unknown(pending.id)?
            };
            if blocked {
                tracing::warn!(
                    request = pending.id,
                    photo_id = pending.photo_id,
                    "unknown deletion reason; request blocked"
                );
            }
            continue;
        }
        let blocked = {
            let db = lock_db(db);
            db.block_deletion_if_jobs_failed(pending.id)?
        };
        if blocked {
            continue;
        }
        let claimed = {
            let db = lock_db(db);
            db.claim_deletion(pending.id)?
        };
        if let Some(request) = claimed {
            match execute_deletion(db, &request) {
                Ok(()) => executed += 1,
                Err(error) => tracing::error!(
                    request = request.id,
                    photo_id = request.photo_id,
                    %error,
                    "deletion failed"
                ),
            }
        }
    }
    Ok(executed)
}

pub(crate) fn execute_deletion(db: &Arc<Mutex<Db>>, request: &DeletionRequest) -> Result<()> {
    if request.reason == DeletionReason::Unknown {
        anyhow::bail!("unknown deletion reason");
    }
    let (photo, targets) = {
        let db = lock_db(db);
        let Some(photo) = db.photo_by_id(request.photo_id)? else {
            db.complete_deletion(request.id)?;
            db.bump_library_revision()?;
            return Ok(());
        };
        let derivatives = db.derivatives_for_photo(photo.id)?;
        if request.reason == DeletionReason::AutoReplace && !request.force {
            let profiles = db.list_profiles()?;
            let Some(valid) = preferred_display_derivative(&derivatives, &profiles) else {
                db.block_deletion(request.id, "missing_asset", None)?;
                return Err(anyhow::anyhow!(
                    "replacement derivative is missing; refusing to delete the original"
                ));
            };
            if photo.status != PhotoStatus::Replaced
                || photo.display_derivative_id != Some(valid.id)
            {
                db.set_replaced(photo.id, valid.id)?;
            }
        }
        let mut targets = Vec::new();
        let mut seen = HashSet::new();
        if request.reason == DeletionReason::Manual {
            for derivative in derivatives {
                if seen.insert(derivative.path.clone()) {
                    targets.push(PathBuf::from(derivative.path));
                }
            }
        }
        if seen.insert(photo.abs_path.clone()) {
            targets.push(PathBuf::from(&photo.abs_path));
        }
        (photo, targets)
    };

    for path in targets {
        if !path.is_file() {
            continue;
        }
        if let Err(error) = trash::delete(&path)
            && path.exists()
        {
            let db = lock_db(db);
            db.block_deletion(request.id, "file_error", Some(&error.to_string()))?;
            return Err(anyhow::anyhow!(
                "cannot move {} to trash: {error}",
                path.display()
            ));
        }
    }

    {
        let db = lock_db(db);
        if request.reason == DeletionReason::Manual {
            db.complete_manual_deletion(photo.id)?;
        }
        db.complete_deletion(request.id)?;
        db.bump_library_revision()?;
    }
    tracing::info!(
        request = request.id,
        photo_id = photo.id,
        reason = request.reason.as_str(),
        "deletion completed"
    );
    Ok(())
}
