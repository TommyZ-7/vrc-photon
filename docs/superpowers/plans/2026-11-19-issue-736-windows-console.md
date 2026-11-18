# Issue #736 Windows コンソール非表示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Windows で UI と daemon をエクスプローラーから起動したときにコンソールウィンドウを表示させない。

**Architecture:** `vrc-photon` と `vrc-photon-daemon` の各バイナリクレートルートに `#![cfg_attr(windows, windows_subsystem = "windows")]` を追加し、Windows では GUI サブシステムでリンクする。ロジックの変更はなく、既に同じ属性を持つ `vrc-photon-updater` と指定方法を揃える。

**Tech Stack:** Rust stable (edition 2024)

**Spec:** https://github.com/TommyZ-7/vrc-photon/issues/736

## Global Constraints

- 変更対象は `crates/ui/src/main.rs` と `crates/daemon/src/main.rs` のみ。
- `vrc-photon-mcp` は stdio サーバーのため変更しない。
- `vrc-photon-updater` は対応済みのため変更しない。
- 属性は Windows では debug / release を問わず常に適用する（updater と同じ）。
- リリース操作（タグ作成・Release 公開）は行わない。

## Review Focus

- エクスプローラー起動時に UI のコンソールが表示されないこと。属性はクレートルート先頭に置く必要がある。
- UI から spawn される daemon も単独でコンソールを開かないこと。daemon 側にも同じ属性が必要。
- Windows のコンソールから daemon の `--help` / `--print-data-dir` を直接実行した場合、標準出力が見えなくなる。これは Issue の指定どおり許容する。
- Linux / macOS のビルドとテストに影響しないこと。`cfg_attr(windows, ...)` により他プラットフォームでは展開されない。
- Windows CI (`ipc-windows`) の workspace check が通ること。リンク時のサブシステム選択は Windows ビルドでのみ確定する。

---

### Task 1: UI と daemon を Windows GUI サブシステムでビルドする

**Files:**
- Modify: `crates/ui/src/main.rs:1`
- Modify: `crates/daemon/src/main.rs:1`
- Test: Windows CI `ipc-windows` job（`cargo check --workspace --all-targets --locked`）

**Interfaces:**
- Consumes: なし
- Produces: `vrc-photon.exe` / `vrc-photon-daemon.exe` が Windows GUI サブシステムでリンクされる

- [x] **Step 1: UI にサブシステム属性を追加する**

`crates/ui/src/main.rs` の先頭（`//!` ドキュメントコメントより前）に次を追加する。

```rust
#![cfg_attr(windows, windows_subsystem = "windows")]
```

- [x] **Step 2: daemon にサブシステム属性を追加する**

`crates/daemon/src/main.rs` の先頭に同じ属性を追加する。

```rust
#![cfg_attr(windows, windows_subsystem = "windows")]
```

- [x] **Step 3: ローカル検証を実行する**

Run:

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Expected: すべて成功。Linux では属性が無効なため挙動は変わらない。

- [x] **Step 4: コミットする**

```bash
git add docs/superpowers/plans/2026-11-19-issue-736-windows-console.md
git commit -m "docs: add the issue 736 implementation plan"
git add crates/ui/src/main.rs crates/daemon/src/main.rs
git commit -m "fix: hide the Windows console for the UI and daemon"
```

## After Implementation

PR に `bug` ラベルを付け、`ipc-windows` job を含む全チェックが green になることを確認する。実機のコンソール非表示確認はリリース後に Windows 環境で行う。
