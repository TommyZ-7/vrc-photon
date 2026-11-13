# 圧縮設定を機能ごとに持たせ、プロファイルの役割フラグを廃止する Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 自動圧縮・クリップボード・Discord 送信が圧縮プロファイルを参照せず、それぞれ専用の圧縮設定だけで動作するようにする。プロファイルは手動圧縮専用にし、役割フラグを廃止する。

**Architecture:** `AppSettings` に自動圧縮用の `CompressionOptions`、クリップボード用の `ResizeSettings`、Discord 用の `CompressionOptions` を追加する。DB は schema v19 で `compression_jobs.origin` と `derivatives.origin` を追加し、`profile_id` を nullable にする。自動圧縮は `origin='auto'` のジョブとして投入し、処理時に現在の自動圧縮設定から合成したプロファイルを使う。表示用派生は `origin='auto'` のものだけを選ぶ。手動圧縮は常に `keep` で、表示用派生を切り替えない。

**Tech Stack:** Rust 2024, rusqlite, serde, gpui-kit 0.9, SQLite

**Spec:** https://github.com/TommyZ-7/vrc-photon/issues/622

**Mock:** `docs/ui-622-feature-compression-mockup.html`（Task 0。承認まで実装しない）

---

## Global Constraints

- 自動圧縮、クリップボード、Discord 送信は `CompressionProfile` を参照しない。プロファイルの編集・削除・並び替えで挙動が変わらない。
- `CompressionProfile` の役割フラグ `auto_enabled` / `clipboard` / `discord` をモデル・JSON・DB から廃止する。プロファイルは手動圧縮専用とする。
- 自動圧縮の出力先は元画像の親ディレクトリ直下の `compressed` に固定する。機能設定に出力先・サフィックスの項目を持たせない。
- 自動圧縮の既定値は「無劣化webp」相当（WebP ロスレス、リサイズなし、品質 82、速度 6）。
- クリップボードの設定はリサイズ項目（有効/無効、方式、幅、高さ、縦横比維持）だけを持つ。形式・画質・エンコード速度は持たない。出力は常に PNG。
- Discord の既定値は「discord用webp」相当（WebP ロスレス、2560×1440 の縮小のみ、品質 82、速度 6）。エンコード速度は保存値のみで UI に出さない。
- Discord とクリップボードは圧縮結果をファイルへ保存せず、派生ファイルとして登録しない。
- 手動圧縮は元画像を常に保持し、出力を派生ファイルとして登録するだけにする。手動圧縮ダイアログから保持／置換の選択とクリップボード出力を削除する。
- 写真が `replaced` になるのは、自動圧縮（`policy='replace'`）経由だけとする。手動ジョブが仮に `replace` を持っていても表示ファイルを切り替えない。
- DB schema は v19。旧バージョンの DB は移行せず、`Db::open` / `Db::open_read_only` がファイルを変更せず拒否する。既存ジョブの移行は扱わない。
- 新規・変更する UI 文言は `crates/i18n/locales/ja.yml` と `en.yml` の両方に追加する。
- UI 変更は Task 0 のモック承認後に実装する。モックとこの計画は docs-only PR とし、コード PR と混ぜない。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` と `scripts/check-docs-drift.sh`。
- 計画ファイルをコミットする場合、実行完了時にチェックボックスを `- [x]` にする。

## Review Focus

1. **プロファイル非依存**: 自動圧縮・クリップボード・Discord が、プロファイルが0件でも動作する。プロファイルの追加・削除・並び替え・出力先変更が3機能に影響しない。Task 3 / Task 4 / Task 5 のテストで固定する。
2. **自動圧縮の出力先**: 自動圧縮がプロファイルの `output_dir` を無視し、常に元画像の隣の `compressed/` へ書く。Task 3 のテストで固定する。
3. **手動圧縮の非置換**: 手動ジョブに `replace` が渡されても `photos.display_derivative_id` と `status` を変えない。Task 3 の防御テストと Task 7 で固定する。
4. **ファイルを残さない**: Discord とクリップボードが写真フォルダにファイルを作らず、`derivatives` 行も追加しない。Task 4 / Task 5 のテストで固定する。
5. **Discord のフォールバック**: 原画が 20 MiB を超える場合、または原画が置換・欠落している場合に、プロファイル派生ではなく Discord 設定または表示用派生を使う。Task 5 のテストで固定する。
6. **クリップボードの範囲**: クリップボード設定に形式・画質・速度が存在せず、リサイズだけが PNG に反映される。Task 4 のテストで固定する。
7. **schema v19**: v18 の DB を変更せず拒否し、新規 DB だけが v19 で初期化される。Task 2 のテストで固定する。

---

## File Structure

- `crates/core/src/model.rs`: `CompressionOrigin`、`ResizeSettings`、`CompressionOptions`、`CompressionProfile` のフラグ削除、`CompressionJob` / `Derivative` の `origin` と nullable `profile_id`。
- `crates/core/src/settings.rs`: `AppSettings` の3設定、既定値、正規化、serde テスト。
- `crates/core/src/compressor.rs`: `CompressionOptions` / `ResizeSettings` を受けるエンコード・リサイズ。
- `crates/core/src/db/schema.rs`: schema v19。
- `crates/core/src/db/compression.rs`: ジョブの `origin`、自動ジョブの一意性、プロファイル書き込み。
- `crates/core/src/db/photos.rs`: `add_derivative` の `origin` / nullable `profile_id`。
- `crates/core/src/discord.rs`: Discord 設定からの送信元選択。
- `crates/daemon/src/compression.rs`: 自動圧縮の設定解決、表示用派生の選択。
- `crates/daemon/src/scan.rs`: 自動投入のシグネチャ維持。
- `crates/daemon/src/deletion.rs`: 表示用派生の選択。
- `crates/daemon/src/discord.rs`: Discord 設定でのエンコード。
- `crates/daemon/src/lifecycle.rs`: 圧縮設定の再読込。
- `crates/ui/src/app/compression.rs`: 手動ダイアログ、クリップボード操作、機能設定の編集メソッド。
- `crates/ui/src/app/discord.rs`: Discord 送信ガードと送信投入。
- `crates/ui/src/ui/compression_settings.rs`（新規）: 3機能で共有する圧縮設定エディタ。
- `crates/ui/src/ui/integrations.rs`: 自動圧縮・クリップボードカード、一覧サマリ。
- `crates/ui/src/ui/discord.rs`: Discord カード、送信履歴の表示。
- `crates/ui/src/ui/compression.rs`: プロファイル一覧・編集、キュー行。
- `crates/ui/src/ui/dialogs.rs`: 手動圧縮ダイアログ。
- `crates/ui/src/ui/theme.rs`: フォーマット・サマリ・lossless 判定。
- `crates/i18n/locales/ja.yml`, `en.yml`, `crates/i18n/tests/placeholders.rs`: 文言。
- `docs/ui-622-feature-compression-mockup.html`（新規）: 実装前モック。
- `docs/adr/0020-feature-compression-settings.md`（新規）: 判断の記録。
- `docs/features.md`, `docs/ui.md`, `docs/data-model.md`, `docs/database-initialization.md`, `docs/architecture.md`, `docs/development.md`, `README.md`, `README.ja.md`: 仕様同期。

---

### Task 0: モック承認ゲート

**Files:**
- Create: `docs/ui-622-feature-compression-mockup.html`
- Modify: `docs/superpowers/plans/2026-11-13-issue-622-feature-compression-settings.md`（完了チェック）

**Interfaces:**
- Produces: 承認済みの UI モック。Task 1 以降はこのモックに従う。

- [x] **Step 1: モックを開いて確認する**

Run: `xdg-open docs/ui-622-feature-compression-mockup.html`
Expected: 自動圧縮・クリップボード・Discord の各カードに圧縮設定があり、手動圧縮ダイアログから保持／置換とクリップボード出力が消えている。

- [x] **Step 2: ユーザーの承認を得る**

承認前にコードを書き換えない。レイアウト・文言・項目の過不足を確認し、修正があればモックだけを更新する。

- [x] **Step 3: docs-only PR を出す**

```bash
git add docs/ui-622-feature-compression-mockup.html docs/superpowers/plans/2026-11-13-issue-622-feature-compression-settings.md
git commit -m "docs: add feature compression settings mock and plan"
git push -u origin docs/622-feature-compression-plan
gh pr create --title "docs: 圧縮設定の機能別化モックと計画 (#622)" --body "Issue #622 の実装前モックと計画。承認後に実装 PR を出します。"
```

Expected: CI（docs-only の fast-pass）が緑。ユーザーの承認を得たら Task 1 へ進む。

---

### Task 1: 圧縮オプションの型と設定既定値

**Files:**
- Modify: `crates/core/src/model.rs`
- Modify: `crates/core/src/settings.rs`
- Test: `crates/core/src/settings.rs`（`#[cfg(test)]` 内）

