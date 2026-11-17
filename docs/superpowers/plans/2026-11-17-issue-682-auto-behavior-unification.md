# 自動動作とキュー表示を3機能で統一する (#682 / #684) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 自動圧縮・Discord 自動送信・AI 自動タグ付けの「自動動作」「一括処理」「キュー表示」を、共通の `AutoBehavior` 型・共通 `QueueEntry` 表示・共通キュー操作に統一する。

**Architecture:** 設定 JSON に共通の `AutoBehavior { mode, scope, activated_at }` と機能別の `*_queue_paused` を持たせ、旧フィールドは読み込み時に移行する。ジョブのテーブル・状態・ワーカーは統合せず、UI が3機能の待機・実行待ち・実行中を `QueueEntry` に変換して同じ行部品で表示する。ダッシュボードは混合キュー、機能詳細は機能別キューを出し、失敗は通知モーダルに集約する。待機行だけを行単位・一括で削除できる。

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
- Discord の一括送信は未送信写真だけを対象にし、件数だけの確認ダイアログを出す。
- AI の `Immediate` はプラグイン準備完了時のみ積む。`Waiting` は未準備でも積む。
- AI カード内の「設定を開く」ボタンと案内文を削除する。ダッシュボードの AI カードは
  この Issue の対象外。
- カード内の表示順を「ヘッダー＋マスタースイッチ → 機能固有の設定 →
  自動動作 → 一括処理 → キュー」に統一する。
- セットアップページには「自動動作」だけを置き、「一括処理」は表示しない。
- 自動動作と一括処理はカード化せず、見出し・セグメント・ボタンをカード本文に
  直接置く。一括処理の対象範囲は自動動作と同じ2分割セグメントにする。
  自動動作と一括処理の間には区切り線を置く。
- Discord の非公開インスタンス警告を、カードにも同意ダイアログにも表示しない。
  `warning.private_instances` を削除し、同意ダイアログの本文からも
  非公開インスタンスの記述を削除する。
- ヒント行、重複ラベル、空表示、出力先の注記を置かない。#627 / #629 の
  「連携・自動タブの余計な表記削除」に合わせる。
- 確認ダイアログは件数だけを表示し、注意書き・補足を置かない。
- #684: キューは待機・実行待ち・実行中だけを表示する。失敗・完了・履歴は出さない。
- #684: 失敗は通知モーダルに最新5件まで出し、行ごとの再試行と機能別の一括再試行を
  用意する。失敗行はキューに戻さない。
- #684: 機能別の一時停止を設定に保存する。停止中も実行中のジョブは完了し、
  自動投入は続ける。停止中も待機の開始・削除はできる。
- #684: 待機行は行単位・一括で削除できる。実行待ち・実行中は削除しない。
  一括削除は件数だけの確認を出す。
- #684: ダッシュボードの圧縮専用キューパネルを3機能の混合キューに置き換える。
  混合キューに一時停止／再開は置かない。
- #684: Discord の送信履歴カード、「履歴を消去」、履歴行を削除する。
  待機の一括送信はキューの「待機をすべて開始」に移す。
- #684: ジョブのテーブル・状態・ワーカーは統合しない。terminal 行は通知と
  テスト送信の結果解決のために DB に残すが、履歴 UI は作らない。
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
7. 3カードで「ヘッダー → 機能固有 → 自動動作 → 一括処理 → キュー」の順が同じ。
   セットアップ3ページには一括処理がなく、自動動作だけがある。
8. AI カードに「設定を開く」ボタンと案内文がない。
9. #684: 混合キューが3機能の待機・実行待ち・実行中を `created_at` の古い順に
   出し、100件上限と合計件数が正しい。
10. #684: 一時停止が再起動後も維持され、停止中は新規取得だけが止まり、
    実行中は完了し、自動投入は続く。
11. #684: 待機だけが行単位・一括で削除でき、削除後に同じ写真を再投入できる。
12. #684: キューに失敗・完了・履歴が出ず、Discord の送信履歴カードと
    「履歴を消去」がない。
13. #684: 通知モーダルに3機能の最新5件の失敗と再試行が出る。
14. 自動動作と一括処理がカード化されず、一括処理の対象範囲が2分割セグメントに
    なっている。両者の間に区切り線があり、ヒント行・重複ラベル・空表示・
    出力先の注記がない。
15. Discord の一括送信と待機の一括削除の確認ダイアログが、件数だけを表示する。
16. Discord の非公開インスタンス警告が、カードにも同意ダイアログにもない。
17. ja / en の両カタログと `placeholders.rs` が更新され、i18n テストが通る。

---

## Phase 0: モック・設計・計画

**Files:**
- Create: `docs/ui-682-auto-behavior-unification-mockup.html`
- Create: `docs/superpowers/specs/2026-11-17-issue-682-auto-behavior-unification-design.md`
- Create: `docs/superpowers/plans/2026-11-17-issue-682-auto-behavior-unification.md`（このファイル）

- [x] **Step 1: モックを作る**

`docs/ui-682-auto-behavior-unification-mockup.html` に次を載せる。

- 共通コントロールの拡大図（自動動作の3分割セグメント、一括処理の対象範囲
  2分割セグメント＋実行ボタン）。説明文は置かず、カード化しない。
  `mode = Off` でも一括処理が使えることを示す。
- 連携・自動の3カード（自動圧縮・Discord・AI）の変更後。表示順は
  「ヘッダー → 機能固有 → 自動動作 → 一括処理 → キュー」。
