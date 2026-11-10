# MCP ランダム抽出ツール `random_search_photos` 設計

- Issue: #522
- Status: 承認済み
- Date: 2026-11-11

## 目的

MCP サーバーに読み取り専用ツール `random_search_photos` を追加する。`search_photos`
と同じ絞り込み条件で一致する写真から `limit` 件を無作為に抽出して返す。`seed` を
指定した場合は、同じ条件・同じライブラリ内容に対して順序込みで同じ結果を返す。

## ゴール

- MCP の新規ツール `random_search_photos` から、ワールド／プレイヤー／手動タグ・
  AI タグ／撮影日時範囲／お気に入りで絞り込める。
- `limit` は既定 50、1〜200 にクランプする。
- `seed`（任意、`u64`）指定時は決定的。未指定時は呼び出しごとに変わる。
- 一致件数が `limit` 未満なら、一致した全件を無作為順で返す。
- 返却項目は `search_photos` と同じ写真サマリー（ID、表示パス、撮影日時、
  ワールド、プレイヤー）。
- 読み取り専用。daemon 未起動時は既存ツールと同じ `daemon_unavailable`
  ツールエラーになる。

## 非ゴール

- UI の変更。
- MCP サーバーの利用手順ドキュメント（#484 で扱う）。
- ライブラリ内容が変わった後も同じ写真集合を保証すること。決定性は同じ
  ライブラリ内容に対する保証とする。
- 抽出結果のページング（`offset` は持たない）。

## 検討した方式

### A. daemon 側で決定的に抽選する（採用）

条件に一致する写真 ID を取得し、`seed` と写真 ID から擬似乱数キーを作り、
キー順に `limit` 件を選ぶ。同じ `seed` と同じライブラリ内容なら、SQL の並び順に
依存せず同じ結果になる。MCP は IPC を 1 回呼ぶだけで、集合の一貫性も保たれる。

### B. MCP 側で総件数を数え、ランダムな offset で複数回検索する

IPC の変更は不要だが、検索のたびに集合が変わりうるうえ、同じ `seed` の再現に
offset の決定論的な生成が必要になる。IPC 往復も増えるため採用しない。

### C. SQLite の `ORDER BY RANDOM()` を使う

実装は最も簡単だが、`seed` を指定した再現ができず、要件を満たさない。

## 設計

### 抽選アルゴリズム

core に次の読み取り API を追加する。

```rust
pub fn random_photos_text(
    &self,
    filter: &PhotoTextFilter,
    limit: i64,
    seed: u64,
) -> Result<RandomPhotoSample>;

pub struct RandomPhotoSample {
    pub photos: Vec<Photo>,
    /// 抽選前の一致件数。
    pub total: i64,
}
```

1. `build_text_filter` と同じ条件で一致する `p.id` を全件取得する。
2. 各 ID について `key = splitmix64(seed ^ id as u64)` を計算する。
3. `(key, id)` を昇順に並べ、先頭 `min(limit, total)` 件を選ぶ。
   `key` が衝突した場合は ID 昇順で決着させる。
4. 選んだ ID を `photos_by_ids` で取得する。この関数は入力順を保つため、
   抽選順がそのまま返却順になる。

SplitMix64 は外部クレートを増やさず、64 bit 環境で安定した結果を得るために
core 内の小さなヘルパーとして実装する。ID と seed からキーを作るため、SQL の
行順や `LIMIT/OFFSET` に依存しない。

### seed

- リクエストの `seed: Option<u64>` が `Some` ならその値を使う。`0` も有効。
- `None` の場合は daemon が現在時刻（ナノ秒）、プロセス ID、プロセス内カウンタを
  混ぜて seed を生成する。同じ瞬間の連続呼び出しでもカウンタで異なる値になる。
- 生成した seed は返却しない（仕様の返却項目に含まれないため）。

### limit

- MCP 層で `limit.unwrap_or(50).clamp(1, 200)`。
- daemon 層でも防御的に 1〜200 にクランプする。
- 返却する `limit` はクランプ後の要求値。実際の返却件数は
  `min(limit, total)` になる。

### IPC

`crates/ipc/src/protocol.rs` に追加する。

```rust
Request::RandomSearchPhotos {
    world: Option<String>,
    player: Option<String>,
    tag: Option<String>,
    ai_tags: Vec<String>,
    from_utc: Option<i64>,
    to_utc: Option<i64>,
    favorite: bool,
    limit: u32,
    seed: Option<u64>,
}

Response::RandomPhotos {
    result: RandomPhotoSearch,
}
```

`RandomPhotoSearch` は `crates/ipc/src/library.rs` に置く。

```rust
pub struct RandomPhotoSearch {
    pub photos: Vec<PhotoSummary>,
    pub total: i64,
    pub limit: u32,
}
```

`offset` は持たない。`total` は抽選前の一致件数、`limit` はクランプ後の要求値。

プロトコルに新しいリクエスト／レスポンスを追加するため、`PROTOCOL_VERSION` を
7 から 8 に上げる。未実装の daemon library writes 設計も v8 を予定しているが、
そちらは実装時に v9 へずらす。

### MCP ツール

`crates/mcp/src/server.rs` に `RandomSearchPhotosParams` と
`random_search_photos` を追加する。引数は `SearchPhotosParams` から `offset` を
除き、`seed: Option<u64>` を加えた形。ツール結果は
`{ "photos": [...], "total": n, "limit": n }` を structured content で返す。

`crates/mcp/src/client.rs` に `RandomSearchPhotosQuery` と
`DaemonClient::random_search_photos` を追加する。

### 変更ファイル

- `crates/core/src/model.rs`: `RandomPhotoSample`
- `crates/core/src/db.rs`: `photo_ids_text`、`random_photos_text`、SplitMix64、テスト
- `crates/ipc/src/library.rs`: `RandomPhotoSearch`
- `crates/ipc/src/protocol.rs`: リクエスト／レスポンス、round-trip テスト
- `crates/ipc/src/lib.rs`: `PROTOCOL_VERSION = 8` とテスト更新
- `crates/daemon/src/ipc_server.rs`: ハンドラ、seed 生成、IPC テスト
- `crates/mcp/src/client.rs`: クエリ型と送信
- `crates/mcp/src/server.rs`: ツール、パラメータ、ツール一覧テスト

## テスト

- core: 同じ seed・同じ条件で同じ ID 順になる。
- core: 異なる seed で選択が変わる（十分な件数と固定 seed で確認）。
- core: 重複なし、`limit` 件以下、一致件数が少なければ全件。
- core: 絞り込み条件（お気に入りなど）が抽選対象に効く。
- core: 空のライブラリでは空を返す。
- IPC: 新リクエスト／レスポンスの wire tag と round-trip。
- daemon: 固定 seed で決定的な結果、`total` と `limit` が正しい。
- daemon: `limit` のクランプ。
- MCP: 引数が IPC に転送され、`limit` が 200 にクランプされる。
- MCP: 不正な日時は `INVALID_PARAMS`。
- MCP: ツール一覧に `random_search_photos` が含まれる。
- 全体: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## リスク

- 10 万件規模では一致 ID の全件取得とソートが発生する。ID のみの取得と
  `O(N log N)` のソートは数 ms〜数十 ms 程度で、既存の全件件数取得と同程度の
  読み取り負荷に収まる見込み。
- プロトコル v8 は未実装の daemon library writes 計画と番号が重なる。先に
  こちらをマージし、同計画側を v9 に更新する。
- `seed` はライブラリ内容が変わると同じ結果を保証しない。これは仕様上の前提と
  してツール説明にも明記する。
