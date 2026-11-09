# 連携・自動 マスター・ディテール Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 連携・自動画面を、左に4項目の一覧、右に選択中項目の詳細を置くマスター・ディテール構成へ変更する（#448）。

**Architecture:** `App` に画面内選択状態 `IntegrationsSection` を追加し、画面遷移をまたいで保持する。`ui::integrations` で一覧と詳細を組み立て、既存の `discord_card` / `auto_compression_card` / `ai_tagging_card` / `clipboard_card` をそのまま詳細ペインへ移す。送信履歴は一覧項目にせず、Discord 詳細の2枚目のカードとして表示する。幅に応じた縦積み判定は `ui::theme` の純粋関数に置く。

**Tech Stack:** Rust stable (edition 2024), gpui-kit 0.9, vrc-photon-i18n

**Spec:** GitHub issue #448 と承認済みモック `docs/integrations-master-detail-mockup.html`（案B）。採用案Bから「送信履歴を独立項目にしない」点だけをユーザー指示で変更する。

## Global Constraints

- 各項目の機能・既存文言は変更しない。一覧のサマリ用に新しい文言だけを追加する。
- 送信履歴は一覧の5項目目にせず、Discord 連携の詳細内に表示する。
- 選択状態は画面遷移では保持し、アプリ再起動では保持しない。既定は Discord 連携。
- 狭い幅では一覧を詳細の上に置く縦並びにする。
- 実アプリはタイトルバーに画面名を出すため、モックのページ見出しは重複させず置かない（`docs/ui.md` §2.10 の既存方針を維持）。
- 既存カードの折りたたみ（機能オフ時の本文非表示）とアニメーションはそのまま使う。
- 連携・自動以外の画面のレイアウトは変更しない。
- コードとドキュメントを同じ PR に混ぜない（`AGENTS.md` §4）。

## Review Focus

- 1080 px（最小ウィンドウ）でサイドバー展開時に一覧と詳細が横並びのまま潰れないこと。→ Task 1 の幅テストで縦積みになることを固定する。
- 画面を離れて戻ったときに選択項目が初期値へ戻らないこと。→ `App` のフィールドに保持し、初期化時だけ Discord を既定にする。
- Discord が無効でも送信履歴が読めること。→ Discord 詳細内の2枚目カードとして独立して描画する。
- プロファイルが0件でもサマリが空にならないこと。→ Task 2 の `0 件` / `プロファイル未選択` テストで固定する。
- AI が未準備でも一覧のサマリと詳細の「準備してください」表示が壊れないこと。→ 既存カードをそのまま使い、サマリは設定値だけで組み立てる。

---

### Task 1: 選択状態と幅判定の土台

**Files:**
- Modify: `crates/ui/src/app.rs`
- Modify: `crates/ui/src/ui/theme.rs`
- Test: `crates/ui/src/app.rs`（`mod tests`）
- Test: `crates/ui/src/ui/theme.rs`（`mod tests`）

**Interfaces:**
- Consumes: `WORKSPACE_PAD`, `WORKSPACE_GAP`, `sidebar_width()` in `ui::theme`
- Produces:
  - `pub(crate) enum IntegrationsSection { Discord, AutoCompression, AiTagging, Clipboard }`（`Default = Discord`）
  - `App::integrations_selected: IntegrationsSection`
  - `pub(crate) const INTEGRATIONS_LIST_WIDTH: f32 = 252.0`
  - `pub(crate) const INTEGRATIONS_DETAIL_MIN_WIDTH: f32 = 620.0`
  - `pub(crate) const INTEGRATIONS_PANE_GAP: f32 = 16.0`
  - `pub(crate) fn integrations_available_width(window_width: f32, sidebar_collapsed: bool) -> f32`
  - `pub(crate) fn integrations_stacks(window_width: f32, sidebar_collapsed: bool) -> bool`

- [x] **Step 1: 幅判定の失敗テストを書く**

`crates/ui/src/ui/theme.rs` の `mod tests` に追加する。