- 自動圧縮カードのマスタースイッチ。
- 初回セットアップの3ページ（自動圧縮・Discord・AI）の変更後。
  一括処理は表示せず、自動動作だけを置く。
- AI カードから「設定を開く」ボタンと案内文を削除した状態。
- Discord の一括送信と待機の一括削除の、件数だけの確認ダイアログ。
- ヒント行・重複ラベル・空表示・出力先の注記を削除した状態（#627 / #629）。
- #684: ダッシュボードの3機能混合キュー（機能バッジ、待機・実行待ち・実行中、
  待機行の削除、100件上限）。
- #684: 機能詳細のキュー（件数、一時停止／再開、待機をすべて開始／削除、
  待機行の削除、失敗・履歴なし）。
- #684: 通知モーダルの「処理の失敗」（最新5件、再試行）。
- #684: Discord の送信履歴カードと「履歴を消去」がないこと。
- 判断項目と決定の一覧、移行の挙動変化、対象外。

- [x] **Step 2: docs PR を作る**

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

- [x] **Step 1: 失敗する移行テストを書く**

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

- [x] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-core settings::tests::legacy_compression_fields_migrate_to_common_behavior`
Expected: FAIL（`migrate_legacy_auto_settings` と `AutoBehavior` が未定義）

- [x] **Step 3: 型と移行を実装する**

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

- [x] **Step 4: テストを実行して通す**

Run: `cargo test -p vrc-photon-core settings::tests`
Expected: PASS

- [x] **Step 5: コミット**

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

- [x] **Step 1: 失敗する DB テストを書く**

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

- [x] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-core db::tests::compression_backlog_scope_filters_by_created_at`
Expected: FAIL（引数が足りない／関数がない）

- [x] **Step 3: SQL を実装する**

圧縮と AI の既存 `INSERT ... SELECT` に次の条件を追加する。

```sql
AND (?N IS NULL OR CAST(strftime('%s', p.created_at) AS INTEGER) >= ?N)
```

Discord は `send.rs` に2関数を追加する。対象条件は設計書の「Discord 自動送信」の
とおり。`enqueue_unsent_photo_backlog` は `INSERT OR IGNORE ... SELECT` の
単一 SQL で `kind = 'photo'`, `origin = 'auto'`, `status = 'pending'`,
`created_at = now` を積む（per-photo ループにしない）。

- [x] **Step 4: テストを実行して通す**

Run: `cargo test -p vrc-photon-core db::tests`
Expected: PASS

- [x] **Step 5: コミット**

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

- [x] **Step 1: 失敗するデーモンテストを書く**

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

- [x] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-daemon initial_scan_never_enqueues_auto_compression_for_any_scope`
Expected: FAIL（新しい型がない／挙動が違う）

- [x] **Step 3: デーモンを実装する**

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

- [x] **Step 4: テストを実行して通す**

Run: `cargo test -p vrc-photon-daemon`
Expected: PASS

- [x] **Step 5: コミット**

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

- [x] **Step 1: 失敗する UI ロジックテストを書く**

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

- [x] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-ui changing_mode_sets_activation_only_when_leaving_off`
Expected: FAIL

- [x] **Step 3: 共通コンポーネントを作る**

`crates/ui/src/ui/auto_behavior.rs` に2つの部品を実装する。

1. `auto_mode_segmented`
   - 見出し「自動動作」＋3分割のセグメント（`{id_prefix}-mode-off/immediate/waiting`）。
   - 選択中のセグメントをアクセント色で塗る。
   - 説明文は置かない。
2. `bulk_action_block`
   - 見出し「一括処理」。
   - 対象範囲2分割のセグメント（`{id_prefix}-scope-new/backlog`）。
     自動動作と同じ `segment` 部品・同じ見た目にする。
   - 説明文は置かない。
   - 実行ボタン（`{id_prefix}-bulk`）。`enabled` が false なら `.disabled(true)`。
   - `cx.listener` で `on_scope` / `on_bulk` を呼ぶ。

自動動作と一括処理はカード化せず、見出し・セグメント・ボタンをカード本文に
直接置く。専用の `segment` 部品を両方で共有する。

- [x] **Step 4: 3カードの表示順を統一する**

すべてのカードを次の順に並べる。

1. ヘッダー（タイトル＋マスタースイッチ）
2. 機能固有の設定
3. `auto_mode_segmented`
4. `bulk_action_block`
5. キュー（待機・実行待ち・実行中。Task 9 で追加）

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
  - 自動動作、区切り線、一括処理（`integrations-discord-bulk`、ラベル
    `integrations.discord.backlog.button`）。非公開インスタンスの警告は出さない。
  - 一括処理は `discord_enabled` と有効な Webhook で有効。`mode = Off` でも有効。
  - `send_discord_backlog` は件数を数え、0件なら情報トースト、1件以上なら
    `window.open_alert_dialog` で件数だけを表示し、OK で
    `confirm_discord_backlog` を呼ぶ。OK 時に `discord_consent_at` が `None` なら
    記録する。
  - 最後にキューを置く（Task 9）。送信履歴は置かない。
- **AI タグ付け**
  - ヘッダーのマスタースイッチは現行のまま。
  - 機能固有: バックエンド。
  - 自動動作、一括処理（`integrations-ai-bulk`、ラベル
    `integrations.ai.process_untagged`）。一括処理は `ai_tagging_enabled` と
    プラグイン準備完了で有効。`mode = Off` でも有効。
  - キュー: Task 9 で追加する。それまでの状態表示は残さない。
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

