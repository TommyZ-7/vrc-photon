use super::*;

impl Db {
    // ---------- log imports / settings ----------

    pub fn log_offset(&self, log_file: &str) -> Result<Option<i64>> {
        let offset = self
            .conn
            .query_row(
                "SELECT last_offset FROM log_imports WHERE log_file = ?1",
                params![log_file],
                |row| row.get(0),
            )
            .optional()?;
        Ok(offset)
    }

    pub fn log_mtime(&self, log_file: &str) -> Result<Option<i64>> {
        let mtime = self
            .conn
            .query_row(
                "SELECT mtime FROM log_imports WHERE log_file = ?1",
                params![log_file],
                |row| row.get(0),
            )
            .optional()?;
        Ok(mtime)
    }

    pub fn set_log_offset(
        &self,
        log_file: &str,
        size: i64,
        mtime: i64,
        last_offset: i64,
        parser_version: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO log_imports(log_file, size, mtime, last_offset, parser_version, parsed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(log_file) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, \
                last_offset = excluded.last_offset, parser_version = excluded.parser_version, \
                parsed_at = excluded.parsed_at",
            params![log_file, size, mtime, last_offset, parser_version, now_string()],
        )?;
        Ok(())
    }

    /// Runs `f` as a single transaction: commits when `f` succeeds and
    /// rolls back when it fails, so bulk writers never leave a
    /// half-written library behind.
    pub fn in_transaction<T>(&self, f: impl FnOnce(&Db) -> Result<T>) -> Result<T> {
        // Bulk writers (log-ingest chunks, deletion batches) read and write
        // through this helper. Take the write lock up front so a concurrent
        // UI write cannot turn the first write into SQLITE_BUSY_SNAPSHOT.
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let value = f(self)?;
        tx.commit()?;
        Ok(value)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_settings(key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let value = self
            .conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
        Ok(())
    }
}
