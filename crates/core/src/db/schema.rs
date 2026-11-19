/// Highest schema version this build understands.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// Schema version created by [`SCHEMA_V1`].
pub(crate) const SCHEMA_BASE_VERSION: i64 = 1;

/// Initial schema, frozen at v1. Do not edit; add a [`Migration`] instead.
pub(crate) const SCHEMA_V1: &str = r##"
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
CREATE TABLE ai_tag_definitions (
    plugin_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    branch TEXT NOT NULL,
    name TEXT NOT NULL,
    name_en TEXT,
    description TEXT NOT NULL,
    default_threshold REAL NOT NULL,
    sort_order INTEGER NOT NULL,
    catalog_version TEXT NOT NULL,
    updated_at TEXT NOT NULL, tag_group TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (plugin_id, tag_key)
);
CREATE TABLE ai_tag_thresholds (
    plugin_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    threshold REAL NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (plugin_id, tag_key)
);
CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE queue_progress (
    feature TEXT PRIMARY KEY,
    run_started_at TEXT,
    completed INTEGER NOT NULL DEFAULT 0
);
INSERT INTO queue_progress(feature) VALUES
    ('compression'), ('discord'), ('tagging');
CREATE TABLE compression_jobs (
    id TEXT PRIMARY KEY,
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    profile_id TEXT,
    origin TEXT NOT NULL CHECK (origin IN ('auto', 'manual')),
    status TEXT NOT NULL,
    progress REAL NOT NULL DEFAULT 0,
    error TEXT,
    output_path TEXT,
    policy TEXT NOT NULL DEFAULT 'keep',
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
, retry_count INTEGER NOT NULL DEFAULT 0, output_dir TEXT);
CREATE TABLE compression_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    json TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE daemon_commands (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command TEXT NOT NULL,
    created_at TEXT NOT NULL
, payload TEXT);
CREATE TABLE daemon_errors (
    operation TEXT PRIMARY KEY,
    message TEXT NOT NULL,
    detail TEXT,
    occurred_at TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 1
, message_key TEXT, message_args TEXT);
CREATE TABLE deletion_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    status TEXT NOT NULL,
    force INTEGER NOT NULL DEFAULT 0,
    blocked_reason TEXT,
    error TEXT,
    requested_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);
