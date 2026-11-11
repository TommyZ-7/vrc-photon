# db / daemon モジュール再編 + schema 初期化統合 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `crates/core/src/db.rs` と `crates/daemon/src/lib.rs` の巨大モジュールをドメイン単位に再編し、#554 で schema 初期化を最新版の統合 SQL に置き換える。

**Architecture:**
- #548 は 2 PR に分ける。PR1 で `db.rs` を `db/` 配下のドメインモジュールへ、PR2 で `daemon/lib.rs` をワーカー単位のモジュールへ分割する。
- 公開 API は維持する。`Db` は公開 facade として残し、複雑なドメインは内部 repository 構造体へ委譲する。
- daemon は `Daemon` のライフサイクルと、scan / logs / compression / thumbnails / hash / deletion のワーカーを分離する。
- #554 は新規 DB を v18 の統合 schema で 1 回だけ初期化する。既存 DB は v18 のみ受け付け、v1〜v17 は明示エラーで開かずファイルを変更しない。

**Tech Stack:** Rust 2024, rusqlite/SQLite, cargo test, cargo clippy.

**Spec:** GitHub issues #548, #554 とユーザー回答（2026-11-12）。

## 決定事項

1. #548 は db → daemon の 2 PR。Issue は 2 本目で close する。
2. #548 は物理移動だけでなく内部設計も再編する。`Db` の公開シグネチャは変えない。
3. 行数（2,000 行以下）は受け入れ条件にしない。構造の明確さを優先する。
4. #554 は新規 DB 用の統合 SQL を採用する。既存 DB の移行コードは削除する。
5. v18 の DB はそのまま開く。v1〜v17 は「このビルドでは移行できない」と明示エラーを返し、ファイルを変更しない。
6. #554 でログフィルタの typo を修正し、テスト専用の thumbnail メソッドを `#[cfg(test)]` にする。
7. ローカル検証は対象クレートの `cargo test -p` と `cargo fmt --all -- --check`。workspace 全体の clippy/test は CI に任せる。

## Global Constraints

- `vrc_photon_core::db::Db` と `vrc_photon_core::Db` の公開 API を維持する。
- 既存のテストを回帰の基準にする。移動時にテストを削除しない。
- DB のトランザクション境界、ロック順序、`complete_replacement` の原子性を変えない。
- schema 変更は #554 の 1 PR に閉じる。#548 では schema の意味を変えない。
- UI 変更はないためモックは不要。

## Review Focus

- `db/` 分割後も `Db` の公開メソッドが全クレートから同じ型で呼べるか。
- daemon のワーカー分離後も、DB ロックを保持したままファイル I/O やコールバックをしないか。
- 新規 DB が v18 の全テーブル・索引・生成列を持ち、`user_version=18` になるか。
- v17 DB を開いたとき、エラーを返しつつ `user_version` とファイル内容が変わらないか。
- `vrc_photonn` の typo を直した後、UI/daemon のログフィルタテストが新しい文字列を期待するか。

---

## Task 0: 計画の docs-only PR

**Files:**
- Create: `docs/superpowers/plans/2026-11-12-db-daemon-modularization.md`

- [ ] PR を作成し、ユーザーの承認を得る。
- [ ] 承認後にマージし、PR1 の worktree を最新 main から作る。

---

## Task 1: `db/` モジュールの骨格と schema の移動（PR1）

**Files:**
- Delete: `crates/core/src/db.rs`（内容は移動）
- Create: `crates/core/src/db/mod.rs`
- Create: `crates/core/src/db/schema.rs`
- Modify: `crates/core/src/lib.rs`（必要なら `pub use`）

**Interfaces:**
- Produces: `pub struct Db`, `pub enum DbProfile`, `pub struct PhotoScanState`, `pub struct PhotoIndexSnapshot` は `db/mod.rs` から再エクスポートする。
- `Db` の `conn` は private のまま。サブモジュール用に `pub(super) fn conn(&self) -> &Connection` と `pub(super) fn now(&self) -> String` を用意する。
- `schema.rs` は `SCHEMA_VERSION`、`SCHEMA_V1`、各 `SCHEMA_V*`、`migrate` を一時的に保持する（#554 で置き換える）。

**Steps:**
- [x] `db.rs` を `db/mod.rs` に移し、`mod schema;` を追加する。
- [x] schema 定数を `db/schema.rs` に移す。`migrate` 系関数は #554 で置き換えるまで `db/mod.rs` に残す。
- [x] `Db::open` / `open_in_memory` / `open_read_only` / `configure` / `path` を `mod.rs` に残す。
- [x] `cargo check -p vrc-photon-core` と `cargo test -p vrc-photon-core db` で公開 API の互換性を確認する。
- [x] コミット: `refactor: #548 db モジュールの骨格を作る`

