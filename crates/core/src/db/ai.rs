use super::*;

impl Db {
    // ---------- AI plugins ----------

    /// Inserts or updates one plugin installation. When `install.active` is
    /// set, every other installation of the same plugin is deactivated in the
    /// same immediate transaction so the partial unique index always holds.
    pub fn upsert_plugin_installation(&self, install: &PluginInstallation) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if install.active {
            tx.execute(
                "UPDATE plugin_installations SET active = 0 \
                 WHERE plugin_id = ?1 AND active = 1",
                params![install.plugin_id],
            )?;
        }
        tx.execute(
            "INSERT INTO plugin_installations(plugin_id, variant, version, protocol_version, \
                path, manifest_json, catalog_version, active, installed_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT(plugin_id, variant, version) DO UPDATE SET \
                protocol_version = excluded.protocol_version, \
                path = excluded.path, \
                manifest_json = excluded.manifest_json, \
                catalog_version = excluded.catalog_version, \
                active = excluded.active, \
                updated_at = excluded.updated_at",
            params![
                install.plugin_id,
                install.variant,
                install.version,
                install.protocol_version,
                install.path,
                install.manifest_json,
                install.catalog_version,
                i64::from(install.active),
                install.installed_at,
                install.updated_at,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_plugin_installations(&self, plugin_id: &str) -> Result<Vec<PluginInstallation>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT plugin_id, variant, version, protocol_version, path, manifest_json, \
                catalog_version, active, installed_at, updated_at \
             FROM plugin_installations WHERE plugin_id = ?1 ORDER BY variant, version",
        )?;
        let rows = stmt.query_map(params![plugin_id], row_to_plugin_installation)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn active_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> Result<Option<PluginInstallation>> {
        let install = self
            .conn
            .query_row(
                "SELECT plugin_id, variant, version, protocol_version, path, manifest_json, \
                    catalog_version, active, installed_at, updated_at \
                 FROM plugin_installations WHERE plugin_id = ?1 AND active = 1",
                params![plugin_id],
                row_to_plugin_installation,
            )
            .optional()?;
        Ok(install)
    }

    /// Marks one installation as active and clears the previous one
    /// atomically. `None` selects the highest semver installed for `variant`.
    /// Returns whether a matching installation was found.
    pub fn set_active_plugin_installation(
        &self,
        plugin_id: &str,
        variant: &str,
        version: Option<&str>,
    ) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let target = match version {
            Some(version) => tx
                .query_row(
                    "SELECT version FROM plugin_installations \
                     WHERE plugin_id = ?1 AND variant = ?2 AND version = ?3",
                    params![plugin_id, variant, version],
                    |row| row.get::<_, String>(0),
                )
                .optional()?,
            None => {
                let mut stmt = tx.prepare_cached(
                    "SELECT version FROM plugin_installations \
                     WHERE plugin_id = ?1 AND variant = ?2",
                )?;
                let versions =
                    stmt.query_map(params![plugin_id, variant], |row| row.get::<_, String>(0))?;
                let mut best: Option<(semver::Version, String)> = None;
                for version in versions {
                    let version = version?;
                    let Ok(parsed) = semver::Version::parse(&version) else {
                        continue;
                    };
                    if best.as_ref().is_none_or(|(current, _)| parsed > *current) {
                        best = Some((parsed, version));
                    }
                }
                best.map(|(_, version)| version)
            }
        };
        let Some(target) = target else {
            tx.commit()?;
            return Ok(false);
        };

        tx.execute(
            "UPDATE plugin_installations SET active = 0 \
             WHERE plugin_id = ?1 AND active = 1",
            params![plugin_id],
        )?;
        tx.execute(
            "UPDATE plugin_installations SET active = 1 \
             WHERE plugin_id = ?1 AND variant = ?2 AND version = ?3",
            params![plugin_id, variant, target],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn remove_plugin_installation(
        &self,
        plugin_id: &str,
        variant: &str,
        version: &str,
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM plugin_installations \
             WHERE plugin_id = ?1 AND variant = ?2 AND version = ?3",
            params![plugin_id, variant, version],
        )?;
        Ok(())
    }

    pub fn plugin_state(&self, plugin_id: &str) -> Result<Option<PluginState>> {
        let state = self
            .conn
            .query_row(
                "SELECT plugin_id, model_state, model_revision, model_bytes_downloaded, \
                    model_bytes_total, model_error, probe_json, updated_at \
                 FROM plugin_state WHERE plugin_id = ?1",
                params![plugin_id],
                row_to_plugin_state,
            )
            .optional()?;
        Ok(state)
    }

    pub fn set_plugin_model_state(
        &self,
        plugin_id: &str,
        state: ModelState,
        revision: Option<&str>,
        downloaded: i64,
        total: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO plugin_state(plugin_id, model_state, model_revision, \
                model_bytes_downloaded, model_bytes_total, model_error, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(plugin_id) DO UPDATE SET \
                model_state = excluded.model_state, \
                model_revision = excluded.model_revision, \
                model_bytes_downloaded = excluded.model_bytes_downloaded, \
                model_bytes_total = excluded.model_bytes_total, \
                model_error = excluded.model_error, \
                updated_at = excluded.updated_at",
            params![
                plugin_id,
                state.as_str(),
                revision,
                downloaded.max(0),
                total,
                error,
                now_string(),
            ],
        )?;
        Ok(())
    }

    /// Stores the raw `--probe` JSON, creating a `missing` model state row if
    /// the plugin has no state yet. The model columns are left untouched on
    /// update.
    pub fn set_plugin_probe_json(&self, plugin_id: &str, probe_json: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO plugin_state(plugin_id, model_state, model_bytes_downloaded, \
                probe_json, updated_at) \
             VALUES (?1, ?2, 0, ?3, ?4) \
             ON CONFLICT(plugin_id) DO UPDATE SET \
                probe_json = excluded.probe_json, \
                updated_at = excluded.updated_at",
            params![
                plugin_id,
                ModelState::Missing.as_str(),
                probe_json,
                now_string()
            ],
        )?;
        Ok(())
    }