CREATE TABLE derivatives (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    path TEXT NOT NULL UNIQUE,
    format TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    file_size INTEGER NOT NULL,
    profile_id TEXT,
    origin TEXT NOT NULL CHECK (origin IN ('auto', 'manual')),
    policy TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE library_roots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE log_imports (
    log_file TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    last_offset INTEGER NOT NULL,
    parser_version TEXT NOT NULL,
    parsed_at TEXT NOT NULL
);
CREATE TABLE photo_ai (
    photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
    plugin_id TEXT NOT NULL,
    variant TEXT NOT NULL,
    sidecar_version TEXT NOT NULL,
    model_version TEXT NOT NULL,
    model_revision TEXT NOT NULL,
    catalog_version TEXT NOT NULL,
    source TEXT NOT NULL,
    presence_threshold REAL NOT NULL,
    character_present INTEGER NOT NULL,
    presence_probability REAL NOT NULL,
    people_min INTEGER,
    people_max INTEGER,
    people_median INTEGER,
    place_branch TEXT,
    place_probabilities_json TEXT,
    tagged_at TEXT NOT NULL
);
CREATE TABLE photo_ai_tags (
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    plugin_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    probability REAL NOT NULL,
    applied INTEGER NOT NULL,
    model_version TEXT NOT NULL,
    catalog_version TEXT NOT NULL,
    tagged_at TEXT NOT NULL,
    PRIMARY KEY (photo_id, plugin_id, tag_key)
);
CREATE TABLE photo_context (
    photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
    session_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
    world_id INTEGER REFERENCES worlds(id) ON DELETE SET NULL,
    world_name TEXT,
    instance_id TEXT,
    link_method TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0,
    linked_at TEXT NOT NULL
, author TEXT);
CREATE TABLE photo_players (
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    PRIMARY KEY(photo_id, user_id)
);
CREATE TABLE photo_tags (
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY(photo_id, tag_id)
);
CREATE TABLE "photos" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id INTEGER NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
    rel_path TEXT NOT NULL,
    abs_path TEXT NOT NULL UNIQUE,
    file_name TEXT NOT NULL,
    captured_at_utc INTEGER NOT NULL,
    captured_at_local TEXT NOT NULL,
    tz_offset_sec INTEGER NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    file_size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    blake3 TEXT,
    status TEXT NOT NULL DEFAULT 'present',
    is_favorite INTEGER NOT NULL DEFAULT 0,
    note TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    display_derivative_id INTEGER REFERENCES derivatives(id) ON DELETE SET NULL,
    link_attempts INTEGER NOT NULL DEFAULT 0,
    link_next_retry_at INTEGER,
    captured_month INTEGER GENERATED ALWAYS AS (
        COALESCE(
            CAST(strftime('%Y%m', captured_at_local) AS INTEGER),
            CAST(strftime('%Y%m', captured_at_utc, 'unixepoch') AS INTEGER),
            0
        )
    ) STORED,
    captured_day INTEGER GENERATED ALWAYS AS (
        COALESCE(
            CAST(strftime('%Y%m%d', captured_at_local) AS INTEGER),
            CAST(strftime('%Y%m%d', captured_at_utc, 'unixepoch') AS INTEGER),
            0
        )
    ) STORED,
    UNIQUE(root_id, rel_path)
);
CREATE TABLE plugin_installations (
    plugin_id TEXT NOT NULL,
    variant TEXT NOT NULL,
    version TEXT NOT NULL,
    protocol_version INTEGER NOT NULL,
    path TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    catalog_version TEXT,
    active INTEGER NOT NULL DEFAULT 0,
    installed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (plugin_id, variant, version)
);
CREATE TABLE plugin_state (
    plugin_id TEXT PRIMARY KEY,
    model_state TEXT NOT NULL,
    model_revision TEXT,
    model_bytes_downloaded INTEGER NOT NULL DEFAULT 0,
    model_bytes_total INTEGER,
    model_error TEXT,
    probe_json TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE send_outbox (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id           INTEGER REFERENCES photos(id),
    kind               TEXT NOT NULL,   -- photo | test
    origin             TEXT NOT NULL,   -- manual | auto
    status             TEXT NOT NULL,   -- waiting | pending | processing | sent | failed | cancelled
    source             TEXT,            -- original | profile | derivative
    profile_id         TEXT,
    destination        TEXT,
    attempts           INTEGER NOT NULL DEFAULT 0,
    next_attempt_at    INTEGER,
    discord_message_id TEXT,
    discord_channel_id TEXT,
    discord_guild_id   TEXT,
    error_code         TEXT,
    error_detail       TEXT,
    created_at         TEXT NOT NULL,
    started_at         TEXT,
    finished_at        TEXT
);
CREATE TABLE session_players (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    joined_at INTEGER NOT NULL,
    left_at INTEGER,
    UNIQUE(session_id, user_id)
);
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    log_file TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    world_id INTEGER NOT NULL REFERENCES worlds(id),
    instance_id TEXT
);
CREATE TABLE tagging_jobs (
    id TEXT PRIMARY KEY,
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    plugin_id TEXT NOT NULL,
    variant TEXT,
    status TEXT NOT NULL,
    progress REAL NOT NULL DEFAULT 0,
    stage TEXT,
    error TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);