**Interfaces:**
- Produces:
  - `pub struct ResizeSettings { pub resize_enabled: bool, pub resize_mode: ResizeMode, pub width: u32, pub height: u32, pub maintain_aspect: bool }`
  - `impl Default for ResizeSettings`（無効、`DownscaleOnly`、3840×2160、縦横比維持）
  - `impl ResizeSettings { pub fn clipboard_default() -> Self; pub fn normalize(&mut self); }`
  - `pub struct CompressionOptions { pub format: CompressionFormat, pub quality: u8, pub lossless: bool, pub encode_speed: u8, #[serde(flatten)] pub resize: ResizeSettings }`
  - `impl CompressionOptions { pub fn auto_default() -> Self; pub fn discord_default() -> Self; pub fn normalize(&mut self); pub fn set_format(&mut self, format: CompressionFormat); pub fn to_profile(&self, id: &str, name: &str, output_dir: Option<String>, suffix: String) -> CompressionProfile; }`
  - `impl CompressionProfile { pub fn options(&self) -> CompressionOptions; }`
  - `CompressionFormat::supports_lossless(&self) -> bool`
  - `AppSettings.auto_compression: CompressionOptions`
  - `AppSettings.clipboard_compression: ResizeSettings`
  - `AppSettings.discord_compression: CompressionOptions`

- [x] **Step 1: 失敗するテストを書く**

`crates/core/src/settings.rs` のテストモジュールに追加する。

