# 1.0.0 バージョンリセットと 0.x 互換コード削除 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** アプリを 1.0.0 とし、DB schema を v1、UI⇔daemon IPC を v1、デモデータを v1 に振り直す。0.x を最初のインストールとして扱い、旧 DB・旧設定・旧キャッシュ・旧プロトコルの互換コードとテストを削除する。

**Architecture:** DB は凍結した `SCHEMA_V1` と、1 版ずつ適用する `MIGRATIONS` ランナーを持ち、`user_version` を正本、`schema_migrations` を適用履歴として記録する。新規 DB は v1 作成後に同じランナーを適用し、既存 DB は `IMMEDIATE` トランザクション内でロック取得後に `user_version` を再確認してから移行する。`open_read_only` は移行しない。IPC / デモ / 設定 / サムネイル / アップデーター / 検索の旧形式受理は削除する。

**Tech Stack:** Rust stable（edition 2024）、SQLite（rusqlite）、serde、GPUI、GitHub Actions。

**Spec:** GitHub issue #688（本計画の「決定事項」で確定）

## 決定事項

- アプリ本体の `1.0.0` bump もこの実装 PR に含める。
- `AppSettings` は container-level `#[serde(default)]` と旧互換の field default / alias を削除し、1.0.0 が書いた完全な設定だけを受け付ける。欠損・旧形式は parse error とし、既存の corrupt backup 経路に任せる。
- DB のマイグレーション履歴は `schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)` に記録する。`user_version` は起動時の判定に使う正本として残す。
- デーモンエラーの `message_key: None` は技術メッセージ用に残す。旧行と技術メッセージを区別しないため、保存済み `message` を表示するフォールバックは維持する。旧行専用の分岐は追加しない。
- 実装は 1 本の PR にまとめる。コミットは下記 Task 単位で分ける。
- 0.x の DB / 設定 / キャッシュからの移行は実装しない。v1 より大きい `user_version`（旧 0.x の 18/20/21 を含む）は「新しすぎる DB」としてファイルを変更せず拒否する。

## Global Constraints

