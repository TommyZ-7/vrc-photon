use super::*;

impl Db {
    // ---------- deletion requests ----------

    /// Records one durable deletion request per photo. Manual requests hide the
    /// photo immediately; auto-replace requests leave `replaced` visible until
    /// the executor removes only the original file. Queued and failed sends are
    /// cancelled in the same transaction: they carry no data-loss risk, while a
    /// `processing` upload is left to finish.
    pub fn request_deletion(
        &self,
        photo_ids: &[i64],
        reason: DeletionReason,
        force: bool,
    ) -> Result<Vec<i64>> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let request_ids = self.request_deletion_in(&tx, photo_ids, reason, force)?;
        tx.commit()?;
        Ok(request_ids)
    }

    pub(super) fn request_deletion_in(
        &self,
        tx: &Transaction<'_>,
        photo_ids: &[i64],
        reason: DeletionReason,
        force: bool,
    ) -> Result<Vec<i64>> {
        if reason == DeletionReason::Unknown {
            return Err(CoreError::Invalid(
                "cannot record an unknown deletion reason".to_string(),
            ));
        }
        let now = now_string();
        let mut request_ids = Vec::new();
        for photo_id in photo_ids {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM photos WHERE id = ?1)",
                params![photo_id],
                |row| row.get(0),
            )?;
            if !exists {
                continue;
            }

            let active = tx
                .query_row(
                    "SELECT id, status, reason, force FROM deletion_requests \
                     WHERE photo_id = ?1 AND status IN ('pending', 'blocked', 'running')",
                    params![photo_id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)? != 0,
                        ))
                    },
                )
                .optional()?;

            let request_id = if let Some((id, status, current_reason, current_force)) = active {
                if status == "running" {
                    return Err(CoreError::Invalid(
                        "deletion is already running".to_string(),
                    ));
                }
                let next_reason = if reason == DeletionReason::Manual {
                    "manual"
                } else {
                    current_reason.as_str()
                };
                let next_force = current_force || force;
                let next_status = if next_force && status == "blocked" {
                    "pending"
                } else {
                    status.as_str()
                };
                tx.execute(
                    "UPDATE deletion_requests SET reason = ?1, force = ?2, status = ?3 \
                     WHERE id = ?4",
                    params![next_reason, next_force as i64, next_status, id],
                )?;
                id
            } else {
                tx.execute(
                    "INSERT INTO deletion_requests \
                        (photo_id, reason, status, force, requested_at) \
                     VALUES (?1, ?2, 'pending', ?3, ?4)",
                    params![photo_id, reason.as_str(), force as i64, now],
                )?;
                tx.last_insert_rowid()
            };

            if reason == DeletionReason::Manual {
                tx.execute(
                    "UPDATE photos SET status = 'deleting', updated_at = ?1 WHERE id = ?2",
                    params![now, photo_id],
                )?;
            }
            tx.execute(
                "UPDATE send_outbox SET status = 'cancelled', finished_at = ?1 \
                 WHERE photo_id = ?2 AND status IN ('waiting', 'pending', 'failed')",
                params![now, photo_id],
            )?;
            request_ids.push(request_id);
        }
        Ok(request_ids)
    }

    pub fn active_deletion_request(&self, photo_id: i64) -> Result<Option<DeletionRequest>> {
        let sql = "SELECT dr.id, dr.photo_id, p.file_name, dr.reason, dr.status, dr.force, \
                dr.blocked_reason, dr.error, dr.requested_at, dr.started_at, dr.finished_at \
             FROM deletion_requests dr JOIN photos p ON p.id = dr.photo_id \
             WHERE dr.photo_id = ?1 AND dr.status IN ('pending', 'blocked', 'running') \
             ORDER BY dr.id DESC LIMIT 1";
        let mut stmt = self.conn.prepare_cached(sql)?;
        Ok(stmt
            .query_row(params![photo_id], row_to_deletion_request)
            .optional()?)
    }

    pub fn active_deletion_requests(&self, limit: i64) -> Result<Vec<DeletionRequest>> {
        let sql = "SELECT dr.id, dr.photo_id, p.file_name, dr.reason, dr.status, dr.force, \
                dr.blocked_reason, dr.error, dr.requested_at, dr.started_at, dr.finished_at \
             FROM deletion_requests dr JOIN photos p ON p.id = dr.photo_id \
             WHERE dr.status IN ('pending', 'blocked', 'running') \
             ORDER BY dr.requested_at, dr.id LIMIT ?1";
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt.query_map(params![clamp_limit(limit)], row_to_deletion_request)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// True when a forced deletion for the photo is waiting to run. The
    /// compression batch uses this between jobs to cancel the remainder.
    pub fn photo_has_forced_deletion(&self, photo_id: i64) -> Result<bool> {
        let exists: i64 = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM deletion_requests \
             WHERE photo_id = ?1 AND force = 1 AND status IN ('pending', 'blocked'))",
            params![photo_id],
            |row| row.get(0),
        )?;
        Ok(exists != 0)
    }

    pub fn deletion_counts(&self) -> Result<DeletionCounts> {
        let (pending, blocked, running): (i64, i64, i64) = self.conn.query_row(
            "SELECT COALESCE(SUM(status = 'pending'), 0), \
                    COALESCE(SUM(status = 'blocked'), 0), \
                    COALESCE(SUM(status = 'running'), 0) \
             FROM deletion_requests",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok(DeletionCounts {
            pending,
            blocked,
            running,
        })
    }

    /// Counts compression, send, and tagging work per status for the warning
    /// modal. Unknown compression statuses are reported as `unknown` and keep
    /// the photo affected: a status written by a newer version is never
    /// terminal. Unknown send and tagging statuses also count as pending.
    /// `affected_photos` is the union of distinct selected photos with
    /// non-terminal jobs, so a photo with several kinds of work counts once.
    pub fn deletion_job_summary(&self, photo_ids: &[i64]) -> Result<DeletionJobSummary> {
        let mut compression = JobStatusCounts::default();
        let mut send = JobStatusCounts::default();
        let mut tagging = JobStatusCounts::default();
        let mut affected_photos = 0;
        for chunk in photo_ids.chunks(500) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT status, COUNT(*) FROM compression_jobs \
                 WHERE photo_id IN ({placeholders}) GROUP BY status"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter().copied()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (status, count) = row?;
                match QueueStatus::parse(&status) {
                    QueueStatus::Waiting => compression.waiting += count,
                    QueueStatus::Pending => compression.pending += count,
                    QueueStatus::Processing => compression.processing += count,
                    QueueStatus::Failed => compression.failed += count,
                    QueueStatus::Unknown => compression.unknown += count,
                    QueueStatus::Completed | QueueStatus::Cancelled => {}
                }
            }

            let sql = format!(
                "SELECT status, COUNT(*) FROM send_outbox \
                 WHERE photo_id IN ({placeholders}) GROUP BY status"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter().copied()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (status, count) = row?;
                match SendStatus::parse(&status) {
                    SendStatus::Waiting => send.waiting += count,
                    SendStatus::Pending => send.pending += count,
                    SendStatus::Processing => send.processing += count,
                    SendStatus::Failed => send.failed += count,
                    SendStatus::Unknown => send.pending += count,
                    SendStatus::Sent | SendStatus::Cancelled => {}
                }
            }

            let sql = format!(
                "SELECT status, COUNT(*) FROM tagging_jobs \
                 WHERE photo_id IN ({placeholders}) GROUP BY status"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter().copied()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (status, count) = row?;
                match TaggingJobStatus::parse(&status) {
                    Some(TaggingJobStatus::Waiting) => tagging.waiting += count,
                    Some(TaggingJobStatus::Pending) => tagging.pending += count,
                    Some(TaggingJobStatus::Processing) => tagging.processing += count,
                    Some(TaggingJobStatus::Failed) => tagging.failed += count,
                    Some(TaggingJobStatus::Completed | TaggingJobStatus::Cancelled) => {}
                    None => tagging.pending += count,
                }
            }

            let union_params: Vec<i64> = chunk
                .iter()
                .copied()
                .chain(chunk.iter().copied())
                .chain(chunk.iter().copied())
                .collect();
            affected_photos += self.conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM ( \
                         SELECT photo_id FROM compression_jobs \
                         WHERE photo_id IN ({placeholders}) \
                           AND status NOT IN ('completed', 'cancelled') \
                         UNION \
                         SELECT photo_id FROM send_outbox \
                         WHERE photo_id IN ({placeholders}) \
                           AND status NOT IN ('sent', 'cancelled') \
                         UNION \
                         SELECT photo_id FROM tagging_jobs \
                         WHERE photo_id IN ({placeholders}) \
                           AND status NOT IN ('completed', 'cancelled') \
                     )"
                ),
                params_from_iter(union_params),
                |row| row.get::<_, i64>(0),
            )?;
        }
        Ok(DeletionJobSummary {
            compression,
            send,
            tagging,
            affected_photos,
        })
    }

    /// Counts work that still blocks a deletion. Active sends and tagging jobs
    /// block like active compression jobs. Failed sends do not, because they
    /// carry no data-loss risk and are cancelled when the request is recorded;
    /// failed compression and tagging jobs keep the request blocked.
    pub fn deletion_blockers(&self, photo_id: i64) -> Result<DeletionBlockers> {
        let blockers = self.conn.query_row(
            "SELECT \
                (SELECT COUNT(*) FROM compression_jobs \
                 WHERE photo_id = ?1 AND status IN ('waiting', 'pending', 'processing')) \
              + (SELECT COUNT(*) FROM send_outbox \
                 WHERE photo_id = ?1 AND status IN ('waiting', 'pending', 'processing')) \
              + (SELECT COUNT(*) FROM tagging_jobs \
                 WHERE photo_id = ?1 AND status IN ('waiting', 'pending', 'processing')), \
                (SELECT COUNT(*) FROM compression_jobs \
                 WHERE photo_id = ?1 AND status = 'failed') \
              + (SELECT COUNT(*) FROM tagging_jobs \
                 WHERE photo_id = ?1 AND status = 'failed')",
            params![photo_id],
            |row| {
                Ok(DeletionBlockers {
                    active: row.get(0)?,
                    failed: row.get(1)?,
                })
            },
        )?;
        Ok(blockers)
    }

    /// Pending requests the executor can try now, ordered so manual requests
    /// come first: a manual deletion must not wait behind any auto_replace
    /// backlog, and the row limit must not hide it. Each group keeps
    /// `requested_at` order. Known reasons with an active compression job, an
    /// active send, or an active tagging job are skipped so they cannot starve
    /// ready requests behind the row limit, while unknown reasons are always
    /// returned so they can be blocked immediately. Requests with failed jobs
    /// stay in the list: the executor still needs to transition them to
    /// `blocked`.
    pub fn pending_deletion_requests(&self, limit: i64) -> Result<Vec<DeletionRequest>> {
        let sql = "SELECT dr.id, dr.photo_id, p.file_name, dr.reason, dr.status, dr.force, \
                dr.blocked_reason, dr.error, dr.requested_at, dr.started_at, dr.finished_at \
             FROM deletion_requests dr JOIN photos p ON p.id = dr.photo_id \
             WHERE dr.status = 'pending' \
               AND ( \
                   dr.reason NOT IN ('auto_replace', 'manual') \
                   OR ( \
                       dr.photo_id NOT IN ( \
                           SELECT j.photo_id FROM compression_jobs j \
                           WHERE j.status IN ('waiting', 'pending', 'processing') \
                       ) \
                       AND dr.photo_id NOT IN ( \
                           SELECT s.photo_id FROM send_outbox s \
                           WHERE s.photo_id IS NOT NULL \
                             AND s.status IN ('waiting', 'pending', 'processing') \
                       ) \
                       AND dr.photo_id NOT IN ( \
                           SELECT t.photo_id FROM tagging_jobs t \
                           WHERE t.status IN ('waiting', 'pending', 'processing') \
                       ) \
                   ) \
               ) \
             ORDER BY (dr.reason = 'manual') DESC, dr.requested_at, dr.id LIMIT ?1";
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt.query_map(params![clamp_limit(limit)], row_to_deletion_request)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn deletion_request_by_id(&self, request_id: i64) -> Result<Option<DeletionRequest>> {
        let sql = "SELECT dr.id, dr.photo_id, p.file_name, dr.reason, dr.status, dr.force, \
                dr.blocked_reason, dr.error, dr.requested_at, dr.started_at, dr.finished_at \
             FROM deletion_requests dr JOIN photos p ON p.id = dr.photo_id \
             WHERE dr.id = ?1";
        let mut stmt = self.conn.prepare_cached(sql)?;
        Ok(stmt
            .query_row(params![request_id], row_to_deletion_request)
            .optional()?)
    }

    /// Atomically claims a pending request and returns the current row. The
    /// caller must execute from the returned object, not from an earlier
    /// snapshot: a UI request may have upgraded `auto_replace` to `manual`
    /// between the snapshot and this call. Active sends and tagging jobs block
    /// the claim even when the request is forced; failed sends never do.
    pub fn claim_deletion(&self, request_id: i64) -> Result<Option<DeletionRequest>> {
        let changed = self.conn.execute(
            "UPDATE deletion_requests \
             SET status = 'running', started_at = ?1, blocked_reason = NULL, error = NULL \
             WHERE id = ?2 AND status = 'pending' \
               AND NOT EXISTS ( \
                   SELECT 1 FROM compression_jobs j \
                   WHERE j.photo_id = deletion_requests.photo_id \
                     AND j.status IN ('waiting', 'pending', 'processing') \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM send_outbox s \
                   WHERE s.photo_id = deletion_requests.photo_id \
                     AND s.status IN ('waiting', 'pending', 'processing') \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM tagging_jobs t \
                   WHERE t.photo_id = deletion_requests.photo_id \
                     AND t.status IN ('waiting', 'pending', 'processing') \
               ) \
               AND ( \
                   force = 1 OR ( \
                       NOT EXISTS ( \
                           SELECT 1 FROM compression_jobs j \
                           WHERE j.photo_id = deletion_requests.photo_id \
                             AND j.status = 'failed' \
                       ) \
                       AND NOT EXISTS ( \
                           SELECT 1 FROM tagging_jobs t \
                           WHERE t.photo_id = deletion_requests.photo_id \
                             AND t.status = 'failed' \
                       ) \
                   ) \
               )",
            params![now_string(), request_id],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.deletion_request_by_id(request_id)
    }

    /// Atomically marks a pending request blocked when a failed job exists and
    /// the request is not forced. Keeping this in SQL avoids acting on a stale
    /// `force` value from the executor's snapshot.
    pub fn block_deletion_if_jobs_failed(&self, request_id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE deletion_requests \
             SET status = 'blocked', blocked_reason = 'job_failed', error = NULL \
             WHERE id = ?1 AND status = 'pending' AND force = 0 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM compression_jobs j \
                   WHERE j.photo_id = deletion_requests.photo_id \
                     AND j.status IN ('waiting', 'pending', 'processing') \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM tagging_jobs t \
                   WHERE t.photo_id = deletion_requests.photo_id \
                     AND t.status IN ('waiting', 'pending', 'processing') \
               ) \
               AND ( \
                   EXISTS ( \
                       SELECT 1 FROM compression_jobs j \
                       WHERE j.photo_id = deletion_requests.photo_id \
                         AND j.status = 'failed' \
                   ) \
                   OR EXISTS ( \
                       SELECT 1 FROM tagging_jobs t \
                       WHERE t.photo_id = deletion_requests.photo_id \
                         AND t.status = 'failed' \
                   ) \
               )",
            params![request_id],
        )?;
        Ok(changed == 1)
    }

    /// Atomically marks a pending request blocked when its stored reason is
    /// still not a known one. Keeping this in SQL avoids acting on a stale
    /// snapshot: a request the UI upgraded to `manual` after the executor
    /// collected it must not be blocked as `unknown_reason`.
    pub fn block_deletion_if_unknown(&self, request_id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE deletion_requests \
             SET status = 'blocked', blocked_reason = 'unknown_reason', error = NULL \
             WHERE id = ?1 AND status = 'pending' \
               AND reason NOT IN ('auto_replace', 'manual')",
            params![request_id],
        )?;
        Ok(changed == 1)
    }

    pub fn block_deletion(
        &self,
        request_id: i64,
        reason: &str,
        error: Option<&str>,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE deletion_requests SET status = 'blocked', blocked_reason = ?1, error = ?2 \
             WHERE id = ?3 AND status IN ('pending', 'blocked', 'running')",
            params![reason, error, request_id],
        )?;
        Ok(changed == 1)
    }

    pub fn complete_deletion(&self, request_id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE deletion_requests \
             SET status = 'completed', finished_at = ?1, blocked_reason = NULL, error = NULL \
             WHERE id = ?2 AND status = 'running'",
            params![now_string(), request_id],
        )?;
        Ok(changed == 1)
    }

    pub fn retry_deletion(&self, request_id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE deletion_requests \
             SET status = 'pending', blocked_reason = NULL, error = NULL \
             WHERE id = ?1 AND status = 'blocked'",
            params![request_id],
        )?;
        Ok(changed == 1)
    }

    /// Ignores failed jobs and cancels queued work. A running compression job
    /// or send still has to finish before `claim_deletion` can advance, so
    /// queued and failed sends are cancelled without touching `processing`.
    /// Tagging jobs are cancelled even while processing: a late sidecar result
    /// is rejected by `save_photo_ai_for_job`.
    pub fn force_deletion(&self, request_id: i64) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let request: Option<(i64, String)> = tx
            .query_row(
                "SELECT photo_id, status FROM deletion_requests WHERE id = ?1",
                params![request_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((photo_id, status)) = request else {
            return Ok(false);
        };
        if status != "pending" && status != "blocked" {
            return Ok(false);
        }
        let now = now_string();
        tx.execute(
            "UPDATE deletion_requests \
             SET force = 1, status = 'pending', blocked_reason = NULL, error = NULL \
             WHERE id = ?1",
            params![request_id],
        )?;
        tx.execute(
            "UPDATE compression_jobs SET status = 'cancelled', progress = 0, finished_at = ?1 \
             WHERE photo_id = ?2 AND status IN ('waiting', 'pending')",
            params![now, photo_id],
        )?;
        tx.execute(
            "UPDATE send_outbox SET status = 'cancelled', finished_at = ?1 \
             WHERE photo_id = ?2 AND status IN ('waiting', 'pending', 'failed')",
            params![now, photo_id],
        )?;
        tx.execute(
            "UPDATE tagging_jobs SET status = 'cancelled', progress = 0, stage = NULL, \
                finished_at = ?1 \
             WHERE photo_id = ?2 AND status IN ('waiting', 'pending', 'processing')",
            params![now, photo_id],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Cancels a still-pending manual request. Sends cancelled when the request
    /// was recorded stay cancelled: they are not restored.
    pub fn cancel_deletion(&self, request_id: i64) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let request: Option<(i64, String, String, Option<String>)> = tx
            .query_row(
                "SELECT photo_id, reason, status, started_at FROM deletion_requests WHERE id = ?1",
                params![request_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((photo_id, reason, status, started_at)) = request else {
            return Ok(false);
        };
        if reason != "manual"
            || (status != "pending" && status != "blocked")
            || started_at.is_some()
        {
            return Ok(false);
        }
        let now = now_string();
        tx.execute(
            "UPDATE deletion_requests SET status = 'cancelled', finished_at = ?1 WHERE id = ?2",
            params![now, request_id],
        )?;
        if reason == "manual" {
            tx.execute(
                "UPDATE photos SET \
                    status = CASE WHEN display_derivative_id IS NOT NULL \
                                  THEN 'replaced' ELSE 'present' END, \
                    updated_at = ?1 \
                 WHERE id = ?2 AND status = 'deleting'",
                params![now, photo_id],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Finishes the database half of a manual deletion after every file step
    /// succeeded. The photo row stays for metadata recovery; derivative rows
    /// are removed because their files are in the OS trash.
    pub fn complete_manual_deletion(&self, photo_id: i64) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM derivatives WHERE photo_id = ?1",
            params![photo_id],
        )?;
        tx.execute(
            "UPDATE photos SET status = 'missing', display_derivative_id = NULL, updated_at = ?1 \
             WHERE id = ?2 AND status = 'deleting'",
            params![now_string(), photo_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn reset_running_deletions(&self) -> Result<usize> {
        // Keep started_at: it is the only evidence that a crash may have left
        // some files already in the OS trash, and cancel must refuse then.
        let changed = self.conn.execute(
            "UPDATE deletion_requests SET status = 'pending' WHERE status = 'running'",
            [],
        )?;
        Ok(changed)
    }

    /// Returns a job-failure block to the queue once every job for the photo is
    /// terminal success. File-error blocks stay blocked until the user retries.
    pub fn resume_blocked_deletions_for_photo(&self, photo_id: i64) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE deletion_requests \
             SET status = 'pending', blocked_reason = NULL, error = NULL \
             WHERE photo_id = ?1 AND status = 'blocked' AND blocked_reason = 'job_failed' \
               AND NOT EXISTS ( \
                   SELECT 1 FROM compression_jobs j \
                   WHERE j.photo_id = ?1 \
                     AND j.status IN ('waiting', 'pending', 'processing', 'failed') \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM tagging_jobs t \
                   WHERE t.photo_id = ?1 \
                     AND t.status IN ('waiting', 'pending', 'processing', 'failed') \
               )",
            params![photo_id],
        )?;
        Ok(changed)
    }
}

fn row_to_deletion_request(row: &Row<'_>) -> rusqlite::Result<DeletionRequest> {
    Ok(DeletionRequest {
        id: row.get(0)?,
        photo_id: row.get(1)?,
        photo_name: row.get(2)?,
        reason: DeletionReason::parse(&row.get::<_, String>(3)?).unwrap_or(DeletionReason::Unknown),
        status: DeletionStatus::parse(&row.get::<_, String>(4)?),
        force: row.get::<_, i64>(5)? != 0,
        blocked_reason: row.get(6)?,
        error: row.get(7)?,
        requested_at: row.get(8)?,
        started_at: row.get(9)?,
        finished_at: row.get(10)?,
    })
}
