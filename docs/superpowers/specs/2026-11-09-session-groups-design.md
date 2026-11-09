# セッション単位のグループ表示 設計 (#452)

## 目的

ライブラリの写真を、VRChat のワールド参加から退出まで（セッション）単位で
グループ表示する。Photos 画面に表示モード「セッション」を追加し、写真グリッドの
流れに見出し行を差し込む。新しい収集処理や新しいデータは持たず、既存の
`sessions` / `session_players` / `photo_context.session_id` をそのまま使う。

モック: `docs/ui-session-groups-mockup.html`

## 対象

- Photos 画面の表示モード切替（時系列 / セッション）。
- セッション見出し（カバー、ワールド名、滞在時刻、枚数、同席プレイヤー、
  折りたたみ）と、未紐付け写真の日付ラベル。
- 見出し行と写真行を混在させる仮想リストの行モデル。
- core の読み取り API（セッション要約）と、軽量索引への `session_id` 追加。
- ギャラリーヘッダー、年レール、スクロールアンカーのモード対応。
- 表示モードのアプリ設定への保存。
- QA 用のデモデータへのセッション追加。

## 対象外

- 検索結果画面へのセッション表示（初版は Photos のみ。判断ポイント 1）。
- セッションの手動編集（結合・分割・写真の割当て変更）。
- セッション単位の一括操作（圧縮・送信・タグ付けなど）。写真単位の既存操作のみ。
- ログのない期間のセッション再構成。
- `photo_context.session_id` がない写真を、撮影時刻の重なりでセッションへ寄せること。
- 見出しの固定表示（sticky）。初版は通常の行としてスクロールさせる。
- daemon / IPC のプロトコル・メッセージ形の変更。UI は従来どおり同一プロセスの
  `Db` を直接読む。

## 前提となる既存の仕組み

- デーモンのログ解析が `sessions`（開始・終了時刻、ワールド、インスタンス）を作成し、
  `photo_context.session_id` で写真と結び付けている。
- 同席プレイヤーは `session_players`（参加・退出時刻つき）にセッション単位で記録されている。
- 写真の索引は `Db::photo_index` が返す軽量スナップショットで、現在は
  `id` / `captured_at_utc` / `captured_month` だけを持つ。
- グリッドは `uniform_list` で固定高の写真行を仮想化している。年レール、
  ページング、スクロールアンカー、キーボード移動はこの固定高を前提にしている。

## 実装方式の比較

| 案 | 内容 | 評価 |
|---|---|---|
| A（採用） | Photos 画面にモードを追加し、セッションモードだけ可変高さ仮想リスト（`v_virtual_list`）で見出し行と写真行を混在させる。時系列モードは既存の `uniform_list` を維持する。 | 見出しを自然な高さで差し込め、既存の時系列モードの回帰リスクを抑えられる。 |
| B | 独立したセッション画面を追加する。 | 画面遷移が増え、選択・右クリック・詳細表示・年レールの共通部品を使いにくい。却下。 |
| C | `uniform_list` のまま、見出しを写真行と同じ高さの 1 行として差し込む。 | 見出しがカード 1 行分の高さになり、情報密度が悪い。却下。 |
| D | 時系列・セッションの両モードを `v_virtual_list` に移行する。 | スクロール・年レール・ページングの既存挙動を広く作り直すことになり、回帰リスクが大きい。却下。 |

## 表示仕様

### モード切替

- Photos 画面のギャラリーヘッダーに「時系列 / セッション」のセグメントを置く。
- 既定は時系列。選択は `AppSettings.ui_photos_view_mode` に保存し、再起動後も復元する。
- 切替時は、現在のモードで先頭に見えている写真を記録し、新しいモードでも
  その写真を含む行が先頭に来るようにスクロールを合わせる。
- 検索結果画面は時系列のまま（判断ポイント 1）。検索結果のカード部品・操作は共通。

### グルーピング

- 並びは全体を通して新しい順（現行の `PhotoSort::Newest` と同じ）。
  セッション内も新しい順で統一する。
- `photo_context.session_id` が同じ写真の連続区間を 1 セッションとしてまとめる。
  セッションの先頭（新しい側）に見出し行を置く。
- `session_id` がない写真は、時系列の位置のまま、日付ラベル行を付けて表示する。
  日付はローカル撮影日（`captured_at_local` の日付）で区切り、疑似セッションにはまとめない。