```rust
#[test]
fn integrations_switches_to_the_stacked_layout_at_narrow_widths() {
    assert_eq!(integrations_available_width(1080.0, false), 804.0);
    assert_eq!(integrations_available_width(1080.0, true), 964.0);
    assert!(integrations_stacks(1080.0, false));
    assert!(!integrations_stacks(1080.0, true));
    assert!(!integrations_stacks(1280.0, false));
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui integrations_switches_to_the_stacked_layout`

Expected: FAIL（`integrations_available_width` と `integrations_stacks` が未定義）

- [x] **Step 3: theme の定数と関数を実装する**

`crates/ui/src/ui/theme.rs` の `CARD_GAP` 付近に定数を、`sidebar_width` の近くに純粋関数を追加する。

```rust
pub(crate) const INTEGRATIONS_LIST_WIDTH: f32 = 252.0;
pub(crate) const INTEGRATIONS_DETAIL_MIN_WIDTH: f32 = 620.0;
pub(crate) const INTEGRATIONS_PANE_GAP: f32 = 16.0;

/// 連携・自動の2ペインに使える幅（シェルの左右パディング、サイドバー、
/// サイドバーとの間隔を除いた値）。
pub(crate) fn integrations_available_width(window_width: f32, sidebar_collapsed: bool) -> f32 {
    (window_width - WORKSPACE_PAD * 2.0 - sidebar_width(sidebar_collapsed) - WORKSPACE_GAP).max(0.0)
}

/// 一覧（252 px）・最小詳細幅（620 px）・間隔（16 px）が入らないときは
/// 一覧を詳細の上へ積む。
pub(crate) fn integrations_stacks(window_width: f32, sidebar_collapsed: bool) -> bool {
    integrations_available_width(window_width, sidebar_collapsed)
        < INTEGRATIONS_LIST_WIDTH + INTEGRATIONS_DETAIL_MIN_WIDTH + INTEGRATIONS_PANE_GAP
}
```

- [x] **Step 4: 選択状態の既定値テストを書く**

`crates/ui/src/app.rs` の `mod tests` に追加する。

```rust
#[test]
fn integrations_default_to_discord() {
    assert_eq!(IntegrationsSection::default(), IntegrationsSection::Discord);
}
```

- [x] **Step 5: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui integrations_default_to_discord`

Expected: FAIL（`IntegrationsSection` が未定義）

- [x] **Step 6: `IntegrationsSection` と `App` のフィールドを追加する**

`Screen` の直後に追加する。

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum IntegrationsSection {
    #[default]
    Discord,
    AutoCompression,
    AiTagging,
    Clipboard,
}
```

`App` の `integrations_scroll` の隣にフィールドを追加する。

```rust
pub(crate) integrations_selected: IntegrationsSection,
```

初期化（`integrations_scroll: ScrollHandle::new(),` の近く）に追加する。

```rust
integrations_selected: IntegrationsSection::default(),
```

- [x] **Step 7: テストを実行する**

Run: `cargo test -p vrc-photon-ui integrations_`

Expected: PASS

- [x] **Step 8: コミット**

```bash
git add crates/ui/src/app.rs crates/ui/src/ui/theme.rs
git commit -m "feat(ui): add integrations master-detail state and width metrics"
```

---

### Task 2: 一覧サマリの文言と純粋関数