CREATE TABLE tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE world_name_lookups (
    world_db_id INTEGER PRIMARY KEY REFERENCES worlds(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER,
    last_attempt_at INTEGER,
    last_error TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE worlds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    world_id TEXT NOT NULL UNIQUE,
    name TEXT,
    name_source TEXT,
    alias TEXT,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL
);
CREATE INDEX idx_context_instance ON photo_context(instance_id);
CREATE INDEX idx_context_session ON photo_context(session_id);
CREATE INDEX idx_context_world ON photo_context(world_id);
CREATE UNIQUE INDEX idx_deletion_requests_active
    ON deletion_requests(photo_id)
    WHERE status IN ('pending', 'blocked', 'running');
CREATE INDEX idx_deletion_requests_status
    ON deletion_requests(status, requested_at);
CREATE INDEX idx_derivatives_photo ON derivatives(photo_id);
CREATE INDEX idx_derivatives_auto ON derivatives(photo_id) WHERE origin = 'auto';
CREATE INDEX idx_jobs_claim ON compression_jobs(status, created_at);
CREATE INDEX idx_jobs_created ON compression_jobs(created_at DESC);
CREATE UNIQUE INDEX idx_jobs_auto_once
    ON compression_jobs(photo_id) WHERE origin = 'auto';
CREATE INDEX idx_jobs_photo_profile
    ON compression_jobs(photo_id, profile_id);
CREATE INDEX idx_jobs_status ON compression_jobs(status);
CREATE INDEX idx_photo_ai_people
    ON photo_ai(people_min, people_max);
CREATE INDEX idx_photo_ai_tags_applied
    ON photo_ai_tags(plugin_id, tag_key, applied, photo_id);
CREATE INDEX idx_photo_players_name ON photo_players(display_name);
CREATE INDEX idx_photo_players_user ON photo_players(user_id);
CREATE INDEX idx_photo_tags_tag ON photo_tags(tag_id);
CREATE INDEX idx_photos_blake3 ON photos(blake3);
CREATE INDEX idx_photos_captured ON photos(captured_at_utc DESC);
CREATE INDEX idx_photos_captured_id
    ON photos(captured_at_utc DESC, id DESC)
    WHERE status IN ('present', 'replaced');
CREATE INDEX idx_photos_favorite ON photos(is_favorite) WHERE is_favorite = 1;
CREATE INDEX idx_photos_link_retry ON photos(link_next_retry_at)
    WHERE status IN ('present', 'replaced');
CREATE INDEX idx_photos_move_match
    ON photos(status, file_size, captured_at_utc);
CREATE UNIQUE INDEX idx_plugin_installations_one_active
    ON plugin_installations(plugin_id) WHERE active = 1;
CREATE UNIQUE INDEX idx_send_outbox_active_photo
    ON send_outbox(photo_id)
    WHERE photo_id IS NOT NULL
      AND status IN ('waiting', 'pending', 'processing');
CREATE INDEX idx_send_outbox_due
    ON send_outbox(status, next_attempt_at, id);
CREATE INDEX idx_send_outbox_photo
    ON send_outbox(photo_id);
CREATE INDEX idx_sessions_ended ON sessions(ended_at) WHERE ended_at IS NOT NULL;
CREATE INDEX idx_sessions_started ON sessions(started_at);
CREATE UNIQUE INDEX idx_tagging_jobs_active
    ON tagging_jobs(photo_id, plugin_id)
    WHERE status IN ('waiting', 'pending', 'processing');
CREATE INDEX idx_tagging_jobs_active_list
    ON tagging_jobs(created_at DESC, id DESC)
    WHERE status IN ('waiting', 'pending', 'processing');
CREATE INDEX idx_tagging_jobs_claim
    ON tagging_jobs(status, created_at);
CREATE INDEX idx_tagging_jobs_photo
    ON tagging_jobs(photo_id, created_at);
CREATE INDEX idx_tagging_jobs_plugin_status
    ON tagging_jobs(plugin_id, status);
CREATE INDEX idx_tagging_jobs_status
    ON tagging_jobs(status);
CREATE INDEX idx_world_name_lookups_due
    ON world_name_lookups(status, next_attempt_at);

CREATE TRIGGER compression_jobs_progress_start
AFTER INSERT ON compression_jobs
WHEN NEW.status IN ('waiting', 'pending', 'processing')
BEGIN
    UPDATE queue_progress
    SET run_started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), completed = 0
    WHERE feature = 'compression'
      AND NOT EXISTS (
          SELECT 1 FROM compression_jobs
          WHERE status IN ('waiting', 'pending', 'processing') AND id <> NEW.id
      );
END;

CREATE TRIGGER compression_jobs_progress_resume
AFTER UPDATE OF status ON compression_jobs
WHEN NEW.status IN ('waiting', 'pending', 'processing')
 AND OLD.status NOT IN ('waiting', 'pending', 'processing')
BEGIN
    UPDATE queue_progress
    SET run_started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), completed = 0
    WHERE feature = 'compression'
      AND NOT EXISTS (
          SELECT 1 FROM compression_jobs
          WHERE status IN ('waiting', 'pending', 'processing') AND id <> NEW.id
      );
END;

CREATE TRIGGER compression_jobs_progress_complete
AFTER UPDATE OF status ON compression_jobs
WHEN NEW.status = 'completed' AND OLD.status <> 'completed'
BEGIN
    UPDATE queue_progress
    SET completed = completed + 1
    WHERE feature = 'compression' AND run_started_at IS NOT NULL;
END;

CREATE TRIGGER send_outbox_progress_start
AFTER INSERT ON send_outbox
WHEN NEW.status IN ('waiting', 'pending', 'processing')
BEGIN
    UPDATE queue_progress
    SET run_started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), completed = 0
    WHERE feature = 'discord'
      AND NOT EXISTS (
          SELECT 1 FROM send_outbox
          WHERE status IN ('waiting', 'pending', 'processing') AND id <> NEW.id
      );
END;

CREATE TRIGGER send_outbox_progress_resume
AFTER UPDATE OF status ON send_outbox
WHEN NEW.status IN ('waiting', 'pending', 'processing')
 AND OLD.status NOT IN ('waiting', 'pending', 'processing')
