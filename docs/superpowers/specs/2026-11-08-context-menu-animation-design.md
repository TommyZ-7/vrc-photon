# 右クリックメニュー開くアニメーション: 設計

- Date: 2026-11-08
- Status: Draft（サブメニュー方式の承認待ち）
- Related: #428, #432（承認済みモック）, #415
- Approved mock: [docs/ui-context-menu-animation-mockup.html](../../ui-context-menu-animation-mockup.html)

## 1. Intent

写真グリッド／検索結果グリッドの右クリックメニューは現在、開いた瞬間に全表示される。
#428 では、開くときだけ短いフェードとわずかな縦の収まりを加え、操作の邪魔をせずに
「今そこに現れた」ことを伝える。

ユーザー承認済みのモックは **B: フェード＋4px スライド、120ms**。閉じる動きは現行どおり
即時のままとする。メインメニュー、圧縮サブメニュー、右クリック、キーボード起動の
すべてで同じモーションを使う。

## 2. Goals / non-goals

### Goals

- メインメニューが 120ms で `opacity 0 -> 1`、`top -4px -> 0` で開く。
- 圧縮サブメニューも同じ 120ms／4px／イージングで開く。
- 右クリックと `Menu` / `Shift+F10` で同じ動きにする。
- アニメーション中も Esc、外側クリック、フォーカス喪失、スクロール、リサイズで閉じられる。
- 画面端で `anchored()` がアンカーを切り替えても破綻しない。
- 連続して開閉しても、前回の不透明度・位置・アニメーション状態が残らない。
- 既存の「アニメーションを減らす」設定では即時表示にする。
- 既存のメニュー関連テストを維持する。

### Non-goals

- 閉じるアニメーション（現行どおり即時）。
- メニュー項目、レイアウト、文言、有効／無効条件の変更。
- スケール（GPUI の通常要素に transform scale がないため。モックでも不採用）。
- ホバーや選択ハイライトのモーション変更。
- メニュー以外のポップオーバーのモーション変更。

## 3. Current implementation

- `crates/ui/src/app/context_menu.rs`
  - `PhotoContextMenuState` が `PopupMenu` の Entity、表示位置、スナップショット、
    復帰フォーカス、dismiss subscription を持つ。
  - `show_photo_context_menu()` が `PopupMenu::build()` でメニューを組み立て、
    `self.photo_context_menu` に保存する。
  - `photo_context_menu_overlay()` が
    `deferred(anchored().position(state.position).child(state.menu.clone()))`
    を `POPUP_PRIORITY` で返す。
  - `close_photo_context_menu()` は状態を `take()` して即時に行を消す。
- `crates/ui/src/app.rs`
  - `App` が `photo_context_menu: Option<PhotoContextMenuState>` を持つ。
  - `render()` の最後で `photo_context_menu_overlay()` を root に追加する。
- `crates/ui/src/ui/theme.rs`
  - 既存のモーション token（`MOTION_FAST` 160ms、`MOTION_BASE` 220ms など）と
    `ease_motion` がある。
- `gpui-component` 0.9.0
  - `PopupMenu::render()` がメニュー本体の `v_flex()` を組み立てる。
  - サブメニューは `render_item()` の中で、選択されたときに
    `deferred(anchored().child(menu.clone()))` として描画される。
  - `popover.rs` の `dropdown_popup()` が Select / Combobox / DatePicker 用に
    150ms のフェード＋8px スライド＋shadow ramp を既に実装している。
    今回の 120ms／4px は、この既存の dropdown motion より控えめな値。

## 4. Main menu (Phase 1, app-side)

メインメニューはアプリ側のラッパーだけで実装できる。`PopupMenu` の内部描画には触れない。

### 4.1 State

- `App` に `photo_context_menu_epoch: u64` を追加する（初期値 0）。
- `PhotoContextMenuState` に `open_epoch: u64` を追加する。
- `show_photo_context_menu()` の冒頭（空選択の early return の後）で epoch を
  `wrapping_add(1)` し、state に保存する。
- `photo_context_menu_overlay()` のアニメーション ID を
  `("photo-context-menu-open", state.open_epoch)` にする。
  - 同じメニューが開いている間の再 render では ID が変わらないため再生されない。
  - 閉じて開き直すと ID が変わるため、必ず 0 から再生される。