---

## Task 2: photos ドメインの抽出（PR1）

**Files:**
- Create: `crates/core/src/db/photos.rs`
- Modify: `crates/core/src/db/mod.rs`

**Interfaces:**
- `Db` の photo CRUD、photo context、players、derivatives、photo index、text search、random sample、stats、filter 構築、`PHOTO_SELECT` / `PHOTO_FROM` / row mapper を `photos.rs` に移す。
- 複雑な読み取りは `pub(super) struct Photos<'a> { conn: &'a Connection }` にまとめ、`Db` の公開メソッドは従来どおり `self.photos().by_id(id)` のように委譲する。
- 他ドメインが使う photo row 取得は `pub(super) fn photo_by_id_in(conn: &Connection, id: i64) -> Result<Option<Photo>>` として公開する。

**Steps:**
- [x] photo 系の `impl Db` ブロックと free function を移動する。
- [x] 参照側の `use` を `db::photos` に合わせる。`db::Db` の公開シグネチャは変えない。
- [x] 既存の photo_index / search / stats テストを `photos.rs` の `#[cfg(test)]` または `db/tests/photos.rs` に移す。
- [x] `cargo test -p vrc-photon-core photo` と `cargo test -p vrc-photon-core search` を実行する。
- [x] コミット: `refactor: #548 photos ドメインを分離する`

---

## Task 3: sessions / worlds / log imports の抽出（PR1）

**Files:**
- Create: `crates/core/src/db/sessions.rs`
- Create: `crates/core/src/db/world_names.rs`
- Modify: `crates/core/src/db/mod.rs`

**Interfaces:**
- worlds, sessions, session_players, log imports、world name lookup を移動する。
- `Db` の公開メソッドは維持する。
- session summary と photo context の結合は `sessions.rs` 内の `pub(super)` helper に閉じる。

**Steps:**
- [x] sessions / worlds / session_players の CRUD と集計を移動する。
- [x] log_imports の読み書きを `sessions.rs` に移す。
- [x] world_name_lookups を `world_names.rs` に移す。
- [x] 既存テストは `db/tests.rs` に移動し、`cargo test -p vrc-photon-core` で全回帰を確認する。
- [x] コミット: `refactor: #548 sessions と world_names を分離する`

---

## Task 4: tags / compression / jobs / send の抽出（PR1）

**Files:**
- Create: `crates/core/src/db/tags.rs`
- Create: `crates/core/src/db/compression.rs`
- Create: `crates/core/src/db/send.rs`
- Modify: `crates/core/src/db/mod.rs`

**Interfaces:**
- tags / photo_tags を `tags.rs`。
- compression_profiles / compression_jobs / derivatives の job 側を `compression.rs`。
- send_outbox / send_history を `send.rs`。
- 公開メソッドは `Db` に残す。

**Steps:**
- [x] 各ドメインの CRUD、集計、原子的クレームを移動する。
- [x] 圧縮 job の claim / complete / fail のトランザクション境界を変えない。
- [x] 既存テストは `db/tests.rs` に移動し、`cargo test -p vrc-photon-core` で全回帰を確認する。
- [x] コミット: `refactor: #548 tags・compression・send を分離する`

---

## Task 5: deletion / daemon / AI / settings / search の抽出（PR1）

**Files:**
- Create: `crates/core/src/db/deletion.rs`
- Create: `crates/core/src/db/daemon.rs`
- Create: `crates/core/src/db/ai.rs`
- Create: `crates/core/src/db/settings.rs`
- Create: `crates/core/src/db/search.rs`
- Modify: `crates/core/src/db/mod.rs`

**Interfaces:**
- deletion_requests、削除の原子クレーム、`complete_replacement` を `deletion.rs` に移す。`complete_replacement` の公開シグネチャは変えない。
- daemon_errors / daemon_commands を `daemon.rs`。
- plugin_installations / plugin_state / photo_ai / ai_tag_* / tagging_jobs を `ai.rs`。
- app_settings を `settings.rs`。
- text search / suggestions / random search のうち photos 以外の検索条件を `search.rs` に整理する（photos の index 本体は `photos.rs` に残す）。
- `schema.rs` 以外から SQL 定数を直接参照しない。

**Steps:**
- [x] deletion / daemon / AI / settings / search を移動する。
- [x] `Db::complete_replacement` の原子性テストを移動し、そのまま通す。
- [x] 既存テストは `db/tests.rs` に移動し、`cargo test -p vrc-photon-core` で全回帰を確認する。
- [x] コミット: `refactor: #548 deletion・AI・settings を分離する`

---

## Task 6: DB テストの整理と PR1 の最終検証

**Files:**
- Create: `crates/core/src/db/tests.rs`
- Modify: `crates/core/src/db/mod.rs`

