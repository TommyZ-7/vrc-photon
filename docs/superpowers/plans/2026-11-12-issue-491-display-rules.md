# 余計な表記の出し分け・集約・部分削除 (#491) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Issue #491 の「正常時は表示しない」「1か所へ集約」「部分削除」を、表示条件・
i18n キー・呼び出し側のコードごと実装する。異常時の文言と、#490 Q13 で全削除済みの
AI カード状態ラベルは変えない。

**Architecture:** 既存の画面構造を維持し、描画条件と文言だけを変える。正常時に隠す
表記はキーを残すもの（停止・失敗・未接続など）と、不要になるため削除するもの
（未圧縮バッジ、待機チップ、重複注記など）を分ける。0件の内訳は要素を組み立ててから
条件付きで追加し、空の詳細行は描画しない。

**Tech Stack:** Rust 2024, gpui-kit 0.9.1 (custom forks), SQLite, rust-i18n

**Spec:** `docs/superpowers/specs/2026-11-12-issue-491-display-rules-design.md`

**Mock:** `docs/ui-491-display-rules-mockup.html`

**Status:** Phase 0 進行中（設計・モック・計画は承認済み）

## Global Constraints

- 異常時の表示（停止・失敗・未接続など）の文言は変えない。
- AI カードの状態ラベル（準備不足・待機中・実行中・失敗）は再追加しない（#490 Q13）。
- エラーメッセージ、同意・ライセンス文面、破壊的操作の警告文は削除しない。
- 対象キーは `crates/i18n/locales/ja.yml` と `en.yml` の両方から削除する。
- `tr_args!` の登録があるキーは `crates/i18n/tests/placeholders.rs` も更新する。
- docs とコードは同じ PR に混ぜない。Phase 0 は docs-only PR、Phase 1 は実装 PR。
- 各 PR は最新の `main` から `git worktree` で切る。`main` への直接 push はしない。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- CI が green になるまで merge しない。CI は `ci-watch` の手順で監視する。
- 計画ファイルのチェックボックスは、その作業の完了時に `- [x]` に更新する。

## Review Focus

1. **最終同期**: 心拍60秒未満と心拍なしでは行が出ない。60秒以上では「N秒前」だけが出て、
   ラベル「最終同期」と重複しない。systemd では「N秒前 · systemd」。
2. **デーモンカード**: 正常時は「稼働中」バッジと「IPC 接続中」チップが出ない。停止・
   一時停止・未接続は現行どおり出る。
3. **AI カード**: 状態ラベルは出ない。件数は0件の項目が消え、全部0件なら件数行ごと消える。
4. **サイドバー**: 内訳の0件項目が出ない。内訳が空なら詳細行が消え、カード高は維持される。
5. **タグ別閾値**: 適用タイミングの注記はモーダル下部の1か所だけ。presence の
   リセットボタンは残る。
6. **バッジ**: ギャラリーとインスペクタで「圧縮済」だけが出る。「未圧縮」「重複」「圧縮版のみ」は出ない。
7. **AI タグ詳細**: ビューア詳細のモデル情報行が消え、チップのツールチップは確率だけになる。
8. **文言**: 空ライブラリ、人数、プロファイル名、取り込み、デーモン停止、ページ読み込み失敗が
   モックの最終形どおり。
9. **i18n**: ja / en のキー集合が一致し、削除キーの呼び出しが残っていない。

---

## Phase 0: 設計・モック・docs PR

**Files:**
- Create: `docs/superpowers/specs/2026-11-12-issue-491-display-rules-design.md`
- Create: `docs/superpowers/plans/2026-11-12-issue-491-display-rules.md`（このファイル）
- Create: `docs/ui-491-display-rules-mockup.html`
- Modify: `docs/ui.md`

**Interfaces:**
- Produces: 承認済みの設計・モック・計画、更新済み `docs/ui.md`

- [x] **Step 1: `docs/ui.md` を更新する**