- [x] **Step 5: テストを実行して通す**

Run: `cargo test -p vrc-photon-ui`
Expected: PASS

- [x] **Step 6: コミット**

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

- [x] **Step 1: 失敗するテストを書く**

既存の `setup_pages_advance_and_return_in_order` は変更しない。追加で
`set_setup_immediate` が不要になったことを確認するため、コンパイルテストとして
`cargo check` を使う。ロジックの追加テストは不要。

- [x] **Step 2: 自動圧縮ページを変更する**

`setup_compression` の `setup-auto-enabled` マスタースイッチは残す。
`setup-auto-immediate` を削除し、`setup-auto-keep`、警告、形式チップ、ロスレスを
残した後、最後に `auto_mode_segmented` を置く。`integration_settings_body` の
淡色化はマスタースイッチに従って維持する。一括処理ブロックは置かない。

- [x] **Step 3: Discord ページを変更する**

既存の連携スイッチ、Webhook、送信先、プレイヤー名、原画優先の後に
`auto_mode_segmented` を置く。連携が `Off` のときは現行どおり淡色にする。
一括処理ブロックは置かない。

- [x] **Step 4: AI ページを変更する**

`setup_ai` に `cx` を渡し、紹介文の後に `auto_mode_segmented` を置く。
プレビューと BOOTH の案内は残す。600×561 に収めるため、サンプル画像の高さを
76 px から 52 px に縮める。一括処理ブロック、設定画面へのボタン・案内は置かない。

- [x] **Step 5: テストと目視確認**

Run: `cargo test -p vrc-photon-ui app::setup::tests`
Expected: PASS

`hyprland-visual-qa` または利用環境の visual QA 手順で `--demo` を起動し、
セットアップの3ページを 600×561 で確認する。

- [x] **Step 6: コミット**

```bash
git add crates/ui/src/ui/setup.rs crates/ui/src/app/setup.rs
git commit -m "feat(ui): show the unified automatic controls in first-run setup (#682)"
```

---

## Phase 6: #684 キューの設定と DB API

### Task 7: 機能別 pause 設定と共通キュー表示モデル

**Files:**
- Modify: `crates/core/src/settings.rs`
- Create: `crates/core/src/queue.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/db/compression.rs`
- Modify: `crates/core/src/db/send.rs`
- Modify: `crates/core/src/db/ai.rs`
- Test: `crates/core/src/settings.rs`, `crates/core/src/db/tests.rs`

**Interfaces:**
- Consumes: なし。
- Produces:
  - `AppSettings { pub auto_compress_queue_paused: bool, pub discord_queue_paused: bool, pub ai_tagging_queue_paused: bool }`（既定 `false`）
  - `pub enum QueueFeature { Compression, Discord, Tagging }`
  - `pub enum QueueState { Waiting, Pending, Processing }`
  - `pub struct QueueEntry { feature, job_id: String, photo_id: Option<i64>, photo_name: Option<String>, state, progress: f32, detail: String, created_at: String }`
  - `pub struct QueueCounts { waiting: i64, pending: i64, processing: i64 }`
  - `pub struct FailedJobEntry { feature, job_id: String, photo_id: Option<i64>, photo_name: Option<String>, error: String, created_at: String }`
  - `Db::mixed_active_queue_entries(limit: i64) -> Result<Vec<QueueEntry>>`
  - `Db::active_queue_entries(feature, limit) -> Result<Vec<QueueEntry>>`
  - `Db::queue_counts(feature) -> Result<QueueCounts>`
  - `Db::delete_waiting_entry(feature, job_id) -> Result<bool>`
  - `Db::delete_all_waiting_entries(feature) -> Result<usize>`
  - `Db::recent_failed_sends(limit) -> Result<Vec<FailedJobEntry>>` / `count_failed_sends()`
  - `Db::recent_failed_tagging_jobs(plugin_id, limit) -> Result<Vec<FailedJobEntry>>` / `failed_tagging_job_count(plugin_id)`

- [x] **Step 1: 失敗するテストを書く**

```rust
#[test]
fn queue_pause_settings_default_to_false_and_survive_roundtrip() {
    let db = Db::open_in_memory().unwrap();
    let mut settings = AppSettings::load(&db).unwrap();
    assert!(!settings.auto_compress_queue_paused);
    assert!(!settings.discord_queue_paused);
    assert!(!settings.ai_tagging_queue_paused);
    settings.auto_compress_queue_paused = true;
    settings.discord_queue_paused = true;
    settings.ai_tagging_queue_paused = true;
    settings.save(&db).unwrap();
    let loaded = AppSettings::load(&db).unwrap();
    assert!(loaded.auto_compress_queue_paused);
    assert!(loaded.discord_queue_paused);
    assert!(loaded.ai_tagging_queue_paused);
}

#[test]
fn mixed_queue_entries_are_oldest_first_and_exclude_terminal_rows() {
    let db = Db::open_in_memory().unwrap();
    // Seed one waiting compression job, one pending send, one processing tagging job,
    // plus a failed and a completed row that must not appear.
    let entries = db.mixed_active_queue_entries(100).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries.windows(2).all(|w| w[0].created_at <= w[1].created_at));
    assert!(entries.iter().all(|e| matches!(e.state, QueueState::Waiting | QueueState::Pending | QueueState::Processing)));
}

#[test]
fn deleting_a_waiting_row_allows_the_same_photo_to_be_enqueued_again() {
    let db = Db::open_in_memory().unwrap();
    // Enqueue waiting compression job, delete it, enqueue again.
    assert!(db.delete_waiting_entry(QueueFeature::Compression, &job_id).unwrap());
    assert!(!db.delete_waiting_entry(QueueFeature::Compression, &job_id).unwrap());
    assert!(db.enqueue_auto_job_with_status(photo_id, "keep", QueueStatus::Waiting).unwrap());
}
```

