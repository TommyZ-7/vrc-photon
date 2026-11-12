# スクロールバーの取り付け方式とトラック表示の統一 (#604) 設計

> 状態: モック承認待ち
> モック: `docs/ui-604-scrollbar-mockup.html`
> 対象 Issue: #604

## 目的

標準スクロールバーのトラック（背景）を全テーマで非表示にし、サムだけを表示する。
あわせて、設定・連携・自動で残っている `Scrollbar::vertical` の直接配置を、
App が保持する `ScrollHandle` を維持したまま wrapper 方式へ揃える。

写真グリッド右端の年レールは、スクロールバーではなく年ナビゲーションとして独立した
要素であるため、今回の変更対象から外す（ユーザー決定 2026-11-12）。

## 現状

- 通常のスクロール領域は `overflow_y_scrollbar()`（wrapper 方式）を使用している。
  対象はダッシュボード、通知モーダル、検索フィルター、ダイアログ本体、
  写真インスペクタ内、圧縮プロファイル一覧、AI タグ一覧、取り込み結果など。
- `Scrollbar::vertical` の直接配置は次の3箇所だけ。
  - `crates/ui/src/ui/settings.rs`: `settings_scroll` を App が保持し、8 px の
    右ガターに直接配置している。
  - `crates/ui/src/ui/integrations.rs`: `integrations_scroll` を App が保持し、
    詳細ペインの右端 8 px ガターに直接配置している。
  - `crates/ui/src/ui/compression.rs`: `queue_scroll`（`VirtualListScrollHandle`）を
    App が保持し、仮想リストの親に `.viewport_from_layout()` で直接配置している。
- スクロールバーのトラック色は、`crates/ui/src/ui/palette.rs` の
  `apply_component_theme` が `theme.colors.scrollbar` に設定している。
  現在は `palette.border_strong` を割り当てており、全スクロールバーの
  通常・ホバー・ドラッグ状態のトラックとして描画される。
- 写真グリッドの年レールは `crates/ui/src/ui/year_rail.rs` が独自に描画する。
  2 px の `line_soft` 線、年ドット、年ラベル、現在位置の月ピルを持ち、
  クリック・ドラッグで年ジャンプする。

## 決定事項

ユーザーが選んだ方針（2026-11-12）。

| # | 判断項目 | 決定 | 補足 |
|---|---|---|---|
| Q1 | 写真グリッドの年レール | 一切変更しない | 2 px の線、年ドット、年ラベル、月ピル、当たり判定を現状維持する。年レールはスクロールバーではなく年ナビゲーションとして扱う。 |
| Q2 | サムの色・幅 | 現状のまま | 通常 6 px / `muted_2`、ホバー・ドラッグ 8 px / `muted`。トラックだけを消す。 |
| Q3 | 設定・連携の直接配置 | wrapper 方式へ変更 | App の `ScrollHandle` は維持する。`overflow_y_scrollbar()` は新しいハンドルを作るため使わず、外側ラッパーへ `.vertical_scrollbar(&handle)` を付ける。 |
| Q4 | 圧縮キューの直接配置 | 直接配置の例外として残す | `VirtualList` は `ScrollableElement` 非対応。`VirtualListScrollHandle` を共有する現在の形を維持する。 |
| Q5 | 年レールと Issue の受け入れ条件 | 年レールをスコープ外にする | Issue #604 の「写真グリッドの独自オーバーレイもトラックを表示しない」は今回の対象外とし、承認後に Issue のスコープを更新する。 |

実装者が決めた事項。

| # | 判断項目 | 決定 | 理由 |
|---|---|---|---|
| Q6 | トラックの透明化方法 | `theme.colors.scrollbar` を `transparent_black()` にする | gpui-kit のテーマ投影がこの色を全スクロールバーのトラックへ割り当てている。アプリ側で個別の `ScrollbarStyles` を持たず、全画面を一度に揃えられる。 |
| Q7 | 設定・連携のラッパー位置 | スクロール領域そのものではなく、外側の非スクロールコンテナに付ける | `.vertical_scrollbar` は絶対配置の子を追加する。スクロール領域に付けるとバーが内容と一緒にスクロールするため、外側の `h_flex` / 詳細ペインに付ける。 |
| Q8 | 8 px ガターの扱い | 削除する | 現在のガターはバーの当たり判定や幅を決めていない（バーはハンドルのビューポート境界を使う）。外側ラッパーにバーを付ければ同じ位置に描画される。 |
| Q9 | トラックのヒット領域 | 現状の 16 px を維持する | 見た目だけを変え、ホイール・トラッククリック・ドラッグの操作範囲は変えない。 |
| Q10 | 圧縮キューのコメント | 例外理由をコードに残す | 将来 `VirtualList` 対応の wrapper が追加されたときに、なぜ直接配置なのかを判断できるようにする。 |

## 仕様

### 標準スクロールバー

- トラックは通常・ホバー・ドラッグのすべてで描画しない。
- サムは現状の色・幅・角丸を維持する。
  - 通常: 幅 6 px、`palette.muted_2`
  - ホバー: 幅 8 px、`palette.muted`
  - ドラッグ: 幅 8 px、`palette.muted`
- 表示モードは変更しない。スクロール中に表示し、停止後にフェードアウトする。
- トラックの 16 px 領域はレイアウト・当たり判定ともに維持する。背景だけを描かない。
- ダーク・ライト・パステルの全テーマに適用する。テーマごとの個別指定はしない。