### 4.2 Render

`photo_context_menu_overlay()` を `&self, window: &Window, cx: &Context<Self>` に変更し、
`anchored()` の child を次のラッパーにする。

```rust
let menu = div().child(state.menu.clone()).with_animation(
    ("photo-context-menu-open", state.open_epoch),
    Animation::new(CONTEXT_MENU_OPEN)
        .with_easing(ease_motion)
        .with_max_fps(self.motion_fps(window, cx)),
    |el, t| {
        let (opacity, offset) = context_menu_open_frame(t);
        el.opacity(opacity).relative().top(px(offset))
    },
);
```

- `CONTEXT_MENU_OPEN = 120ms`、`CONTEXT_MENU_SLIDE = 4.0` を `theme.rs` に追加する。
- `context_menu_open_frame(t) -> (opacity, top_offset)` は
  `(t.clamp(0, 1), -4.0 * (1.0 - t))` を返す純粋関数とし、unit test で両端と clamp を固定する。
- `anchored()` はラッパーの外側に置く。位置計算と画面端のアンカー切り替えは現行のまま。
- ラッパーには `occlude()` を付けない。メニュー本体の `PopupMenu` が持つ `occlude()` と
  フォーカス処理をそのまま使う。
- `cx.defer_in` による開いた後のフォーカス移動は変更しない。
- 閉じる処理は変更しない。exit animation は付けない。

### 4.3 Shadow

`with_animation` の `opacity` はメニュー本体とその `popover_style` の shadow をまとめて
半透明にする。GPUI にはグループ合成がないため、フェード中に shadow がパネルの下から
透けて見える可能性がある。

- Phase 1 の目視 QA では、フェード中の shadow は dark slab として目立たなかった。
  120ms と短いこともあり、このまま採用できる。
- 将来テーマや shadow を変えて目立つ場合の fallback:
  1. 開始 opacity を 0 ではなく 0.2 程度にする。
  2. アプリ側に `popover_shadow` 相当の shadow ramp をコピーし、
     `t^3` で shadow の alpha を上げる（`gpui-component` の `dropdown_popup()` と同じ手法）。
  3. それでも不十分なら、サブメニュー方式 A と同じく上流 hook に寄せる。
- この判断は実装後の目視 QA で行い、設計の再承認は不要とする（見た目の微調整の範囲）。

### 4.4 Edge / rapid open-close

- `anchored()` の `SwitchAnchor` により、画面端ではメニューが上／左に切り替わる。
  4px の相対オフセットはアンカー決定後の描画位置にだけ乗るため、位置計算は変わらない。
- 画面端では開始位置が 4px 上に寄る。ウィンドウ上端でメニューが上に切り替わる場合も、
  4px はウィンドウ内に収まる範囲で確認する。必要なら `CONTEXT_MENU_SLIDE` を 3px に
  下げるが、承認済みモックは 4px なので、まず 4px で確認する。
- 連続開閉は epoch と再マウントで状態がリセットされる。アニメーション中に閉じた場合も
  wrapper ごと unmount されるため、残像は残らない。

### 4.5 Reduce motion

`with_animation` は GPUI の reduce-motion 設定を尊重し、最初のフレームで最終値を採用する。
追加の分岐は不要。既存の `motion_fps` でフレーム上限も既存設定に従う。

## 5. Submenu (Phase 2, decision required)

圧縮サブメニューは `PopupMenu` の内部で、親メニューの `render_item()` が selected のときだけ
`menu.clone()` を描画する。アプリ側からこの描画をラップする公開 API はない。
そのため、同じアニメーションを付ける方法は次の 4 つ。

### A. `PopupMenu` に opt-in の open animation を追加する（推奨）

`gpui-component` の `src/menu/popup_menu.rs` に小さな hook を追加する。

- `PopupMenu` に `open_animation: bool`（または `Option<Duration>`）を追加する。
- public builder `open_animation(bool)`（名前は実装時に調整）を追加する。既定は `false`。
- `render()` の root `v_flex()` を、`dropdown_popup()` と同じ要領で
  `with_animation` する:
  - 120ms、`ease_out_cubic`。
  - `opacity: 0 -> 1`、`relative().top(-4px -> 0)`。
  - `shadow(popover_shadow(ring, t^3))` で shadow の透けを防ぐ。
  - ID は menu entity ごとに一意にする（entity id または連番）。
