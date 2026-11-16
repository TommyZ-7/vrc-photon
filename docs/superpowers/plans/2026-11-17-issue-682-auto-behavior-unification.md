# 自動動作の設定を3機能で統一する (#682) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 自動圧縮・Discord 自動送信・AI 自動タグ付けの「自動動作」と「一括処理」を、共通の `AutoBehavior` 型と共通 UI に統一する。

**Architecture:** 設定 JSON に共通の `AutoBehavior { mode, scope, activated_at }` を3機能分持たせ、旧フィールドは読み込み時に移行する。各機能のマスタースイッチは残し、3値は自動動作の実行契機として分離する。デーモンは新規索引写真だけを自動投入し、既存写真は「一括処理」ボタンで `scope` に従って処理する。ジョブのテーブル・状態・ワーカーは変更しない。

**Tech Stack:** Rust 2024, gpui-kit 0.9.1, SQLite, rust-i18n, Plannotator

**Spec:** `docs/superpowers/specs/2026-11-17-issue-682-auto-behavior-unification-design.md`

**Mock:** `docs/ui-682-auto-behavior-unification-mockup.html`（承認まで実装しない）

**Status:** 計画中（2026-11-17）

## Global Constraints

- 設定の DB スキーマバージョンは上げない。設定は `app_settings` の JSON 1行のまま。
- 旧設定キー `auto_compress`（bool）/ `auto_compress_immediate` /
  `auto_compress_backlog` / `discord_auto_mode` / `discord_auto_since` は読み込み時に
  移行し、保存時に書き出さない。
- 自動圧縮のマスタースイッチ `auto_compress_enabled` を追加し、旧 `auto_compress`
  （bool）をそこへ移す。3値の `auto_compress.mode` とは分離する。
- `auto_compress_keep_original`、`auto_compression`、`discord_enabled`、
  `discord_destination`、`discord_consent_at`、`ai_tagging_enabled`、
  `ai_tagging_backend`、`ai_tagging_idle_timeout_sec` は変更しない。
- ジョブのテーブル、状態、ワーカー、再試行、待機ジョブの解放 API は統合しない。
- 自動投入は新規索引写真だけ。初回フルスキャンでは3機能とも自動投入しない。
- 対象範囲は「一括処理」ボタンにだけ適用する。自動投入には適用しない。
- 自動動作が `Off` でも、機能のマスタースイッチがオンなら一括処理を使える。
  一括処理が無効になるのは、マスタースイッチがオフのときだけ
  （Discord は Webhook 不正、AI はプラグイン未準備も追加条件）。
- Discord の一括送信は未送信写真だけを対象にし、件数付き確認ダイアログを出す。
- AI の `Immediate` はプラグイン準備完了時のみ積む。`Waiting` は未準備でも積む。
- AI カード内の「設定を開く」ボタンと案内文を削除する。ダッシュボードの AI カードは
  この Issue の対象外。
- カード内の表示順を「ヘッダー＋マスタースイッチ → 機能固有の設定 →
  自動動作 → 一括処理 → 状態・履歴」に統一する。
- セットアップページには「自動動作」だけを置き、「一括処理」は表示しない。
- docs とコードは同じ PR に混ぜない。Phase 0 は docs-only PR、Phase 1 は実装 PR。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- CI が green になるまで merge しない。CI は `ci-watch` の手順で監視する。
- 各 PR は最新の `main` から `git worktree` で切る。`main` への直接 push はしない。
- 計画ファイルのチェックボックスは、その作業の完了時に `- [x]` に更新する。

## Review Focus

1. 旧設定 JSON が欠損・部分的な場合でも、マスタースイッチ、モード、対象範囲が
   意図どおり移行され、保存後に旧キーが消える。
2. 初回フルスキャンでは3機能とも自動投入されず、取り込みの `ScanRoot`、監視、
   2回目以降のフルスキャンでは新規写真が自動投入される。
3. マスタースイッチがオンで自動動作が `Off` のとき、一括処理が使える。
   マスタースイッチがオフのときだけ一括処理が無効になる。
4. `NewOnly` の一括処理が `activated_at` と `photos.created_at` で正しく絞られ、
   `IncludeBacklog` と `activated_at = None` の境界が正しい。
5. Discord の一括送信が、送信済み・待機中・実行待ち・処理中・削除中の写真を
   除外し、失敗・取消は含める。0件ではダイアログを出さない。
6. AI の `Immediate` は未準備で積まず、`Waiting` は未準備でも `waiting` で積む。
7. 3カードで「ヘッダー → 機能固有 → 自動動作 → 一括処理 → 状態」の順が同じ。
   セットアップ3ページには一括処理がなく、自動動作だけがある。
8. AI カードに「設定を開く」ボタンと案内文がない。
9. ja / en の両カタログと `placeholders.rs` が更新され、i18n テストが通る。

---

## Phase 0: モック・設計・計画

**Files:**
- Create: `docs/ui-682-auto-behavior-unification-mockup.html`
- Create: `docs/superpowers/specs/2026-11-17-issue-682-auto-behavior-unification-design.md`
- Create: `docs/superpowers/plans/2026-11-17-issue-682-auto-behavior-unification.md`（このファイル）

- [ ] **Step 1: モックを作る**

`docs/ui-682-auto-behavior-unification-mockup.html` に次を載せる。

- 共通コントロールの拡大図（自動動作の3分割セグメント、一括処理の対象範囲2チップ＋
  実行ボタン、説明）。`mode = Off` でも一括処理が使えることを示す。
- 連携・自動の3カード（自動圧縮・Discord・AI）の変更後。表示順は
  「ヘッダー → 機能固有 → 自動動作 → 一括処理 → 状態・履歴」。
- 自動圧縮カードのマスタースイッチ。
- 初回セットアップの3ページ（自動圧縮・Discord・AI）の変更後。
  一括処理は表示せず、自動動作だけを置く。
