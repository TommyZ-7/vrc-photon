# MCP ランダム抽出ツール Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** MCP に `random_search_photos` を追加し、`search_photos` と同じ条件の集合から `limit` 件を seed で再現可能に無作為抽出する。

**Architecture:** core が写真 ID と seed から SplitMix64 キーを作って決定的に抽選し、daemon が IPC 越しに結果を返す。MCP は新しい IPC リクエストを 1 回送るだけで、SQL の行順に依存しない。

**Tech Stack:** Rust 2024, rusqlite, rmcp 3.9, serde

**Spec:** `docs/superpowers/specs/2026-11-11-issue-522-random-search-design.md`

## Global Constraints

- 直接 `main` へ push しない。実装は `feat/522-random-search` の worktree で行う。
- `limit` は既定 50、1〜200 にクランプする。daemon も防御的に 1〜200 にクランプする。
- `seed` は任意の `u64`。同じ seed・同じライブラリ内容なら順序込みで同じ結果を返す。
- 一致件数が `limit` 未満なら、一致した全件を無作為順で返す。
- 抽選結果に同じ写真を重複して含めない。
- 読み取り専用。既存の検索条件（ワールド／プレイヤー／手動タグ・AI タグ／撮影日時範囲／お気に入り）をすべて使う。
- 返却は `photos` / `total` / `limit` で、`offset` は持たない。
- `PROTOCOL_VERSION` を 7 から 8 に上げる。
- 新規 UI 文言・UI 変更はない。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## Review Focus

1. **seed の再現性が SQL の行順に依存しない**: 同じ seed で 2 回呼び、ID 順まで一致することを core と daemon のテストで固定する。
2. **同じ写真の重複**: 抽選結果の ID を集合として比較し、重複がないことを core テストで固定する。
3. **limit の境界**: 0、負数、200 超、一致件数未満を core / daemon / MCP の各層で固定する。
4. **一致件数が limit 未満**: 空きを無理に埋めず、一致した全件を返すことを core テストで固定する。
5. **daemon 未起動・不正な日時**: 既存ツールと同じツールエラー／`INVALID_PARAMS` になることを MCP テストで固定する。

---

## File Structure

- `crates/core/src/model.rs`: 抽選結果 `RandomPhotoSample`。
- `crates/core/src/db.rs`: 一致 ID の取得、SplitMix64、抽選 API、テスト。
- `crates/ipc/src/library.rs`: `RandomPhotoSearch` ペイロード。
- `crates/ipc/src/protocol.rs`: `RandomSearchPhotos` / `RandomPhotos` と round-trip テスト。
- `crates/ipc/src/lib.rs`: プロトコル版 8。
- `crates/daemon/src/ipc_server.rs`: IPC ハンドラ、seed 生成、daemon テスト。
- `crates/mcp/src/client.rs`: MCP 用クエリ型と送信。
- `crates/mcp/src/server.rs`: MCP ツール、パラメータ、ツール一覧テスト。

---

### Task 1: core に決定的な抽選 API を追加する

**Files:**
- Modify: `crates/core/src/model.rs`（`PhotoTextFilter` の近く）
- Modify: `crates/core/src/db.rs`（`count_photos_text` の後）
- Test: `crates/core/src/db.rs` の `mod tests`

**Interfaces:**
- Produces: `pub struct RandomPhotoSample { pub photos: Vec<Photo>, pub total: i64 }`
- Produces: `Db::random_photos_text(&self, filter: &PhotoTextFilter, limit: i64, seed: u64) -> Result<RandomPhotoSample>`
- Produces (private): `Db::photo_ids_text(&self, filter: &PhotoTextFilter) -> Result<Vec<i64>>`
- Produces (private): `fn splitmix64(value: u64) -> u64`

- [x] **Step 1: 失敗するテストを書く**

`crates/core/src/db.rs` の `mod tests` に追加する。

