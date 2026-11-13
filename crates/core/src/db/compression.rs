use super::*;

impl Db {
    // ---------- compression profiles ----------

    pub fn save_profile(&self, profile: &CompressionProfile) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let sort_order = Self::next_sort_order(&tx)?;
        Self::write_profile(&tx, profile, sort_order)?;
        tx.commit()?;
        Ok(())
    }

    fn next_sort_order(conn: &Connection) -> Result<i64> {
        Ok(conn.query_row(
            "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM compression_profiles",
            [],
            |row| row.get(0),
        )?)
    }

    fn write_profile(
        conn: &Connection,
        profile: &CompressionProfile,
        sort_order: i64,
    ) -> Result<()> {
        if profile.clipboard {
            Self::clear_other_clipboard_profiles(conn, &profile.id)?;
        }
        if profile.discord {
            Self::clear_other_discord_profiles(conn, &profile.id)?;
        }
        let json = serde_json::to_string(profile)?;
        conn.execute(
            "INSERT INTO compression_profiles(id, name, enabled, json, sort_order) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, enabled = excluded.enabled, \
                json = excluded.json",
            params![
                profile.id,
                profile.name,
                profile.enabled as i64,
                json,
                sort_order,
            ],
        )?;
        Ok(())
    }

    /// Keeps the clipboard target unique even when a caller other than the
    /// profile switch saves a flagged profile.
    fn clear_other_clipboard_profiles(conn: &Connection, keep_id: &str) -> Result<()> {
        let mut stmt =
            conn.prepare_cached("SELECT id, json FROM compression_profiles WHERE id != ?1")?;
        let rows = stmt.query_map(params![keep_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut updates = Vec::new();
        for row in rows {
            let (id, json) = row?;
            let mut profile: CompressionProfile = serde_json::from_str(&json)?;
            if profile.clipboard {
                profile.clipboard = false;
                updates.push((id, serde_json::to_string(&profile)?));
            }
        }
        drop(stmt);
        for (id, json) in updates {
            conn.execute(
                "UPDATE compression_profiles SET json = ?1 WHERE id = ?2",
                params![json, id],
            )?;
        }
        Ok(())
    }

    /// Keeps the Discord target unique even when a caller other than the
    /// profile switch saves a flagged profile.
    fn clear_other_discord_profiles(conn: &Connection, keep_id: &str) -> Result<()> {
        let mut stmt =
            conn.prepare_cached("SELECT id, json FROM compression_profiles WHERE id != ?1")?;
        let rows = stmt.query_map(params![keep_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut updates = Vec::new();
        for row in rows {
            let (id, json) = row?;
            let mut profile: CompressionProfile = serde_json::from_str(&json)?;
            if profile.discord {
                profile.discord = false;
                updates.push((id, serde_json::to_string(&profile)?));
            }
        }
        drop(stmt);
        for (id, json) in updates {
            conn.execute(
                "UPDATE compression_profiles SET json = ?1 WHERE id = ?2",
                params![json, id],
            )?;
        }
        Ok(())
    }

    pub fn list_profiles(&self) -> Result<Vec<CompressionProfile>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT json FROM compression_profiles ORDER BY sort_order, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut profiles = Vec::new();
        for row in rows {
            let json = row?;
            profiles.push(serde_json::from_str(&json)?);
        }
        Ok(profiles)
    }

    pub fn reorder_profiles(&self, ids: &[String]) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let stored: i64 = tx.query_row("SELECT COUNT(*) FROM compression_profiles", [], |row| {
            row.get(0)
        })?;
        if stored != ids.len() as i64 {
            return Err(CoreError::Invalid(
                "profile order must list every profile".to_string(),
            ));
        }
        let mut seen = HashSet::new();
        for id in ids {
            if !seen.insert(id.as_str()) {
                return Err(CoreError::Invalid(
                    "profile order contains duplicate ids".to_string(),
                ));
            }
        }
        for (index, id) in ids.iter().enumerate() {
            let changed = tx.execute(
                "UPDATE compression_profiles SET sort_order = ?1 WHERE id = ?2",
                params![index as i64, id],
            )?;
            if changed != 1 {
                return Err(CoreError::NotFound(format!("compression profile {id}")));
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Generated copy names keep the source name readable but never exceed
    /// the UI's display limit, even when the source is already at the cap.
    fn copy_profile_name(source: &str, suffix: usize) -> String {
        let ending = if suffix <= 1 {
            " のコピー".to_string()
        } else {
            format!(" のコピー {suffix}")
        };
        let keep = MAX_PROFILE_NAME_CHARS
            .saturating_sub(ending.chars().count())
            .max(1);
        let mut name: String = source.chars().take(keep).collect();
        name.push_str(&ending);
        name
    }

    pub fn duplicate_profile(&self, id: &str) -> Result<CompressionProfile> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let ordered = {
            let mut stmt = tx.prepare_cached(
                "SELECT id, json FROM compression_profiles \
                 ORDER BY sort_order, name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut ordered = Vec::new();
            for row in rows {
                let (stored_id, json) = row?;
                ordered.push((
                    stored_id,
                    serde_json::from_str::<CompressionProfile>(&json)?,
                ));
            }
            ordered
        };

        let source_index = ordered
            .iter()
            .position(|(stored_id, _)| stored_id == id)
            .ok_or_else(|| CoreError::NotFound(format!("compression profile {id}")))?;

        let source_name = ordered[source_index].1.name.clone();
        let mut suffix = 1;
        let mut name = Self::copy_profile_name(&source_name, suffix);
        while ordered.iter().any(|(_, profile)| profile.name == name) {
            suffix += 1;
            name = Self::copy_profile_name(&source_name, suffix);
        }

        for (index, (stored_id, _)) in ordered.iter().enumerate() {
            tx.execute(
                "UPDATE compression_profiles SET sort_order = ?1 WHERE id = ?2",
                params![index as i64, stored_id],
            )?;
        }
        tx.execute(
            "UPDATE compression_profiles SET sort_order = sort_order + 1 WHERE sort_order > ?1",
            params![source_index as i64],
        )?;

        let copy = CompressionProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            auto_enabled: false,
            // The clipboard target is unique system-wide (the UI switch clears
            // every other profile). A duplicate must not bypass that invariant.
            clipboard: false,
            // The Discord target is unique for the same reason.
            discord: false,
            ..ordered[source_index].1.clone()
        };
        Self::write_profile(&tx, &copy, source_index as i64 + 1)?;
        tx.commit()?;
        Ok(copy)
    }

    /// Seeds the built-in presets when no profile exists.
    ///
    /// Returns the profiles currently stored. A read or write error is returned
    /// without leaving a partial seed: existing rows are never touched, and the
    /// preset inserts commit together.
    pub fn ensure_default_profiles(&self) -> Result<Vec<CompressionProfile>> {
        let existing = self.list_profiles()?;
        if !existing.is_empty() {
            return Ok(existing);
        }
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        for (index, profile) in CompressionProfile::defaults().into_iter().enumerate() {
            Self::write_profile(&transaction, &profile, index as i64)?;
        }
        transaction.commit()?;
        self.list_profiles()
    }

    pub fn active_job_count_for_profile(&self, profile_id: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM compression_jobs \
             WHERE profile_id = ?1 AND status IN ('waiting', 'pending', 'processing')",
            params![profile_id],
            |row| row.get(0),
        )?)
    }

    pub fn delete_profile(&self, id: &str) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let exists: i64 = tx.query_row(
            "SELECT COUNT(*) FROM compression_profiles WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        if exists == 0 {
            return Err(CoreError::NotFound(format!("compression profile {id}")));
        }

        let active: i64 = tx.query_row(
            "SELECT COUNT(*) FROM compression_jobs \
             WHERE profile_id = ?1 AND status IN ('waiting', 'pending', 'processing')",
            params![id],
            |row| row.get(0),
        )?;
        if active > 0 {
            return Err(CoreError::Invalid(format!(
                "compression profile {id} has {active} active jobs"
            )));
        }

        let total: i64 = tx.query_row("SELECT COUNT(*) FROM compression_profiles", [], |row| {
            row.get(0)
        })?;
        if total <= 1 {
            return Err(CoreError::Invalid(
                "at least one compression profile is required".to_string(),
            ));
        }

        let deleted = tx.execute(
            "DELETE FROM compression_profiles WHERE id = ?1",
            params![id],
        )?;
        if deleted == 0 {
            return Err(CoreError::NotFound(format!("compression profile {id}")));
        }
        tx.commit()?;
        Ok(())
    }
}