    /// Mirrors the plugin's catalog into the local definition table. User
    /// threshold overrides are never touched. If a tag's default threshold
    /// changed, applied flags are recomputed after the upserts.
    pub fn sync_ai_catalog(&self, plugin_id: &str, catalog: &Catalog) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let mut defaults_changed = false;
        for (branch, definitions) in [
            (TagBranch::Outdoor, &catalog.outdoor),
            (TagBranch::Indoor, &catalog.indoor),
            (TagBranch::Atmosphere, &catalog.atmosphere),
        ] {
            for (sort_order, definition) in definitions.iter().enumerate() {
                let previous: Option<f32> = tx
                    .query_row(
                        "SELECT default_threshold FROM ai_tag_definitions \
                         WHERE plugin_id = ?1 AND tag_key = ?2",
                        params![plugin_id, definition.key],
                        |row| row.get(0),
                    )
                    .optional()?;
                if previous.is_some_and(|previous| previous != definition.default_threshold) {
                    defaults_changed = true;
                }
                tx.execute(
                    "INSERT INTO ai_tag_definitions(plugin_id, tag_key, branch, tag_group, name, \
                        name_en, description, default_threshold, sort_order, catalog_version, \
                        updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                     ON CONFLICT(plugin_id, tag_key) DO UPDATE SET \
                        branch = excluded.branch, \
                        tag_group = excluded.tag_group, \
                        name = excluded.name, \
                        name_en = excluded.name_en, \
                        description = excluded.description, \
                        default_threshold = excluded.default_threshold, \
                        sort_order = excluded.sort_order, \
                        catalog_version = excluded.catalog_version, \
                        updated_at = excluded.updated_at",
                    params![
                        plugin_id,
                        definition.key,
                        branch.as_str(),
                        definition.group,
                        definition.name,
                        definition.name_en,
                        definition.description,
                        definition.default_threshold,
                        sort_order as i64,
                        catalog.version,
                        now_string(),
                    ],
                )?;
            }
        }
        let updated = if defaults_changed {
            reapply_thresholds(&tx, plugin_id)?
        } else {
            0
        };
        tx.commit()?;
        if defaults_changed || updated > 0 {
            self.bump_ai_revision()?;
        }
        Ok(())
    }

    pub fn ai_tag_definitions(&self, plugin_id: &str) -> Result<Vec<AiTagDefinition>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT plugin_id, tag_key, branch, tag_group, name, name_en, description, \
                default_threshold, sort_order, catalog_version, updated_at \
             FROM ai_tag_definitions WHERE plugin_id = ?1 \
             ORDER BY branch, sort_order, tag_key",
        )?;
        let rows = stmt.query_map(params![plugin_id], row_to_ai_tag_definition)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Applies a batch of tag-threshold overrides in one transaction.
    ///
    /// `Some(threshold)` upserts an override; `None` removes it so the
    /// catalog default applies again. After writing, `photo_ai_tags.applied`
    /// is recomputed once. Returns the number of re-evaluated tag rows.
    pub fn apply_ai_tag_thresholds(
        &self,
        plugin_id: &str,
        updates: &[(String, Option<f32>)],
    ) -> Result<usize> {
        for (_, threshold) in updates {
            if let Some(threshold) = threshold
                && (!threshold.is_finite() || !(0.0..=1.0).contains(threshold))
            {
                return Err(CoreError::Invalid(format!(
                    "AI tag threshold {threshold} is outside 0.0..=1.0"
                )));
            }
        }

        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        for (tag_key, threshold) in updates {
            match threshold {
                Some(threshold) => {
                    tx.execute(
                        "INSERT INTO ai_tag_thresholds(plugin_id, tag_key, threshold, updated_at) \
                         VALUES (?1, ?2, ?3, ?4) \
                         ON CONFLICT(plugin_id, tag_key) DO UPDATE SET \
                            threshold = excluded.threshold, \
                            updated_at = excluded.updated_at",
                        params![plugin_id, tag_key, *threshold, now_string()],
                    )?;
                }
                None => {
                    tx.execute(
                        "DELETE FROM ai_tag_thresholds WHERE plugin_id = ?1 AND tag_key = ?2",
                        params![plugin_id, tag_key],
                    )?;
                }
            }
        }
        let updated = reapply_thresholds(&tx, plugin_id)?;
        tx.commit()?;
        if updated > 0 {
            self.bump_ai_revision()?;
        }
        Ok(updated)
    }

    pub fn set_ai_tag_threshold(
        &self,
        plugin_id: &str,
        tag_key: &str,
        threshold: f32,
    ) -> Result<()> {
        self.apply_ai_tag_thresholds(plugin_id, &[(tag_key.to_string(), Some(threshold))])
            .map(|_| ())
    }

    pub fn reset_ai_tag_threshold(&self, plugin_id: &str, tag_key: &str) -> Result<()> {
        self.apply_ai_tag_thresholds(plugin_id, &[(tag_key.to_string(), None)])
            .map(|_| ())
    }

    pub fn ai_tag_thresholds(&self, plugin_id: &str) -> Result<Vec<AiTagThreshold>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT plugin_id, tag_key, threshold, updated_at \
             FROM ai_tag_thresholds WHERE plugin_id = ?1 ORDER BY tag_key",
        )?;
        let rows = stmt.query_map(params![plugin_id], row_to_ai_tag_threshold)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Recomputes `photo_ai_tags.applied` from the effective threshold
    /// (user override, else the catalog default) in one UPDATE. Returns the
    /// number of tag rows that were re-evaluated. The AI revision advances
    /// after the commit when at least one row was touched.
    pub fn reapply_ai_tag_thresholds(&self, plugin_id: &str) -> Result<usize> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let updated = reapply_thresholds(&tx, plugin_id)?;
        tx.commit()?;
        if updated > 0 {
            self.bump_ai_revision()?;
        }
        Ok(updated)
    }
}