```rust
#[test]
fn random_photos_text_is_deterministic_for_a_seed() {
    let db = Db::open_in_memory().unwrap();
    seed_photos(&db, 20);
    let run = || {
        db.random_photos_text(&PhotoTextFilter::default(), 5, 42)
            .unwrap()
            .photos
            .into_iter()
            .map(|photo| photo.id)
            .collect::<Vec<_>>()
    };
    let first = run();
    assert_eq!(first.len(), 5);
    assert_eq!(first, run());
    let mut unique = first;
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 5);
}

#[test]
fn random_photos_text_varies_with_the_seed() {
    let db = Db::open_in_memory().unwrap();
    seed_photos(&db, 20);
    let sample = |seed| {
        db.random_photos_text(&PhotoTextFilter::default(), 5, seed)
            .unwrap()
            .photos
            .into_iter()
            .map(|photo| photo.id)
            .collect::<Vec<_>>()
    };
    let results: Vec<_> = (0..8).map(sample).collect();
    assert!(
        results.windows(2).any(|pair| pair[0] != pair[1]),
        "different seeds must produce different samples: {results:?}"
    );
}

#[test]
fn random_photos_text_returns_all_matches_when_the_limit_is_larger() {
    let db = Db::open_in_memory().unwrap();
    let ids = seed_photos(&db, 3);
    let sample = db
        .random_photos_text(&PhotoTextFilter::default(), 10, 1)
        .unwrap();
    assert_eq!(sample.total, 3);
    assert_eq!(sample.photos.len(), 3);
    let mut got: Vec<_> = sample.photos.iter().map(|photo| photo.id).collect();
    got.sort_unstable();
    assert_eq!(got, ids);

    let empty = db
        .random_photos_text(&PhotoTextFilter::default(), 10, 0)
        .unwrap();
    assert_eq!(empty.total, 0);
    assert!(empty.photos.is_empty());
}

#[test]
fn random_photos_text_honors_filters() {
    let db = Db::open_in_memory().unwrap();
    let ids = seed_photos(&db, 5);
    db.set_favorite(ids[2], true).unwrap();
    let sample = db
        .random_photos_text(
            &PhotoTextFilter {
                favorite: true,
                ..Default::default()
            },
            10,
            99,
        )
        .unwrap();
    assert_eq!(sample.total, 1);
    assert_eq!(sample.photos.len(), 1);
    assert_eq!(sample.photos[0].id, ids[2]);
}
```

既存の `photo_list_and_search_limits_agree` テストに、次の契約チェックを追加する。

```rust
assert_limit_contract("random_photos_text", |limit| {
    db.random_photos_text(&PhotoTextFilter::default(), limit, 7)
        .unwrap()
        .photos
        .len()
});
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core random_photos_text -- --nocapture`
Expected: FAIL（`random_photos_text` が存在しない）

- [x] **Step 3: `RandomPhotoSample` と抽選処理を実装する**

`crates/core/src/model.rs` の `PhotoTextFilter` の近くに追加する。

```rust
/// A deterministic random sample of photos matching a text filter.
#[derive(Debug, Clone, Default)]
pub struct RandomPhotoSample {
    pub photos: Vec<Photo>,
    /// Number of photos matching the filter before sampling.
    pub total: i64,
}
```

`crates/core/src/db.rs` の `count_photos_text` の後に、次の private 関数と公開 API を追加する。`RandomPhotoSample` を `crate::model` の import に加えること。

