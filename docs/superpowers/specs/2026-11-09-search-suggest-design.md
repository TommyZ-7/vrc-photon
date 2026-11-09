# 検索入力の候補表示 設計 (#446)

## 目的

検索画面のワールド・プレイヤー・タグの各入力欄に、入力中の文字列で
部分一致する候補をフローティングリストとして表示する。候補には写真枚数を
併記し、上下キーと Enter / Esc で操作できるようにする。

モック: `docs/ui-search-suggest-mockup.html`

## 対象

- `crates/ui/src/ui/search.rs` の検索フィルタパネル
  （ワールド／プレイヤー／タグの3入力欄）。
- 候補取得のための core クエリ（`crates/core/src/db.rs`）。
- タグ欄のテキスト検索を、手動タグ名に加えて**適用済み AI タグ名**にも
  一致させる変更（下記「AI タグの扱い」）。
- 検索画面の AI タグ説明文を「タグ欄とは別条件」に修正する。

## 対象外

- Photos 画面など、検索画面以外の入力欄への展開。
- 候補の履歴（最近使った条件）の保持。
- 候補選択後の自動検索。検索は従来どおり「検索」ボタンで実行する。
- 一致部分のハイライト、候補のページング、10件を超える候補の追加表示。
- daemon / IPC のプロトコル・メッセージ形の変更。UI は従来どおり
  同一プロセスの `Db` を直接読む。共有フィルタの意味の変化は
  「共有フィルタへの影響」に記載する。

## AI タグの扱い

現状のタグ欄は `photo_tags` の手動タグにしか一致せず、AI タグは
`search_ai_tags` のチップで別条件として指定する。issue の「タグ候補は
手動タグと AI タグの両方を対象とする」を満たすため、タグ欄のテキスト
条件を次の両方に一致させる。

- 手動タグ: `tags.name`
- 適用済み AI タグ: `ai_tag_definitions.name` / `name_en` / `tag_key`
  （`photo_ai_tags.applied = 1` かつ `plugin_id = 'vrc-photon-autotag'`）

候補リストでは手動タグと AI タグを同じ名前空間で扱い、同名の候補は
1行にまとめて写真枚数を合算する。AI タグの表示名は UI のロケールに
合わせ、英語ロケールでは `name_en` を優先する。候補の部分一致は
表示名に対して行う。

AI タグチップ（`search_ai_tags`）は従来どおり独立した AND 条件として
残す。説明文の「手動タグとは別条件です」は、タグ欄が AI タグにも
一致するようになるため「タグ欄とは別条件です」に改める。

## UI 仕様

### 表示

- 入力欄の直下に 4px の間隔でフローティング表示する。フィルタカードの
  内側にインラインで押し込まず、`deferred(anchored(...))` で window
  座標に描画する。下の項目を押し出さず、パネルのスクロール領域でも
  切れない。
- 幅は入力欄と同じ。行高は 27px、上下パディング 4px。
- 行は「名前（左、省略表示）＋写真枚数（右、`format_count`）」。
  枚数は `search.count` と同じ桁区切りに合わせる。
- 選択行は `accent_soft` の背景と左端のアクセントバーで示す。
- 候補が画面下端を超える場合は入力欄の上側に表示し、window 内に収める。
- 候補が空、または入力が空／空白のみなら何も表示しない。
- 最大10件。スクロールは不要（10行 × 27px + パディング）。

### 取得

- 入力変更から 120ms のデバウンス後に、検索と同じ方法でバックグラウンド
  クエリを実行する。読み取り専用ハンドルを開けない場合は、検索と同様に
  ライブ接続へフォールバックする。
- 入力変更ごとに世代番号を進め、古い結果は破棄する。
- フォーカス時に既存の入力値があれば候補を再取得する。
- 候補の元データは可視写真（`present` / `replaced`）のみ。
- AI タグの表示名は `current_locale()` が `en` で始まるとき `name_en` を
  優先する。

### キーボード / マウス

- キー操作は `App` 起動時に登録する `cx.intercept_keystrokes` で処理する。
  gpui-kit の `Input` は `up` / `down` / `enter` / `escape` をアクション
  として先に解決するため、要素の `capture_key_down` では間に合わない。
  interceptor はアクション解決より前に呼ばれ、`cx.stop_propagation()` で
  入力欄の既定動作を止められる。
- interceptor は候補表示中かつ対象の入力欄がフォーカスされている場合だけ
  反応する。`Keystroke::is_ime_in_progress()` が真のときは IME の変換操作を
  優先して何もしない。
- `↑` / `↓`: 選択行を移動。端では止まる（ラップしない）。初期選択は先頭行。
- `Enter`: 選択中の正式名を入力欄へ設定し、候補を閉じる。
- `Esc`: 候補を閉じる。入力値とフォーカスは保持し、次の文字入力まで
  再表示しない。
- 行のマウスダウンで確定する。クリック後のフォーカス移動で候補が先に
  閉じても選択が失われないようにする。