更新箇所:

- §2.3 サイドバー: 圧縮カードの内訳は0件の項目を出さず、内訳が空なら詳細行を出さない。
- §2.4 写真: カードの圧縮状態バッジは「圧縮済」だけ出し、未圧縮・重複・圧縮版のみでは
  出さない。
- §2.8 ダッシュボード: デーモンカードは「稼働中」バッジと「IPC 接続中」チップを正常時に
  出さない。最終同期は心拍が60秒以上古いときだけ「N秒前」で出す。AI カードの件数は
  0件の項目を出さない。デーモン停止の注記は「デーモン停止中です」だけにする。
- §2.9 設定: ライブラリ保存の通知は「保存しました。」に統一する。AI タグ付けの説明は
  設定側に出さない。タグ別閾値の適用タイミングはモーダル下部の注記だけにする。
- §5.1 アップデート: 状態チップは待機のときだけ出さない。
- §5 同期表示: 最終同期はダッシュボードのデーモンカードに、滞っているときだけ出す。

- [x] **Step 2: Plannotator で設計とモックを提示する**

```bash
plannotator annotate \
  docs/superpowers/specs/2026-11-12-issue-491-display-rules-design.md \
  docs/ui-491-display-rules-mockup.html \
  docs/superpowers/plans/2026-11-12-issue-491-display-rules.md \
  --gate --json
```

Expected: `decision: "approved"`。Q1〜Q6 の推奨案と文言の最終形を確認する。
注釈が返った場合は設計・モック・計画へ反映して再提示する。

- [ ] **Step 3: docs PR を作る**

```bash
git add docs/ui.md docs/ui-491-display-rules-mockup.html \
  docs/superpowers/specs/2026-11-12-issue-491-display-rules-design.md \
  docs/superpowers/plans/2026-11-12-issue-491-display-rules.md
git commit -m "docs: 余計な表記の出し分け・集約・部分削除 #491 の設計・モック・計画"
git push -u origin docs/491-display-rules
gh pr create --title "docs: #491 余計な表記の出し分け・集約・部分削除の設計・モック" \
  --body "Refs #491" --label documentation
```

- [ ] **Step 4: CI green を確認して merge する**

`ci-watch` の手順で `CI` を監視する。docs-only のため Release の bundle は走らない。
green 後 `gh pr merge <n> --merge`、`main` へ戻って pull し、docs 用 worktree を削除する。

---

## Phase 1: 実装 PR

### Task 1: 実装用 worktree とベースライン

**Files:**
- None（作業ディレクトリのみ）

- [ ] **Step 1: 最新の `main` から worktree を作る**

```bash
git fetch origin
git worktree add ../vrc-photo-manager-491 -b feat/491-display-rules origin/main
```

- [ ] **Step 2: worktree でベースラインを確認する**

```bash
cargo test --workspace
```

Expected: PASS。失敗する場合は環境の問題を先に解消する。

### Task 2: i18n キーの削除・追加・変更