```rust
fn splitmix64(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Db {
    fn photo_ids_text(&self, filter: &PhotoTextFilter) -> Result<Vec<i64>> {
        let (where_sql, values) = build_text_filter(filter);
        let sql = format!("SELECT p.id{PHOTO_FROM} WHERE {where_sql}");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Deterministically samples up to `limit` matching photos using `seed`.
    /// `total` is the number of matches before sampling; `limit` follows
    /// [`clamp_limit`].
    pub fn random_photos_text(
        &self,
        filter: &PhotoTextFilter,
        limit: i64,
        seed: u64,
    ) -> Result<RandomPhotoSample> {
        let ids = self.photo_ids_text(filter)?;
        let total = ids.len() as i64;
        let take = clamp_limit(limit) as usize;
        let mut keyed: Vec<(u64, i64)> = ids
            .iter()
            .map(|&id| (splitmix64(seed ^ id as u64), id))
            .collect();
        keyed.sort_unstable();
        keyed.truncate(take.min(keyed.len()));
        let ids: Vec<i64> = keyed.into_iter().map(|(_, id)| id).collect();
        let photos = self.photos_by_ids(&ids)?;
        Ok(RandomPhotoSample { photos, total })
    }
}
```

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core random_photos_text && cargo test -p vrc-photon-core limits_agree`
Expected: PASS

- [x] **Step 5: コミット**

```bash
git add crates/core/src/model.rs crates/core/src/db.rs
git commit -m "feat(core): add deterministic random photo sampling"
```

---

### Task 2: IPC にランダム検索のリクエスト／レスポンスを追加する

**Files:**
- Modify: `crates/ipc/src/library.rs`
- Modify: `crates/ipc/src/protocol.rs`
- Modify: `crates/ipc/src/lib.rs`
- Test: `crates/ipc/src/protocol.rs` の `mod tests`
- Test: `crates/ipc/src/lib.rs` の `mod tests`

**Interfaces:**
- Consumes: なし
- Produces: `pub struct RandomPhotoSearch { pub photos: Vec<PhotoSummary>, pub total: i64, pub limit: u32 }`
- Produces: `Request::RandomSearchPhotos { world, player, tag, ai_tags, from_utc, to_utc, favorite, limit, seed: Option<u64> }`
- Produces: `Response::RandomPhotos { result: RandomPhotoSearch }`
- Produces: `PROTOCOL_VERSION == 8`

- [x] **Step 1: 失敗するテストを書く**

`crates/ipc/src/protocol.rs` の `library_requests_round_trip_with_their_wire_tags` に追加する。

```rust
let random = RequestEnvelope {
    id: 3,
    request: Request::RandomSearchPhotos {
        world: Some("World".into()),
        player: None,
        tag: Some("beach".into()),
        ai_tags: vec!["sea".into()],
        from_utc: Some(100),
        to_utc: Some(200),
        favorite: true,
        limit: 50,
        seed: Some(42),
    },
};
let json = serde_json::to_string(&random).unwrap();
assert!(json.contains("\"type\":\"random_search_photos\""));
assert_eq!(
    serde_json::from_str::<RequestEnvelope>(&json).unwrap(),
    random
);
```

`library_responses_round_trip_with_their_wire_tags` に追加する。

```rust
let random = ServerMessage::Response {
    id: 3,
    response: Response::RandomPhotos {
        result: RandomPhotoSearch {
            photos: Vec::new(),
            total: 3,
            limit: 50,
        },
    },
};
let json = serde_json::to_string(&random).unwrap();
assert!(json.contains("\"type\":\"random_photos\""));
assert_eq!(
    serde_json::from_str::<ServerMessage>(&json).unwrap(),
    random
);
```

`crates/ipc/src/lib.rs` のテストを次のように変更する。

```rust
#[test]
fn protocol_version_is_eight() {
    assert_eq!(PROTOCOL_VERSION, 8);
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ipc random_`
Expected: FAIL（`RandomSearchPhotos` / `RandomPhotoSearch` が存在しない）

- [x] **Step 3: 型とメッセージを追加する**

`crates/ipc/src/library.rs` の `PhotoSearch` の後に追加する。

```rust
/// 条件一致写真から無作為抽出した結果。`total` は抽選前の一致件数。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomPhotoSearch {
    pub photos: Vec<PhotoSummary>,
    pub total: i64,
    pub limit: u32,
}
```

`crates/ipc/src/protocol.rs` の `Request::SearchPhotos` の後に追加する。

```rust
/// 条件一致写真からの決定的な無作為抽出。
RandomSearchPhotos {
    world: Option<String>,
    player: Option<String>,
    tag: Option<String>,
    ai_tags: Vec<String>,
    from_utc: Option<i64>,
    to_utc: Option<i64>,
    favorite: bool,
    limit: u32,
    /// 同じ seed と同じライブラリ内容で同じ結果を返す。
    seed: Option<u64>,
},
```

`Response::Photos` の後に追加する。

```rust
RandomPhotos {
    result: RandomPhotoSearch,
},
```

`crates/ipc/src/protocol.rs` 冒頭の `use crate::library::{...}` に `RandomPhotoSearch` を加える。
`crates/ipc/src/lib.rs` の `PROTOCOL_VERSION` を `8` にする。

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ipc`
Expected: PASS

