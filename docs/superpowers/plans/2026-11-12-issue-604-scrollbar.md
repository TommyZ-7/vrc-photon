# スクロールバーのトラック非表示と取り付け方式の統一 (#604) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 標準スクロールバーのトラックを全テーマで非表示にし、設定・連携の直接配置を App の `ScrollHandle` を維持した wrapper 方式へ揃える。

**Architecture:** gpui-kit のテーマ投影が使う `theme.colors.scrollbar` を透明にすることで、全画面の標準スクロールバーからトラックを消す。設定・連携は外側の非スクロールコンテナへ `.vertical_scrollbar(&handle)` を付け、8 px ガターと直接配置を削除する。圧縮キューの仮想リストだけは `VirtualList` の制約により直接配置の例外として残す。写真グリッドの年レールは変更しない。

**Tech Stack:** Rust stable (edition 2024), gpui-kit 0.9.1, GPUI Fast, SQLite。

**Spec:** `docs/superpowers/specs/2026-11-12-issue-604-scrollbar-design.md`

**Mock:** `docs/ui-604-scrollbar-mockup.html`（2026-11-12 承認済み）

## Global Constraints

- UI の変更範囲は、標準スクロールバーのトラック非表示と、設定・連携の取り付け方式だけ。写真グリッドの年レールは一切変更しない。
- サムの色・幅・角丸・フェード時間は変更しない。
- スクロールバーの 16 px トラック領域と当たり判定は変更しない。
- 設定・連携の `ScrollHandle` は App に保持したままとし、画面遷移後のスクロール位置を維持する。
- 圧縮キューの `VirtualListScrollHandle` と仮想リスト方式は変更しない。
- gpui-kit の依存更新・fork は行わない。
- リリース操作（タグ、Release 公開）は行わない。
- ブランチは `feat/604-scrollbar`。main への直接 push はしない。

## Review Focus

- テーマ変更後にトラックが再表示されないこと。`Theme::update` のトークン再調整が `colors.scrollbar` を上書きしないかを、ダーク・ライト・パステルで確認する。
- 8 px ガター削除後にスクロールバーが右端からずれないこと。外側ラッパーとスクロール領域の境界が一致するかを設定・連携で確認する。
- 設定・連携のスクロール位置が画面遷移後に保持されること。`overflow_y_scrollbar()` へ置き換えて新しいハンドルを作っていないかを diff で確認する。
- 圧縮キューのサムが表示され、ドラッグ・ホイールでスクロールできること。直接配置の例外を誤って wrapper 化していないかを確認する。
- ダイアログ本体と入れ子リストの両方でトラックが消えること。内側だけ・外側だけの取りこぼしを確認する。
- 写真グリッドの年レールが表示され、クリック・ドラッグでジャンプできること。`year_rail.rs` に diff がないことを確認する。

---

### Task 1: 標準スクロールバーのトラックを透明にする

**Files:**
- Modify: `crates/ui/src/ui/palette.rs:845-995`
- Test: `crates/ui/src/ui/palette.rs` の `mod tests`

**Interfaces:**
- Consumes: `gpui_kit::transparent_black() -> Hsla`
- Produces: `fn scrollbar_track_color() -> Hsla`（palette モジュール内。`apply_component_theme` とテストだけが使う）

- [ ] **Step 1: 失敗するテストを書く**

`crates/ui/src/ui/palette.rs` の `mod tests` に追加する。

```rust
#[test]
fn scrollbar_track_is_transparent() {
    assert_eq!(scrollbar_track_color().a, 0.0);
}
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui scrollbar_track_is_transparent`

Expected: `cannot find function scrollbar_track_color in this scope` でコンパイルエラー。

- [ ] **Step 3: ヘルパーと割り当てを実装する**

`crates/ui/src/ui/palette.rs` の `use gpui_kit::{...}` に `Hsla` を追加する。

```rust
use gpui_kit::{Anchor, App, Hsla, px, rgb, rgba, transparent_black};
```

`impl Palette` の直前（`with_alpha` の近く）に追加する。

```rust
/// Track colour for standard scrollbars. Transparent in every theme so only
/// the thumb paints; the track keeps its layout width and hit area.
fn scrollbar_track_color() -> Hsla {
    transparent_black()
}
```

`apply_component_theme` の次の行を置き換える。

```rust
theme.colors.scrollbar = rgb(palette.border_strong).into();
```

置き換え後:

```rust
theme.colors.scrollbar = scrollbar_track_color().into();
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ui scrollbar_track_is_transparent`

Expected: `test ui::palette::tests::scrollbar_track_is_transparent ... ok`

