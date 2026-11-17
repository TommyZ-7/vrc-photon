# AIタグ付けの表示条件 (#692 / #693) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 初回セットアップのAIタグ付けページから自動動作を外し、プラグイン未導入時は連携・自動のAIタグ付けスイッチを表示しない。

**Architecture:** 「プラグインが導入されているか」と「保存された有効化設定」から実効状態を計算する純粋関数を追加し、連携・自動の一覧ドットとカードの表示に使う。セットアップページは自動動作セグメントの描画ノードを削除するだけ。保存値・処理・キューは変更しない。

**Tech Stack:** Rust stable（edition 2024）、GPUI（gpui-kit 0.9.1）、cargo test

**Spec:** `docs/superpowers/specs/2026-11-17-issue-692-693-ai-tagging-visibility-design.md`

## Global Constraints

- 対象 Issue は #692 と #693。1つの実装 PR で両方を閉じる。
- 保存設定 `ai_tagging_enabled` の値は書き換えない。未導入時の無効化は表示時の計算だけ。
- 「未導入」の判定は `AiUiState::installed()` を使う。`ready()` では判定しない。
- 新しい翻訳キー、案内文、ボタン、区切り線は追加しない。
- コードとドキュメントを同じ PR に混ぜない。ドキュメントは docs PR で先に更新する。
- 実装は `fix/692-693-ai-tagging-visibility` ブランチの worktree で行う。
- 検証は `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`。

## Review Focus

- 未導入で保存値が true のとき、スイッチが出ず、一覧ドットが淡色で、本文が操作不可でも保存値は true のまま残る。
- プラグイン導入済みでモデル/probe が未準備のとき、スイッチは従来どおり表示され、保存値を反映する（`ready()` で隠さない）。
- セットアップ AIタグ付けページを 600×561 で開いたとき、日本語・英語のどちらでも紹介文とプレビューがはみ出さない。
- 起動直後の AI状態読み込み前でも panic せず、未導入として描画できる。
- 未導入で待機ジョブが残っていても、削除や開始を誤って実行できない（本文の操作ブロックが効く）。

---

### Task 1: 実効状態のヘルパーとテスト

**Files:**
- Modify: `crates/ui/src/app/ai.rs`（`ai_dashboard_view` の近く、テストモジュール）
- Test: `crates/ui/src/app/ai.rs` の `mod tests`

**Interfaces:**
- Produces: `pub(crate) fn effective_ai_tagging_enabled(configured: bool, installed: bool) -> bool`
  - `configured && installed` を返す。
  - Task 2 の一覧ドットとカードが使う。

- [ ] **Step 1: 失敗するテストを書く**

`mod tests` の `use super::{...}` に `effective_ai_tagging_enabled` を追加し、次のテストを足す。

```rust
#[test]
fn ai_tagging_is_disabled_while_the_plugin_is_not_installed() {
    assert!(!effective_ai_tagging_enabled(true, false));
    assert!(!effective_ai_tagging_enabled(false, false));
    assert!(effective_ai_tagging_enabled(true, true));
    assert!(!effective_ai_tagging_enabled(false, true));
}
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui ai_tagging_is_disabled_while_the_plugin_is_not_installed`

Expected: FAIL（`effective_ai_tagging_enabled` が見つからない）

- [ ] **Step 3: 最小限の実装を追加する**

`ai_dashboard_view` の近くに次を置く。

```rust
/// AI tagging only runs while the plugin is installed. The stored preference
/// is kept as-is so reinstalling the plugin restores it.
pub(crate) fn effective_ai_tagging_enabled(configured: bool, installed: bool) -> bool {
    configured && installed
}
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ui ai_tagging_is_disabled_while_the_plugin_is_not_installed`

Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/app/ai.rs
git commit -m "fix(ui): compute the effective AI tagging state (#693)"
```

---

### Task 2: 連携・自動のスイッチと状態ドット (#693)

**Files:**
- Modify: `crates/ui/src/ui/integrations.rs`
- Test: Task 1 のヘルパーテスト（UI 自体は描画テストを持たないため目視確認）

**Interfaces:**
- Consumes: `effective_ai_tagging_enabled(configured: bool, installed: bool) -> bool`
- Consumes: `App::ai.installed()` / `App::settings.ai_tagging_enabled`
- Produces: 未導入時のカードヘッダーから `Switch` を除いた表示

- [ ] **Step 1: 一覧の AI 行だけ実効状態を使う**

`integrations_list` の先頭で次を計算する。

```rust
let ai_enabled =
    effective_ai_tagging_enabled(self.settings.ai_tagging_enabled, self.ai.installed());