### 取り付け方式

- 通常のスクロール領域は `overflow_y_scrollbar()` を原則とする。既存の呼び出しは変更しない。
- App が `ScrollHandle` を保持する設定・連携・自動は、外側の非スクロールコンテナに
  `.vertical_scrollbar(&handle)` を付ける。スクロール領域は従来どおり
  `.overflow_y_scroll().track_scroll(&handle)` を使う。
- 圧縮キューは `VirtualList` の制約により、
  `Scrollbar::vertical(&queue_scroll).viewport_from_layout()` の直接配置を例外とする。
  この例外はコードコメントと `docs/ui.md` に記録する。
- 写真グリッドの年レールは変更しない。

## 実装方針

1. `crates/ui/src/ui/palette.rs`
   - `apply_component_theme` の `theme.colors.scrollbar` を
     `transparent_black().into()` に変更する。
   - 「トラックは描かず、サムだけを表示する」旨のコメントを残す。
2. `crates/ui/src/ui/settings.rs`
   - `use gpui_kit::component::scroll::Scrollbar;` を削除する。
   - `use gpui_kit::component::scroll::ScrollableElement as _;` を追加する。
   - `settings_screen` の外側 `h_flex()` に `.vertical_scrollbar(&self.settings_scroll)` を付ける。
   - 右端 8 px ガターの `div().absolute()...` と `Scrollbar::vertical` を削除する。
3. `crates/ui/src/ui/integrations.rs`
   - `Scrollbar` の import を削除する（`ScrollableElement` は他の
     `overflow_y_scrollbar()` で使うため残す）。
   - `detail` の `h_flex()` に `.vertical_scrollbar(&self.integrations_scroll)` を付ける。
   - 右端 8 px ガターの `div().absolute()...` と `Scrollbar::vertical` を削除する。
4. `crates/ui/src/ui/compression.rs`
   - 機能変更なし。直接配置の直前に、`VirtualList` が wrapper API 非対応である
     例外理由のコメントを追加する。
5. `docs/ui.md`
   - シェル節に「スクロールバー」を追加し、取り付け方式・トラック非表示・
     圧縮キューの例外・年レールの扱いを記録する。

## 非対象

- 写真グリッドの年レール（`crates/ui/src/ui/year_rail.rs`、
  `crates/ui/src/state/year_rail.rs`）の見た目・当たり判定・年ジャンプ。
- サムの色・幅・角丸・フェード時間の変更。
- スクロールバーのトラック幅や当たり判定の変更。
- 圧縮キューの仮想リスト方式の変更、または gpui-kit の fork / 依存更新。
- 設定・連携以外の画面を `Scrollbar::vertical` から別方式へ移すこと。
- テーマごとのトラック色の個別設定。
- Issue #604 の年レール項目の実装。

## テスト

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- 手動確認（`--demo`）:
  - 設定画面でトラックが表示されず、サムだけが表示される。
  - 設定画面をスクロールし、別画面へ移動して戻ったときにスクロール位置が保持される。
  - 連携・自動の詳細ペインで同じ確認を行う。マスター・ディテールの積み重ね表示でも確認する。
  - 圧縮キューのサムが表示され、ドラッグ・ホイールでスクロールできる。
  - ダッシュボード、通知モーダル、検索フィルター、ダイアログ、写真インスペクタで
    トラックが表示されない。
  - ダイアログ内の入れ子スクロール（本体とプロファイル一覧）で、両方ともトラックが表示されない。
  - ダーク・ライト・パステルの各テーマで確認する。
  - 写真グリッドの年レールが従来どおり表示され、クリック・ドラッグでジャンプできる。
- 回帰確認:
  - スクロールバーのホバーでサムが太くなる。
  - スクロール停止後にサムがフェードアウトする。
  - 設定・連携の再描画後もスクロール位置が飛ばない。

## リスク

- `.vertical_scrollbar` を外側ラッパーに付けるため、ラッパーとスクロール領域の境界が
  一致しないとバーの位置がずれる。設定は外側 `h_flex`、連携は詳細ペインの
  `h_flex` で、どちらもスクロール領域が `flex_1` で同じ境界になることを確認する。
- 8 px ガターを削除すると、バーの描画位置がわずかに変わる可能性がある。
  現在のバーはハンドルのビューポート境界を使うため、外側ラッパーの境界と一致するかを
  実機で確認する。
- `Theme::update` のトークン再調整が `colors.scrollbar` を上書きする可能性がある。
  既存の `colors.*` 割り当てと同じ経路であり、テーマ変更後にトラックが再表示されないことを
  ダーク・ライト・パステルで確認する。
- gpui-kit の将来の更新で、コンポーネントが独自のトラック色を設定する可能性がある。
  その場合はアプリ側のテーマ投影で上書きできるかを都度確認する。
- 年レールを対象外にすることで、Issue #604 の受け入れ条件を完全には満たさない。
  承認後に Issue のスコープを更新し、実装 PR でその判断を記録する。

## Issue との差分

Issue #604 の受け入れ条件には「写真グリッドの独自オーバーレイスクロールバーも、
トラックを表示しない見た目に揃える」が含まれる。今回のユーザー決定により、
年レールは年ナビゲーションとして一切変更しないため、この項目はスコープ外とする。
モック承認後に Issue #604 へ判断をコメントし、必要なら本文の受け入れ条件を更新する。