impl Db {
    // ---------- AI results ----------

    /// Stores one tagging result for `photo_id`. The photo must still be in
    /// the library (`present` or `replaced`); missing, deleting, or unknown
    /// photos are skipped without writing anything. The judged tags replace
    /// the previous result for this plugin, and `applied` is derived from the
    /// effective threshold (user override, else catalog default). Manual tags
    /// are never touched. The AI revision advances after the commit.
    pub fn save_photo_ai(
        &self,
        photo_id: i64,
        plugin_id: &str,
        result: &NewPhotoAi,
    ) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let saved = write_photo_ai(&tx, photo_id, plugin_id, result)?;
        tx.commit()?;
        if saved {
            self.bump_ai_revision()?;
        }
        Ok(saved)
    }

    /// Stores a result only while its job is still `processing`. A cancelled,
    /// completed, failed, or missing job makes this a no-op so late sidecar
    /// results are never written after a cancellation.
    pub fn save_photo_ai_for_job(&self, job_id: &str, result: &NewPhotoAi) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let job: Option<(i64, String, String)> = tx
            .query_row(
                "SELECT photo_id, plugin_id, status FROM tagging_jobs WHERE id = ?1",
                params![job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((photo_id, plugin_id, status)) = job else {
            tx.commit()?;
            return Ok(false);
        };
        if status != TaggingJobStatus::Processing.as_str() {
            tx.commit()?;
            return Ok(false);
        }
        let saved = write_photo_ai(&tx, photo_id, &plugin_id, result)?;
        tx.commit()?;
        if saved {
            self.bump_ai_revision()?;
        }
        Ok(saved)
    }

    pub fn photo_ai(&self, photo_id: i64) -> Result<Option<PhotoAi>> {
        let ai = self
            .conn
            .query_row(
                "SELECT photo_id, plugin_id, variant, sidecar_version, model_version, \
                    model_revision, catalog_version, source, presence_threshold, \
                    character_present, presence_probability, people_min, people_max, \
                    people_median, place_branch, place_probabilities_json, tagged_at \
                 FROM photo_ai WHERE photo_id = ?1",
                params![photo_id],
                row_to_photo_ai,
            )
            .optional()?;
        Ok(ai)
    }

    pub fn photo_ai_tags(&self, photo_id: i64, applied_only: bool) -> Result<Vec<PhotoAiTag>> {
        let applied_clause = if applied_only { " AND applied = 1" } else { "" };
        let sql = format!(
            "SELECT photo_id, plugin_id, tag_key, probability, applied, model_version, \
                catalog_version, tagged_at FROM photo_ai_tags \
             WHERE photo_id = ?1{applied_clause} ORDER BY tag_key"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![photo_id], row_to_photo_ai_tag)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Returns every judged tag with its catalog display names. Tags that are
    /// no longer in the catalog fall back to the raw key.
    pub fn photo_ai_tags_with_names(&self, photo_id: i64) -> Result<Vec<PhotoAiTagView>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT t.tag_key, COALESCE(d.branch, ''), COALESCE(d.tag_group, ''), \
                COALESCE(d.name, t.tag_key), d.name_en, t.probability, t.applied \
             FROM photo_ai_tags t \
             LEFT JOIN ai_tag_definitions d \
               ON d.plugin_id = t.plugin_id AND d.tag_key = t.tag_key \
             WHERE t.photo_id = ?1 \
             ORDER BY COALESCE(d.branch, ''), COALESCE(d.tag_group, ''), \
                COALESCE(d.sort_order, 9223372036854775807), t.tag_key",
        )?;
        let rows = stmt.query_map(params![photo_id], |row| {
            Ok(PhotoAiTagView {
                tag_key: row.get(0)?,
                branch: row.get(1)?,
                tag_group: row.get(2)?,
                name: row.get(3)?,
                name_en: row.get(4)?,
                probability: row.get(5)?,
                applied: row.get::<_, i64>(6)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

impl Db {
    // ---------- tagging jobs ----------

    /// Enqueues one job per photo for `plugin_id`. Only `waiting` (backlog)
    /// and `pending` (ready) are accepted. The partial active index dedupes
    /// when the photo already has an unfinished job; photos outside the
    /// library or pending deletion are skipped. Returns the inserted count.
    pub fn enqueue_tagging_jobs(
        &self,
        photo_ids: &[i64],
        plugin_id: &str,
        status: TaggingJobStatus,
    ) -> Result<usize> {
        if !matches!(
            status,
            TaggingJobStatus::Waiting | TaggingJobStatus::Pending
        ) {
            return Err(CoreError::Invalid(format!(
                "cannot enqueue a tagging job as {}",
                status.as_str()
            )));
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let now = now_string();
        let mut inserted = 0;
        for photo_id in photo_ids {
            inserted += tx.execute(
                "INSERT OR IGNORE INTO tagging_jobs \
                    (id, photo_id, plugin_id, status, progress, created_at) \
                 SELECT ?1, p.id, ?2, ?3, 0, ?4 FROM photos p \
                 WHERE p.id = ?5 AND p.status IN ('present', 'replaced') \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM deletion_requests dr \
                       WHERE dr.photo_id = p.id AND dr.status IN ('pending', 'blocked', 'running') \
                   )",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    plugin_id,
                    status.as_str(),
                    now,
                    photo_id,
                ],
            )?;
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Queues every visible photo without an AI result as `waiting`, newest
    /// first. Re-running it is idempotent: the active index keeps existing
    /// waiting/pending/processing rows from being duplicated. The whole
    /// backlog is one ordered `INSERT ... SELECT`, so a large library does not
    /// pay for a per-photo round trip inside the foreground transaction.
    pub fn enqueue_tagging_backlog(&self, plugin_id: &str) -> Result<usize> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO tagging_jobs \
                (id, photo_id, plugin_id, status, progress, created_at) \
             SELECT lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || \
                    lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || \
                    lower(hex(randomblob(6))), \
                    p.id, ?1, 'waiting', 0, ?2 \
             FROM photos p \
             WHERE p.status IN ('present', 'replaced') \
               AND NOT EXISTS (SELECT 1 FROM photo_ai a WHERE a.photo_id = p.id) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM deletion_requests dr \
                   WHERE dr.photo_id = p.id AND dr.status IN ('pending', 'blocked', 'running') \
               ) \
             ORDER BY p.captured_at_utc DESC, p.id DESC",
            params![plugin_id, now_string()],
        )?;
        Ok(inserted)
    }

    /// Moves every `waiting` backlog job for the plugin to `pending`.
    pub fn release_waiting_tagging_jobs(&self, plugin_id: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE tagging_jobs SET status = 'pending', progress = 0, stage = NULL, \
                error = NULL, started_at = NULL, finished_at = NULL \
             WHERE plugin_id = ?1 AND status = 'waiting'",
            params![plugin_id],
        )?;
        Ok(changed)
    }

    /// Atomically claims the oldest pending job, records its start time, and
    /// stamps the active installation's variant when one exists.
    pub fn next_pending_tagging_job(&self) -> Result<Option<TaggingJob>> {
        let job = self
            .conn
            .query_row(
                "UPDATE tagging_jobs SET status = 'processing', started_at = ?1, \
                    progress = 0, stage = NULL, error = NULL, \
                    variant = COALESCE(( \
                        SELECT pi.variant FROM plugin_installations pi \
                        WHERE pi.plugin_id = tagging_jobs.plugin_id AND pi.active = 1 \
                    ), variant) \
                 WHERE id = ( \
                     SELECT id FROM tagging_jobs WHERE status = 'pending' \
                     ORDER BY created_at, rowid LIMIT 1 \
                 ) \
                 RETURNING id, photo_id, plugin_id, variant, status, progress, stage, error, \
                    retry_count, created_at, started_at, finished_at",
                params![now_string()],
                row_to_tagging_job,
            )
            .optional()?;
        Ok(job)
    }

    pub fn tagging_job(&self, job_id: &str) -> Result<Option<TaggingJob>> {
        let job = self
            .conn
            .query_row(
                &format!(
                    "SELECT {} FROM tagging_jobs WHERE id = ?1",
                    TAGGING_JOB_COLUMNS
                ),
                params![job_id],
                row_to_tagging_job,
            )
            .optional()?;
        Ok(job)
    }

    /// Records progress for a job that is still processing. Cancelled jobs
    /// keep their last visible state instead of moving backwards.
    pub fn update_tagging_progress(
        &self,
        job_id: &str,
        progress: f64,
        stage: TaggingStage,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE tagging_jobs SET progress = ?1, stage = ?2 \
             WHERE id = ?3 AND status = 'processing'",
            params![progress, stage.as_str(), job_id],
        )?;
        Ok(())
    }

    /// Marks a processing job completed. Terminal jobs are never overwritten.
    pub fn complete_tagging_job(&self, job_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tagging_jobs SET status = 'completed', progress = 1, stage = NULL, \
                error = NULL, finished_at = ?1 \
             WHERE id = ?2 AND status = 'processing'",
            params![now_string(), job_id],
        )?;
        Ok(())
    }

    /// Marks a processing job failed with a host error code.
    pub fn fail_tagging_job(&self, job_id: &str, error: TaggingErrorCode) -> Result<()> {
        self.conn.execute(
            "UPDATE tagging_jobs SET status = 'failed', progress = 0, stage = NULL, \
                error = ?1, finished_at = ?2 \
             WHERE id = ?3 AND status = 'processing'",
            params![error.as_str(), now_string(), job_id],
        )?;
        Ok(())
    }

    /// Returns a processing job to `pending` while the plugin is not ready.
    /// The error stays visible for the UI, and `retry_count` is unchanged.
    pub fn defer_tagging_job(&self, job_id: &str, error: TaggingErrorCode) -> Result<()> {
        self.conn.execute(
            "UPDATE tagging_jobs SET status = 'pending', progress = 0, stage = NULL, \
                error = ?1, started_at = NULL, finished_at = NULL \
             WHERE id = ?2 AND status = 'processing'",
            params![error.as_str(), job_id],
        )?;
        Ok(())
    }

    /// Consumes one automatic retry while the job stays claimed (`processing`).
    ///
    /// The worker backs off without releasing the job, so a cancel during the
    /// delay still sees a processing row and a daemon restart leaves enough
    /// state for `resume_interrupted_tagging_jobs` to continue the same job.
    pub fn bump_tagging_retry(&self, job_id: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE tagging_jobs SET retry_count = retry_count + 1, progress = 0, \
                stage = NULL, error = NULL \
             WHERE id = ?1 AND status = 'processing' AND retry_count < ?2",
            params![job_id, MAX_TAGGING_RETRIES],
        )?;
        Ok(changed == 1)
    }

    /// Returns a failed job to the queue while it still has retries left and
    /// no newer job for the same photo/plugin is active.
    pub fn retry_tagging_job(&self, job_id: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE tagging_jobs SET status = 'pending', retry_count = retry_count + 1, \
                progress = 0, stage = NULL, error = NULL, started_at = NULL, finished_at = NULL \
             WHERE id = ?1 AND status = 'failed' AND retry_count < ?2 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM tagging_jobs other \
                   WHERE other.photo_id = tagging_jobs.photo_id \
                     AND other.plugin_id = tagging_jobs.plugin_id \
                     AND other.id <> tagging_jobs.id \
                     AND other.status IN ('waiting', 'pending', 'processing') \
               )",
            params![job_id, MAX_TAGGING_RETRIES],
        )?;
        Ok(changed == 1)
    }

    /// Cancels an unfinished job. Returns false for terminal or missing jobs.
    pub fn cancel_tagging_job(&self, job_id: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE tagging_jobs SET status = 'cancelled', progress = 0, stage = NULL, \
                finished_at = ?1 \
             WHERE id = ?2 AND status IN ('waiting', 'pending', 'processing')",
            params![now_string(), job_id],
        )?;
        Ok(changed == 1)
    }

    /// Cancels every unfinished job for one photo, including one currently
    /// processing. Returns the number of cancelled rows.
    pub fn cancel_tagging_jobs_for_photo(&self, photo_id: i64) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE tagging_jobs SET status = 'cancelled', progress = 0, stage = NULL, \
                finished_at = ?1 \
             WHERE photo_id = ?2 AND status IN ('waiting', 'pending', 'processing')",
            params![now_string(), photo_id],
        )?;
        Ok(changed)
    }

    /// Cancels every unfinished tagging job whose photo has a pending forced
    /// deletion. `pending_deletion_requests` hides those requests while their
    /// jobs are active, so the executor runs this before collecting work. One
    /// statement keeps the per-tick cost bounded even when the deletion backlog
    /// is large.
    pub fn cancel_tagging_for_forced_deletions(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE tagging_jobs SET status = 'cancelled', progress = 0, stage = NULL, \
                finished_at = ?1 \
             WHERE status IN ('waiting', 'pending', 'processing') \
               AND photo_id IN ( \
                   SELECT photo_id FROM deletion_requests \
                   WHERE force = 1 AND status = 'pending' \
               )",
            params![now_string()],
        )?;
        Ok(changed)
    }

    /// Cancels every unfinished job for one plugin. Returns the number of
    /// cancelled rows.
    pub fn cancel_tagging_jobs_for_plugin(&self, plugin_id: &str) -> Result<usize> {
        cancel_tagging_jobs_for_plugin(&self.conn, plugin_id)
    }

    /// Returns jobs left `processing` by a crash to the queue without
    /// consuming a retry.
    pub fn resume_interrupted_tagging_jobs(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE tagging_jobs SET status = 'pending', progress = 0, stage = NULL, \
                error = NULL, started_at = NULL, finished_at = NULL \
             WHERE status = 'processing'",
            [],
        )?;
        Ok(changed)
    }

    pub fn tagging_job_counts(&self, plugin_id: &str) -> Result<JobStatusCounts> {
        let mut counts = JobStatusCounts::default();
        let mut stmt = self.conn.prepare_cached(
            "SELECT status, COUNT(*) FROM tagging_jobs WHERE plugin_id = ?1 GROUP BY status",
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (status, count) = row?;
            match TaggingJobStatus::parse(&status) {
                Some(TaggingJobStatus::Waiting) => counts.waiting += count,
                Some(TaggingJobStatus::Pending) => counts.pending += count,
                Some(TaggingJobStatus::Processing) => counts.processing += count,
                Some(TaggingJobStatus::Failed) => counts.failed += count,
                Some(TaggingJobStatus::Completed | TaggingJobStatus::Cancelled) => {}
                None => counts.pending += count,
            }
        }
        Ok(counts)
    }

    /// Newest unfinished jobs, for the settings/integrations status card.
    /// `limit` is normalized by [`clamp_limit`]. The partial index keeps this bounded
    /// even when the backlog holds six figures of waiting rows, and the
    /// explicit `INDEXED BY` prevents SQLite from falling back to a temp
    /// B-tree over every active job.
    pub fn active_tagging_jobs(&self, limit: i64) -> Result<Vec<TaggingJob>> {
        let sql = format!(
            "SELECT {} FROM tagging_jobs INDEXED BY idx_tagging_jobs_active_list \
             WHERE status IN ('waiting', 'pending', 'processing') \
             ORDER BY created_at DESC, id DESC LIMIT ?1",
            TAGGING_JOB_COLUMNS
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![clamp_limit(limit)], row_to_tagging_job)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Active jobs for the runtime card, ordered so the jobs actually running
    /// are never pushed out by a large ready queue. Processing jobs come first
    /// (oldest start first), then pending jobs (oldest first). The waiting
    /// backlog is excluded: the card never shows waiting rows, so including
    /// them in the limited snapshot only hides real work.
    pub fn active_tagging_jobs_for_ui(&self, limit: i64) -> Result<Vec<TaggingJob>> {
        let limit = clamp_limit(limit);
        let mut jobs = self.tagging_jobs_with_status(
            TaggingJobStatus::Processing,
            "COALESCE(started_at, created_at), created_at, rowid",
            limit,
        )?;
        let remaining = limit - i64::try_from(jobs.len()).unwrap_or(limit);
        if remaining > 0 {
            jobs.extend(self.tagging_jobs_with_status(
                TaggingJobStatus::Pending,
                "created_at, rowid",
                remaining,
            )?);
        }
        Ok(jobs)
    }

    fn tagging_jobs_with_status(
        &self,
        status: TaggingJobStatus,
        order: &str,
        limit: i64,
    ) -> Result<Vec<TaggingJob>> {
        let sql = format!(
            "SELECT {} FROM tagging_jobs WHERE status = ?1 ORDER BY {order} LIMIT ?2",
            TAGGING_JOB_COLUMNS
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![status.as_str(), limit], row_to_tagging_job)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn tagging_jobs_for_photo(&self, photo_id: i64) -> Result<Vec<TaggingJob>> {
        let sql = format!(
            "SELECT {} FROM tagging_jobs WHERE photo_id = ?1 \
             ORDER BY created_at, rowid",
            TAGGING_JOB_COLUMNS
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![photo_id], row_to_tagging_job)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Removes every AI result and catalog row for one plugin in a single
    /// transaction. Manual tags and other plugins' data are left untouched.
    /// Active jobs for the plugin are cancelled first so an in-flight sidecar
    /// result cannot recreate the rows after the delete. The AI revision
    /// advances only when something was removed.
    pub fn delete_ai_data(&self, plugin_id: &str) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        cancel_tagging_jobs_for_plugin(&tx, plugin_id)?;
        let mut removed = 0;
        for statement in [
            "DELETE FROM photo_ai_tags WHERE plugin_id = ?1",
            "DELETE FROM photo_ai WHERE plugin_id = ?1",
            "DELETE FROM ai_tag_definitions WHERE plugin_id = ?1",
            "DELETE FROM ai_tag_thresholds WHERE plugin_id = ?1",
        ] {
            removed += tx.execute(statement, params![plugin_id])?;
        }
        tx.commit()?;
        if removed > 0 {
            self.bump_ai_revision()?;
        }
        Ok(())
    }
}

fn write_photo_ai(
    conn: &Connection,
    photo_id: i64,
    plugin_id: &str,
    result: &NewPhotoAi,
) -> Result<bool> {
    let visible: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM photos \
         WHERE id = ?1 AND status IN ('present', 'replaced'))",
        params![photo_id],
        |row| row.get(0),
    )?;
    if !visible {
        return Ok(false);
    }

    let now = now_string();
    conn.execute(
        "INSERT INTO photo_ai(photo_id, plugin_id, variant, sidecar_version, model_version, \
            model_revision, catalog_version, source, presence_threshold, character_present, \
            presence_probability, people_min, people_max, people_median, place_branch, \
            place_probabilities_json, tagged_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17) \
         ON CONFLICT(photo_id) DO UPDATE SET \
            plugin_id = excluded.plugin_id, \
            variant = excluded.variant, \
            sidecar_version = excluded.sidecar_version, \
            model_version = excluded.model_version, \
            model_revision = excluded.model_revision, \
            catalog_version = excluded.catalog_version, \
            source = excluded.source, \
            presence_threshold = excluded.presence_threshold, \
            character_present = excluded.character_present, \
            presence_probability = excluded.presence_probability, \
            people_min = excluded.people_min, \
            people_max = excluded.people_max, \
            people_median = excluded.people_median, \
            place_branch = excluded.place_branch, \
            place_probabilities_json = excluded.place_probabilities_json, \
            tagged_at = excluded.tagged_at",
        params![
            photo_id,
            plugin_id,
            result.variant,
            result.sidecar_version,
            result.model_version,
            result.model_revision,
            result.catalog_version,
            result.source,
            result.presence_threshold,
            i64::from(result.character_present),
            result.presence_probability,
            result.people_min.map(i64::from),
            result.people_max.map(i64::from),
            result.people_median.map(i64::from),
            result.place_branch,
            result.place_probabilities_json,
            now,
        ],
    )?;

    // Re-tagging may move the photo to another branch, so replace the whole
    // judged set for this plugin instead of only upserting the new keys.
    conn.execute(
        "DELETE FROM photo_ai_tags WHERE photo_id = ?1 AND plugin_id = ?2",
        params![photo_id, plugin_id],
    )?;
    for (tag_key, probability) in &result.tags {
        conn.execute(
            "INSERT INTO photo_ai_tags(photo_id, plugin_id, tag_key, probability, applied, \
                model_version, catalog_version, tagged_at) \
             VALUES (?1, ?2, ?3, ?4, CASE WHEN ?4 >= COALESCE( \
                (SELECT threshold FROM ai_tag_thresholds \
                 WHERE plugin_id = ?2 AND tag_key = ?3), \
                (SELECT default_threshold FROM ai_tag_definitions \
                 WHERE plugin_id = ?2 AND tag_key = ?3)) \
             THEN 1 ELSE 0 END, ?5, ?6, ?7)",
            params![
                photo_id,
                plugin_id,
                tag_key,
                probability,
                result.model_version,
                result.catalog_version,
                now,
            ],
        )?;
    }
    Ok(true)
}