- AI カードから「設定を開く」ボタンと案内文を削除した状態。
- Discord の件数付き確認ダイアログ。
- 判断項目と決定の一覧、移行の挙動変化、対象外。

- [ ] **Step 2: docs PR を作る**

`docs: 自動動作設定の統一モックと計画 (#682)`。ラベルは `enhancement`。
Plannotator で計画書・設計書・モックをレビューし、承認を得るまで Phase 1 は
開始しない。現行 docs (`docs/ui.md` / `docs/features.md` /
`docs/data-model.md` / `docs/testing.md`) の更新は、実装内容が確定する
Phase 6 で行う。

---

## Phase 1: 設定モデルと旧 JSON 移行

### Task 1: `AutoBehavior` と移行

**Files:**
- Modify: `crates/core/src/settings.rs`
- Test: `crates/core/src/settings.rs` の `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: なし。
- Produces:
  - `pub enum AutoMode { Off, Immediate, Waiting }`（`as_str`, `is_active`）
  - `pub enum AutoScope { NewOnly, IncludeBacklog }`
  - `pub struct AutoBehavior { pub mode, pub scope, pub activated_at: Option<i64> }`
  - `impl AutoBehavior { pub const fn off() -> Self; pub const fn immediate() -> Self; pub const fn waiting() -> Self; pub const fn with_scope(self, scope: AutoScope) -> Self; pub fn set_mode(&mut self, mode: AutoMode, now: i64); pub fn touch_activation(&mut self, now: i64); pub fn bulk_since(&self) -> Option<i64>; pub fn is_active(&self) -> bool; }`
  - `AppSettings { pub auto_compress_enabled: bool, pub auto_compress: AutoBehavior, pub discord_auto: AutoBehavior, pub ai_tagging_auto: AutoBehavior }`
  - `fn migrate_legacy_auto_settings(value: &mut serde_json::Value, now: i64)`

- [ ] **Step 1: 失敗する移行テストを書く**

`crates/core/src/settings.rs` のテストに追加する。

```rust
#[test]
fn legacy_compression_fields_migrate_to_common_behavior() {
    let mut value = serde_json::json!({
        "auto_compress": true,
        "auto_compress_immediate": false,
        "auto_compress_backlog": true,
    });
    migrate_legacy_auto_settings(&mut value, 1_700_000_000);
    assert_eq!(value["auto_compress_enabled"], true);
    assert_eq!(value["auto_compress"]["mode"], "waiting");
    assert_eq!(value["auto_compress"]["scope"], "include_backlog");
    assert_eq!(value["auto_compress"]["activated_at"], 1_700_000_000);
    assert!(value.get("auto_compress_immediate").is_none());
    assert!(value.get("auto_compress_backlog").is_none());
}

#[test]
fn legacy_disabled_compression_migrates_to_a_disabled_master_switch() {
    let mut value = serde_json::json!({
        "auto_compress": false,
        "auto_compress_immediate": true,
    });
    migrate_legacy_auto_settings(&mut value, 1_700_000_000);
    assert_eq!(value["auto_compress_enabled"], false);
    assert_eq!(value["auto_compress"]["mode"], "immediate");
    assert_eq!(value["auto_compress"]["activated_at"], serde_json::Value::Null);
}

#[test]
fn legacy_discord_fields_migrate_to_common_behavior() {
    let mut value = serde_json::json!({
        "discord_auto_mode": "immediate",
        "discord_auto_since": 42,
    });
    migrate_legacy_auto_settings(&mut value, 1_700_000_000);
    assert_eq!(value["discord_auto"]["mode"], "immediate");
    assert_eq!(value["discord_auto"]["scope"], "new_only");
    assert_eq!(value["discord_auto"]["activated_at"], 42);
    assert!(value.get("discord_auto_mode").is_none());
    assert!(value.get("discord_auto_since").is_none());
}

#[test]
fn legacy_ai_auto_boolean_migrates_to_mode() {
    let mut value = serde_json::json!({
        "ai_tagging_enabled": true,
        "ai_tagging_auto": true,
    });
    migrate_legacy_auto_settings(&mut value, 1_700_000_000);
    assert_eq!(value["ai_tagging_auto"]["mode"], "immediate");
    assert_eq!(value["ai_tagging_auto"]["scope"], "new_only");
    assert_eq!(value["ai_tagging_auto"]["activated_at"], 1_700_000_000);
}

#[test]
fn auto_behavior_bulk_since_uses_activation_only_for_new_only() {
    let mut behavior = AutoBehavior::default();
    assert_eq!(behavior.bulk_since(), None);
    behavior.mode = AutoMode::Waiting;
    behavior.activated_at = Some(42);
    behavior.scope = AutoScope::NewOnly;
    assert_eq!(behavior.bulk_since(), Some(42));
    behavior.scope = AutoScope::IncludeBacklog;
    assert_eq!(behavior.bulk_since(), None);
}

#[test]
fn set_mode_sets_activation_only_when_leaving_off() {
    let mut behavior = AutoBehavior::default();
    behavior.set_mode(AutoMode::Waiting, 100);
    assert_eq!(behavior.activated_at, Some(100));
    behavior.set_mode(AutoMode::Immediate, 200);
    assert_eq!(behavior.activated_at, Some(100));
    behavior.set_mode(AutoMode::Off, 300);
    assert_eq!(behavior.activated_at, Some(100));
    behavior.set_mode(AutoMode::Immediate, 400);
    assert_eq!(behavior.activated_at, Some(400));
}