- [x] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-core db::tests::mixed_queue_entries_are_oldest_first_and_exclude_terminal_rows`
Expected: FAIL（型と API がない）

- [x] **Step 3: 型と DB API を実装する**

`crates/core/src/queue.rs` に表示モデルを置く。DB のクエリは3テーブルを
`status IN ('waiting','pending','processing')` で絞り、`photos.file_name` を
LEFT JOIN する。混合キューは3つの一覧を Rust 側でマージし、`created_at` の
古い順に並べて先頭 `limit` 件を返す。

削除 API は `DELETE ... WHERE status = 'waiting'` のみ。実行待ち・実行中は
変更しない。失敗通知用に、各テーブルの `status = 'failed'` を新しい順に返す
クエリと件数クエリを追加する。

`AppSettings` に `*_queue_paused` を追加し、`Default` と `#[serde(default)]` を
設定する。既存 JSON の移行は不要（欠損は false）。

- [x] **Step 4: テストを実行して通す**

Run: `cargo test -p vrc-photon-core settings::tests && cargo test -p vrc-photon-core db::tests`
Expected: PASS

- [x] **Step 5: コミット**

```bash
git add crates/core
git commit -m "feat(core): add queue pause settings and unified queue queries (#684)"
```

---

## Phase 7: #684 機能別一時停止

### Task 8: ワーカーの停止ゲート

**Files:**
- Modify: `crates/daemon/src/compression.rs`
- Modify: `crates/daemon/src/discord.rs`
- Modify: `crates/daemon/src/tagging.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Test: `crates/daemon/src/tests.rs`, `crates/daemon/src/tagging.rs`

**Interfaces:**
- Consumes: `AppSettings::*_queue_paused`（Task 7）。
- Produces:
  - `compression::worker_loop(..., queue_paused: Arc<AtomicBool>)`
  - `TaggingWorker::with_queue_paused(Arc<AtomicBool>)`
  - `Daemon` の `compression_queue_paused` / `tagging_queue_paused` フィールド
  - `refresh_compression_settings` で pause 設定を反映

- [x] **Step 1: 失敗する一時停止テストを書く**

```rust
#[test]
fn paused_compression_worker_does_not_claim_pending_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(&dir.path().join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"));
    daemon.scan_root_photos(dir.path().join("photos")).unwrap();
    save_daemon_settings(&daemon, |settings| settings.auto_compress_queue_paused = true);
    daemon.refresh_compression_settings();
    daemon.compress_pending().unwrap();
    let counts = daemon.db.lock().unwrap();
    assert_eq!(counts.count_jobs(QueueStatus::Processing).unwrap(), 0);
    assert_eq!(counts.count_jobs(QueueStatus::Pending).unwrap(), 1);
}

#[test]
fn paused_discord_worker_does_not_claim_pending_sends() { /* seed pending send, set paused, process_once, assert pending */ }

#[test]
fn paused_tagging_worker_does_not_claim_pending_jobs() { /* seed pending job, set paused, process_once, assert pending */ }
```

- [x] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-daemon paused_compression_worker_does_not_claim_pending_jobs`
Expected: FAIL

- [x] **Step 3: ワーカーに停止ゲートを追加する**

- 圧縮: `worker_loop` に `queue_paused` を追加し、`paused || queue_paused` を
  バッチ取得前に見る。既存のグローバル停止は維持する。
- Discord: `claim_ready_send` / `process_next_send` の前に
  `settings.discord_queue_paused` を見る。停止中は `Ok(ProcessOutcome::Idle)` を返し、
  `JobSignal` で再開を待つ。
- AI: `TaggingWorker` に `queue_paused` を追加し、`process_once` と
  `run_worker_loop` の取得前に見る。
- `lifecycle.rs`:
  - `compression_queue_paused` / `tagging_queue_paused` の `Arc<AtomicBool>` を持つ。
  - `refresh_compression_settings` で設定値を store する。
  - ワーカー起動時に `Arc` を渡す。
  - `refresh_compression_settings` が pause 値の変化を検出したら、
    `jobs.notify()` / `sends.notify()` / `tagging.notify()` を呼ぶ。
- UI は設定を保存するだけでよい。デーモンの1秒ごとの設定再読込が
  pause の変化を拾い、ワーカーを起こす。即時性が必要なら、保存後に
  `WakeJobs` / `WakeSends` / `WakeTagging` を送ってもよい。

- [x] **Step 4: テストを実行して通す**

Run: `cargo test -p vrc-photon-daemon`
Expected: PASS

- [x] **Step 5: コミット**

```bash
git add crates/daemon
git commit -m "feat(daemon): honor per-feature queue pause (#684)"
```

---

## Phase 8: #684 キュー UI とダッシュボード

### Task 9: 共通キュー部品・機能別キュー・混合キュー