- [x] **Step 5: コミット**

```bash
git add crates/ipc/src/library.rs crates/ipc/src/protocol.rs crates/ipc/src/lib.rs
git commit -m "feat(ipc): add random photo search messages"
```

---

### Task 3: daemon にハンドラと seed 生成を追加する

**Files:**
- Modify: `crates/daemon/src/ipc_server.rs`
- Test: `crates/daemon/src/ipc_server.rs` の `mod tests`

**Interfaces:**
- Consumes: `Db::random_photos_text`, `RandomPhotoSample`, `RandomPhotoSearch`
- Produces: `fn random_library_search_response(...) -> Response`
- Produces (private): `fn random_seed() -> u64`
- Produces: `const MAX_RANDOM_LIMIT: u32 = 200;`

- [x] **Step 1: 失敗するテストを書く**

`crates/daemon/src/ipc_server.rs` の `mod tests` に追加する。

```rust
#[test]
fn random_search_photos_is_deterministic_for_a_seed() {
    let (_dir, endpoint, control) = test_control();
    let (a, b) = seed_library(&control);
    let _server = IpcServer::start(endpoint.clone(), control.clone()).unwrap();

    let request = |seed| Request::RandomSearchPhotos {
        world: None,
        player: None,
        tag: None,
        ai_tags: Vec::new(),
        from_utc: None,
        to_utc: None,
        favorite: false,
        limit: 1,
        seed: Some(seed),
    };
    let first = match request_raw(&endpoint, request(42)) {
        Response::RandomPhotos { result } => result,
        other => panic!("unexpected response: {other:?}"),
    };
    let second = match request_raw(&endpoint, request(42)) {
        Response::RandomPhotos { result } => result,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(first.total, 2);
    assert_eq!(first.limit, 1);
    assert_eq!(first.photos.len(), 1);
    assert_eq!(first.photos[0].id, second.photos[0].id);
    assert!([a, b].contains(&first.photos[0].id));

    let filtered = match request_raw(
        &endpoint,
        Request::RandomSearchPhotos {
            world: Some("World A".to_string()),
            player: None,
            tag: None,
            ai_tags: Vec::new(),
            from_utc: None,
            to_utc: None,
            favorite: false,
            limit: 10,
            seed: Some(42),
        },
    ) {
        Response::RandomPhotos { result } => result,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.photos[0].id, a);
}

#[test]
fn random_seed_changes_between_calls() {
    assert_ne!(random_seed(), random_seed());
}

#[test]
fn random_search_photos_clamps_the_limit() {
    let (_dir, endpoint, control) = test_control();
    seed_library(&control);
    let _server = IpcServer::start(endpoint.clone(), control.clone()).unwrap();

    match request_raw(
        &endpoint,
        Request::RandomSearchPhotos {
            world: None,
            player: None,
            tag: None,
            ai_tags: Vec::new(),
            from_utc: None,
            to_utc: None,
            favorite: false,
            limit: u32::MAX,
            seed: Some(1),
        },
    ) {
        Response::RandomPhotos { result } => assert_eq!(result.limit, MAX_RANDOM_LIMIT),
        other => panic!("unexpected response: {other:?}"),
    }
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon random_search_photos`
Expected: FAIL（`RandomSearchPhotos` の match がない）

- [x] **Step 3: ハンドラと seed 生成を実装する**

`MAX_LIBRARY_LIMIT` の近くに追加する。

```rust
/// Upper bound for one random sample requested over IPC. The MCP tool clamps
/// to the same value.
const MAX_RANDOM_LIMIT: u32 = 200;
```

`library_search_response` の後に追加する。