#[test]
fn loading_legacy_settings_then_saving_writes_only_common_keys() {
    let db = crate::db::Db::open_in_memory().unwrap();
    db.set_setting(
        SETTINGS_KEY,
        r#"{"language":"ja","auto_compress":true,"auto_compress_immediate":false,
            "auto_compress_backlog":true,"discord_auto_mode":"waiting",
            "discord_auto_since":42,"ai_tagging_auto":true}"#,
    )
    .unwrap();
    let settings = AppSettings::load(&db).unwrap();
    assert!(settings.auto_compress_enabled);
    assert_eq!(settings.auto_compress.mode, AutoMode::Waiting);
    assert_eq!(settings.auto_compress.scope, AutoScope::IncludeBacklog);
    assert_eq!(settings.discord_auto.mode, AutoMode::Waiting);
    assert_eq!(settings.discord_auto.activated_at, Some(42));
    assert_eq!(settings.ai_tagging_auto.mode, AutoMode::Immediate);
    settings.save(&db).unwrap();
    let raw = db.get_setting(SETTINGS_KEY).unwrap().unwrap();
    for key in [
        "auto_compress_immediate",
        "auto_compress_backlog",
        "discord_auto_mode",
        "discord_auto_since",
    ] {
        assert!(!raw.contains(&format!("\"{key}\"")));
    }
    assert!(raw.contains("\"auto_compress\""));
    assert!(raw.contains("\"discord_auto\""));
    assert!(raw.contains("\"ai_tagging_auto\""));
}

#[test]
fn fresh_settings_start_the_compression_activation_at_now() {
    let db = crate::db::Db::open_in_memory().unwrap();
    let before = chrono::Utc::now().timestamp();
    let settings = AppSettings::load(&db).unwrap();
    let after = chrono::Utc::now().timestamp();
    assert!(settings.auto_compress_enabled);
    let activated = settings.auto_compress.activated_at.unwrap();
    assert!(activated >= before && activated <= after);
}

#[test]
fn loading_legacy_settings_persists_the_new_shape() {
    let db = crate::db::Db::open_in_memory().unwrap();
    db.set_setting(
        SETTINGS_KEY,
        r#"{"language":"ja","auto_compress":true,"auto_compress_immediate":true,
            "auto_compress_backlog":false,"ai_tagging_auto":true}"#,
    )
    .unwrap();
    let _ = AppSettings::load(&db).unwrap();
    let raw = db.get_setting(SETTINGS_KEY).unwrap().unwrap();
    assert!(!raw.contains("auto_compress_immediate"));
    assert!(!raw.contains("auto_compress_backlog"));
    assert!(!raw.contains("discord_auto_mode"));
    assert!(!raw.contains("discord_auto_since"));
    assert!(raw.contains("\"auto_compress\""));
    assert!(raw.contains("\"ai_tagging_auto\""));
}
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-core settings::tests::legacy_compression_fields_migrate_to_common_behavior`
Expected: FAIL（`migrate_legacy_auto_settings` と `AutoBehavior` が未定義）

- [ ] **Step 3: 型と移行を実装する**

`crates/core/src/settings.rs` に `AutoMode`, `AutoScope`, `AutoBehavior`、
`auto_compress_enabled: bool` を追加し、`DiscordAutoMode` と旧フィールドを削除する。
旧 `auto_compress`（bool）は `auto_compress_enabled` に移し、旧
`auto_compress_immediate` は `auto_compress.mode`（`true` → `Immediate`、
`false` → `Waiting`）に移す。旧 `auto_compress = false` は
`auto_compress_enabled = false` とし、`mode` は既定の `Immediate` を保存する。

`AppSettings::load` は `serde_json::Value` に一度パースし、
`migrate_legacy_auto_settings(&mut value, Utc::now().timestamp())` を通してから
`serde_json::from_value` する。移行した場合は正規化後の JSON を
`db.set_setting(SETTINGS_KEY, ...)` で best-effort に書き戻す（失敗は
`tracing::warn!` のみで読み込みは続行）。壊れた JSON のバックアップ処理は
現行の分岐を維持する。設定行がない新規インストールでは、`Self::default()` の後で
`auto_compress.activated_at = Some(Utc::now().timestamp())` を入れる
（`auto_compress_enabled` の既定は `true`）。

`schema.rs` の `photos.created_at` は `to_rfc3339()` の文字列なので、後の SQL では
`CAST(strftime('%s', p.created_at) AS INTEGER)` で Unix 秒に変換する。

- [ ] **Step 4: テストを実行して通す**

Run: `cargo test -p vrc-photon-core settings::tests`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/core/src/settings.rs
git commit -m "feat(core): unify automatic behavior settings (#682)"
```

---

## Phase 2: 対象範囲の DB クエリ

### Task 2: 圧縮・AI・Discord の対象範囲

**Files:**
- Modify: `crates/core/src/db/compression.rs`
- Modify: `crates/core/src/db/ai.rs`
- Modify: `crates/core/src/db/send.rs`
- Test: `crates/core/src/db/tests.rs`

**Interfaces:**
- Consumes: `AutoBehavior::bulk_since()`（Task 1）。
- Produces:
  - `Db::enqueue_auto_jobs_for_uncompressed(&self, policy: &str, since: Option<i64>) -> Result<usize>`
  - `Db::enqueue_tagging_backlog(&self, plugin_id: &str, since: Option<i64>) -> Result<usize>`
  - `Db::count_unsent_photo_backlog(&self, since: Option<i64>) -> Result<i64>`
  - `Db::enqueue_unsent_photo_backlog(&self, since: Option<i64>) -> Result<usize>`

- [ ] **Step 1: 失敗する DB テストを書く**

`crates/core/src/db/tests.rs` に、写真の `created_at` を固定するヘルパーを追加する。

```rust
fn set_photo_created_at(db: &Db, photo_id: i64, created_at: &str) {
    db.conn
        .execute(
            "UPDATE photos SET created_at = ?1 WHERE id = ?2",
            rusqlite::params![created_at, photo_id],
        )
        .unwrap();
}
```

テスト:

```rust
#[test]
fn compression_backlog_scope_filters_by_created_at() {
    let db = Db::open_in_memory().unwrap();
    let old = seed_photo(&db);
    let new = seed_other_photo(&db);
    set_photo_created_at(&db, old, "2026-01-01T00:00:00+00:00");
    set_photo_created_at(&db, new, "2026-02-01T00:00:00+00:00");
    let since = chrono::DateTime::parse_from_rfc3339("2026-01-15T00:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(db.enqueue_auto_jobs_for_uncompressed("keep", Some(since)).unwrap(), 1);
    let jobs = db.list_jobs(10).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].photo_id, new);
    assert_ne!(jobs[0].photo_id, old);
}

#[test]
fn compression_backlog_without_since_includes_every_uncompressed_photo() {
    let db = Db::open_in_memory().unwrap();
    seed_photo(&db);
    seed_other_photo(&db);
    assert_eq!(db.enqueue_auto_jobs_for_uncompressed("keep", None).unwrap(), 2);
}

#[test]
fn tagging_backlog_scope_filters_by_created_at() {
    let db = Db::open_in_memory().unwrap();
    let old = seed_photo(&db);
    let new = seed_other_photo(&db);
    set_photo_created_at(&db, old, "2026-01-01T00:00:00+00:00");
    set_photo_created_at(&db, new, "2026-02-01T00:00:00+00:00");
    let since = chrono::DateTime::parse_from_rfc3339("2026-01-15T00:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(db.enqueue_tagging_backlog("vrc-photon-ai", Some(since)).unwrap(), 1);
    assert!(db.tagging_jobs_for_photo(old).unwrap().is_empty());
    assert_eq!(db.tagging_jobs_for_photo(new).unwrap().len(), 1);
}

#[test]
fn discord_unsent_backlog_counts_only_never_sent_photos() {
    let db = Db::open_in_memory().unwrap();
    let sent = seed_photo(&db);
    let failed = seed_other_photo(&db);
    let sent_id = db
        .enqueue_send(sent, SendKind::Photo, SendOrigin::Manual, SendStatus::Pending)
        .unwrap()
        .unwrap();
    db.conn
        .execute(
            "UPDATE send_outbox SET status = 'sent' WHERE id = ?1",
            [sent_id],
        )
        .unwrap();
    let failed_id = db
        .enqueue_send(failed, SendKind::Photo, SendOrigin::Manual, SendStatus::Pending)
        .unwrap()
        .unwrap();
    db.conn
        .execute(
            "UPDATE send_outbox SET status = 'failed' WHERE id = ?1",
            [failed_id],
        )
        .unwrap();
    assert_eq!(db.count_unsent_photo_backlog(None).unwrap(), 1);
    assert_eq!(db.enqueue_unsent_photo_backlog(None).unwrap(), 1);
    assert_eq!(db.count_unsent_photo_backlog(None).unwrap(), 0);
}

#[test]
fn discord_unsent_backlog_respects_created_at_and_active_rows() {
    let db = Db::open_in_memory().unwrap();
    let old = seed_photo(&db);
    let new = seed_other_photo(&db);
    set_photo_created_at(&db, old, "2026-01-01T00:00:00+00:00");
    set_photo_created_at(&db, new, "2026-02-01T00:00:00+00:00");
    db.enqueue_send(new, SendKind::Photo, SendOrigin::Manual, SendStatus::Waiting)
        .unwrap();
    let since = chrono::DateTime::parse_from_rfc3339("2026-01-15T00:00:00+00:00")
        .unwrap()
        .timestamp();
    assert_eq!(db.count_unsent_photo_backlog(Some(since)).unwrap(), 1);
    assert_eq!(db.enqueue_unsent_photo_backlog(Some(since)).unwrap(), 1);
    assert_eq!(db.count_unsent_photo_backlog(Some(since)).unwrap(), 0);
}
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-core db::tests::compression_backlog_scope_filters_by_created_at`
Expected: FAIL（引数が足りない／関数がない）

- [ ] **Step 3: SQL を実装する**

圧縮と AI の既存 `INSERT ... SELECT` に次の条件を追加する。

```sql
AND (?N IS NULL OR CAST(strftime('%s', p.created_at) AS INTEGER) >= ?N)
```

Discord は `send.rs` に2関数を追加する。対象条件は設計書の「Discord 自動送信」の
とおり。`enqueue_unsent_photo_backlog` は `INSERT OR IGNORE ... SELECT` の
単一 SQL で `kind = 'photo'`, `origin = 'auto'`, `status = 'pending'`,
`created_at = now` を積む（per-photo ループにしない）。

- [ ] **Step 4: テストを実行して通す**

Run: `cargo test -p vrc-photon-core db::tests`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/core/src/db
git commit -m "feat(core): scope automatic backlog queries (#682)"
```

---

## Phase 3: デーモンの自動投入

### Task 3: 3機能の自動投入を新しいモードへ

**Files:**
- Modify: `crates/daemon/src/compression.rs`
- Modify: `crates/daemon/src/scan.rs`
- Modify: `crates/daemon/src/discord.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Modify: `crates/daemon/src/ai_e2e.rs`
- Test: `crates/daemon/src/tests.rs`
- Test: `crates/daemon/src/discord.rs` のテスト

**Interfaces:**
- Consumes: `AutoMode`, `AutoBehavior`, `AppSettings`（Task 1）。
- Produces:
  - `enqueue_auto_jobs(db: &Db, new_ids: &[i64], keep_original: bool, mode: AutoMode) -> Result<usize>`
  - `enqueue_auto_tagging(db: &Db, new_ids: &[i64], settings: &AppSettings, plugin_ready: bool) -> Result<usize>`
  - `enqueue_auto_sends(db: &Db, photo_ids: &[i64]) -> Result<usize>`（既存シグネチャ、中身を変更）
  - `AppSettings::auto_compress_enabled` / `auto_compress` / `ai_tagging_auto` のデーモンコピー

- [ ] **Step 1: 失敗するデーモンテストを書く**