- submenu にも同じ flag を伝播する。`submenu_with_icon()` で親の flag をコピーするか、
  アプリ側の submenu builder で `open_animation(true)` を呼ぶ。
- アプリ側はメインメニューの builder で `open_animation(true)` を呼ぶ。
  - **A1**: メインも submenu も patched hook に寄せ、Phase 1 のアプリ側ラッパーを外す。
    実装が 1 箇所に集約され、メイン／サブの見た目が完全に一致する。
  - **A2**: メインは Phase 1 のアプリ側ラッパーのまま、submenu だけ patched hook を使う。
    Phase 1 のコードを残せるが、同じモーションの実装が 2 箇所になる。

依存の入れ方:

```toml
[patch.crates-io]
gpui-component = { git = "https://github.com/TommyZ-7/gpui-kit", rev = "<patch-commit>" }
```

- `v0.9.0` tag から fork し、`crates/component/src/menu/popup_menu.rs` だけを変更する。
- branch ではなく commit SHA に固定する。
- 上流に PR を出し、リリースされたら patch を外して `gpui-kit` を上げる。
- リリース build は git dependency を fetch する。`Cargo.lock` に SHA が残る。
- `gpui-component` の license（Apache-2.0 / MIT）と provenance を
  `docs/licenses.md` に追記するかは、実際の配布物に同梱されるかで判断する。

Pros:

- メイン／サブメニューが同じコードで動く。挙動の一致が保証される。
- アプリ側のイベント処理、フォーカス、dismiss、キーボードナビゲーションを変更しない。
- shadow ramp も crate 内の既存 helper で正しく扱える。
- 上流に還元できる。

Cons:

- 上流リリースまで fork/patch 依存が残る。
- 依存の取得元が crates.io から git に変わる。CI / release のネットワーク前提が増える。
- fork の管理（tag 追従、PR、リリース後の削除）が必要。

### B. 圧縮サブメニューをアプリ側で再実装する

圧縮項目を `PopupMenuItem::ElementItem` にし、アプリ所有の flyout を
メインメニューと同じ wrapper で描画する。

再実装が必要なもの:

- hover で開く／離れる、click で toggle、キーボード Right/Left で開閉。
- submenu 内の Up/Down/Enter、Esc、外側クリック。
- 親メニューの dismiss chain、フォーカス復帰、z-order、画面端での左右反転。
- 親 `PopupMenu` の `on_mouse_down_out` は、submenu が親の bounds 外にあると
  親を閉じようとする。現行の `active_submenu()` に相当する状態を親に持たせる必要がある。

Pros:

- 依存変更なし。すべてこのリポジトリ内で完結する。

Cons:

- `PopupMenu` のサブメニュー挙動をかなり重複実装する。
- キーボード、外側クリック、フォーカス、z-order の回帰リスクが高い。
- 既存のメニュー関連テストだけでは検出力が足りず、UI テストを追加する必要がある。
- 実装量に対して得られるものが少ない。

### C. `gpui-component` を vendoring して patch する

`gpui-component` 0.9.0 の crate 全体（約 5.0MB / 286 files）を `vendor/` に置き、
`popup_menu.rs` を patch して `[patch.crates-io] gpui-component = { path = ... }` する。

Pros:

- fork 不要。ビルドはリポジトリ内で完結し、再現性が高い。
- 上流のリリースを待たない。

Cons:

- 第三者の crate 全体をリポジトリに持ち込む。レビュー差分と更新コストが大きい。
- 将来の gpui-kit 更新時に vendored copy の追従が必要。
- license notice / provenance の管理が必要。

### D. スコープを分割する

メインメニューだけ先に実装し、サブメニューは別 issue に送る。

- #428 の受け入れ条件「圧縮サブメニューの展開にも同じアニメーション」を満たさない。
- ユーザーが明示的にスコープ変更を承認した場合だけ選択する。

### Comparison