- アプリ版: `Cargo.toml` / `Cargo.lock` / `Packager.toml` / `packaging/licenses/THIRD-PARTY-NOTICES.md` / `docs/ui.md` を 1.0.0 に揃える。
- DB: `SCHEMA_VERSION = 1`、`SCHEMA_BASE_VERSION = 1`、`SCHEMA_V1` は凍結。1.0.0 時点の `MIGRATIONS` は 0 件。
- IPC: `PROTOCOL_VERSION = 1`。旧 daemon 互換の `#[serde(default)]` と `StatusSnapshot.version == 0` 受理を削除する。
- デモ: `DEMO_DATA_VERSION = 1`。`previews` を許可エントリから外す。
- AI プラグイン fixture: `0.99.99` を `1.99.99` に更新し、1.0.0 を含める。`packaging/ai-plugin.json` の `host_version_min` は `1.0.0` にする。
- `SCHEMA_V1` は今後変更しない。schema 変更は `MIGRATIONS` に 1 版ずつ追加し、`SCHEMA_VERSION` を上げる。
- 移行は `IMMEDIATE` トランザクションで行い、ロック取得後に `user_version` を再確認する。途中失敗はロールバックする。
- `Db::open_read_only` は移行せず、`SCHEMA_VERSION` と一致する DB だけを受け付ける。
- schema / IPC の値を参照する docs は `scripts/check-docs-drift.sh` の都合で同じ PR に含める。
- `crates/core/src/db/ai.rs` の `photo_ai_tags_with_names` にある「カタログから消えたタグを生キーで表示する」フォールバックは実行時の防御なので削除しない。
- リリース操作（tag 作成 / push、Release workflow の手動実行、`gh release` での公開）は行わない。
- ローカル最終検証:
  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ./scripts/check-docs-drift.sh
  ```

## Review Focus

1. **旧 0.x DB（user_version 18/20/21）** — v1 より数値が大きいため「新しすぎる DB」として拒否される。移行を試みたり、ファイルを変更したりしないこと。Task 1 の拒否テストで固定する。
2. **旧形式・欠損ありの `app_settings` JSON** — 型既定で補完せず、parse error として corrupt backup に退避し、元の行を書き換えないこと。Task 4 の strict 設定テストで固定する。
3. **`message_key: None` の技術メッセージ** — 保存済み `message` がそのまま表示され、翻訳キー必須化で壊れないこと。Task 6 で既存テストを維持する。
4. **旧 IPC snapshot / event** — `version` や `pending_sends` などを欠く JSON は decode エラーになり、現行 daemon / UI の完全な JSON は引き続き decode できること。Task 2 の必須フィールドテストで固定する。
5. **旧サムネイルファイル名** — `.webp`、旧 `<id>` キー、旧 `q/s` キー、空 suffix の一時名を現行キャッシュとして数えないこと。現行 `<id>-s<width>x<height>.bc7` は prune / clear の対象に残ること。Task 5 で固定する。

---

### Task 1: DB schema v1 と段階的マイグレーションランナー

**Files:**
- Modify: `crates/core/src/db/schema.rs`
- Modify: `crates/core/src/db/mod.rs`
- Test: `crates/core/src/db/tests.rs`

**Interfaces:**
- Consumes: 既存の `Db::open_with_profile` / `Db::open_read_only_with_profile`。
- Produces:
  - `pub(crate) const SCHEMA_VERSION: i64 = 1;`
  - `pub(crate) const SCHEMA_BASE_VERSION: i64 = 1;`
  - `pub(crate) const SCHEMA_V1: &str`
  - `pub(crate) struct Migration { pub version: i64, pub sql: &'static str }`
  - `pub(crate) const MIGRATIONS: &[Migration] = &[];`
  - `pub(super) fn initialize(conn: &Connection) -> Result<()>`
  - `pub(crate) fn apply_migrations(tx: &Transaction<'_>, from: i64, migrations: &[Migration], target: i64) -> Result<i64>`

- [x] **Step 1: 失敗するテストを書く**

`crates/core/src/db/tests.rs` の `fresh_database_is_initialized_to_v21` を `fresh_database_is_initialized_to_v1` に改名し、次を検証する。

```rust
let version: i64 = db
    .connection()
    .pragma_query_value(None, "user_version", |row| row.get(0))
    .unwrap();
assert_eq!(version, 1);

let migrations_table: i64 = db
    .connection()
    .query_row(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |row| row.get(0),
    )
    .unwrap();
assert_eq!(migrations_table, 1);

let applied: Vec<i64> = db
    .connection()
    .prepare("SELECT version FROM schema_migrations ORDER BY version")
    .unwrap()
    .query_map([], |row| row.get(0))
    .unwrap()
    .collect::<rusqlite::Result<Vec<_>>>()
    .unwrap();
assert_eq!(applied, vec![1]);
```

`existing_v21_database_opens_without_reinitializing` は `existing_v1_database_opens_without_reinitializing` に改名し、`marker = keep` の検証を維持する。

`v20_database_is_rejected_without_modification` と `v18_database_is_rejected_without_modification` は、v1 より大きい版として拒否されることを検証する。期待メッセージを次に変える。

```rust
assert!(message.contains("newer than this build supports"), "{message}");
assert!(message.contains("update VRC PhotoN before opening it"), "{message}");
assert!(!message.contains("cannot be migrated"), "{message}");
```

`v18_database_is_rejected_by_open_read_only_without_modification` はそのまま維持する。`v18_wal_database_is_rejected_without_modification` は `message.contains("18")` の検証を維持する（v1 より大きいため拒否される）。

次のテストを追加する。

```rust
#[test]
fn migration_runner_applies_steps_one_version_at_a_time() {
    let db = Db::open_in_memory().unwrap();
    let tx = Transaction::new_unchecked(db.connection(), TransactionBehavior::Immediate).unwrap();
    let applied = schema::apply_migrations(
        &tx,
        SCHEMA_BASE_VERSION,
        &[
            schema::Migration {
                version: 2,
                sql: "CREATE TABLE migration_probe(value TEXT); \
                      INSERT INTO migration_probe(value) VALUES ('two');",
            },
            schema::Migration {
                version: 3,
                sql: "ALTER TABLE migration_probe ADD COLUMN extra TEXT;",
            },
        ],
        3,
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(applied, 3);
    let version: i64 = db
        .connection()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3);
    let applied_versions: Vec<i64> = db
        .connection()
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(applied_versions, vec![1, 2, 3]);
}

#[test]
fn migration_runner_rejects_a_gap() {
    let db = Db::open_in_memory().unwrap();
    let tx = Transaction::new_unchecked(db.connection(), TransactionBehavior::Immediate).unwrap();
    let error = schema::apply_migrations(
        &tx,
        SCHEMA_BASE_VERSION,
        &[schema::Migration {
            version: 3,
            sql: "CREATE TABLE skipped(value TEXT);",
        }],
        3,
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not follow version 1"));
}

#[test]
fn migration_chain_matches_schema_version() {
    let mut expected = SCHEMA_BASE_VERSION;
    for migration in schema::MIGRATIONS {
        assert_eq!(migration.version, expected + 1);
        expected = migration.version;
    }
    assert_eq!(expected, SCHEMA_VERSION);
}
```

`Db::open_with_profile` が旧版を移行する経路は、本番 `MIGRATIONS` が空のため直接テストできない。ランナー本体のテストで固定する。`crates/core/src/db/tests.rs` で `Transaction` / `TransactionBehavior` が未 import なら、先頭に `use rusqlite::{Transaction, TransactionBehavior};` を追加する。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core db::tests::fresh_database_is_initialized_to_v1 db::tests::migration_runner_applies_steps_one_version_at_a_time`
Expected: コンパイルエラー（`SCHEMA_BASE_VERSION`、`Migration`、`apply_migrations` が未定義）または v21 期待値の不一致。

- [x] **Step 3: `schema.rs` を実装する**

先頭を次の形にする。

```rust
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::error::Result;

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
-- 既存の CREATE TABLE / INSERT / CREATE INDEX / CREATE TRIGGER 群をそのまま置く
"##;

pub(crate) struct Migration {
    pub version: i64,
    pub sql: &'static str,
}

/// Ordered one-version-at-a-time migrations above [`SCHEMA_BASE_VERSION`].
/// 1.0.0 時点では 0 件。
pub(crate) const MIGRATIONS: &[Migration] = &[];
```

`SCHEMA_LATEST` を `SCHEMA_V1` に改名し、冒頭コメントを「v21 の統合 schema」から「v1 の凍結スキーマ」に変える。`schema_migrations` は `SCHEMA_V1` の先頭に追加する。

`initialize` を次に置き換える。

```rust
pub(super) fn initialize(conn: &Connection) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == SCHEMA_VERSION {
        tx.commit()?;
        return Ok(());
    }
    if version > SCHEMA_VERSION {
        return Err(crate::error::CoreError::Invalid(format!(
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
            return Err(crate::error::CoreError::Invalid(
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
            return Err(crate::error::CoreError::Invalid(format!(
                "schema migration {} does not follow version {version}",
                migration.version
            )));
        }
        if migration.version > target {
            return Err(crate::error::CoreError::Invalid(format!(
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
        return Err(crate::error::CoreError::Invalid(format!(
            "database schema version {version} cannot be migrated to {target}"
        )));
    }
    Ok(version)
}
```

- [x] **Step 4: `Db::open` 系を新しい判定に合わせる**

`crates/core/src/db/mod.rs` の `open_with_profile` を次の形にする。

- 読み取り専用 probe と read-write open の両方で、`version > SCHEMA_VERSION` なら `CoreError::Invalid`（「newer than this build supports ... update VRC PhotoN」）を返す。
- `version == 0 && has_user_tables` は既存のエラーを維持する。
- `Self::configure` の後、`if version != SCHEMA_VERSION { schema::initialize(&conn)?; }` を呼ぶ。v0 の新規 DB と将来の旧版移行の両方がここを通る。
- `open_read_only_with_profile` は `SCHEMA_VERSION` と一致する場合だけ許可し、それ以外は移行せずエラーにする。エラーメッセージは「requires migration; open it with VRC PhotoN before using the read-only index」を含める。
- 既存の `check_schema_version` は削除し、`check_open_version`（新しすぎる DB だけ拒否）と `check_read_only_version`（完全一致だけ許可）に分ける。
- `version < 0` は「not supported」として拒否する。

- [x] **Step 5: テストを実行する**

Run: `cargo test -p vrc-photon-core db::tests`
Expected: PASS。`fresh_database_is_initialized_to_v1`、`existing_v1_database_opens_without_reinitializing`、`migration_runner_*`、`open_rejects_a_database_from_a_newer_schema_version` が通る。

- [x] **Step 6: コミット**

```bash
git add crates/core/src/db/schema.rs crates/core/src/db/mod.rs crates/core/src/db/tests.rs
git commit -m "feat(db): reset the schema to v1 with a migration runner (#688)"
```

---

### Task 2: IPC protocol v1 と旧 daemon 互換の削除

**Files:**
- Modify: `crates/ipc/src/lib.rs`
- Modify: `crates/ipc/src/protocol.rs`
- Modify: `crates/ui/src/app/daemon.rs`

**Interfaces:**
- Consumes: 既存の `StatusSnapshot` / `Event`。
- Produces: `PROTOCOL_VERSION = 1`、`status_snapshot_is_newer(version, last_applied) -> version > last_applied`。

- [x] **Step 1: 失敗するテストを書く**

`crates/ipc/src/lib.rs` のテストを `protocol_version_is_one` に改名し、`assert_eq!(PROTOCOL_VERSION, 1);` にする。

`crates/ipc/src/protocol.rs` の `status_snapshot_without_a_version_decodes_as_zero` と `status_snapshot_without_ai_fields_decodes` を次に置き換える。

```rust
#[test]
fn status_snapshot_requires_every_protocol_field() {
    for field in [
        "pending_sends",
        "version",
        "pending_tagging_jobs",
        "tagging_ready",
    ] {
        let mut value = serde_json::to_value(StatusSnapshot::default()).unwrap();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<StatusSnapshot>(value).is_err(),
            "missing {field} must be rejected"
        );
    }
}
```

`crates/ui/src/app/daemon.rs` のテスト `status_snapshot_versions_drop_stale_publishes` を次に合わせる。

```rust
assert!(status_snapshot_is_newer(3, 2));
assert!(!status_snapshot_is_newer(2, 2));
assert!(!status_snapshot_is_newer(1, 2));
assert!(!status_snapshot_is_newer(0, 0));
assert!(!status_snapshot_is_newer(0, 2));
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ipc protocol_version_is_one status_snapshot_requires_every_protocol_field`
Expected: `PROTOCOL_VERSION` が 8 のため失敗。

- [x] **Step 3: 実装する**

- `PROTOCOL_VERSION` を `1` にする。
- `StatusSnapshot` の `pending_sends` / `version` / `pending_tagging_jobs` / `tagging_ready` から `#[serde(default)]` を削除する。コメントから「old daemon」「Defaults to ... so ... still decodes」を外し、現行プロトコルの必須フィールドとして説明する。
- `Event::ScanStarted.target` と `Event::ScanFinished.auto_enqueued` / `target` から `#[serde(default)]` を削除する。
- `status_snapshot_is_newer` を `version > last_applied` にし、コメントから「Version 0 comes from a daemon without versioning」を削除する。`last_status_version` の初期値 0 はそのまま（最初の version 1 以上を受理する）。

- [x] **Step 4: テストを実行する**

Run: `cargo test -p vrc-photon-ipc && cargo test -p vrc-photon-ui app::daemon::tests`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/ipc/src/lib.rs crates/ipc/src/protocol.rs crates/ui/src/app/daemon.rs
git commit -m "feat(ipc): reset the protocol to v1 and drop old-daemon defaults (#688)"
```

---

### Task 3: デモデータ v1 と `previews` 受理の削除

**Files:**
- Modify: `crates/ui/src/app/demo.rs`

**Interfaces:**
- Consumes: 既存の `DEMO_DATA_VERSION` / `DEMO_ROOT_ENTRIES`。
- Produces: `DEMO_DATA_VERSION = 1`、許可エントリ 8 件。

- [x] **Step 1: テストを修正する**

`prepare_demo_at_rebuilds_a_stale_marker` の `std::fs::write(root.join(DEMO_VERSION_FILE), "1")` を `"0"` に変える（v1 では `"1"` が現行のため stale にならない）。

`prepare_demo_at_reseeds_when_the_marker_version_changes` の `(DEMO_DATA_VERSION - 1).to_string()` を `DEMO_DATA_VERSION.saturating_sub(1).to_string()` に変える。

`demo_version_marker_detects_stale_data` に `"0"` が stale である検証を追加する。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui app::demo::tests::prepare_demo_at_rebuilds_a_stale_marker`
Expected: 現行 version 5 では `"0"` が stale なので通ってしまう可能性がある。続けて `DEMO_DATA_VERSION` を 1 にした後、`"1"` のままだと失敗することを確認する。

- [x] **Step 3: 実装する**

- `DEMO_DATA_VERSION` を `1` にする。
- `DEMO_ROOT_ENTRIES` から `"previews"` を削除し、型を `[&str; 8]` にする。コメントを「デモが作るエントリだけを許可する」に変える。
- `demo_data_is_current` は `.parse::<u32>()` のまま。負値や非数値は現行どおり stale。

- [x] **Step 4: テストを実行する**

Run: `cargo test -p vrc-photon-ui app::demo::tests`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/ui/src/app/demo.rs
git commit -m "feat(demo): reset the demo fixture version to 1 (#688)"
```

---

### Task 4: `AppSettings` の完全な厳格化

**Files:**
- Modify: `crates/core/src/settings.rs`
- Modify: `crates/core/src/model.rs`
- Test: `crates/core/src/settings.rs`（`mod tests`）、`crates/core/src/model.rs`（`mod tests`）

**Interfaces:**
- Consumes: 既存の `AppSettings::load` / `save`、`AutoBehavior`、`CompressionProfile`、`ResizeSettings`。
- Produces: 完全な 1.0.0 設定 JSON だけを受理する `AppSettings`。`load` は移行リライトを行わない。

- [x] **Step 1: 失敗するテストを書く**

`crates/core/src/settings.rs` の `mod tests` に追加する。

```rust
#[test]
fn partial_settings_are_rejected() {
    assert!(serde_json::from_str::<AppSettings>(r#"{"language":"ja"}"#).is_err());
}

#[test]
fn settings_missing_update_fields_are_rejected() {
    for field in [
        "setup_completed",
        "auto_compress_keep_original",
        "auto_compression",
        "clipboard_compression",
        "discord_compression",
        "ui_sidebar_collapsed",
        "auto_compress",
    ] {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<AppSettings>(value).is_err(),
            "missing {field} must be rejected"
        );
    }
}

#[test]
fn legacy_compression_alias_is_rejected() {
    let mut value = serde_json::to_value(AppSettings::default()).unwrap();
    let object = value.as_object_mut().unwrap();
    let keep = object.remove("auto_compress_keep_original").unwrap();
    object.insert("compression_keep_original".to_string(), keep);
    assert!(serde_json::from_value::<AppSettings>(value).is_err());
}
```

`crates/core/src/model.rs` のテスト `legacy_profile_json_defaults_to_downscale_only` を `resize_mode_is_required_in_profile_json` に置き換える。

```rust
#[test]
fn resize_mode_is_required_in_profile_json() {
    let json = serde_json::json!({
        "id": "profile",
        "name": "Profile",
        "format": "webp",
        "quality": 82,
        "lossless": false,
        "resize_enabled": true,
        "width": 1920,
        "height": 1080,
        "maintain_aspect": true,
        "output_dir": null,
        "suffix": "_webp",
        "encode_speed": 6
    });
    assert!(serde_json::from_value::<CompressionProfile>(json).is_err());
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core settings::tests::partial_settings_are_rejected model::tests::resize_mode_is_required_in_profile_json`
Expected: FAIL。現行コードは `#[serde(default)]` と field default で欠損を補うため、`partial_settings_are_rejected` は `is_err()` に反して Ok を返し、`resize_mode_is_required_in_profile_json` も Ok を返す。

- [x] **Step 3: 属性と旧移行コードを削除する**

`crates/core/src/settings.rs`:

- `AutoBehavior` の `#[serde(default)]` を削除する。
- `AppSettings` の `#[serde(default)]` を削除する。
- `AppSettings` の全フィールドから `serde(default)` / `serde(default = "...")` を削除する。
  - `#[serde(default, deserialize_with = "...")]` は `#[serde(deserialize_with = "...")]` にする（`ui_card_size`、`ui_grid_gap`、`ui_card_radius`、`ui_photos_view_mode`、`ai_tagging_max_edge`）。
  - `auto_compress_keep_original` の `alias = "compression_keep_original"` を削除する。
- `migrate_legacy_auto_settings` 関数を削除する。
- `AppSettings::load` から `let before = value.clone();`、`migrate_legacy_auto_settings(...)`、`if value != before { ... }` の再保存ブロックを削除し、`serde_json::from_value::<Self>(value)` の結果に normalize を適用して返す。
- `default_true` / `default_auto_compression` / `default_clipboard_compression` / `default_discord_compression` / `default_max_fps` / `default_thumbnail_size` / `default_auto_update_check` / `default_ui_theme_*` / `default_discord_destination` / `default_ai_backend` / `default_ai_tagging_max_edge` / `default_ai_idle_timeout` / `default_ui_grid_gap` / `default_ui_card_radius` は `Default` 実装で使われているため残す。`default_true` が `Default` 以外で未使用になった場合は削除する。

`crates/core/src/model.rs`:

- `ResizeSettings` の `resize_mode` から `#[serde(default)]` を削除する。
- `CompressionProfile` の `resize_mode` から `#[serde(default)]` を削除する。

`crates/core/src/settings.rs` の `mod tests` から旧互換テストを削除する。対象:

- `legacy_settings_without_feature_compression_use_defaults`
- `missing_ribbon_collapsed_defaults_to_true_and_keeps_other_settings`
- `old_settings_default_to_keep_originals_and_immediate_auto_compress`
- `legacy_compression_policy_loads_as_the_automatic_setting`
- `missing_card_display_fields_default_to_visible_medium`
- `missing_grid_display_fields_use_defaults`
- `incomplete_settings_json_uses_field_defaults`
- `missing_thumbnail_settings_use_the_new_defaults`
- `missing_theme_fields_default_to_charcoal_and_sky`
- `settings_deserialize_without_ui_fields`
- `old_app_settings_json_keeps_discord_defaults`
- `ai_settings_default_off_and_survive_old_json`
- `old_settings_json_defaults_import_structure_off`
- `legacy_settings_without_the_setup_flag_count_as_completed`
- `invalid_migrated_settings_are_not_persisted`
- `loading_legacy_settings_then_saving_writes_only_common_keys`
- `loading_legacy_settings_persists_the_new_shape`
- `old_settings_default_queue_pause_to_false`

部分 JSON を使っているが互換目的ではないテストは、`serde_json::to_value(AppSettings::default())` から必要なフィールドだけ変更する形に書き換える:

- `unknown_thumbnail_quality_key_is_ignored`
- `unknown_theme_id_loads_verbatim`
- `world_name_resolution_defaults_off_and_roundtrips`（`old` の partial JSON 部分を削除し、`AppSettings::default()` と roundtrip だけにする）

- [x] **Step 4: テストを実行する**

Run: `cargo test -p vrc-photon-core settings::tests model::tests`
Expected: PASS。`malformed_settings_json_returns_an_error_and_backs_up_the_raw_row` と `wrong_typed_settings_value_is_an_error_not_a_silent_reset` も通る。

- [x] **Step 5: コミット**

```bash
git add crates/core/src/settings.rs crates/core/src/model.rs
git commit -m "feat(settings): require the complete 1.0.0 settings shape (#688)"
```

---

### Task 5: 旧サムネイル互換の削除

**Files:**
- Modify: `crates/core/src/thumbnail.rs`
- Modify: `crates/daemon/src/lifecycle.rs`

**Interfaces:**
- Consumes: 既存の `thumbnail_cache_usage_in` / `prune_thumbnails_in` / `clear_thumbnail_cache_in` / `remove_thumbnail_files_in`。
- Produces: 現行 `<id>-s<width>x<height>.bc7` とその `.meta` / `.tmp-<pid>-<sequence>` だけをサムネイル成果物として扱う。`remove_legacy_webp_entries` は存在しない。

- [x] **Step 1: 失敗するテストを書く／既存テストを現行形式に直す**

`crates/core/src/thumbnail.rs` の `mod tests`:

- `legacy_webp_entries_are_removed_without_touching_other_files` を削除する。
- `thumbnail_cache_usage_counts_old_and_spec_keys` を削除する。
- `prune_thumbnails_*` / `clear_thumbnail_cache_*` / `prune_retries_*` のテストファイル名を、すべて現行形式に置き換える。
  - `1.webp` → `1-s384x216.bc7`
  - `2.webp` → `2-s384x216.bc7`
  - `3.webp` → `3-s384x216.bc7`
  - `4.webp` → `4-s384x216.bc7`
  - `keep.webp` → `999-s384x216.bc7`（テストの意図に合わせて有効な別 ID）
  - `3.webp.tmp` → `3-s384x216.bc7.tmp`
- `clear_thumbnail_cache_removes_old_new_and_temporary_files` を `clear_thumbnail_cache_removes_current_and_temporary_files` に改名し、次を使う。
  - `1-s384x216.bc7`、`1-s384x216.bc7.meta`
  - `2-s384x216.bc7`、`2-s384x216.bc7.meta`
  - `3-s384x216.bc7.tmp`、`3-s384x216.bc7.tmp-42-7`、`3-s384x216.bc7.meta.tmp-42-8`
  - 期待値は `removed_files = 7`、`removed_bytes = 9_030` のまま。
- `racing_thumbnail_publish_keeps_the_existing_file` の `thumb.webp` を `thumb.bc7` にする。

追加テスト:

```rust
#[test]
fn legacy_thumbnail_names_are_not_cache_entries() {
    let dir = tempdir().unwrap();
    for name in [
        "1.webp",
        "1.webp.meta",
        "1-s384x216.webp",
        "2-q80-s256.bc7",
        "3.bc7",
        "4-s384x216.bc7.tmp",
    ] {
        fs::write(dir.path().join(name), vec![0u8; 100]).unwrap();
    }
    let usage = thumbnail_cache_usage_in(dir.path()).unwrap();
    assert_eq!(usage.files, 0);
    assert_eq!(usage.bytes, 0);
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core thumbnail::tests::legacy_thumbnail_names_are_not_cache_entries`
Expected: 現行コードは `.webp` や旧キーを受理するため `files > 0` で FAIL。

- [x] **Step 3: 実装する**

`crates/core/src/thumbnail.rs`:

- `remove_legacy_webp_entries` を削除する。
- `thumbnail_artifact_stem` を `.bc7` だけに限定する。
  ```rust
  fn thumbnail_artifact_stem(name: &str) -> Option<&str> {
      let extension = ".bc7";
      if let Some(stem) = name.strip_suffix(extension) {
          return Some(stem);
      }
      if let Some(stem) = name.strip_suffix(&format!("{extension}.meta")) {
          return Some(stem);
      }
      if let Some((stem, suffix)) = name.split_once(&format!("{extension}.meta.tmp")) {
          return is_temp_suffix(suffix).then_some(stem);
      }
      if let Some((stem, suffix)) = name.split_once(&format!("{extension}.tmp")) {
          return is_temp_suffix(suffix).then_some(stem);
      }
      None
  }
  ```
- `is_temp_suffix` は `-<pid>-<sequence>` だけを受理し、空 suffix を受理しない。
- `is_thumbnail_stem` は `<id>-s<width>x<height>` だけを受理する。`<id>` 単独と `<id>-q<quality>-s<size>` の分岐を削除する。
- `crates/daemon/src/lifecycle.rs` の `start_workers` から `remove_legacy_webp_entries` の呼び出しを削除する。

- [x] **Step 4: テストを実行する**

Run: `cargo test -p vrc-photon-core thumbnail::tests && cargo test -p vrc-photon-daemon lifecycle`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/core/src/thumbnail.rs crates/daemon/src/lifecycle.rs
git commit -m "feat(thumbnail): drop legacy WebP cache name handling (#688)"
```

---

### Task 6: 取り込み・アップデーター・デーモンエラーの互換整理

**Files:**
- Modify: `crates/core/src/importer.rs`
- Modify: `crates/update/src/install.rs`
- Modify: `crates/update/src/helper.rs`
- Modify: `crates/ui/src/ui/daemon_error.rs`（コメントのみ）
- Test: 各ファイルの `mod tests`

**Interfaces:**
- Consumes: 既存の `copy_file` / `ApplyKind` / `LastResult` / `HealthMarker` / `daemon_error_message_in`。
- Produces: 旧一時名の削除なし、`app_image` alias なし、`LastResult.code` と `HealthMarker.database` は必須フィールド。`message_key: None` の表示フォールバックは維持。

- [x] **Step 1: 失敗するテストを書く**

`crates/core/src/importer.rs` の `stale_temp_file_does_not_block_the_import` を `legacy_temp_file_does_not_block_and_is_not_removed` に変える。

```rust
assert_eq!(result.imported.len(), 1);
assert!(result.issues.is_empty());
assert!(
    stale_temp.exists(),
    "an old .tmp file is not this build's temporary file"
);
assert_eq!(
    fs::metadata(&result.imported[0]).unwrap().len(),
    fs::metadata(&source).unwrap().len()
);
```

`crates/update/src/install.rs` の `apply_kind_accepts_the_legacy_app_image_alias` を `apply_kind_rejects_the_legacy_app_image_alias` に変える。

```rust
assert!(serde_json::from_str::<ApplyKind>("\"app_image\"").is_err());
```

`crates/update/src/helper.rs` の `last_result_accepts_a_missing_code` を `last_result_requires_a_code` に変える。

```rust
assert!(LastResult::read(&paths).is_none());
```

 `HealthMarker` の必須化テストを追加する。

```rust
#[test]
fn health_marker_requires_a_database_field() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    std::fs::write(
        paths.health_file("token"),
        br#"{"version":"1.0.0","mode":"normal"}"#,
    )
    .unwrap();
    assert!(read_health_marker(&paths, "token").is_err());
}
```

`crates/ui/src/ui/daemon_error.rs` の `missing_or_invalid_keys_fall_back_to_the_stored_message` は維持する。コメントを「技術メッセージ（`None`）と旧行は区別せず、保存済み `message` を表示する」に更新する。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-update last_result_requires_a_code health_marker_requires_a_database_field && cargo test -p vrc-photon-core importer::tests::legacy_temp_file_does_not_block_and_is_not_removed`
Expected: `LastResult` / `HealthMarker` が default で通ってしまい、importer は stale temp が削除されて FAIL。

- [x] **Step 3: 実装する**

- `crates/core/src/importer.rs`: `copy_file` 内の `clear_legacy_temp(&target);` と呼び出し先関数 `clear_legacy_temp` を削除する。
- `crates/update/src/install.rs`: `#[serde(rename = "appimage", alias = "app_image")]` を `#[serde(rename = "appimage")]` にする。
- `crates/update/src/helper.rs`:
  - `LastResult.code` の `#[serde(default)]` を削除し、コメントを「失敗理由のコード。1.0.0 以降は必須」に変える。
  - `HealthMarker.database` の `#[serde(default)]` を削除し、コメントを「1.0.0 以降は必須（DB なしは null）」に変える。
- `crates/ui/src/ui/daemon_error.rs`: 実装は変えず、コメントだけ更新する。

- [x] **Step 4: テストを実行する**

Run: `cargo test -p vrc-photon-core importer::tests && cargo test -p vrc-photon-update && cargo test -p vrc-photon-ui ui::daemon_error`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/core/src/importer.rs crates/update/src/install.rs crates/update/src/helper.rs crates/ui/src/ui/daemon_error.rs
git commit -m "feat: remove remaining 0.x updater and importer compatibility (#688)"
```

---

### Task 7: 旧 `PhotoFilter` 検索経路の削除

**Files:**
- Modify: `crates/core/src/model.rs`
- Modify: `crates/core/src/db/mod.rs`
- Modify: `crates/core/src/db/photos.rs`
- Modify: `crates/core/src/db/tests.rs`
- Modify: `crates/ui/src/app/demo.rs`

**Interfaces:**
- Consumes: `PhotoTextFilter`、`search_photos_text(filter, limit, offset)`、`count_photos_text(filter)`、`photo_index_text_limited(filter, limit)`。
- Produces: `PhotoFilter`、`Db::search_photos`、`Db::count_search`、`build_photo_filter` は存在しない。

- [x] **Step 1: テストを新しい検索 API に置き換える**

`crates/core/src/db/tests.rs`:

- `photo_list_and_search_limits_agree`:
  - `search_photos` の closure を `search_photos_text(&PhotoTextFilter::default(), limit, 0)` に置き換える。
  - 比較ループの `search_photos` を `search_photos_text(&PhotoTextFilter::default(), limit, 0)` に置き換え、`Photo` の `id` を比較する。
- `photo_filter_ai_conditions_apply_to_search_photos_and_count` を `text_filter_ai_conditions_apply_to_search_and_count` に改名し、`PhotoTextFilter` + `count_photos_text` / `search_photos_text` に置き換える。
- `deleting_photo_is_hidden_from_index_search_and_counts` の `search_photos(&PhotoFilter::default())` を `search_photos_text(&PhotoTextFilter::default(), 10, 0)` に置き換える。
- その他の `PhotoFilter` 使用箇所（world / AI 条件）を `PhotoTextFilter` の `world` / `ai` フィールドに置き換える。`count_search` は `count_photos_text` にする。

`crates/ui/src/app/demo.rs` のテスト:

- `PhotoFilter` / `PhotoSort` の import を `PhotoTextFilter` / `PhotoSort` に変える。
- `count_search(&PhotoFilter { from_utc, to_utc, limit: 500, .. })` を `count_photos_text(&PhotoTextFilter { from_utc, to_utc, ..Default::default() })` に変える。
- `count_search(&PhotoFilter { favorite: Some(true), .. })` を `count_photos_text(&PhotoTextFilter { favorite: true, ..Default::default() })` に変える。
- `search_photos(&PhotoFilter { limit: 500, .. })` を `search_photos_text(&PhotoTextFilter::default(), 500, 0)` に変える。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core db::tests::photo_list_and_search_limits_agree`
Expected: `PhotoFilter` 削除前は通り、削除後にコンパイルエラーになる。Step 3 と同時に確認する。

- [x] **Step 3: 実装する**

- `crates/core/src/model.rs` から `PhotoFilter` struct を削除する。
- `crates/core/src/db/mod.rs` の import から `PhotoFilter` を削除する。
- `crates/core/src/db/tests.rs` の `use crate::model::{...}` から `PhotoFilter` を削除する。
- `crates/core/src/db/photos.rs`:
  - `search_photos` を削除する。
  - `count_search` を削除する。
  - `build_photo_filter` を削除する。
  - `push_ai_filter` は `build_text_filter` が使うため残す。

- [x] **Step 4: テストを実行する**

Run: `cargo test -p vrc-photon-core db::tests && cargo test -p vrc-photon-ui app::demo::tests`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/core/src/model.rs crates/core/src/db/mod.rs crates/core/src/db/photos.rs crates/core/src/db/tests.rs crates/ui/src/app/demo.rs
git commit -m "feat(search): remove the legacy PhotoFilter search path (#688)"
```

---

### Task 8: AI プラグイン fixture とアプリ版 1.0.0

**Files:**
- Modify: `crates/ai-host/src/test_support.rs`
- Modify: `crates/ai-host/src/manifest.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`（`cargo check` で更新）
- Modify: `Packager.toml`
- Modify: `packaging/licenses/THIRD-PARTY-NOTICES.md`
- Modify: `packaging/ai-plugin.json`

**Interfaces:**
- Consumes: 既存の `TEST_HOST_VERSION_MAX` / manifest fixture JSON / workspace version。
- Produces: `TEST_HOST_VERSION_MAX = "1.99.99"`、workspace version `1.0.0`、`packaging/ai-plugin.json` の `host_version_min = "1.0.0"`。

- [x] **Step 1: fixture を更新する**

- `crates/ai-host/src/test_support.rs`: `TEST_HOST_VERSION_MAX` を `"1.99.99"` にする。
- `crates/ai-host/src/manifest.rs` の `parse_manifest_accepts_absent_optional_fields` と `parse_manifest_rejects_unsupported_formats_and_malformed_json` 内の `"host_version_max": "0.99.99"` を `"1.99.99"` に置き換える。
- `packaging/ai-plugin.json` の `host_version_min` を `"1.0.0"` にする。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ai-host manifest`
Expected: 0.99.99 のままだと 1.0.0 host の検証テストがある場合に失敗する。少なくとも Step 3 の bump 後に 1.0.0 が範囲外にならないことを確認する。

- [x] **Step 3: アプリ版を bump する**

- `Cargo.toml` の `[workspace.package] version` を `"1.0.0"` にする。
- `Packager.toml` の `version` を `"1.0.0"` にする。
- `packaging/licenses/THIRD-PARTY-NOTICES.md` の workspace crate 一覧行の `0.14.5` をすべて `1.0.0` に置き換える（`vrc-photon-ai-host` から `vrc-photon-update` までの行）。
- `cargo check --workspace` を実行して `Cargo.lock` の workspace package version を更新する。`git diff Cargo.lock` で workspace crate の version 以外が変わっていないことを確認する。

- [x] **Step 4: テストを実行する**

Run: `cargo test -p vrc-photon-ai-host && cargo test -p vrc-photon-update`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add Cargo.toml Cargo.lock Packager.toml packaging/licenses/THIRD-PARTY-NOTICES.md packaging/ai-plugin.json crates/ai-host/src/test_support.rs crates/ai-host/src/manifest.rs
git commit -m "chore: bump the app to 1.0.0 and update AI host fixtures (#688)"
```

---

### Task 9: ドキュメントと docs drift チェック

**Files:**
- Modify: `docs/data-model.md`
- Modify: `docs/database-initialization.md`
- Modify: `docs/architecture.md`
- Modify: `docs/development.md`
- Modify: `docs/features.md`
- Modify: `docs/troubleshooting.md`
- Modify: `docs/release.md`
- Modify: `docs/ui.md`
- Modify: `scripts/check-docs-drift.sh`

**Interfaces:**
- Consumes: 実装後の `PROTOCOL_VERSION = 1` / `SCHEMA_VERSION = 1` / 1.0.0。
- Produces: `./scripts/check-docs-drift.sh` が PASS する docs。

- [x] **Step 1: `docs/data-model.md` を更新する**

- 冒頭の schema 節を次に合わせる。
  - `現在のバージョンは **1**`
  - `user_version` が `2` 以上（`user_version` が 2 以上）の DB は `Db::open` / `Db::open_read_only` で拒否する。このビルドが理解できる最大値は `最大値（1）` であることも明記する。
  - 新規 DB は `SCHEMA_V1` を 1 トランザクションで実行して v1 にする。
  - 1.0.0 未満の DB は存在しない前提。旧 0.x の `user_version`（18/20/21 など）は v1 より大きいため「新しすぎる DB」として拒否される。
- `schema_migrations` の節を追加する。
  - `version INTEGER PRIMARY KEY`
  - `applied_at TEXT NOT NULL`（RFC3339）
  - 新規 DB は v1、移行時は各版を記録する。`user_version` が起動時の判定に使う正本。
- 6 章を書き換える。
  - `SCHEMA_V1` は凍結。将来の変更は `MIGRATIONS` に版ごとの SQL を追加し、`SCHEMA_VERSION` を上げる。
  - 新規 DB も v1 作成後に `MIGRATIONS` を適用する。
  - 移行は `IMMEDIATE` トランザクションで、ロック取得後に `user_version` を再確認する。失敗時はロールバック。
  - `open_read_only` は移行せず、v1 のみ受け付ける。
- 履歴表現を整理する。
  - `probe_json` の `v13 の拡張` / `v13 で追加した` を削除し、現行の意味だけ残す。
  - `schema v10 で message_key ...` を削除し、「キーがある行は UI が現在の言語で描画し、キーがない技術メッセージは `message` を表示する」に書き換える。

- [x] **Step 2: `docs/database-initialization.md` を書き換える**

次を含める。

- 現在のビルドは v1 を扱う。新規 DB は `SCHEMA_V1` を `IMMEDIATE` トランザクションで作成し、`user_version = 1`、`schema_migrations(1, applied_at)` を記録する。
- 既存 DB の表:
  - `| 1 | そのまま開く。 |`
  - `| 2 以上 | このビルドより新しい schema としてエラーを返す。VRC PhotoN を更新する。 |`
  - `| 0（テーブルあり） | 不明な初期化状態としてエラーを返す。ファイルは変更しない。 |`
- 1.0.0 未満の DB は存在しない前提。0.x の DB は自動移行しない。
- 読み取り専用接続は v1 のみ受け付け、移行しない。
- `SCHEMA_V1` は凍結。`MIGRATIONS` に 1 版ずつ追加し、新規 DB も v1 作成後に適用する。各ステップは同じ `IMMEDIATE` トランザクション内で実行し、ロック取得後に `user_version` を再確認する。

- [x] **Step 3: その他の docs を更新する**

- `docs/architecture.md`:
  - `PROTOCOL_VERSION = 8` → `PROTOCOL_VERSION = 1`（IPC の節）。
  - `src/db/schema.rs # SCHEMA_LATEST と SCHEMA_VERSION` → `SCHEMA_V1 / MIGRATIONS / SCHEMA_VERSION`。
  - 起動手順の schema 検査を「新規 DB は `SCHEMA_V1` で初期化し、既存 DB は v1 のみ受け付ける。v2 以上は拒否する」に更新。
  - `スキーマ v12 で追加した world_name_lookups` の版参照を削除。
- `docs/development.md`:
  - §7 の `SCHEMA_LATEST` を `SCHEMA_V1 / MIGRATIONS / SCHEMA_VERSION` に更新。
  - §9 を「`MIGRATIONS` に次の版を追加し、`SCHEMA_VERSION` を上げる。`SCHEMA_V1` は凍結する。新規 DB の初期化テストと直前版からの移行テストを追加する。docs と drift check を同じ PR で更新する」に書き換える。
- `docs/features.md`: `PROTOCOL_VERSION = 8` → `PROTOCOL_VERSION = 1`。
- `docs/troubleshooting.md`:
  - `schema v21 のみ` → `schema v1 のみ`。
  - `v1〜v20` の記述を「1.0.0 未満の DB は存在しない前提。古い DB はバックアップして新規作成」に変える。
  - `v21 より新しい DB` → `v2 以上のように、対応版より新しい DB` に更新する。
- `docs/release.md`:
  - リリース前チェックの「v21のみ・v1〜v20」を「v1のみ・1.0.0未満は自動移行せず明示エラー」に更新。
  - ロールバック節の `DB は v21 のみ` を `DB は v1 のみ` に更新。
  - `host_version_min: 0.14.4` の例を `1.0.0` に更新。
- `docs/ui.md`: 開発者情報カードのバージョン例 `0.14.5` → `1.0.0`。

- [x] **Step 4: `scripts/check-docs-drift.sh` を schema 1 に対応させる**

`docs/database-initialization.md` の `| 1〜$((schema - 1)) |` チェックを、`schema > 1` のときだけ実行する。

```bash
if [ "${schema}" -gt 1 ]; then
    grep -Fq "| 1〜$((schema - 1)) |" docs/database-initialization.md \
        || fail "docs/database-initialization.md must reject 1〜$((schema - 1))"
fi
```

他のチェック（`現在のバージョンは **${schema}**`、`user_version が $((schema + 1)) 以上`、`最大値（${schema}）`、`| ${schema} | そのまま開く。`、`| $((schema + 1)) 以上 |`）はそのまま使える。

- [x] **Step 5: docs drift とテストを実行する**

Run: `./scripts/check-docs-drift.sh`
Expected: `docs drift check OK (PROTOCOL_VERSION=1, SCHEMA_VERSION=1, thumbnails=...)`

- [x] **Step 6: コミット**

```bash
git add docs/data-model.md docs/database-initialization.md docs/architecture.md docs/development.md docs/features.md docs/troubleshooting.md docs/release.md docs/ui.md scripts/check-docs-drift.sh
git commit -m "docs: describe the 1.0.0 schema and IPC versions (#688)"
```

---

### Task 10: 最終検証・計画チェック・PR 作成

**Files:**
- Modify: `docs/superpowers/plans/2026-11-19-issue-688-version-reset.md`（チェックボックス）

- [x] **Step 1: 全検証を実行する**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/check-docs-drift.sh
```

Expected: すべて PASS。

- [x] **Step 2: 残存する旧バージョン参照を確認する**

Run:

```bash
rg -n "SCHEMA_LATEST|version 21|v21|v20|v1〜v20|PROTOCOL_VERSION = 8|0\.99\.99|remove_legacy_webp_entries|clear_legacy_temp|compression_keep_original|app_image|DEMO_DATA_VERSION" \
  crates docs scripts packaging --glob '!docs/superpowers/**' --glob '!*.lock'
```

Expected: 実装上の旧参照が残っていないこと。`docs/superpowers/` の過去計画は対象外。`0.14.5` が `docs/ui.md` と `THIRD-PARTY-NOTICES` に残っていないことも確認する。

- [x] **Step 3: 計画のチェックボックスを `- [x]` に更新してコミットする**

```bash
git add docs/superpowers/plans/2026-11-19-issue-688-version-reset.md
git commit -m "docs: record the completed 1.0.0 version reset plan (#688)"
```

- [x] **Step 4: ブランチを push して PR を作成する**

```bash
git push -u origin feat/688-version-reset
gh pr create --title "feat: reset versions for 1.0.0 and remove 0.x compatibility (#688)" \
  --body "Closes #688. schema を v1、IPC を v1、デモを v1 にリセットし、0.x の互換コードとテストを削除します。アプリ版は 1.0.0 です。" \
  --label enhancement
```

- [x] **Step 5: CI を監視する**

`ci-watch` skill の手順で `CI` と `Release`（bundle）の結果を確認する。失敗時はログを解析して修正を push する。tag 作成・Release 公開は行わない。