```rust
#[allow(clippy::too_many_arguments)]
fn random_library_search_response(
    control: &IpcControl,
    world: Option<String>,
    player: Option<String>,
    tag: Option<String>,
    ai_tags: Vec<String>,
    from_utc: Option<i64>,
    to_utc: Option<i64>,
    favorite: bool,
    limit: u32,
    seed: Option<u64>,
) -> Response {
    let limit = limit.clamp(1, MAX_RANDOM_LIMIT);
    let filter = PhotoTextFilter {
        world,
        player,
        tag,
        favorite,
        from_utc,
        to_utc,
        ai: AiPhotoFilter {
            tags: ai_tags,
            people_count: None,
        },
    };
    let seed = seed.unwrap_or_else(random_seed);
    let result = (|| -> vrc_photon_core::Result<RandomPhotoSearch> {
        let db = control
            .db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sample = db.random_photos_text(&filter, i64::from(limit), seed)?;
        let photos = sample
            .photos
            .iter()
            .map(|photo| photo_summary(&db, photo))
            .collect::<vrc_photon_core::Result<Vec<_>>>()?;
        Ok(RandomPhotoSearch {
            photos,
            total: sample.total,
            limit,
        })
    })();
    match result {
        Ok(result) => Response::RandomPhotos { result },
        Err(error) => library_error(error),
    }
}

fn random_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    now.rotate_left(17)
        ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ u64::from(std::process::id())
}
```

`handle_connection` の `Request::SearchPhotos` の後に match アームを追加する。

```rust
Request::RandomSearchPhotos {
    world,
    player,
    tag,
    ai_tags,
    from_utc,
    to_utc,
    favorite,
    limit,
    seed,
} => {
    let response = random_library_search_response(
        &control, world, player, tag, ai_tags, from_utc, to_utc, favorite, limit, seed,
    );
    write_response(reader.get_mut(), id, response)?;
}
```

import に `RandomPhotoSearch` を加える。

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon random_search_photos random_seed && cargo test -p vrc-photon-daemon search_photos`
Expected: PASS

- [x] **Step 5: コミット**

```bash
git add crates/daemon/src/ipc_server.rs
git commit -m "feat(daemon): serve random photo search"
```

---

### Task 4: MCP ツール `random_search_photos` を追加する

**Files:**
- Modify: `crates/mcp/src/client.rs`
- Modify: `crates/mcp/src/server.rs`
- Test: `crates/mcp/src/server.rs` の `mod tests`

**Interfaces:**
- Consumes: `Request::RandomSearchPhotos`, `Response::RandomPhotos`, `RandomPhotoSearch`
- Produces: `struct RandomSearchPhotosQuery`
- Produces: `DaemonClient::random_search_photos(&self, query: RandomSearchPhotosQuery) -> Result<RandomPhotoSearch, DaemonError>`
- Produces: MCP ツール `random_search_photos`

- [x] **Step 1: 失敗するテストを書く**

`crates/mcp/src/server.rs` の `mod tests` に追加する。`vrc_photon_ipc` の import に `RandomPhotoSearch` を加えること。

```rust
#[tokio::test]
async fn random_search_photos_clamps_limits_and_forwards_filters() {
    let daemon = FakeDaemon::start(vec![Response::RandomPhotos {
        result: RandomPhotoSearch {
            photos: photo_search().photos,
            total: 20,
            limit: 200,
        },
    }]);
    let server = PhotoNMcp::new(daemon.client());

    let result = server
        .random_search_photos(Parameters(RandomSearchPhotosParams {
            world: Some(" World ".to_string()),
            player: None,
            tag: Some("海".to_string()),
            ai_tags: vec!["sea".to_string()],
            captured_from: Some("2026-11-09T00:00:00Z".to_string()),
            captured_to: Some("2026-11-09T23:59:59Z".to_string()),
            favorite: Some(true),
            limit: Some(9999),
            seed: Some(42),
        }))
        .await
        .unwrap();

    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["total"], 20);
    assert_eq!(structured["limit"], 200);
    assert_eq!(structured["photos"][0]["id"], 7);

    match &daemon.requests()[0] {
        vrc_photon_ipc::Request::RandomSearchPhotos {
            world,
            tag,
            ai_tags,
            from_utc,
            to_utc,
            favorite,
            limit,
            seed,
            ..
        } => {
            assert_eq!(world.as_deref(), Some(" World "));
            assert_eq!(tag.as_deref(), Some("海"));
            assert_eq!(ai_tags, &vec!["sea".to_string()]);
            assert_eq!(from_utc, &Some(1_794_182_400));
            assert_eq!(to_utc, &Some(1_794_268_799));
            assert!(favorite);
            assert_eq!(*limit, 200);
            assert_eq!(*seed, Some(42));
        }
        other => panic!("unexpected request: {other:?}"),
    }
}