```rust
#[test]
fn legacy_settings_without_feature_compression_use_defaults() {
    let settings: AppSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.auto_compression, CompressionOptions::auto_default());
    assert_eq!(
        settings.clipboard_compression,
        ResizeSettings::clipboard_default()
    );
    assert_eq!(
        settings.discord_compression,
        CompressionOptions::discord_default()
    );
}

#[test]
fn feature_compression_defaults_match_the_builtin_profiles() {
    let auto = CompressionOptions::auto_default();
    assert_eq!(auto.format, CompressionFormat::Webp);
    assert!(auto.lossless);
    assert!(!auto.resize.resize_enabled);

    let discord = CompressionOptions::discord_default();
    assert_eq!(discord.format, CompressionFormat::Webp);
    assert!(discord.lossless);
    assert!(discord.resize.resize_enabled);
    assert_eq!((discord.resize.width, discord.resize.height), (2560, 1440));
    assert_eq!(discord.resize.resize_mode, ResizeMode::DownscaleOnly);
    assert_eq!(discord.encode_speed, 6);
}

#[test]
fn feature_compression_settings_normalize_out_of_range_values() {
    let mut options = CompressionOptions::auto_default();
    options.quality = 0;
    options.encode_speed = 99;
    options.resize.width = 0;
    options.resize.height = 999_999;
    options.normalize();
    assert_eq!(options.quality, 1);
    assert_eq!(options.encode_speed, 10);
    assert_eq!(options.resize.width, 1);
    assert_eq!(options.resize.height, 16_384);
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core legacy_settings_without_feature_compression_use_defaults`
Expected: `CompressionOptions` / `ResizeSettings` が未定義でコンパイル失敗。

- [x] **Step 3: 型と既定値を実装する**

`model.rs` に `ResizeSettings` と `CompressionOptions` を追加する。`CompressionOptions` の `resize` は `#[serde(flatten)]` で、プロファイル JSON と同じキー名になるようにする。

`settings.rs` の `AppSettings` に3フィールドを追加し、`Default` と `load` / `save` の正規化に組み込む。正規化は以下を保証する。

- `quality`: 1〜100
- `encode_speed`: 1〜10
- `width` / `height`: 1〜16384
- `resize_mode` は既存の `ResizeMode`

`CompressionOptions::to_profile` は現行の `CompressionProfile` フィールドへ写す。役割フラグが残っている間は `auto_enabled: false, clipboard: false, discord: false` を設定する（Task 6 で削除）。

- [x] **Step 4: テストを通す**

Run: `cargo test -p vrc-photon-core settings::tests`
Expected: PASS。既存の設定テストもすべて通る。

- [x] **Step 5: コミット**

```bash
git add crates/core/src/model.rs crates/core/src/settings.rs
git commit -m "feat: add per-feature compression options"
```

---

### Task 2: schema v19 とジョブ・派生の origin

**Files:**
- Modify: `crates/core/src/model.rs`
- Modify: `crates/core/src/discord.rs`
- Modify: `crates/core/src/db/schema.rs`
- Modify: `crates/core/src/db/compression.rs`
- Modify: `crates/core/src/db/photos.rs`
- Modify: `crates/core/src/db/tests.rs`
- Modify: `crates/daemon/src/compression.rs`
- Modify: `crates/daemon/src/tests.rs`
- Modify: `crates/ui/src/app/demo.rs`
- Modify: `crates/ui/src/app/live.rs`
- Modify: `crates/ui/src/ui/compression.rs`

**Interfaces:**
- Produces:
  - `pub enum CompressionOrigin { Auto, Manual }` with `as_str(self) -> &'static str`, `parse(value: &str) -> Self`
  - `CompressionJob { pub profile_id: Option<String>, pub origin: CompressionOrigin, ... }`
  - `Derivative { pub profile_id: Option<String>, pub origin: CompressionOrigin, ... }`
  - `Db::add_derivative(photo_id, path, format, width, height, file_size, profile_id: Option<&str>, origin: CompressionOrigin, policy: &str) -> Result<i64>`
  - schema v19: `compression_jobs.profile_id TEXT`, `compression_jobs.origin TEXT NOT NULL`, `derivatives.profile_id TEXT`, `derivatives.origin TEXT NOT NULL`, `compression_profiles` から `auto_enabled` 列を削除

- [x] **Step 1: 失敗する schema テストを書く**

`crates/core/src/db/tests.rs` を更新する。

```rust
#[test]
fn fresh_database_is_initialized_to_v19() {
    // 既存の fresh_database_is_initialized_to_v18 を置き換える。
    // user_version == 19、schema_migrations なしを確認する。
}

#[test]
fn v18_database_is_rejected_without_modification() {
    // user_version = 18 の DB を作り、Db::open が CoreError::Invalid を返し、
    // ファイルのバイト列と user_version が変わらないことを確認する。
}

#[test]
fn compression_job_origin_and_nullable_profile_roundtrip() {
    // origin=auto / profile_id=None と origin=manual / profile_id=Some("p") の
    // 2件を enqueue し、list_jobs で往復する。
}

#[test]
fn derivative_origin_and_nullable_profile_roundtrip() {
    // add_derivative に None + Auto と Some("p") + Manual を渡し、
    // derivatives_for_photo で往復する。
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core fresh_database_is_initialized_to_v19`
Expected: FAIL（現在のバージョンは 18、列も未定義）。

- [x] **Step 3: schema v19 とモデルを実装する**

`SCHEMA_VERSION` を `19` に上げ、`SCHEMA_LATEST` を更新する。

```sql
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
    finished_at TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    output_dir TEXT
);
CREATE INDEX idx_jobs_photo_profile ON compression_jobs(photo_id, profile_id);

CREATE TABLE compression_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    json TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
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
CREATE INDEX idx_derivatives_auto ON derivatives(photo_id) WHERE origin = 'auto';
```

`compression_jobs` の既存 INSERT / RETURNING / SELECT と `JOB_COLUMNS` / `row_to_job` を新しい列順に更新する。手動ジョブは `origin='manual'`、自動ジョブは `origin='auto'` を書く。`add_derivative` の呼び出し元をすべて新しい引数へ更新する。`Db::write_profile` は `auto_enabled` 列を書かないようにする（モデルのフィールドは Task 6 まで残る）。