**Files:**
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`
- Modify: `crates/i18n/tests/placeholders.rs`
- Modify: `crates/ui/src/ui/integrations.rs`
- Test: `crates/ui/src/ui/integrations.rs`（`mod tests`）

**Interfaces:**
- Consumes: `tr!`, `tr_args!`
- Produces:
  - `fn on_off_label(enabled: bool) -> String`
  - `pub(crate) fn discord_summary(configured: bool, destination: &str) -> String`
  - `pub(crate) fn auto_compression_summary(enabled: bool, profile_count: usize) -> String`
  - `pub(crate) fn ai_tagging_summary(enabled: bool, running: i64, waiting: i64) -> String`
  - `pub(crate) fn clipboard_summary(enabled: bool, profile: Option<&str>) -> String`

- [x] **Step 1: サマリの失敗テストを書く**

`crates/ui/src/ui/integrations.rs` の `mod tests` に追加する。

```rust
#[test]
fn section_summaries_follow_the_existing_card_values() {
    assert_eq!(
        discord_summary(true, "VRChat写真部"),
        "設定済み ・ 送信先：VRChat写真部"
    );
    assert_eq!(
        discord_summary(false, "Discord"),
        "未設定 ・ 送信先：Discord"
    );
    assert_eq!(
        auto_compression_summary(true, 2),
        "有効 ・ 対象プロファイル 2 件"
    );
    assert_eq!(
        auto_compression_summary(false, 0),
        "無効 ・ 対象プロファイル 0 件"
    );
    assert_eq!(
        ai_tagging_summary(true, 1, 12),
        "有効 ・ 実行中 1 件 / 待機 12 件"
    );
    assert_eq!(
        clipboard_summary(true, Some("標準")),
        "有効 ・ プロファイル：標準"
    );
    assert_eq!(
        clipboard_summary(false, None),
        "無効 ・ プロファイル未選択"
    );
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui section_summaries_follow_the_existing_card_values`

Expected: FAIL（サマリ関数が未定義）

- [x] **Step 3: 日本語・英語の文言を追加する**

`crates/i18n/locales/ja.yml` の `integrations.clipboard.*` の後ろに追加する。

```yaml
integrations.summary.enabled: "有効"
integrations.summary.disabled: "無効"
integrations.summary.discord: "%{state} ・ 送信先：%{destination}"
integrations.summary.auto: "%{state} ・ 対象プロファイル %{count} 件"
integrations.summary.ai: "%{state} ・ 実行中 %{running} 件 / 待機 %{waiting} 件"
integrations.summary.clipboard: "%{state} ・ プロファイル：%{name}"
integrations.summary.clipboard_empty: "%{state} ・ プロファイル未選択"
```

`crates/i18n/locales/en.yml` の同じ位置に追加する。

```yaml
integrations.summary.enabled: "Enabled"
integrations.summary.disabled: "Disabled"
integrations.summary.discord: "%{state} · Destination: %{destination}"
integrations.summary.auto: "%{state} · %{count} target profile(s)"
integrations.summary.ai: "%{state} · Running %{running} / Waiting %{waiting}"
integrations.summary.clipboard: "%{state} · Profile: %{name}"
integrations.summary.clipboard_empty: "%{state} · No profile selected"
```

- [x] **Step 4: プレースホルダー登録を追加する**

`crates/i18n/tests/placeholders.rs` の `CALL_SITES` に、キー順を保ったまま追加する。

```rust
("integrations.summary.ai", &["running", "state", "waiting"]),
("integrations.summary.auto", &["count", "state"]),
("integrations.summary.clipboard", &["name", "state"]),
("integrations.summary.clipboard_empty", &["state"]),
("integrations.summary.discord", &["destination", "state"]),
```

- [x] **Step 5: サマリ関数を実装する**

`crates/ui/src/ui/integrations.rs` の `impl App` の外、`AiToggle` の近くに追加する。

```rust
fn on_off_label(enabled: bool) -> String {
    if enabled {
        tr!("integrations.summary.enabled")
    } else {
        tr!("integrations.summary.disabled")
    }
}

pub(crate) fn discord_summary(configured: bool, destination: &str) -> String {
    let state = if configured {
        tr!("integrations.discord.webhook.configured")
    } else {
        tr!("integrations.discord.webhook.missing_short")
    };
    tr_args!("integrations.summary.discord", state = state, destination = destination)
}

pub(crate) fn auto_compression_summary(enabled: bool, profile_count: usize) -> String {
    tr_args!(
        "integrations.summary.auto",
        state = on_off_label(enabled),
        count = profile_count
    )
}

pub(crate) fn ai_tagging_summary(enabled: bool, running: i64, waiting: i64) -> String {
    tr_args!(
        "integrations.summary.ai",
        state = on_off_label(enabled),
        running = running,
        waiting = waiting
    )
}

pub(crate) fn clipboard_summary(enabled: bool, profile: Option<&str>) -> String {
    let state = on_off_label(enabled);
    match profile {
        Some(name) => tr_args!(
            "integrations.summary.clipboard",
            state = state,
            name = name
        ),
        None => tr_args!("integrations.summary.clipboard_empty", state = state),
    }
}
```

あわせて、既存の `integrations.discord.webhook.missing`（「Webhook URL が未設定です」）とは別に、短い状態ラベルを追加する。

`ja.yml`:

```yaml
integrations.discord.webhook.configured: "設定済み"
integrations.discord.webhook.missing_short: "未設定"
```

`en.yml`:

```yaml
integrations.discord.webhook.configured: "Configured"
integrations.discord.webhook.missing_short: "Not configured"
```

これらはプレースホルダーを持たないため `CALL_SITES` への追加は不要。

- [x] **Step 6: テストを実行する**

Run: `cargo test -p vrc-photon-ui section_summaries_follow_the_existing_card_values && cargo test -p vrc-photon-i18n`

Expected: PASS

- [x] **Step 7: コミット**

```bash
git add crates/i18n/locales/ja.yml crates/i18n/locales/en.yml \
  crates/i18n/tests/placeholders.rs crates/ui/src/ui/integrations.rs
git commit -m "feat(i18n): add integrations master-detail summaries"
```

---

### Task 3: マスター・ディテールの描画

**Files:**
- Modify: `crates/ui/src/ui/integrations.rs`（`integrations_screen`、新規 `integrations_list` / `integrations_list_item`、tests）
- Modify: `crates/ui/src/ui/discord.rs`（必要な場合のみ `send_history_card` の可視性）
- Test: `crates/ui/src/ui/integrations.rs`

**Interfaces:**
- Consumes: Task 1 の `IntegrationsSection`, `integrations_stacks`, `integrations_available_width`, `INTEGRATIONS_LIST_WIDTH`, `INTEGRATIONS_PANE_GAP`; Task 2 のサマリ関数; 既存の `discord_card`, `send_history_card`, `auto_compression_card`, `ai_tagging_card`, `clipboard_card`
- Produces: `App::integrations_screen` が2ペイン（狭幅時は縦積み）を返す。一覧の項目は `integrations-list-discord` などのIDを持つ。

- [x] **Step 1: `integrations_screen` を2ペインへ組み替える**

現在の1カラムの `v_flex()` を、`window` から幅を取って `integrations_stacks` を呼ぶ形に変える。`content_viewport(window)` は `theme::*` から使える。

```rust
pub(crate) fn integrations_screen(
    &mut self,
    window: &Window,
    cx: &mut Context<Self>,
) -> impl IntoElement {
    let p = self.palette;
    let viewport = content_viewport(window);
    let stacked = integrations_stacks(f32::from(viewport.width), self.sidebar_collapsed);
    let selected = self.integrations_selected;

    let list = self.integrations_list(selected, stacked, cx);
    let detail = v_flex()
        .id("integrations-detail")
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&self.integrations_scroll)
        .gap(px(CARD_GAP));
    let detail = match selected {
        IntegrationsSection::Discord => detail
            .child(self.discord_card(window, cx))
            .child(self.send_history_card(window, cx)),
        IntegrationsSection::AutoCompression => {
            detail.child(self.auto_compression_card(window, cx))
        }
        IntegrationsSection::AiTagging => detail.child(self.ai_tagging_card(window, cx)),
        IntegrationsSection::Clipboard => detail.child(self.clipboard_card(window, cx)),
    };

    let body = if stacked {
        v_flex()
            .flex_1()
            .min_h_0()
            .gap(px(CARD_GAP))
            .child(list)
            .child(detail)
            .into_any_element()
    } else {
        h_flex()
            .flex_1()
            .min_h_0()
            .items_stretch()
            .gap(px(INTEGRATIONS_PANE_GAP))
            .child(list)
            .child(detail)
            .into_any_element()
    };

    h_flex()
        .relative()
        .flex_1()
        .min_h_0()
        .items_stretch()
        .child(body)
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(px(8.))
                .child(Scrollbar::vertical(&self.integrations_scroll)),
        )
}
```

- [x] **Step 2: 一覧を実装する**

既存のカード値を読み、4項目を次の順で並べる。`integrations_list` は幅252px（横並び時）、右側に区切り線を置く。

```rust
fn integrations_list(
    &self,
    selected: IntegrationsSection,
    stacked: bool,
    cx: &mut Context<Self>,
) -> AnyElement {
    let p = self.palette;
    let auto_profiles = self
        .profiles
        .iter()
        .filter(|profile| profile.enabled && profile.auto_enabled)
        .count();
    let clipboard_profile = self
        .profiles
        .iter()
        .find(|profile| profile.clipboard && profile.enabled)
        .map(|profile| profile.name.as_str());
    let running = self.ai.job_counts.pending + self.ai.job_counts.processing;
    let configured = self.discord_url.is_some();

    let items = [
        (
            IntegrationsSection::Discord,
            "integrations-list-discord",
            tr!("integrations.discord.title"),
            discord_summary(configured, &self.settings.discord_destination),
            configured,
        ),
        (
            IntegrationsSection::AutoCompression,
            "integrations-list-auto",
            tr!("integrations.auto.title"),
            auto_compression_summary(self.settings.auto_compress, auto_profiles),
            self.settings.auto_compress,
        ),
        (
            IntegrationsSection::AiTagging,
            "integrations-list-ai",
            tr!("integrations.ai.title"),
            ai_tagging_summary(
                self.settings.ai_tagging_enabled,
                running,
                self.ai.job_counts.waiting,
            ),
            self.settings.ai_tagging_enabled,
        ),
        (
            IntegrationsSection::Clipboard,
            "integrations-list-clipboard",
            tr!("integrations.clipboard.title"),
            clipboard_summary(self.settings.clipboard_enabled, clipboard_profile),
            self.settings.clipboard_enabled,
        ),
    ];

    v_flex()
        .id("integrations-list")
        .flex_shrink_0()
        .gap(px(4.))
        .when(!stacked, |this| {
            this.w(px(INTEGRATIONS_LIST_WIDTH))
                .pr(px(12.))
                .border_r_1()
                .border_color(rgba(p.highlight))
        })
        .children(items.into_iter().map(|(section, id, name, summary, on)| {
            let active = section == selected;
            v_flex()
                .id(id)
                .gap(px(3.))
                .p(px(11.))
                .rounded(px(9.))
                .border_1()
                .border_color(if active {
                    rgba(with_alpha(p.accent, 0x66))
                } else {
                    rgba(0x00000000)
                })
                .when(active, |this| this.bg(rgb(p.accent_soft)))
                .cursor_pointer()
                .hover(|this| this.bg(rgb(p.panel_2)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.integrations_selected = section;
                    this.integrations_scroll.set_offset(point(px(0.), px(0.)));
                    cx.notify();
                }))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .size(px(8.))
                                .rounded_full()
                                .bg(rgb(if on { p.green } else { p.muted_2 })),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_semibold()
                                .text_color(rgb(p.text))
                                .child(name),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(rgb(p.muted_2))
                        .child(summary),
                )
                .into_any_element()
        }))
        .into_any_element()
}
```

補足:
- `id` は `&'static str` のまま `SharedString::from(id)` に変換してよい。
- 選択済み項目もクリック可能にし、再クリックで詳細のスクロール位置を先頭へ戻す。
- 縦積み時は一覧の幅指定と右区切り線を出さない。
- 選択変更時に `set_offset(point(px(0.), px(0.)))` で詳細を先頭へ戻す。