BEGIN
    UPDATE queue_progress
    SET run_started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), completed = 0
    WHERE feature = 'discord'
      AND NOT EXISTS (
          SELECT 1 FROM send_outbox
          WHERE status IN ('waiting', 'pending', 'processing') AND id <> NEW.id
      );
END;

CREATE TRIGGER send_outbox_progress_complete
AFTER UPDATE OF status ON send_outbox
WHEN NEW.status = 'sent' AND OLD.status <> 'sent'
BEGIN
    UPDATE queue_progress
    SET completed = completed + 1
    WHERE feature = 'discord' AND run_started_at IS NOT NULL;
END;

CREATE TRIGGER tagging_jobs_progress_start
AFTER INSERT ON tagging_jobs
WHEN NEW.status IN ('waiting', 'pending', 'processing')
BEGIN
    UPDATE queue_progress
    SET run_started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), completed = 0
    WHERE feature = 'tagging'
      AND NOT EXISTS (
          SELECT 1 FROM tagging_jobs
          WHERE status IN ('waiting', 'pending', 'processing') AND id <> NEW.id
      );
END;

CREATE TRIGGER tagging_jobs_progress_resume
AFTER UPDATE OF status ON tagging_jobs
WHEN NEW.status IN ('waiting', 'pending', 'processing')
 AND OLD.status NOT IN ('waiting', 'pending', 'processing')
BEGIN
    UPDATE queue_progress
    SET run_started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), completed = 0
    WHERE feature = 'tagging'
      AND NOT EXISTS (
          SELECT 1 FROM tagging_jobs
          WHERE status IN ('waiting', 'pending', 'processing') AND id <> NEW.id
      );
END;

CREATE TRIGGER tagging_jobs_progress_complete
AFTER UPDATE OF status ON tagging_jobs
WHEN NEW.status = 'completed' AND OLD.status <> 'completed'
BEGIN
    UPDATE queue_progress
    SET completed = completed + 1
    WHERE feature = 'tagging' AND run_started_at IS NOT NULL;
END;
"##;

/// One schema upgrade step. Steps are applied in ascending, gap-free order.
pub(crate) struct Migration {
    pub version: i64,
    pub sql: &'static str,
}

/// Ordered one-version-at-a-time migrations above [`SCHEMA_BASE_VERSION`].
/// 1.0.0 時点では 0 件。
pub(crate) const MIGRATIONS: &[Migration] = &[];

use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::error::{CoreError, Result};

/// Creates the v1 schema and applies every pending migration in one immediate
/// transaction.
pub(super) fn initialize(conn: &Connection) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    // Another process may have initialized or migrated the database while this
    // one waited for the write lock. Re-check under the lock before writing.
    let mut version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == SCHEMA_VERSION {
        tx.commit()?;
        return Ok(());
    }
    if version > SCHEMA_VERSION {
        return Err(CoreError::Invalid(format!(
            "database schema version {version} is newer than this build supports \
             (maximum {SCHEMA_VERSION}); update VRC PhotoN before opening it"
        )));
    }
    if version == 0 {
        let tables: i64 = tx.query_row(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if tables > 0 {
            return Err(CoreError::Invalid(
                "database has tables but user_version is 0; refusing to modify it".to_string(),
            ));
        }
        tx.execute_batch(SCHEMA_V1)?;
        tx.pragma_update(None, "user_version", SCHEMA_BASE_VERSION)?;
        tx.execute(
            "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
            params![SCHEMA_BASE_VERSION, super::now_string()],
        )?;
        version = SCHEMA_BASE_VERSION;
    }
    apply_migrations(&tx, version, MIGRATIONS, SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

/// Applies `migrations` from `version` through `target`, one version at a time.
///
/// The caller owns the `IMMEDIATE` transaction and must have re-read
/// `user_version` after acquiring its lock.
pub(crate) fn apply_migrations(
    tx: &Transaction<'_>,
    mut version: i64,
    migrations: &[Migration],
    target: i64,
) -> Result<i64> {
    for migration in migrations {
        if migration.version <= version {
            continue;
        }
        if migration.version != version + 1 {
            return Err(CoreError::Invalid(format!(
                "schema migration {} does not follow version {version}",
                migration.version
            )));
        }
        if migration.version > target {
            return Err(CoreError::Invalid(format!(
                "schema migration {} exceeds schema version {target}",
                migration.version
            )));
        }
        tx.execute_batch(migration.sql)?;
        tx.execute(
            "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
            params![migration.version, super::now_string()],
        )?;
        tx.pragma_update(None, "user_version", migration.version)?;
        version = migration.version;
    }
    if version != target {
        return Err(CoreError::Invalid(format!(
            "database schema version {version} cannot be migrated to {target}"
        )));
    }
    Ok(version)
}