impl Db {
    // ---------- compression jobs ----------

    pub fn enqueue_job(&self, photo_id: i64, profile_id: &str, policy: &str) -> Result<String> {
        self.enqueue_job_with_options(photo_id, profile_id, policy, QueueStatus::Pending, None)
    }

    pub fn enqueue_job_with_status(
        &self,
        photo_id: i64,
        profile_id: &str,
        policy: &str,
        status: QueueStatus,
    ) -> Result<String> {
        self.enqueue_job_with_options(photo_id, profile_id, policy, status, None)
    }

    /// Enqueues a manual job whose output directory overrides the profile.
    pub fn enqueue_job_with_output_dir(
        &self,
        photo_id: i64,
        profile_id: &str,
        policy: &str,
        output_dir: Option<&str>,
    ) -> Result<String> {
        self.enqueue_job_with_options(
            photo_id,
            profile_id,
            policy,
            QueueStatus::Pending,
            output_dir,
        )
    }

    fn enqueue_job_with_options(
        &self,
        photo_id: i64,
        profile_id: &str,
        policy: &str,
        status: QueueStatus,
        output_dir: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let inserted = self.conn.execute(
            "INSERT INTO compression_jobs(id, photo_id, profile_id, origin, status, progress, policy, output_dir, created_at) \
             SELECT ?1, ?2, ?3, 'manual', ?4, 0, ?5, ?6, ?7 \
             WHERE EXISTS (SELECT 1 FROM photos WHERE id = ?2 AND status <> 'deleting') \
               AND NOT EXISTS (SELECT 1 FROM deletion_requests \
                   WHERE photo_id = ?2 AND status IN ('pending', 'blocked', 'running'))",
            params![
                id,
                photo_id,
                profile_id,
                status.as_str(),
                policy,
                output_dir,
                now_string()
            ],
        )?;
        if inserted == 0 {
            return Err(CoreError::Invalid("photo is pending deletion".to_string()));
        }
        Ok(id)
    }