```

`items` 配列の `IntegrationsSection::AiTagging` の 4 番目（状態）を
`self.settings.ai_tagging_enabled` から `ai_enabled` に変える。他の行は変更しない。

- [ ] **Step 2: カードの実効状態と導入判定を計算する**

`ai_tagging_card` の先頭を次の値に変える。

```rust
let installed = self.ai.installed();
let enabled =
    effective_ai_tagging_enabled(self.settings.ai_tagging_enabled, installed);
```

`backend_section`、`body`、`integration_settings_body(enabled, body)` の構造は
そのまま使う。`enabled` が false になることで本文の淡色化と操作ブロックが
既存の仕組みで行われる。

- [ ] **Step 3: 未導入時だけヘッダースイッチを生成しない**

カードヘッダーの `Switch::new("integrations-ai-enabled")` の子を
`.when(installed, |this| { ... })` で包む。タイトルは常に表示する。
スイッチの `checked(enabled)`、`on_click` の `set_ai_tagging_enabled` は
現行のまま。インストール済みの見た目・動作は変えない。

- [ ] **Step 4: ビルドとテスト**

Run: `cargo test -p vrc-photon-ui && cargo build -p vrc-photon-ui`

Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/ui/integrations.rs
git commit -m "fix(ui): hide the AI tagging switch before the plugin is installed (#693)"
```

---

### Task 3: 初回セットアップの自動動作を削除 (#692)

**Files:**
- Modify: `crates/ui/src/ui/setup.rs`
- Test: `cargo build -p vrc-photon-ui` と目視確認（既存のページ遷移テストは変更しない）

**Interfaces:**
- Consumes: `App::settings.ai_tagging_auto` を使わなくなる
- Produces: `fn setup_ai(&self) -> Div`（`Context` 引数を削除）

- [ ] **Step 1: 自動動作セグメントの描画を削除する**

`setup_ai` の `v_flex()` から次の `.child(auto_mode_segmented(...))` ブロック
（`"setup-ai"`、`self.settings.ai_tagging_auto.mode`、`App::set_ai_auto_mode`）を
削除する。紹介文、BOOTH の案内、プレビューの子はそのまま残す。

- [ ] **Step 2: 不要になった `cx` 引数を削除する**

`setup_ai` のシグネチャを `fn setup_ai(&self) -> Div` に変え、呼び出し側を
`SetupPage::AiTagging => self.setup_ai().into_any_element(),` にする。

`crate::ui::auto_behavior::auto_mode_segmented` の import は Discord と
自動圧縮のセットアップページで使うため残す。

- [ ] **Step 3: ビルドとテスト**

Run: `cargo build -p vrc-photon-ui && cargo test -p vrc-photon-ui app::setup`

Expected: PASS

- [ ] **Step 4: コミット**

```bash
git add crates/ui/src/ui/setup.rs
git commit -m "fix(ui): drop the automatic behavior control from the AI setup page (#692)"
```

---

### Task 4: 全体検証と目視確認

**Files:**
- Verify only: `crates/ui/src/app/ai.rs`, `crates/ui/src/ui/integrations.rs`,
  `crates/ui/src/ui/setup.rs`

**Interfaces:**
- Consumes: Task 1〜3 の変更
- Produces: 実装 PR に貼る検証結果

- [ ] **Step 1: ローカル検証を実行する**

Run:

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Expected: PASS（既知の並列実行フレーク #647 が出た場合だけ単体再実行し、
失敗ではないことを確認して PR に記録する）

- [ ] **Step 2: 初回セットアップを目視確認する**

新しい設定ディレクトリでアプリを起動し、AIタグ付けページを開く。
自動動作のラベルと3分割セグメントがなく、紹介文の次にプレビューが来ること、
600×561 のパネルからはみ出さないことを確認する。日本語と英語の両方を見る。

- [ ] **Step 3: 連携・自動を目視確認する**

- プラグイン未導入: AIタグ付けカードのヘッダーにスイッチがなく、一覧ドットが
  淡色で、本文が淡色・操作不可。
- プラグイン導入済み: スイッチが表示され、ON/OFF が本文と一覧ドットに反映される。
- モデル未準備の導入済み状態でもスイッチが消えない。

- [ ] **Step 4: PR を作成する**

`fix/692-693-ai-tagging-visibility` から `main` へ PR を作成し、`bug` ラベルを付け、
本文に `Closes #692` と `Closes #693`、検証結果、目視確認の結果を書く。
CI が green になってからマージする。