#[tokio::test]
async fn random_search_photos_forwards_an_unspecified_seed_as_none() {
    let daemon = FakeDaemon::start(vec![Response::RandomPhotos {
        result: RandomPhotoSearch {
            photos: Vec::new(),
            total: 0,
            limit: 50,
        },
    }]);
    let server = PhotoNMcp::new(daemon.client());

    let result = server
        .random_search_photos(Parameters(RandomSearchPhotosParams {
            world: None,
            player: None,
            tag: None,
            ai_tags: Vec::new(),
            captured_from: None,
            captured_to: None,
            favorite: None,
            limit: None,
            seed: None,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));
    assert_eq!(result.structured_content.unwrap()["limit"], 50);

    match &daemon.requests()[0] {
        vrc_photon_ipc::Request::RandomSearchPhotos { limit, seed, .. } => {
            assert_eq!(*limit, 50);
            assert_eq!(*seed, None);
        }
        other => panic!("unexpected request: {other:?}"),
    }
}

#[tokio::test]
async fn random_search_photos_rejects_invalid_timestamps() {
    let daemon = FakeDaemon::start(Vec::new());
    let server = PhotoNMcp::new(daemon.client());
    let error = server
        .random_search_photos(Parameters(RandomSearchPhotosParams {
            world: None,
            player: None,
            tag: None,
            ai_tags: Vec::new(),
            captured_from: Some("yesterday".to_string()),
            captured_to: None,
            favorite: None,
            limit: None,
            seed: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn random_search_photos_reports_daemon_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let server = PhotoNMcp::new(DaemonClient::new(
        vrc_photon_ipc::Endpoint::for_tests(dir.path()),
        Duration::from_secs(1),
    ));
    let result = server
        .random_search_photos(Parameters(RandomSearchPhotosParams {
            world: None,
            player: None,
            tag: None,
            ai_tags: Vec::new(),
            captured_from: None,
            captured_to: None,
            favorite: None,
            limit: None,
            seed: None,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.structured_content.unwrap()["error"],
        "daemon_unavailable"
    );
}
```

既存の `tool_router_lists_all_five_tools` を `tool_router_lists_all_six_tools` に改名し、期待リストへ `"random_search_photos"` を追加する。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-mcp random_search_photos && cargo test -p vrc-photon-mcp tool_router`
Expected: FAIL（`RandomSearchPhotosParams` / `random_search_photos` が存在しない）

- [x] **Step 3: MCP クライアントとツールを実装する**

`crates/mcp/src/client.rs` に追加する。

```rust
/// Parameters for one random photo search.
#[derive(Debug, Clone, Default)]
pub struct RandomSearchPhotosQuery {
    pub world: Option<String>,
    pub player: Option<String>,
    pub tag: Option<String>,
    pub ai_tags: Vec<String>,
    pub from_utc: Option<i64>,
    pub to_utc: Option<i64>,
    pub favorite: bool,
    pub limit: u32,
    pub seed: Option<u64>,
}

pub fn random_search_photos(
    &self,
    query: RandomSearchPhotosQuery,
) -> Result<RandomPhotoSearch, DaemonError> {
    match self.request(Request::RandomSearchPhotos {
        world: query.world,
        player: query.player,
        tag: query.tag,
        ai_tags: query.ai_tags,
        from_utc: query.from_utc,
        to_utc: query.to_utc,
        favorite: query.favorite,
        limit: query.limit,
        seed: query.seed,
    })? {
        Response::RandomPhotos { result } => Ok(result),
        other => Err(unexpected(&other)),
    }
}
```

`crates/mcp/src/server.rs` にパラメータ型を追加する。

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RandomSearchPhotosParams {
    /// World name, world ID, or instance ID substring.
    #[serde(default)]
    pub world: Option<String>,
    /// Player display name or user ID substring.
    #[serde(default)]
    pub player: Option<String>,
    /// Manual tag or applied AI tag name/key substring.
    #[serde(default)]
    pub tag: Option<String>,
    /// Applied AI tag keys, matched with OR.
    #[serde(default)]
    pub ai_tags: Vec<String>,
    /// Inclusive lower capture bound as an RFC 3339 timestamp.
    #[serde(default)]
    pub captured_from: Option<String>,
    /// Inclusive upper capture bound as an RFC 3339 timestamp.
    #[serde(default)]
    pub captured_to: Option<String>,
    /// Restrict the result to favorites when true.
    #[serde(default)]
    pub favorite: Option<bool>,
    /// Sample size. Defaults to 50 and is clamped to 1..=200.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Reproducibility seed. The same seed and library contents return the same
    /// sample; omit it to get a different sample on each call.
    #[serde(default)]
    pub seed: Option<u64>,
}
```

`#[tool_router]` の `search_photos` の後にツールを追加する。

```rust
#[tool(
    description = "Return a random sample of photos matching the same filters as search_photos. The same seed and library contents return the same sample; omit seed to vary each call."
)]
async fn random_search_photos(
    &self,
    Parameters(params): Parameters<RandomSearchPhotosParams>,
) -> Result<CallToolResult, ErrorData> {
    let captured_from = parse_timestamp(params.captured_from.as_deref(), "captured_from")?;
    let captured_to = parse_timestamp(params.captured_to.as_deref(), "captured_to")?;
    let query = RandomSearchPhotosQuery {
        world: params.world,
        player: params.player,
        tag: params.tag,
        ai_tags: params.ai_tags,
        from_utc: captured_from,
        to_utc: captured_to,
        favorite: params.favorite.unwrap_or(false),
        limit: params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
        seed: params.seed,
    };
    let client = self.client.clone();
    match call_daemon(move || client.random_search_photos(query)).await {
        Ok(result) => structured(result),
        Err(error) => Ok(tool_error(error)),
    }
}
```

import を `use crate::client::{DaemonClient, DaemonError, RandomSearchPhotosQuery, SearchPhotosQuery};` に更新する。

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-mcp`
Expected: PASS

- [x] **Step 5: コミット**

```bash
git add crates/mcp/src/client.rs crates/mcp/src/server.rs
git commit -m "feat(mcp): add random_search_photos tool"
```

---

### Task 5: 全体検証と PR

**Files:**
- Verify: ワークスペース全体

**Interfaces:**
- Consumes: Task 1〜4 の成果
- Produces: CI green の PR

- [x] **Step 1: フォーマットを確認する**

Run: `cargo fmt --all -- --check`
Expected: 差分なし

- [x] **Step 2: clippy を確認する**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 警告なし

- [x] **Step 3: 全テストを確認する**

Run: `cargo test --workspace`
Expected: PASS

- [x] **Step 4: 最終コミットと PR 作成**

```bash
git add -A
git commit -m "feat: add MCP random search tool (#522)"
git push -u origin feat/522-random-search
gh pr create --title "feat: MCP にランダム抽出ツール random_search_photos を追加する" \
  --body "Closes #522" --label enhancement
```

- [x] **Step 5: CI を監視して green ならマージする**

`ci-watch` スキルに従い、最新の CI run を完了まで監視する。green なら
`gh pr merge <n> --merge` でマージし、`main` に戻って pull する。

---

## 完了後のフォローアップ

- 実装 PR のマージ後、計画ファイルのチェックボックスを `- [x]` に更新し、
  設計書の Status を「承認済み・実装完了」に変更する docs PR を作る。
- 未実装の `docs/superpowers/plans/2026-11-09-daemon-library-writes.md` が
  プロトコル v8 を前提にしている場合、v9 へ更新する（この計画のスコープ外）。