**Files:**
- Create: `crates/ui/src/ui/queue.rs`
- Modify: `crates/ui/src/ui/mod.rs`
- Modify: `crates/ui/src/ui/integrations.rs`
- Modify: `crates/ui/src/ui/discord.rs`
- Modify: `crates/ui/src/ui/dashboard.rs`
- Modify: `crates/ui/src/ui/compression.rs`
- Modify: `crates/ui/src/app.rs`
- Modify: `crates/ui/src/app/live.rs`
- Modify: `crates/ui/src/app/compression.rs`
- Modify: `crates/ui/src/app/discord.rs`
- Modify: `crates/ui/src/app/ai.rs`
- Test: 各ファイルの `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `QueueEntry`, `QueueCounts`, `QueueFeature`（Task 7）、pause 設定（Task 7/8）。
- Produces:
  - `ui::queue::queue_section(app: &App, feature: QueueFeature, entries: &[QueueEntry], counts: QueueCounts, paused: bool, cx: &mut Context<App>) -> AnyElement`
  - `ui::queue::queue_row(app: &App, entry: &QueueEntry, show_feature: bool, cx: &mut Context<App>) -> AnyElement`
  - `App::set_queue_paused(feature, paused, cx)`
  - `App::delete_waiting_entry(feature, job_id, cx)`
  - `App::delete_all_waiting(feature, window, cx)`
  - `App::release_waiting_queue(feature, window, cx)`
  - `App::mixed_queue_entries` / `mixed_queue_counts`

- [x] **Step 1: 失敗する UI ロジックテストを書く**

```rust
#[test]
fn mixed_queue_orders_features_by_created_at() {
    let entries = vec![
        entry(QueueFeature::Tagging, "3"),
        entry(QueueFeature::Compression, "1"),
        entry(QueueFeature::Discord, "2"),
    ];
    let sorted = sort_mixed_queue(entries);
    assert_eq!(sorted[0].created_at, "1");
    assert_eq!(sorted[2].created_at, "3");
}

#[test]
fn waiting_rows_can_be_deleted_but_pending_and_processing_cannot() {
    assert!(waiting_delete_available(QueueState::Waiting));
    assert!(!waiting_delete_available(QueueState::Pending));
    assert!(!waiting_delete_available(QueueState::Processing));
}
```

- [x] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-ui mixed_queue_orders_features_by_created_at`
Expected: FAIL

- [x] **Step 3: 共通キュー部品を作る**

`crates/ui/src/ui/queue.rs` に次を実装する。

- `queue_section`: 見出し「キュー」、件数、`一時停止／再開`、
  `待機をすべて開始`、`待機をすべて削除`、行一覧。
- `queue_row`: 機能バッジ（混合キューのみ）、状態バッジ、写真名、
  進捗／ステージ、待機行の削除ボタン。
- 行の高さ・仮想リストは既存の `QueueRow` を置き換え、100件上限で使う。
- 状態ラベルは `QueueState` から翻訳キーへ変換する。
- 削除ボタンは `QueueState::Waiting` のときだけ出す。
- `待機をすべて削除` は `delete_all_waiting` を呼ぶ前に件数だけの確認を出す。
- `待機をすべて開始` は既存の release API を呼ぶ。圧縮は `keep_original=false`
  のとき既存の置換確認を出す。

- [x] **Step 4: 3カードとダッシュボードを置き換える**

- 自動圧縮: 一括処理の下に `queue_section(Compression, ...)`。
- Discord: 送信履歴カードを削除し、一括処理の下に
  `queue_section(Discord, ...)`。`send_history_card`、履歴行、メッセージリンク、
  再送・取消、`履歴を消去` を削除する。テスト送信の結果解決に必要な
  terminal 行の取得は残し、表示しない。
- AI: 失敗件数・最新失敗行を削除し、一括処理の下に
  `queue_section(Tagging, ...)`。待機・実行中・実行待ちだけを出す。
- ダッシュボード: `queue_panel` を `mixed_queue_panel` に置き換える。
  機能バッジ付きの行、合計件数、100件上限、空表示を出す。一時停止／再開は置かない。
- 「未完了」統計カードは新しい `mixed_queue_counts` の合計にする。
  既存の `active_job_count`（圧縮専用）はサイドバーなどで使い続ける。
- 圧縮の `queue_panel` / failed row / retry-all は削除または通知側へ移す。
- サイドバーの圧縮カードとリボンの圧縮操作は変更しない。

- [x] **Step 5: テストを実行して通す**

Run: `cargo test -p vrc-photon-ui`
Expected: PASS

- [x] **Step 6: コミット**

```bash
git add crates/ui
git commit -m "feat(ui): unify the feature queues and the dashboard mixed queue (#684)"
```

---

## Phase 9: #684 失敗通知

### Task 10: 通知モーダルの失敗行と再試行

**Files:**
- Modify: `crates/ui/src/ui/notifications.rs`
- Modify: `crates/ui/src/app/live.rs`
- Modify: `crates/ui/src/app.rs`
- Modify: `crates/ui/src/app/compression.rs`
- Modify: `crates/ui/src/app/discord.rs`
- Modify: `crates/ui/src/app/ai.rs`
- Modify: `crates/core/src/db/send.rs`
- Modify: `crates/core/src/db/ai.rs`
- Test: `crates/ui/src/ui/notifications.rs`, `crates/ui/src/app.rs`,
  `crates/core/src/db/tests.rs`

