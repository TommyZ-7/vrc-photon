use super::*;

impl Db {
    // ---------- photos ----------

    pub fn upsert_photo(&self, photo: &NewPhoto) -> Result<i64> {
        upsert_photo_in(&self.conn, photo)
    }

    /// Upserts a batch of photos in one immediate transaction.
    ///
    /// Returns IDs in input order. A failure rolls back the whole batch; the
    /// scanner retries the photos individually so one bad row does not
    /// discard its neighbors.
    pub fn upsert_photos(&self, photos: &[NewPhoto]) -> Result<Vec<i64>> {
        if photos.is_empty() {
            return Ok(Vec::new());
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let mut ids = Vec::with_capacity(photos.len());
        for photo in photos {
            ids.push(upsert_photo_in(&tx, photo)?);
        }
        tx.commit()?;
        Ok(ids)
    }

    pub fn photo_by_id(&self, id: i64) -> Result<Option<Photo>> {
        let sql = format!("SELECT {PHOTO_SELECT}{PHOTO_FROM} WHERE p.id = ?1");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let photo = stmt.query_row(params![id], row_to_photo).optional()?;
        Ok(photo)
    }

    /// Returns only the path a thumbnail reader should open.
    ///
    /// This skips the three context joins and the full [`Photo`] row that
    /// [`Self::photo_by_id`] materializes; replaced photos still resolve to
    /// their display derivative.
    pub fn photo_effective_path(&self, id: i64) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT COALESCE(d.path, p.abs_path) FROM photos p \
             LEFT JOIN derivatives d ON d.id = p.display_derivative_id \
             WHERE p.id = ?1",
        )?;
        let path = stmt.query_row(params![id], |row| row.get(0)).optional()?;
        Ok(path)
    }

    pub fn photo_by_path(&self, path: &str) -> Result<Option<Photo>> {
        let sql = format!("SELECT {PHOTO_SELECT}{PHOTO_FROM} WHERE p.abs_path = ?1");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let photo = stmt.query_row(params![path], row_to_photo).optional()?;
        Ok(photo)
    }

    /// Returns a lightweight, ordered snapshot of every present photo. The
    /// `photo_context` join is 1:1 and keeps every `PhotoSort` executable,
    /// including world order.
    ///
    /// The result is capped at [`MAX_ROW_LIMIT`]; use
    /// [`Self::photo_index_snapshot`] when the caller needs to know whether
    /// the library was truncated.
    pub fn photo_index(&self, sort: PhotoSort) -> Result<Vec<PhotoIndexEntry>> {
        Ok(self.photo_index_snapshot(sort)?.entries)
    }

    /// Returns the index snapshot and whether [`MAX_ROW_LIMIT`] hid any rows.
    pub fn photo_index_snapshot(&self, sort: PhotoSort) -> Result<PhotoIndexSnapshot> {
        self.photo_index_snapshot_with_limit(sort, MAX_ROW_LIMIT)
    }

    pub(super) fn photo_index_snapshot_with_limit(
        &self,
        sort: PhotoSort,
        limit: i64,
    ) -> Result<PhotoIndexSnapshot> {
        let limit = clamp_limit(limit);
        let sql = format!("{} LIMIT ?1", photo_index_sql(sort));
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map([limit + 1], row_to_photo_index)?;
        let mut entries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let truncated = entries.len() as i64 > limit;
        if truncated {
            entries.truncate(limit as usize);
        }
        Ok(PhotoIndexSnapshot { entries, truncated })
    }

    /// Returns an ordered index snapshot matching the text filters.
    ///
    /// The result is capped at [`MAX_ROW_LIMIT`] so a broad filter cannot
    /// allocate without bound.
    pub fn photo_index_text(&self, filter: &PhotoTextFilter) -> Result<Vec<PhotoIndexEntry>> {
        self.photo_index_text_limited(filter, MAX_ROW_LIMIT)
    }

    /// Same as [`Self::photo_index_text`] with an explicit row limit.
    pub fn photo_index_text_limited(
        &self,
        filter: &PhotoTextFilter,
        limit: i64,
    ) -> Result<Vec<PhotoIndexEntry>> {
        let (where_sql, mut values) = build_text_filter(filter);
        let sql = format!(
            "SELECT p.id, p.captured_at_utc, p.captured_month, p.captured_day, c.session_id\
             {PHOTO_FROM} WHERE {where_sql} \
             ORDER BY p.captured_at_utc DESC, p.id DESC LIMIT ?"
        );
        values.push(Value::Integer(clamp_limit(limit)));
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), row_to_photo_index)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// World-name autocomplete candidates ordered by photo count.
    pub fn world_name_suggestions(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchSuggestion>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let like = format!("%{}%", escape_like(query));
        let sql = format!(
            "SELECT c.world_name AS name, COUNT(DISTINCT p.id) AS photo_count \
             FROM photo_context c JOIN photos p ON p.id = c.photo_id \
             WHERE {VISIBLE_STATUS} AND c.world_name IS NOT NULL AND c.world_name <> '' \
               AND c.world_name LIKE ? ESCAPE '\\' \
             GROUP BY c.world_name \
             ORDER BY photo_count DESC, name COLLATE NOCASE ASC LIMIT ?"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![like, limit as i64], |row| {
            Ok(SearchSuggestion {
                name: row.get(0)?,
                photo_count: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Player display-name autocomplete candidates ordered by photo count.
    pub fn player_name_suggestions(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchSuggestion>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let like = format!("%{}%", escape_like(query));
        let sql = format!(
            "SELECT pp.display_name AS name, COUNT(DISTINCT pp.photo_id) AS photo_count \
             FROM photo_players pp JOIN photos p ON p.id = pp.photo_id \
             WHERE {VISIBLE_STATUS} AND pp.display_name IS NOT NULL AND pp.display_name <> '' \
               AND pp.display_name LIKE ? ESCAPE '\\' \
             GROUP BY pp.display_name \
             ORDER BY photo_count DESC, name COLLATE NOCASE ASC LIMIT ?"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![like, limit as i64], |row| {
            Ok(SearchSuggestion {
                name: row.get(0)?,
                photo_count: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Tag-name autocomplete candidates from manual and applied AI tags.
    ///
    /// AI names follow the UI locale; `prefer_english` selects `name_en` when
    /// available. Rows with the same display name merge, and a photo tagged
    /// both manually and by the AI plugin is counted once.
    pub fn tag_name_suggestions(
        &self,
        query: &str,
        limit: usize,
        prefer_english: bool,
    ) -> Result<Vec<SearchSuggestion>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let like = format!("%{}%", escape_like(query));
        let sql = format!(
            "WITH candidates(name, photo_id) AS ( \
                 SELECT t.name, pt.photo_id \
                 FROM tags t JOIN photo_tags pt ON pt.tag_id = t.id \
                 UNION ALL \
                 SELECT CASE WHEN ?1 THEN COALESCE(d.name_en, d.name) ELSE d.name END, \
                        at.photo_id \
                 FROM photo_ai_tags at \
                 JOIN ai_tag_definitions d \
                   ON d.plugin_id = at.plugin_id AND d.tag_key = at.tag_key \
                 WHERE at.applied = 1 AND at.plugin_id = ?4 \
             ) \
             SELECT c.name AS name, COUNT(DISTINCT c.photo_id) AS photo_count \
             FROM candidates c JOIN photos p ON p.id = c.photo_id \
             WHERE {VISIBLE_STATUS} AND c.name <> '' AND c.name LIKE ?2 ESCAPE '\\' \
             GROUP BY c.name \
             ORDER BY photo_count DESC, c.name COLLATE NOCASE ASC LIMIT ?3"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(
            params![i64::from(prefer_english), like, limit as i64, AI_PLUGIN_ID],
            |row| {
                Ok(SearchSuggestion {
                    name: row.get(0)?,
                    photo_count: row.get(1)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Fetches photos by primary key, preserving the order of `ids` and
    /// skipping ids that no longer exist.
    pub fn photos_by_ids(&self, ids: &[i64]) -> Result<Vec<Photo>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut by_id = HashMap::with_capacity(ids.len());
        for chunk in ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!("SELECT {PHOTO_SELECT}{PHOTO_FROM} WHERE p.id IN ({placeholders})");
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter().copied()), row_to_photo)?;
            for photo in rows {
                let photo = photo?;
                by_id.insert(photo.id, photo);
            }
        }
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }

    pub fn list_photos(&self, limit: i64, offset: i64, sort: PhotoSort) -> Result<Vec<Photo>> {
        let order = photo_order(sort);
        let sql = format!(
            "SELECT {PHOTO_SELECT}{PHOTO_FROM} WHERE {VISIBLE_STATUS} \
             ORDER BY {order} LIMIT ?1 OFFSET ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![clamp_limit(limit), offset.max(0)], row_to_photo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Legacy `PhotoFilter` search retained as an oracle for tests and demo
    /// data. Production UI search uses [`Self::photo_index_text`].
    /// `filter.limit` follows [`clamp_limit`].
    pub fn search_photos(&self, filter: &PhotoFilter) -> Result<Vec<Photo>> {
        let (where_sql, values) = build_photo_filter(filter);
        let sort = filter.sort.unwrap_or(PhotoSort::Newest);
        let order = photo_order(sort);
        let limit = clamp_limit(filter.limit);
        let offset = filter.offset.max(0);
        let sql = format!(
            "SELECT {PHOTO_SELECT}{PHOTO_FROM} WHERE {where_sql} \
             ORDER BY {order} LIMIT ? OFFSET ?"
        );
        let mut all_values = values;
        all_values.push(Value::Integer(limit));
        all_values.push(Value::Integer(offset));
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params_from_iter(all_values), row_to_photo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Counts rows matching a legacy `PhotoFilter` query. Retained together
    /// with [`Self::search_photos`] as a test/demo oracle; production search
    /// uses [`Self::photo_index_text`].
    pub fn count_search(&self, filter: &PhotoFilter) -> Result<i64> {
        let (where_sql, values) = build_photo_filter(filter);
        let sql = format!("SELECT COUNT(*){PHOTO_FROM} WHERE {where_sql}");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let count = stmt.query_row(params_from_iter(values), |row| row.get(0))?;
        Ok(count)
    }

    /// Returns a page of full photo rows matching the text filters.
    ///
    /// This is the read model behind the daemon's MCP search: it uses the same
    /// filters as the UI index search but keeps paths and metadata for the
    /// caller. Rows are ordered newest first.
    pub fn search_photos_text(
        &self,
        filter: &PhotoTextFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Photo>> {
        let (where_sql, mut values) = build_text_filter(filter);
        values.push(Value::Integer(clamp_limit(limit)));
        values.push(Value::Integer(offset.max(0)));
        let sql = format!(
            "SELECT {PHOTO_SELECT}{PHOTO_FROM} WHERE {where_sql} \
             ORDER BY p.captured_at_utc DESC, p.id DESC LIMIT ? OFFSET ?"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), row_to_photo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Counts full photo rows matching the same text filters as
    /// [`Self::search_photos_text`].
    pub fn count_photos_text(&self, filter: &PhotoTextFilter) -> Result<i64> {
        let (where_sql, values) = build_text_filter(filter);
        let sql = format!("SELECT COUNT(*){PHOTO_FROM} WHERE {where_sql}");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let count = stmt.query_row(params_from_iter(values), |row| row.get(0))?;
        Ok(count)
    }

    fn photo_ids_text(&self, filter: &PhotoTextFilter) -> Result<Vec<i64>> {
        let (where_sql, values) = build_text_filter(filter);
        let sql = format!("SELECT p.id{PHOTO_FROM} WHERE {where_sql}");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Deterministically samples up to `limit` matching photos using `seed`.
    /// `total` is the number of matches before sampling; `limit` follows
    /// [`clamp_limit`].
    pub fn random_photos_text(
        &self,
        filter: &PhotoTextFilter,
        limit: i64,
        seed: u64,
    ) -> Result<RandomPhotoSample> {
        let ids = self.photo_ids_text(filter)?;
        let total = ids.len() as i64;
        let take = clamp_limit(limit) as usize;
        let mut keyed: Vec<(u64, i64)> = ids
            .iter()
            .map(|&id| (splitmix64(seed ^ id as u64), id))
            .collect();
        keyed.sort_unstable();
        keyed.truncate(take.min(keyed.len()));
        let ids: Vec<i64> = keyed.into_iter().map(|(_, id)| id).collect();
        let photos = self.photos_by_ids(&ids)?;
        Ok(RandomPhotoSample { photos, total })
    }

    pub fn unlinked_photos(&self, limit: i64) -> Result<Vec<Photo>> {
        let sql = format!(
            "SELECT {PHOTO_SELECT}{PHOTO_FROM} WHERE c.photo_id IS NULL \
             AND {VISIBLE_STATUS} ORDER BY p.captured_at_utc DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![clamp_limit(limit)], row_to_photo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Unlinked photos that are due for a link attempt, least-attempted and
    /// oldest first so a large backlog cannot starve photos behind the limit.
    /// `now` is a Unix timestamp in seconds.
    pub fn unlinked_photos_due(&self, limit: i64, now: i64) -> Result<Vec<Photo>> {
        let sql = format!(
            "SELECT {PHOTO_SELECT}{PHOTO_FROM} WHERE c.photo_id IS NULL \
             AND {VISIBLE_STATUS} \
             AND (p.link_next_retry_at IS NULL OR p.link_next_retry_at <= ?1) \
             ORDER BY p.link_attempts ASC, p.captured_at_utc ASC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![now, clamp_limit(limit)], row_to_photo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Records a failed link attempt and schedules the next one. The first
    /// three failures retry after an hour, six hours, and a day; later
    /// failures wait a week. `now` is a Unix timestamp in seconds.
    pub fn record_link_attempt(&self, photo_id: i64, now: i64) -> Result<()> {
        let [first, second, third, later] = LINK_RETRY_BACKOFF_SECONDS;
        self.conn.execute(
            "UPDATE photos SET link_attempts = link_attempts + 1, \
                link_next_retry_at = ?2 + CASE link_attempts + 1 \
                    WHEN 1 THEN ?3 WHEN 2 THEN ?4 WHEN 3 THEN ?5 ELSE ?6 END \
             WHERE id = ?1",
            params![photo_id, now, first, second, third, later],
        )?;
        Ok(())
    }

    /// Clears the retry schedule of every photo that still has no context, so
    /// an explicit Relog/Rescan retries all of them immediately. Attempt
    /// counts are kept for ordering and for the next failure's backoff.
    pub fn make_unlinked_photos_due(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE photos SET link_next_retry_at = NULL \
             WHERE link_next_retry_at IS NOT NULL \
               AND NOT EXISTS (SELECT 1 FROM photo_context c WHERE c.photo_id = photos.id)",
            [],
        )?;
        Ok(changed)
    }

    pub fn photos_for_root(&self, root_id: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, abs_path FROM photos WHERE root_id = ?1 AND status = 'present'",
        )?;
        let rows = stmt.query_map(params![root_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Fetches the row state a rescan needs for every path, in chunks that
    /// stay under SQLite's variable limit. Missing paths are simply absent
    /// from the map.
    pub fn photo_scan_states(&self, paths: &[String]) -> Result<HashMap<String, PhotoScanState>> {
        let mut states = HashMap::with_capacity(paths.len());
        for chunk in paths.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT id, abs_path, file_size, mtime, blake3, status, display_derivative_id \
                 FROM photos WHERE abs_path IN ({placeholders})"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                Ok(PhotoScanState {
                    id: row.get(0)?,
                    abs_path: row.get(1)?,
                    file_size: row.get::<_, i64>(2)? as u64,
                    mtime: row.get(3)?,
                    blake3: row.get(4)?,
                    status: PhotoStatus::parse(&row.get::<_, String>(5)?),
                    display_derivative_id: row.get(6)?,
                })
            })?;
            for state in rows {
                let state = state?;
                states.insert(state.abs_path.clone(), state);
            }
        }
        Ok(states)
    }

    pub fn count_photos(&self) -> Result<i64> {
        let count = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM photos p WHERE {VISIBLE_STATUS}"),
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn set_favorite(&self, photo_id: i64, favorite: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE photos SET is_favorite = ?1, updated_at = ?2 WHERE id = ?3",
            params![favorite as i64, now_string(), photo_id],
        )?;
        Ok(())
    }

    pub fn set_note(&self, photo_id: i64, note: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE photos SET note = ?1, updated_at = ?2 WHERE id = ?3",
            params![note, now_string(), photo_id],
        )?;
        Ok(())
    }

    pub fn mark_missing(&self, photo_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE photos SET status = 'missing', updated_at = ?1 WHERE id = ?2",
            params![now_string(), photo_id],
        )?;
        Ok(())
    }

    /// Marks a photo missing only when the row still points at `abs_path`.
    /// A concurrent move may already have rebound the row to another root.
    pub fn mark_missing_if_path(&self, photo_id: i64, abs_path: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE photos SET status = 'missing', updated_at = ?1 \
             WHERE id = ?2 AND abs_path = ?3 AND status = 'present'",
            params![now_string(), photo_id, abs_path],
        )?;
        Ok(changed == 1)
    }

    /// Promotes a completed derivative to the library display asset. The
    /// original file is expected to be removed by the caller.
    pub fn set_replaced(&self, photo_id: i64, derivative_id: i64) -> Result<bool> {
        self.set_replaced_in(&self.conn, photo_id, derivative_id)
    }

    fn set_replaced_in(
        &self,
        conn: &Connection,
        photo_id: i64,
        derivative_id: i64,
    ) -> Result<bool> {
        let changed = conn.execute(
            "UPDATE photos SET display_derivative_id = ?1, \
                status = CASE WHEN status = 'deleting' THEN 'deleting' ELSE 'replaced' END, \
                updated_at = ?2 \
             WHERE id = ?3 AND EXISTS (SELECT 1 FROM derivatives \
                 WHERE id = ?1 AND photo_id = photos.id)",
            params![derivative_id, now_string(), photo_id],
        )?;
        Ok(changed == 1)
    }

    /// Attaches a replacement derivative and queues its automatic deletion in
    /// one transaction. Either both writes commit or neither does.
    pub fn complete_replacement(&self, photo_id: i64, derivative_id: i64) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if !self.set_replaced_in(&tx, photo_id, derivative_id)? {
            return Err(CoreError::Invalid(
                "derivative was not attached to the photo".to_string(),
            ));
        }
        self.request_deletion_in(&tx, &[photo_id], DeletionReason::AutoReplace, false)?;
        tx.commit()?;
        Ok(())
    }

    /// Returns a replaced photo to the original-file state. Used when trashing
    /// the original fails after the database was updated optimistically.
    pub fn restore_present(&self, photo_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE photos SET display_derivative_id = NULL, status = 'present', \
                updated_at = ?1 WHERE id = ?2 AND status <> 'deleting'",
            params![now_string(), photo_id],
        )?;
        Ok(())
    }

    /// Removes the display derivative reference. The caller decides whether
    /// the row should also become missing or present.
    pub fn clear_display_derivative(&self, photo_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE photos SET display_derivative_id = NULL, updated_at = ?1 WHERE id = ?2",
            params![now_string(), photo_id],
        )?;
        Ok(())
    }

    /// Finds a unique existing row that describes the same original file.
    /// Exact BLAKE3 matches win; size/name/capture-time matching is the
    /// fallback for rows indexed before hashing was introduced.
    pub fn find_rebind_candidate(&self, photo: &NewPhoto) -> Result<Option<Photo>> {
        if let Some(hash) = photo.blake3.as_deref() {
            let sql = format!(
                "SELECT {PHOTO_SELECT}{PHOTO_FROM} \
                 WHERE p.blake3 = ?1 AND p.abs_path <> ?2 \
                   AND p.status IN ('present', 'replaced', 'missing') \
                   AND NOT EXISTS (SELECT 1 FROM deletion_requests dr \
                       WHERE dr.photo_id = p.id \
                         AND dr.status IN ('pending', 'blocked', 'running')) \
                 ORDER BY p.id LIMIT 2"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params![hash, photo.abs_path], row_to_photo)?;
            let mut matches = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            if matches.len() > 1 {
                return Ok(None);
            }
            if let Some(candidate) = matches.pop() {
                if candidate.blake3.as_deref().is_some_and(|old| old != hash) {
                    return Ok(None);
                }
                return Ok(Some(candidate));
            }
        }

        let sql = format!(
            "SELECT {PHOTO_SELECT}{PHOTO_FROM} \
             WHERE p.abs_path <> ?1 AND p.file_name = ?2 AND p.file_size = ?3 \
               AND p.width = ?4 AND p.height = ?5 AND p.captured_at_utc = ?6 \
               AND p.status IN ('present', 'replaced', 'missing') \
               AND NOT EXISTS (SELECT 1 FROM deletion_requests dr \
                   WHERE dr.photo_id = p.id \
                     AND dr.status IN ('pending', 'blocked', 'running')) \
             ORDER BY p.id LIMIT 2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(
            params![
                photo.abs_path,
                photo.file_name,
                photo.file_size as i64,
                photo.width as i64,
                photo.height as i64,
                photo.captured_at_utc,
            ],
            row_to_photo,
        )?;
        let mut matches = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        if matches.len() != 1 {
            return Ok(None);
        }
        let candidate = matches.pop().unwrap();
        if let (Some(old), Some(new)) = (candidate.blake3.as_deref(), photo.blake3.as_deref())
            && old != new
        {
            return Ok(None);
        }
        Ok(Some(candidate))
    }

    /// Rebinds a row to a new path without changing its id or any related
    /// tags, notes, context, derivatives, or jobs. Capture time and dimensions
    /// stay as originally indexed even when the file was renamed. Returns
    /// false when the new path or relative path is already occupied.
    pub fn rebind_photo(&self, photo_id: i64, photo: &NewPhoto) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let abs_conflict: i64 = tx.query_row(
            "SELECT COUNT(*) FROM photos WHERE abs_path = ?1 AND id <> ?2",
            params![photo.abs_path, photo_id],
            |row| row.get(0),
        )?;
        if abs_conflict > 0 {
            return Ok(false);
        }
        let rel_conflict: i64 = tx.query_row(
            "SELECT COUNT(*) FROM photos WHERE root_id = ?1 AND rel_path = ?2 AND id <> ?3",
            params![photo.root_id, photo.rel_path, photo_id],
            |row| row.get(0),
        )?;
        if rel_conflict > 0 {
            return Ok(false);
        }
        let changed = tx.execute(
            "UPDATE photos SET root_id = ?1, rel_path = ?2, abs_path = ?3, file_name = ?4, \
                file_size = ?5, mtime = ?6, blake3 = COALESCE(?7, blake3), \
                status = CASE WHEN status IN ('present', 'replaced', 'missing', 'deleting') \
                    THEN 'present' ELSE status END, \
                display_derivative_id = NULL, updated_at = ?8 \
             WHERE id = ?9 AND NOT EXISTS (SELECT 1 FROM deletion_requests dr \
                 WHERE dr.photo_id = photos.id \
                   AND dr.status IN ('pending', 'blocked', 'running'))",
            params![
                photo.root_id,
                photo.rel_path,
                photo.abs_path,
                photo.file_name,
                photo.file_size as i64,
                photo.mtime,
                photo.blake3,
                now_string(),
                photo_id,
            ],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn photos_without_hash(&self, limit: i64) -> Result<Vec<(i64, String)>> {
        self.photos_without_hash_after(limit, 0)
    }

    /// Returns present photos with no hash whose id is greater than `after_id`.
    /// The daemon pages through these rows with a cursor so a permanently
    /// unreadable row cannot block every later photo.
    pub fn photos_without_hash_after(
        &self,
        limit: i64,
        after_id: i64,
    ) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, abs_path FROM photos \
             WHERE status = 'present' AND blake3 IS NULL AND id > ?2 \
             ORDER BY id LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![clamp_limit(limit), after_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Returns replaced photos and the derivative path the library displays.
    /// The daemon stats these paths so a manually deleted compressed file does
    /// not leave a broken card behind.
    pub fn display_derivative_paths(
        &self,
        limit: i64,
    ) -> Result<Vec<(i64, Option<String>, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT p.id, d.path, r.path FROM photos p \
             LEFT JOIN derivatives d ON d.id = p.display_derivative_id \
             JOIN library_roots r ON r.id = p.root_id \
             WHERE p.status = 'replaced' ORDER BY p.id LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![clamp_limit(limit)], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_photo_blake3(&self, photo_id: i64, blake3: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE photos SET blake3 = ?1 WHERE id = ?2 AND blake3 IS NULL",
            params![blake3, photo_id],
        )?;
        Ok(())
    }

    /// Returns ids whose content hash is shared with another visible photo.
    /// Hashing itself does not merge rows; this query is the duplicate report
    /// surface used by move matching and future duplicate-management UI.
    pub fn duplicate_photo_ids(&self) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT p.id FROM photos p \
             WHERE {VISIBLE_STATUS} AND p.blake3 IS NOT NULL \
               AND EXISTS (SELECT 1 FROM photos q \
                 WHERE q.blake3 = p.blake3 AND q.id <> p.id \
                   AND q.status IN ('present', 'replaced')) \
             ORDER BY p.id"
        ))?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Monotonic counter used by the UI to detect daemon-side changes even
    /// when the number of photos stays the same (for example a moved file).
    pub fn library_revision(&self) -> Result<i64> {
        self.revision("library_revision")
    }

    pub fn bump_library_revision(&self) -> Result<i64> {
        self.bump_revision("library_revision")
    }

    /// Monotonic counter for AI metadata changes that do not alter the photo
    /// index (results, thresholds, and catalog updates).
    pub fn ai_revision(&self) -> Result<i64> {
        self.revision("ai_revision")
    }

    pub fn bump_ai_revision(&self) -> Result<i64> {
        self.bump_revision("ai_revision")
    }

    /// Monotonic counter for session lifecycle changes that affect header
    /// summaries but not the photo index.
    pub fn session_revision(&self) -> Result<i64> {
        self.revision("session_revision")
    }

    pub fn bump_session_revision(&self) -> Result<i64> {
        self.bump_revision("session_revision")
    }

    fn revision(&self, key: &str) -> Result<i64> {
        Ok(self
            .get_setting(key)?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0))
    }

    fn bump_revision(&self, key: &str) -> Result<i64> {
        let revision = self.conn.query_row(
            "INSERT INTO app_settings(key, value) VALUES (?1, '1') \
             ON CONFLICT(key) DO UPDATE SET \
                value = CAST(CAST(app_settings.value AS INTEGER) + 1 AS TEXT) \
             RETURNING CAST(value AS INTEGER)",
            params![key],
            |row| row.get(0),
        )?;
        Ok(revision)
    }

    pub fn stats(&self) -> Result<LibraryStats> {
        let (photo_count, total_bytes): (i64, i64) = self.conn.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(\
                    CASE WHEN p.display_derivative_id IS NOT NULL \
                         THEN d.file_size ELSE p.file_size END), 0) \
                 FROM photos p LEFT JOIN derivatives d ON d.id = p.display_derivative_id \
                 WHERE {VISIBLE_STATUS}"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (compressed_count, derivative_bytes): (i64, i64) = self.conn.query_row(
            &format!(
                "SELECT COUNT(DISTINCT d.photo_id), COALESCE(SUM(d.file_size), 0) \
                 FROM derivatives d JOIN photos p ON p.id = d.photo_id \
                 WHERE {VISIBLE_STATUS}"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let world_count: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM ( \
                    SELECT DISTINCT COALESCE(c.world_id, c.world_name) AS world \
                    FROM photos p JOIN photo_context c ON c.photo_id = p.id \
                    WHERE {VISIBLE_STATUS} \
                      AND COALESCE(c.world_id, c.world_name) IS NOT NULL \
                 )"
            ),
            [],
            |row| row.get(0),
        )?;
        Ok(LibraryStats {
            photo_count,
            total_bytes: total_bytes.max(0) as u64,
            world_count,
            compressed_count,
            derivative_bytes: derivative_bytes.max(0) as u64,
        })
    }
}

impl Db {
    // ---------- photo context ----------

    pub fn set_photo_context(&self, context: &PhotoContext) -> Result<()> {
        self.conn.execute(
            "INSERT INTO photo_context(photo_id, session_id, world_id, world_name, instance_id, \
                link_method, confidence, author, linked_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(photo_id) DO UPDATE SET \
                session_id = excluded.session_id, world_id = excluded.world_id, \
                world_name = excluded.world_name, instance_id = excluded.instance_id, \
                link_method = excluded.link_method, confidence = excluded.confidence, \
                author = excluded.author, linked_at = excluded.linked_at",
            params![
                context.photo_id,
                context.session_id,
                context.world_id,
                context.world_name,
                context.instance_id,
                context.link_method,
                context.confidence,
                context.author,
                context.linked_at,
            ],
        )?;
        // A successful link ends the retry cycle; the photo is no longer a
        // candidate for `unlinked_photos_due`.
        self.conn.execute(
            "UPDATE photos SET link_attempts = 0, link_next_retry_at = NULL WHERE id = ?1",
            params![context.photo_id],
        )?;
        Ok(())
    }

    pub fn photo_context(&self, photo_id: i64) -> Result<Option<PhotoContext>> {
        let context = self
            .conn
            .query_row(
                "SELECT photo_id, session_id, world_id, world_name, instance_id, \
                    link_method, confidence, author, linked_at FROM photo_context WHERE photo_id = ?1",
                params![photo_id],
                |row| {
                    Ok(PhotoContext {
                        photo_id: row.get(0)?,
                        session_id: row.get(1)?,
                        world_id: row.get(2)?,
                        world_name: row.get(3)?,
                        instance_id: row.get(4)?,
                        link_method: row.get(5)?,
                        confidence: row.get(6)?,
                        author: row.get(7)?,
                        linked_at: row.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(context)
    }

    pub fn set_photo_players(&self, photo_id: i64, players: &[PhotoPlayer]) -> Result<()> {
        // A savepoint instead of a transaction, so the method also works
        // inside a caller's transaction (e.g. `in_transaction`).
        self.conn.execute_batch("SAVEPOINT set_photo_players")?;
        let result = (|| {
            self.conn.execute(
                "DELETE FROM photo_players WHERE photo_id = ?1",
                params![photo_id],
            )?;
            for player in players {
                if player.user_id.trim().is_empty() {
                    continue;
                }
                self.conn.execute(
                    "INSERT OR REPLACE INTO photo_players(photo_id, user_id, display_name) \
                     VALUES (?1, ?2, ?3)",
                    params![photo_id, player.user_id, player.display_name],
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("RELEASE set_photo_players")?;
                Ok(())
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK TO set_photo_players");
                let _ = self.conn.execute_batch("RELEASE set_photo_players");
                Err(error)
            }
        }
    }

    pub fn photo_players(&self, photo_id: i64) -> Result<Vec<PhotoPlayer>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT user_id, display_name FROM photo_players WHERE photo_id = ?1 \
             ORDER BY display_name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![photo_id], |row| {
            Ok(PhotoPlayer {
                user_id: row.get(0)?,
                display_name: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Loads the players of many photos in one query per 500 ids.
    ///
    /// Search results are limited to 500 rows, so the caller normally pays a
    /// single query instead of one query per photo. The per-photo order matches
    /// [`Self::photo_players`].
    pub fn photo_players_many(&self, photo_ids: &[i64]) -> Result<HashMap<i64, Vec<PhotoPlayer>>> {
        let mut grouped: HashMap<i64, Vec<PhotoPlayer>> = HashMap::new();
        for chunk in photo_ids.chunks(500) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT photo_id, user_id, display_name FROM photo_players \
                 WHERE photo_id IN ({placeholders}) \
                 ORDER BY photo_id, display_name COLLATE NOCASE"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter().copied()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    PhotoPlayer {
                        user_id: row.get(1)?,
                        display_name: row.get(2)?,
                    },
                ))
            })?;
            for row in rows {
                let (photo_id, player) = row?;
                grouped.entry(photo_id).or_default().push(player);
            }
        }
        Ok(grouped)
    }
}

impl Db {
    // ---------- derivatives ----------

    #[allow(clippy::too_many_arguments)]
    pub fn add_derivative(
        &self,
        photo_id: i64,
        path: &str,
        format: &str,
        width: u32,
        height: u32,
        file_size: u64,
        profile_id: Option<&str>,
        origin: CompressionOrigin,
        policy: &str,
    ) -> Result<i64> {
        let id = self
            .conn
            .query_row(
                "INSERT INTO derivatives(photo_id, path, format, width, height, file_size, profile_id, origin, policy, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                 ON CONFLICT(path) DO UPDATE SET photo_id = excluded.photo_id, \
                    format = excluded.format, width = excluded.width, height = excluded.height, \
                    file_size = excluded.file_size, profile_id = excluded.profile_id, \
                    origin = excluded.origin, policy = excluded.policy, created_at = excluded.created_at \
                 WHERE derivatives.photo_id = excluded.photo_id RETURNING id",
                params![
                    photo_id,
                    path,
                    format,
                    width as i64,
                    height as i64,
                    file_size as i64,
                    profile_id,
                    origin.as_str(),
                    policy,
                    now_string(),
                ],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                CoreError::Invalid(format!(
                    "derivative path {path} belongs to a different photo"
                ))
            })?;
        Ok(id)
    }

    pub fn compressed_photo_ids(&self) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT d.photo_id FROM derivatives d \
                 JOIN photos p ON p.id = d.photo_id WHERE p.status IN ('present', 'replaced')",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn derivatives_for_photo(&self, photo_id: i64) -> Result<Vec<Derivative>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, photo_id, path, format, width, height, file_size, profile_id, origin, policy, created_at \
             FROM derivatives WHERE photo_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![photo_id], |row| {
            Ok(Derivative {
                id: row.get(0)?,
                photo_id: row.get(1)?,
                path: row.get(2)?,
                format: row.get(3)?,
                width: row.get(4)?,
                height: row.get(5)?,
                file_size: row.get::<_, i64>(6)? as u64,
                profile_id: row.get(7)?,
                origin: CompressionOrigin::parse(&row.get::<_, String>(8)?),
                policy: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn upsert_photo_in(conn: &Connection, photo: &NewPhoto) -> Result<i64> {
    let now = now_string();
    let id = conn.query_row(
        "INSERT INTO photos(root_id, rel_path, abs_path, file_name, captured_at_utc, \
            captured_at_local, tz_offset_sec, width, height, file_size, mtime, blake3, \
            status, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'present', ?13, ?13) \
         ON CONFLICT(abs_path) DO UPDATE SET \
            root_id = excluded.root_id, rel_path = excluded.rel_path, \
            file_name = excluded.file_name, captured_at_utc = excluded.captured_at_utc, \
            captured_at_local = excluded.captured_at_local, \
            tz_offset_sec = excluded.tz_offset_sec, width = excluded.width, \
            height = excluded.height, file_size = excluded.file_size, \
            mtime = excluded.mtime, blake3 = COALESCE(excluded.blake3, photos.blake3), \
            status = CASE WHEN EXISTS ( \
                SELECT 1 FROM deletion_requests dr \
                WHERE dr.photo_id = photos.id \
                  AND dr.status IN ('pending', 'blocked', 'running') \
            ) OR photos.status NOT IN ('present', 'replaced', 'missing', 'deleting') \
            THEN photos.status ELSE 'present' END, \
            display_derivative_id = CASE WHEN EXISTS ( \
                SELECT 1 FROM deletion_requests dr \
                WHERE dr.photo_id = photos.id \
                  AND dr.status IN ('pending', 'blocked', 'running') \
            ) THEN photos.display_derivative_id ELSE NULL END, \
            updated_at = excluded.updated_at \
         RETURNING id",
        params![
            photo.root_id,
            photo.rel_path,
            photo.abs_path,
            photo.file_name,
            photo.captured_at_utc,
            photo.captured_at_local,
            photo.tz_offset_sec,
            photo.width as i64,
            photo.height as i64,
            photo.file_size as i64,
            photo.mtime,
            photo.blake3,
            now,
        ],
        |row| row.get(0),
    )?;
    Ok(id)
}

pub(super) fn photo_index_sql(sort: PhotoSort) -> String {
    let order = photo_order(sort);
    format!(
        "SELECT p.id, p.captured_at_utc, p.captured_month, p.captured_day, c.session_id\
         {PHOTO_FROM} WHERE {VISIBLE_STATUS} ORDER BY {order}"
    )
}

fn photo_order(sort: PhotoSort) -> &'static str {
    match sort {
        PhotoSort::Newest => "p.captured_at_utc DESC, p.id DESC",
        PhotoSort::Oldest => "p.captured_at_utc ASC, p.id ASC",
        PhotoSort::Largest => "p.file_size DESC, p.id DESC",
        PhotoSort::World => "c.world_name COLLATE NOCASE ASC, p.captured_at_utc DESC",
    }
}

fn push_ai_filter(clauses: &mut Vec<String>, values: &mut Vec<Value>, ai: &AiPhotoFilter) {
    if !ai.tags.is_empty() {
        let placeholders = vec!["?"; ai.tags.len()].join(",");
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM photo_ai_tags at \
             WHERE at.photo_id = p.id AND at.plugin_id = ? AND at.applied = 1 \
             AND at.tag_key IN ({placeholders}))"
        ));
        values.push(Value::Text(AI_PLUGIN_ID.to_string()));
        values.extend(ai.tags.iter().map(|tag| Value::Text(tag.clone())));
    }

    if let Some(count) = ai.people_count {
        clauses.push(
            "EXISTS (SELECT 1 FROM photo_ai pa WHERE pa.photo_id = p.id \
             AND pa.plugin_id = ? AND pa.people_min <= ? AND pa.people_max >= ?)"
                .to_string(),
        );
        values.push(Value::Text(AI_PLUGIN_ID.to_string()));
        values.push(Value::Integer(i64::from(count)));
        values.push(Value::Integer(i64::from(count)));
    }
}

fn build_photo_filter(filter: &PhotoFilter) -> (String, Vec<Value>) {
    let mut clauses = vec![VISIBLE_STATUS.to_string()];
    let mut values = Vec::new();

    if !filter.world_ids.is_empty() {
        let placeholders = vec!["?"; filter.world_ids.len()].join(",");
        clauses.push(format!("c.world_id IN ({placeholders})"));
        values.extend(filter.world_ids.iter().map(|id| Value::Integer(*id)));
    }

    if !filter.player_user_ids.is_empty() {
        let placeholders = vec!["?"; filter.player_user_ids.len()].join(",");
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM photo_players pp WHERE pp.photo_id = p.id \
             AND pp.user_id IN ({placeholders}))"
        ));
        values.extend(
            filter
                .player_user_ids
                .iter()
                .map(|id| Value::Text(id.clone())),
        );
    }

    if !filter.tags.is_empty() {
        let placeholders = vec!["?"; filter.tags.len()].join(",");
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id \
             WHERE pt.photo_id = p.id AND t.name IN ({placeholders}))"
        ));
        values.extend(filter.tags.iter().map(|tag| Value::Text(tag.clone())));
    }

    push_ai_filter(&mut clauses, &mut values, &filter.ai);

    if let Some(from) = filter.from_utc {
        clauses.push("p.captured_at_utc >= ?".to_string());
        values.push(Value::Integer(from));
    }
    if let Some(to) = filter.to_utc {
        clauses.push("p.captured_at_utc <= ?".to_string());
        values.push(Value::Integer(to));
    }
    if filter.favorite == Some(true) {
        clauses.push("p.is_favorite = 1".to_string());
    }
    if filter.unlinked == Some(true) {
        clauses.push("c.photo_id IS NULL".to_string());
    }

    (clauses.join(" AND "), values)
}

fn escape_like(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len());
    for character in query.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(super) fn build_text_filter(filter: &PhotoTextFilter) -> (String, Vec<Value>) {
    let mut clauses = vec![VISIBLE_STATUS.to_string()];
    let mut values: Vec<Value> = Vec::new();

    if let Some(query) = filter
        .world
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
    {
        let like = format!("%{}%", escape_like(query));
        clauses.push(
            "(c.world_name LIKE ? ESCAPE '\\' OR c.instance_id LIKE ? ESCAPE '\\' OR EXISTS (\
                SELECT 1 FROM worlds w WHERE w.id = c.world_id AND w.world_id LIKE ? ESCAPE '\\'))"
                .to_string(),
        );
        values.push(Value::Text(like.clone()));
        values.push(Value::Text(like.clone()));
        values.push(Value::Text(like));
    }
    if let Some(query) = filter
        .player
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
    {
        let like = format!("%{}%", escape_like(query));
        clauses.push(
            "EXISTS (SELECT 1 FROM photo_players pp WHERE pp.photo_id = p.id \
             AND (pp.display_name LIKE ? ESCAPE '\\' OR pp.user_id LIKE ? ESCAPE '\\'))"
                .to_string(),
        );
        values.push(Value::Text(like.clone()));
        values.push(Value::Text(like));
    }
    if let Some(query) = filter
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
    {
        let like = format!("%{}%", escape_like(query));
        clauses.push(
            "(EXISTS (SELECT 1 FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id \
             WHERE pt.photo_id = p.id AND t.name LIKE ? ESCAPE '\\') \
             OR EXISTS (SELECT 1 FROM photo_ai_tags at \
             JOIN ai_tag_definitions d \
               ON d.plugin_id = at.plugin_id AND d.tag_key = at.tag_key \
             WHERE at.photo_id = p.id AND at.applied = 1 AND at.plugin_id = ? \
               AND (d.name LIKE ? ESCAPE '\\' \
                    OR COALESCE(d.name_en, d.name) LIKE ? ESCAPE '\\' \
                    OR d.tag_key LIKE ? ESCAPE '\\')))"
                .to_string(),
        );
        values.push(Value::Text(like.clone()));
        values.push(Value::Text(AI_PLUGIN_ID.to_string()));
        values.push(Value::Text(like.clone()));
        values.push(Value::Text(like.clone()));
        values.push(Value::Text(like));
    }

    push_ai_filter(&mut clauses, &mut values, &filter.ai);

    if filter.favorite {
        clauses.push("p.is_favorite = 1".to_string());
    }
    if let Some(from) = filter.from_utc {
        clauses.push("p.captured_at_utc >= ?".to_string());
        values.push(Value::Integer(from));
    }
    if let Some(to) = filter.to_utc {
        clauses.push("p.captured_at_utc <= ?".to_string());
        values.push(Value::Integer(to));
    }

    (clauses.join(" AND "), values)
}

fn row_to_photo(row: &Row<'_>) -> rusqlite::Result<Photo> {
    Ok(Photo {
        id: row.get(0)?,
        root_id: row.get(1)?,
        rel_path: row.get(2)?,
        abs_path: row.get(3)?,
        file_name: row.get(4)?,
        captured_at_utc: row.get(5)?,
        captured_at_local: row.get(6)?,
        tz_offset_sec: row.get(7)?,
        width: row.get(8)?,
        height: row.get(9)?,
        file_size: row.get::<_, i64>(10)? as u64,
        mtime: row.get(11)?,
        blake3: row.get(12)?,
        status: PhotoStatus::parse(&row.get::<_, String>(13)?),
        is_favorite: row.get::<_, i64>(14)? != 0,
        note: row.get(15)?,
        display_derivative_id: row.get(16)?,
        display_path: row.get(17)?,
        display_format: row.get(18)?,
        display_file_size: row.get::<_, Option<i64>>(19)?.map(|size| size as u64),
        display_width: row.get(20)?,
        display_height: row.get(21)?,
        world_name: row.get(22)?,
        instance_id: row.get(23)?,
        link_method: row.get(24)?,
        confidence: row.get(25)?,
        author: row.get(26)?,
        world_id: row.get(27)?,
    })
}

fn row_to_photo_index(row: &Row<'_>) -> rusqlite::Result<PhotoIndexEntry> {
    Ok(PhotoIndexEntry {
        id: row.get(0)?,
        captured_at_utc: row.get(1)?,
        captured_month: row.get(2)?,
        captured_day: row.get(3)?,
        session_id: row.get(4)?,
    })
}

pub(super) const PHOTO_SELECT: &str = "p.id, p.root_id, p.rel_path, p.abs_path, p.file_name, \
    p.captured_at_utc, p.captured_at_local, p.tz_offset_sec, p.width, p.height, \
    p.file_size, p.mtime, p.blake3, p.status, p.is_favorite, p.note, \
    p.display_derivative_id, d.path, d.format, d.file_size, d.width, d.height, \
    c.world_name, c.instance_id, c.link_method, c.confidence, c.author, pw.world_id";

pub(super) const PHOTO_FROM: &str = " FROM photos p \
    LEFT JOIN derivatives d ON d.id = p.display_derivative_id \
    LEFT JOIN photo_context c ON c.photo_id = p.id \
    LEFT JOIN worlds pw ON pw.id = c.world_id ";

/// Statuses that belong in the library. `missing` rows stay in the database
/// so tags and context survive a restore, but they are not displayed.
pub(super) const VISIBLE_STATUS: &str = "p.status IN ('present', 'replaced')";