```rust
#[test]
fn initial_scan_never_enqueues_auto_compression_for_any_scope() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(&dir.path().join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress = AutoBehavior::immediate().with_scope(AutoScope::IncludeBacklog);
    });
    daemon.scan_photos_with_initial(Some(true)).unwrap();
    assert_eq!(daemon.db.lock().unwrap().count_active_jobs().unwrap(), 0);
}

#[test]
fn scan_root_enqueues_waiting_auto_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(&dir.path().join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress = AutoBehavior::waiting();
    });
    daemon.scan_root_photos(dir.path().join("photos")).unwrap();
    assert_eq!(
        daemon.db.lock().unwrap().count_jobs(QueueStatus::Waiting).unwrap(),
        1
    );
}

#[test]
fn disabled_compression_master_switch_skips_auto_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(&dir.path().join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress_enabled = false;
        settings.auto_compress = AutoBehavior::immediate();
    });
    daemon.scan_root_photos(dir.path().join("photos")).unwrap();
    assert_eq!(daemon.db.lock().unwrap().count_active_jobs().unwrap(), 0);
}

#[test]
fn compression_mode_off_skips_auto_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(&dir.path().join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress_enabled = true;
        settings.auto_compress = AutoBehavior::off();
    });
    daemon.scan_root_photos(dir.path().join("photos")).unwrap();
    assert_eq!(daemon.db.lock().unwrap().count_active_jobs().unwrap(), 0);
}

#[test]
fn waiting_ai_mode_enqueues_without_a_ready_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(&dir.path().join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"));
    save_daemon_settings(&daemon, |settings| {
        settings.ai_tagging_enabled = true;
        settings.ai_tagging_auto = AutoBehavior::waiting();
    });
    daemon.scan_root_photos(dir.path().join("photos")).unwrap();
    assert_eq!(
        daemon.db
            .lock()
            .unwrap()
            .tagging_job_counts(AI_PLUGIN_ID)
            .unwrap()
            .waiting,
        1
    );
}

#[test]
fn immediate_ai_mode_still_requires_a_ready_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(&dir.path().join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"));
    save_daemon_settings(&daemon, |settings| {
        settings.ai_tagging_enabled = true;
        settings.ai_tagging_auto = AutoBehavior::immediate();
    });
    daemon.scan_root_photos(dir.path().join("photos")).unwrap();
    let counts = daemon
        .db
        .lock()
        .unwrap()
        .tagging_job_counts(AI_PLUGIN_ID)
        .unwrap();
    assert_eq!(counts.waiting, 0);
    assert_eq!(counts.pending, 0);
    assert_eq!(counts.processing, 0);
}
```

既存テストの `auto_compress_backlog = true` は初回スキャンで自動投入を期待している
ため、`scan_root_photos` か手動の一括処理に置き換える。`discord_auto_since` を
設定しているテストは `discord_auto = AutoBehavior::immediate()` に置き換える。

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-daemon initial_scan_never_enqueues_auto_compression_for_any_scope`
Expected: FAIL（新しい型がない／挙動が違う）

- [ ] **Step 3: デーモンを実装する**

- `enqueue_auto_jobs` は `mode` を受け取り、`Off` なら0、`Immediate` なら
  `QueueStatus::Pending`、`Waiting` なら `QueueStatus::Waiting`。
- `enqueue_auto_tagging` は `settings.ai_tagging_auto.mode` を見る。
  `Off` は0。`Immediate` は `plugin_ready` が false なら0、true なら
  `TaggingJobStatus::Pending`。`Waiting` は `TaggingJobStatus::Waiting` を
  `plugin_ready` に関係なく積む。
- `enqueue_auto_sends` は `settings.discord_auto.mode` を見る。
  `discord_enabled`、Webhook の有効性は現行どおり。`discord_auto_since` と
  `captured_at_utc` の比較を削除する。
- `scan.rs` の自動投入条件:
  - 圧縮: `auto_compress_enabled && auto_compress.mode != Off && !new_ids.is_empty()`
  - Discord: `discord_enabled && discord_auto.mode != Off && !new_ids.is_empty()`
  - AI: `ai_tagging_enabled && ai_tagging_auto.mode != Off && !new_ids.is_empty()`
  - フルスキャン経路では上記に `&& !initial_scan` を足す。差分索引経路は
    `initial_scan` がないためそのまま。
- `lifecycle.rs::refresh_compression_settings` は
  `self.settings.auto_compress_enabled = settings.auto_compress_enabled;`、
  `self.settings.auto_compress = settings.auto_compress;`、
  `self.settings.ai_tagging_auto = settings.ai_tagging_auto;` に置き換える。
- `ai_e2e.rs` の `ai_tagging_auto = false` を `AutoBehavior::off()` に置き換える。

- [ ] **Step 4: テストを実行して通す**

Run: `cargo test -p vrc-photon-daemon`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/daemon
git commit -m "feat(daemon): apply the unified automatic modes (#682)"
```

---

## Phase 4: UI 共通コントロールと詳細カード

### Task 4: 3カードの共通化と表示順の統一