- 同じ `session_id` の写真が未紐付け写真などで分断された場合は、分断ごとに
  同じ見出しを繰り返さず、最初の見出しの後は写真行だけを続ける。実データでは
  通常発生しない（判断ポイント 6）。
- 写真が 1 枚もないセッションは索引に現れないため、見出しも出さない。

### セッション見出し

- 高さは固定（写真カードのサイズ設定に依存しない）。カバー写真、ワールド名、
  滞在時刻の範囲、写真枚数、同席プレイヤー、折りたたみボタンを 1 行にまとめる。
- カバー写真は「お気に入りの最新 → 最新写真」の順で選ぶ。ワールド別ブラウズ
  （#456）と同じ規則に揃える。カバーがない場合は色付きのプレースホルダー。
- ワールド名は既存の表示名解決（`worlds.alias` → `worlds.name` →
  `worlds.world_id`）を使う。解決できない場合は「不明なワールド」。
- 滞在時刻はローカル時刻で `11/9 21:14 –`、`11/8 22:03 – 23:47` のように表す。
  日付をまたぐ場合は `11/7 23:41 – 11/8 0:26` と両日を出す。
  終了時刻のないセッションは「滞在中」バッジを付ける。
- 写真枚数は可視写真（`present` / `replaced`）だけを数える。
- 同席プレイヤーは `session_players` の参加順に、先頭 3 名まで＋「他n名」。
  0 名なら「同席 なし（ソロ）」。表示名は 1 行で省略する。
- 見出しの折りたたみは、チェブロンのクリックで切り替える。折りたたみ中は
  見出しだけを残し、写真行を描かない。状態は保存せず、画面を離れる・索引を
  再読込すると展開に戻る（判断ポイント 2）。
- 折りたたみ中にキーボードで隠れた写真へ移動した場合は、そのセッションを
  自動で展開して選択を見せる。写真の選択・タグ・右クリックメニューには影響しない。
- 見出し自体は写真ではないため、選択・ダブルクリック・右クリックメニューの対象外。
  折りたたみボタンだけが操作対象。

### 未紐付け写真の日付ラベル

- `session_id` のない連続区間を、ローカル撮影日ごとに分ける。
- ラベルは `11/7 ・ セッションに属さない写真 6枚` のように、日付と枚数を出す。
- 日付が不明な写真（`captured_month == 0` 相当）は「日付不明」にまとめる。
- 未紐付け区間が複数に分かれても、同じ日付ラベルを繰り返してよい。
  写真の時系列位置を動かさないことを優先する。

### 写真カード・操作

- 写真カードは時系列モードと同じ部品を使う。カードサイズ、ワールド名・日時・
  サイズの表示設定、圧縮・重複バッジ、選択表示、右クリックメニュー、
  ダブルクリックの詳細表示は共通。
- ページ単位の読み込み（256 枚）、失敗ページのインラインパネルと再試行も共通。
- Shift 範囲選択、Ctrl/Cmd 複数選択、全選択、キーボード移動は既存の索引順
  （写真の並び）をそのまま使う。セッション見出しは選択対象にしない。

### 年レール

- セッションモードでも年レールを表示する（判断ポイント 5）。
- 年バケットの位置は写真枚数の比率で決め、時系列モードと同じ見た目を保つ。
- 年クリック時は、その年の最初の写真を含む写真行の表示位置へスクロールする。
  見出し行・日付ラベル行の高さを含む行モデルでオフセットを計算する。
- スクロール中の月ピルは、先頭に見えている写真の撮影月を表示する。

### ギャラリーヘッダー

- 左側は時系列モードと同じ「月・枚数 / 全体枚数」を表示する。セッションモードでも
  先頭に見えている写真の月を使う。
- 右側にモード切替、フィルター、並び（新しい順）を置く。
- 見出しの折りたたみやモード切替でギャラリーヘッダーの高さは変えない。

## データ取得

### 軽量索引への追加

`PhotoIndexEntry` に次の 2 項目を追加する。

| 項目 | 内容 |
|---|---|
| `captured_day` | ローカル撮影日（YYYYMMDD、不明は 0）。未紐付け写真の日付ラベルに使う。 |
| `session_id` | `photo_context.session_id`。NULL は未紐付け。 |

