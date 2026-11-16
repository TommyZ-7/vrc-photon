# ヘッダーバッジの削除 (#666 / #667) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ダッシュボードの圧縮キューと「連携・自動」の送信履歴から、ヘッダーの件数・方針バッジだけを削除する。

**Architecture:** 表示ノードと呼び出し側の翻訳キーだけを削除する。ヘッダーのタイトル、伸縮スペーサー、一括操作ボタン、行の仮想リストはそのまま残す。振る舞い・DB・設定は変えない。

**Tech Stack:** Rust 2024, gpui-kit 0.9.1, SQLite（変更なし）, rust-i18n

**Spec:** `docs/superpowers/specs/2026-11-16-issue-666-667-header-badges-design.md`

**Mock:** `docs/ui-666-667-header-badges-mockup.html`

**Status:** Phase 0 進行中（モック承認待ち）

## Global Constraints

- 削除のみ。新しいバッジ・文言・アイコン・設定・代替の件数表示を追加しない。
- 圧縮キューは 4 バッジ（未完了 / 未実行 / 自動方針 / 失敗）、送信履歴は 1 バッジ（待機）を削除する。
- ヘッダーのタイトル、ボタン、表示条件、高さ、余白は変えない。
- キューの行と送信履歴の行は変えない。
- 未使用になる `compression.queue.*` の 5 キーを ja / en と `placeholders.rs` から削除する。
  `send.status.waiting` は残す。
- docs とコードは同じ PR に混ぜない。Phase 0 は docs-only PR、Phase 1 は実装 PR。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- CI が green になるまで merge しない。CI は `ci-watch` の手順で監視する。
- 各 PR は最新の `main` から `git worktree` で切る。`main` への直接 push はしない。
- 計画ファイルのチェックボックスは、その作業の完了時に `- [x]` に更新する。

## Review Focus

1. 圧縮キューのヘッダーに 4 バッジが残っていない。タイトルと 2 ボタンだけになる。
2. 送信履歴のヘッダーに「待機 N」が残っていない。タイトルと 2 ボタンだけになる。
3. 一括操作ボタンの表示条件が変わっていない（待機 0 件、再試行可能 0 件）。
4. キューの行・履歴の行の状態バッジ、失敗理由、再試行、日時が変わっていない。
5. ja / en のキーと `placeholders.rs` の登録が同時に消え、i18n テストが通る。
6. 1080×700 + サイドバー展開でヘッダーが崩れない。

---

## Phase 0: モック・設計・docs

**Files:**
- Create: `docs/ui-666-667-header-badges-mockup.html`
- Create: `docs/superpowers/specs/2026-11-16-issue-666-667-header-badges-design.md`
- Create: `docs/superpowers/plans/2026-11-16-issue-666-667-header-badges.md`（このファイル）
- Modify: `docs/ui.md`

- [x] **Step 1: モックを作る**

Before / After で圧縮キューのヘッダーと送信履歴のヘッダーを示す。狭い幅（約 640 px）の
After ヘッダーも示し、ボタンだけが右に残ることを確認できるようにする。

- [x] **Step 2: `docs/ui.md` を実装に合わせて更新する**

- §2.8 ダッシュボード: 「キュー上部に「未完了 N」「未実行 N」「失敗 N」のバッジを出し、
  `waiting` があるときは現在の「自動: 元画像を保持 / 置換」と「待機をすべて圧縮」を表示する。」
  のバッジ部分を削除する。
- 送信履歴: 「ヘッダーに最新100件を表示する旨、待機件数のバッジ、…」を
  「ヘッダーにタイトル、「待機をすべて送信」、「履歴を消去」を置く。」へ直す。

- [x] **Step 3: docs PR を作る**

`docs: 圧縮キューと送信履歴のヘッダーバッジ削除モック (#666, #667)`。
ラベルは `enhancement`。承認を得るまで Phase 1 は開始しない。

---

## Phase 1: 実装

**Files:**
- Modify: `crates/ui/src/ui/compression.rs`
- Modify: `crates/ui/src/ui/discord.rs`
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`
- Modify: `crates/i18n/tests/placeholders.rs`

- [ ] **Step 1: 圧縮キューのヘッダーバッジを削除する**

`queue_panel` のヘッダーから次の 4 ブロックを削除する。

```rust
.when(self.active_job_count > 0, |this| { /* 未完了 N */ })
.when(self.waiting_job_count > 0, |this| { /* 未実行 N */ })
.when(self.waiting_job_count > 0, |this| { /* 自動: 元画像を保持 / 置換 */ })
.when(self.failed_job_count > 0, |this| { /* 失敗 N */ })
```

タイトル、`div().flex_1()`、`release-waiting-jobs`、`retry-all-failed-jobs` は残す。

- [ ] **Step 2: 圧縮キューの翻訳キーを削除する**

`ja.yml` と `en.yml` から `compression.queue.active` / `.waiting` / `.auto_keep` /
`.auto_replace` / `.failed` を削除する。`placeholders.rs` の `CALL_SITES` から
`compression.queue.active` / `.failed` / `.waiting` を削除する。

- [ ] **Step 3: 送信履歴のヘッダーバッジを削除する**

`send_history_card` のヘッダーから `when(waiting > 0, ...)` のバッジブロックを削除する。
`waiting` は「待機をすべて送信」の無効判定で使い続けるため、変数は残す。

- [ ] **Step 4: ローカル検証**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] **Step 5: 目視確認**

`gnome-wayland-visual-qa` の手順でアプリを起動し、1080×700 + サイドバー展開で
圧縮キューと送信履歴のヘッダーを確認する。待機あり・失敗あり・待機 0 件の各状態を見る。

- [ ] **Step 6: 実装 PR を作る**

`feat(ui): remove the compression queue and send history header badges (#666, #667)`。
本文に `Closes #666` と `Closes #667` を書く。ラベルは `enhancement`。
CI green を確認して merge し、`main` を pull する。

---

## 完了条件

- [ ] 圧縮キューのヘッダーがタイトルと一括操作ボタンだけになる。
- [ ] 送信履歴のヘッダーがタイトルと 2 ボタンだけになる。
- [ ] 行内の状態バッジ・失敗理由・再試行・日時が変わらない。
- [ ] i18n のカタログ・placeholder テストを含む全テストが通る。
- [ ] 実装 PR が merge され、#666 と #667 が閉じる。
