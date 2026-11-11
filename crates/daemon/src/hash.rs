use super::*;

/// Retries a pending library-revision bump and reports whether it is clear.
///
/// A failure keeps `pending` set so the caller can retry on its next idle
/// cycle instead of silently dropping the UI refresh.
pub(crate) fn retry_revision<F>(pending: &mut bool, mut bump: F) -> bool
where
    F: FnMut() -> Result<()>,
{
    if !*pending {
        return true;
    }
    match bump() {
        Ok(()) => {
            *pending = false;
            true
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "cannot bump library revision after hashing; will retry"
            );
            false
        }
    }
}

/// Fills `photos.blake3` for rows indexed before hashing was introduced.
/// The worker is deliberately low-frequency so a large first run does not
/// starve compression or watcher work.
pub(crate) fn hash_loop(db: Arc<Mutex<Db>>, running: Arc<AtomicBool>, paused: Arc<AtomicBool>) {
    let mut hashed_since_idle = false;
    let mut revision_pending = false;
    while running.load(Ordering::Relaxed) {
        if paused.load(Ordering::Relaxed) {
            thread::sleep(PAUSED_WORKER_POLL);
            continue;
        }
        let rows = {
            let db = lock_db(&db);
            match db.photos_without_hash(16) {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(%error, "cannot load photos for hashing");
                    Vec::new()
                }
            }
        };
        if rows.is_empty() {
            if hashed_since_idle {
                revision_pending = true;
                hashed_since_idle = false;
            }
            if revision_pending {
                let db = lock_db(&db);
                retry_revision(&mut revision_pending, || {
                    db.bump_library_revision()?;
                    Ok(())
                });
                drop(db);
            }
            thread::sleep(Duration::from_secs(2));
            continue;
        }
        let mut progressed = false;
        for (photo_id, path) in rows {
            if !running.load(Ordering::Relaxed) {
                break;
            }
            match hashing::file_blake3(Path::new(&path)) {
                Ok(Some(hash)) => {
                    let db = lock_db(&db);
                    if let Err(error) = db.set_photo_blake3(photo_id, &hash) {
                        tracing::warn!(%error, photo_id, "cannot store photo hash");
                    } else {
                        hashed_since_idle = true;
                        progressed = true;
                    }
                }
                Ok(None) => tracing::debug!(photo_id, path, "photo disappeared before hashing"),
                Err(error) => tracing::warn!(%error, photo_id, path, "cannot hash photo"),
            }
        }
        thread::sleep(if progressed {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(2)
        });
    }
}