- `Db::photo_index` と `Db::photo_index_text_limited` の SELECT に
  `PHOTO_DAY` と `c.session_id` を追加する。
- `PHOTO_DAY` は `captured_at_local` → `captured_at_utc` の順で
  `strftime('%Y%m%d', ...)` し、失敗時は 0。
- 既存の `PhotoIndexEntry` の構築箇所（テストフィクスチャを含む）を更新する。
- 検索結果も同じ型を使うが、セッションモードの表示には使わない。

### セッション要約 API

core に読み取り API を 1 本追加する。

```rust
pub struct SessionSummary {
    pub session: Session,
    pub world_name: Option<String>,
    pub world_external_id: Option<String>,
    pub photo_count: i64,
    pub players: Vec<String>,
    pub cover_photo: Option<Photo>,
}

impl Db {
    pub fn session_summaries(&self, ids: &[i64]) -> Result<Vec<SessionSummary>>;
}
```

- 入力 ID は重複を除き、最大 256 件に制限する（表示範囲のページ単位で呼ぶため）。
  上限を超えた分は切り捨てる。空なら空を返す。
- `sessions` と `worlds` を結合し、`world_name` は
  `COALESCE(w.alias, w.name, w.world_id, '')`、`world_external_id` は `w.world_id`。
- `photo_count` は `photo_context.session_id` と可視写真（`present` / `replaced`）の
  集計。0 件のセッションも返す（UI は索引に現れないため表示しない）。
- `players` は `session_players` を `joined_at, id` 順に並べた `display_name`。
- `cover_photo` はセッション内の可視写真から
  `is_favorite DESC, captured_at_utc DESC, id DESC` の先頭。
  `Photo` は既存の `PHOTO_SELECT` と同じ列を返し、UI がサムネイル要求に使えるようにする。
- 返す順序は `started_at, id` 昇順。存在しない ID は無視する。

### 表示範囲ごとの読み込み

- UI はセッション行モデルを索引全体から組み立てる。行モデルは `session_id` と
  `captured_day` だけで作れる。
- 見出しのメタデータ（ワールド名・時刻・枚数・同席者・カバー）は、仮想リストの
  可視行に現れたセッション ID についてだけ、ページ単位（最大 2 バッチ）で
  バックグラウンド取得する。
- 取得中・失敗中は同じ ID を再要求しない。索引の generation が変わったら
  要約キャッシュと失敗状態をリセットする。
- 取得は既存のページ読み込みと同じ方法で、読み取り専用接続を開いて行う。
  開けない場合はライブ接続へフォールバックする。
- 要約が届くまで見出しは固定高のプレースホルダー（枚数は索引から出せる）を描き、
  届いたらワールド名・時刻・同席者・カバーを差し替える。行の高さは変えない。
- 失敗した見出しは「セッション情報を取得できません」と表示し、索引の再読込まで
  自動再試行しない。

## UI 実装

### 行モデル

`crates/ui/src/state/session_rows.rs`（新規）に純粋な行モデルを置く。

```rust
pub(crate) enum SessionRowKind {
    Header { session_id: i64, photo_count: usize, collapsed: bool },
    Photos { first_photo: usize, count: usize },
    DateLabel { day: i32, photo_count: usize },
}

pub(crate) struct SessionRows {
    rows: Vec<SessionRow>,
    heights: Vec<f32>,
    origins: Vec<f32>,
    photo_rows: Vec<(usize /* first_photo */, usize /* row */)>,
    photo_count: usize,
}
```

- `SessionRows::build(index, columns, photo_row_height, header_height, collapsed)`:
  - 連続する `session_id` ごとに Header 行＋写真行（折りたたみ時は写真行なし）。
  - `session_id == None` の連続区間は `captured_day` ごとに DateLabel 行＋写真行。
  - `heights` は行の高さ（写真行は既存の `GridMetrics.row_height`、
    見出し行は固定高＋行間、日付ラベル行は固定高＋行間）。
  - `origins` は先頭からの累積。`v_virtual_list` の item_sizes と同じ値にする。
  - `photo_rows` は写真行の先頭写真 index と行番号。写真 index から行を
    バイナリサーチできるようにする。
- 便利メソッド:
  - `row_count()`, `heights()`, `top_photo_index(scroll_top)`,
    `offset_for_photo(photo_index)`, `offset_for_session(session_id)`,
    `row_for_photo(photo_index)`, `session_at_row(row)`。
