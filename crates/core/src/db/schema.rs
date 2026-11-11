/// Highest schema version this build understands. A database with a newer
/// `user_version` was written by a future release and must not be opened.
pub(crate) const SCHEMA_VERSION: i64 = 18;

pub(crate) const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS library_roots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS worlds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    world_id TEXT NOT NULL UNIQUE,
    name TEXT,
    name_source TEXT,
    alias TEXT,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    log_file TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    world_id INTEGER NOT NULL REFERENCES worlds(id),
    instance_id TEXT
);

CREATE TABLE IF NOT EXISTS session_players (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    joined_at INTEGER NOT NULL,
    left_at INTEGER,
    UNIQUE(session_id, user_id)
);

CREATE TABLE IF NOT EXISTS photos (
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

CREATE INDEX IF NOT EXISTS idx_photos_captured ON photos(captured_at_utc DESC);
CREATE INDEX IF NOT EXISTS idx_photos_favorite ON photos(is_favorite) WHERE is_favorite = 1;
CREATE INDEX IF NOT EXISTS idx_photos_blake3 ON photos(blake3);

CREATE TABLE IF NOT EXISTS photo_context (
    photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
    session_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
    world_id INTEGER REFERENCES worlds(id) ON DELETE SET NULL,
    world_name TEXT,
    instance_id TEXT,
    link_method TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0,
    linked_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_context_world ON photo_context(world_id);
CREATE INDEX IF NOT EXISTS idx_context_instance ON photo_context(instance_id);

CREATE TABLE IF NOT EXISTS photo_players (
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    PRIMARY KEY(photo_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_photo_players_user ON photo_players(user_id);
CREATE INDEX IF NOT EXISTS idx_photo_players_name ON photo_players(display_name);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS photo_tags (
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY(photo_id, tag_id)
);

CREATE INDEX IF NOT EXISTS idx_photo_tags_tag ON photo_tags(tag_id);

CREATE TABLE IF NOT EXISTS compression_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    auto_enabled INTEGER NOT NULL,
    json TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS derivatives (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    path TEXT NOT NULL UNIQUE,
    format TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    file_size INTEGER NOT NULL,
    profile_id TEXT NOT NULL,
    policy TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_derivatives_photo ON derivatives(photo_id);

CREATE TABLE IF NOT EXISTS compression_jobs (
    id TEXT PRIMARY KEY,
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL,
    status TEXT NOT NULL,
    progress REAL NOT NULL DEFAULT 0,
    error TEXT,
    output_path TEXT,
    policy TEXT NOT NULL DEFAULT 'keep',
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_jobs_status ON compression_jobs(status);
CREATE INDEX IF NOT EXISTS idx_jobs_photo_profile
    ON compression_jobs(photo_id, profile_id);

CREATE TABLE IF NOT EXISTS log_imports (
    log_file TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    last_offset INTEGER NOT NULL,
    parser_version TEXT NOT NULL,
    parsed_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

pub(crate) const SCHEMA_V2: &str = r#"
ALTER TABLE photo_context ADD COLUMN author TEXT;
"#;

pub(crate) const SCHEMA_V3: &str = r#"
CREATE INDEX IF NOT EXISTS idx_photos_captured_id
    ON photos(captured_at_utc DESC, id DESC)
    WHERE status = 'present';
"#;

pub(crate) const SCHEMA_V4_RETRY_COUNT: &str =
    "ALTER TABLE compression_jobs ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;";

pub(crate) const SCHEMA_V4_DAEMON_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS daemon_errors (
    operation TEXT PRIMARY KEY,
    message TEXT NOT NULL,
    detail TEXT,
    occurred_at TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS daemon_commands (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command TEXT NOT NULL,
    created_at TEXT NOT NULL
);
"#;

pub(crate) const SCHEMA_V5_DISPLAY_DERIVATIVE: &str = r#"
ALTER TABLE photos ADD COLUMN display_derivative_id INTEGER
    REFERENCES derivatives(id) ON DELETE SET NULL;
"#;

pub(crate) const SCHEMA_V5_VISIBLE_INDEX: &str = r#"
DROP INDEX IF EXISTS idx_photos_captured_id;
CREATE INDEX IF NOT EXISTS idx_photos_captured_id
    ON photos(captured_at_utc DESC, id DESC)
    WHERE status IN ('present', 'replaced');
"#;

pub(crate) const SCHEMA_V5_MOVE_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_photos_move_match
    ON photos(status, file_size, captured_at_utc);
"#;

pub(crate) const SCHEMA_V6_JOB_OUTPUT_DIR: &str =
    "ALTER TABLE compression_jobs ADD COLUMN output_dir TEXT;";

pub(crate) const SCHEMA_V7_DELETION: &str = r#"
CREATE TABLE IF NOT EXISTS deletion_requests (
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

CREATE UNIQUE INDEX IF NOT EXISTS idx_deletion_requests_active
    ON deletion_requests(photo_id)
    WHERE status IN ('pending', 'blocked', 'running');

CREATE INDEX IF NOT EXISTS idx_deletion_requests_status
    ON deletion_requests(status, requested_at);
"#;

pub(crate) const SCHEMA_V8_COMMAND_PAYLOAD: &str =
    "ALTER TABLE daemon_commands ADD COLUMN payload TEXT;";

pub(crate) const SCHEMA_V9_JOB_PHOTO_PROFILE_INDEX: &str = "CREATE INDEX IF NOT EXISTS idx_jobs_photo_profile \
     ON compression_jobs(photo_id, profile_id);";

pub(crate) const SCHEMA_V10_MESSAGE_KEY: &str =
    "ALTER TABLE daemon_errors ADD COLUMN message_key TEXT;";
pub(crate) const SCHEMA_V10_MESSAGE_ARGS: &str =
    "ALTER TABLE daemon_errors ADD COLUMN message_args TEXT;";

pub(crate) const SCHEMA_V11_SEND_OUTBOX: &str = r#"
CREATE TABLE IF NOT EXISTS send_outbox (
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

CREATE UNIQUE INDEX IF NOT EXISTS idx_send_outbox_active_photo
    ON send_outbox(photo_id)
    WHERE photo_id IS NOT NULL
      AND status IN ('waiting', 'pending', 'processing');

CREATE INDEX IF NOT EXISTS idx_send_outbox_due
    ON send_outbox(status, next_attempt_at, id);

CREATE INDEX IF NOT EXISTS idx_send_outbox_photo
    ON send_outbox(photo_id);
"#;

pub(crate) const SCHEMA_V12_WORLD_NAME_LOOKUPS: &str = r#"
CREATE TABLE IF NOT EXISTS world_name_lookups (
    world_db_id INTEGER PRIMARY KEY REFERENCES worlds(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER,
    last_attempt_at INTEGER,
    last_error TEXT,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_world_name_lookups_due
    ON world_name_lookups(status, next_attempt_at);
"#;

pub(crate) const SCHEMA_V13_AI_TAGGING: &str = r#"
CREATE TABLE IF NOT EXISTS plugin_installations (
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

CREATE UNIQUE INDEX IF NOT EXISTS idx_plugin_installations_one_active
    ON plugin_installations(plugin_id) WHERE active = 1;

CREATE TABLE IF NOT EXISTS plugin_state (
    plugin_id TEXT PRIMARY KEY,
    model_state TEXT NOT NULL,
    model_revision TEXT,
    model_bytes_downloaded INTEGER NOT NULL DEFAULT 0,
    model_bytes_total INTEGER,
    model_error TEXT,
    probe_json TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ai_tag_definitions (
    plugin_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    branch TEXT NOT NULL,
    name TEXT NOT NULL,
    name_en TEXT,
    description TEXT NOT NULL,
    default_threshold REAL NOT NULL,
    sort_order INTEGER NOT NULL,
    catalog_version TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (plugin_id, tag_key)
);

CREATE TABLE IF NOT EXISTS ai_tag_thresholds (
    plugin_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    threshold REAL NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (plugin_id, tag_key)
);

CREATE TABLE IF NOT EXISTS photo_ai (
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

CREATE TABLE IF NOT EXISTS photo_ai_tags (
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

CREATE TABLE IF NOT EXISTS tagging_jobs (
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

CREATE INDEX IF NOT EXISTS idx_photo_ai_tags_applied
    ON photo_ai_tags(plugin_id, tag_key, applied, photo_id);
CREATE INDEX IF NOT EXISTS idx_photo_ai_people
    ON photo_ai(people_min, people_max);
CREATE INDEX IF NOT EXISTS idx_tagging_jobs_status
    ON tagging_jobs(status);
CREATE INDEX IF NOT EXISTS idx_tagging_jobs_claim
    ON tagging_jobs(status, created_at);
CREATE INDEX IF NOT EXISTS idx_tagging_jobs_photo
    ON tagging_jobs(photo_id, created_at);
CREATE INDEX IF NOT EXISTS idx_tagging_jobs_plugin_status
    ON tagging_jobs(plugin_id, status);
CREATE INDEX IF NOT EXISTS idx_tagging_jobs_active_list
    ON tagging_jobs(created_at DESC, id DESC)
    WHERE status IN ('waiting', 'pending', 'processing');
CREATE UNIQUE INDEX IF NOT EXISTS idx_tagging_jobs_active
    ON tagging_jobs(photo_id, plugin_id)
    WHERE status IN ('waiting', 'pending', 'processing');
"#;

pub(crate) const SCHEMA_V14_INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_jobs_claim ON compression_jobs(status, created_at);
CREATE INDEX IF NOT EXISTS idx_jobs_created ON compression_jobs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);
CREATE INDEX IF NOT EXISTS idx_sessions_ended ON sessions(ended_at) WHERE ended_at IS NOT NULL;
"#;

pub(crate) const SCHEMA_V15_LINK_ATTEMPTS: &str =
    "ALTER TABLE photos ADD COLUMN link_attempts INTEGER NOT NULL DEFAULT 0;";

pub(crate) const SCHEMA_V15_LINK_NEXT_RETRY_AT: &str =
    "ALTER TABLE photos ADD COLUMN link_next_retry_at INTEGER;";

pub(crate) const SCHEMA_V15_LINK_RETRY_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_photos_link_retry ON photos(link_next_retry_at)
    WHERE status IN ('present', 'replaced');
"#;

pub(crate) const SCHEMA_V17_CONTEXT_SESSION_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_context_session ON photo_context(session_id);";

/// Rebuilds `photos` with the stored capture month/day columns.
///
/// SQLite cannot add a STORED generated column with `ALTER TABLE ADD COLUMN`,
/// so the table is recreated with the exact final shape. All non-generated
/// columns are copied, then every index is recreated. `migrate_v18` disables
/// foreign keys around this batch so dropping the old table is allowed.
pub(crate) const SCHEMA_V18_PHOTOS_GENERATED: &str = r#"
CREATE TABLE photos_v18 (
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

INSERT INTO photos_v18 (
    id, root_id, rel_path, abs_path, file_name, captured_at_utc,
    captured_at_local, tz_offset_sec, width, height, file_size, mtime,
    blake3, status, is_favorite, note, created_at, updated_at,
    display_derivative_id, link_attempts, link_next_retry_at
)
SELECT
    id, root_id, rel_path, abs_path, file_name, captured_at_utc,
    captured_at_local, tz_offset_sec, width, height, file_size, mtime,
    blake3, status, is_favorite, note, created_at, updated_at,
    display_derivative_id, link_attempts, link_next_retry_at
FROM photos;

DROP TABLE photos;
ALTER TABLE photos_v18 RENAME TO photos;

CREATE INDEX IF NOT EXISTS idx_photos_captured ON photos(captured_at_utc DESC);
CREATE INDEX IF NOT EXISTS idx_photos_favorite ON photos(is_favorite) WHERE is_favorite = 1;
CREATE INDEX IF NOT EXISTS idx_photos_blake3 ON photos(blake3);
CREATE INDEX IF NOT EXISTS idx_photos_captured_id
    ON photos(captured_at_utc DESC, id DESC)
    WHERE status IN ('present', 'replaced');
CREATE INDEX IF NOT EXISTS idx_photos_move_match
    ON photos(status, file_size, captured_at_utc);
CREATE INDEX IF NOT EXISTS idx_photos_link_retry ON photos(link_next_retry_at)
    WHERE status IN ('present', 'replaced');

DELETE FROM sqlite_sequence WHERE name IN ('photos', 'photos_v18');
INSERT INTO sqlite_sequence(name, seq)
    SELECT 'photos', COALESCE(MAX(id), 0) FROM photos;
"#;