    pub fn enqueue_job_for_existing_profile(
        &self,
        photo_id: i64,
        profile_id: &str,
        policy: &str,
    ) -> Result<bool> {
        self.enqueue_job_for_existing_profile_with_options(
            photo_id,
            profile_id,
            policy,
            QueueStatus::Pending,
            None,
        )
    }

    pub fn enqueue_job_for_existing_profile_with_status(
        &self,
        photo_id: i64,
        profile_id: &str,
        policy: &str,
        status: QueueStatus,
    ) -> Result<bool> {
        self.enqueue_job_for_existing_profile_with_options(
            photo_id, profile_id, policy, status, None,
        )
    }

    /// Enqueues an automatic job unless the photo already has any job for the
    /// profile. Auto compression runs once per photo/profile; re-indexing must
    /// not recreate completed, failed, or cancelled work.
    pub fn enqueue_auto_job_for_existing_profile_with_status(
        &self,
        photo_id: i64,
        profile_id: &str,
        policy: &str,
        status: QueueStatus,
    ) -> Result<bool> {
        let id = uuid::Uuid::new_v4().to_string();
        let inserted = self.conn.execute(
            "INSERT INTO compression_jobs(id, photo_id, profile_id, origin, status, progress, policy, output_dir, created_at) \
             SELECT ?1, ?2, ?3, 'auto', ?4, 0, ?5, NULL, ?6 \
             WHERE EXISTS (SELECT 1 FROM compression_profiles WHERE id = ?3) \
               AND EXISTS (SELECT 1 FROM photos WHERE id = ?2 AND status <> 'deleting') \
               AND NOT EXISTS (SELECT 1 FROM deletion_requests \
                   WHERE photo_id = ?2 AND status IN ('pending', 'blocked', 'running')) \
               AND NOT EXISTS (SELECT 1 FROM compression_jobs \
                   WHERE photo_id = ?2 AND profile_id = ?3)",
            params![
                id,
                photo_id,
                profile_id,
                status.as_str(),
                policy,
                now_string()
            ],
        )?;
        Ok(inserted == 1)
    }

