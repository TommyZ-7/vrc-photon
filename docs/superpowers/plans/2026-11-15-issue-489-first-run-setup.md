# 初回起動セットアップ Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 初回起動時だけ、8ページのセットアップウィザードを表示し、言語・起動・フォルダ・表示・Discord・自動圧縮・テーマ・AIタグ付けをまとめて設定できるようにする。既存ユーザーには表示せず、完了または閉じた後は通常のメイン画面へ移る。

**Architecture:** `AppSettings` に `setup_completed` を追加し、設定行が存在しない新規インストールだけを未完了として扱う。UI 側は `app/setup.rs` にページ遷移と永続化の状態を置き、`ui/setup.rs` にウィザード本体を描く。ウィザード表示中はシェルを描かず、46 px の最小タイトルバー＋中央のモーダルだけを表示する。各コントロールは変更時に保存し、フォルダと Webhook だけはページを離れるときに検証して保存する。表示ページには、スイッチ・スライダー・解像度の変更が即時反映されるカードプレビューを置く。

**Tech Stack:** Rust stable (edition 2024), gpui-kit 0.9.1 (gpui-component), SQLite, rust-i18n

**Spec:** Issue #489（初回起動時に初期セットアップモーダルを表示する）

**Mock:** `docs/ui-489-first-run-setup-mockup.html`（Plannotator で承認済み）。`/home/tommyz7/work/design-489/page1.png` 〜 `page8.png` は方向性の参照であり、設定項目・ラベル・選択肢は実装の設定画面（`docs/ui.md` §2.9 / §2.10）と `AppSettings` を正とする。旧モック PR #648 はこのモックに置き換わるため、実装後にクローズする。

**Design measurement:** デザイン画像は 1280×774。既定ウィンドウ 1280×820 からタイトルバー 46 px を引いた領域と一致する。モーダルは 600×561 で、残り領域の中央（x=340, y=107）に置く。承認済みモックの座標を実装値として使う。

---

## Global Constraints