- 入力欄の外側のマウスダウン、画面遷移、検索実行、クリアで閉じる。
- 候補選択後も入力欄にフォーカスを残す。検索は自動実行しない。

### 状態

App に次の候補状態を1つだけ持つ（同時に開くのは1欄のみ）。

- 対象欄（World / Player / Tag）
- 入力文字列
- 取得結果 `Vec<SearchSuggestion>`
- 選択行
- 世代番号
- Esc で閉じたか
- 入力欄とポップアップの bounds

入力欄の bounds は、各フィールドを包む `canvas` から更新する。
外側クリックは、ポップアップ表面の `on_mouse_down_out` で閉じる。

## Core API

`crates/core/src/model.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchSuggestion {
    pub name: String,
    pub photo_count: i64,
}
```

`crates/core/src/db.rs` に3つのメソッドを追加する。いずれも `query` を
trim し、空なら空ベクタを返す。`limit` は呼び出し側から受け取り、
SQL の `LIMIT` に渡す。

- `world_name_suggestions(&self, query: &str, limit: usize)`
- `player_name_suggestions(&self, query: &str, limit: usize)`
- `tag_name_suggestions(&self, query: &str, limit: usize, prefer_english: bool)`

### ワールド

`photo_context.world_name` を名前でグループ化し、可視写真の
`COUNT(DISTINCT p.id)` を返す。NULL と空文字は除外する。

```sql
SELECT c.world_name AS name, COUNT(DISTINCT p.id) AS photo_count
FROM photo_context c
JOIN photos p ON p.id = c.photo_id
WHERE p.status IN ('present', 'replaced')
  AND c.world_name IS NOT NULL AND c.world_name <> ''
  AND c.world_name LIKE ? ESCAPE '\'
GROUP BY c.world_name
ORDER BY photo_count DESC, name COLLATE NOCASE ASC
LIMIT ?
```

### プレイヤー

`photo_players.display_name` を名前でグループ化する。同じ表示名が複数の
`user_id` にまたがる場合は1行にまとめ、写真の重複を除いて数える。

```sql
SELECT pp.display_name AS name, COUNT(DISTINCT pp.photo_id) AS photo_count
FROM photo_players pp
JOIN photos p ON p.id = pp.photo_id
WHERE p.status IN ('present', 'replaced')
  AND pp.display_name IS NOT NULL AND pp.display_name <> ''
  AND pp.display_name LIKE ? ESCAPE '\'
GROUP BY pp.display_name
ORDER BY photo_count DESC, name COLLATE NOCASE ASC
LIMIT ?
```

### タグ

手動タグと AI タグを `UNION ALL` した候補に対して、表示名でグループ化
する。AI タグ名は `prefer_english` で `name_en` / `name` を選ぶ。
同名の手動・AI タグは1行にまとまり、同じ写真は重複して数えない。

```sql
WITH candidates(name, photo_id) AS (
    SELECT t.name, pt.photo_id
    FROM tags t JOIN photo_tags pt ON pt.tag_id = t.id
    UNION ALL
    SELECT CASE WHEN ?english THEN COALESCE(d.name_en, d.name) ELSE d.name END,
           at.photo_id
    FROM photo_ai_tags at
    JOIN ai_tag_definitions d
      ON d.plugin_id = at.plugin_id AND d.tag_key = at.tag_key
    WHERE at.applied = 1 AND at.plugin_id = 'vrc-photon-autotag'
)
SELECT c.name, COUNT(DISTINCT c.photo_id) AS photo_count
FROM candidates c
JOIN photos p ON p.id = c.photo_id
WHERE p.status IN ('present', 'replaced')
  AND c.name <> ''
  AND c.name LIKE ? ESCAPE '\'
GROUP BY c.name
ORDER BY photo_count DESC, c.name COLLATE NOCASE ASC
LIMIT ?
```

`?english` は SQL のプレースホルダではなく、実装時に
`CASE WHEN ?N THEN ...` へバインドする真偽値として扱う。

### タグ検索条件の拡張

`build_text_filter` のタグ条件を次の形にする（既存の手動タグ条件は保持）。

```sql
(
  EXISTS (SELECT 1 FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
          WHERE pt.photo_id = p.id AND t.name LIKE ? ESCAPE '\')
  OR
  EXISTS (SELECT 1 FROM photo_ai_tags at
          JOIN ai_tag_definitions d
            ON d.plugin_id = at.plugin_id AND d.tag_key = at.tag_key
          WHERE at.photo_id = p.id
            AND at.applied = 1
            AND at.plugin_id = 'vrc-photon-autotag'
            AND (d.name LIKE ? ESCAPE '\'
                 OR COALESCE(d.name_en, d.name) LIKE ? ESCAPE '\'
                 OR d.tag_key LIKE ? ESCAPE '\'))
)
```

LIKE のメタ文字は既存の `escape_like` を使う。

### 共有フィルタへの影響（MCP / daemon）

