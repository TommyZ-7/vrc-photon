# 写真グリッドの間隔とカード角丸 (#487) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Issue #487 の2設定（グリッド間隔 0〜32 px、カード角丸 0〜24 px）を `AppSettings` に追加し、設定画面の「表示（ライブラリ・検索）」カードからスライダーで変更できるようにする。変更は Photos の時系列表示・セッション別表示と検索結果へ再起動なしで反映する。

**Architecture:** 設定値は `vrc-photon-core` の `AppSettings` に保存し、読み込み時・保存時に範囲内へクランプする。UI のレイアウト計算は純粋関数のまま `grid_gap` を引数で受け取り、`gallery.rs` の共通描画から現在値を渡す。カード角丸は `render_card` が設定値を読み、内側画像は既存の `inner_corner_radius` で導出する。スライダーはドラッグ中にメモリ上の値を更新し、離した時に1回だけ保存する。

**Tech Stack:** Rust 2024, gpui-kit 0.9.1, SQLite, rust-i18n

**Spec:** `docs/superpowers/specs/2026-11-10-issue-487-grid-spacing-design.md`

**Mock:** `docs/ui-487-grid-spacing-mockup.html`

**Status:** 計画作成中。Phase 0 の docs PR 承認後に実装を開始する。

## Global Constraints

- 設定は2件だけ。グリッド間隔 `ui_grid_gap`（0〜32、既定14）、カード角丸 `ui_card_radius`（0〜24、既定13）。
- 間隔は横方向と縦方向の両方に適用する。セッション見出し行・日付行・継続行の間隔も同じ値にする。
- 角丸は写真カードとその内側画像だけに適用する。インスペクタ・ビューア・設定カードなどは変更しない。
- 適用対象は Photos の時系列表示・セッション別表示・検索結果。検索専用の設定は作らない。
- 値は1 px単位の整数。範囲外は読み込み時・保存時にクランプする。
- ドラッグ中は即時反映、`SliderEvent::Release` で保存する。ドラッグ中のDB書き込みはしない。
- 新規文言は `crates/i18n/locales/ja.yml` と `en.yml` の両方に登録する。
- 既存のカードサイズ・表示スイッチ・サムネイル設定の挙動を変えない。
- docs とコードは同じ PR に混ぜない。Phase 0 は docs-only PR、Phase 1 は実装 PR。
- 各 PR は最新の `main` から `git worktree` で切る。`main` への直接 push はしない。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- CI が green になるまで merge しない。CI は `ci-watch` の手順で監視する。
- 計画ファイルのチェックボックスは、その作業の完了時に `- [x]` に更新する。

## Review Focus

1. **既存の見た目**: 既定値 14 px / 13 px で、変更前のカード間隔・行高・角丸が完全に一致する。
2. **列数計算**: 間隔を変えたとき、列数とカード幅が同じ `grid_gap` で計算される。片方だけ古い定数を使わない。
3. **縦方向**: 写真行だけでなく、セッション見出し行・日付ラベル行・継続行の高さにも設定値が入る。
4. **ページエラー行**: `render_page_error` の高さ計算が `CARD_ROW_GAP` ではなく渡された `grid_gap` を使う。
5. **アンカー**: 間隔変更後も表示中の先頭写真が維持される。セッションの折りたたみ状態が維持される。
6. **角丸0**: カードと内側画像の角丸が0になり、境界線・選択リング・バッジが崩れない。
7. **角丸24**: 内側画像が23 pxになり、カードの1 px境界からはみ出さない。
8. **範囲外の設定JSON**: `999`、`-1`、`"wide"` などで `AppSettings` 全体のパースが失敗せず、該当項目だけ既定値または範囲内になる。
9. **保存タイミング**: スライダーをドラッグしてもDB書き込みが連続せず、離した時に1回だけ保存される。
10. **検索結果**: 検索グリッドも同じ間隔・角丸で描画される。

---

## Phase 0: 設計・モック・docs PR

**Files:**
- Create: `docs/superpowers/specs/2026-11-10-issue-487-grid-spacing-design.md`
- Create: `docs/superpowers/plans/2026-11-10-issue-487-grid-spacing.md`（このファイル）
- Create: `docs/ui-487-grid-spacing-mockup.html`
- Modify: `docs/ui.md`