- セットアップは初回起動時のみ。`AppSettings::setup_completed` を永続化し、2回目以降は表示しない。
- 既存の設定行がある環境（アップデート移行を含む）は、`setup_completed = true` なら表示しない。旧 JSON に `setup_completed` が無い場合は「完了済み」として扱う。`false` は中断したセットアップとして再開する。
- 設定行が無くても、ライブラリに写真がある環境では表示しない（設定行の有無で新規インストールと既存ライブラリを区別する）。設定行が壊れている場合は表示しない。
- デモ DB、`--demo`、DB オープン失敗時のフォールバックでは表示しない。
- 完了・閉じる（× / Esc）のどちらでも `setup_completed = true` を保存し、通常のメイン画面へ移る。
- ウィザード表示中はシェルを描かない。ウィンドウ操作のため 46 px の最小タイトルバーだけ残す。
- 8ページの順序は ようこそ → 起動 → フォルダ → 表示とサムネイル → Discord → 自動圧縮 → テーマ → AIタグ付け。ページ番号・進捗バー・アイコン・説明文は追加しない。
- アニメーションは付けない。ページ切り替えは即時。
- 設定項目は実装の設定画面に合わせる。テーマは **メイン18色＋サブ18色**（6色×3行、実ラベル・実プレビュー色）、表示は **スイッチ4項目＋グリッド間隔/角丸スライダー＋サムネイル解像度チップ**、起動は **自動起動/トレイ常駐/アニメーションを減らす**、Discord は **有効化/Webhook URL/送信先名/プレイヤー名を含める/原画を優先**、自動圧縮は **有効化/新規写真をすぐに圧縮/元画像を保持/形式チップ/ロスレス**、AI は **有効化/プラグインZIP/モデルDL/新規画像を自動タグ付け**。
- 初回に不要な高度設定は設定画面に残す: 自動送信・送信履歴・Discord 圧縮、圧縮の画質・リサイズ、AI バックエンド・閾値・variant 管理、ワールド名解決、最大FPS、自動更新、サムネイルキャッシュ、クリップボード。
- デザイン画像のトグル・選択状態は一例であり、既定値は変更しない（現在の `AppSettings::default()` を正とする）。Discord は項目を見せるためモックでは ON 表示だが、実際の初期値は OFF。
- テーマの全18色は設定画面の既存 `theme_axis` を再利用し、選択は即時反映・保存する。
- 表示ページのプレビューは、4スイッチ・グリッド間隔・角丸・サムネイル解像度の変更に即時追従する。実画像は使わず、パレット色のグラデーションとサンプル文言で描く。
- AI のモデルダウンロードはプラグイン導入後に有効化する。未導入時は無効表示。
- フォルダと Webhook はページを離れるときに保存する。Webhook が不正ならページに留まり、インラインでエラーを出す。空欄は「変更なし」。
- 新規 UI 文言は `crates/i18n/locales/ja.yml` と `en.yml` の両方に追加する。
- Playwright は使用しない。モックは Plannotator と Firefox ヘッドレスで確認済み。実装の視覚確認は X11/XWayland のウィンドウキャプチャで行う。
- 検証コマンド: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` と `scripts/check-docs-drift.sh`。
- 計画ファイルのチェックボックスは実装完了時に `- [x]` へ更新する。

## Review Focus

1. **既存ユーザーに出ない**: 旧設定 JSON（`setup_completed` 無し）は true、設定行が無く写真がある DB も非表示。Task 1 / Task 2 のテストで固定する。
2. **完了状態の永続化**: 完了・×・Esc のすべてで true が保存され、次回起動で出ない。途中で落ちた場合は false のまま残り、再表示される。Task 1 / Task 3 で固定する。
3. **Webhook の取りこぼし防止**: 不正 URL は保存せず、ページに留まって既存のエラー文言を出す。空欄は保存も削除もしない。Task 6 で固定する。
4. **表示プレビューの一致**: 4スイッチ・グリッド間隔・角丸・サムネイル解像度の変更がプレビューへ即時反映され、設定値と一致する。Task 5 で固定する。
5. **テーマの完全性**: メイン18色・サブ18色が設定画面と同じ並び・色・ラベルで表示され、選択が即時反映・保存される。Task 5 で固定する。
6. **AI ライセンスモーダルの重なり**: セットアップ画面からモデルダウンロードを押してライセンス同意が必要な場合、同意モーダルがセットアップの上に出る。Task 7 で固定する。
7. **フォルダ変更の反映**: ページ3の保存で写真ルート変更時にライブラリスナップショットを更新し、デーモンへ再スキャンを要求する。Task 4 で固定する。
8. **レイアウト**: 600×561 のモーダルが 46 px タイトルバー下の中央に収まり、8ページすべてでフッターが同じ位置に来る。Task 8 のスクリーンショットで確認する。

---

## File Structure

- `crates/core/src/settings.rs`: `setup_completed` フィールド、既定値、serde テスト。
- `crates/ui/src/app/setup.rs`（新規）: `SetupPage`、`SetupState`、表示判定、ページ遷移、フォルダ／Webhook の保存、自動圧縮の保存ヘルパー。
- `crates/ui/src/app.rs`: `mod setup`、`App.setup`、初回判定、`overlay_open`、`render` のセットアップ分岐。
- `crates/ui/src/ui/setup.rs`（新規）: ウィザードの描画（タイトルバー、パネル、8ページの本文、表示プレビュー、フッター）。
- `crates/ui/src/ui/mod.rs`: `pub(crate) mod setup;`。
- `crates/ui/src/ui/settings.rs`: パス保存処理を `save_library_paths` に抽出、`set_reduce_motion` と `theme_axis` を `pub(crate)` に。
- `crates/ui/src/ui/integrations.rs`: `set_auto_compress_enabled` を `pub(crate)` に。
- `crates/ui/src/app/discord.rs`: `webhook_error_key` を `pub(crate)` に。
- `crates/i18n/locales/ja.yml`, `crates/i18n/locales/en.yml`: セットアップ用文言。
- `docs/ui-489-first-run-setup-mockup.html`: 承認済みモック。
- `docs/ui.md`: 初回セットアップの仕様節。
- `docs/data-model.md`: `setup_completed` の保存仕様。
- `docs/superpowers/plans/2026-11-15-issue-489-first-run-setup.md`: この計画。

---

### Task 1: `setup_completed` を設定に追加する

**Files:**
- Modify: `crates/core/src/settings.rs`

**Interfaces:**
- Produces: `AppSettings::setup_completed: bool`

- [x] **Step 1: 失敗するテストを書く**

`crates/core/src/settings.rs` の `mod tests` に追加する。

```rust
#[test]
fn fresh_settings_start_with_setup_pending() {
    let db = Db::open_in_memory().unwrap();
    assert!(!AppSettings::load(&db).unwrap().setup_completed);
}

