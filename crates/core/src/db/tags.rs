use super::*;

impl Db {
    // ---------- tags ----------

    pub fn add_tag(&self, name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO tags(name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            params![name],
        )?;
        let id = self.conn.query_row(
            "SELECT id FROM tags WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn set_photo_tags(&self, photo_id: i64, tags: &[String]) -> Result<()> {
        // A savepoint instead of a transaction, so the method also works
        // inside a caller's transaction (e.g. `in_transaction`).
        self.conn.execute_batch("SAVEPOINT set_photo_tags")?;
        let result = (|| {
            self.conn.execute(
                "DELETE FROM photo_tags WHERE photo_id = ?1",
                params![photo_id],
            )?;
            for name in tags {
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                self.conn.execute(
                    "INSERT INTO tags(name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
                    params![name],
                )?;
                let tag_id: i64 = self.conn.query_row(
                    "SELECT id FROM tags WHERE name = ?1",
                    params![name],
                    |row| row.get(0),
                )?;
                self.conn.execute(
                    "INSERT OR IGNORE INTO photo_tags(photo_id, tag_id) VALUES (?1, ?2)",
                    params![photo_id, tag_id],
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("RELEASE set_photo_tags")?;
                Ok(())
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK TO set_photo_tags");
                let _ = self.conn.execute_batch("RELEASE set_photo_tags");
                Err(error)
            }
        }
    }

    pub fn photo_tags(&self, photo_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT t.name FROM tags t JOIN photo_tags pt ON pt.tag_id = t.id \
             WHERE pt.photo_id = ?1 ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![photo_id], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Adds each normalized tag to every selected photo in one transaction.
    ///
    /// Empty tags are ignored, duplicate tags are collapsed, and photos that
    /// already carry a tag are left unchanged. A missing photo ID aborts the
    /// whole operation so a bulk edit never leaves a partial result.
    pub fn add_tags_to_photos(&self, photo_ids: &[i64], tags: &[String]) -> Result<()> {
        let photo_ids = Self::unique_photo_ids(photo_ids);
        let tags = Self::normalized_tag_names(tags);
        if photo_ids.is_empty() || tags.is_empty() {
            return Ok(());
        }
        self.in_transaction(|db| db.add_tags_in_transaction(&photo_ids, &tags))
    }

    /// Applies additions and removals to every selected photo in one
    /// transaction. The dialog uses this so a mixed edit cannot leave a
    /// partial result.
    pub fn update_tags_for_photos(
        &self,
        photo_ids: &[i64],
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let photo_ids = Self::unique_photo_ids(photo_ids);
        let add = Self::normalized_tag_names(add);
        let remove = Self::normalized_tag_names(remove);
        if photo_ids.is_empty() || (add.is_empty() && remove.is_empty()) {
            return Ok(());
        }
        self.in_transaction(|db| {
            db.add_tags_in_transaction(&photo_ids, &add)?;
            db.remove_tags_in_transaction(&photo_ids, &remove)?;
            Ok(())
        })
    }

    /// Removes each tag from every selected photo in one transaction.
    ///
    /// Unknown tags are ignored. A missing photo ID aborts the whole
    /// operation, matching [`Self::add_tags_to_photos`].
    pub fn remove_tags_from_photos(&self, photo_ids: &[i64], tags: &[String]) -> Result<()> {
        let photo_ids = Self::unique_photo_ids(photo_ids);
        let tags = Self::normalized_tag_names(tags);
        if photo_ids.is_empty() || tags.is_empty() {
            return Ok(());
        }
        self.in_transaction(|db| db.remove_tags_in_transaction(&photo_ids, &tags))
    }

    fn add_tags_in_transaction(&self, photo_ids: &[i64], tags: &[String]) -> Result<()> {
        if tags.is_empty() {
            return Ok(());
        }
        self.ensure_photos_exist(photo_ids)?;
        let mut tag_ids = Vec::with_capacity(tags.len());
        for name in tags {
            tag_ids.push(self.add_tag(name)?);
        }
        for photo_id in photo_ids {
            for tag_id in &tag_ids {
                self.conn.execute(
                    "INSERT OR IGNORE INTO photo_tags(photo_id, tag_id) VALUES (?1, ?2)",
                    params![*photo_id, *tag_id],
                )?;
            }
        }
        Ok(())
    }

    fn remove_tags_in_transaction(&self, photo_ids: &[i64], tags: &[String]) -> Result<()> {
        if tags.is_empty() {
            return Ok(());
        }
        self.ensure_photos_exist(photo_ids)?;
        for name in tags {
            let tag_id: Option<i64> = self
                .conn
                .query_row(
                    "SELECT id FROM tags WHERE name = ?1",
                    params![name],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(tag_id) = tag_id else {
                continue;
            };
            for photo_id in photo_ids {
                self.conn.execute(
                    "DELETE FROM photo_tags WHERE photo_id = ?1 AND tag_id = ?2",
                    params![*photo_id, tag_id],
                )?;
            }
        }
        Ok(())
    }

    /// Returns the manual tags present on every selected photo, sorted by
    /// name. An empty selection has no common tags.
    pub fn common_manual_tags(&self, photo_ids: &[i64]) -> Result<Vec<String>> {
        let photo_ids = Self::unique_photo_ids(photo_ids);
        if photo_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure_photos_exist(&photo_ids)?;

        let placeholders = (2..=photo_ids.len() + 1)
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT t.name FROM tags t JOIN photo_tags pt ON pt.tag_id = t.id \
             WHERE pt.photo_id IN ({placeholders}) \
             GROUP BY t.id, t.name \
             HAVING COUNT(DISTINCT pt.photo_id) = ?1 \
             ORDER BY t.name COLLATE NOCASE"
        );
        let mut values = Vec::with_capacity(photo_ids.len() + 1);
        values.push(photo_ids.len() as i64);
        values.extend(photo_ids.iter().copied());
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn unique_photo_ids(photo_ids: &[i64]) -> Vec<i64> {
        let mut seen = HashSet::with_capacity(photo_ids.len());
        photo_ids
            .iter()
            .copied()
            .filter(|id| seen.insert(*id))
            .collect()
    }

    fn normalized_tag_names(tags: &[String]) -> Vec<String> {
        let mut seen = HashSet::with_capacity(tags.len());
        let mut normalized = Vec::with_capacity(tags.len());
        for tag in tags {
            let tag = tag.trim();
            if tag.is_empty() {
                continue;
            }
            if seen.insert(tag.to_string()) {
                normalized.push(tag.to_string());
            }
        }
        normalized
    }

    fn ensure_photos_exist(&self, photo_ids: &[i64]) -> Result<()> {
        for chunk in photo_ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!("SELECT COUNT(*) FROM photos WHERE id IN ({placeholders})");
            let count: i64 =
                self.conn
                    .query_row(&sql, params_from_iter(chunk.iter().copied()), |row| {
                        row.get(0)
                    })?;
            if count as usize != chunk.len() {
                let missing = chunk.len() - count as usize;
                return Err(CoreError::NotFound(format!(
                    "{missing} photo(s) in a bulk tag operation"
                )));
            }
        }
        Ok(())
    }
}