    /// Enqueues a manual job for an existing profile with an optional output
    /// directory override.
    pub fn enqueue_job_for_existing_profile_with_output_dir(
        &self,
        photo_id: i64,
        profile_id: &str,
        policy: &str,
        output_dir: Option<&str>,
    ) -> Result<bool> {
        self.enqueue_job_for_existing_profile_with_options(
            photo_id,
            profile_id,
            policy,
            QueueStatus::Pending,
            output_dir,
        )
    }

    fn enqueue_job_for_existing_profile_with_options(
        &self,
        photo_id: i64,
        profile_id: &str,
        policy: &str,
        status: QueueStatus,
        output_dir: Option<&str>,
    ) -> Result<bool> {
        let id = uuid::Uuid::new_v4().to_string();
        let inserted = self.conn.execute(
            "INSERT INTO compression_jobs(id, photo_id, profile_id, origin, status, progress, policy, output_dir, created_at) \
             SELECT ?1, ?2, ?3, 'manual', ?4, 0, ?5, ?6, ?7 \
             WHERE EXISTS (SELECT 1 FROM compression_profiles WHERE id = ?3) \
               AND EXISTS (SELECT 1 FROM photos WHERE id = ?2 AND status <> 'deleting') \
               AND NOT EXISTS (SELECT 1 FROM deletion_requests \
                   WHERE photo_id = ?2 AND status IN ('pending', 'blocked', 'running'))",
            params![
                id,
                photo_id,
                profile_id,
                status.as_str(),
                policy,
                output_dir,
                now_string()
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn next_pending_job(&self) -> Result<Option<CompressionJob>> {
        let job = self
            .conn
            .query_row(
                "UPDATE compression_jobs SET status = 'processing', started_at = ?1 \
                 WHERE id = (SELECT id FROM compression_jobs WHERE status = 'pending' \
                    ORDER BY created_at, rowid LIMIT 1) \
                 RETURNING id, photo_id, profile_id, origin, status, progress, error, output_path, policy, output_dir, \
                    retry_count, created_at, started_at, finished_at",
                params![now_string()],
                row_to_job,
            )
            .optional()?;
        Ok(job)
    }

    /// Claims every pending job for the photo owning the oldest pending job.
    ///
    /// The immediate transaction keeps the photo selection and the claim
    /// atomic across connections (the daemon and a maintenance CLI can run
    /// at the same time), so a claimer never sees an empty batch while
    /// another connection still has pending work.
    pub fn next_pending_photo_jobs(&self) -> Result<Vec<CompressionJob>> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let photo_id: Option<i64> = tx
            .query_row(
                "SELECT photo_id FROM compression_jobs WHERE status = 'pending' \
                 ORDER BY created_at, rowid LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(photo_id) = photo_id else {
            tx.commit()?;
            return Ok(Vec::new());
        };
        let mut jobs = {
            let mut stmt = tx.prepare_cached(
                "UPDATE compression_jobs SET status = 'processing', started_at = ?1 \
                 WHERE status = 'pending' AND photo_id = ?2 \
                 RETURNING id, photo_id, profile_id, origin, status, progress, error, output_path, policy, output_dir, \
                    retry_count, created_at, started_at, finished_at, rowid",
            )?;
            let rows = stmt.query_map(params![now_string(), photo_id], |row| {
                Ok((row.get::<_, i64>(14)?, row_to_job(row)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.commit()?;
        jobs.sort_by_key(|(rowid, _)| *rowid);
        Ok(jobs.into_iter().map(|(_, job)| job).collect())
    }

    /// Marks a pending or processing job failed. Terminal jobs are left
    /// untouched and their current status is returned instead.
    pub fn fail_job_if_active(&self, id: &str, error: &str) -> Result<Option<QueueStatus>> {
        let changed = self.conn.execute(
            "UPDATE compression_jobs SET status = 'failed', progress = 0, error = ?1, \
                finished_at = ?2 WHERE id = ?3 AND status IN ('pending', 'processing')",
            params![error, now_string(), id],
        )?;
        if changed == 1 {
            return Ok(Some(QueueStatus::Failed));
        }
        let status = self
            .conn
            .query_row(
                "SELECT status FROM compression_jobs WHERE id = ?1",
                params![id],
                |row| Ok(QueueStatus::parse(&row.get::<_, String>(0)?)),
            )
            .optional()?;
        Ok(status)
    }

    pub fn update_job(
        &self,
        id: &str,
        status: QueueStatus,
        progress: f64,
        error: Option<&str>,
        output_path: Option<&str>,
    ) -> Result<()> {
        let finished_at = if matches!(
            status,
            QueueStatus::Completed | QueueStatus::Failed | QueueStatus::Cancelled
        ) {
            Some(now_string())
        } else {
            None
        };
        self.conn.execute(
            "UPDATE compression_jobs SET status = ?1, progress = ?2, error = ?3, \
                output_path = ?4, finished_at = COALESCE(?5, finished_at) WHERE id = ?6",
            params![
                status.as_str(),
                progress,
                error,
                output_path,
                finished_at,
                id,
            ],
        )?;
        Ok(())
    }

    /// Columns read for every `compression_jobs` row, in `row_to_job` order.
    const JOB_COLUMNS: &str = "id, photo_id, profile_id, origin, status, progress, error, \
        output_path, policy, output_dir, retry_count, created_at, started_at, finished_at";

    /// Lists the newest jobs first. `limit` is normalized by
    /// [`clamp_limit`], so a non-positive limit returns the single newest job
    /// and an oversized limit is capped at [`MAX_ROW_LIMIT`].
    pub fn list_jobs(&self, limit: i64) -> Result<Vec<CompressionJob>> {
        let sql = format!(
            "SELECT {} FROM compression_jobs ORDER BY created_at DESC LIMIT ?1",
            Self::JOB_COLUMNS
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![clamp_limit(limit)], row_to_job)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Lists the newest jobs with `status` first. `limit` is normalized by
    /// [`clamp_limit`], so a non-positive limit returns the single newest
    /// match and an oversized limit is capped at [`MAX_ROW_LIMIT`].
    pub fn list_jobs_by_status(
        &self,
        status: QueueStatus,
        limit: i64,
    ) -> Result<Vec<CompressionJob>> {
        let sql = format!(
            "SELECT {} FROM compression_jobs WHERE status = ?1 \
             ORDER BY created_at DESC LIMIT ?2",
            Self::JOB_COLUMNS
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![status.as_str(), clamp_limit(limit)], row_to_job)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Promotes every waiting job to pending so workers can pick it up,
    /// restamping the current global keep/replace policy. Returns how many
    /// rows changed.
    pub fn release_waiting_jobs(&self, policy: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE compression_jobs SET status = 'pending', progress = 0, error = NULL, \
                started_at = NULL, finished_at = NULL, policy = ?1 \
             WHERE status = 'waiting'",
            params![policy],
        )?;
        Ok(changed)
    }

    /// Assignments that return a failed job to the queue.
    const RETRY_JOB_SET: &str = "status = 'pending', progress = 0, error = NULL, \
        started_at = NULL, finished_at = NULL, retry_count = retry_count + 1";

    /// Guard that keeps a job queued while its photo is being deleted.
    const NO_RUNNING_DELETION_GUARD: &str = "NOT EXISTS (SELECT 1 FROM deletion_requests dr \
        WHERE dr.photo_id = compression_jobs.photo_id AND dr.status = 'running')";

    /// Returns a failed job to the queue once, incrementing its retry count.
    /// Returns false when the job is missing, not failed, or already at the
    /// retry cap.
    /// Returns jobs left `processing` by a crash to the queue without
    /// consuming a retry. A daemon restart must not strand a compression job:
    /// a stranded row also blocks deletion of its photo forever.
    pub fn resume_interrupted_compression_jobs(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE compression_jobs SET status = 'pending', progress = 0, error = NULL, \
                started_at = NULL, finished_at = NULL \
             WHERE status = 'processing'",
            [],
        )?;
        Ok(changed)
    }

    pub fn retry_job(&self, id: &str) -> Result<bool> {
        let sql = format!(
            "UPDATE compression_jobs SET {} WHERE id = ?1 AND status = 'failed' \
             AND retry_count < ?2 AND {}",
            Self::RETRY_JOB_SET,
            Self::NO_RUNNING_DELETION_GUARD
        );
        let changed = self.conn.execute(&sql, params![id, MAX_JOB_RETRIES])?;
        Ok(changed == 1)
    }

    /// Returns every eligible failed job to the queue and reports how many
    /// rows changed. Jobs at the retry cap are left untouched.
    pub fn retry_failed_jobs(&self) -> Result<usize> {
        let sql = format!(
            "UPDATE compression_jobs SET {} WHERE status = 'failed' \
             AND retry_count < ?1 AND {}",
            Self::RETRY_JOB_SET,
            Self::NO_RUNNING_DELETION_GUARD
        );
        let changed = self.conn.execute(&sql, params![MAX_JOB_RETRIES])?;
        Ok(changed)
    }

    pub fn count_jobs(&self, status: QueueStatus) -> Result<i64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM compression_jobs WHERE status = ?1",
            params![status.as_str()],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Counts failed jobs that can still return to the queue. Callers use
    /// this aggregate — not a page of failed jobs — to decide whether a
    /// retry-all action exists, so the answer stays correct past the
    /// page size.
    pub fn count_retryable_failed_jobs(&self) -> Result<i64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM compression_jobs \
             WHERE status = 'failed' AND retry_count < ?1",
            params![MAX_JOB_RETRIES],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Counts jobs a user still expects to run or finish: waiting, pending,
    /// and processing. Failed and completed jobs are excluded.
    pub fn count_active_jobs(&self) -> Result<i64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM compression_jobs \
             WHERE status IN ('waiting', 'pending', 'processing')",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }
}

fn row_to_job(row: &Row<'_>) -> rusqlite::Result<CompressionJob> {
    Ok(CompressionJob {
        id: row.get(0)?,
        photo_id: row.get(1)?,
        profile_id: row.get(2)?,
        origin: CompressionOrigin::parse(&row.get::<_, String>(3)?),
        status: QueueStatus::parse(&row.get::<_, String>(4)?),
        progress: row.get(5)?,
        error: row.get(6)?,
        output_path: row.get(7)?,
        policy: row.get(8)?,
        output_dir: row.get(9)?,
        retry_count: row.get(10)?,
        created_at: row.get(11)?,
        started_at: row.get(12)?,
        finished_at: row.get(13)?,
    })
}