#[test]
fn legacy_settings_without_the_setup_flag_count_as_completed() {
    let settings: AppSettings = serde_json::from_str(r#"{"language":"ja"}"#).unwrap();
    assert!(settings.setup_completed);
}

#[test]
fn setup_completed_roundtrips() {
    let db = Db::open_in_memory().unwrap();
    let settings = AppSettings {
        setup_completed: true,
        ..Default::default()
    };
    settings.save(&db).unwrap();
    assert!(AppSettings::load(&db).unwrap().setup_completed);
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core settings::tests::fresh_settings_start_with_setup_pending`
Expected: `no field setup_completed` でコンパイル失敗。

- [x] **Step 3: フィールドを実装する**

`AppSettings` の `ai_model_license_consent_at` の後に追加する。

```rust
/// Whether the first-run setup was completed or dismissed. Older settings
/// without this field belong to an existing install and must not show the
/// wizard again, so serde defaults to `true`; a brand-new install uses
/// `Default::default()` (false).
#[serde(default = "default_true")]
pub setup_completed: bool,
```

`Default for AppSettings` の `ai_model_license_consent_at: None,` の後に `setup_completed: false,` を追加する。

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core settings::tests::`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/core/src/settings.rs
git commit -m "feat: persist whether the first-run setup was completed"
```

---

### Task 2: Setup の状態モデルと表示判定を作る

**Files:**
- Create: `crates/ui/src/app/setup.rs`
- Modify: `crates/ui/src/app.rs`

**Interfaces:**
- Produces: `SetupPage::{next, prev, is_last}`, `SetupState`, `should_show_setup(...)`

- [x] **Step 1: 失敗するテストを書く**

`crates/ui/src/app/setup.rs` に、まず型とテストだけ置く。

```rust
#[cfg(test)]
mod tests {
    use super::{SetupPage, should_show_setup};

    #[test]
    fn setup_pages_advance_and_return_in_order() {
        use SetupPage::*;
        let pages = [Welcome, Startup, Folders, Display, Discord, Compression, Theme, AiTagging];
        for pair in pages.windows(2) {
            assert_eq!(pair[0].next(), Some(pair[1]));
            assert_eq!(pair[1].prev(), Some(pair[0]));
        }
        assert_eq!(AiTagging.next(), None);
        assert!(AiTagging.is_last());
        assert_eq!(Welcome.prev(), None);
    }

    #[test]
    fn setup_only_shows_for_a_fresh_install() {
        assert!(should_show_setup(false, true, false, false, false, 0));
        assert!(!should_show_setup(true, true, false, false, false, 0));
        assert!(!should_show_setup(false, false, false, false, false, 0));
        assert!(!should_show_setup(false, true, true, false, false, 0));
        assert!(!should_show_setup(false, true, false, false, false, 3));
        assert!(!should_show_setup(false, true, false, true, true, 0));
    }

    #[test]
    fn an_interrupted_setup_resumes_even_after_photos_are_indexed() {
        assert!(should_show_setup(false, true, false, true, false, 0));
        assert!(should_show_setup(false, true, false, true, false, 3));
        assert!(!should_show_setup(false, true, false, true, true, 3));
    }
}
```

- [x] **Step 2: テストが失敗することを確認する**

`app.rs` の `mod ai;` の並びに `pub(crate) mod setup;` を足してから実行する。

Run: `cargo test -p vrc-photon-ui app::setup::tests`
Expected: 型未定義でコンパイル失敗。

- [x] **Step 3: 型と判定を実装する**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetupPage {
    Welcome,
    Startup,
    Folders,
    Display,
    Discord,
    Compression,
    Theme,
    AiTagging,
}

impl SetupPage {
    pub(crate) fn next(self) -> Option<Self> {
        match self {
            Self::Welcome => Some(Self::Startup),
            Self::Startup => Some(Self::Folders),
            Self::Folders => Some(Self::Display),
            Self::Display => Some(Self::Discord),
            Self::Discord => Some(Self::Compression),
            Self::Compression => Some(Self::Theme),
            Self::Theme => Some(Self::AiTagging),
            Self::AiTagging => None,
        }
    }

    pub(crate) fn prev(self) -> Option<Self> { /* next の逆 */ }

    pub(crate) fn is_last(self) -> bool {
        self.next().is_none()
    }
}

pub(crate) fn should_show_setup(
    demo: bool,
    database_ok: bool,
    settings_load_failed: bool,
    settings_row_present: bool,
    setup_completed: bool,
    library_photo_count: i64,
) -> bool {
    if demo || !database_ok || settings_load_failed {
        return false;
    }
    if settings_row_present {
        !setup_completed
    } else {
        library_photo_count == 0
    }
}

pub(crate) struct SetupState {
    pub(crate) open: bool,
    pub(crate) page: SetupPage,
    pub(crate) focus: FocusHandle,
    pub(crate) focus_pending: bool,
    pub(crate) error: Option<SharedString>,
}

impl SetupState {
    pub(crate) fn new(open: bool, cx: &mut Context<App>) -> Self {
        Self {
            open,
            page: SetupPage::Welcome,
            focus: cx.focus_handle(),
            focus_pending: open,
            error: None,
        }
    }
}
```

セレクト状態は不要。言語はチップ、出力形式はチップ、元画像はスイッチで表現する。

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ui app::setup::tests`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/ui/src/app/setup.rs crates/ui/src/app.rs
git commit -m "feat: model the first-run setup pages and visibility rule"
```

---

### Task 3: セットアップ画面を App に接続する

**Files:**
- Modify: `crates/ui/src/app/setup.rs`
- Modify: `crates/ui/src/app.rs`
- Create: `crates/ui/src/ui/setup.rs`
- Modify: `crates/ui/src/ui/mod.rs`
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`

**Interfaces:**
- Consumes: `SetupPage`, `SetupState`, `should_show_setup` (Task 2)
- Produces: `App::setup_screen(&mut self, window, cx) -> Div`, `App::advance_setup`, `App::back_setup`, `App::dismiss_setup`

- [x] **Step 1: i18n キーを追加する**

`ja.yml`:

```yaml
setup.title.welcome: "VRC PhotoN へようこそ"
setup.language.label: "表示言語"
setup.back: "戻る"
setup.next: "次へ"
setup.done: "完了"
setup.close: "セットアップを閉じる"
```

`en.yml`:

```yaml
setup.title.welcome: "Welcome to VRC PhotoN"
setup.language.label: "Display language"
setup.back: "Back"
setup.next: "Next"
setup.done: "Done"
setup.close: "Close setup"
```

- [x] **Step 2: `App::new` に初回判定と状態を接続する**

```rust
let settings_load = AppSettings::load(&db);
let settings_load_failed = settings_load.is_err();
let mut initial_settings = settings_load.unwrap_or_else(|error| {
    tracing::warn!(%error, "cannot load app settings; using defaults for this frame");
    AppSettings::default()
});
...
let settings_row_present = db.get_setting("app_settings").ok().flatten().is_some();
if settings_load_failed {
    initial_settings.setup_completed = true;
}
let library_photo_count = if settings_row_present {
    0
} else {
    db.count_photos().unwrap_or(0)
};
let setup_open = should_show_setup(
    demo,
    database_ok,
    settings_load_failed,
    settings_row_present,
    initial_settings.setup_completed,
    library_photo_count,
);
let setup = setup::SetupState::new(setup_open, cx);
```

`App` に `pub(crate) setup: setup::SetupState,` を追加し、初期化リストへ `setup,` を入れる。

- [x] **Step 3: オーバーレイ判定と描画を接続する**

`App::overlay_open` の `|| self.tag_edit.open` を `|| self.tag_edit.open || self.setup.open` にする。

`App::render` の `self.sync_photo_context_menu(window, cx);` の直後、シェル構築の前に追加する。

```rust
if self.setup.open {
    if std::mem::take(&mut self.setup.focus_pending) {
        window.focus(&self.setup.focus, cx);
    }
    return self.setup_screen(window, cx);
}
```

`render` の戻り値は既存の `Div` と揃えるため、`setup_screen` は `Div` を返す。

- [x] **Step 4: 遷移と終了を実装する**

```rust
pub(crate) fn advance_setup(&mut self, cx: &mut Context<Self>) {
    if !self.commit_setup_page(cx) {
        cx.notify();
        return;
    }
    match self.setup.page.next() {
        Some(page) => {
            self.setup.page = page;
            self.setup.error = None;
            self.setup.focus_pending = true;
            cx.notify();
        }
        None => self.finish_setup(cx),
    }
}

pub(crate) fn back_setup(&mut self, cx: &mut Context<Self>) {
    if let Some(page) = self.setup.page.prev() {
        self.setup.page = page;
        self.setup.error = None;
        self.setup.focus_pending = true;
        cx.notify();
    }
}

pub(crate) fn dismiss_setup(&mut self, cx: &mut Context<Self>) {
    if self.commit_setup_page(cx) {
        self.finish_setup(cx);
    } else {
        cx.notify();
    }
}

fn commit_setup_page(&mut self, _cx: &mut Context<Self>) -> bool {
    true // Task 4 / Task 6 でフォルダと Webhook を追加する
}

fn finish_setup(&mut self, cx: &mut Context<Self>) {
    self.settings.setup_completed = true;
    if let Err(error) = self.settings.save(&self.db) {
        tracing::warn!(%error, "cannot save the setup completion flag");
        self.toast_error(tr_args!("settings.save_failed", error = error), cx);
    }
    self.setup.open = false;
    self.initial_focus_pending = true;
    cx.notify();
}
```

- [x] **Step 5: ウィザードの骨格を描く**

`ui/setup.rs` に `impl App { pub(crate) fn setup_screen(...) -> Div }` を作る。

- ルート: `v_flex().size_full().bg(rgba(p.glass)).font_family("Noto Sans JP")`
- タイトルバー: `TitleBar::new().h(px(46.)).pl(px(14.)).pr(px(8.))` にアプリグリフ22px、`VRC PhotoN` 11px muted、`div().flex_1()`。ウィンドウ操作は `TitleBar` 標準に任せる。
- 本体: `div().flex_1().min_h_0().flex().items_center().justify_center().p(px(24.))`
- パネル: `v_flex().id("setup-panel").w(px(600.)).h(px(561.)).pt(px(36.)).px(px(38.)).pb(px(28.)).rounded(px(20.)).bg(rgb(p.panel_card)).border_1().border_color(rgba(p.line_soft)).shadow(...)`
- `track_focus(&self.setup.focus)`、`focus_trap("setup", &self.setup.focus)`、`.key_context("setup_modal")`、`on_key_down` で Escape → `dismiss_setup`。
- ヘッダー: タイトル `text_size(px(24.)).font_semibold()`、右に `Button::new("setup-close").icon(Icon::new(IconName::Close)).small().ghost()`。
- ヘッダーと本文の間: `pt(px(30.))`。本文は `flex_1().min_h_0()`。
- フッター: `h_flex().items_center().gap(px(8.))`、左に `div().flex_1()`、戻る（`secondary`）と次へ／完了（`primary`）。どちらも `h(px(38.))`。ページ1は戻るを出さない。最終ページは「完了」。
- ページ本文は Task 4 以降で埋める。この時点ではタイトルと空の本文、フッターだけでよい。

- [x] **Step 6: ビルドとテスト**

Run: `cargo test -p vrc-photon-ui app::setup::tests && cargo build -p vrc-photon-ui`
Expected: PASS。

- [x] **Step 7: コミット**

```bash
git add crates/ui/src/app/setup.rs crates/ui/src/app.rs crates/ui/src/ui/setup.rs \
  crates/ui/src/ui/mod.rs crates/i18n/locales/ja.yml crates/i18n/locales/en.yml
git commit -m "feat: show the first-run setup shell over a minimal titlebar"
```

---

### Task 4: ようこそ・起動・フォルダの3ページを実装する

**Files:**
- Modify: `crates/ui/src/ui/setup.rs`
- Modify: `crates/ui/src/app/setup.rs`
- Modify: `crates/ui/src/ui/settings.rs`
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`

**Interfaces:**
- Consumes: `SetupState` (Task 3)
- Produces: `App::save_library_paths(&mut self, cx) -> bool`, `App::pick_setup_folder(field, window, cx)`

- [x] **Step 1: i18n キーを追加する**

```yaml
# ja.yml
setup.folders.title: "フォルダ"
setup.folders.change: "変更"
```

```yaml
# en.yml
setup.folders.title: "Folders"
setup.folders.change: "Change"
```

起動ページの3項目は既存キー（`settings.startup.autostart` / `tray` / `reduce_motion`）を使う。`reduce_motion` の実際の文言は「アニメーションを減らす」。

- [x] **Step 2: パス保存処理を抽出する**

`ui/settings.rs` の保存ボタンのインライン処理を `pub(crate) fn save_library_paths(&mut self, cx: &mut Context<Self>) -> bool` に移す。既存の挙動（保存失敗でロールバック、ルート変更で `apply_library_snapshot_with_anchor`、パス変更で `DaemonCommand::Rescan` と `send_wake_commands`）をそのまま維持する。保存失敗時はトーストに加えて、ウィザード表示中なら `setup.error` にも同じメッセージを入れる。設定画面のボタンは成功時に `toast_success(tr!("settings.library.saved"), cx)` を出す。

- [x] **Step 3: フォルダピッカーを追加する**

`app/setup.rs` に `SetupPathField { PhotoRoot, LogDir, ImportDir }` と `pick_setup_folder` を追加する。既存の `file_picker::pick_folder` と `cx.spawn_in` パターン（`ui/compression.rs` の profile output picker と同じ）を使い、選択結果を `photo_root_input` / `log_dir_input` / `import_dir_input` の `set_value` に書く。取り込み先の初期値は `settings.import_dir()`（既定 `%LOCALAPPDATA%\vrc-photon\imported`）を使う。

- [x] **Step 4: ページ本文を描く**

- ページ1: `setup.language.label` 13px と、`vrc_photon_i18n::locales()` のチップ（`日本語` / `English`）。選択中のチップは `accent_soft` 背景＋`accent` 文字。クリックで `set_language`。タイトルは `setup.title.welcome`。
- ページ2: `settings.startup.title` をタイトルにし、13px ラベル＋右寄せスイッチの行を `gap(px(18.))` で3つ。
  - 自動起動: `settings.autostart` を更新して保存。
  - トレイ常駐: `settings.tray` を更新して保存。
  - アニメーションを減らす: `settings.ui_reduce_motion` を更新し、`set_reduce_motion`（`pub(crate)` 化）を呼ぶ。
- ページ3: `setup.folders.title`。3行を `gap(px(18.))` で並べ、各行は13px ラベル、`Input`（高さ34、`flex_1`）、`変更` ボタン（高さ34）。入力は既存の3つの `InputState` を使う。

- [x] **Step 5: ページ離脱時の保存を接続する**

`App::commit_setup_page` に `SetupPage::Folders => self.save_library_paths(cx)` を追加する。失敗時はページに留まり、`setup.error` に保存失敗メッセージを入れてフォルダページにインライン表示する（トーストはウィザード終了までキューに残るため）。

- [x] **Step 6: ビルドとテスト**

Run: `cargo fmt --all -- --check && cargo build -p vrc-photon-ui`
Expected: PASS。

- [x] **Step 7: コミット**

```bash
git add crates/ui/src/ui/setup.rs crates/ui/src/app/setup.rs crates/ui/src/ui/settings.rs \
  crates/i18n/locales/ja.yml crates/i18n/locales/en.yml
git commit -m "feat: add the welcome, startup, and folder setup pages"
```

---

### Task 5: 表示とサムネイル・テーマの2ページを実装する

**Files:**
- Modify: `crates/ui/src/ui/setup.rs`
- Modify: `crates/ui/src/app/setup.rs`
- Modify: `crates/ui/src/ui/settings.rs`
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`

**Interfaces:**
- Consumes: `set_thumbnail_size`, `set_main_theme`, `set_accent_theme`, `theme_axis`, `ThemeAxis`, `Slider`, `self.grid_gap_slider`, `self.card_radius_slider`
- Produces: なし

- [x] **Step 1: i18n キーを追加する**

```yaml
# ja.yml
setup.display.title: "表示とサムネイル"
setup.display.preview: "プレビュー"
setup.display.preview_live: "設定は即時反映"
setup.display.preview_thumbnail: "サムネイル %{size}"
```

```yaml
# en.yml
setup.display.title: "Display and thumbnails"
setup.display.preview: "Preview"
setup.display.preview_live: "Changes update the preview"
setup.display.preview_thumbnail: "Thumbnails %{size}"
```

- [x] **Step 2: 表示ページを2カラムで描く**

本文を `h_flex().items_start().gap(px(14.))` にし、左 240px の設定列と右のプレビュー列に分ける。

左列:

- `settings.display.title`（「表示（ライブラリ・検索）」）13px semibold。
- `settings.display.status.label` / `world.label` / `date.label` / `size.label` を **Switch** で縦に並べる。変更時に `ui_card_show_*` を更新して保存する。
- `card_divider`。
- `settings.display.grid_gap.label` と現在値（`{n} px`）、`Slider::new(&self.grid_gap_slider).w_full()`。既存の subscription が Change で settings を更新し、Release で保存する。
- `settings.display.card_radius.label` と現在値、`Slider::new(&self.card_radius_slider).w_full()`。
- `card_divider`。
- `settings.thumbnail.title` 13px semibold。
- `THUMBNAIL_SIZES` の3つのチップ（`256×144` / `384×216` / `512×288`）。選択中は `accent_soft`。クリックで `set_thumbnail_size(size, cx)`。

右列（プレビュー）:

- `setup.display.preview` 10px muted。
- `panel_deep` 背景・角丸10・1px境界のプレビュー面に、2枚のミニカードを `h_flex().gap(px(self.settings.ui_grid_gap))` で並べる。
- ミニカードは幅100px。角丸は `settings.ui_card_radius`。画像は `linear_gradient` のサンプル（2枚で色違い）、高さ60px。
- `ui_card_show_status` が true のときだけ画像左上に「圧縮済」バッジ（`tr!("photos.badge.compressed")`）。
- `ui_card_show_world` が true のときだけワールド名（サンプル: `VRChat Home` / `The Black Cat`）。
- `ui_card_show_date` / `ui_card_show_size` に応じて `2026/11/15 · 1.2 MB` の行を出す。両方 false なら行ごと出さない。
- プレビュー下に `setup.display.preview_thumbnail` を `size = format!("{}×{}", w, h)` で出し、右に `setup.display.preview_live` を出す。
- プレビューは App の再描画で `self.settings` から毎回組み立てるため、スイッチ・スライダー・解像度の変更に即時追従する。

- [x] **Step 3: テーマページを既存 `theme_axis` で描く**

- タイトル `settings.theme.title`。
- `self.theme_axis(ThemeAxis::Main, cx)` と `card_divider(&p)` と `self.theme_axis(ThemeAxis::Accent, cx)` を縦に並べる。`theme_axis` は設定画面と同じ18色×2軸、6色×3行、30px スウォッチ、選択ラベル、フォーカス対応をそのまま使う。
- `theme_axis` を `ui/settings.rs` で `pub(crate)` にする。`ThemeAxis` は既に `pub(crate)`。

- [x] **Step 4: ビルドとテスト**

Run: `cargo fmt --all -- --check && cargo build -p vrc-photon-ui`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/ui/src/ui/setup.rs crates/ui/src/app/setup.rs crates/ui/src/ui/settings.rs \
  crates/i18n/locales/ja.yml crates/i18n/locales/en.yml
git commit -m "feat: add the display preview and full theme catalog to setup"
```

---

### Task 6: Discord・自動圧縮の2ページを実装する

**Files:**
- Modify: `crates/ui/src/ui/setup.rs`
- Modify: `crates/ui/src/app/setup.rs`
- Modify: `crates/ui/src/app/discord.rs`
- Modify: `crates/ui/src/ui/integrations.rs`
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`

**Interfaces:**
- Consumes: `validate_webhook_url`, `save_discord_webhook`, `webhook_error_key`, `set_discord_enabled`, `save_discord_destination`, `set_discord_include_players`, `set_discord_send_original`, `set_auto_compress_enabled`, `edit_feature_compression`, `compression_policy_warning`
- Produces: `App::save_setup_webhook(&mut self, cx) -> bool`, `App::set_setup_keep_original`, `App::set_setup_immediate`

- [x] **Step 1: i18n キーを追加する**

```yaml
# ja.yml
setup.discord.title: "Discord"
setup.discord.webhook_label: "Webhook URL"
```

```yaml
# en.yml
setup.discord.title: "Discord"
setup.discord.webhook_label: "Webhook URL"
```

自動圧縮ページの文言は既存キーを使う（`integrations.auto.title`、`settings.startup.immediate`、`settings.startup.keep_original`、`integrations.compression.section`、`compression.lossless`）。

- [x] **Step 2: Discord ページを描く**

- タイトルは `setup.discord.title`（「Discord」）。
- 13px ラベル＋スイッチの行。ラベルは `integrations.discord.title`（「Discord 連携」）、値は `settings.discord_enabled`。変更時に `set_discord_enabled`。
- `card_divider`。
- `setup.discord.webhook_label` 13px と `Input::new(&self.discord_url_input).h(px(38.))`。`setup.error` があれば下に赤11pxで出す。
- `integrations.discord.destination.label` 13px と `Input::new(&self.discord_destination_input).h(px(34.))`。Enter/Blur で `save_discord_destination`。
- `integrations.discord.players.label` のスイッチ（`set_discord_include_players`）。
- `integrations.discord.source.prefer_original` のスイッチ（`set_discord_send_original`）。
- モックでは項目を見せるため Discord を ON 表示しているが、実際の初期値は OFF。OFF のときもこのページでは項目を表示し、スイッチだけが OFF の状態にする（設定画面の折りたたみは持ち込まない）。

- [x] **Step 3: Webhook 保存を実装する**

`app/setup.rs` に純粋な検証ヘルパーと保存処理を追加する。

```rust
pub(crate) fn setup_webhook_value(raw: &str) -> Result<Option<String>, &'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None); // 空欄は変更なし
    }
    validate_webhook_url(raw)
        .map(|parsed| Some(parsed.as_str().to_string()))
        .map_err(webhook_error_key)
}
```

`save_setup_webhook` はこのヘルパーを使い、`Ok(None)` は何もせず true、`Ok(Some(url))` は `save_discord_webhook`、`Err(key)` は `setup.error` に `tr(key)` を入れて false を返す。`webhook_error_key` を `pub(crate)` にする。保存成功時のトーストは出さない。

テストで空欄 → `Ok(None)`、HTTP URL → `Err`、正しい HTTPS URL → `Ok(Some(url))` を固定する。

- [x] **Step 4: 自動圧縮ページを描く**

- タイトルは `integrations.auto.title`。
- `settings.startup.auto_compress`（「自動圧縮」）のスイッチ（`settings.auto_compress`、`set_auto_compress_enabled` を `pub(crate)` 化）。
- `settings.startup.immediate`（「新規写真をすぐに圧縮」）のスイッチ。`auto_compress_immediate` を更新して保存する。
- `settings.startup.keep_original`（「自動圧縮後に元画像を保持」）のスイッチ。`auto_compress_keep_original` を更新して保存する。OFF のときは `compression_policy_warning` を出し、`red_soft` 背景の警告ボックスを表示する。
- `card_divider`。
- `integrations.compression.section`（「圧縮設定」）13px semibold。
- `[Jpeg, Webp, Avif, JpegXl]` のチップを `format_label` で並べ、選択中は `accent_soft`。クリックで `edit_feature_compression(FeatureCompressionTarget::Auto, |o| o.set_format(format), cx)`。
- `compression.lossless` のスイッチ。`edit_feature_compression(..., |o| o.lossless = value, cx)`。
- 自動圧縮が OFF のときは、即時圧縮・元画像保持・圧縮設定を 0.5 不透明度にし、操作を無効化する（`integration_settings_body` と同じ見た目）。

- [x] **Step 5: ページ離脱時の保存を接続する**

`App::commit_setup_page` に `SetupPage::Discord => self.save_setup_webhook(cx)` を追加する。自動圧縮ページは各操作で即時保存するため、離脱時の処理は不要。

- [x] **Step 6: ビルドとテスト**

Run: `cargo fmt --all -- --check && cargo test -p vrc-photon-ui app::setup::tests && cargo build -p vrc-photon-ui`
Expected: PASS。

- [x] **Step 7: コミット**

```bash
git add crates/ui/src/ui/setup.rs crates/ui/src/app/setup.rs crates/ui/src/app/discord.rs \
  crates/ui/src/ui/integrations.rs crates/i18n/locales/ja.yml crates/i18n/locales/en.yml