**Interfaces:**
- Consumes: `recent_failed_sends` / `recent_failed_tagging_jobs`（Task 7）、
  `FailedJobEntry`。
- Produces:
  - `FailureNotice::{Discord, Tagging}`
  - `NotificationSection::JobFailures`
  - `NotificationSnapshot { compression_failed, discord_failed, tagging_failed, entries }`
  - `App::retry_failed_send(id, cx)` / `App::retry_failed_tagging_job(id, cx)`
  - `App::retry_all_failed_sends(cx)` / `App::retry_all_failed_tagging_jobs(cx)`
  - `Db::retry_all_failed_sends()` / `Db::retry_all_failed_tagging_jobs(plugin_id)`

- [x] **Step 1: 失敗する通知テストを書く**

```rust
#[test]
fn job_failures_section_appears_only_when_a_feature_has_failures() {
    assert!(!notification_sections(false, false, 0, 0, 0, 0)
        .contains(&NotificationSection::JobFailures));
    assert!(notification_sections(false, false, 0, 0, 1, 0)
        .contains(&NotificationSection::JobFailures));
}

#[test]
fn failed_job_entries_are_newest_first_and_capped_at_five() {
    let entries = vec![/* 7 entries */];
    let shown = latest_failed_entries(entries, 5);
    assert_eq!(shown.len(), 5);
    assert!(shown.windows(2).all(|w| w[0].created_at >= w[1].created_at));
}
```

- [x] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p vrc-photon-ui job_failures_section_appears_only_when_a_feature_has_failures`
Expected: FAIL

- [x] **Step 3: 通知モーダルを拡張する**

- `FailureNotice` に Discord / Tagging を追加し、`failure_transition` の
  比較対象を3機能＋daemon errors に広げる。
- `NotificationSection::JobFailures` を追加し、いずれかの失敗件数が1以上なら出す。
- 通知カードに機能バッジ、写真名、エラー概要、再試行ボタンを出す。
  最新5件まで。機能ごとの件数と「すべて再試行」も出す。
- 再試行後は該当行が消え、再試行で `pending` になった行がキューに出る。
- 失敗行はキューに表示しない。
- 既存の圧縮通知カードの再試行は統合する。

- [x] **Step 4: 再試行 API を追加する**

Discord と AI に失敗全件の一括再試行を追加する。既存の個別再試行
（`retry_send`, `retry_tagging_job`）を UI から呼ぶ。

- [x] **Step 5: テストを実行して通す**

Run: `cargo test -p vrc-photon-ui && cargo test -p vrc-photon-core db::tests`
Expected: PASS

- [x] **Step 6: コミット**

```bash
git add crates/ui crates/core/src/db
git commit -m "feat(ui): notify failed queue jobs with retry (#684)"
```

---

## Phase 10: i18n・docs・検証・PR

### Task 11: 文言と仕上げ

**Files:**
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`
- Modify: `crates/i18n/tests/placeholders.rs`
- Modify: `docs/ui.md`, `docs/features.md`, `docs/data-model.md`, `docs/testing.md`
- Test: workspace

**Interfaces:**
- Consumes: Task 4/5/9/10 の UI キー。
- Produces: なし。

- [x] **Step 1: 翻訳キーを追加する**

`ja.yml` / `en.yml` に次を追加する（番号は目安）。

```
auto.behavior.mode: 自動動作 / Automatic behavior
auto.behavior.mode.off: しない / Off
auto.behavior.mode.immediate: 即時 / Immediate
auto.behavior.mode.waiting: 待機 / Waiting
auto.bulk.title: 一括処理 / Bulk actions
auto.bulk.scope.new_only: 有効化以降の新規のみ / New photos after enabling
auto.bulk.scope.include_backlog: 既存の未処理分も含める / Include existing unprocessed photos
integrations.discord.backlog.button: 未送信を一括送信 / Send unsent photos
integrations.discord.backlog.confirm.title: 未送信写真の一括送信 / Send unsent photos
integrations.discord.backlog.confirm.body: 送信対象: %{count} 件 / %{count} selected
integrations.discord.backlog.confirm.ok: 送信する / Send
integrations.discord.backlog.empty: 送信対象の未送信写真はありません。
integrations.discord.backlog.started: 未送信写真 %{count} 件を送信キューに追加しました。
integrations.discord.backlog.error: 未送信写真を送信キューに追加できませんでした: %{error}
queue.title: キュー / Queue
queue.count.waiting: 待機 %{count}
queue.count.pending: 実行待ち %{count}
queue.count.processing: 実行中 %{count}
queue.pause: 一時停止 / Pause
queue.resume: 再開 / Resume
queue.paused: 一時停止中 / Paused
queue.start_waiting: 待機をすべて開始 / Start all waiting
queue.delete_waiting_all: 待機をすべて削除 / Delete all waiting
queue.delete_waiting_all.confirm.title: 待機中の行を削除 / Delete waiting rows
queue.delete_waiting_all.confirm.body: 削除対象: %{count} 件 / %{count} selected
queue.delete_waiting_all.confirm.ok: 削除する / Delete
queue.delete_waiting_all.done: 待機中の %{count} 件を削除しました。
queue.delete_waiting_all.error: 待機中の行を削除できませんでした: %{error}
queue.delete_waiting_row: 削除 / Delete
queue.delete_waiting_row.done: 待機中の行を削除しました。
queue.delete_waiting_row.error: 待機中の行を削除できませんでした: %{error}
queue.start_waiting.done: %{count} 件を開始しました。
queue.start_waiting.error: 待機中の行を開始できませんでした: %{error}
queue.empty: キューは空です。 / The queue is empty.
queue.limit: 先頭 %{count} 件を表示 / Showing the first %{count}
queue.feature.compression: 自動圧縮 / Auto compression
queue.feature.discord: Discord / Discord
queue.feature.tagging: AI タグ付け / AI tagging
queue.state.waiting: 待機 / Waiting
queue.state.pending: 実行待ち / Pending
queue.state.processing: 実行中 / Processing
notifications.job_failures.title: 処理の失敗 / Job failures
notifications.job_failures.count: %{count} 件 / %{count}
notifications.job_failures.retry: 再試行 / Retry
notifications.job_failures.retry_all: すべて再試行 / Retry all
notifications.job_failures.error: 失敗した処理を再試行できませんでした: %{error}
```