- [x] **Step 3: 送信履歴を Discord 詳細へ移す**

`integrations_screen` の1カラム時代の `.child(self.send_history_card(window, cx))` を削除し、`IntegrationsSection::Discord` の腕だけで `discord_card` の直後に描画する。`send_history_card` 自体の内容は変更しない。

- [x] **Step 4: コンパイルとテストを実行する**

Run: `cargo test -p vrc-photon-ui`

Expected: PASS

- [x] **Step 5: lint を実行する**

Run: `cargo clippy -p vrc-photon-ui --all-targets -- -D warnings`

Expected: PASS

- [x] **Step 6: コミット**

```bash
git add crates/ui/src/ui/integrations.rs crates/ui/src/ui/discord.rs
git commit -m "feat(ui): render integrations as master detail"
```

---

### Task 4: 全体検証と目視確認

**Files:**
- Test: なし（検証のみ）

**Interfaces:**
- Consumes: Task 1–3 の成果
- Produces: 検証済みのブランチ

- [x] **Step 1: ワークスペース全体の検証を実行する**

Run:

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Expected: PASS

- [x] **Step 2: アプリを起動して目視確認する**

`hyprland-visual-qa` スキルに従い、別ワークスペースでアプリを起動する。連携・自動画面で次を確認する。

- 4項目（Discord 連携 / 自動圧縮 / AI タグ付け / クリップボード）が左一覧に並ぶ。
- 各項目に状態ドットと1行サマリが出る。
- 項目を切り替えると右詳細が対応するカードへ変わる。
- Discord 詳細に送信履歴カードが続けて表示される。
- AI 詳細に待機・実行中・失敗、実行中ジョブ最大3件、バックログ開始、キャンセルがある。
- 幅を狭くすると一覧が詳細の上へ積まれる。
- 画面を離れて戻っても選択項目が維持される。