- 同じ入力（generation / columns / 写真行高 / 見出し高 / 折りたたみ集合）では
  再構築しない。`PhotoBrowser` がキーでキャッシュする。

### PhotoBrowser の追加状態

- `session_scroll: VirtualListScrollHandle`
- `session_rows: Option<Arc<SessionRows>>` と構築キー
- `collapsed_sessions: HashSet<i64>`
- `session_summaries: HashMap<i64, Arc<SessionSummary>>`
- `session_summary_pending: HashSet<i64>` / `session_summary_failed: HashSet<i64>`
- `replace_index` で行モデル・要約・折りたたみ状態をリセットする。
- タイムライン用の `scroll` / `pages` / `labels` はそのまま残す。
- モードに応じたスクロール操作を App 側で切り替えるための小さなヘルパーを置く。

### ギャラリー

- `GalleryView::render` は検索画面なら従来どおり、Photos 画面なら設定のモードで分岐する。
- セッションモード:
  - `v_virtual_list` に `SessionRows` の高さを渡す。
  - 見出し行・日付ラベル行・写真行をそれぞれ描く。
  - 写真行は既存の `render_card` を再利用する。
  - 可視範囲の見出し ID を集めて要約取得を計画し、ページ読み込み・サムネイル要求も
    既存と同じ deferred パスで行う。
  - 失敗ページのインラインパネルは写真行の中に既存ロジックで差し込む。
- 時系列モードは既存の `render_grid` をそのまま使う。

### スクロール・アンカー・キーボード

- 現在モードの先頭写真を返す `top_photo_id` 相当の処理を App に置き、
  カードサイズ変更・モード切替・索引再読込時のアンカーに使う。
- モード切替時は、切替前の先頭写真を新しいモードの行に写してスクロールする。
- キーボードの上下移動は写真 index の `columns` 移動のまま。選択先が
  折りたたみ中のセッションなら展開してからスクロールする。
- `scroll_grid_to_first_page` はモードに応じて先頭行へ戻す。

### 年レール

- `YearRail` のスクロール対象を、時系列の `UniformListScrollHandle` と
  セッションの `VirtualListScrollHandle` + `SessionRows` の両方に対応させる。
- 年バケットの位置計算は写真 index の比率のまま。スクロール位置の変換だけを
  行モデル経由にする。
- 既存のヒットテスト・月ピル・ドラッグ・ホイール透過は共通。

## 設定

`crates/core/src/settings.rs` に次を追加する。

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PhotosViewMode {
    #[default]
    Timeline,
    Sessions,
}
```

- `AppSettings.ui_photos_view_mode: PhotosViewMode` を追加する。
- 既存の設定 JSON に項目がない場合は `Timeline`。
- 不明な値（将来の削除・改称）は `Timeline` にフォールバックし、設定全体の
  パースを失敗させない（`ui_card_size` と同じ扱い）。
- 設定画面には項目を追加しない。ギャラリーヘッダーの切替が保存の入口。

## i18n

`crates/i18n/locales/ja.yml` と `en.yml` に追加する。

- モード名: `session.mode.timeline` / `session.mode.sessions`
- 見出し: `session.photo_count`, `session.ongoing`, `session.players`,
  `session.players.none`, `session.players.more`
- 未紐付け: `session.unlinked_date`（日付＋枚数）
- 折りたたみ: `session.collapse`, `session.expand`
- 不明: `session.unknown_world`, `session.summary_failed`, `date.unknown_day`
- 必要なら `date.month_day`（`11/7` 形式）。時刻は数値フォーマットを Rust 側で行う。

`tr_args!` を使うキーは `crates/i18n/tests/placeholders.rs` の `CALL_SITES` に追加する。

## テスト

### core

- `photo_index` が `session_id` とローカル撮影日を返す。
- `session_summaries` がワールド名（alias → name → world_id）、可視写真枚数、
  参加順プレイヤー、お気に入り優先のカバー写真を返す。
- 存在しない ID、可視でない写真、写真 0 件のセッションを正しく扱う。
- ID の重複・上限（256 件）を守る。
- 既存の `photo_index` / `photo_index_text` の並び・月のテストが通る。

### UI state

- セッションの連続区間が見出し＋写真行になる。
- 未紐付け区間がローカル日付ごとに日付ラベル＋写真行になる。
- 折りたたみで写真行が消え、展開で戻る。
- 見出し・日付ラベルの高さを含む `offset_for_photo` と
  `top_photo_index` が往復する。
- 写真 index から行を引くバイナリサーチが、見出し行をまたいでも正しい。
- 索引 generation が変わると要約キャッシュ・失敗状態・行モデルがリセットされる。
- 年レールのジャンプがセッション行のオフセットを使う。

### UI / 手動

- `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- `cargo run -p vrc-photon-ui --bin vrc-photon -- --demo --demo-reset` で
  1280×820 / 1440×900 を確認する。