- `settings.startup.immediate` を ja / en から削除する。
  `settings.startup.auto_compress` は圧縮のマスタースイッチで使い続ける。
- `integrations.discord.auto.off/immediate/waiting/title` を削除する。
  `integrations.discord.auto.consent.*` は残すが、`consent.body` から
  非公開インスタンスの記述を削除する。
- `warning.private_instances` を削除する。カードの警告と同意ダイアログの
  非公開インスタンス記述をなくす。
- `integrations.ai.auto` と `integrations.ai.open_settings` を削除する。
  `dashboard.ai.open_settings` はダッシュボード用なので残す。
- `integrations.discord.history.*` と `compression.queue.*` のうち、履歴・失敗行・
  再試行で未使用になったキーを削除する。`send.status.*` は通知・キューで
  使うものを残す。
- `integrations.ai.jobs.failed` / `integrations.ai.jobs.latest_failure` が
  未使用になったら削除する。

- [x] **Step 2: `placeholders.rs` を更新する**

`tr_args!` を使う次のキーを `CALL_SITES` に追加する。引数名は `count` と `error`。

- `integrations.discord.backlog.confirm.body` / `.started` / `.error`
- `queue.count.waiting` / `.pending` / `.processing`
- `queue.delete_waiting_all.confirm.body` / `.done` / `.error`
- `queue.delete_waiting_row.error`
- `queue.start_waiting.done` / `.error`
- `queue.limit`
- `notifications.job_failures.count` / `.error`

- [x] **Step 3: docs を実装に合わせる**

- `docs/ui.md`
  - §2.9 の保存値のみの一覧から `auto_compress_backlog` を外す。
  - §2.10 の自動圧縮カードはマスタースイッチを残し、3値の自動動作、
    「一括処理」セクション、圧縮設定の順に書く。一括処理は `mode = Off` でも
    使えることを明記する。対象範囲は2分割セグメントにする。
  - §2.10 の AI カードは自動動作と一括処理を共通化し、設定画面へのボタン・案内を
    置かないことを書く。
  - §2.10 の Discord 自動送信を共通化し、未送信の一括送信、件数だけの確認、
    撮影時刻カットオフ廃止を書く。
  - §2.10 の自動動作と一括処理はカード化せず、ヒント行・重複ラベル・空表示・
    出力先の注記を置かない（#627 / #629 に合わせる）。
  - §2.10 の3カードの表示順を「ヘッダー → 機能固有 → 自動動作 → 一括処理 →
    キュー」に統一する。
  - §2.12 の Discord に自動動作を追加し、自動圧縮はマスタースイッチ＋
    自動動作＋元画像保持に変更し、AI に自動動作だけを追加する。
    セットアップには一括処理を置かない。
  - §2.8 ダッシュボードの圧縮専用キューを、3機能の混合キューに置き換える。
    機能バッジ、待機・実行待ち・実行中、待機行の削除、100件上限を書く。
  - §2.10 の3カードに「キュー」セクションを追加し、件数、一時停止／再開、
    待機をすべて開始、待機をすべて削除、待機行の削除を書く。
  - §2.10 の Discord 送信履歴の節を削除し、履歴を表示しないことを書く。
  - §2.10 の AI カードから失敗件数・最新失敗・設定導線を削除する。
  - §通知/モーダルに「処理の失敗」セクションを追加し、最新5件と再試行を書く。
  - §10 参考に、#682 / #684 の設計・計画・モックのリンクを追加する。
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
    一括処理・キュー）、自動圧縮後の元画像保持、…」に直す。
  - §3.5 キュー: 待機・実行待ち・実行中だけを表示し、失敗・完了・履歴を
    出さないことを書く。一時停止の保存、実行中の完了、自動投入の継続、
    待機の行削除・一括削除、件数確認を書く。
  - §10 エラー表示: 失敗通知の最新5件と再試行、Discord/AI の失敗も通知に
    集約することを書く。
  - §13 Discord: 送信履歴カードと「履歴を消去」を削除し、待機の一括送信を
    キューの「待機をすべて開始」に移す。terminal 行は DB に残すが表示しない。
  - §14 AI: 連携カードの失敗件数・最新失敗・設定導線を削除し、キューは
    待機・実行待ち・実行中だけにする。