git commit -m "feat: add the Discord and automatic compression setup pages"
```

---

### Task 7: AIタグ付けページとライセンスモーダルの重なりを実装する

**Files:**
- Modify: `crates/ui/src/ui/setup.rs`
- Modify: `crates/ui/src/app/setup.rs`
- Modify: `crates/ui/src/app.rs`

**Interfaces:**
- Consumes: `install_ai_plugin`, `request_ai_model_download`, `save_ai_runtime_settings`, `ai.active`, `ai.license_modal_open`, `ai_license_overlay`

- [x] **Step 1: AI ページを描く**

- タイトルは `integrations.ai.title`（「AIタグ付け」）。
- `integrations.ai.title` ラベルのスイッチ（`settings.ai_tagging_enabled`）。変更時に `save_ai_runtime_settings(cx, |s| s.ai_tagging_enabled = value)`。
- `card_divider`。
- `Button::new("setup-ai-install").label(tr!("settings.ai.install")).secondary().w_full().h(px(38.))` → `install_ai_plugin(window, cx)`。
- `Button::new("setup-ai-download").label(tr!("settings.ai.download")).secondary().w_full().h(px(38.))` → `request_ai_model_download(window, cx)`。`self.ai.active.is_none()` のときは `.disabled(true)` とし、0.45 不透明度にする。
- `card_divider`。
- `integrations.ai.auto`（「新規画像を自動タグ付け」）のスイッチ（`settings.ai_tagging_auto`）。`save_ai_runtime_settings` で保存。
- バックエンド・閾値・variant 管理はこのページに置かない（設定画面に残す）。

- [x] **Step 2: ライセンスモーダルをセットアップの上に描く**

`setup_screen` の末尾で、`self.ai.license_modal_open` なら `self.ai_license_overlay(window, cx)` をルートに追加する。`setup_screen` の戻り値を `Div` のままにするため、`let mut root = ...; if ... { root = root.child(...); } root` の形にする。

`request_ai_model_download` はライセンス同意が必要なら `open_ai_license_modal` を呼ぶ。同意後は `accept_ai_model_license` が `start_ai_model_download` を呼び、セットアップ画面の上で進捗が更新される。

- [x] **Step 3: オーバーレイ判定を確認する**

`App::overlay_open` は `setup.open` を含むため、セットアップ中のグローバルショートカットは無効。`ai.license_modal_open` は既存の `ai_overlay_open` で無効化される。追加の変更は不要。

- [x] **Step 4: ビルドとテスト**

Run: `cargo fmt --all -- --check && cargo build -p vrc-photon-ui`
Expected: PASS。

- [x] **Step 5: コミット**

```bash
git add crates/ui/src/ui/setup.rs crates/ui/src/app/setup.rs crates/ui/src/app.rs
git commit -m "feat: add the AI tagging setup page"
```

---

### Task 8: ドキュメント同期・旧モック整理・最終検証

**Files:**
- Modify: `docs/ui.md`
- Modify: `docs/data-model.md`
- Modify: `docs/superpowers/plans/2026-11-15-issue-489-first-run-setup.md`
- Add: `docs/ui-489-first-run-setup-mockup.html`（承認済みモック。未コミットならこのタイミングで追加）

- [x] **Step 1: `docs/ui.md` に仕様節を追加する**

「2.12 初回セットアップ」を追加し、次を書く。

- 表示条件: `setup_completed = false` かつ設定行なし・写真0・デモでない。
- 8ページの順序と各ページの項目（実装の設定画面の項目名で記載）。
- 600×561、46px タイトルバー下の中央、シェル非表示、アニメーションなし。
- ×/Esc/完了で `setup_completed` を保存し、メイン画面へ移る。
- フォルダと Webhook はページ離脱時に保存し、不正 URL は留まる。
- 表示ページのプレビューは4スイッチ・グリッド間隔・角丸・サムネイル解像度に追従する。
- テーマは設定画面と同じ18色×2軸を使う。
- 初回に含めない高度設定の一覧。

- [x] **Step 2: `docs/data-model.md` を更新する**

設定 JSON の一覧に `setup_completed` を追加し、次の既定を書く。

- 新規インストール: `false`
- `setup_completed` を持たない旧設定: `true`（既存ユーザーにウィザードを出さない）

- [x] **Step 3: 旧モック PR をクローズする**

`gh pr close 648 --comment "新しい8ページのデザインと承認済みモックに置き換わるためクローズします。実装は feat/489-first-run-setup で進めます。"`

ブランチは削除しない（ユーザーの作業記録として残す）。

- [x] **Step 4: 計画のチェックボックスを更新する**

完了したタスクを `- [x]` にする。

- [x] **Step 5: 全体検証**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bash scripts/check-docs-drift.sh
```