- [x] **Step 4: テストを通す**

Run: `cargo test -p vrc-photon-core`
Expected: PASS。`fresh_database_is_initialized_to_v19` と v18 拒否テストが通る。

- [x] **Step 5: ワークスペースのコンパイルを通す**

Run: `cargo check --workspace`
Expected: PASS。`CompressionJob` / `Derivative` の構築箇所をすべて更新する。

- [x] **Step 6: コミット**

```bash
git add crates/core/src/model.rs crates/core/src/db crates/daemon crates/ui
git commit -m "feat: add schema v19 with compression origins"
```

---

### Task 3: 自動圧縮を専用設定だけで動かす

**Files:**
- Modify: `crates/core/src/compressor.rs`
- Modify: `crates/daemon/src/compression.rs`
- Modify: `crates/daemon/src/deletion.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Modify: `crates/daemon/src/tests.rs`
- Modify: `crates/core/src/db/schema.rs`（自動ジョブ一意インデックス）
- Modify: `crates/core/src/db/compression.rs`
- Modify: `crates/ui/src/app/compression.rs`
- Create: `crates/ui/src/ui/compression_settings.rs`
- Modify: `crates/ui/src/ui/mod.rs`
- Modify: `crates/ui/src/ui/integrations.rs`
- Modify: `crates/ui/src/ui/compression.rs`
- Modify: `crates/i18n/locales/ja.yml`, `en.yml`

**Interfaces:**
- Consumes: Task 1 の `CompressionOptions` / `ResizeSettings`、Task 2 の `CompressionOrigin`。
- Produces:
  - `pub(crate) fn preferred_display_derivative(derivatives: &[Derivative]) -> Option<&Derivative>`（`origin == Auto` のみ、存在するファイル、最大 id）
  - `Db::enqueue_auto_job_with_status(photo_id: i64, policy: &str, status: QueueStatus) -> Result<bool>`（`profile_id=NULL`, `origin='auto'`）
  - `FeatureCompressionTarget { Auto, Discord }`
  - `App::edit_feature_compression(target, edit, cx)`
  - `App::compression_options_editor(id_prefix, options, target, cx) -> AnyElement`
  - `App::resize_settings_editor(id_prefix, resize, cx) -> AnyElement`
  - `auto_compression_summary(enabled: bool, options: &CompressionOptions) -> String`

- [x] **Step 1: 失敗する自動圧縮テストを書く**

`crates/daemon/src/tests.rs` に追加する。

```rust
#[test]
fn auto_compression_enqueues_without_any_profiles() {
    // DB のプロファイルを空にし、auto_compress=true の daemon で写真を索引する。
    // origin=auto のジョブが1件でき、profile_id が NULL であることを確認する。
}

#[test]
fn auto_compression_always_writes_to_the_compressed_folder() {
    // 別の output_dir（例 "custom"）を持つプロファイルを保存してから自動圧縮を実行する。
    // 出力が <photo dir>/compressed/ にあり、custom/ にはファイルがないことを確認する。
}

#[test]
fn auto_replace_selects_the_auto_derivative() {
    // auto_compress_keep_original=false で自動圧縮を実行する。
    // photos.status == Replaced、display_derivative_id が origin=auto の派生を指す。
}