const TAGGING_JOB_COLUMNS: &str = "id, photo_id, plugin_id, variant, status, progress, stage, \
    error, retry_count, created_at, started_at, finished_at";

fn cancel_tagging_jobs_for_plugin(conn: &Connection, plugin_id: &str) -> Result<usize> {
    let changed = conn.execute(
        "UPDATE tagging_jobs SET status = 'cancelled', progress = 0, stage = NULL, \
            finished_at = ?1 \
         WHERE plugin_id = ?2 AND status IN ('waiting', 'pending', 'processing')",
        params![now_string(), plugin_id],
    )?;
    Ok(changed)
}

fn row_to_photo_ai(row: &Row<'_>) -> rusqlite::Result<PhotoAi> {
    Ok(PhotoAi {
        photo_id: row.get(0)?,
        plugin_id: row.get(1)?,
        variant: row.get(2)?,
        sidecar_version: row.get(3)?,
        model_version: row.get(4)?,
        model_revision: row.get(5)?,
        catalog_version: row.get(6)?,
        source: row.get(7)?,
        presence_threshold: row.get(8)?,
        character_present: row.get::<_, i64>(9)? != 0,
        presence_probability: row.get(10)?,
        people_min: row
            .get::<_, Option<i64>>(11)?
            .map(|value| value.max(0) as u32),
        people_max: row
            .get::<_, Option<i64>>(12)?
            .map(|value| value.max(0) as u32),
        people_median: row
            .get::<_, Option<i64>>(13)?
            .map(|value| value.max(0) as u32),
        place_branch: row.get(14)?,
        place_probabilities_json: row.get(15)?,
        tagged_at: row.get(16)?,
    })
}