**Ruling:** 8,000 行超の monolithic test module は `db/tests.rs` へ移動し、`db/mod.rs` から分離した。ドメイン別の更なるテスト分割は、共有 helper の依存が多く行数制限も受け入れ条件ではないため、PR2 以降の follow-up とする。

**Steps:**
- [x] 既存テストを `db/tests.rs` に移動し、`#[path = "tests.rs"] mod tests;` で読み込む。
- [x] `cargo fmt --all -- --check`
- [x] `cargo test -p vrc-photon-core`（562 passed / 1 ignored）
- [x] `cargo clippy -p vrc-photon-core --all-targets -- -D warnings`
- [x] 全 workspace の `cargo check` を CI で確認する。
- [x] PR を作成し、`Closes #548` は付けずに `Refs #548` とする。
- [x] コミット: `refactor: #548 db のテストを分離する`

---

## Task 7: daemon の骨格と scan の分離（PR2）

**Files:**
- Modify: `crates/daemon/src/lib.rs`
- Create: `crates/daemon/src/lifecycle.rs`
- Create: `crates/daemon/src/scan.rs`

**Interfaces:**
- `Daemon` 構造体、起動・停止・設定再読込・ステータスを `lifecycle.rs` に移す。`lib.rs` は `mod` 宣言と再エクスポートだけにする。
- scan loop と `ScanObserver`、relink を `scan.rs` に移す。`Daemon` からは `scan::run(...)` を呼ぶ。
- ワーカーの依存（`Arc<Mutex<Db>>`, `AtomicBool`, チャネル）は構造体フィールドにまとめ、フリーパラメータの増加を避ける。

**Steps:**
- [x] `DaemonStatus`、`LocalizedError`、`Daemon`、`Drop` を `lifecycle.rs` に移す。
- [x] scan 関連の関数と `ScanObserver` を `scan.rs` に移す。
- [x] `cargo check -p vrc-photon-daemon`
- [x] 既存の scan テストを移動し、`cargo test -p vrc-photon-daemon scan` を実行する。
- [x] コミット: `refactor: #548 daemon のライフサイクルと scan を分離する`

---

## Task 8: daemon の logs / sessions の分離

**Files:**
- Create: `crates/daemon/src/logs.rs`
- Modify: `crates/daemon/src/lib.rs`

**Interfaces:**
- `LogTail`、`LogPass`、ログ取り込み、line budget、session 終了判定を `logs.rs` に移す。
- `read_new_lines` 系の純粋関数も `logs.rs` に置く。

**Steps:**
- [x] log ingestion と session 更新を移動する。
- [x] 既存テストを移動し、`cargo test -p vrc-photon-daemon logs` を実行する。
- [x] コミット: `refactor: #548 daemon の log ingestion を分離する`

---

## Task 9: daemon の compression / thumbnails / hash / deletion の分離

**Files:**
- Create: `crates/daemon/src/compression.rs`
- Create: `crates/daemon/src/thumbnails.rs`
- Create: `crates/daemon/src/hash.rs`
- Create: `crates/daemon/src/deletion.rs`
- Modify: `crates/daemon/src/lib.rs`

**Interfaces:**
- compression worker / batch / decode / publish を `compression.rs`。
- thumbnail budget / prune / loop / spec packing を `thumbnails.rs`。
- hash loop と `retry_revision` を `hash.rs`。
- deletion loop / execute / cancel tagging を `deletion.rs`。
- 各モジュールは `pub(super)` の `run` か `Worker` 構造体を公開し、`Daemon` は起動時にそれらを spawn する。

**Steps:**
- [x] 各ワーカーを移動し、DB ロックを保持したままファイル I/O しない構造を維持する。
- [x] 既存テストを移動し、`cargo test -p vrc-photon-daemon compression`、`thumbnail`、`hash`、`deletion` を実行する。
- [x] コミット: `refactor: #548 daemon のワーカーを分離する`

---

## Task 10: daemon の status / IPC / tray / テスト整理と PR2 の最終検証

**Files:**
- Create: `crates/daemon/src/status.rs`
- Modify: `crates/daemon/src/ipc_server.rs`, `crates/daemon/src/tray.rs`, `crates/daemon/src/lib.rs`

**Steps:**
- [x] status snapshot / supervisor label / IPC operation mapping を `status.rs` に移す。
- [x] `lib.rs` に残った helper を適切なモジュールへ移す。
- [x] monolithic test module をドメイン別に分割する。
- [x] `cargo fmt --all -- --check`
- [x] `cargo test -p vrc-photon-daemon`
- [x] `cargo clippy -p vrc-photon-daemon --all-targets -- -D warnings`
- [x] PR を作成し、`Closes #548` を付ける。
- [x] コミット: `refactor: #548 daemon のテストを整理する`