#[test]
fn reordering_profiles_does_not_change_the_auto_derivative() {
    // プロファイルを複数保存して並び替え、自動圧縮の表示用派生が変わらないことを確認する。
}
```

`crates/ui/src/ui/integrations.rs` のテストに追加する。

```rust
#[test]
fn auto_compression_summary_uses_the_auto_options() {
    let options = CompressionOptions::discord_default();
    assert!(auto_compression_summary(true, &options).contains("WebP"));
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon auto_compression_enqueues_without_any_profiles`
Expected: FAIL（現在は auto プロファイルが必要）。

- [x] **Step 3: 自動投入と処理を実装する**

- `enqueue_auto_jobs` から `db.list_profiles()` と `auto_profiles` を削除する。`auto_compress` は呼び出し側で判定済みなので、写真ごとに1件だけ `enqueue_auto_job_with_status` を呼ぶ。
- `enqueue_auto_job_with_status` は `profile_id` を受け取らず、`NULL` と `origin='auto'` を書く。`WHERE NOT EXISTS (photo_id, origin='auto')` で再投入を防ぐ。
- `SCHEMA_LATEST` に `CREATE UNIQUE INDEX idx_jobs_auto_once ON compression_jobs(photo_id) WHERE origin = 'auto';` を追加する。
- `process_decoded_job` を分岐する。
  - `CompressionOrigin::Auto`: `AppSettings::load` から `auto_compression` を取り、`to_profile("auto", "自動圧縮", Some("compressed".into()), String::new())` で合成する。`job.output_dir` は無視する。
  - `CompressionOrigin::Manual`: 従来どおり `profile_id` のプロファイルを読み、`job.output_dir` の上書きを適用する。
- `add_derivative` には `job.profile_id.as_deref()` と `job.origin` を渡す。
- 表示用派生の選択を `preferred_display_derivative(&derivatives)` に変え、`origin == Auto` かつ実在するファイルの最大 id を返す。`promote_display_derivative` と `deletion.rs` の呼び出しを更新する。
- `refresh_compression_settings` で `self.settings.auto_compression = settings.auto_compression;` をコピーする。
- `crates/ui/src/ui/compression_settings.rs` を作成する。既存のプロファイル編集 UI から形式・ロスレス・品質・リサイズの描画を移し、`FeatureCompressionTarget` に応じて `edit_feature_compression` を呼ぶ。クリップボード用に `resize_settings_editor` を用意する。
- 自動圧縮カードの「対象プロファイル」チェックボックスを `compression_options_editor("auto-compression", &options, FeatureCompressionTarget::Auto, cx)` に置き換える。`AppSettings` の自動圧縮設定をクローンしてから描画する。
- 出力先の注記をカード内に追加する（`integrations.compression.output_note.auto`）。
- キューの auto ジョブはプロファイル名ではなく `compression.queue.auto`（「自動圧縮」）を表示する。`profile_id=None` の manual ジョブは従来どおり `compression.unknown_profile`。

- [x] **Step 4: i18n を追加する**

`ja.yml` / `en.yml` に以下を追加する。

```yaml
integrations.compression.section: "圧縮設定"
integrations.compression.output_note.auto: "出力先は元画像の隣の compressed/ に固定されます。"
compression.queue.auto: "自動圧縮"
```

`integrations.auto.profiles*` と `integrations.summary.auto` の引数を新しいサマリに合わせて更新する。

- [x] **Step 5: テストを通す**

Run: `cargo test -p vrc-photon-daemon auto_`
Expected: PASS。
Run: `cargo test -p vrc-photon-ui auto_compression_summary_uses_the_auto_options`
Expected: PASS。

- [x] **Step 6: コミット**

```bash
git add crates/core/src/compressor.rs crates/core/src/db crates/daemon crates/ui crates/i18n
git commit -m "feat: run automatic compression from its own settings"
```

---

### Task 4: クリップボードをリサイズ設定だけで動かす

**Files:**
- Modify: `crates/core/src/compressor.rs`
- Modify: `crates/ui/src/app/compression.rs`
- Modify: `crates/ui/src/ui/dialogs.rs`
- Modify: `crates/ui/src/ui/integrations.rs`
- Modify: `crates/ui/src/ui/compression_settings.rs`
- Modify: `crates/i18n/locales/ja.yml`, `en.yml`

**Interfaces:**
- Consumes: Task 1 の `ResizeSettings`、Task 3 の `resize_settings_editor`。
- Produces:
  - `pub fn render_clipboard_png(source: &Path, resize: &ResizeSettings) -> Result<Vec<u8>>`
  - `clipboard_summary(enabled: bool, resize: &ResizeSettings) -> String`
  - `ManualOutputTarget` は `Folder` / `Profile` のみ（`Clipboard` を削除）
  - `ManualCompressionDialog` からクリップボード関連の分岐を削除

- [x] **Step 1: 失敗するテストを書く**

`crates/core/src/compressor.rs` のテストに追加する。

```rust
#[test]
fn clipboard_png_applies_resize_settings_only() {
    // 32x20 の画像と 8x8 の ResizeSettings を渡し、PNG の寸法が 8x8 になることを確認する。
    // 形式・品質・速度のフィールドが存在しないことは型で保証される。
}

#[test]
fn clipboard_render_writes_no_files() {
    // 一時ディレクトリに元画像だけを置き、render_clipboard_png 後に
    // ディレクトリ内のエントリ数が 1 のままであることを確認する。
}
```

`crates/ui/src/ui/integrations.rs` に追加する。

```rust
#[test]
fn clipboard_summary_uses_the_resize_settings() {
    let resize = ResizeSettings::clipboard_default();
    assert!(clipboard_summary(true, &resize).contains("2560"));
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core clipboard_png_applies_resize_settings_only`
Expected: FAIL（現在は `CompressionProfile` を受ける）。

- [x] **Step 3: 実装する**

- `render_clipboard_png` の引数を `&ResizeSettings` に変え、`resize_for_profile` を `resize_for_settings` として共通化する。
- `compress_selection_to_clipboard` は `self.settings.clipboard_compression` を直接使い、プロファイル検索・ダイアログ・`ManualOutputTarget::Clipboard` 分岐を削除する。写真1枚の検証と `clipboard_enabled` の検証は残す。
- `ManualOutputTarget` から `Clipboard` を削除し、`manual_output_available` は不要なら削除する。`ui/dialogs.rs` のクリップボード行、`clipboard_note`、`clipboard_target` の分岐を削除する。
- `clipboard_card` のプロファイル単一選択を `resize_settings_editor("clipboard-resize", &resize, cx)` に置き換える。`set_clipboard_profile` を削除する。
- クリップボードのサマリを `有効 ・ PNG ・ 2560×1440（縮小のみ）` のような内容にする。
- i18n から `integrations.clipboard.profiles_*`、`dialog.output.clipboard*`、`dialog.clipboard_note` を削除し、`integrations.clipboard.resize_note`（「クリップボードは PNG でコピーされます。設定はリサイズのみです。」）を追加する。
- `crates/i18n/tests/placeholders.rs` の `CALL_SITES` を更新する。

- [x] **Step 4: テストを通す**

Run: `cargo test -p vrc-photon-core clipboard_`
Expected: PASS。
Run: `cargo test -p vrc-photon-ui clipboard`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/core/src/compressor.rs crates/ui crates/i18n
git commit -m "feat: use resize-only settings for clipboard compression"
```

---

### Task 5: Discord 送信を専用設定で動かす

**Files:**
- Modify: `crates/core/src/discord.rs`
- Modify: `crates/core/src/model.rs`
- Modify: `crates/daemon/src/discord.rs`
- Modify: `crates/daemon/src/tests.rs`
- Modify: `crates/ui/src/app/compression.rs`
- Modify: `crates/ui/src/app/discord.rs`
- Modify: `crates/ui/src/ui/discord.rs`
- Modify: `crates/ui/src/ui/compression_settings.rs`
- Modify: `crates/i18n/locales/ja.yml`, `en.yml`

**Interfaces:**
- Consumes: Task 1 の `CompressionOptions`、Task 3 の `compression_options_editor`。
- Produces:
  - `pub fn choose_send_source(photo: &Photo, send_original: bool, options: &CompressionOptions) -> SendImageSource`
  - `SendImageSource::Encode { source_path: PathBuf, format: SendImageFormat }`（`profile_id` を削除）
  - `pub fn encode_for_send(source: &Path, options: &CompressionOptions) -> Result<EncodedImage>`
  - `discord_format_warning(options: &CompressionOptions) -> Option<String>`
  - `set_discord_profile` / `discord_profile_toggle` / `discord_profile_label` を削除

- [x] **Step 1: 失敗するテストを書く**

`crates/core/src/discord.rs` のテストを更新・追加する。

```rust
#[test]
fn discord_send_uses_the_discord_options_without_a_profile() {
    // send_original=false、CompressionOptions::discord_default() を渡す。
    // SendImageSource::Encode が返り、format が WebP であることを確認する。
}

#[test]
fn discord_replaced_photo_sends_the_display_derivative() {
    // status=Replaced、effective_path が存在する写真を渡す。
    // SendImageSource::File { source: SendSource::Derivative } が返る。
}

#[test]
fn discord_over_limit_original_falls_back_to_the_configured_format() {
    // 20 MiB 超の原画と send_original=true を渡す。
    // Encode が返り、format が設定の形式になる。
}
```

`crates/daemon/src/tests.rs` に追加する。

```rust
#[test]
fn discord_send_encodes_with_the_discord_settings_and_writes_no_file() {
    // プロファイルを空にし、discord_compression を設定して送信を実行する。
    // 送信ファイルが写真フォルダに残らず、derivatives が増えないことを確認する。
}
```

`crates/ui/src/ui/discord.rs` に追加する。

```rust
#[test]
fn discord_format_warning_uses_the_discord_options() {
    let avif = CompressionOptions {
        format: CompressionFormat::Avif,
        ..CompressionOptions::discord_default()
    };
    assert!(discord_format_warning(&avif).is_some());
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core discord_send_uses_the_discord_options_without_a_profile`
Expected: FAIL（現在はプロファイルを受ける）。

- [x] **Step 3: 実装する**

- `choose_send_source` から `profile` / `derivatives` を削除し、`options` を受ける。置換・欠落時は `photo.effective_path()` を `SendSource::Derivative` として返す。原画が 20 MiB 以下で `send_original` なら `Original`。それ以外は `Encode { source_path, format: options.format.into() }`。
- `encode_for_send` を `CompressionOptions` 受けに変更する。
- `build_photo_plan` は `db.list_profiles()` を呼ばず、`settings.discord_compression` を `PhotoPlan` に持たせる。
- `materialise_source` は `CompressionOptions` を受けて `encode_for_send` を呼ぶ。`PreparedRequest` の `profile_id` は常に `None` を渡す（`send_outbox.profile_id` 列は nullable のまま残す）。
- `SendImageSource::MissingProfile` と `SendErrorCode::ProfileMissing` を削除し、`send.error.profile_missing` を i18n から削除する。
- Discord カードの「連携用プロファイル」一覧を `compression_options_editor("discord-compression", &options, FeatureCompressionTarget::Discord, cx)` に置き換える。「原画を優先」の説明を「20 MiB を超える原画は Discord の圧縮設定で送信します」に更新する。
- `set_discord_profile`、`discord_profile_toggle`、`discord_profile_label` と関連テストを削除する。`discord_format_warning` は `CompressionOptions` を受ける形にする。
- i18n の `integrations.discord.profile.*` を削除し、`integrations.discord.compression.hint` を追加する。`integrations.discord.source.prefer_original_hint` を更新する。

- [x] **Step 4: テストを通す**

Run: `cargo test -p vrc-photon-core discord_`
Expected: PASS。
Run: `cargo test -p vrc-photon-daemon discord_`
Expected: PASS。
Run: `cargo test -p vrc-photon-ui discord_`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/core/src/discord.rs crates/core/src/model.rs crates/daemon crates/ui crates/i18n
git commit -m "feat: use dedicated compression settings for Discord"
```

---

### Task 6: プロファイルの役割フラグを廃止する

**Files:**
- Modify: `crates/core/src/model.rs`
- Modify: `crates/core/src/db/compression.rs`
- Modify: `crates/core/src/db/tests.rs`
- Modify: `crates/ui/src/app/compression.rs`
- Modify: `crates/ui/src/ui/compression.rs`
- Modify: `crates/ui/src/ui/integrations.rs`
- Modify: `crates/daemon/src/tests.rs`

**Interfaces:**
- Produces:
  - `CompressionProfile { id, name, enabled, format, quality, lossless, resize_enabled, resize_mode, width, height, maintain_aspect, output_dir, suffix, encode_speed }`
  - `CompressionProfile::defaults()` に役割フラグなし
  - `Db::save_profile` / `duplicate_profile` に clipboard / discord の一意性処理なし

- [x] **Step 1: 失敗するテストを書く**

`crates/core/src/db/tests.rs` の既存テストを更新する。

```rust
#[test]
fn default_profiles_have_no_role_flags() {
    let db = Db::open_in_memory().unwrap();
    let profiles = db.ensure_default_profiles().unwrap();
    assert_eq!(profiles.len(), 3);
    // 旧フラグのフィールドが存在しないことはコンパイルで保証される。
    assert_eq!(profiles[0].name, "無劣化webp");
}

#[test]
fn profile_json_ignores_removed_role_flags() {
    let legacy = r#"{
        "id":"p","name":"Legacy","enabled":true,
        "auto_enabled":true,"clipboard":true,"discord":true,
        "format":"webp","quality":82,"lossless":true,
        "resize_enabled":false,"resize_mode":"downscale_only",
        "width":3840,"height":2160,"maintain_aspect":true,
        "output_dir":null,"suffix":"","encode_speed":6
    }"#;
    let profile: CompressionProfile = serde_json::from_str(legacy).unwrap();
    assert_eq!(profile.name, "Legacy");
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core profile_json_ignores_removed_role_flags`
Expected: FAIL（`auto_enabled` などのフィールドが残っている）。

- [x] **Step 3: 実装する**

- `CompressionProfile` から `auto_enabled` / `clipboard` / `discord` を削除する。
- `CompressionProfile::default` と `defaults()` からフラグを削除する。`to_profile`（Task 1）のフラグ設定も削除する。
- `Db::write_profile` は `auto_enabled` を書かない（Task 2 で列は削除済み）。`save_profile` から `clear_other_clipboard_profiles` / `clear_other_discord_profiles` を削除する。`duplicate_profile` のフラグリセットを削除する。
- UI のプロファイル一覧から「自動／手動」バッジと「クリップボード」バッジを削除する。新規プロファイル作成時の `auto_enabled: false` を削除する。
- `integrations.rs` の一覧サマリから `auto_profiles` / `clipboard_profile` の集計を削除し、`auto_compression_summary(&settings.auto_compression)` / `clipboard_summary(&settings.clipboard_compression)` を使う。
- 旧テスト `ensure_default_profiles_is_idempotent` のフラグ assertion、`duplicate_profile_*_clipboard_flag`、`failed_profile_save_restores_the_previous_clipboard_flag`、`saving_a_clipboard_profile_clears_the_previous_one` を削除または新しい期待値へ置き換える。

- [x] **Step 4: テストを通す**

Run: `cargo test -p vrc-photon-core`
Run: `cargo test -p vrc-photon-ui`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/core crates/ui crates/daemon
git commit -m "feat: remove compression profile role flags"
```

---

### Task 7: 手動圧縮を非置換に固定する

**Files:**
- Modify: `crates/ui/src/app/compression.rs`
- Modify: `crates/ui/src/ui/dialogs.rs`
- Modify: `crates/ui/src/ui/theme.rs`
- Modify: `crates/daemon/src/compression.rs`
- Modify: `crates/daemon/src/tests.rs`
- Modify: `crates/i18n/locales/ja.yml`, `en.yml`
- Modify: `crates/i18n/tests/placeholders.rs`

**Interfaces:**
- Produces:
  - `pub(crate) fn manual_compression_policy() -> &'static str`（常に `"keep"`）
  - `ManualCompressionDialog` から `keep_original` を削除
  - `process_decoded_job` は `CompressionOrigin::Auto && policy == "replace"` のときだけ `complete_replacement` を呼ぶ

- [x] **Step 1: 失敗するテストを書く**

`crates/ui/src/app/compression.rs` に追加する。

```rust
#[test]
fn manual_compression_always_uses_keep() {
    assert_eq!(manual_compression_policy(), "keep");
}
```

`crates/daemon/src/tests.rs` に追加する。

```rust
#[test]
fn manual_job_with_replace_policy_does_not_change_the_display() {
    // origin=manual、policy=replace のジョブを実行する。
    // photos.status が Present のまま、display_derivative_id が NULL のまま、
    // derivative の policy が replace でも表示が切り替わらないことを確認する。
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon manual_job_with_replace_policy_does_not_change_the_display`
Expected: FAIL（現在は policy だけで置換する）。

- [x] **Step 3: 実装する**

- `confirm_manual_compression` は `manual_compression_policy()` を使い、`dialog.keep_original` を削除する。成功トーストから `policy` 引数を外す。
- `ManualCompressionDialog` から `keep_original` を削除し、`open_manual_compression*` の初期化を更新する。
- `ui/dialogs.rs` から「元画像」セクション、保持／ゴミ箱の行、関連する `can_confirm` / `confirm_label` の分岐を削除する。
- `process_decoded_job` の表示切り替え条件を `job.origin == CompressionOrigin::Auto && job.policy == "replace"` にする。手動ジョブの `add_derivative` は `policy="keep"` を記録する。
- i18n から `dialog.keep_original*`、`dialog.trash_original*`、`dialog.section.original`、`compression.policy.*` を削除し、`compression.enqueue.success` から `%{policy}` を外す。`crates/i18n/tests/placeholders.rs` の `CALL_SITES` を更新する。
- `compression_policy` は自動圧縮の待機解放で使うため残す。名前はそのままでよい。

- [x] **Step 4: テストを通す**

Run: `cargo test -p vrc-photon-daemon manual_`
Run: `cargo test -p vrc-photon-ui manual_compression_always_uses_keep`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/ui crates/daemon crates/i18n
git commit -m "feat: keep originals for every manual compression"
```

---

### Task 8: ドキュメント・i18n を同期して全体検証する

**Files:**
- Modify: `docs/features.md`
- Modify: `docs/ui.md`
- Modify: `docs/data-model.md`
- Modify: `docs/database-initialization.md`
- Modify: `docs/architecture.md`
- Modify: `docs/development.md`
- Modify: `README.md`
- Modify: `README.ja.md`
- Create: `docs/adr/0020-feature-compression-settings.md`
- Modify: `docs/adr/README.md`
- Modify: `docs/adr/0014-manual-compression-policy.md`
- Modify: `crates/i18n/locales/ja.yml`, `en.yml`
- Modify: `crates/i18n/tests/placeholders.rs`

- [x] **Step 1: ADR を追加する**

`docs/adr/0020-feature-compression-settings.md` を作成する。

```markdown
# ADR-0020: 圧縮設定を機能ごとに分離し、プロファイルを手動圧縮専用にする

- Status: Accepted
- Date: 2026-11-13
- Supersedes in part: ADR-0013, ADR-0014

## Context
自動圧縮・クリップボード・Discord がプロファイルの役割フラグを共有していたため、
プロファイルの編集・削除・並び替えが3機能の挙動を変えていた。

## Decision
- AppSettings に auto_compression / clipboard_compression / discord_compression を持つ。
- 自動圧縮の出力先は compressed/ 固定。クリップボードはリサイズのみの PNG。
- Discord は送信時に専用設定でエンコードし、ファイルを保存しない。
- プロファイルは手動圧縮専用。表示用派生は自動圧縮だけが作る。
- schema v19 で compression_jobs.origin と derivatives.origin を追加する。

## Consequences
- プロファイルの操作が3機能に影響しない。
- 自動圧縮はプロファイル0件でも動く。
- 手動圧縮は元画像を置換しない。
- 既存 v18 DB は作り直しが必要。

## Alternatives
- 非表示プロファイル行として保存する: プロファイルの意味が曖昧になるため不採用。
- クリップボードで形式・画質を反映する: OS のクリップボードが PNG 前提のため不採用。
```

`docs/adr/README.md` の一覧に 0020 を追加し、0014 の Status を「Superseded in part by ADR-0020」に更新する。`0014` の冒頭にも同じ注記を追加する。

- [x] **Step 2: 仕様ドキュメントを更新する**

- `docs/features.md` 3.2〜3.5: プロファイルは手動専用、自動圧縮は単一設定、出力固定、手動は常に保持、クリップボードはリサイズのみ、Discord は専用設定。
- `docs/features.md` Discord 節: 連携用プロファイルの説明を Discord 圧縮設定に置き換える。送信元は原画 → 表示用派生 → Discord 設定でエンコードの順。
- `docs/ui.md` 2.6: プロファイル一覧の自動／手動バッジを削除。手動ダイアログから保持／置換とクリップボード出力を削除。
- `docs/ui.md` 2.10: 自動圧縮カードは圧縮設定エディタ、クリップボードはリサイズのみ、Discord は圧縮設定エディタ。プロファイル選択の記述を削除。
- `docs/data-model.md`: v19 へ更新。`compression_profiles` の `auto_enabled` 削除、`compression_jobs` / `derivatives` の `origin` と nullable `profile_id`、`AppSettings` の3設定を追記。`user_version` 18→19 の全箇所を更新。
- `docs/database-initialization.md`: 現在のバージョンを v19、v18 を拒否、v1〜v18 は移行しない、新規 DB は v19。
- `docs/architecture.md`: v18 のみ受け付ける記述を v19 に更新。圧縮フローの「プロファイル」を「自動圧縮設定」に更新。
- `docs/development.md`: v1〜v17 を v1〜v18 に更新。
- `README.md` / `README.ja.md`: 「プロファイルを自動圧縮・クリップボード・Discord で共用する」記述を、機能ごとの設定に更新。

- [x] **Step 3: i18n の未使用キーを整理する**

`ja.yml` / `en.yml` から、削除した UI に対応するキー（`integrations.auto.profiles*`、`integrations.clipboard.profiles_*`、`integrations.discord.profile.*`、`send.error.profile_missing`、`dialog.output.clipboard*`、`dialog.keep_original*`、`dialog.trash_original*`、`dialog.section.original`、`dialog.clipboard_note`、`compression.policy.*`）を削除する。`crates/i18n/tests/placeholders.rs` の `CALL_SITES` を実際の呼び出しに合わせる。

- [x] **Step 4: ドキュメントドリフトを確認する**

Run: `scripts/check-docs-drift.sh`
Expected: `docs drift check OK (PROTOCOL_VERSION=..., SCHEMA_VERSION=19, ...)`

- [x] **Step 5: 全体検証**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: すべて PASS。

- [x] **Step 6: コミット**

```bash
git add docs README.md README.ja.md crates/i18n
git commit -m "docs: sync feature compression settings"
```

---

## 実行完了時の確認

- [x] 自動圧縮・クリップボード・Discord がプロファイルを参照していない。
- [x] プロファイルの編集・削除・並び替えが3機能に影響しない。
- [x] 自動圧縮の出力が常に `compressed/` にある。
- [x] 手動圧縮が表示ファイルを切り替えない。
- [x] Discord とクリップボードが写真フォルダにファイルを残さない。
- [x] schema v19 の新規 DB が作成でき、v18 DB が変更されず拒否される。
- [x] `cargo fmt` / `cargo clippy` / `cargo test --workspace` / `scripts/check-docs-drift.sh` が通る。
- [x] `docs/superpowers/plans/2026-11-13-issue-622-feature-compression-settings.md` のチェックボックスを `- [x]` に更新する。