**Interfaces:**
- Produces: 承認済みの設計・モック・計画、更新済み `docs/ui.md`

- [ ] **Step 1: `docs/ui.md` を更新する**

更新箇所:

- §2.4 写真: 「列間9 px」を「列間14 px（設定で0〜32 px）」へ修正する。
- §2.4 写真: 「カードは角丸13 px。」を「写真カードの角丸は設定（既定13 px、0〜24 px）に従う。」へ変更する。
- §2.4 写真: 列数・行高がグリッド間隔設定に連動することを追記する。
- §2.9 設定: 「表示（ライブラリ・検索）」カードに、グリッド間隔とカード角丸のスライダー、範囲、既定値、即時反映、保存タイミングを追記する。
- §6 デザイントークン: 写真カードの角丸が設定可能（既定13 px、0〜24 px）であることを追記する。
- §10 参考: #487 の設計・計画・モックへのリンクを追加する。

- [ ] **Step 2: Plannotator で計画を提示する**

```bash
plannotator annotate docs/superpowers/plans/2026-11-10-issue-487-grid-spacing.md --gate --json
```

Expected: `decision: "approved"`。注釈が返った場合は計画へ反映して再提示する。

- [ ] **Step 3: Plannotator でモックを提示する**

```bash
plannotator annotate docs/ui-487-grid-spacing-mockup.html --gate --json
```

Expected: `decision: "approved"`。スライダーの配置、値表示、範囲ヒント、間隔・角丸の端の見え方を確認する。

- [ ] **Step 4: docs PR を作る**

```bash
git add docs/ui.md docs/ui-487-grid-spacing-mockup.html \
  docs/superpowers/specs/2026-11-10-issue-487-grid-spacing-design.md \
  docs/superpowers/plans/2026-11-10-issue-487-grid-spacing.md
git commit -m "docs: 写真グリッドの間隔とカード角丸 #487 の設計・モック・計画"
git push -u origin docs/487-grid-spacing
gh pr create --title "docs: #487 写真グリッドの間隔とカード角丸の設計・モック" \
  --body "Refs #487" --label documentation
```

- [ ] **Step 5: CI green を確認して merge する**

`ci-watch` の手順で `CI` を監視する。docs-only のため Release の bundle は走らない。
green 後 `gh pr merge <n> --merge`、`main` へ戻って pull する。

---

## Phase 1: 実装 PR

### Task 1: 実装用 worktree とベースライン

**Files:**
- None（作業ディレクトリのみ）

- [ ] **Step 1: 最新の `main` から worktree を作る**

```bash
git fetch origin
git worktree add ../vrc-photo-manager-487 -b feat/487-grid-spacing origin/main
```

- [ ] **Step 2: worktree でベースラインを確認する**

```bash
cargo test --workspace
```

Expected: PASS。失敗する場合は環境の問題を先に解消する。

---

### Task 2: `AppSettings` に2設定を追加する

**Files:**
- Modify: `crates/core/src/settings.rs`

**Interfaces:**
- Produces: `ui_grid_gap: u8`、`ui_card_radius: u8`
- Produces: `UI_GRID_GAP_MIN/MAX/DEFAULT`、`UI_CARD_RADIUS_MIN/MAX/DEFAULT`
- Produces: `AppSettings::normalize_grid_display`

- [ ] **Step 1: 範囲と既定値の定数を追加する**

```rust
pub const UI_GRID_GAP_MIN: u8 = 0;
pub const UI_GRID_GAP_MAX: u8 = 32;
pub const UI_GRID_GAP_DEFAULT: u8 = 14;
pub const UI_CARD_RADIUS_MIN: u8 = 0;
pub const UI_CARD_RADIUS_MAX: u8 = 24;
pub const UI_CARD_RADIUS_DEFAULT: u8 = 13;
```

- [ ] **Step 2: `AppSettings` にフィールドを追加する**

既存の `ui_card_size` の近くに追加する。

```rust
#[serde(default = "default_ui_grid_gap", deserialize_with = "deserialize_ui_grid_gap")]
pub ui_grid_gap: u8,
#[serde(default = "default_ui_card_radius", deserialize_with = "deserialize_ui_card_radius")]
pub ui_card_radius: u8,
```

`Default` 実装にも既定値を追加する。

- [ ] **Step 3: 専用 deserializer を追加する**