`build_text_filter` は UI の `photo_index_text` と、daemon / MCP の
`search_photos_text` で共有されている。タグ条件の拡張により、MCP の
`search_photos` の `tag` パラメータも手動タグ名と適用済み AI タグ名の
両方に一致するようになる。IPC のメッセージ形・フィールドは変更しないが、
検索結果の意味は広がる。

- MCP ツールの説明を「manual tag」から「manual tag or applied AI tag
  text」へ更新する。
- MCP の検索テストに、AI タグ名で `tag` 検索が一致するケースを追加する。
- `docs/` の MCP 利用説明があれば同じ表現に揃える。

## UI 実装

- `crates/ui/src/app/search_suggest.rs`（新規）: 状態、デバウンス、
  バックグラウンド取得、選択移動の純粋関数。
- `crates/ui/src/app.rs`: `App` に候補状態と input 購読を追加。
  `world_input` / `player_input` / `tag_input` の `InputEvent::Change` /
  `Focus` を購読する。`cx.intercept_keystrokes` の戻り値
  `Subscription` も App に保持する。
- `crates/ui/src/ui/search_suggest.rs`（新規）: ポップアップの描画、
  bounds canvas、行のクリック、キーボード処理。
- `crates/ui/src/ui/search.rs`: 3つのフィールドを `field_with_suggestions`
  で包み、パネル末尾で開いている候補を1つ描画する。
- `crates/ui/src/app/search.rs`: `run_search` / `clear_search` /
  画面遷移で候補を閉じる。`refresh_search_after_ai_tag_edit` の判定を
  `ai_filter_is_active(filter) || has_tag_filter(filter)` に広げる。
  タグ欄が AI タグにも一致するため、AI タグの再適用でタグ欄だけの検索も
  古くなるため。
- `crates/i18n/locales/ja.yml`, `en.yml`: `search.ai.any` の文言修正のみ。

## テスト

### core

- 各候補クエリ: 部分一致、大文字小文字、`%` / `_` のエスケープ、
  空クエリ、NULL / 空名の除外。
- 枚数: 可視 status のみ、同じ写真の重複を数えない。
- 並び順: 枚数降順、同数は名前順、`limit` が効く。
- タグ: 手動タグと AI タグの両方が出る。`applied = 0` と別プラグインは
  除外される。同名の手動・AI タグが1行にまとまる。
- タグ検索拡張: 手動タグ名、AI タグの `name` / `name_en` / `tag_key` の
  いずれでも一致する。`applied = 0` は一致しない。`photo_index_text` と
  `search_photos_text` の両方で同じ結果になる。
- 既存のタグ検索テスト（削除したタグが一致しなくなる等）が通る。
- MCP の検索で AI タグ名を `tag` に指定すると一致する。

### ui

- 選択移動が端で止まり、初期選択が先頭になる。
- 入力変更で世代が進み、古い結果を適用しない。
- 空文字・空白のみで候補を閉じる。
- Esc 後は次の入力変更まで再表示しない。
- 候補選択で入力欄の値が正式名になり、候補が閉じる。
- タグ欄だけの検索が適用済みのとき、AI タグ編集後に再検索対象になる
  （`has_tag_filter` を含む更新判定）。

### 手動確認

- デモデータでワールド／プレイヤー／タグの候補表示、キーボード操作、
  外側クリック、下端での上側表示を確認する。
- 既存の検索ボタン、クリア、AI タグチップが従来どおり動くことを確認する。

## リスク

- `LIKE '%...%'` は先頭ワイルドカードのためインデックスが効かない。
  既存の検索と同じ走査になるが、10万件規模ではデバウンスと
  バックグラウンド実行で UI を止めない。実測で問題が残る場合は
  FTS 導入を別 issue とする。
- タグ欄が AI タグにも一致するようになるため、既存ユーザーの検索結果が
  広がる可能性がある。意図した変更としてリリースノートに記載する。
- 入力欄のキー操作を interceptor で横取りするため、候補が閉じている
  ときや IME 変換中に、矢印キーのカーソル移動・変換操作を妨げないことを
  確認する。
- ポップアップの位置計算を誤ると画面外に出る。上側フリップと
  window 内クランプをテスト可能な純粋関数に分離する。
- 候補取得中に画面を離れた場合、古い結果で状態を更新しない
  （世代番号と画面チェック）。

## 受け入れ条件

- 検索画面のワールド／プレイヤー／タグ欄で、入力中に部分一致候補が
  入力欄直下へ最大10件表示される。
- 候補は写真枚数の多い順、同数は名前順で、枚数が併記される。
- タグ候補には手動タグと適用済み AI タグの両方が含まれ、選択した名前で
  検索すると両方のタグに一致する。
- 入力が空なら候補は出ない。
- `↑` / `↓` で選択、`Enter` で正式名を入力欄へ反映、`Esc` で閉じる。
- 候補表示でレイアウトが動かず、フィルタパネルのスクロール領域で
  クリップされない。window の外にはみ出さない。
- daemon / IPC のメッセージ形に変更がない。
- `cargo fmt --all -- --check`、clippy、workspace テストが通る。