**Files:**
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`
- Modify: `crates/i18n/tests/placeholders.rs`

**Interfaces:**
- Produces: 実装で使う最終キー集合
- Produces: 削除キーのプレースホルダ登録の除去

- [ ] **Step 1: 削除するキー**

ja / en の両方から次を削除する:

`gallery.badge.uncompressed`, `gallery.badge.duplicate`, `gallery.badge.compressed_only`,
`photos.badge.uncompressed`,
`settings.ai.thresholds.presence_hint`, `settings.ai.thresholds.next_run`,
`settings.ai.thresholds.applies_now`, `settings.ai.hint`,
`sidebar.status.waiting.detail`, `sidebar.status.running.detail`,
`sidebar.status.failed.detail`, `sidebar.status.idle.detail`,
`dialog.trash_note`, `photos.ai.probability_tooltip`,
`settings.library.save_hint`, `settings.library.saved_rescan`,
`settings.library.saved_restart`, `update.phase.idle`,
`update.phase.checking`, `update.phase.up_to_date`, `update.phase.downloading`,
`update.phase.ready`, `update.phase.applying`, `update.phase.error`.

Q1 の決定どおり、`daemon.sync.ipc` と `daemon.sync.stopped` も削除する。

- [ ] **Step 2: 追加するキー**

- `sidebar.status.item.running`: `実行中 %{count}` / `Running %{count}`
- `sidebar.status.item.failed`: `失敗 %{count}` / `Failed %{count}`

- [ ] **Step 3: 値を変更するキー**

設計の「文言の最終形」表どおりに変更する:
`daemon.sync.age`, `daemon.sync.age_systemd`, `compression.name_hint`,
`search.ai.people_hint`, `gallery.empty`, `app.empty_library`,
`import.structure.default_hint`, `dashboard.errors.daemon_stopped`,
`photos.page_load_failed_body`。

- [ ] **Step 4: `placeholders.rs` を更新する**

- `photos.ai.probability_tooltip` の登録を削除する。
- `sidebar.status.running.detail` と `sidebar.status.waiting.detail` の登録を削除する。
- `sidebar.status.item.running` / `sidebar.status.item.failed` の登録を追加する。
- `daemon.sync.age` / `age_systemd` の登録は残す。

- [ ] **Step 5: i18n テストを確認する**

```bash
cargo test -p vrc-photon-i18n
```

Expected: PASS（ja/en パリティ、プレースホルダ、ソースキー）。

### Task 3: 最終同期とデーモンカードの出し分け

**Files:**
- Modify: `crates/ui/src/app/daemon.rs`
- Modify: `crates/ui/src/ui/dashboard.rs`

**Interfaces:**
- Produces: `daemon_sync_label(...) -> Option<String>`
- Produces: 正常時に「稼働中」バッジと「IPC 接続中」チップを出さないカード

- [ ] **Step 1: 停滞判定の定数を追加する**

同期表示専用の定数（例: `DAEMON_STALE_SECS = 60`）を追加する。`daemon_liveness` の
20秒判定は変えない（定数化する場合も値は20のまま）。

- [ ] **Step 2: `daemon_sync_label` を `Option<String>` にする**

- `age >= DAEMON_STALE_SECS`: `Some("N秒前")`、systemd なら `Some("N秒前 · systemd")`。
- `age < DAEMON_STALE_SECS` または `None`: `None`。
- `daemon.sync.ipc` / `daemon.sync.stopped` の分岐を削除する。

- [ ] **Step 3: デーモンカードを条件付き描画にする**

- 状態バッジは停止中・一時停止中のみ `child` する。
- IPC チップは未接続のみ `child` する。
- 最終同期の `meta_row` は `Some` のときだけ `child` する。
- 「IPC」と「監視」の `meta_row` は現行どおり残す。

- [ ] **Step 4: `daemon.rs` のテストを更新する**

- `Some(59)` は `None`。
- `Some(60)` は `Some("60秒前")`、systemd は `Some("60秒前 · systemd")`。
- `None` は `None`。
- `daemon_liveness` の既存テストは変えない。

### Task 4: AI カードの件数行

**Files:**
- Modify: `crates/ui/src/ui/dashboard_ai.rs`

**Interfaces:**
- Produces: 0件の項目を描画しない件数行

- [ ] **Step 1: 件数行を条件付きで組み立てる**

`waiting`, `pending + processing`, `failed` のそれぞれについて、0より大きいときだけ
`ai_count` を `child` する。3項目すべて0なら件数行自体を `child` しない。

- [ ] **Step 2: 状態ラベルを追加しないことを確認する**

`dashboard_ai.rs` のヘッダーにはタイトル以外を追加しない。`dashboard.ai.state.*` の
再追加も行わない。

### Task 5: サイドバーの内訳表示

**Files:**
- Modify: `crates/ui/src/ui/shell.rs`

**Interfaces:**
- Produces: 0件の内訳を出さない `compression_card_status`

- [ ] **Step 1: 内訳を組み立てる**

- 未実行あり: headline は現行どおり。detail は `running > 0` と `failed > 0` の項目を
  `" · "` で連結する。両方0なら空文字。
- 実行中あり: detail は `failed > 0` のときだけ「失敗 N」。
- 失敗あり: detail は空文字。
- 空: detail は空文字。

- [ ] **Step 2: headline キーは残す**

`sidebar.status.waiting.headline` / `running.headline` / `failed.headline` /
`idle.headline` は現行のまま使う。

- [ ] **Step 3: テストを更新する**

`compression_card_status_prioritizes_waiting_running_and_failed` の期待値を更新する:

- `(6, 5, 2)`: `"実行中 1 · 失敗 2"`
- `(3, 0, 0)`: detail 空
- `(0, 0, 2)`: detail 空
- `(0, 0, 0)`: detail 空

### Task 6: アップデートの状態チップ

**Files:**
- Modify: `crates/ui/src/update_view.rs`

**Interfaces:**
- Produces: 待機ではチップを描画しないアップデートカード

- [ ] **Step 1: `UpdatePhase::badge` を `Option` にする**

`Available(_) => Some(...)`、それ以外は `None`。チップ専用の
`update.phase.checking` / `up_to_date` / `downloading` / `ready` / `applying` / `error`
はカタログから削除する。

- [ ] **Step 2: カードのヘッダーを条件付きにする**

チップが `Some` のときだけ `self.badge(...)` を `child` する。エラーは本文の
メッセージで示し、チップは出さない。

### Task 7: 設定の保存通知・AI 説明・閾値モーダル

**Files:**
- Modify: `crates/ui/src/ui/settings.rs`
- Modify: `crates/ui/src/ui/ai_thresholds.rs`

**Interfaces:**
- Produces: `settings.library.saved` に統一した保存通知
- Produces: 適用タイミングの注記が1か所だけの閾値モーダル

- [ ] **Step 1: ライブラリ保存の通知を統一する**

`request_daemon_command` の成否分岐で文面を変えず、成功トーストは
`settings.library.saved` にする。`save_hint` の注記ノードを削除する。再スキャン要求の
失敗ログ（`tracing::warn!`）は残す。

- [ ] **Step 2: 設定の AI 説明を削除する**

`settings.ai.hint` の `div` を削除する。タイトルとカードの動作は変えない。

- [ ] **Step 3: 閾値モーダルを整理する**

- presence カードの `next_run` 表示を削除する。
- `presence_hint` の `div` を削除する。
- `applies_now` の注記を削除し、presence のリセットボタンは
  `h_flex().justify_end()` で右寄せにして残す。
- モーダル下部の `apply_note` は残す。

### Task 8: 圧縮状態バッジ

**Files:**
- Modify: `crates/ui/src/ui/gallery.rs`
- Modify: `crates/ui/src/ui/library.rs`

**Interfaces:**
- Produces: 未圧縮ではバッジを出さないカードとインスペクタ

- [ ] **Step 1: ギャラリーカードの条件を変える**

`show_status && compressed` のときだけバッジを描画する。バッジは常に「圧縮済」。
`duplicate` / `replaced` はバッジ描画に使わなくなるため、カード内の変数と
`gallery.badge.duplicate` / `gallery.badge.compressed_only` を削除する。

- [ ] **Step 2: インスペクタのバッジを変える**

`compressed` のときだけ「圧縮済」バッジを出す。`else` の「未圧縮」を削除する。

### Task 9: AI タグのツールチップとモデル情報行

**Files:**
- Modify: `crates/ui/src/app/ai.rs`
- Modify: `crates/ui/src/ui/library.rs`

**Interfaces:**
- Produces: 確率だけのツールチップ
- Produces: `AiMetaView` から `foot` を除いた AI タグ節

- [ ] **Step 1: ツールチップを `photos.ai.probability` に統一する**

`ai_tag_view_with_context` と `ai_tag_views_with_context` を削除し、`ai_tag_view` /
`ai_tag_views` を本番でも使う。ツールチップは常に `photos.ai.probability` で組み立てる。
`format_tagged_at` が不要になれば削除する。

- [ ] **Step 2: `AiMetaView.foot` を削除する**

`AiMetaCache::view` から `foot` の構築を削除し、`library.rs` の `foot` 受け取りと
描画ブロックを削除する。

- [ ] **Step 3: `AiMetaKey` を整理する**

`tagged_at` / `model_version` をキーから外し、`photo_id` / `language` / `revision` だけに
する。`AiMetaKey::new` の呼び出し側とテストを更新する。

- [ ] **Step 4: テストを更新する**

- `ai_tag_chip_shows_probability`: ツールチップに確率が入り、モデル名・時刻が入らない。
- `ai_meta_cache_*`: 新しいキーで選択・言語・リビジョンの変化だけを検証する。
- `format_tagged_at` のテストは関数削除に合わせて削除する。

### Task 10: 残りの文言・注記

**Files:**
- Modify: `crates/ui/src/ui/dialogs.rs`
- Modify: `crates/ui/src/ui/search.rs`（キーのみ）
- Modify: `crates/ui/src/ui/compression.rs`（キーのみ）
- Modify: `crates/ui/src/ui/import.rs`（キーのみ）
- Modify: `crates/ui/src/ui/gallery.rs`（空表示のキーのみ）
- Modify: `crates/ui/src/app/live.rs` / `crates/ui/src/app.rs`（空表示のキーのみ）
- Modify: `crates/ui/src/ui/dashboard.rs`（デーモン停止のキーのみ）

- [ ] **Step 1: `dialog.trash_note` を削除する**

手動圧縮ダイアログの注記ノードを削除し、元画像の選択肢（`keep_row` / `trash_row`）は
そのまま残す。

- [ ] **Step 2: 残りのキーを設計の最終形に合わせる**

`compression.name_hint`, `search.ai.people_hint`, `gallery.empty`,
`app.empty_library`, `import.structure.default_hint`,
`dashboard.errors.daemon_stopped`, `photos.page_load_failed_body` の表示が
設計どおりになることを確認する。コード側の変更が必要なのは文言だけで、構造は変えない。

### Task 11: テストと目視確認

**Files:**
- None

- [ ] **Step 1: ローカル検証を実行する**

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Expected: PASS。

- [ ] **Step 2: `--demo` で目視確認する**

ダッシュボード（正常・停止・一時停止・最終同期が古い状態）、AI カード（0件混在）、
サイドバー（待機・実行中・失敗・空）、設定（アップデート待機、ライブラリ保存、
AI 説明）、タグ別閾値、ギャラリー（圧縮済・未圧縮・重複・圧縮版のみ）、インスペクタ、ビューア、
検索、取り込み、手動圧縮ダイアログを確認する。

### Task 12: 実装 PR とマージ

**Files:**
- None

- [ ] **Step 1: PR を作る**

```bash
git add crates docs
git commit -m "feat(ui): 余計な表記の出し分け・集約・部分削除 (#491)"
git push -u origin feat/491-display-rules
gh pr create --title "feat(ui): #491 余計な表記の出し分け・集約・部分削除" \
  --body "Closes #491" --label enhancement
```

- [ ] **Step 2: CI green を確認して merge する**

`ci-watch` の手順で `CI` を監視する。green 後 `gh pr merge <n> --merge`、`main` へ戻って
pull し、実装用 worktree を削除する。

- [ ] **Step 3: 計画ファイルのチェックボックスを完了にする**

Phase 0 と Phase 1 の全項目を `- [x]` にし、完了状態を docs へ反映する。