`serde_json::Value` を受け取り、`as_f64()` で数値だけを読む。
`round()` して範囲内へ `clamp` し、`u8` へ変換する。数値以外は既定値にする。
負数・上限超過・小数・巨大値でも `AppSettings` 全体のパースを失敗させない。

```rust
fn deserialize_ui_grid_gap<'de, D>(deserializer: D) -> std::result::Result<u8, D::Error>
where D: serde::Deserializer<'de> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_f64()
        .map(|v| v.round().clamp(f64::from(UI_GRID_GAP_MIN), f64::from(UI_GRID_GAP_MAX)) as u8)
        .unwrap_or(UI_GRID_GAP_DEFAULT))
}
```

カード角丸も同じ形にする。

- [ ] **Step 4: 読み込み時・保存時の正規化を追加する**

```rust
fn normalize_grid_display(&mut self) {
    self.ui_grid_gap = self.ui_grid_gap.clamp(UI_GRID_GAP_MIN, UI_GRID_GAP_MAX);
    self.ui_card_radius = self.ui_card_radius.clamp(UI_CARD_RADIUS_MIN, UI_CARD_RADIUS_MAX);
}
```

`AppSettings::load` の成功パスと `AppSettings::save` の複製に対して呼ぶ。
`save` は既存どおり `self.clone()` を正規化して書き込む。

- [ ] **Step 5: core のテストを追加する**

- 既定値が 14 / 13。
- `{}` から読むと 14 / 13。
- `ui_grid_gap: 33` / `255` / `999` / `-1` / `"wide"` が範囲内または既定値になる。
- `ui_card_radius: 25` / `255` / `-1` / `"round"` が範囲内または既定値になる。
- 範囲内の値はそのまま読める。
- `save` が範囲外の値をクランプして書き込む。

```bash
cargo test -p vrc-photon-core settings
```

Expected: PASS。

---

### Task 3: レイアウト関数へ `grid_gap` を渡す

**Files:**
- Modify: `crates/ui/src/ui/theme.rs`
- Modify: `crates/ui/src/state/photo_browser.rs`（テストの呼び出し）
- Modify: `crates/ui/src/app.rs`（初期行高の呼び出し）

**Interfaces:**
- Produces: `grid_columns(available_width, min_card_width, grid_gap)`
- Produces: `photo_card_width(available_width, columns, grid_gap)`
- Produces: `photo_row_height(card_width, show_world, show_meta, grid_gap)`
- Produces: `library_grid_metrics(..., grid_gap)`
- Produces: `search_grid_metrics(..., grid_gap)`

- [ ] **Step 1: 定数の二重定義を整理する**

`GRID_GAP` と `CARD_ROW_GAP` はどちらも14 pxの固定値である。
`GRID_GAP` を core の `UI_GRID_GAP_DEFAULT` から導出する既定値として残し、
`CARD_ROW_GAP` は削除して `GRID_GAP` に統一する。テストと初期行高は `GRID_GAP` を使う。
既定値が core と一致することをテストで固定する。

- [ ] **Step 2: レイアウト関数のシグネチャを変更する**

`grid_columns`、`photo_card_width`、`photo_row_height`、`library_grid_metrics`、
`search_grid_metrics`、`grid_metrics_for` に `grid_gap: f32` を追加し、
内部の `GRID_GAP` / `CARD_ROW_GAP` 参照を引数へ置き換える。

- [ ] **Step 3: 全呼び出しを更新する**

`rg -n "grid_columns|photo_card_width|photo_row_height|library_grid_metrics|search_grid_metrics" crates/ui/src`
で全呼び出しを確認し、テストは `GRID_GAP`、本体は後続タスクで設定値を渡す。

- [ ] **Step 4: theme のテストを更新・追加する**

- 既定の間隔で既存の期待値が変わらない。
- 間隔0 / 14 / 32 で列数・カード幅・行高が計算どおりになる。
- 間隔0でゼロ除算しない。
- 検索グリッドも同じ計算になる。
- `GRID_GAP` が `f32::from(UI_GRID_GAP_DEFAULT)` と一致する。

```bash
cargo test -p vrc-photon-ui theme
```

Expected: PASS。

---

### Task 4: ギャラリーとセッション行へ設定値を配線する

