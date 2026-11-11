use super::*;

impl Db {
    // ---------- worlds / sessions ----------

    pub fn upsert_world(
        &self,
        world_id: &str,
        name: Option<&str>,
        source: Option<&str>,
    ) -> Result<i64> {
        // Standalone calls take the write lock up front: with a deferred
        // transaction another process could write between the SELECT and the
        // UPDATE, turning our snapshot stale (SQLITE_BUSY_SNAPSHOT, which
        // busy_timeout does not retry) or racing the insert on a new world.
        // Calls already inside a caller-owned transaction (for example the
        // demo seed) must join it instead of nesting a second transaction.
        if self.conn.is_autocommit() {
            let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
            let id = self.upsert_world_inner(&tx, world_id, name, source)?;
            tx.commit()?;
            Ok(id)
        } else {
            self.upsert_world_inner(&self.conn, world_id, name, source)
        }
    }

    fn upsert_world_inner(
        &self,
        conn: &Connection,
        world_id: &str,
        name: Option<&str>,
        source: Option<&str>,
    ) -> Result<i64> {
        let now = now_string();
        let existing = conn
            .query_row(
                "SELECT id, name, name_source, alias FROM worlds WHERE world_id = ?1",
                params![world_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;

        let Some((id, old_name, old_source, alias)) = existing else {
            return conn
                .query_row(
                    "INSERT INTO worlds(world_id, name, name_source, first_seen, last_seen) \
                 VALUES (?1, ?2, ?3, ?4, ?4) RETURNING id",
                    params![world_id, name, source, now],
                    |row| row.get(0),
                )
                .map_err(Into::into);
        };

        // A missing name is always filled. An API name may be replaced by a
        // later log/XMP name; log/XMP names keep the first one seen.
        let replaces_api_name =
            old_source.as_deref() == Some("vrchat-api") && matches!(source, Some("log" | "xmp"));
        let stored_name = if old_name.is_none() || replaces_api_name {
            name.map(str::to_string).or_else(|| old_name.clone())
        } else {
            old_name.clone()
        };
        let stored_source = if name.is_some() && (old_name.is_none() || replaces_api_name) {
            source.map(str::to_string).or_else(|| old_source.clone())
        } else {
            old_source.clone()
        };
        conn.execute(
            "UPDATE worlds SET name = ?1, name_source = ?2, last_seen = ?3 WHERE id = ?4",
            params![stored_name, stored_source, now, id],
        )?;

        // Alias always wins for display. When an API name is replaced, photos
        // that already show that API name must follow the new name too.
        let effective_name = alias.as_deref().or(stored_name.as_deref());
        let updated = match effective_name {
            Some(effective_name) if replaces_api_name => conn.execute(
                "UPDATE photo_context SET world_name = ?1 \
                 WHERE world_id = ?2 AND (world_name IS NULL OR world_name = ?3)",
                params![effective_name, id, old_name],
            )?,
            Some(effective_name) => conn.execute(
                "UPDATE photo_context SET world_name = ?1 \
                 WHERE world_id = ?2 AND world_name IS NULL",
                params![effective_name, id],
            )?,
            None => 0,
        };

        if stored_name.is_some() && matches!(source, Some("log" | "xmp")) {
            self.mark_world_name_lookup_satisfied(id)?;
        }
        if updated > 0 {
            self.bump_library_revision()?;
        }
        Ok(id)
    }

    pub fn world_by_id(&self, id: i64) -> Result<Option<World>> {
        let world = self
            .conn
            .query_row(
                "SELECT id, world_id, name, name_source, alias FROM worlds WHERE id = ?1",
                params![id],
                row_to_world,
            )
            .optional()?;
        Ok(world)
    }

    pub fn world_by_world_id(&self, world_id: &str) -> Result<Option<World>> {
        let world = self
            .conn
            .query_row(
                "SELECT id, world_id, name, name_source, alias FROM worlds WHERE world_id = ?1",
                params![world_id],
                row_to_world,
            )
            .optional()?;
        Ok(world)
    }

    /// Lists the worlds represented by visible photos, with photo counts.
    ///
    /// Photos linked only by name are included with `world_id = None`. An
    /// alias wins over the stored or log-derived name, matching the rest of
    /// the UI.
    pub fn worlds_with_photo_counts(&self) -> Result<Vec<WorldPhotoCount>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT w.world_id, \
                COALESCE(w.alias, c.world_name, w.name, w.world_id, '') AS name, \
                COUNT(*) \
             FROM photos p \
             JOIN photo_context c ON c.photo_id = p.id \
             LEFT JOIN worlds w ON w.id = c.world_id \
             WHERE p.status IN ('present', 'replaced') \
               AND COALESCE(c.world_id, c.world_name) IS NOT NULL \
             GROUP BY c.world_id, \
                COALESCE(w.alias, c.world_name, w.name, w.world_id, ''), \
                w.world_id \
             ORDER BY COUNT(*) DESC, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WorldPhotoCount {
                world_id: row.get(0)?,
                name: row.get(1)?,
                photo_count: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn row_to_world(row: &Row<'_>) -> rusqlite::Result<World> {
    Ok(World {
        id: row.get(0)?,
        world_id: row.get(1)?,
        name: row.get(2)?,
        name_source: row.get(3)?,
        alias: row.get(4)?,
    })
}
