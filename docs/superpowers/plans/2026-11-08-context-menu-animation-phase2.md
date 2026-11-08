# 右クリックメニュー開くアニメーション Phase 2（サブメニュー）実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `PopupMenu` に opt-in の開くアニメーションを追加し、メインメニューと圧縮サブメニューを同じ実装で 120ms／4px／shadow ramp にする。

**Architecture:** fork した `gpui-component` の `PopupMenu::render()` に、surface 自身を包む `with_animation` を追加する。アプリは `PopupMenu::open_animation(CONTEXT_MENU_OPEN, px(CONTEXT_MENU_SLIDE))` を 1 回呼び、submenu には `submenu_with_icon()` が設定を伝播する。Phase 1 のアプリ側ラッパーは削除する。

**Tech Stack:** Rust 2024, gpui-kit 0.9.0, gpui-component 0.9.0（fork patch）, git dependency, gpui animation API

**Spec:** `docs/superpowers/specs/2026-11-08-context-menu-animation-design.md`

## Global Constraints

- アニメーションは開くときだけ。閉じるは即時（0ms）。
- 120ms、4px、`ease_out_cubic`、`opacity 0 -> 1`、`top -4px -> 0`。
- shadow は `popover_shadow(ring, t^3)` を初版から入れる。
- メイン／サブ、右クリック／`Menu`・`Shift+F10` で同じ動き。
- 上流 PR は出さない。fork `TommyZ-7/gpui-kit` の branch を commit SHA で固定する。
- メニュー項目、レイアウト、文言、有効／無効条件は変更しない。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` を通す。
- docs とコードは同じ PR に混ぜない。

## Review Focus

- 画面右端／下端でサブメニューが左／上に反転するとき、4px の開始位置が画面外にはみ出さないか。
- 開閉を素早く繰り返したとき、前回の opacity・位置・shadow が残らないか。
- アニメーション中に Esc、外側クリック、フォーカス喪失が正しく閉じるか。
- 親メニューの再 render（hover 移動など）でサブメニューのアニメーションが途中から再生されないか。
- 「アニメーションを減らす」で即時表示になるか。

## File Structure

Fork (`TommyZ-7/gpui-kit`, branch `vrc/popup-menu-open-animation`):

- `crates/component/src/menu/popup_menu.rs` — `open_animation` API、frame helper、render wrapper、submenu 伝播、unit tests。

App (`TommyZ-7/vrc-photon`):

- `Cargo.toml` — `[patch.crates-io] gpui-component` を fork の commit SHA に固定。
- `Cargo.lock` — git source / SHA を反映。
- `crates/ui/src/app/context_menu.rs` — `open_animation` 呼び出し、Phase 1 ラッパー削除。
- `crates/ui/src/app.rs` — `photo_context_menu_epoch` 削除、overlay 呼び出しを元に戻す。
- `crates/ui/src/ui/theme.rs` — `CONTEXT_MENU_OPEN` / `CONTEXT_MENU_SLIDE` を維持。

---

### Task 0: 実装用 worktree を用意する

**Files:**
- None（作業ディレクトリのみ）

- [ ] **Step 1: main を更新して worktree を作る**

```bash
git fetch origin
git worktree add ../vrc-photo-manager-428-sub -b feat/context-menu-submenu-animation origin/main
```

- [ ] **Step 2: worktree で作業する**

以降の app 側の変更は `/home/tommyz7/work/vrc-photo-manager-428-sub` で行う。
fork 側の変更は `/tmp/opencode/gpui-kit-428` で行う。

---

### Task 1: fork と patch branch を作る

**Files:**
- External: `TommyZ-7/gpui-kit` fork, branch `vrc/popup-menu-open-animation`

**Interfaces:**
- Produces: fork の branch と commit SHA（Task 5 で app の `rev` に使う）。

- [ ] **Step 1: fork を作る**

```bash
gh repo fork longbridge/gpui-kit --clone=false
```

- [ ] **Step 2: fork を clone し、v0.9.0 から branch を切る**

```bash
git clone https://github.com/TommyZ-7/gpui-kit /tmp/opencode/gpui-kit-428
cd /tmp/opencode/gpui-kit-428
git fetch --tags origin
git checkout -b vrc/popup-menu-open-animation v0.9.0
```

- [ ] **Step 3: tag と crate の場所を確認する**

Run: `git rev-parse v0.9.0 && test -f crates/component/src/menu/popup_menu.rs && grep -n '^name = "gpui-component"' crates/component/Cargo.toml`
Expected: tag SHA、`popup_menu.rs` の存在、package 名 `gpui-component`。

- [ ] **Step 4: 何も変更せず一度ビルド確認する**

Run: `cargo check -p gpui-component`
Expected: PASS。失敗する場合は環境の問題を先に解消する。

---

### Task 2: frame helper と unit test を追加する（TDD）

**Files:**
- Modify: `crates/component/src/menu/popup_menu.rs`

**Interfaces:**
- Produces:
  ```rust
  pub(crate) fn popup_menu_open_frame(t: f32, offset: Pixels) -> (f32, Pixels, f32)
  ```
  戻り値は `(opacity, top_offset, shadow_strength)`。

- [ ] **Step 1: 失敗する test を書く**

`popup_menu.rs` の `mod tests` に追加する。

```rust
#[test]
fn popup_menu_open_frame_fades_slides_and_ramps_shadow() {
    let offset = px(4.);
    assert_eq!(popup_menu_open_frame(0.0, offset), (0.0, -offset, 0.0));
    assert_eq!(popup_menu_open_frame(1.0, offset), (1.0, px(0.), 1.0));

    let (opacity, top, shadow) = popup_menu_open_frame(0.5, offset);
    assert!((opacity - 0.5).abs() < f32::EPSILON);
    assert_eq!(top, -px(2.));
    assert!((shadow - 0.125).abs() < f32::EPSILON);

    assert_eq!(popup_menu_open_frame(-1.0, offset), (0.0, -offset, 0.0));
    assert_eq!(popup_menu_open_frame(2.0, offset), (1.0, px(0.), 1.0));
}
```

- [ ] **Step 2: test が失敗することを確認する**

Run: `cargo test -p gpui-component popup_menu_open_frame_fades_slides_and_ramps_shadow`
Expected: FAIL（`popup_menu_open_frame` が未定義）。

- [ ] **Step 3: helper を実装する**

```rust
/// Opacity, vertical offset, and shadow strength for the menu's opening frame.
pub(crate) fn popup_menu_open_frame(t: f32, offset: Pixels) -> (f32, Pixels, f32) {
    let t = t.clamp(0.0, 1.0);
    (t, -offset * (1.0 - t), t * t * t)
}
```

- [ ] **Step 4: test が通ることを確認する**

Run: `cargo test -p gpui-component popup_menu_open_frame_fades_slides_and_ramps_shadow`
Expected: PASS。

- [ ] **Step 5: commit する**

```bash
git add crates/component/src/menu/popup_menu.rs
git commit -m "feat(menu): add PopupMenu open animation frame helper"
```

---

### Task 3: `PopupMenu` に opt-in API と submenu 伝播を追加する

**Files:**
- Modify: `crates/component/src/menu/popup_menu.rs`

**Interfaces:**
- Produces:
  ```rust
  impl PopupMenu {
      pub fn open_animation(self, duration: Duration, offset: Pixels) -> Self;
  }
  ```
  `PopupMenu` に `open_animation: Option<(Duration, Pixels)>` と
  `animation_id: u64` を持たせる。

- [ ] **Step 1: 失敗する test を書く**

`mod tests` に、field の opt-in と submenu 伝播を確認する 2 つの test を追加する。

```rust
#[gpui::test]
fn open_animation_is_opt_in(cx: &mut gpui::TestAppContext) {
    let menu = cx.update(|cx| {
        PopupMenu::new(cx).open_animation(Duration::from_millis(120), px(4.))
    });
    assert_eq!(
        menu.open_animation,
        Some((Duration::from_millis(120), px(4.)))
    );

    let plain = cx.update(|cx| PopupMenu::new(cx));
    assert_eq!(plain.open_animation, None);
}