- [x] **Step 3: スクリーンショットを保存して確認する**

横並び（広い幅）と縦積み（狭い幅）、Discord 詳細と AI 詳細の最低4枚を撮る。文字切れ、カードの重なり、スクロール不能がないことを確認する。問題があれば Task 3 に戻って修正する。

- [x] **Step 4: 最終コミット**

目視修正があればコミットする。

```bash
git add -A
git commit -m "fix(ui): polish integrations master-detail layout"
```

---

### Task 5: ドキュメント更新（コードPRのマージ後、別の docs PR）

**Files:**
- Modify: `docs/ui.md`（§2.10）

- [ ] **Step 1: 新しい main から docs ブランチを作る**

```bash
REPO=$(basename "$(git rev-parse --show-toplevel)")
git worktree add ../${REPO}-448-docs -b docs/448-integrations-master-detail
```

- [ ] **Step 2: §2.10 の冒頭を差し替える**

```markdown
ページ見出しは置かない。左に項目一覧、右に選択中項目の詳細を置く
マスター・ディテール構成にする。一覧は Discord 連携、自動圧縮、AI タグ付け、
クリップボードの4項目で、各項目に状態ドットと1行サマリを表示する。
ウィンドウ幅が狭いときは一覧を詳細の上へ積む。選択は画面遷移では保持し、
アプリ再起動では保持しない。右の詳細は選択中項目のカードだけを内部スクロールで
表示する。送信履歴は独立した一覧項目にせず、Discord 連携の詳細に置く。

AI タグ付けカードは、有効化 / 自動 / バックログのスイッチ、バックエンド選択、
待機・処理中・失敗の件数（色付きのカード表示はしない）、実行中ジョブの進捗と
キャンセル、バックログ開始、設定への導線を表示する。状態バッジは表示しない。
```

- [ ] **Step 3: コミットして docs PR を作る**

```bash
git add docs/ui.md
git commit -m "docs: describe the integrations master-detail layout"
git push -u origin docs/448-integrations-master-detail
gh pr create --title "docs: describe the integrations master-detail layout" \
  --body "Refs #448"
```

ラベルは `enhancement`。CI の docs-only fast-pass が緑になってからマージする。