fn row_to_photo_ai_tag(row: &Row<'_>) -> rusqlite::Result<PhotoAiTag> {
    Ok(PhotoAiTag {
        photo_id: row.get(0)?,
        plugin_id: row.get(1)?,
        tag_key: row.get(2)?,
        probability: row.get(3)?,
        applied: row.get::<_, i64>(4)? != 0,
        model_version: row.get(5)?,
        catalog_version: row.get(6)?,
        tagged_at: row.get(7)?,
    })
}

fn row_to_tagging_job(row: &Row<'_>) -> rusqlite::Result<TaggingJob> {
    let status = row.get::<_, String>(4)?;
    Ok(TaggingJob {
        id: row.get(0)?,
        photo_id: row.get(1)?,
        plugin_id: row.get(2)?,
        variant: row.get(3)?,
        status: TaggingJobStatus::parse(&status).unwrap_or(TaggingJobStatus::Pending),
        progress: row.get(5)?,
        stage: row
            .get::<_, Option<String>>(6)?
            .and_then(|stage| TaggingStage::parse(&stage)),
        error: row.get(7)?,
        retry_count: row.get(8)?,
        created_at: row.get(9)?,
        started_at: row.get(10)?,
        finished_at: row.get(11)?,
    })
}

fn reapply_thresholds(conn: &Connection, plugin_id: &str) -> Result<usize> {
    let updated = conn.execute(
        "UPDATE photo_ai_tags SET applied = CASE WHEN probability >= COALESCE( \
            (SELECT t.threshold FROM ai_tag_thresholds t \
             WHERE t.plugin_id = photo_ai_tags.plugin_id \
               AND t.tag_key = photo_ai_tags.tag_key), \
            (SELECT d.default_threshold FROM ai_tag_definitions d \
             WHERE d.plugin_id = photo_ai_tags.plugin_id \
               AND d.tag_key = photo_ai_tags.tag_key)) \
         THEN 1 ELSE 0 END \
         WHERE plugin_id = ?1",
        params![plugin_id],
    )?;
    Ok(updated)
}