#[gpui::test]
fn submenu_inherits_the_parent_open_animation(cx: &mut gpui::TestAppContext) {
    let (parent, cx) = cx.add_window_view(|window, cx| {
        PopupMenu::new(cx)
            .open_animation(Duration::from_millis(120), px(4.))
            .submenu("More", window, cx, |menu, _, _| menu)
    });
    let (parent_animation, submenu_animation) = cx.update(|cx| {
        let parent = parent.read(cx);
        let Some(PopupMenuItem::Submenu { menu, .. }) = parent.menu_items.first() else {
            panic!("expected a submenu item");
        };
        (parent.open_animation, menu.read(cx).open_animation)
    });
    let expected = Some((Duration::from_millis(120), px(4.)));
    assert_eq!(parent_animation, expected);
    assert_eq!(submenu_animation, expected);
}
```

- [ ] **Step 2: test が失敗することを確認する**

Run:
```bash
cargo test -p gpui-component open_animation_is_opt_in
cargo test -p gpui-component submenu_inherits_the_parent_open_animation
```
Expected: FAIL（`open_animation` が未定義）。

- [ ] **Step 3: field / 採番 / builder を実装する**

```rust
static NEXT_ANIMATION_ID: AtomicU64 = AtomicU64::new(1);

pub struct PopupMenu {
    // ...
    open_animation: Option<(Duration, Pixels)>,
    animation_id: u64,
}