- 確認項目: モード切替、先頭写真アンカー、セッション見出し、日付またぎ、
  未紐付け、折りたたみ、年レール、選択・Shift 範囲選択・右クリック・
  ダブルクリック、カードサイズ変更、再起動後のモード復元。

## リスク

- **仮想リストの移行**: セッションモードだけ `v_virtual_list` を使うが、
  スクロールハンドル・アンカー・年レールの計算が固定高前提から外れる。
  行モデルの純粋テストと、時系列モードを触らない分岐でリスクを抑える。
- **要約取得の遅延**: 見出しのメタデータが後から届くため、行の高さを固定し、
  プレースホルダーでもレイアウトが動かないようにする。
- **セッションの分断**: 同一 session_id が未紐付け写真で分断される異常系。
  見出しを繰り返さない規則にし、テストで固定する。
- **年レールのズレ**: 見出し行を含むオフセット計算を誤ると、年クリックや
  月ピルがずれる。`offset_for_photo` / `top_photo_index` の往復テストを置く。
- **ライブ更新**: セッション終了時に ended_at が変わっても、写真索引が
  変わらなければ見出しは「滞在中」のままになり得る。既存の 3 秒ライブ更新で
  開いているセッションの要約を再取得するか、次回の索引再読込で更新する。
  初版は後者を基本とし、必要ならライブ更新を追加する。

## 受け入れ条件

- Photos 画面で「時系列 / セッション」を切り替えられ、セッションモードでは
  セッション見出しと写真行が新しい順に混在する。
- 見出しにワールド名、滞在時刻（日付またぎ・滞在中を含む）、枚数、
  同席プレイヤー（3 名＋他 n 名）が出る。
- 未紐付け写真は時系列の位置のまま日付ラベル付きで表示される。
- 見出しを折りたたむと写真行が消え、展開すると戻る。
- 写真の選択・右クリック・詳細表示・年レールが時系列モードと同じように動く。
- モードの選択が再起動後も復元される。
- 検索結果画面の表示・操作に変化がない。
- `cargo fmt` / `clippy` / `test` が通る。

## 判断ポイント（決定待ち）

| # | 項目 | 推奨 | 代替 |
|---|---|---|---|
| 1 | 検索結果への適用 | Photos のみ（初版） | 検索結果にも適用 |
| 2 | 見出しの折りたたみ | 初版に含める（状態は保存しない） | 初版は含めない |
| 3 | モードの保持 | アプリ設定に保存する | 起動ごとに時系列へ戻す |
| 4 | XMP のみの写真の帰属 | `session_id` の有無だけで判定し、寄せない | 撮影時刻の重なりで寄せる |
| 5 | 年レール | セッションモードでも表示し、行モデルに合わせる | セッションモードでは隠す |
| 6 | セッションが分断された場合 | 見出しは最初の 1 回だけ | 分断ごとに見出しを繰り返す |

推奨案でモックを作成している。承認後に本設計の「判断ポイント」を確定し、
実装計画を更新する。

## 参考

- Issue: [#452 写真をセッション（ワールド参加回）単位でグループ表示する](https://github.com/TommyZ-7/vrc-photon/issues/452)
- 宗一郎のモック・方針案: `/tmp/opencode/452/saeki-mock.html`, `/tmp/opencode/452/design.md`
- ワールド別ブラウズ（カバー写真の規則を揃える）: [#456](https://github.com/TommyZ-7/vrc-photon/issues/456)
- 既存のデータモデル: `docs/data-model.md` の `sessions` / `session_players` / `photo_context`
- 既存の写真グリッド: `crates/ui/src/ui/gallery.rs`, `crates/ui/src/state/photo_browser.rs`
