use super::photos::*;
use super::*;

impl Db {
    // ---------- world name lookups ----------

    /// Fills NULL `photo_context.world_name` rows from the stored world name,
    /// preferring the user alias. Returns the number of updated contexts.
    pub fn backfill_photo_context_world_names(&self) -> Result<usize> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let updated = tx.execute(
            "UPDATE photo_context SET world_name = ( \
                 SELECT COALESCE(w.alias, w.name) FROM worlds w \
                 WHERE w.id = photo_context.world_id \
             ) \
             WHERE world_name IS NULL AND world_id IS NOT NULL \
               AND EXISTS (SELECT 1 FROM worlds w \
                           WHERE w.id = photo_context.world_id \
                             AND COALESCE(w.alias, w.name) IS NOT NULL)",
            [],
        )?;
        if updated > 0 {
            self.bump_library_revision()?;
        }
        tx.commit()?;
        Ok(updated)
    }

    /// Queues a world for name resolution unless it is already queued.
    pub fn enqueue_world_name_lookup(&self, world_db_id: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO world_name_lookups(world_db_id, status, updated_at) \
             SELECT ?1, 'pending', ?2 \
             WHERE EXISTS (SELECT 1 FROM worlds WHERE id = ?1)",
            params![world_db_id, now_string()],
        )?;
        Ok(())
    }

    /// Queues every world referenced by a photo that has no name anywhere yet.
    /// Idempotent: existing lookup rows are never modified.
    pub fn enqueue_missing_world_name_lookups(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "INSERT INTO world_name_lookups(world_db_id, status, updated_at) \
             SELECT w.id, 'pending', ?1 FROM worlds w \
             WHERE w.alias IS NULL AND w.name IS NULL \
               AND EXISTS (SELECT 1 FROM photo_context c \
                           WHERE c.world_id = w.id AND c.world_name IS NULL) \
               AND NOT EXISTS (SELECT 1 FROM world_name_lookups l \
                               WHERE l.world_db_id = w.id)",
            params![now_string()],
        )?;
        Ok(changed)
    }

    /// Oldest due lookup. `pending`, `retry`, and `not_found` rows are
    /// eligible; rows scheduled in the future are skipped.
    pub fn next_due_world_name_lookup(&self, now: i64) -> Result<Option<WorldNameLookup>> {
        let lookup = self
            .conn
            .query_row(
                "SELECT l.world_db_id, w.world_id, l.status, l.attempts, l.next_attempt_at \
                 FROM world_name_lookups l JOIN worlds w ON w.id = l.world_db_id \
                 WHERE l.status IN ('pending', 'retry', 'not_found') \
                   AND (l.next_attempt_at IS NULL OR l.next_attempt_at <= ?1) \
                 ORDER BY COALESCE(l.next_attempt_at, 0), l.world_db_id LIMIT 1",
                params![now],
                row_to_world_name_lookup,
            )
            .optional()?;
        Ok(lookup)
    }

    pub fn world_name_lookup(&self, world_db_id: i64) -> Result<Option<WorldNameLookup>> {
        let lookup = self
            .conn
            .query_row(
                "SELECT l.world_db_id, w.world_id, l.status, l.attempts, l.next_attempt_at \
                 FROM world_name_lookups l JOIN worlds w ON w.id = l.world_db_id \
                 WHERE l.world_db_id = ?1",
                params![world_db_id],
                row_to_world_name_lookup,
            )
            .optional()?;
        Ok(lookup)
    }

    /// Marks a queued lookup resolved because a log/XMP name already exists.
    pub fn mark_world_name_lookup_satisfied(&self, world_db_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE world_name_lookups SET status = 'resolved', next_attempt_at = NULL, \
                last_error = NULL, updated_at = ?1 \
             WHERE world_db_id = ?2 AND status IN ('pending', 'retry', 'not_found')",
            params![now_string(), world_db_id],
        )?;
        Ok(())
    }

    pub fn mark_world_name_lookup_resolved(&self, world_db_id: i64, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE world_name_lookups SET status = 'resolved', next_attempt_at = NULL, \
                last_error = NULL, last_attempt_at = ?1, updated_at = ?2 \
             WHERE world_db_id = ?3 AND status IN ('pending', 'retry', 'not_found')",
            params![now, now_string(), world_db_id],
        )?;
        Ok(())
    }

    /// Stores an API result without overriding a log/XMP name. NULL photo
    /// contexts are filled with the effective stored name either way.
    pub fn record_world_name_resolved(&self, world_db_id: i64, name: &str, now: i64) -> Result<()> {
        let Some(world) = self.world_by_id(world_db_id)? else {
            return Ok(());
        };
        self.upsert_world(&world.world_id, Some(name), Some("vrchat-api"))?;
        self.mark_world_name_lookup_resolved(world_db_id, now)?;
        Ok(())
    }

    /// Records the outcome of a failed attempt and schedules the next try
    /// according to the shared retry policy. 429s do not count as attempts
    /// and never schedule sooner than the global request interval.
    pub fn record_world_name_failure(
        &self,
        world_db_id: i64,
        failure: WorldNameFailure,
        now: i64,
    ) -> Result<()> {
        // Read-then-write must take the write lock up front, like
        // `upsert_world`: a deferred transaction can hit SQLITE_BUSY_SNAPSHOT
        // when the UI writes between our SELECT and UPDATE.
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let attempts = tx
            .query_row(
                "SELECT attempts FROM world_name_lookups WHERE world_db_id = ?1 \
                 AND status IN ('pending', 'retry', 'not_found')",
                params![world_db_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(attempts) = attempts else {
            return Ok(());
        };

        let (status, attempts, next_attempt_at, last_error) = match failure {
            WorldNameFailure::RateLimited { retry_after } => (
                WorldNameLookupStatus::Retry,
                attempts,
                Some(now + retry_after.max(REQUEST_INTERVAL_SECONDS)),
                "http_429",
            ),
            WorldNameFailure::Retryable { code } => {
                let attempts = attempts + 1;
                (
                    WorldNameLookupStatus::Retry,
                    attempts,
                    Some(now + backoff_seconds(attempts)),
                    code,
                )
            }
            WorldNameFailure::NotFound => (
                WorldNameLookupStatus::NotFound,
                attempts + 1,
                Some(now + NOT_FOUND_RETRY_SECONDS),
                "not_found",
            ),
            WorldNameFailure::Invalid { code } => {
                (WorldNameLookupStatus::Failed, attempts + 1, None, code)
            }
        };

        tx.execute(
            "UPDATE world_name_lookups SET status = ?1, attempts = ?2, next_attempt_at = ?3, \
                last_attempt_at = ?4, last_error = ?5, updated_at = ?6 \
             WHERE world_db_id = ?7",
            params![
                status.as_str(),
                attempts,
                next_attempt_at,
                now,
                last_error,
                now_string(),
                world_db_id,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn world_name_next_request_at(&self) -> Result<Option<i64>> {
        Ok(self
            .get_setting("world_name_next_request_at")?
            .and_then(|value| value.parse().ok()))
    }

    pub fn set_world_name_next_request_at(&self, timestamp: i64) -> Result<()> {
        self.set_setting("world_name_next_request_at", &timestamp.to_string())
    }

    pub fn world_name_lookup_counts(&self) -> Result<WorldNameLookupCounts> {
        let (pending, resolved, failed) = self.conn.query_row(
            "SELECT \
                COALESCE(SUM(CASE WHEN status IN ('pending', 'retry', 'not_found') \
                    THEN 1 ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN status = 'resolved' THEN 1 ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0) \
             FROM world_name_lookups",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok(WorldNameLookupCounts {
            pending,
            resolved,
            failed,
        })
    }

    pub fn create_session(
        &self,
        log_file: &str,
        started_at: i64,
        world_id: i64,
        instance_id: Option<&str>,
    ) -> Result<i64> {
        let id = self.conn.query_row(
            "INSERT INTO sessions(log_file, started_at, world_id, instance_id) \
             VALUES (?1, ?2, ?3, ?4) RETURNING id",
            params![log_file, started_at, world_id, instance_id],
            |row| row.get(0),
        )?;
        self.bump_session_revision()?;
        Ok(id)
    }

    pub fn close_session(&self, session_id: i64, ended_at: i64) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE id = ?2 AND ended_at IS NULL",
            params![ended_at, session_id],
        )?;
        if updated > 0 {
            self.bump_session_revision()?;
        }
        Ok(())
    }

    pub fn open_sessions(&self) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, log_file, started_at, ended_at, world_id, instance_id \
             FROM sessions WHERE ended_at IS NULL ORDER BY started_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                log_file: row.get(1)?,
                started_at: row.get(2)?,
                ended_at: row.get(3)?,
                world_id: row.get(4)?,
                instance_id: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn session_by_id(&self, id: i64) -> Result<Option<Session>> {
        let session = self
            .conn
            .query_row(
                "SELECT id, log_file, started_at, ended_at, world_id, instance_id \
                 FROM sessions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Session {
                        id: row.get(0)?,
                        log_file: row.get(1)?,
                        started_at: row.get(2)?,
                        ended_at: row.get(3)?,
                        world_id: row.get(4)?,
                        instance_id: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(session)
    }

    pub fn session_at(&self, timestamp: i64) -> Result<Option<Session>> {
        let session = self
            .conn
            .query_row(
                "SELECT id, log_file, started_at, ended_at, world_id, instance_id \
                 FROM sessions WHERE started_at <= ?1 AND (ended_at IS NULL OR ?1 < ended_at) \
                 ORDER BY started_at DESC LIMIT 1",
                params![timestamp],
                |row| {
                    Ok(Session {
                        id: row.get(0)?,
                        log_file: row.get(1)?,
                        started_at: row.get(2)?,
                        ended_at: row.get(3)?,
                        world_id: row.get(4)?,
                        instance_id: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(session)
    }

    /// Sessions that do not contain `timestamp` but end or start within
    /// `window_seconds` of it. At most two are returned: callers only need
    /// to distinguish none, exactly one, and ambiguous.
    pub fn sessions_near(&self, timestamp: i64, window_seconds: i64) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare_cached(SESSIONS_NEAR_SQL)?;
        let rows = stmt.query_map(params![timestamp, window_seconds], |row| {
            Ok(Session {
                id: row.get(0)?,
                log_file: row.get(1)?,
                started_at: row.get(2)?,
                ended_at: row.get(3)?,
                world_id: row.get(4)?,
                instance_id: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_session_player(
        &self,
        session_id: i64,
        user_id: &str,
        display_name: &str,
        joined_at: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_players(session_id, user_id, display_name, joined_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(session_id, user_id) DO UPDATE SET \
                display_name = excluded.display_name, left_at = NULL",
            params![session_id, user_id, display_name, joined_at],
        )?;
        self.bump_session_revision()?;
        Ok(())
    }

    pub fn close_session_player(&self, session_id: i64, user_id: &str, left_at: i64) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE session_players SET left_at = ?1 \
             WHERE session_id = ?2 AND user_id = ?3 AND left_at IS NULL",
            params![left_at, session_id, user_id],
        )?;
        if updated > 0 {
            self.bump_session_revision()?;
        }
        Ok(())
    }

    /// Closes the open player row and stores the name reported by the leave
    /// line, so a rename during the session is visible on the player's row.
    pub fn close_session_player_with_name(
        &self,
        session_id: i64,
        user_id: &str,
        display_name: &str,
        left_at: i64,
    ) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE session_players SET display_name = ?1, left_at = ?2 \
             WHERE session_id = ?3 AND user_id = ?4 AND left_at IS NULL",
            params![display_name, left_at, session_id, user_id],
        )?;
        if updated > 0 {
            self.bump_session_revision()?;
        }
        Ok(())
    }

    pub fn players_at(&self, session_id: i64, timestamp: i64) -> Result<Vec<SessionPlayer>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, session_id, user_id, display_name, joined_at, left_at \
             FROM session_players WHERE session_id = ?1 AND joined_at <= ?2 \
             AND (left_at IS NULL OR ?2 < left_at) ORDER BY joined_at",
        )?;
        let rows = stmt.query_map(params![session_id, timestamp], |row| {
            Ok(SessionPlayer {
                id: row.get(0)?,
                session_id: row.get(1)?,
                user_id: row.get(2)?,
                display_name: row.get(3)?,
                joined_at: row.get(4)?,
                left_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Returns display metadata for the given sessions.
    ///
    /// The UI requests only the sessions visible in the current grid page.
    /// Duplicate IDs are removed and the input is capped at
    /// [`MAX_SESSION_SUMMARY_IDS`]. Unknown IDs are skipped. The result is
    /// ordered by `started_at, id`.
    pub fn session_summaries(&self, ids: &[i64]) -> Result<Vec<SessionSummary>> {
        let mut unique_ids = Vec::with_capacity(ids.len().min(MAX_SESSION_SUMMARY_IDS));
        for id in ids {
            if !unique_ids.contains(id) {
                unique_ids.push(*id);
                if unique_ids.len() == MAX_SESSION_SUMMARY_IDS {
                    break;
                }
            }
        }
        if unique_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; unique_ids.len()].join(",");

        let sessions: Vec<(Session, Option<String>, Option<String>)> = {
            let sql = format!(
                "SELECT s.id, s.log_file, s.started_at, s.ended_at, s.world_id, s.instance_id, \
                    COALESCE(w.alias, w.name, w.world_id, ''), w.world_id \
                 FROM sessions s LEFT JOIN worlds w ON w.id = s.world_id \
                 WHERE s.id IN ({placeholders}) ORDER BY s.started_at, s.id"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(unique_ids.iter().copied()), |row| {
                let world_name: String = row.get(6)?;
                Ok((
                    Session {
                        id: row.get(0)?,
                        log_file: row.get(1)?,
                        started_at: row.get(2)?,
                        ended_at: row.get(3)?,
                        world_id: row.get(4)?,
                        instance_id: row.get(5)?,
                    },
                    (!world_name.is_empty()).then_some(world_name),
                    row.get(7)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if sessions.is_empty() {
            return Ok(Vec::new());
        }

        let counts: HashMap<i64, i64> = {
            let sql = format!(
                "SELECT c.session_id, COUNT(*) FROM photo_context c \
                 JOIN photos p ON p.id = c.photo_id \
                 WHERE c.session_id IN ({placeholders}) AND {VISIBLE_STATUS} \
                 GROUP BY c.session_id"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(unique_ids.iter().copied()), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
            rows.collect::<rusqlite::Result<HashMap<_, _>>>()?
        };

        let mut players: HashMap<i64, Vec<String>> = HashMap::new();
        {
            let sql = format!(
                "SELECT session_id, display_name FROM session_players \
                 WHERE session_id IN ({placeholders}) ORDER BY session_id, joined_at, id"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(unique_ids.iter().copied()), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (session_id, display_name) = row?;
                players.entry(session_id).or_default().push(display_name);
            }
        }

        let cover_ids: HashMap<i64, i64> = {
            let sql = format!(
                "SELECT c.session_id, p.id FROM photo_context c \
                 JOIN photos p ON p.id = c.photo_id \
                 WHERE c.session_id IN ({placeholders}) AND {VISIBLE_STATUS} \
                 ORDER BY c.session_id, p.is_favorite DESC, p.captured_at_utc DESC, p.id DESC"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(unique_ids.iter().copied()), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?;
            let mut covers = HashMap::new();
            for row in rows {
                let (session_id, photo_id) = row?;
                covers.entry(session_id).or_insert(photo_id);
            }
            covers
        };
        let mut cover_photos: HashMap<i64, Photo> = if cover_ids.is_empty() {
            HashMap::new()
        } else {
            let mut ids: Vec<i64> = cover_ids.values().copied().collect();
            ids.sort_unstable();
            self.photos_by_ids(&ids)?
                .into_iter()
                .map(|photo| (photo.id, photo))
                .collect()
        };

        let mut summaries = Vec::with_capacity(sessions.len());
        for (session, world_name, world_external_id) in sessions {
            let cover_photo = cover_ids
                .get(&session.id)
                .and_then(|photo_id| cover_photos.remove(photo_id));
            summaries.push(SessionSummary {
                photo_count: counts.get(&session.id).copied().unwrap_or(0),
                players: players.remove(&session.id).unwrap_or_default(),
                session,
                world_name,
                world_external_id,
                cover_photo,
            });
        }
        Ok(summaries)
    }
}

fn row_to_world_name_lookup(row: &Row<'_>) -> rusqlite::Result<WorldNameLookup> {
    let status: String = row.get(2)?;
    let status = WorldNameLookupStatus::parse(&status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            format!("unknown world name lookup status '{status}'").into(),
        )
    })?;
    Ok(WorldNameLookup {
        world_db_id: row.get(0)?,
        world_id: row.get(1)?,
        status,
        attempts: row.get(3)?,
        next_attempt_at: row.get(4)?,
    })
}