**Files:**
- Create: `crates/ui/src/ui/auto_behavior.rs`
- Modify: `crates/ui/src/ui/mod.rs`
- Modify: `crates/ui/src/ui/integrations.rs`
- Modify: `crates/ui/src/ui/discord.rs`
- Modify: `crates/ui/src/app/compression.rs`
- Modify: `crates/ui/src/app/discord.rs`
- Modify: `crates/ui/src/app/ai.rs`
- Test: 各ファイルの `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `AutoBehavior`, `AutoMode`, `AutoScope`, `AutoBehavior::bulk_since()`（Task 1）。
- Produces:
  - `ui::auto_behavior::auto_mode_segmented(palette: &Palette, id_prefix: &'static str, mode: AutoMode, cx: &mut Context<App>, on_mode: fn(&mut App, AutoMode, &mut Window, &mut Context<App>)) -> AnyElement`
  - `ui::auto_behavior::bulk_action_block(palette: &Palette, id_prefix: &'static str, behavior: AutoBehavior, enabled: bool, button_label: String, cx: &mut Context<App>, on_scope: fn(&mut App, AutoScope, &mut Window, &mut Context<App>), on_bulk: fn(&mut App, &mut Window, &mut Context<App>)) -> AnyElement`
  - `App::set_auto_compress_enabled(enabled, cx)`
  - `App::set_auto_compress_mode(mode, _window, cx)`
  - `App::set_auto_compress_scope(scope, _window, cx)`
  - `App::set_ai_auto_mode(mode, _window, cx)`
  - `App::set_ai_auto_scope(scope, _window, cx)`
  - `App::set_discord_auto_mode(mode, window, cx)`（`AutoMode` に変更）
  - `App::set_discord_auto_scope(scope, _window, cx)`
  - `App::send_discord_backlog(window, cx)`
  - `App::confirm_discord_backlog(cx)`
  - `pub(crate) fn bulk_action_available(master_enabled: bool, feature_ready: bool) -> bool`
  - `pub(crate) fn discord_backlog_available(settings: &AppSettings, webhook: Option<&str>) -> bool`

- [ ] **Step 1: 失敗する UI ロジックテストを書く**

`crates/ui/src/app/discord.rs` のテストを新しい型に更新し、次を追加する。

```rust
#[test]
fn changing_mode_sets_activation_only_when_leaving_off() {
    let mut settings = AppSettings::default();
    settings.discord_auto.mode = AutoMode::Off;
    apply_discord_auto_mode(&mut settings, AutoMode::Waiting, 100, true);
    assert_eq!(settings.discord_auto.activated_at, Some(100));
    apply_discord_auto_mode(&mut settings, AutoMode::Immediate, 200, false);
    assert_eq!(settings.discord_auto.activated_at, Some(100));
}

#[test]
fn discord_backlog_button_does_not_require_an_active_auto_mode() {
    let mut settings = AppSettings::default();
    settings.discord_enabled = true;
    settings.discord_auto = AutoBehavior::off();
    assert!(discord_backlog_available(&settings, Some("https://discord.com/api/webhooks/1/token")));
    settings.discord_enabled = false;
    assert!(!discord_backlog_available(&settings, Some("https://discord.com/api/webhooks/1/token")));
}

#[test]
fn bulk_action_is_enabled_when_the_mode_is_off_and_the_master_is_on() {
    assert!(bulk_action_available(true, true));
    assert!(!bulk_action_available(false, true));
    assert!(!bulk_action_available(true, false));
}

#[test]
fn bulk_since_uses_the_activation_time_for_new_only() {
    let mut behavior = AutoBehavior::waiting();
    behavior.activated_at = Some(42);
    behavior.scope = AutoScope::NewOnly;
    assert_eq!(behavior.bulk_since(), Some(42));
    behavior.scope = AutoScope::IncludeBacklog;
    assert_eq!(behavior.bulk_since(), None);
}
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-ui changing_mode_sets_activation_only_when_leaving_off`
Expected: FAIL

- [ ] **Step 3: 共通コンポーネントを作る**

`crates/ui/src/ui/auto_behavior.rs` に2つの部品を実装する。

1. `auto_mode_segmented`
   - 見出し「自動動作」＋3分割のセグメント（`{id_prefix}-mode-off/immediate/waiting`）。
   - 選択中のセグメントをアクセント色で塗る。
   - 下に選択中のモードの説明（`auto.behavior.mode.*_hint`）。
2. `bulk_action_block`
   - 見出し「一括処理」。
   - 対象範囲2チップ（`{id_prefix}-scope-new/backlog`）。
   - 説明「一括処理の対象にする写真の範囲を選びます。」
   - 選択中の対象範囲の説明（`auto.bulk.scope.*_hint`）。
   - 実行ボタン（`{id_prefix}-bulk`）。`enabled` が false なら `.disabled(true)`。
   - `cx.listener` で `on_scope` / `on_bulk` を呼ぶ。

セグメントは `crate::ui::components::chip` をベースにした専用の `segment` を使い、
チップの「対象範囲」と見た目を分ける。

- [ ] **Step 4: 3カードの表示順を統一する**

すべてのカードを次の順に並べる。

1. ヘッダー（タイトル＋マスタースイッチ）
2. 機能固有の設定
3. `auto_mode_segmented`
4. `bulk_action_block`
5. 状態・履歴・ジョブ

- **自動圧縮**
  - ヘッダーに `integrations-auto-enabled` スイッチを置く。
  - 機能固有: `keep_original`、警告、圧縮設定。
  - 自動動作、一括処理（`integrations-auto-bulk`、ラベル
    `integrations.auto.process_uncompressed`）。
  - 一括処理は `auto_compress_enabled` で有効／無効。`mode = Off` でも有効。
  - 一覧の状態ドットを `auto_compress_enabled` にする。
  - `process_uncompressed_compression` は `auto_compress.bulk_since()` を渡す。
- **Discord**
  - ヘッダーのマスタースイッチは現行のまま。
  - 機能固有: Webhook、送信先、テスト送信、送信画像、圧縮設定。
  - 自動動作、一括処理（`integrations-discord-bulk`、ラベル
    `integrations.discord.backlog.button`）。`mode != Off` の警告は自動動作の直後に出す。
  - 一括処理は `discord_enabled` と有効な Webhook で有効。`mode = Off` でも有効。
  - `send_discord_backlog` は件数を数え、0件なら情報トースト、1件以上なら
    `window.open_alert_dialog` で件数と非公開インスタンスの注意を表示し、OK で
    `confirm_discord_backlog` を呼ぶ。OK 時に `discord_consent_at` が `None` なら
    記録する。
  - 送信履歴は最後に置く。
- **AI タグ付け**
  - ヘッダーのマスタースイッチは現行のまま。
  - 機能固有: バックエンド。
  - 自動動作、一括処理（`integrations-ai-bulk`、ラベル
    `integrations.ai.process_untagged`）。一括処理は `ai_tagging_enabled` と
    プラグイン準備完了で有効。`mode = Off` でも有効。
  - 状態・履歴: 待機・処理中・失敗、実行中ジョブ、「待機中のものを処理」。
  - `integrations-ai-open-settings-not-ready` と
    `integrations-ai-open-settings-ready`、およびその案内文を削除する。
    未準備でも自動動作と一括処理ブロックを表示し、一括処理ボタンだけを無効にする。
  - 連携スイッチを `false` から `true` に戻したとき、
    `ai_tagging_auto.mode` に関係なく `activated_at` を `now` にする。
- Discord の `auto_since_for_feature_resume` / `auto_since_for_webhook_resume` を
  `auto_activation_for_feature_resume` / `auto_activation_for_webhook_resume` に
  置き換え、`discord_auto.activated_at` を更新する。テスト名も
  `enabling_the_feature_updates_the_activation_only_when_resuming` などに直す。
- 圧縮の `set_auto_compress_enabled` は、`false → true` のとき
  `auto_compress.activated_at = Some(now)` を入れる。
- `process_untagged_ai` は `ai_tagging_auto.bulk_since()` を
  `enqueue_tagging_backlog` に渡す。

- [ ] **Step 5: テストを実行して通す**

Run: `cargo test -p vrc-photon-ui`
Expected: PASS

- [ ] **Step 6: コミット**

```bash
git add crates/ui
git commit -m "feat(ui): unify the automatic and bulk controls in the integration cards (#682)"
```

---

## Phase 5: 初回セットアップ

### Task 5: 3ページを同じ形にする

**Files:**
- Modify: `crates/ui/src/ui/setup.rs`
- Modify: `crates/ui/src/app/setup.rs`
- Test: `crates/ui/src/app/setup.rs` のテスト

**Interfaces:**
- Consumes: `auto_behavior_editor`、Task 4 のハンドラ。
- Produces: `setup_ai(&self, cx: &mut Context<Self>) -> Div`。

- [ ] **Step 1: 失敗するテストを書く**

既存の `setup_pages_advance_and_return_in_order` は変更しない。追加で
`set_setup_immediate` が不要になったことを確認するため、コンパイルテストとして
`cargo check` を使う。ロジックの追加テストは不要。

- [ ] **Step 2: 自動圧縮ページを変更する**

`setup_compression` の `setup-auto-enabled` マスタースイッチは残す。
`setup-auto-immediate` を削除し、`setup-auto-keep`、警告、形式チップ、ロスレスを
残した後、最後に `auto_mode_segmented` を置く。`integration_settings_body` の
淡色化はマスタースイッチに従って維持する。一括処理ブロックは置かない。

- [ ] **Step 3: Discord ページを変更する**

既存の連携スイッチ、Webhook、送信先、プレイヤー名、原画優先の後に
`auto_mode_segmented` を置く。連携が `Off` のときは現行どおり淡色にする。
一括処理ブロックは置かない。

- [ ] **Step 4: AI ページを変更する**

`setup_ai` に `cx` を渡し、紹介文の後に `auto_mode_segmented` を置く。
プレビューと BOOTH の案内は残す。600×561 に収めるため、サンプル画像の高さを
76 px から 52 px に縮める。一括処理ブロック、設定画面へのボタン・案内は置かない。

- [ ] **Step 5: テストと目視確認**

Run: `cargo test -p vrc-photon-ui app::setup::tests`
Expected: PASS

`hyprland-visual-qa` または利用環境の visual QA 手順で `--demo` を起動し、
セットアップの3ページを 600×561 で確認する。

- [ ] **Step 6: コミット**

```bash
git add crates/ui/src/ui/setup.rs crates/ui/src/app/setup.rs
git commit -m "feat(ui): show the unified automatic controls in first-run setup (#682)"
```

---

## Phase 6: i18n・docs・検証・PR

### Task 6: 文言と仕上げ

**Files:**
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`
- Modify: `crates/i18n/tests/placeholders.rs`
- Modify: `docs/ui.md`, `docs/features.md`, `docs/data-model.md`, `docs/testing.md`
- Test: workspace

**Interfaces:**
- Consumes: Task 4/5 の UI キー。
- Produces: なし。

- [ ] **Step 1: 翻訳キーを追加する**

`ja.yml` / `en.yml` に次を追加する（番号は目安）。

```
auto.behavior.mode: 自動動作 / Automatic behavior
auto.behavior.mode.off: しない / Off
auto.behavior.mode.immediate: 即時 / Immediate
auto.behavior.mode.waiting: 待機 / Waiting
auto.behavior.mode.off_hint: 新しい写真を自動では処理しません。
auto.behavior.mode.immediate_hint: 新しく追加された写真をすぐに処理します。
auto.behavior.mode.waiting_hint: 新しく追加された写真を待機ジョブとして積み、あとから一括で開始できます。
auto.bulk.title: 一括処理 / Bulk actions
auto.bulk.hint: 一括処理の対象にする写真の範囲を選びます。
auto.bulk.scope.new_only: 有効化以降の新規のみ / New photos after enabling
auto.bulk.scope.include_backlog: 既存の未処理分も含める / Include existing unprocessed photos
auto.bulk.scope.new_only_hint: 一括処理は、機能を有効にした後に追加された写真だけを対象にします。
auto.bulk.scope.include_backlog_hint: 一括処理は、有効化前の写真も含めたすべての未処理分を対象にします。
integrations.discord.backlog.button: 未送信を一括送信 / Send unsent photos
integrations.discord.backlog.confirm.title: 未送信写真の一括送信 / Send unsent photos
integrations.discord.backlog.confirm.body: 既存の未送信写真 %{count} 件を Discord へ送信します。非公開インスタンスで撮影した写真も含まれます。
integrations.discord.backlog.confirm.ok: 送信する / Send
integrations.discord.backlog.empty: 送信対象の未送信写真はありません。
integrations.discord.backlog.started: 未送信写真 %{count} 件を送信キューに追加しました。
integrations.discord.backlog.error: 未送信写真を送信キューに追加できませんでした: %{error}
```

- `settings.startup.immediate` を ja / en から削除する。
  `settings.startup.auto_compress` は圧縮のマスタースイッチで使い続ける。
- `integrations.discord.auto.off/immediate/waiting/title` を削除する。
  `integrations.discord.auto.consent.*` は残す。
- `integrations.ai.auto` と `integrations.ai.open_settings` を削除する。
  `dashboard.ai.open_settings` はダッシュボード用なので残す。

- [ ] **Step 2: `placeholders.rs` を更新する**

`tr_args!` を使う `integrations.discord.backlog.confirm.body` /
`.started` / `.error` を `CALL_SITES` に追加する。引数名は `count` と `error`。

- [ ] **Step 3: docs を実装に合わせる**

- `docs/ui.md`
  - §2.9 の保存値のみの一覧から `auto_compress_backlog` を外す。
  - §2.10 の自動圧縮カードはマスタースイッチを残し、3値の自動動作、
    「一括処理」ブロック、圧縮設定の順に書く。一括処理は `mode = Off` でも
    使えることを明記する。
  - §2.10 の AI カードは自動動作と一括処理を共通化し、設定画面へのボタン・案内を
    置かないことを書く。
  - §2.10 の Discord 自動送信を共通化し、未送信の一括送信、件数付き確認、
    撮影時刻カットオフ廃止を書く。
  - §2.10 の3カードの表示順を「ヘッダー → 機能固有 → 自動動作 → 一括処理 →
    状態・履歴」に統一する。
  - §2.12 の Discord に自動動作を追加し、自動圧縮はマスタースイッチ＋
    自動動作＋元画像保持に変更し、AI に自動動作だけを追加する。
    セットアップには一括処理を置かない。
  - §10 参考に、#682 の設計・計画・モックのリンクを追加する。
- `docs/features.md`
  - §3.3 自動圧縮: `auto_compress_enabled`（マスタースイッチ）と
    `auto_compress.mode` の3値、`scope` の2値、初回スキャン除外、
    一括処理の対象、`Off` でも一括処理が使えることを書く。
    `auto_compress_immediate` と `auto_compress_backlog` の記述を削除する。
  - §4 取り込み: 初回スキャンの既存写真は自動圧縮しない。`ScanRoot` の新規写真は
    `scope` に関係なく自動圧縮する。既存写真は一括処理で処理する。
  - §13.5 Discord 自動送信: 3値、ライブラリ追加以降、初回スキャン除外、
    `scope`、未送信の一括送信、件数確認、`discord_auto_since` 廃止を書く。
  - §14.3 AI: 3値、`Immediate` の準備完了条件、`Waiting` の未準備時投入、
    初回スキャン除外、`scope`、一括処理の `Off` 時有効を書く。
    設定画面へのボタン・案内を削除する。
  - §9 の設定一覧（320行付近）から `auto_compress_backlog` を外し、
    「自動圧縮、新規写真の即時圧縮、…」を「自動圧縮（有効化・自動動作・
    一括処理）、自動圧縮後の元画像保持、…」に直す。
- `docs/data-model.md`
  - `app_settings` のキー一覧に `auto_compress_enabled` を追加し、
    `auto_compress` / `discord_auto` / `ai_tagging_auto` をオブジェクトにする。
  - 既定値を `auto_compress_enabled = true`、
    `auto_compress = Immediate / NewOnly`、`discord_auto = Off / NewOnly`、
    `ai_tagging_auto = Off / NewOnly` にする。
  - `discord_auto_since` を削除し、`activated_at` の説明を追加する。
- `docs/testing.md`
  - `scan_root` の新規写真が `auto_compress_backlog=false` でも自動圧縮に載る、の
    記述を、`scope` に関係なく `ScanRoot` の新規写真は自動圧縮に載る、に直す。
  - 初回フルスキャンでは自動圧縮・自動送信・自動タグ付けが走らないことを
    手動確認項目に追加する。
  - マスタースイッチがオンで自動動作が `Off` のとき、一括処理が使えることを
    手動確認項目に追加する。

- [ ] **Step 4: ローカル検証**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] **Step 5: 目視確認**

`--demo` で 1280×820 と 1080×700 を起動し、次を確認する。

- 連携・自動の3カードで「ヘッダー → 機能固有 → 自動動作 → 一括処理 →
  状態・履歴」の順序が同じ。
- 自動圧縮カードにマスタースイッチがあり、一括処理は `mode = Off` でも使える。
- マスタースイッチをオフにすると、一括処理が無効になる。
- AI カードに「設定を開く」ボタンと案内文がない。
- Discord の一括送信ボタンと件数確認ダイアログ。
- 初回セットアップの3ページに一括処理がなく、自動動作だけがあり、
  600×561 に収まる。

- [ ] **Step 6: 実装 PR を作る**

`feat: unify the automatic behavior settings across three features (#682)`。
本文に `Closes #682` を書く。ラベルは `enhancement`。CI green を確認して merge し、
`main` を pull する。

---

## 完了条件

- [ ] `AutoBehavior { mode, scope, activated_at }` が3機能で共有されている。
- [ ] 自動圧縮のマスタースイッチ `auto_compress_enabled` が実装されている。
- [ ] 旧設定 JSON が読み込み時に移行され、保存後に旧キーが消える。
- [ ] 初回フルスキャンでは3機能とも自動投入されない。
- [ ] `ScanRoot`・監視・2回目以降のフルスキャンで新規写真が自動投入される。
- [ ] 一括処理が `scope` に従い、`mode = Off` でも使え、マスタースイッチが
      オフのときだけ無効になる。
- [ ] Discord の一括送信が件数確認付きで、未送信写真だけを対象にする。
- [ ] AI の `Immediate` は未準備で積まず、`Waiting` は未準備でも積む。
- [ ] AI カードに「設定を開く」ボタンと案内文がない。
- [ ] 3カードの表示順が統一され、3セットアップページには一括処理がない。
- [ ] i18n のカタログ・placeholder テストを含む全テストが通る。
- [ ] 実装 PR が merge され、#682 が閉じる。
