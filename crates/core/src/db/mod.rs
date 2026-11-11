use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::types::Value;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior, params,
    params_from_iter,
};
use vrc_photon_ai_protocol::{
    Catalog, ModelState, TagBranch, TaggingErrorCode, TaggingJobStatus, TaggingStage,
};

use crate::ai::{
    AI_PLUGIN_ID, AiTagDefinition, AiTagThreshold, MAX_TAGGING_RETRIES, NewPhotoAi, PhotoAi,
    PhotoAiTag, PhotoAiTagView, PluginInstallation, PluginState, TaggingJob,
};
use crate::error::{CoreError, Result};
use crate::model::{
    AiPhotoFilter, CompressionJob, CompressionProfile, DaemonCommand, DaemonError, DaemonOperation,
    DeletionBlockers, DeletionCounts, DeletionJobSummary, DeletionReason, DeletionRequest,
    DeletionStatus, Derivative, JobStatusCounts, LibraryRoot, LibraryStats, MAX_JOB_RETRIES,
    MAX_PROFILE_NAME_CHARS, NewPhoto, Photo, PhotoContext, PhotoFilter, PhotoIndexEntry,
    PhotoPlayer, PhotoSort, PhotoStatus, PhotoTextFilter, QueueStatus, RandomPhotoSample,
    SearchSuggestion, SendCompletion, SendErrorCode, SendHistoryEntry, SendKind, SendOrigin,
    SendOutboxEntry, SendSource, SendStatus, Session, SessionPlayer, SessionSummary, World,
    WorldPhotoCount,
};
use crate::world_name::{
    NOT_FOUND_RETRY_SECONDS, REQUEST_INTERVAL_SECONDS, WorldNameFailure, WorldNameLookup,
    WorldNameLookupCounts, WorldNameLookupStatus, backoff_seconds,
};

mod ai;
mod compression;
mod daemon;
mod deletion;
mod photos;
mod roots;
mod schema;
mod send;
mod sessions;
mod settings;
mod tags;
mod world_names;

use schema::*;

/// Backoff applied after consecutive failed link attempts: one hour, six
/// hours, one day, then weekly. Later attempts keep the weekly interval.
const LINK_RETRY_BACKOFF_SECONDS: [i64; 4] = [3_600, 21_600, 86_400, 604_800];

/// Sessions that end or start within an inclusive window around a capture.
/// `UNION` (not `UNION ALL`) collapses a zero-length session that matches both
/// ranges; the final ordering matches the caller's "at most two" contract.
const SESSIONS_NEAR_SQL: &str = "SELECT id, log_file, started_at, ended_at, world_id, instance_id \
     FROM sessions \
     WHERE started_at BETWEEN ?1 AND ?1 + ?2 \
     UNION \
     SELECT id, log_file, started_at, ended_at, world_id, instance_id \
     FROM sessions \
     WHERE ended_at IS NOT NULL AND ended_at BETWEEN ?1 - ?2 AND ?1 \
     ORDER BY started_at LIMIT 2";

/// Hard cap for any caller-supplied row limit. The product target is 100k
/// photos; 200k leaves headroom while preventing a buggy or hostile request
/// from materializing an unbounded result vector. This is also the cap for a
/// full text-index snapshot.
const MAX_ROW_LIMIT: i64 = 200_000;

/// Upper bound on session IDs accepted by [`Db::session_summaries`]. The UI
/// requests the sessions visible on one page, so this only guards against a
/// bug or an unbounded caller.
const MAX_SESSION_SUMMARY_IDS: usize = 256;

/// Normalizes a caller-supplied row limit.
///
/// Non-positive requests clamp to 1 so a missing limit never implies
/// "unlimited", and requests above [`MAX_ROW_LIMIT`] clamp to that safe
/// maximum. Methods whose zero has a distinct meaning (for example
/// [`Db::prune_send_history`], where zero removes all terminal rows) must not
/// use this helper and must document their policy instead.
fn clamp_limit(requested: i64) -> i64 {
    requested.clamp(1, MAX_ROW_LIMIT)
}