fn row_to_plugin_installation(row: &Row<'_>) -> rusqlite::Result<PluginInstallation> {
    Ok(PluginInstallation {
        plugin_id: row.get(0)?,
        variant: row.get(1)?,
        version: row.get(2)?,
        protocol_version: row.get(3)?,
        path: row.get(4)?,
        manifest_json: row.get(5)?,
        catalog_version: row.get(6)?,
        active: row.get::<_, i64>(7)? != 0,
        installed_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn row_to_plugin_state(row: &Row<'_>) -> rusqlite::Result<PluginState> {
    let model_state: String = row.get(1)?;
    let model_state = ModelState::parse(&model_state).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("unknown model state '{model_state}'").into(),
        )
    })?;
    Ok(PluginState {
        plugin_id: row.get(0)?,
        model_state,
        model_revision: row.get(2)?,
        model_bytes_downloaded: row.get(3)?,
        model_bytes_total: row.get(4)?,
        model_error: row.get(5)?,
        probe_json: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn row_to_ai_tag_definition(row: &Row<'_>) -> rusqlite::Result<AiTagDefinition> {
    Ok(AiTagDefinition {
        plugin_id: row.get(0)?,
        tag_key: row.get(1)?,
        branch: row.get(2)?,
        tag_group: row.get(3)?,
        name: row.get(4)?,
        name_en: row.get(5)?,
        description: row.get(6)?,
        default_threshold: row.get(7)?,
        sort_order: row.get(8)?,
        catalog_version: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn row_to_ai_tag_threshold(row: &Row<'_>) -> rusqlite::Result<AiTagThreshold> {
    Ok(AiTagThreshold {
        plugin_id: row.get(0)?,
        tag_key: row.get(1)?,
        threshold: row.get(2)?,
        updated_at: row.get(3)?,
    })
}