**Files:**
- Modify: `crates/ui/src/ui/gallery.rs`
- Modify: `crates/ui/src/app/session.rs`
- Modify: `crates/ui/src/app.rs`

**Interfaces:**
- Produces: `App::grid_gap() -> f32`
- Consumes: `app.settings.ui_grid_gap`、`app.settings.ui_card_radius`

- [ ] **Step 1: `App::grid_gap` を追加する**

```rust
pub(crate) fn grid_gap(&self) -> f32 {
    f32::from(self.settings.ui_grid_gap)
}
```

- [ ] **Step 2: `render_grid` と `render_sessions` で設定値を渡す**

両方の先頭で `let grid_gap = app.grid_gap();` を取得し、`library_grid_metrics` /
`search_grid_metrics` と各行の `.gap(px(grid_gap))`、`render_page_error` に渡す。
ページエラー行の高さ計算は `(metrics.row_height - grid_gap)` にする。

- [ ] **Step 3: `render_card` で角丸を読む**

```rust
let card_radius = f32::from(app.settings.ui_card_radius);
let image_radius = inner_corner_radius(card_radius, 1.0);
```

`13.0` の固定値を置き換える。カード本体・内側画像・角丸を伴うバッジの構造は変えない。

- [ ] **Step 4: セッション行の高さを設定値に揃える**

`crates/ui/src/app/session.rs` の `SESSION_HEADER_ROW_HEIGHT + CARD_ROW_GAP`、
`SESSION_DATE_LABEL_ROW_HEIGHT + CARD_ROW_GAP`、
`SESSION_CONTINUATION_ROW_HEIGHT + CARD_ROW_GAP` を
`+ self.grid_gap()` に置き換える。`ensure_library_session_rows`、
`capture_top_photo`、`scroll_to_photo`、`toggle_session_collapsed` の全経路を揃える。

- [ ] **Step 5: gallery 側のセッション行の高さも揃える**

`render_sessions` 内の同じ3つの加算を `+ grid_gap` に置き換える。

- [ ] **Step 6: アンカー維持を確認する**

スライダーの `Release` で `capture_display_layout_anchors()` を呼ぶ（Task 5）。
ギャラリー側は既存の `layout_changed` 判定で先頭写真をアンカーする。
タイムライン・セッション表示の両方で、変更後に先頭写真が維持されることをテストまたは手動で確認する。

- [ ] **Step 7: 既存テストを更新する**

`photo_browser.rs` などで `library_grid_metrics` を呼ぶテストに `GRID_GAP` を渡す。
セッション行高の期待値があるテストは `grid_gap` を明示する。

```bash
cargo test -p vrc-photon-ui
```

Expected: PASS。

---

### Task 5: 設定画面のスライダーと保存処理を追加する