- `docs/data-model.md`
  - `app_settings` のキー一覧に `auto_compress_enabled` を追加し、
    `auto_compress` / `discord_auto` / `ai_tagging_auto` をオブジェクトにする。
  - 既定値を `auto_compress_enabled = true`、
    `auto_compress = Immediate / NewOnly`、`discord_auto = Off / NewOnly`、
    `ai_tagging_auto = Off / NewOnly` にする。
  - `discord_auto_since` を削除し、`activated_at` の説明を追加する。
  - `auto_compress_queue_paused` / `discord_queue_paused` /
    `ai_tagging_queue_paused` を追加し、既定 `false` を書く。
  - キューは待機・実行待ち・実行中だけを表示し、terminal 行は通知・テスト送信の
    結果解決のために残すが履歴 UI は作らないことを書く。
  - 待機の削除は行削除であり、`pending` / `processing` は削除しないことを書く。
- `docs/testing.md`
  - `scan_root` の新規写真が `auto_compress_backlog=false` でも自動圧縮に載る、の
    記述を、`scope` に関係なく `ScanRoot` の新規写真は自動圧縮に載る、に直す。
  - 初回フルスキャンでは自動圧縮・自動送信・自動タグ付けが走らないことを
    手動確認項目に追加する。
  - マスタースイッチがオンで自動動作が `Off` のとき、一括処理が使えることを
    手動確認項目に追加する。
  - 混合キューの3機能表示、100件上限、並び順を確認する。
  - 一時停止の保存・再起動後の維持・実行中の完了・自動投入の継続を確認する。
  - 待機の行削除・一括削除と、実行待ち・実行中が削除できないことを確認する。
  - Discord の履歴カードと「履歴を消去」がないこと、失敗が通知に出ることを確認する。

- [x] **Step 4: ローカル検証**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [x] **Step 5: 目視確認**

`--demo` で 1280×820 と 1080×700 を起動し、次を確認する。

- 連携・自動の3カードで「ヘッダー → 機能固有 → 自動動作 → 一括処理 →
  キュー」の順序が同じ。
- 自動圧縮カードにマスタースイッチがあり、一括処理は `mode = Off` でも使える。
- マスタースイッチをオフにすると、一括処理が無効になる。
- AI カードに「設定を開く」ボタンと案内文がない。
- Discord の一括送信ボタンと、件数だけの確認ダイアログ。
- 自動動作と一括処理がカード化されず、一括処理の対象範囲も2分割セグメントに
  なっている。両者の間に区切り線があり、ヒント行・重複ラベル・空表示がない。
- Discord の非公開インスタンス警告が、カードにも同意ダイアログにもない。
- 初回セットアップの3ページに一括処理がなく、自動動作だけがあり、
  600×561 に収まる。
- ダッシュボードが3機能の混合キューになり、機能バッジ・状態・写真名・
  待機行の削除が出る。圧縮専用キューがない。
- 機能詳細のキューに件数、一時停止／再開、待機をすべて開始／削除、
  待機行の削除がある。失敗・履歴行がない。
- 一時停止中は新規行が実行中にならず、実行中の行は完了する。
- Discord の送信履歴カードと「履歴を消去」がない。
- 通知モーダルに3機能の失敗行（最新5件）と再試行がある。

- [x] **Step 6: 実装 PR を作る**

`feat: unify the automatic behavior and queue views across three features (#682, #684)`。
本文に `Closes #682` と `Closes #684` を書く。ラベルは `enhancement`。
CI green を確認して merge し、`main` を pull する。

---

## 完了条件

- [x] `AutoBehavior { mode, scope, activated_at }` が3機能で共有されている。
- [x] 自動圧縮のマスタースイッチ `auto_compress_enabled` が実装されている。
- [x] 旧設定 JSON が読み込み時に移行され、保存後に旧キーが消える。
- [x] 初回フルスキャンでは3機能とも自動投入されない。
- [x] `ScanRoot`・監視・2回目以降のフルスキャンで新規写真が自動投入される。
- [x] 一括処理が `scope` に従い、`mode = Off` でも使え、マスタースイッチが
      オフのときだけ無効になる。
- [x] Discord の一括送信が件数確認付きで、未送信写真だけを対象にする。
- [x] AI の `Immediate` は未準備で積まず、`Waiting` は未準備でも積む。
- [x] AI カードに「設定を開く」ボタンと案内文がない。
- [x] 3カードの表示順が統一され、3セットアップページには一括処理がない。
- [x] 自動動作と一括処理がカード化されず、一括処理の対象範囲が2分割セグメントに
      なっている。両者の間に区切り線がある。
- [x] 確認ダイアログが件数だけを表示し、ヒント行・重複ラベル・空表示・
      出力先の注記がない。
- [x] Discord の非公開インスタンス警告が、カードにも同意ダイアログにもない。
- [x] #684: ダッシュボードが3機能の混合キューになり、圧縮専用キューがない。
- [x] #684: 機能詳細のキューが待機・実行待ち・実行中だけを表示する。
- [x] #684: 機能別の一時停止が設定に保存され、再起動後も維持される。
- [x] #684: 停止中は新規取得が止まり、実行中は完了し、自動投入は続く。
- [x] #684: 待機だけを行単位・一括で削除でき、削除後に再投入できる。
- [x] #684: Discord の送信履歴カードと「履歴を消去」がない。
- [x] #684: 失敗行がキューに出ず、通知モーダルに最新5件と再試行が出る。
- [x] i18n のカタログ・placeholder テストを含む全テストが通る。
- [x] 実装 PR が merge され、#682 と #684 が閉じる。