impl PopupMenu {
    pub(crate) fn new(cx: &mut App) -> Self {
        Self {
            // ...
            open_animation: None,
            animation_id: NEXT_ANIMATION_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub fn open_animation(mut self, duration: Duration, offset: Pixels) -> Self {
        self.open_animation = Some((duration, offset));
        self
    }
}
```

`submenu_with_icon()` の `submenu.update(cx, |view, _| { ... })` に
`view.open_animation = self.open_animation;` を追加する。

- [ ] **Step 4: test が通ることを確認する**

Run:
```bash
cargo test -p gpui-component open_animation_is_opt_in
cargo test -p gpui-component submenu_inherits_the_parent_open_animation
```
Expected: PASS。

- [ ] **Step 5: commit する**

```bash
git add crates/component/src/menu/popup_menu.rs
git commit -m "feat(menu): add opt-in PopupMenu open animation"
```

---

### Task 4: `render()` に animation と shadow ramp を適用する

**Files:**
- Modify: `crates/component/src/menu/popup_menu.rs`

**Interfaces:**
- Consumes: `popup_menu_open_frame`, `open_animation`, `animation_id`.
- Produces: `PopupMenu` の root surface が設定時だけ `with_animation` で包まれる。
  `open_animation` が `None` の既存メニューは不変。

- [ ] **Step 1: `render()` の root を組み立てる**

既存の `v_flex()...` を `let surface = ...;` に置き換え、最後に次を返す。

```rust
let surface = if let Some((duration, offset)) = self.open_animation {
    let ring = popover_ring(cx);
    surface
        .with_animation(
            ("popup-menu-open", self.animation_id),
            Animation::new(duration).with_easing(ease_out_cubic),
            move |el, t| {
                let (opacity, top, shadow) = popup_menu_open_frame(t, offset);
                el.opacity(opacity)
                    .relative()
                    .top(top)
                    .shadow(popover_shadow(ring, shadow))
            },
        )
        .into_any_element()
} else {
    surface.into_any_element()
};
surface
```

`render` の戻り値は `AnyElement` にする。imports を追加する:
`Animation`, `AnimationExt as _`, `crate::animation::ease_out_cubic`,
`crate::styled::{popover_ring, popover_shadow}`, `std::time::Duration`。

- [ ] **Step 2: 既存テストと新規テストを通す**

Run: `cargo test -p gpui-component menu::`
Expected: PASS（既存の menu tests を含む）。

- [ ] **Step 3: commit する**

```bash
git add crates/component/src/menu/popup_menu.rs
git commit -m "feat(menu): animate PopupMenu opening with shadow ramp"
```

---

### Task 5: fork branch を push し、app の dependency を固定する

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`

**Interfaces:**
- Consumes: Task 4 の fork commit SHA。
- Produces: `gpui-component` が fork の SHA から解決される。

- [ ] **Step 1: fork branch を push する**

```bash
cd /tmp/opencode/gpui-kit-428
git push -u origin vrc/popup-menu-open-animation
git rev-parse HEAD
```

- [ ] **Step 2: app の `Cargo.toml` に patch を追加する**

```toml
[patch.crates-io]
gpui-component = { git = "https://github.com/TommyZ-7/gpui-kit", rev = "<Task 5 Step 1 の SHA>" }
```

- [ ] **Step 3: lockfile を更新する**

Run: `cargo check -p vrc-photon-ui`
Expected: PASS。`Cargo.lock` の `gpui-component` が `source = "git+https://github.com/TommyZ-7/gpui-kit?rev=..."` になる。

- [ ] **Step 4: commit する**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: pin gpui-component to the PopupMenu animation fork"
```

---

### Task 6: Phase 1 のアプリ側ラッパーを削除し、hook に切り替える

**Files:**
- Modify: `crates/ui/src/app/context_menu.rs`, `crates/ui/src/app.rs`
- Test: `crates/ui/src/app/context_menu.rs`

**Interfaces:**
- Consumes: `PopupMenu::open_animation(Duration, Pixels)`（Task 3）。
- Produces: アプリ側の `photo_context_menu_epoch` / `open_epoch` /
  `context_menu_open_frame` を削除し、`PopupMenu::build` の戻り値に
  `.open_animation(CONTEXT_MENU_OPEN, px(CONTEXT_MENU_SLIDE))` を付ける。

- [ ] **Step 1: `context_menu.rs` の builder に hook を付ける**

`PopupMenu::build(window, cx, move |menu, window, cx| { ... })` の closure 内で、
`menu.action_context(...)` のチェーンに
`.open_animation(CONTEXT_MENU_OPEN, px(CONTEXT_MENU_SLIDE))` を追加する。
**`submenu_with_icon()` より前**に呼ぶこと。`submenu_with_icon()` は呼ばれた時点の
親の `open_animation` を子へコピーするため、後ろに置くと伝播しない。

```rust
menu.action_context(action_context)
    .min_w(px(252.))
    .open_animation(CONTEXT_MENU_OPEN, px(CONTEXT_MENU_SLIDE))
    .submenu_with_icon(...)
```

submenu 側の追加呼び出しは不要。

- [ ] **Step 2: Phase 1 の state と wrapper を削除する**

- `PhotoContextMenuState.open_epoch` を削除。
- `App.photo_context_menu_epoch` と init を削除。
- `show_photo_context_menu()` の epoch increment を削除。
- `photo_context_menu_overlay()` を元の `&self` signature に戻し、
  `deferred(anchored().position(state.position).child(state.menu.clone()))` に戻す。
- `app.rs` の `render()` 呼び出しを `self.photo_context_menu_overlay()` に戻す。
- `context_menu_open_frame` とその test を削除。
- `CONTEXT_MENU_OPEN` / `CONTEXT_MENU_SLIDE` と duration test は残す。

- [ ] **Step 3: fmt / clippy / test**

Run:
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
Expected: PASS。

- [ ] **Step 4: commit する**

```bash
git add crates/ui/src/app/context_menu.rs crates/ui/src/app.rs crates/ui/src/ui/theme.rs
git commit -m "feat(ui): use the PopupMenu open animation for submenus"
```

---

### Task 7: 手動 QA と PR

**Files:**
- No code changes.

- [ ] **Step 1: メインメニューの再確認**

`--demo` で起動し、右クリック／`Menu`・`Shift+F10` でメインメニューが
120ms で開くことを確認する。

- [ ] **Step 2: サブメニューの確認**

「圧縮」を hover / click / Right で開き、サブメニューが同じ 120ms／4px で
開くことを確認する。画面右端／下端で左右・上下の反転を確認する。

- [ ] **Step 3: クローズ条件と連続開閉**

アニメーション中に Esc、外側クリック、スクロール、ウィンドウ非アクティブ化。
素早く開閉を繰り返し、残像・不透明度・位置のずれがないこと。

- [ ] **Step 4: reduce motion と shadow**

「アニメーションを減らす」で即時表示。ダークテーマで shadow が dark slab に
見えないこと。

- [ ] **Step 5: PR を作成する**

```bash
git push -u origin feat/context-menu-submenu-animation
gh pr create --title "feat(ui): animate the compression submenu opening" --label enhancement
```

PR 本文に、fork SHA、`cargo` 検証、手動 QA の結果、上流 PR を出さない方針を記録する。

---

## Self-Review

- [x] Spec の A1 / fork-only / shadow ramp を Task 1–4 で実装する。
- [x] Phase 1 の削除を Task 6 に含めた。
- [x] 画面端、連続開閉、キーボード、reduce motion、shadow を Review Focus と QA に入れた。
- [x] 型名と API 名（`open_animation`, `popup_menu_open_frame`, `animation_id`）を統一した。
- [x] docs とコードの PR を分ける。
