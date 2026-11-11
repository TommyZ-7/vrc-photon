use super::*;

impl Db {
    // ---------- send outbox ----------

    /// Columns read for every `send_outbox` row, in `row_to_send_outbox` order.
    const SEND_COLUMNS: &str = "id, photo_id, kind, origin, status, source, profile_id, \
        destination, attempts, next_attempt_at, discord_message_id, discord_channel_id, \
        discord_guild_id, error_code, error_detail, created_at, started_at, finished_at";

    /// Same columns qualified for the `send_history` join, which also reads
    /// `photo_name` and `world_name` at indexes 18 and 19.
    const SEND_COLUMNS_QUALIFIED: &str = "o.id, o.photo_id, o.kind, o.origin, o.status, \
        o.source, o.profile_id, o.destination, o.attempts, o.next_attempt_at, \
        o.discord_message_id, o.discord_channel_id, o.discord_guild_id, o.error_code, \
        o.error_detail, o.created_at, o.started_at, o.finished_at";

    /// Queues a photo send unless the photo is being deleted or another active
    /// send already exists. The partial `idx_send_outbox_active_photo` index is
    /// what dedupes active rows; `INSERT OR IGNORE` turns that conflict into a
    /// quiet `None`. Only `waiting` and `pending` rows may be enqueued.
    pub fn enqueue_send(
        &self,
        photo_id: i64,
        kind: SendKind,
        origin: SendOrigin,
        status: SendStatus,
    ) -> Result<Option<i64>> {
        if !matches!(status, SendStatus::Waiting | SendStatus::Pending) {
            return Err(CoreError::Invalid(format!(
                "send status '{}' cannot be enqueued",
                status.as_str()
            )));
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO send_outbox(photo_id, kind, origin, status, created_at) \
             SELECT ?1, ?2, ?3, ?4, ?5 \
             WHERE EXISTS (SELECT 1 FROM photos WHERE id = ?1 AND status <> 'deleting')",
            params![
                photo_id,
                kind.as_str(),
                origin.as_str(),
                status.as_str(),
                now_string()
            ],
        )?;
        if self.conn.changes() == 0 {
            return Ok(None);
        }
        Ok(Some(self.conn.last_insert_rowid()))
    }

    /// Queues a webhook test send. Test rows have no photo, so they are never
    /// covered by the per-photo dedupe index.
    pub fn enqueue_test_send(&self) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO send_outbox(photo_id, kind, origin, status, created_at) \
             VALUES (NULL, 'test', 'manual', 'pending', ?1)",
            params![now_string()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Atomically claims the oldest due pending send. `now` is epoch seconds;
    /// `next_attempt_at` is compared against it and left untouched.
    pub fn claim_next_send(&self, now: i64) -> Result<Option<SendOutboxEntry>> {
        let sql = format!(
            "UPDATE send_outbox SET status = 'processing', started_at = ?1 \
             WHERE id = ( \
                 SELECT id FROM send_outbox \
                 WHERE status = 'pending' \
                   AND (next_attempt_at IS NULL OR next_attempt_at <= ?2) \
                 ORDER BY id LIMIT 1 \
             ) \
             RETURNING {}",
            Self::SEND_COLUMNS
        );
        let entry = self
            .conn
            .query_row(&sql, params![now_string(), now], row_to_send_outbox)
            .optional()?;
        Ok(entry)
    }

    /// Delays the next claim of `id` until the given epoch second.
    pub fn set_send_next_attempt(&self, id: i64, next_attempt_at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE send_outbox SET next_attempt_at = ?1 WHERE id = ?2",
            params![next_attempt_at, id],
        )?;
        Ok(())
    }

    pub fn send_entry_by_id(&self, id: i64) -> Result<Option<SendOutboxEntry>> {
        let sql = format!(
            "SELECT {} FROM send_outbox WHERE id = ?1",
            Self::SEND_COLUMNS
        );
        let entry = self
            .conn
            .query_row(&sql, params![id], row_to_send_outbox)
            .optional()?;
        Ok(entry)
    }

    /// Returns the active (`waiting` / `pending` / `processing`) row for a
    /// photo, if any. The partial UNIQUE index guarantees at most one.
    pub fn active_send_for_photo(&self, photo_id: i64) -> Result<Option<SendOutboxEntry>> {
        let sql = format!(
            "SELECT {} FROM send_outbox WHERE photo_id = ?1 \
             AND status IN ('waiting', 'pending', 'processing') ORDER BY id LIMIT 1",
            Self::SEND_COLUMNS
        );
        let entry = self
            .conn
            .query_row(&sql, params![photo_id], row_to_send_outbox)
            .optional()?;
        Ok(entry)
    }