Expected: すべて成功。

- [x] **Step 6: 視覚確認（Playwright 不使用）**

1. `/tmp/opencode/setup-qa.db` を削除し、空の `VRC_PHOTON_DIR` / `VRC_LOG_DIR` を用意する。
2. `WAYLAND_DISPLAY` を外して X11/XWayland で起動し、ウィンドウ ID を `_NET_CLIENT_LIST` から取って ImageMagick `import -window <id>` でキャプチャする。
3. ページ1を承認済みモックと比較する。パネルが 600×561、中央、タイトル・言語チップ・フッターの位置が一致することを確認する。
4. ページ2〜8は、ローカルビルドでのみ初期ページを一時的に変更してキャプチャし、確認後に元へ戻す（デバッグ用フラグはコミットしない）。表示ページはスイッチ・スライダー・解像度を操作してプレビューが追従することを確認する。
5. 完了後に再起動し、モーダルが出ないこと、DB の `app_settings` に `"setup_completed": true` があることを確認する。
6. 既存 DB（写真あり）で起動し、モーダルが出ないことを確認する。

- [x] **Step 7: コミット**

```bash
git add docs/ui.md docs/data-model.md docs/ui-489-first-run-setup-mockup.html \
  docs/superpowers/plans/2026-11-15-issue-489-first-run-setup.md
git commit -m "docs: describe the first-run setup flow"
```

---

## Self-Review

- 仕様の「初回のみ」「既存ユーザー除外」「完了/閉じるでメイン画面」は Task 1〜3 で実装・テストする。
- 8ページの内容は Task 4〜7 で実装する。設定項目は承認済みモックと `docs/ui.md` §2.9/§2.10 に一致させる。
- 表示ページのプレビューは App の再描画で `settings` から組み立て、操作に即時追従する（Task 5）。
- テーマは既存 `theme_axis` を再利用し、18色×2軸の完全なカタログを表示する（Task 5）。
- AI のモデルDLはプラグイン導入後だけ有効にし、ライセンスモーダルはセットアップの上に重ねる（Task 7）。
- 新規文言はすべて ja/en 両方に追加する。`crates/i18n/tests/source_keys.rs` が `tr!` / `tr_args!` の欠落を検出する。
- 視覚確認は Playwright を使わず、Task 8 の X11 キャプチャで行う。
- 旧モック PR #648 はクローズし、ブランチは残す。