- [ ] **Step 5: コミットする**

```bash
git add crates/ui/src/ui/palette.rs
git commit -m "feat: スクロールバーのトラックを透明にする"
```

---

### Task 2: 設定画面を wrapper 方式へ変更する

**Files:**
- Modify: `crates/ui/src/ui/settings.rs:9-11`（import）
- Modify: `crates/ui/src/ui/settings.rs:71-84`（外側 `h_flex`）
- Modify: `crates/ui/src/ui/settings.rs:627-639`（8 px ガター削除）

**Interfaces:**
- Consumes: `ScrollableElement::vertical_scrollbar(&self.settings_scroll)`（gpui-kit 0.9.1）
- Produces: なし（画面内部の変更）

- [ ] **Step 1: import を置き換える**

次の行を削除する。

```rust
use gpui_kit::component::scroll::Scrollbar;
```

次の行を追加する。

```rust
use gpui_kit::component::scroll::ScrollableElement as _;
```

- [ ] **Step 2: 外側 `h_flex` にスクロールバーを付ける**

`settings_screen` の外側 `h_flex()` の `.items_stretch()` の直後に追加する。

```rust
.vertical_scrollbar(&self.settings_scroll)
```

変更後の先頭は次の形になる。

```rust
h_flex()
    .relative()
    .flex_1()
    .min_h_0()
    .items_stretch()
    .vertical_scrollbar(&self.settings_scroll)
    .child(
        v_flex()
            .id("settings-sections")
            // ...
```

- [ ] **Step 3: 8 px ガターの直接配置を削除する**

`settings_screen` の末尾にある次のブロックを削除する。

```rust
.child(
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(px(8.))
        .child(Scrollbar::vertical(&self.settings_scroll)),
)
```

- [ ] **Step 4: コンパイルを確認する**

Run: `cargo check -p vrc-photon-ui`

Expected: `Finished` で終わり、`Scrollbar` の未使用 import や型エラーがない。

- [ ] **Step 5: 実機で設定画面を確認する**

`--demo` でアプリを起動し、設定画面を開く。次を確認する。

- 右端にトラックが表示されず、サムだけが表示される。
- サムのホバーで幅が 6 px から 8 px に変わる。
- 設定画面を下へスクロールし、別画面へ移動して戻ったときに位置が保持される。
- 画面幅を狭くしても、バーが右端からずれない。

- [ ] **Step 6: コミットする**

```bash
git add crates/ui/src/ui/settings.rs
git commit -m "refactor: 設定画面のスクロールバーを取り付けラッパーへ移す"
```

---

### Task 3: 連携・自動画面を wrapper 方式へ変更する

**Files:**
- Modify: `crates/ui/src/ui/integrations.rs:7`（import）
- Modify: `crates/ui/src/ui/integrations.rs:117-140`（`detail` の `h_flex` と 8 px ガター）

**Interfaces:**
- Consumes: `ScrollableElement::vertical_scrollbar(&self.integrations_scroll)`（gpui-kit 0.9.1）
- Produces: なし（画面内部の変更）

- [ ] **Step 1: import から `Scrollbar` を外す**

次の行を置き換える。

```rust
use gpui_kit::component::scroll::{ScrollableElement as _, Scrollbar};
```

置き換え後:

```rust
use gpui_kit::component::scroll::ScrollableElement as _;
```

- [ ] **Step 2: `detail` の `h_flex` にスクロールバーを付ける**

`detail` の `.items_stretch()` の直後に追加する。

```rust
.vertical_scrollbar(&self.integrations_scroll)
```

変更後の先頭は次の形になる。

```rust
let detail = h_flex()
    .relative()
    .flex_1()
    .min_w_0()
    .min_h_0()
    .items_stretch()
    .vertical_scrollbar(&self.integrations_scroll)
    .child(detail_body)
```

- [ ] **Step 3: 8 px ガターの直接配置を削除する**

`detail` の末尾にある次のブロックを削除する。

```rust
.child(
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(px(8.))
        .child(Scrollbar::vertical(&self.integrations_scroll)),
)
```

- [ ] **Step 4: コンパイルを確認する**

Run: `cargo check -p vrc-photon-ui`

Expected: `Finished` で終わり、`Scrollbar` の未使用 import や型エラーがない。

- [ ] **Step 5: 実機で連携・自動画面を確認する**

`--demo` でアプリを起動し、連携・自動画面を開く。次を確認する。

- 詳細ペインの右端にトラックが表示されず、サムだけが表示される。
- 詳細ペインをスクロールし、別の項目へ切り替えて戻ったときに位置が保持される。
- ウィンドウ幅を狭くして一覧が詳細の上に積まれる表示でも、詳細ペインのバーが右端に収まる。
- 設定画面と同じ見た目・操作感である。