    /// Counts work a user still expects to start: waiting and pending sends.
    pub fn count_pending_sends(&self) -> Result<i64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM send_outbox WHERE status IN ('waiting', 'pending')",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Counts only sends held in automatic `waiting` mode.
    pub fn count_waiting_sends(&self) -> Result<i64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM send_outbox WHERE status = 'waiting'",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Marks a claimed send as sent and stores the Discord identity produced by
    /// the upload. Rows that are not `processing` are left untouched.
    pub fn complete_send(&self, id: i64, completion: &SendCompletion) -> Result<()> {
        let source = completion.source.map(SendSource::as_str);
        self.conn.execute(
            "UPDATE send_outbox SET status = 'sent', source = ?1, profile_id = ?2, \
                destination = ?3, discord_message_id = ?4, discord_channel_id = ?5, \
                discord_guild_id = ?6, error_code = NULL, error_detail = NULL, \
                finished_at = ?7 \
             WHERE id = ?8 AND status = 'processing'",
            params![
                source,
                completion.profile_id,
                completion.destination,
                completion.message_id,
                completion.channel_id,
                completion.guild_id,
                now_string(),
                id,
            ],
        )?;
        Ok(())
    }

    /// Delays a send and records the last error without increasing `attempts`
    /// beyond what the caller computed.
    pub fn reschedule_send(
        &self,
        id: i64,
        attempts: i64,
        next_attempt_at: i64,
        error_code: SendErrorCode,
        error_detail: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE send_outbox SET status = 'pending', attempts = ?1, \
                next_attempt_at = ?2, error_code = ?3, error_detail = ?4, \
                started_at = NULL \
             WHERE id = ?5",
            params![
                attempts,
                next_attempt_at,
                error_code.as_str(),
                error_detail,
                id,
            ],
        )?;
        Ok(())
    }

    /// Returns a claimed send to the queue without uploading. Used when the
    /// Discord integration was disabled between claim and upload.
    pub fn release_send(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE send_outbox SET status = 'pending', started_at = NULL \
             WHERE id = ?1 AND status = 'processing'",
            params![id],
        )?;
        Ok(())
    }

    /// Marks a send as failed and stores the error plus whatever Discord
    /// identity was known. Pending rows are accepted: a send can be abandoned
    /// before it is ever claimed.
    pub fn fail_send(
        &self,
        id: i64,
        error_code: SendErrorCode,
        error_detail: Option<&str>,
        source: Option<SendSource>,
        profile_id: Option<&str>,
        destination: Option<&str>,
    ) -> Result<()> {
        let source = source.map(SendSource::as_str);
        self.conn.execute(
            "UPDATE send_outbox SET status = 'failed', error_code = ?1, \
                error_detail = ?2, source = ?3, profile_id = ?4, destination = ?5, \
                finished_at = ?6 \
             WHERE id = ?7",
            params![
                error_code.as_str(),
                error_detail,
                source,
                profile_id,
                destination,
                now_string(),
                id,
            ],
        )?;
        Ok(())
    }

    /// Marks a send as failed after the automatic retry budget is exhausted.
    ///
    /// Unlike [`Db::fail_send`], this stores the final attempt count so the
    /// history can show how many automatic retries were spent, and clears the
    /// pending retry timestamp because no further attempt is scheduled.
    #[allow(clippy::too_many_arguments)]
    pub fn fail_send_after_attempts(
        &self,
        id: i64,
        attempts: i64,
        error_code: SendErrorCode,
        error_detail: Option<&str>,
        source: Option<SendSource>,
        profile_id: Option<&str>,
        destination: Option<&str>,
    ) -> Result<()> {
        let source = source.map(SendSource::as_str);
        self.conn.execute(
            "UPDATE send_outbox SET status = 'failed', attempts = ?1, error_code = ?2, \
                error_detail = ?3, source = ?4, profile_id = ?5, destination = ?6, \
                next_attempt_at = NULL, finished_at = ?7 \
             WHERE id = ?8",
            params![
                attempts,
                error_code.as_str(),
                error_detail,
                source,
                profile_id,
                destination,
                now_string(),
                id,
            ],
        )?;
        Ok(())
    }

    /// Returns a failed send to the queue with a clean slate. Refuses when the
    /// same photo already has another active row, since that would violate the
    /// partial `idx_send_outbox_active_photo` index.
    pub fn retry_send(&self, id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE send_outbox SET status = 'pending', attempts = 0, \
                next_attempt_at = NULL, error_code = NULL, error_detail = NULL, \
                started_at = NULL, finished_at = NULL \
             WHERE id = ?1 AND status = 'failed' \
               AND (photo_id IS NULL OR NOT EXISTS ( \
                   SELECT 1 FROM send_outbox active \
                   WHERE active.photo_id = send_outbox.photo_id \
                     AND active.id <> send_outbox.id \
                     AND active.status IN ('waiting', 'pending', 'processing') \
               ))",
            params![id],
        )?;
        Ok(changed == 1)
    }

    /// Cancels a queued or failed send. Claimed (`processing`) rows are left
    /// alone so an upload in flight can finish.
    pub fn cancel_send(&self, id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE send_outbox SET status = 'cancelled', finished_at = ?1 \
             WHERE id = ?2 AND status IN ('waiting', 'pending', 'failed')",
            params![now_string(), id],
        )?;
        Ok(changed == 1)
    }

    /// Cancels every automatic send still held in `waiting` mode.
    pub fn cancel_waiting_auto_sends(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE send_outbox SET status = 'cancelled', finished_at = ?1 \
             WHERE status = 'waiting' AND origin = 'auto'",
            params![now_string()],
        )?;
        Ok(changed)
    }

    /// Promotes every waiting send to pending so workers can pick it up.
    pub fn release_waiting_sends(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE send_outbox SET status = 'pending' WHERE status = 'waiting'",
            [],
        )?;
        Ok(changed)
    }

    /// Fails sends that were still `processing` when the process stopped.
    /// They are not retried automatically, so a possibly delivered message is
    /// not sent twice.
    pub fn reset_processing_sends(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE send_outbox SET status = 'failed', error_code = 'interrupted', \
                error_detail = NULL, finished_at = ?1 \
             WHERE status = 'processing'",
            params![now_string()],
        )?;
        Ok(changed)
    }

    /// Newest-first send history joined with the photo file name and world
    /// name. `limit` is normalized by [`clamp_limit`].
    pub fn send_history(&self, limit: i64) -> Result<Vec<SendHistoryEntry>> {
        let sql = format!(
            "SELECT {}, p.file_name, c.world_name \
             FROM send_outbox o \
             LEFT JOIN photos p ON p.id = o.photo_id \
             LEFT JOIN photo_context c ON c.photo_id = o.photo_id \
             ORDER BY o.id DESC LIMIT ?1",
            Self::SEND_COLUMNS_QUALIFIED
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![clamp_limit(limit)], |row| {
            Ok(SendHistoryEntry {
                entry: row_to_send_outbox(row)?,
                photo_name: row.get(18)?,
                world_name: row.get(19)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Deletes every terminal send row and returns how many were removed.
    pub fn clear_send_history(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM send_outbox WHERE status IN ('sent', 'failed', 'cancelled')",
            [],
        )?;
        Ok(changed)
    }

    /// Deletes terminal send rows older than the newest `keep` ones. Active
    /// rows are never pruned.
    ///
    /// `keep` deliberately does not use [`clamp_limit`]: zero means "keep no
    /// terminal rows", and negative values behave the same way. This is the
    /// prune operation's existing explicit policy, not a list limit.
    pub fn prune_send_history(&self, keep: i64) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM send_outbox WHERE status IN ('sent', 'failed', 'cancelled') \
               AND id NOT IN ( \
                   SELECT id FROM send_outbox \
                   WHERE status IN ('sent', 'failed', 'cancelled') \
                   ORDER BY id DESC LIMIT ?1 \
               )",
            params![keep.max(0)],
        )?;
        Ok(changed)
    }
}

fn row_to_send_outbox(row: &Row<'_>) -> rusqlite::Result<SendOutboxEntry> {
    Ok(SendOutboxEntry {
        id: row.get(0)?,
        photo_id: row.get(1)?,
        kind: SendKind::parse(&row.get::<_, String>(2)?),
        origin: SendOrigin::parse(&row.get::<_, String>(3)?),
        status: SendStatus::parse(&row.get::<_, String>(4)?),
        source: row
            .get::<_, Option<String>>(5)?
            .map(|value| SendSource::parse(&value)),
        profile_id: row.get(6)?,
        destination: row.get(7)?,
        attempts: row.get(8)?,
        next_attempt_at: row.get(9)?,
        discord_message_id: row.get(10)?,
        discord_channel_id: row.get(11)?,
        discord_guild_id: row.get(12)?,
        error_code: row.get(13)?,
        error_detail: row.get(14)?,
        created_at: row.get(15)?,
        started_at: row.get(16)?,
        finished_at: row.get(17)?,
    })
}