**Files:**
- Modify: `crates/ui/src/app.rs`
- Modify: `crates/ui/src/ui/settings.rs`
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`

**Interfaces:**
- Consumes: `SliderState`、`SliderEvent`
- Produces: `grid_gap_slider`、`card_radius_slider` とそれぞれの subscription

- [ ] **Step 1: `App` にスライダー状態を追加する**

既存の `thumbnail_quality_slider` の近くに追加する。

```rust
pub(crate) grid_gap_slider: Entity<SliderState>,
pub(crate) _grid_gap_subscription: Subscription,
pub(crate) card_radius_slider: Entity<SliderState>,
pub(crate) _card_radius_subscription: Subscription,
```

- [ ] **Step 2: コンストラクタで `SliderState` を初期化する**

```rust
let grid_gap_slider = cx.new(|_| {
    SliderState::new()
        .min(f32::from(UI_GRID_GAP_MIN))
        .max(f32::from(UI_GRID_GAP_MAX))
        .step(1.0)
        .default_value(f32::from(initial_settings.ui_grid_gap))
});
```

カード角丸も同じ形で、`UI_CARD_RADIUS_MIN/MAX` と `initial_settings.ui_card_radius` を使う。

- [ ] **Step 3: Change / Release の subscription を追加する**

`Change` では値を `round().clamp(...) as u8` して `self.settings` を更新し、
`cx.notify()` だけ行う。`Release` では同じ値を確定し、
`capture_display_layout_anchors()` を呼んでから `self.settings.save(&self.db)` を1回実行する。
保存失敗は `tracing::warn!` を出し、メモリ上の値は維持する。

2つのスライダーで同じ処理を繰り返す場合は、小さなヘルパー
`save_grid_display_settings(&mut self, cx)` を作って共有する。
既存のサムネイル画質の保存挙動（キャッシュを消すなど）は変えない。

- [ ] **Step 4: 設定画面に2行を追加する**

`crates/ui/src/ui/settings.rs` の「表示（ライブラリ・検索）」カードで、
4つの `setting_switch` の後に区切り線と2つのスライダー行を追加する。
各行は既存のサムネイル画質と同じ構造にする。

- ラベルと現在値（`format!("{} px", value)`）を同じ行に置く。
- `Slider::new(&self.grid_gap_slider).w_full()` / `Slider::new(&self.card_radius_slider).w_full()`。
- ヒント行を追加する。

- [ ] **Step 5: i18n キーを追加する**

`ja.yml`:

```yaml
settings.display.grid_gap.label: グリッド間隔
settings.display.grid_gap.hint: カード同士のすき間（0〜32 px）
settings.display.card_radius.label: カードの角丸
settings.display.card_radius.hint: 写真カードの角丸（0〜24 px）
```

`en.yml`:

```yaml
settings.display.grid_gap.label: Grid spacing
settings.display.grid_gap.hint: Space between cards (0–32 px)
settings.display.card_radius.label: Card corner radius
settings.display.card_radius.hint: Photo card corner radius (0–24 px)
```

- [ ] **Step 6: i18n テストを通す**

```bash
cargo test -p vrc-photon-i18n
```

Expected: PASS。ja / en の両方にキーが存在する。

- [ ] **Step 7: UI をビルドして設定画面を確認する**

```bash
cargo run -p vrc-photon-ui --bin vrc-photon -- --demo
```

設定画面で2つのスライダーが表示され、値を動かすと現在値が更新されることを確認する。

---

### Task 6: 動作確認と回帰確認

**Files:**
- None（確認のみ）

- [ ] **Step 1: 自動検証を実行する**

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Expected: PASS。

- [ ] **Step 2: 既定値の回帰を確認する**

`--demo` で起動し、設定が 14 px / 13 px のとき、変更前と同じ間隔・角丸・列数・行高で
表示されることを確認する。タイムラインとセッション表示の両方を見る。

- [ ] **Step 3: 端の値を確認する**

- 間隔0 / 角丸0: カードが接し、角が直角になる。境界線と選択リングが崩れない。
- 間隔32 / 角丸24: 列数が減る場合がある。内側画像がカードからはみ出さない。
- 検索結果でも同じ値になる。

- [ ] **Step 4: 永続化と即時反映を確認する**

スライダーを動かした直後にプレビューが変わり、指を離した後に再起動しても値が残る。
設定画面を離れて Photos へ戻ったとき、表示中の先頭写真が維持される。

- [ ] **Step 5: 設定JSONの異常値を確認する**

テストで `999` / `-1` / 文字列を確認済み。必要ならデモDBの `app_settings` を手で書き換え、
アプリ起動時に他の設定が初期化されないことを確認する。

---

### Task 7: 実装 PR と CI

**Files:**
- None

- [ ] **Step 1: 変更をコミットする**

```bash
git add crates/core/src/settings.rs crates/ui/src/ui/theme.rs crates/ui/src/ui/gallery.rs \
  crates/ui/src/app/session.rs crates/ui/src/app.rs crates/ui/src/ui/settings.rs \
  crates/ui/src/state/photo_browser.rs crates/i18n/locales/ja.yml crates/i18n/locales/en.yml
git commit -m "feat(ui): 写真グリッドの間隔とカード角丸を設定可能にする (#487)"
```

- [ ] **Step 2: PR を作る**

```bash
git push -u origin feat/487-grid-spacing
gh pr create --title "feat(ui): 写真グリッドの間隔とカード角丸を設定可能にする" \
  --body "Closes #487" --label enhancement
```

- [ ] **Step 3: CI green を確認して merge する**

`ci-watch` の手順で `CI` を監視する。green 後 `gh pr merge <n> --merge`、
`main` へ戻って pull する。作業 worktree とブランチを削除する。

- [ ] **Step 4: Issue #487 のクローズを確認する**

PR 本文の `Closes #487` で自動クローズされる。クローズされない場合は
ユーザーへ確認してから手動でクローズする。