/// Mixes a 64-bit value for deterministic sampling keys.
fn splitmix64(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A lightweight, ordered photo index plus whether the query hit the row cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoIndexSnapshot {
    pub entries: Vec<PhotoIndexEntry>,
    pub truncated: bool,
}

pub struct Db {
    conn: Connection,
    path: Option<PathBuf>,
}

/// The database state a rescan compares against a scanned file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoScanState {
    pub id: i64,
    pub abs_path: String,
    pub file_size: u64,
    pub mtime: i64,
    pub blake3: Option<String>,
    pub status: PhotoStatus,
    pub display_derivative_id: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbProfile {
    /// Long-running service connections (the daemon) keep the original cache
    /// sizes because they run the heavy import and compression queries.
    Service,
    /// The UI process shares its address space with the renderer, thumbnail
    /// cache, and decoded full image, so its connections use a smaller page
    /// cache and mmap window.
    Ui,
}

impl DbProfile {
    const fn cache_size_kib(self) -> i64 {
        match self {
            Self::Service => 65_536,
            Self::Ui => 16_384,
        }
    }

    /// Prepared-statement cache slots per connection. The live poller and the
    /// library queries reuse several dozen fixed statements, so both profiles
    /// keep the same small cache.
    const fn statement_cache_capacity(self) -> usize {
        match self {
            Self::Service | Self::Ui => 64,
        }
    }

    const fn mmap_size_bytes(self) -> i64 {
        match self {
            Self::Service => 268_435_456,
            Self::Ui => 67_108_864,
        }
    }
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_profile(path, DbProfile::Service)
    }

    pub fn open_with_profile(path: impl AsRef<Path>, profile: DbProfile) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::configure(&conn, profile)?;
        let db = Self {
            conn,
            path: Some(path.to_path_buf()),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::open_in_memory_with_profile(DbProfile::Service)
    }

    pub fn open_in_memory_with_profile(profile: DbProfile) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn, profile)?;
        let db = Db { conn, path: None };
        db.migrate()?;
        Ok(db)
    }

    /// Opens an existing database file without running migrations, for readers
    /// such as background index workers. Callers should fall back to a
    /// synchronous query when this fails.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_read_only_with_profile(path, DbProfile::Service)
    }

    pub fn open_read_only_with_profile(path: impl AsRef<Path>, profile: DbProfile) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Self::configure(&conn, profile)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        check_schema_version(version)?;
        Ok(Self {
            conn,
            path: Some(path.to_path_buf()),
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn configure(conn: &Connection, profile: DbProfile) -> Result<()> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Best-effort tuning hints; the results are intentionally ignored. WAL
        // is unavailable for in-memory databases (they stay in `memory` journal
        // mode), and read-only connections may reject these writes.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        // Library snapshots and the stats aggregates scan six figures of rows
        // on a regular cadence. Keep a larger page cache and avoid spilling
        // ORDER BY temp b-trees to disk. These are per-connection hints; read-
        // only handles accept them, in-memory databases ignore mmap.
        let _ = conn.pragma_update(None, "cache_size", -profile.cache_size_kib());
        let _ = conn.pragma_update(None, "mmap_size", profile.mmap_size_bytes());
        let _ = conn.pragma_update(None, "temp_store", "MEMORY");
        conn.set_prepared_statement_cache_capacity(profile.statement_cache_capacity());
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        check_schema_version(version)?;
        if version < 1 {
            self.migrate_v1()?;
        }
        if version < 2 {
            self.migrate_v2()?;
        }
        if version < 3 {
            self.migrate_v3()?;
        }
        if version < 4 {
            self.migrate_v4()?;
        }
        if version < 5 {
            self.migrate_v5()?;
        }
        if version < 6 {
            self.migrate_v6()?;
        }
        if version < 7 {
            self.migrate_v7()?;
        }
        if version < 8 {
            self.migrate_v8()?;
        }
        if version < 9 {
            self.migrate_v9()?;
        }
        if version < 10 {
            self.migrate_v10()?;
        }
        if version < 11 {
            self.migrate_v11()?;
        }
        if version < 12 {
            self.migrate_v12()?;
        }
        if version < 13 {
            self.migrate_v13()?;
        }
        if version < 14 {
            self.migrate_v14()?;
        }
        if version < 15 {
            self.migrate_v15()?;
        }
        if version < 16 {
            self.migrate_v16()?;
        }
        if version < 17 {
            self.migrate_v17()?;
        }
        if version < 18 {
            self.migrate_v18()?;
        }
        Ok(())
    }

    /// Applies schema version 1 in one immediate transaction. The write lock
    /// serializes concurrent UI/daemon startups, and the `IF NOT EXISTS` DDL
    /// makes the step safe to retry after an interrupted upgrade.
    fn migrate_v1(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        // Re-read under the write lock: another process may have completed V1
        // between the caller's version check and this transaction.
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 1 {
            tx.commit()?;
            return Ok(());
        }

        tx.execute_batch(SCHEMA_V1)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (1, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
        Ok(())
    }

    /// Applies schema version 2 in one immediate transaction. The column
    /// check makes the step safe to retry after a crash between the ALTER
    /// and `user_version`, which the pre-transaction runner could leave
    /// behind as a duplicate column on the next open.
    fn migrate_v2(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 2 {
            tx.commit()?;
            return Ok(());
        }

        let has_author: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('photo_context') \
             WHERE name = 'author')",
            [],
            |row| row.get(0),
        )?;
        if !has_author {
            tx.execute_batch(SCHEMA_V2)?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (2, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
        Ok(())
    }

    /// Applies schema version 3 in one immediate transaction. The index uses
    /// `IF NOT EXISTS`, so the step is safe to retry after an interrupted
    /// upgrade.
    fn migrate_v3(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 3 {
            tx.commit()?;
            return Ok(());
        }

        tx.execute_batch(SCHEMA_V3)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (3, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 3)?;
        tx.commit()?;
        Ok(())
    }

    /// Applies schema version 4 in one immediate transaction. The write lock
    /// serializes concurrent UI/daemon startups, and the existence checks make
    /// the step safe to retry after an interrupted upgrade.
    fn migrate_v4(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        // Re-read under the write lock: another process may have completed V4
        // between the caller's version check and this transaction.
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 4 {
            tx.commit()?;
            return Ok(());
        }

        let has_retry_count: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('compression_jobs') \
             WHERE name = 'retry_count')",
            [],
            |row| row.get(0),
        )?;
        if !has_retry_count {
            tx.execute_batch(SCHEMA_V4_RETRY_COUNT)?;
        }
        tx.execute_batch(SCHEMA_V4_DAEMON_TABLES)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (4, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 4)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the primary derivative reference used by `replaced` photos and
    /// rebuilds the visible-photo index. The column check makes the step safe
    /// to retry after a crash between the ALTER and `user_version`.
    fn migrate_v5(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 5 {
            tx.commit()?;
            return Ok(());
        }

        let has_display_derivative: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('photos') \
             WHERE name = 'display_derivative_id')",
            [],
            |row| row.get(0),
        )?;
        if !has_display_derivative {
            tx.execute_batch(SCHEMA_V5_DISPLAY_DERIVATIVE)?;
        }
        tx.execute_batch(SCHEMA_V5_VISIBLE_INDEX)?;
        tx.execute_batch(SCHEMA_V5_MOVE_INDEX)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (5, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 5)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the per-job output directory used by manual compression. The
    /// column check makes the step safe to retry after an interrupted upgrade.
    fn migrate_v6(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 6 {
            tx.commit()?;
            return Ok(());
        }

        let has_output_dir: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('compression_jobs') \
             WHERE name = 'output_dir')",
            [],
            |row| row.get(0),
        )?;
        if !has_output_dir {
            tx.execute_batch(SCHEMA_V6_JOB_OUTPUT_DIR)?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (6, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 6)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the durable deletion queue. The table and indexes use
    /// `IF NOT EXISTS`, so the transaction is safe to retry after a crash
    /// between the DDL and `user_version`.
    fn migrate_v7(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 7 {
            tx.commit()?;
            return Ok(());
        }

        tx.execute_batch(SCHEMA_V7_DELETION)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (7, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 7)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the optional payload used by path-targeted daemon commands. The
    /// column check makes the step safe to retry after a crash between the
    /// ALTER and `user_version`.
    fn migrate_v8(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 8 {
            tx.commit()?;
            return Ok(());
        }

        let has_payload: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('daemon_commands') \
             WHERE name = 'payload')",
            [],
            |row| row.get(0),
        )?;
        if !has_payload {
            tx.execute_batch(SCHEMA_V8_COMMAND_PAYLOAD)?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (8, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 8)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the `(photo_id, profile_id)` index used by the automatic
    /// compression dedup check. `IF NOT EXISTS` makes the step safe to retry
    /// after a crash between the DDL and `user_version`.
    fn migrate_v9(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 9 {
            tx.commit()?;
            return Ok(());
        }

        tx.execute_batch(SCHEMA_V9_JOB_PHOTO_PROFILE_INDEX)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (9, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 9)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the key/args columns used to render stored daemon errors in the
    /// current language. Additive and nullable, so old rows keep their
    /// `message` fallback.
    fn migrate_v10(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 10 {
            tx.commit()?;
            return Ok(());
        }

        for (column, sql) in [
            ("message_key", SCHEMA_V10_MESSAGE_KEY),
            ("message_args", SCHEMA_V10_MESSAGE_ARGS),
        ] {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('daemon_errors') \
                 WHERE name = ?1)",
                params![column],
                |row| row.get(0),
            )?;
            if !exists {
                tx.execute_batch(sql)?;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (10, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 10)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the durable Discord send outbox. The table and indexes use
    /// `IF NOT EXISTS`, so the step is safe to retry after a crash between the
    /// DDL and `user_version`.
    fn migrate_v11(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 11 {
            tx.commit()?;
            return Ok(());
        }

        tx.execute_batch(SCHEMA_V11_SEND_OUTBOX)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (11, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 11)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the world name lookup queue. The table and index use
    /// `IF NOT EXISTS`, so the step is safe to retry after a crash between the
    /// DDL and `user_version`.
    fn migrate_v12(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 12 {
            tx.commit()?;
            return Ok(());
        }

        tx.execute_batch(SCHEMA_V12_WORLD_NAME_LOOKUPS)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (12, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 12)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the AI tagging plugin, catalog, result, and job tables. The
    /// tables and indexes use `IF NOT EXISTS`, so the step is safe to retry
    /// after a crash between the DDL and `user_version`.
    fn migrate_v13(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 13 {
            tx.commit()?;
            return Ok(());
        }

        tx.execute_batch(SCHEMA_V13_AI_TAGGING)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (13, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 13)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the claim/sort indexes for compression jobs and session time
    /// lookups. `IF NOT EXISTS` makes the step safe to retry after a crash
    /// between the DDL and `user_version`.
    fn migrate_v14(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 14 {
            tx.commit()?;
            return Ok(());
        }

        tx.execute_batch(SCHEMA_V14_INDEXES)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (14, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 14)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the link retry backoff columns and their due index. Each `ALTER`
    /// is guarded by a column check so the step is safe to retry after a
    /// crash between the DDL and `user_version`.
    fn migrate_v15(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 15 {
            tx.commit()?;
            return Ok(());
        }

        for (column, sql) in [
            ("link_attempts", SCHEMA_V15_LINK_ATTEMPTS),
            ("link_next_retry_at", SCHEMA_V15_LINK_NEXT_RETRY_AT),
        ] {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('photos') WHERE name = ?1)",
                params![column],
                |row| row.get(0),
            )?;
            if !exists {
                tx.execute_batch(sql)?;
            }
        }
        tx.execute_batch(SCHEMA_V15_LINK_RETRY_INDEX)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (15, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 15)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the atmosphere tag group to the catalog definitions. The column
    /// check makes the step safe to retry after a crash between the DDL and
    /// `user_version`.
    fn migrate_v16(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 16 {
            tx.commit()?;
            return Ok(());
        }

        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('ai_tag_definitions') \
             WHERE name = 'tag_group')",
            [],
            |row| row.get(0),
        )?;
        if !exists {
            tx.execute_batch(
                "ALTER TABLE ai_tag_definitions \
                 ADD COLUMN tag_group TEXT NOT NULL DEFAULT '';",
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (16, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 16)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the `photo_context.session_id` lookup index used by session
    /// summaries. `IF NOT EXISTS` makes the step safe to retry after an
    /// interrupted upgrade.
    fn migrate_v17(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 17 {
            tx.commit()?;
            return Ok(());
        }

        tx.execute_batch(SCHEMA_V17_CONTEXT_SESSION_INDEX)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (17, ?1)",
            params![now_string()],
        )?;
        tx.pragma_update(None, "user_version", 17)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the stored capture month/day columns.
    ///
    /// Fresh databases get them from [`SCHEMA_V1`]. Older databases need a
    /// table rebuild because SQLite refuses `ALTER TABLE ADD COLUMN ... STORED`.
    fn migrate_v18(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 18 {
            return Ok(());
        }

        let has_generated: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('photos') \
             WHERE name = 'captured_month')",
            [],
            |row| row.get(0),
        )?;
        if has_generated {
            let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (18, ?1)",
                params![now_string()],
            )?;
            tx.pragma_update(None, "user_version", 18)?;
            tx.commit()?;
            return Ok(());
        }

        // `DROP TABLE photos` is only allowed while foreign keys are off, and
        // the pragma is a no-op inside a transaction.
        self.conn.pragma_update(None, "foreign_keys", "OFF")?;
        let result = (|| -> Result<()> {
            let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V18_PHOTOS_GENERATED)?;
            tx.execute(
                "INSERT OR REPLACE INTO schema_migrations(version, applied_at) VALUES (18, ?1)",
                params![now_string()],
            )?;
            tx.pragma_update(None, "user_version", 18)?;
            tx.commit()?;
            Ok(())
        })();
        let restored = self.conn.pragma_update(None, "foreign_keys", "ON");
        match (result, restored) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error.into()),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// Writes one AI result and its judged tags. Returns false without touching
/// the database when the photo is no longer in the library.
fn check_schema_version(version: i64) -> Result<()> {
    if version > SCHEMA_VERSION {
        return Err(CoreError::Invalid(format!(
            "database schema version {version} is newer than this build supports \
             (maximum {SCHEMA_VERSION}); update VRC PhotoN before opening it"
        )));
    }
    Ok(())
}

fn now_string() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