| 方式 | 見た目の一致 | 回帰リスク | 依存変更 | 保守コスト | 備考 |
|---|---|---|---|---|---|
| A1: patched hook（メインも寄せる） | ◎ | 低 | git patch | 中 | 推奨 |
| A2: patched hook（サブのみ） | ○ | 低 | git patch | 中 | Phase 1 を残す |
| B: アプリ側再実装 | ○ | 高 | なし | 高 | 非推奨 |
| C: vendoring | ◎ | 低 | vendored crate | 高 | fork 不可なら |
| D: スコープ分割 | △ | 低 | なし | 低 | #428 未達 |

## 6. Recommendation

- **Phase 1（メイン）**: アプリ側ラッパーで実装する。依存変更なしで完結し、
  右クリック／キーボード／画面端／連続開閉の受け入れ条件を満たせる。
  実装 PR: #434。
- **Phase 2（サブ）**: **A1** を推奨する。`PopupMenu` に opt-in hook を入れ、
  メインとサブの両方を同じ実装に寄せる。上流に PR を出し、リリース後に patch を外す。
- A1 が不可（fork を増やせない）なら **C**。B は回帰リスクが高く、D は #428 を満たさない。

Phase 1 は #434 として実装・マージ済み。Phase 2 は submenu 方式の決定後に別 PR で進める。
Phase 2 の patch が入ったら、Phase 1 のアプリ側ラッパーを A1 に合わせて削除するか、
A2 として残すかを決める。

## 7. Testing

### Unit tests

- `context_menu_open_frame(0.0) == (0.0, -4.0)`
- `context_menu_open_frame(1.0) == (1.0, 0.0)`
- 0.5 で opacity 0.5、offset -2.0。
- 範囲外の t が clamp される。
- 既存の context menu テスト 7 件＋新規 2 件（frame と duration）が通る。
- `CONTEXT_MENU_OPEN` が 100–150ms の範囲にあること（定数テスト）。

### Local commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### Manual QA

1. 写真カードを右クリック: メニューが 120ms でフェード＋4px スライドして開く。
2. 選択済みカードを `Menu` / `Shift+F10`: 同じ動きで開く。
3. 圧縮サブメニューを hover / click / Right: 同じ動きで開く（Phase 2）。
4. アニメーション中に Esc、外側クリック、スクロール、ウィンドウ非アクティブ化:
   即時に閉じる。
5. 画面右端／下端のカードで右クリック: アンカーが切り替わり、4px の開始位置が
   画面外にはみ出さない。
6. 開閉を素早く繰り返す: 残像、途中の opacity、位置のずれがない。
7. 「アニメーションを減らす」を有効化: 即時表示。
8. ダークテーマでフェード中の shadow が dark slab に見えないか確認する。
9. 既存のメニュー項目、有効／無効、キーボードナビゲーションが変わっていないこと。

スクリーンショットは `gnome-wayland-visual-qa` の手順で取得する。Phase 1 の実装では
XWayland の XTest 合成入力が届かなかったため、debug ビルドに一時的な auto-open フックを
入れ、QA 中だけ `CONTEXT_MENU_OPEN` を 1200ms にしてフェード中の半透明フレームと
最終位置を取得した。フックと 1200ms は QA 後に削除し、最終コミットは 120ms に戻している。
実際の右クリック／キーボード操作はユーザーの手動 QA 項目として残す。

## 8. Risks and accepted trade-offs

- **Shadow slab**: wrapper の opacity が shadow も半透明にする。120ms と短いため
  まず目視で許容範囲を確認する。目立つ場合のみ shadow ramp を追加する。
- **Edge offset**: 4px の開始位置が上端でわずかに寄る。必要なら 3px に下げるが、
  承認済みモックは 4px。まず 4px で確認する。
- **Patched dependency**: A1/C は依存の取得元が変わる。commit SHA 固定、上流 PR、
  リリース後の patch 削除を運用に含める。
- **Submenu regression**: A1 は crate の既存イベント処理を変えないため低リスク。
  B は高リスクなので採用しない。
- **No exit animation**: 現行どおり。閉じる瞬間のチラつきは今回の対象外。

## 9. Open decisions

1. サブメニュー方式: **A1（推奨） / A2 / B / C / D** のどれにするか。
2. A1/A2 の場合、fork は `TommyZ-7/gpui-kit` でよいか。上流 PR を出すか。
3. A1/A2 の場合、shadow ramp を初版から入れるか、目視で必要になってから入れるか。

Phase 1（メイン）は #434 でマージ済みのため、判断は不要。
