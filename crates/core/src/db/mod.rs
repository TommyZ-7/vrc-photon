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
    AiPhotoFilter, CompressionJob, CompressionOrigin, CompressionProfile, DaemonCommand,
    DaemonError, DaemonOperation, DeletionBlockers, DeletionCounts, DeletionJobSummary,
    DeletionReason, DeletionRequest, DeletionStatus, Derivative, JobStatusCounts, LibraryRoot,
    LibraryStats, MAX_JOB_RETRIES, MAX_PROFILE_NAME_CHARS, NewPhoto, Photo, PhotoContext,
    PhotoFilter, PhotoIndexEntry, PhotoPlayer, PhotoSort, PhotoStatus, PhotoTextFilter,
    QueueStatus, RandomPhotoSample, SearchSuggestion, SendCompletion, SendErrorCode,
    SendHistoryEntry, SendKind, SendOrigin, SendOutboxEntry, SendSource, SendStatus, Session,
    SessionPlayer, SessionSummary, World, WorldPhotoCount,
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
        let version = schema_version(&conn)?;
        if version != 0 && version != SCHEMA_VERSION {
            check_schema_version(version)?;
        }
        if version == 0 && has_user_tables(&conn)? {
            return Err(CoreError::Invalid(
                "database has tables but user_version is 0; refusing to modify it".to_string(),
            ));
        }
        Self::configure(&conn, profile)?;
        if version == 0 {
            schema::initialize(&conn)?;
        }
        Ok(Self {
            conn,
            path: Some(path.to_path_buf()),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::open_in_memory_with_profile(DbProfile::Service)
    }

    pub fn open_in_memory_with_profile(profile: DbProfile) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn, profile)?;
        schema::initialize(&conn)?;
        Ok(Self { conn, path: None })
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
        check_schema_version(schema_version(&conn)?)?;
        Self::configure(&conn, profile)?;
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

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// Writes one AI result and its judged tags. Returns false without touching
/// the database when the photo is no longer in the library.
fn schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn has_user_tables(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn check_schema_version(version: i64) -> Result<()> {
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    if (1..SCHEMA_VERSION).contains(&version) {
        return Err(CoreError::Invalid(format!(
            "database schema version {version} cannot be migrated by this build; \
             back up the database and create a new library (supported version: {SCHEMA_VERSION})"
        )));
    }
    if version > SCHEMA_VERSION {
        return Err(CoreError::Invalid(format!(
            "database schema version {version} is newer than this build supports \
             (maximum {SCHEMA_VERSION}); update VRC PhotoN before opening it"
        )));
    }
    Err(CoreError::Invalid(format!(
        "database schema version {version} is not supported; expected {SCHEMA_VERSION}"
    )))
}

fn now_string() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