- [ ] **Step 6: コミットする**

```bash
git add crates/ui/src/ui/integrations.rs
git commit -m "refactor: 連携画面のスクロールバーを取り付けラッパーへ移す"
```

---

### Task 4: 圧縮キューの例外を記録し、全体を検証する

**Files:**
- Modify: `crates/ui/src/ui/compression.rs:1129-1131`
- Verify: `crates/ui/src/ui/year_rail.rs` と `crates/ui/src/state/year_rail.rs` に diff がないこと

**Interfaces:**
- Consumes: `Scrollbar::vertical(&queue_scroll).viewport_from_layout()`
- Produces: なし

- [ ] **Step 1: 直接配置の例外理由をコメントする**

`compression.rs` の次の行の直前にコメントを追加する。

```rust
.child(Scrollbar::vertical(&queue_scroll).viewport_from_layout())
```

追加するコメント:

```rust
// Exception: VirtualList is not a ScrollableElement, so the bar binds to
// the shared VirtualListScrollHandle and uses the layout bounds.
```

- [ ] **Step 2: フォーマット・Clippy・全テストを実行する**

Run:

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Expected: すべて成功。Clippy の警告なし。

- [ ] **Step 3: 年レールに変更がないことを確認する**

Run:

```bash
git diff main -- crates/ui/src/ui/year_rail.rs crates/ui/src/state/year_rail.rs
```

Expected: 出力なし。

- [ ] **Step 4: 全画面の回帰を確認する**

`--demo` でアプリを起動し、次を確認する。

- ダッシュボード、通知モーダル、検索フィルター、ダイアログ本体、写真インスペクタ、
  圧縮プロファイル一覧、AI タグ一覧、取り込み結果でトラックが表示されない。
- ダイアログ内の入れ子スクロール（本体とプロファイル一覧）で、両方ともトラックが表示されない。
- 圧縮キューのサムが表示され、ドラッグ・ホイールでスクロールできる。
- 写真グリッドの年レールが従来どおり表示され、クリック・ドラッグでジャンプできる。
- ダーク・ライト・パステルの各テーマで、トラックが再表示されない。
- スクロール停止後にサムがフェードアウトする。

- [ ] **Step 5: コミットする**

```bash
git add crates/ui/src/ui/compression.rs docs/superpowers/plans/2026-11-12-issue-604-scrollbar.md
git commit -m "docs: 圧縮キューのスクロールバー例外を記録する"
```

---

### Task 5: PR を作成し、CI とレビューを完了する

**Files:**
- なし（ブランチ全体）

**Interfaces:**
- Consumes: Task 1〜4 のコミット
- Produces: PR

- [ ] **Step 1: ブランチを push する**

```bash
git push -u origin feat/604-scrollbar
```

- [ ] **Step 2: PR を作成する**

```bash
gh pr create --base main --head feat/604-scrollbar \
  --title "refactor: #604 スクロールバーのトラック表示と取り付け方式を統一する" \
  --body "## 変更\n\n- 標準スクロールバーのトラックを全テーマで透明にする。\n- 設定・連携の直接配置を外側ラッパーの .vertical_scrollbar(&handle) へ移す。\n- 圧縮キューの直接配置は VirtualList の制約による例外として残す。\n- 写真グリッドの年レールは変更しない。\n\n## 確認\n\n- cargo fmt / clippy / test --workspace\n- 設定・連携・圧縮・ダイアログの visual QA\n- 年レールの回帰確認\n\n設計: docs/superpowers/specs/2026-11-12-issue-604-scrollbar-design.md" \
  --label enhancement
```

- [ ] **Step 3: CI を監視する**

`.opencode/skills/ci-watch/scripts/ci-watch.sh feat/604-scrollbar CI` を background で実行し、最新 run を毎回解決する。成功したら次へ進む。

- [ ] **Step 4: 最終レビューを受ける**

`requesting-code-review` の code-reviewer を fresh context で実行する。Critical / Important の指摘は 1 回の fix pass で直し、テストで RED→GREEN を確認する。Minor は記録してユーザーへ報告する。

- [ ] **Step 5: マージする**

CI が green でレビュー指摘が解消したら `gh pr merge <n> --merge`。マージ後は main へ戻って pull し、作業ブランチをローカル・リモートから削除する。

- [ ] **Step 6: Issue #604 のスコープを更新する**

年レールを対象外にした決定を Issue #604 へコメントする。必要なら受け入れ条件を更新する。
