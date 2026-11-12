use super::*;

impl Db {
    // ---------- daemon errors and commands ----------

    /// Records a daemon failure for `operation`, replacing any previous
    /// message/detail/time and incrementing the attempt count.
    pub fn record_daemon_error(
        &self,
        operation: DaemonOperation,
        message: &str,
        detail: Option<&str>,
    ) -> Result<()> {
        self.record_daemon_error_i18n(operation, message, None, None, detail)
    }

    /// Records a daemon failure with an optional catalog key and JSON args.
    /// `message` stays the fallback for older UIs and direct database readers.
    pub fn record_daemon_error_i18n(
        &self,
        operation: DaemonOperation,
        message: &str,
        message_key: Option<&str>,
        message_args: Option<&str>,
        detail: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO daemon_errors(operation, message, detail, occurred_at, attempts, \
                message_key, message_args) \
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6) \
             ON CONFLICT(operation) DO UPDATE SET message = excluded.message, \
                detail = excluded.detail, occurred_at = excluded.occurred_at, \
                attempts = daemon_errors.attempts + 1, \
                message_key = excluded.message_key, \
                message_args = excluded.message_args",
            params![
                operation.as_str(),
                message,
                detail,
                now_string(),
                message_key,
                message_args
            ],
        )?;
        Ok(())
    }

    /// Removes the stored error for `operation`, if any.
    pub fn clear_daemon_error(&self, operation: DaemonOperation) -> Result<()> {
        self.conn.execute(
            "DELETE FROM daemon_errors WHERE operation = ?1",
            params![operation.as_str()],
        )?;
        Ok(())
    }

    /// Returns every stored daemon error, newest first. Rows whose operation
    /// string is unknown to this build are skipped.
    pub fn daemon_errors(&self) -> Result<Vec<DaemonError>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT operation, message, detail, occurred_at, attempts, message_key, message_args \
             FROM daemon_errors ORDER BY occurred_at DESC, operation",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut errors = Vec::new();
        for row in rows {
            let (operation, message, detail, occurred_at, attempts, message_key, message_args) =
                row?;
            let Some(operation) = DaemonOperation::parse(&operation) else {
                continue;
            };
            errors.push(DaemonError {
                operation,
                message,
                detail,
                occurred_at,
                attempts,
                message_key,
                message_args,
            });
        }
        Ok(errors)
    }

    /// Inserts `command` unless the same command and payload are already
    /// queued. Returns whether a row was inserted.
    pub fn request_daemon_command(&self, command: DaemonCommand) -> Result<bool> {
        let kind = command.as_str();
        let payload = command.payload();
        let changed = self.conn.execute(
            "INSERT INTO daemon_commands(command, payload, created_at) \
             SELECT ?1, ?2, ?3 WHERE NOT EXISTS \
                (SELECT 1 FROM daemon_commands WHERE command = ?1 AND payload IS ?2)",
            params![kind, payload, now_string()],
        )?;
        Ok(changed == 1)
    }

    /// Queues a scan of one photo root. Multiple imports into the same root
    /// collapse into one request; different roots stay separate.
    pub fn request_daemon_scan_root(&self, path: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "INSERT INTO daemon_commands(command, payload, created_at) \
             SELECT 'scan_root', ?1, ?2 WHERE NOT EXISTS \
                (SELECT 1 FROM daemon_commands WHERE command = 'scan_root' AND payload = ?1)",
            params![path, now_string()],
        )?;
        Ok(changed == 1)
    }

    /// Returns whether any command is queued. Lets the daemon avoid opening a
    /// write transaction on every idle loop iteration.
    pub fn has_daemon_commands(&self) -> Result<bool> {
        let exists: bool =
            self.conn
                .query_row("SELECT EXISTS(SELECT 1 FROM daemon_commands)", [], |row| {
                    row.get(0)
                })?;
        Ok(exists)
    }

    /// Atomically removes and returns every queued command in request order.
    /// Unknown command strings are discarded.
    pub fn take_daemon_commands(&self) -> Result<Vec<DaemonCommand>> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let mut commands = Vec::new();
        {
            let mut stmt =
                tx.prepare_cached("SELECT command, payload FROM daemon_commands ORDER BY id")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (command, payload) = row?;
                if let Some(command) = DaemonCommand::parse(&command, payload.as_deref()) {
                    commands.push(command);
                }
            }
        }
        tx.execute("DELETE FROM daemon_commands", [])?;
        tx.commit()?;
        Ok(commands)
    }
}