---

## Task 11: #554 新規 DB の統合 schema と旧 DB の明示エラー（PR3）

**Files:**
- Modify: `crates/core/src/db/schema.rs`
- Modify: `crates/core/src/db/mod.rs`
- Create: `docs/database-initialization.md`

**Interfaces:**
- `SCHEMA_LATEST`: v18 の全テーブル・索引・生成列・制約を含む 1 本の SQL。
- `schema::initialize(conn: &Connection) -> Result<()>`: `SCHEMA_LATEST` を 1 トランザクションで実行し、`user_version=18` にする。
- `Db::open`:
  - `user_version == 0` かつユーザーテーブルがない → `initialize`。
  - `user_version == 18` → そのまま開く。
  - `user_version` が 1〜17 → `CoreError::Invalid` で「schema vX はこのビルドでは移行できない。バックアップして新しいライブラリを作成する」と返す。ファイルは変更しない。
  - `user_version > 18` → 既存の「新しいビルドが必要」エラーを維持する。
- `open_read_only` も v18 以外を拒否する。
- `schema_migrations` テーブルと `migrate_v*` は削除する。

**Steps:**
- [ ] 現行 v18 DB の `sqlite_master` から `SCHEMA_LATEST` を作る。
- [ ] 新規 DB 初期化と v18 既存 DB の両方で同じ schema になることを確認する。
- [ ] v17 DB を seed し、open がエラーを返し、`user_version` とファイル内容が変わらないテストを追加する。
- [ ] 移行専用テスト（v1→v18 など）を削除または新規初期化テストへ置き換える。
- [ ] `cargo test -p vrc-photon-core db` を実行する。
- [ ] コミット: `refactor: #554 schema 初期化を統合 SQL に置き換える`

---

## Task 12: #554 ログ typo とテスト専用メソッドの cfg(test) 化

**Files:**
- Modify: `crates/ui/src/main.rs`
- Modify: `crates/ui/src/logging.rs`
- Modify: `crates/daemon/src/logging.rs`
- Modify: `crates/ui/src/state/thumbnail.rs`

**Steps:**
- [ ] `vrc_photonn=debug` を `vrc_photon=debug` に直す。
- [ ] UI / daemon のログフィルタテストの期待値を更新する。
- [ ] `ThumbnailCache` の `generation`, `begin`, `insert_if_generation`, `fail`, `fail_if_generation` に `#[cfg(test)]` を付け、`#[allow(dead_code)]` を外す。
- [ ] 本番コードからの参照がないことを確認する。
- [ ] `cargo test -p vrc-photon-ui logging`、`thumbnail`、`cargo test -p vrc-photon-daemon logging` を実行する。
- [ ] コミット: `chore: #554 ログフィルタとテスト専用コードを整理する`

---

## Task 13: #554 ドキュメントと PR3 の最終検証

**Files:**
- Modify: `docs/database-initialization.md`
- Modify: `docs/release.md`（必要な場合のみ）
- Modify: `crates/core/src/db/schema.rs` の doc comment

**Steps:**
- [ ] 新規 DB の初期化手順、v18 のみサポートすること、旧 DB のエラーとバックアップ手順を文書化する。
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test -p vrc-photon-core db`
- [ ] `cargo test -p vrc-photon-ui logging thumbnail`
- [ ] `cargo test -p vrc-photon-daemon logging`
- [ ] `cargo clippy -p vrc-photon-core -p vrc-photon-ui -p vrc-photon-daemon --all-targets -- -D warnings`
- [ ] PR を作成し、`Closes #554` を付ける。
- [ ] コミット: `docs: #554 DB 初期化と旧 schema の扱いを文書化する`

---

## Execution order

1. Task 0（この計画 PR）を承認・マージ。
2. PR1: Task 1〜6。`Refs #548`。CI green 後にマージ。
3. PR2: Task 7〜10。`Closes #548`。CI green 後にマージ。
4. PR3: Task 11〜13。`Closes #554`。CI green 後にマージ。

## Risks

- `Db` の公開 API を壊すと UI / daemon / MCP が同時に壊れる。各タスクで `cargo check -p vrc-photon-core` だけでなく、依存クレートのコンパイルを CI で確認する。
- テスト移動時に helper の可視性が変わり、テストだけがコンパイルエラーになる。テスト用 helper は `#[cfg(test)] pub(super)` に閉じる。
- #554 の旧 DB 拒否は破壊的変更である。エラー時にファイルを変更しないことをテストで固定し、docs にバックアップ手順を明記する。
- daemon のワーカー分離でロック順序が変わり得る。`xmp_read_happens_without_the_db_lock` などの既存バリアテストを維持する。
