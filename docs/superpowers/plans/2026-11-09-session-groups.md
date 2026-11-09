# セッション単位のグループ表示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Photos 画面に表示モード「セッション」を追加し、VRChat のワールド参加回ごとに写真を見出し付きでグループ表示する。

**Architecture:** core の軽量索引に `session_id` とローカル撮影日を追加し、`sessions` / `session_players` / `photo_context` からセッション要約を返す読み取り API を 1 本追加する。UI はセッションモードだけ `v_virtual_list` で見出し行・日付ラベル行・写真行を混在させ、時系列モードは既存の `uniform_list` を維持する。モード選択はアプリ設定に保存する。

**Tech Stack:** Rust 2024, rusqlite, gpui-kit 0.9, SQLite

**Spec:** `docs/superpowers/specs/2026-11-09-session-groups-design.md`

**Mock:** `docs/ui-session-groups-mockup.html`

## Global Constraints

- 新しいテーブル・カラム・収集処理は追加しない。既存の `sessions` / `session_players` / `photo_context.session_id` だけを使う。
- daemon / IPC のプロトコル・メッセージ形は変更しない。UI は同一プロセスの `Db` を直接読む。
- 並びは全体・セッション内とも新しい順（`PhotoSort::Newest`）。
- 対象は可視写真（`p.status IN ('present', 'replaced')`）のみ。
- 初版のセッション表示は Photos 画面のみ。検索結果画面は時系列のまま。
- `photo_context.session_id` のない写真を、撮影時刻の重なりでセッションへ寄せない。
- 見出し・日付ラベルの高さは固定し、要約の到着で行の高さを変えない。
- 新規・変更する UI 文言は `crates/i18n/locales/ja.yml` と `en.yml` の両方に追加する。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- 実装開始前に `docs/ui-session-groups-mockup.html` の承認を得る。承認前に製品コードを書かない。

## Review Focus

1. **混在行のスクロール計算**: 見出し行・日付ラベル行を含むオフセットで、先頭写真の取得と年レールのジャンプが往復する。Task 3 / Task 7 の純粋テストで固定する。
2. **要約の遅延到着**: 索引 generation が変わった後に古い要約を挿入しない。見出しの高さが変わらない。Task 2 / Task 4 で固定する。
3. **未紐付け・日付またぎ**: `session_id` のない写真を動かさず、ローカル日付で区切る。Task 3 で固定する。
4. **折りたたみとキーボード**: 折りたたみ中の写真へ選択が移ったら展開し、隠れたままスクロールしない。Task 5 / Task 6 で固定する。
5. **年レール**: 写真枚数の比率を保ったまま、セッション行のオフセットへ変換する。Task 7 で固定する。
6. **i18n プレースホルダー**: 新しい `tr_args!` キーを `placeholders.rs` の `CALL_SITES` に追加し、両ロケールで解決する。Task 8 で固定する。

---

## 決定待ち（実装前に回答を得る）

- [ ] **Q1. 検索結果への適用**: Photos のみ（推奨） / 検索結果にも適用
- [ ] **Q2. 見出しの折りたたみ**: 初版に含める（推奨、状態は保存しない） / 含めない
- [ ] **Q3. モードの保持**: アプリ設定に保存（推奨） / 起動ごとに時系列
- [ ] **Q4. XMP のみの写真の帰属**: 寄せない（推奨） / 時刻の重なりで寄せる
- [ ] **Q5. 年レール**: セッションモードでも表示（推奨） / 隠す
- [ ] **Q6. セッションが分断された場合**: 見出しは最初の 1 回（推奨） / 分断ごとに繰り返す

回答が推奨案と異なる場合、該当タスク（Q1: Task 6、Q2: Task 3/5/6、Q3: Task 5、Q4: Task 3、Q5: Task 7、Q6: Task 3）を読み替える。

---

## File Structure

- `crates/core/src/model.rs`: `PhotoIndexEntry` の追加フィールド、`SessionSummary`。
- `crates/core/src/db.rs`: `PHOTO_DAY`、索引 SELECT、`session_summaries`、テスト。
- `crates/core/src/settings.rs`: `PhotosViewMode`、`AppSettings.ui_photos_view_mode`、テスト。
- `crates/ui/src/state/session_rows.rs`（新規）: 見出し・日付ラベル・写真行の純粋な行モデル。
- `crates/ui/src/state/photo_browser.rs`: セッション用スクロール・行モデル・要約キャッシュ。
- `crates/ui/src/state/test_support.rs`: 索引フィクスチャの追加フィールド。
- `crates/ui/src/app.rs`: `mod session;`、App フィールドの初期化。
- `crates/ui/src/app/session.rs`（新規）: モード切替、折りたたみ、要約の非同期取得、モード別スクロール。
- `crates/ui/src/app/lifecycle.rs`: 索引再読込時のアンカーとセッション行の再構築。
- `crates/ui/src/app/photos.rs`: キーボード移動・先頭へ戻るのモード対応。
- `crates/ui/src/ui/gallery.rs`: セッションモードのヘッダー、見出し、日付ラベル、仮想リスト。
- `crates/ui/src/ui/year_rail.rs`: スクロール対象の抽象化。
- `crates/ui/src/state/year_rail.rs`: セッション行のジャンプ計算。
- `crates/ui/src/ui/theme.rs`: 見出し・日付ラベルの固定高。
- `crates/i18n/locales/ja.yml`, `en.yml`: 新規文言。
- `crates/i18n/tests/placeholders.rs`: `CALL_SITES` の追加。
- `crates/ui/src/app/demo.rs`, `demo_fixture.rs`: QA 用セッションデータ。
- `docs/ui.md`: 実装後の参考リンク（必要なら）。

---

### Task 1: 軽量索引に session_id とローカル撮影日を追加

**Files:**
- Modify: `crates/core/src/model.rs`（`PhotoIndexEntry`）
- Modify: `crates/core/src/db.rs`（`PHOTO_DAY`、`photo_index`、`photo_index_text_limited`、`row_to_photo_index`）
- Modify: `crates/ui/src/state/test_support.rs`
- Test: `crates/core/src/db.rs` の `mod tests`

**Interfaces:**
- Consumes: 既存の `PHOTO_FROM`、`VISIBLE_STATUS`、`photo_index` / `photo_index_text_limited`。
- Produces:
  - `PhotoIndexEntry { id, captured_at_utc, captured_month, captured_day, session_id }`
  - `PHOTO_DAY` 定数（`YYYYMMDD`、不明は 0）

- [ ] **Step 1: Write the failing tests**

`crates/core/src/db.rs` の `mod tests` に追加する。

```rust
#[test]
fn photo_index_includes_session_id_and_local_capture_day() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let world = db.upsert_world("wrld_test", Some("Test World"), Some("log")).unwrap();
    let session = db.create_session("log.txt", 100, world, None).unwrap();
    let linked = db.upsert_photo(&photo_at(
        root,
        "/photos/linked.png",
        100,
        "2025-04-30 23:41:00.000",
    )).unwrap();
    db.set_photo_context(&PhotoContext {
        photo_id: linked,
        session_id: Some(session),
        world_id: Some(world),
        world_name: Some("Test World".into()),
        instance_id: None,
        link_method: "log".into(),
        confidence: 0.9,
        author: None,
        linked_at: now_string(),
    }).unwrap();
    let unlinked = db.upsert_photo(&photo_at(
        root,
        "/photos/unlinked.png",
        99,
        "2025-04-30 22:00:00.000",
    )).unwrap();

    let entries = db.photo_index(PhotoSort::Newest).unwrap();
    let linked_entry = entries.iter().find(|entry| entry.id == linked).unwrap();
    assert_eq!(linked_entry.session_id, Some(session));
    assert_eq!(linked_entry.captured_day, 20250430);
    let unlinked_entry = entries.iter().find(|entry| entry.id == unlinked).unwrap();
    assert_eq!(unlinked_entry.session_id, None);
    assert_eq!(unlinked_entry.captured_day, 20250430);
}

#[test]
fn photo_index_day_falls_back_to_utc_when_local_time_is_invalid() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let id = db.upsert_photo(&photo_at(root, "/photos/a.png", 0, "not-a-date")).unwrap();
    // photo_at sets captured_at_utc to 0 => 1970-01-01 UTC.
    let entry = db.photo_index(PhotoSort::Newest).unwrap().into_iter().find(|e| e.id == id).unwrap();
    assert_eq!(entry.captured_day, 19700101);
}
```

`photo_at` は既存ヘルパー。`now_string()` は既存のものでよい。無ければ `"2025-01-01T00:00:00Z".to_string()` を使う。

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-core photo_index_includes_session_id_and_local_capture_day photo_index_day_falls_back_to_utc_when_local_time_is_invalid`

Expected: FAIL with "no field `session_id`" / "no field `captured_day`".

- [ ] **Step 3: Implement the model and SQL**

`crates/core/src/model.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhotoIndexEntry {
    pub id: i64,
    pub captured_at_utc: i64,
    /// Local capture month encoded as YYYYMM, e.g. 202504.
    pub captured_month: i32,
    /// Local capture day encoded as YYYYMMDD, or 0 when unknown.
    pub captured_day: i32,
    /// `photo_context.session_id`; `None` means unlinked.
    pub session_id: Option<i64>,
}
```

`crates/core/src/db.rs`:

```rust
const PHOTO_DAY: &str = "COALESCE(\
    CAST(strftime('%Y%m%d', p.captured_at_local) AS INTEGER), \
    CAST(strftime('%Y%m%d', p.captured_at_utc, 'unixepoch') AS INTEGER), \
    0)";
```

- `photo_index` の SELECT を
  `SELECT p.id, p.captured_at_utc, {PHOTO_MONTH}, {PHOTO_DAY}, c.session_id{PHOTO_FROM} ...`
  に変える。
- `photo_index_text_limited` も同じ列順にする。
- `row_to_photo_index` を
  `captured_month: row.get(2)?, captured_day: row.get(3)?, session_id: row.get(4)?`
  に変える。
- `crates/ui/src/state/test_support.rs` の `index_entry` を
  `captured_day: month * 100 + 1, session_id: None` で更新する。

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-core photo_index`

Expected: PASS。既存の `photo_index_orders_newest_first_and_uses_local_month` なども通る。

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/model.rs crates/core/src/db.rs crates/ui/src/state/test_support.rs
git commit -m "feat(core): include session and local day in the photo index"
```

---

### Task 2: セッション要約 API

**Files:**
- Modify: `crates/core/src/model.rs`（`SessionSummary`）
- Modify: `crates/core/src/db.rs`（`session_summaries`、テスト）
- Test: `crates/core/src/db.rs` の `mod tests`

**Interfaces:**
- Consumes: `Db`, `Session`, `Photo`, `photos_by_ids`, `PHOTO_SELECT`, `VISIBLE_STATUS`。
- Produces:
  - `pub struct SessionSummary { session: Session, world_name: Option<String>, world_external_id: Option<String>, photo_count: i64, players: Vec<String>, cover_photo: Option<Photo> }`
  - `Db::session_summaries(&self, ids: &[i64]) -> Result<Vec<SessionSummary>>`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn session_summaries_return_world_count_players_and_cover() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let world = db.upsert_world("wrld_test", Some("Test World"), Some("log")).unwrap();
    let session = db.create_session("log.txt", 100, world, Some("123~region(jp)")).unwrap();
    db.close_session(session, 200).unwrap();
    db.upsert_session_player(session, "usr_b", "Ren", 120).unwrap();
    db.upsert_session_player(session, "usr_a", "Aoi", 110).unwrap();

    let older = db.upsert_photo(&photo_at(root, "/photos/older.png", 130, "2025-04-30 09:00:00")).unwrap();
    let favorite = db.upsert_photo(&photo_at(root, "/photos/fav.png", 140, "2025-04-30 09:01:00")).unwrap();
    let newest = db.upsert_photo(&photo_at(root, "/photos/newest.png", 150, "2025-04-30 09:02:00")).unwrap();
    for id in [older, favorite, newest] {
        db.set_photo_context(&PhotoContext {
            photo_id: id,
            session_id: Some(session),
            world_id: Some(world),
            world_name: Some("Test World".into()),
            instance_id: Some("123~region(jp)".into()),
            link_method: "log".into(),
            confidence: 0.9,
            author: None,
            linked_at: now_string(),
        }).unwrap();
    }
    db.set_favorite(favorite, true).unwrap();

    let summaries = db.session_summaries(&[session]).unwrap();
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert_eq!(summary.session.id, session);
    assert_eq!(summary.world_name.as_deref(), Some("Test World"));
    assert_eq!(summary.world_external_id.as_deref(), Some("wrld_test"));
    assert_eq!(summary.photo_count, 3);
    assert_eq!(summary.players, vec!["Aoi".to_string(), "Ren".to_string()]);
    assert_eq!(summary.cover_photo.as_ref().map(|p| p.id), Some(favorite));
}

#[test]
fn session_summaries_skip_unknown_ids_and_count_only_visible_photos() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    let session = db.create_session("log.txt", 100, world, None).unwrap();
    let missing = db.upsert_photo(&photo_at(root, "/photos/missing.png", 120, "2025-04-30 09:00:00")).unwrap();
    db.set_photo_context(&PhotoContext {
        photo_id: missing,
        session_id: Some(session),
        world_id: Some(world),
        world_name: None,
        instance_id: None,
        link_method: "log".into(),
        confidence: 0.9,
        author: None,
        linked_at: now_string(),
    }).unwrap();
    db.mark_missing(missing).unwrap();

    let summaries = db.session_summaries(&[session, 9999]).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].photo_count, 0);
    assert_eq!(summaries[0].world_name.as_deref(), Some("wrld_test"));
    assert!(summaries[0].cover_photo.is_none());
}

#[test]
fn session_summaries_deduplicate_ids_and_accept_many_ids() {
    let db = Db::open_in_memory().unwrap();
    assert!(db.session_summaries(&[]).unwrap().is_empty());
    // A duplicate ID must not produce a duplicate row.
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    let session = db.create_session("log.txt", 100, world, None).unwrap();
    assert_eq!(db.session_summaries(&[session, session]).unwrap().len(), 1);
    // The 256-ID cap must reject the excess without erroring.
    let many: Vec<i64> = (1..=300).collect();
    assert!(db.session_summaries(&many).unwrap().is_empty());
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-core session_summaries`

Expected: FAIL with "no method named `session_summaries`".

- [ ] **Step 3: Implement `SessionSummary` and `session_summaries`**

`crates/core/src/model.rs`:

```rust
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub session: Session,
    pub world_name: Option<String>,
    pub world_external_id: Option<String>,
    pub photo_count: i64,
    pub players: Vec<String>,
    pub cover_photo: Option<Photo>,
}
```

`crates/core/src/db.rs`:

1. ID を重複除去し、`MAX_SESSION_SUMMARY_IDS = 256` 件に切り詰める。空なら `Ok(Vec::new())`。
2. `sessions` + `worlds` を取得する。
   ```sql
   SELECT s.id, s.log_file, s.started_at, s.ended_at, s.world_id, s.instance_id,
          COALESCE(w.alias, w.name, w.world_id, '') AS world_name,
          w.world_id AS world_external_id
   FROM sessions s
   LEFT JOIN worlds w ON w.id = s.world_id
   WHERE s.id IN (...)
   ORDER BY s.started_at, s.id
   ```
3. 可視写真の枚数を集計する。
   ```sql
   SELECT c.session_id, COUNT(*)
   FROM photo_context c JOIN photos p ON p.id = c.photo_id
   WHERE c.session_id IN (...) AND p.status IN ('present', 'replaced')
   GROUP BY c.session_id
   ```
4. プレイヤーを取得する。
   ```sql
   SELECT session_id, display_name
   FROM session_players
   WHERE session_id IN (...)
   ORDER BY session_id, joined_at, id
   ```
5. カバー写真 ID を取得する。
   ```sql
   SELECT c.session_id, p.id
   FROM photo_context c JOIN photos p ON p.id = c.photo_id
   WHERE c.session_id IN (...) AND p.status IN ('present', 'replaced')
   ORDER BY c.session_id, p.is_favorite DESC, p.captured_at_utc DESC, p.id DESC
   ```
   各セッションの先頭だけを採用し、`photos_by_ids` で `Photo` を引いて `cover_photo` に入れる。
6. セッション ID ごとに `players` / `photo_count` / `cover_photo` を割り当て、`started_at, id` 順で返す。

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-core session_summaries`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/model.rs crates/core/src/db.rs
git commit -m "feat(core): add session summaries for grouped browsing"
```

---

### Task 3: セッション行モデル（純粋関数）

**Files:**
- Create: `crates/ui/src/state/session_rows.rs`
- Modify: `crates/ui/src/state/mod.rs`（`pub(crate) mod session_rows;`）
- Test: `crates/ui/src/state/session_rows.rs` の `mod tests`

**Interfaces:**
- Consumes: `PhotoIndexEntry`（`id`, `captured_day`, `session_id`）。
- Produces:
  ```rust
  pub(crate) enum SessionRowKind {
      Header { session_id: i64, photo_count: usize, collapsed: bool },
      Photos { first_photo: usize, count: usize },
      DateLabel { day: i32, photo_count: usize },
  }
  pub(crate) struct SessionRows { /* rows, heights, origins, photo_rows, photo_count */ }
  impl SessionRows {
      pub(crate) fn build(
          entries: &[PhotoIndexEntry],
          columns: usize,
          photo_row_height: f32,
          header_height: f32,
          date_label_height: f32,
          collapsed: &HashSet<i64>,
      ) -> Self;
      pub(crate) fn rows(&self) -> &[SessionRow];
      pub(crate) fn heights(&self) -> &[f32];
      pub(crate) fn row_count(&self) -> usize;
      pub(crate) fn top_photo_index(&self, scroll_top: f32) -> Option<usize>;
      pub(crate) fn offset_for_photo(&self, photo_index: usize) -> Option<f32>;
      pub(crate) fn offset_for_session(&self, session_id: i64) -> Option<f32>;
      pub(crate) fn row_for_photo(&self, photo_index: usize) -> Option<usize>;
  }
  ```

- [ ] **Step 1: Write the failing tests**

`session_rows.rs` の `mod tests` に、`index_entry` を拡張したローカルヘルパーを置く。

```rust
fn entry(id: i64, day: i32, session: Option<i64>) -> PhotoIndexEntry {
    PhotoIndexEntry {
        id,
        captured_at_utc: id,
        captured_month: day / 100,
        captured_day: day,
        session_id: session,
    }
}

#[test]
fn groups_contiguous_session_photos_behind_one_header() {
    let entries = vec![
        entry(5, 20261109, Some(7)),
        entry(4, 20261109, Some(7)),
        entry(3, 20261108, Some(8)),
        entry(2, 20261108, Some(8)),
    ];
    let rows = SessionRows::build(&entries, 2, 200.0, 92.0, 46.0, &HashSet::new());
    assert_eq!(
        rows.rows().iter().map(|row| row.kind()).collect::<Vec<_>>(),
        vec![
            SessionRowKind::Header { session_id: 7, photo_count: 2, collapsed: false },
            SessionRowKind::Photos { first_photo: 0, count: 2 },
            SessionRowKind::Header { session_id: 8, photo_count: 2, collapsed: false },
            SessionRowKind::Photos { first_photo: 2, count: 2 },
        ]
    );
    assert_eq!(rows.heights(), &[92.0, 200.0, 92.0, 200.0]);
}

#[test]
fn splits_unlinked_runs_by_local_day_and_never_groups_them_as_sessions() {
    let entries = vec![
        entry(6, 20261109, None),
        entry(5, 20261109, None),
        entry(4, 20261108, None),
        entry(3, 20261108, None),
        entry(2, 20261108, Some(3)),
    ];
    let rows = SessionRows::build(&entries, 2, 200.0, 92.0, 46.0, &HashSet::new());
    assert_eq!(
        rows.rows().iter().map(|row| row.kind()).collect::<Vec<_>>(),
        vec![
            SessionRowKind::DateLabel { day: 20261109, photo_count: 2 },
            SessionRowKind::Photos { first_photo: 0, count: 2 },
            SessionRowKind::DateLabel { day: 20261108, photo_count: 2 },
            SessionRowKind::Photos { first_photo: 2, count: 2 },
            SessionRowKind::Header { session_id: 3, photo_count: 1, collapsed: false },
            SessionRowKind::Photos { first_photo: 4, count: 1 },
        ]
    );
}

#[test]
fn collapsed_sessions_keep_the_header_and_drop_photo_rows() {
    let entries = vec![entry(4, 20261109, Some(7)), entry(3, 20261109, Some(7)), entry(2, 20261108, Some(8))];
    let collapsed = HashSet::from([7]);
    let rows = SessionRows::build(&entries, 2, 200.0, 92.0, 46.0, &collapsed);
    assert_eq!(
        rows.rows().iter().map(|row| row.kind()).collect::<Vec<_>>(),
        vec![
            SessionRowKind::Header { session_id: 7, photo_count: 2, collapsed: true },
            SessionRowKind::Header { session_id: 8, photo_count: 1, collapsed: false },
            SessionRowKind::Photos { first_photo: 2, count: 1 },
        ]
    );
}

#[test]
fn offsets_and_top_photo_index_round_trip_across_headers() {
    let entries: Vec<_> = (0..10).map(|i| entry(10 - i, 20261109, Some((i / 2) as i64))).collect();
    let rows = SessionRows::build(&entries, 2, 200.0, 92.0, 46.0, &HashSet::new());
    for photo in 0..10 {
        let offset = rows.offset_for_photo(photo).unwrap();
        assert_eq!(rows.top_photo_index(offset), Some(photo));
    }
    // A header row maps to the photo below it.
    let header_offset = rows.offset_for_session(7).unwrap();
    assert_eq!(rows.top_photo_index(header_offset), Some(0));
}
```

`SessionRow` に `kind(&self) -> SessionRowKind` を用意するか、テスト用に `SessionRowKind` を Copy にして `row.kind` を直接比較する。実装しやすい方でよい。

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-ui session_rows`

Expected: FAIL with "unresolved module" / "no method named `build`".

- [ ] **Step 3: Implement the row model**

- 連続する `session_id` をまとめる。`session_id == None` は `captured_day` で分割する。
- `columns.max(1)` ごとに写真行を切る。写真行の `count` は `min(columns, 残り)`。
- `Header` の `photo_count` は、その連続区間の写真数（`run_len`）。
- `heights`:
  - Header: `header_height`
  - DateLabel: `date_label_height`
  - Photos: `photo_row_height`
- `origins`: 累積和。
- `photo_rows`: `(first_photo, row_index)` を写真行ごとに持つ。
- `top_photo_index(scroll_top)`:
  - `origins` を `partition_point(|origin| *origin <= scroll_top)` で引き、行を特定する。
  - Header / DateLabel 行なら次の Photos 行の `first_photo`。Photos 行なら `first_photo`。
  - 末尾を超えたら `None`。
- `offset_for_photo(photo_index)`: `photo_rows` を二分探索し、その行の `origins[row]`。
- `offset_for_session(session_id)`: Header 行の `origins[row]`。
- 同じ `session_id` が分断された場合は、最初の Header の後は Header を追加せず、
  Photos 行だけを続ける（Q6 の推奨案）。
- `captured_day == 0` の未紐付け区間は `DateLabel { day: 0, .. }` にまとめる。

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-ui session_rows`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/state/session_rows.rs crates/ui/src/state/mod.rs
git commit -m "feat(ui): add the session row model"
```

---

### Task 4: PhotoBrowser のセッション状態

**Files:**
- Modify: `crates/ui/src/state/photo_browser.rs`
- Test: `crates/ui/src/state/photo_browser.rs` の `mod tests`

**Interfaces:**
- Consumes: `SessionRows`, `SessionSummary`, `VirtualListScrollHandle`。
- Produces:
  - `PhotoBrowser.session_scroll: VirtualListScrollHandle`
  - `PhotoBrowser.session_rows: Option<Arc<SessionRows>>`
  - `PhotoBrowser.collapsed_sessions: HashSet<i64>`
  - `PhotoBrowser.session_summaries: HashMap<i64, Arc<SessionSummary>>`
  - `PhotoBrowser.session_summary_pending: HashSet<i64>`
  - `PhotoBrowser.session_summary_failed: HashSet<i64>`
  - `PhotoBrowser::ensure_session_rows(...) -> Arc<SessionRows>`
  - `PhotoBrowser::plan_session_summaries(ids, limit) -> Vec<i64>`
  - `PhotoBrowser::insert_session_summaries(generation, summaries)`
  - `PhotoBrowser::fail_session_summaries(generation, ids)`
  - `PhotoBrowser::toggle_session_collapsed(id) -> bool`
  - `PhotoBrowser::top_photo_index(scroll_top) -> Option<usize>`（セッションモード用）

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn session_rows_rebuild_only_when_the_key_changes() {
    let index = Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)]));
    let mut browser = PhotoBrowser::new(index, TEST_ROW_HEIGHT);
    let first = browser.ensure_session_rows(4, 200.0, 92.0, 46.0);
    let second = browser.ensure_session_rows(4, 200.0, 92.0, 46.0);
    assert!(Arc::ptr_eq(&first, &second));
    let changed = browser.ensure_session_rows(3, 200.0, 92.0, 46.0);
    assert!(!Arc::ptr_eq(&first, &changed));
}

#[test]
fn session_summaries_ignore_stale_generations_and_do_not_replan() {
    let index = Arc::new(PhotoIndex::new(vec![]));
    let mut browser = PhotoBrowser::new(index, TEST_ROW_HEIGHT);
    let generation = browser.generation;
    let planned = browser.plan_session_summaries([7, 7, 8], 2);
    assert_eq!(planned, vec![7, 8]);
    assert!(browser.plan_session_summaries([7, 8], 2).is_empty());
    browser.insert_session_summaries(generation + 1, vec![summary(7)]);
    assert!(browser.session_summary(7).is_none());
    browser.insert_session_summaries(generation, vec![summary(7)]);
    assert!(browser.session_summary(7).is_some());
}

#[test]
fn replace_index_clears_session_state() {
    let index = Arc::new(PhotoIndex::new(vec![index_entry(1, 202601)]));
    let mut browser = PhotoBrowser::new(index, TEST_ROW_HEIGHT);
    browser.toggle_session_collapsed(7);
    browser.ensure_session_rows(4, 200.0, 92.0, 46.0);
    browser.replace_index(Arc::new(PhotoIndex::new(vec![index_entry(2, 202602)])));
    assert!(browser.collapsed_sessions.is_empty());
    assert!(browser.session_rows.is_none());
}
```

`summary(7)` は `SessionSummary` のテストフィクスチャ。`Session { id: 7, ... }` を埋める。

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-ui session_rows_rebuild_only_when_the_key_changes session_summaries_ignore_stale_generations_and_do_not_replan replace_index_clears_session_state`

Expected: FAIL with "no method named `ensure_session_rows`".

- [ ] **Step 3: Implement the session state**

- `PhotoBrowser::new` で `session_scroll: VirtualListScrollHandle::new()` を初期化する。
- `replace_index` で `session_rows = None`, `session_summaries.clear()`, pending/failed clear,
  `collapsed_sessions.clear()`, `session_scroll` のオフセットを 0 に戻す。
- `ensure_session_rows`:
  - キー `(generation, columns, photo_row_height, header_height, date_label_height, collapsed hash)` を比較する。
  - 変わっていれば `SessionRows::build(...)` を `Arc` で構築してキャッシュする。
  - 変わっていなければ既存の `Arc` を返す。
- `plan_session_summaries`: 入力 ID を重複除去し、`summaries` / `pending` / `failed` にあるものを除き、
  `limit` 件まで `pending` に入れて返す。
- `insert_session_summaries`: `generation` が一致するときだけ、`pending` から外して
  `Arc<SessionSummary>` を挿入する。
- `fail_session_summaries`: `generation` が一致するときだけ、`pending` から外して `failed` に入れる。
- `toggle_session_collapsed`: `collapsed_sessions` を反転し、`session_rows = None` にして `true` を返す。
  存在しないセッション ID でも状態だけ持ってよい。

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-ui state::photo_browser`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/state/photo_browser.rs
git commit -m "feat(ui): cache session rows and summaries in the photo browser"
```

---

### Task 5: 設定・App 配線（モード切替、折りたたみ、要約取得）

**Files:**
- Modify: `crates/core/src/settings.rs`
- Modify: `crates/ui/src/app.rs`（`mod session;`、初期化）
- Create: `crates/ui/src/app/session.rs`
- Modify: `crates/ui/src/app/lifecycle.rs`
- Modify: `crates/ui/src/app/photos.rs`
- Modify: `crates/ui/src/ui/theme.rs`（固定高）
- Test: `crates/core/src/settings.rs` の `mod tests`、`crates/ui/src/app/session.rs` の `mod tests`

**Interfaces:**
- Consumes: `PhotoBrowser` のセッション状態、`SessionSummary`、`AppSettings`。
- Produces:
  - `PhotosViewMode { Timeline, Sessions }`
  - `AppSettings.ui_photos_view_mode`
  - `App::set_photos_view_mode(mode, cx)`
  - `App::load_session_summaries(search, generation, ids, cx)`
  - `App::toggle_session_collapsed(session_id, cx)`
  - `App::capture_top_photo(search) -> Option<i64>`
  - `App::scroll_to_photo(search, id, strategy)`
  - `next_view_mode(mode: PhotosViewMode) -> PhotosViewMode`（純粋ヘルパー）

- [ ] **Step 1: Write the failing settings tests**

```rust
#[test]
fn photos_view_mode_defaults_to_timeline_and_round_trips() {
    let db = Db::open_in_memory().unwrap();
    let settings = AppSettings {
        ui_photos_view_mode: PhotosViewMode::Sessions,
        ..AppSettings::default()
    };
    settings.save(&db).unwrap();
    let loaded = AppSettings::load(&db).unwrap();
    assert_eq!(loaded.ui_photos_view_mode, PhotosViewMode::Sessions);
    assert_eq!(AppSettings::default().ui_photos_view_mode, PhotosViewMode::Timeline);
}

#[test]
fn unknown_photos_view_mode_falls_back_to_timeline() {
    let mut value = serde_json::to_value(AppSettings::default()).unwrap();
    value["ui_photos_view_mode"] = serde_json::json!("future_mode");
    let settings: AppSettings = serde_json::from_value(value).unwrap();
    assert_eq!(settings.ui_photos_view_mode, PhotosViewMode::Timeline);
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-core photos_view_mode`

Expected: FAIL with "no field `ui_photos_view_mode`".

- [ ] **Step 3: Implement `PhotosViewMode` and settings**

- `PhotosViewMode` を `CardSize` の近くに追加する。
- `AppSettings` に `#[serde(default, deserialize_with = "deserialize_photos_view_mode")]`
  で `ui_photos_view_mode` を追加する。
- `deserialize_photos_view_mode` は `serde_json::Value` を受けて
  `serde_json::from_value(value).unwrap_or_default()` する（`CardSize` と同じ）。
- `Default` 実装に `Timeline` を追加する。

- [ ] **Step 4: Write the failing App tests**

`app/session.rs` の `mod tests` に、モード切替の保存とアンカー計算の純粋部分を置く。

```rust
#[test]
fn view_mode_toggle_keeps_the_top_photo() {
    // Use the pure helper `next_view_mode` and a fake top-photo id.
    assert_eq!(next_view_mode(PhotosViewMode::Timeline), PhotosViewMode::Sessions);
    assert_eq!(next_view_mode(PhotosViewMode::Sessions), PhotosViewMode::Timeline);
}
```

App 全体を起動するテストは既存の test_support に合わせて追加できる範囲で行う。
難しければ、`next_view_mode` と `mode_label` の純粋テストだけに留め、モード切替の
動作は手動確認項目に回す。

- [ ] **Step 5: Implement App wiring**

- `crates/ui/src/app.rs` に `mod session;` を追加し、`App` の初期化で
  `settings.ui_photos_view_mode` を読む（既存の `AppSettings` から自動的に入る）。
- `app/session.rs`:
  - `set_photos_view_mode`: 設定を保存し、切替前の先頭写真を取得して
    切替後のモードでスクロールを合わせる。失敗時は `tracing::warn!`。
  - `load_session_summaries`: `load_pages` と同じパターン。
    `self.db.path()` があれば `Db::open_read_only` で `session_summaries` を実行し、
    無ければライブ接続で実行する。generation を確認して
    `insert_session_summaries` / `fail_session_summaries` を呼ぶ。
  - `toggle_session_collapsed`: 折りたたみを反転し、見出し行のオフセットへ
    スクロールを合わせる。`cx.notify()`。
  - `capture_top_photo(search)`: 現在のモードに応じて先頭写真 ID を返す。
    timeline は既存の `anchor_photo_id`、session は
    `browser.session_scroll` と `SessionRows::top_photo_index`。
  - `scroll_to_photo(search, id, strategy)`: timeline は `row_for_photo`、
    session は `SessionRows::offset_for_photo` で `set_offset`。
- `app/lifecycle.rs` の `capture_display_layout_anchors` と `reload_library` を
  `capture_top_photo` / `scroll_to_photo` に置き換える。索引再読込後は
  session モードなら `ensure_session_rows` を呼んでからアンカーへ戻す。
- `app/photos.rs` の `move_grid_selection_by` / `scroll_grid_to_first_page` を
  モード対応にする。選択先が折りたたみ中のセッションなら `toggle_session_collapsed`
  で展開してからスクロールする。
- `ui/theme.rs` に `SESSION_HEADER_ROW_HEIGHT: f32 = 78.0`,
  `SESSION_DATE_LABEL_ROW_HEIGHT: f32 = 30.0` を追加する
  （行間は `CARD_ROW_GAP` を足して `SessionRows` に渡す）。

- [ ] **Step 6: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-core photos_view_mode && cargo test -p vrc-photon-ui app::session`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/settings.rs crates/ui/src/app.rs crates/ui/src/app/session.rs crates/ui/src/app/lifecycle.rs crates/ui/src/app/photos.rs crates/ui/src/ui/theme.rs
git commit -m "feat(ui): wire the photos view mode and session summaries"
```

---

### Task 6: ギャラリーのセッション表示

**Files:**
- Modify: `crates/ui/src/ui/gallery.rs`
- Test: `crates/ui/src/ui/gallery.rs` の `mod tests`（純粋ヘルパー）

**Interfaces:**
- Consumes: `SessionRows`, `PhotoBrowser` のセッション状態、`SessionSummary`、`LibraryGridMetrics`、`render_card`、`v_virtual_list`。
- Produces:
  - `render_session_grid(app, shared, frame, window) -> AnyElement`
  - `render_session_header(...)`
  - `render_session_date_label(...)`
  - `session_player_summary(players: &[String]) -> String`（純粋関数）

- [ ] **Step 1: Write the failing pure tests**

```rust
#[test]
fn session_player_summary_keeps_three_names_and_counts_the_rest() {
    assert_eq!(session_player_summary(&[]), "同席 なし（ソロ）");
    assert_eq!(session_player_summary(&["Aoi".into()]), "同席 Aoi");
    assert_eq!(
        session_player_summary(&["Aoi".into(), "Ren".into(), "Mika".into(), "Sora".into()]),
        "同席 Aoi、Ren、Mika 他1名"
    );
}

#[test]
fn session_time_label_handles_ongoing_and_cross_midnight() {
    // Fixed offset +09:00 to keep the test deterministic.
    let offset = FixedOffset::east_opt(9 * 3600).unwrap();
    assert_eq!(
        session_time_label(ts(2026, 11, 9, 21, 14), None, offset),
        "11/9 21:14 –"
    );
    assert_eq!(
        session_time_label(ts(2026, 11, 8, 22, 3), Some(ts(2026, 11, 8, 23, 47)), offset),
        "11/8 22:03 – 23:47"
    );
    assert_eq!(
        session_time_label(ts(2026, 11, 7, 23, 41), Some(ts(2026, 11, 8, 0, 26)), offset),
        "11/7 23:41 – 11/8 0:26"
    );
}
```

`ts` はテスト用に `NaiveDate::from_ymd_opt(...).and_hms_opt(...)` から
`FixedOffset` で epoch 秒に変換するローカルヘルパー。

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-ui session_player_summary_keeps_three_names_and_counts_the_rest session_time_label_handles_ongoing_and_cross_midnight`

Expected: FAIL with "cannot find function".

- [ ] **Step 3: Implement the header helpers and mode toggle**

- `ui/gallery.rs` に `session_player_summary` と `session_time_label` を追加する。
  `session_time_label` は `chrono::FixedOffset` / `TimeZone` を使い、
  `ended_at == None` は `11/9 21:14 –`、同日は `11/8 22:03 – 23:47`、
  日付またぎは `11/7 23:41 – 11/8 0:26`。
- `render_header` の右側にモード切替を追加する。
  - `app.settings.ui_photos_view_mode` を読む。
  - 2 つの `Button`（`session.mode.timeline` / `session.mode.sessions`）を
    `h_flex` のセグメント風コンテナに入れ、クリックで
    `shared.app.upgrade()` の `set_photos_view_mode` を呼ぶ。
  - 選択中のボタンは `primary()`、非選択は `secondary()` または `ghost()`。
  - 既存のフィルター・並び表示は残す。
- セッションモードでは `render_grid` の代わりに `render_session_grid` を呼ぶ。

- [ ] **Step 4: Implement `render_session_grid`**

1. `library_grid_metrics` で columns / card_width / image_height / row_height を求める。
2. `browser.ensure_session_rows(columns, row_height, header_height + CARD_ROW_GAP, date_height + CARD_ROW_GAP)`
   を呼び、`Arc<SessionRows>` を得る。
3. `item_sizes: Rc<Vec<Size<Pixels>>>` を `rows.heights()` から作る。
4. `v_virtual_list` を作る。
   - list ID: `"session-photo-grid"`。
   - コールバックは `range` の各行を描く。
   - 写真行: `photo_start..photo_end` を計算し、`browser.plan_pages` / `failed_pages_in_range` /
     `page_error_panels` を使って既存の `render_card` とエラーパネルを描く。
   - 見出し行: `session_id` を集めて `plan_session_summaries` を呼ぶ。
     要約があれば `render_session_header`、なければプレースホルダー見出し。
   - 日付ラベル行: `render_session_date_label`。
   - サムネイル不足（カバー含む）を集めて `request_thumbnail` に回す。
   - `plan_pages` / `plan_session_summaries` の結果は既存と同じ `cx.defer` で
     App に渡す。
5. `.track_scroll(&browser.session_scroll)` でスクロールを追跡する。
6. 外側の wrapper は既存の `photo-grid` と同じくフォーカス・キーコンテキストを設定する。
7. 年レールは `RailList::Sessions`（Task 7）で描く。
8. 写真が 0 件なら既存の空状態を返す。写真があるがセッションが 0 件なら
   日付ラベル付きの写真行だけになる。

- [ ] **Step 5: Implement `render_session_header`**

- 高さ `SESSION_HEADER_ROW_HEIGHT`、幅 full、`h_flex`。
- カバー: 104×58、`rounded(8)`。サムネイルがあれば `img`、なければ
  `p.scene_color(photo_scene_key(cover))` の背景。
- 本文:
  - ワールド名: `summary.world_name`、無ければ `tr!("session.unknown_world")`。
  - メタ: `session_time_label(...)`, `tr_args!("session.photo_count", count = ...)`,
    `session_player_summary(...)`。
  - 進行中は `tr!("session.ongoing")` の緑バッジ。
- 右側: 折りたたみボタン。`Button::new(...).label("▾"/"▸").small().ghost()` か
  同等の小さなボタン。クリックで `App::toggle_session_collapsed`。
- 見出し全体は写真ではないので、カードの `on_mouse_down` を付けない。
- 要約未取得時は、ワールド名を `tr!("session.loading")`、メタは枚数のみ、
  カバーは色プレースホルダーにする。
- 要約失敗時は `tr!("session.summary_failed")` を出す。

- [ ] **Step 6: Implement `render_session_date_label`**

- 高さ `SESSION_DATE_LABEL_ROW_HEIGHT`、`h_flex`、左右にダッシュ線。
- 日付 `11/7`（`captured_day` から `month/day` を復元）と
  `tr_args!("session.unlinked_date", date = ..., count = ...)`。
- `day == 0` は `tr!("date.unknown_day")`。

- [ ] **Step 7: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-ui session_player_summary session_time_label`

Expected: PASS.

- [ ] **Step 8: Manual smoke test**

Run: `cargo run -p vrc-photon-ui --bin vrc-photon -- --demo --demo-reset`

- Photos 画面で「時系列 / セッション」を切り替える。
- 見出し・写真行・日付ラベルが表示される。
- 写真カードの選択・右クリック・ダブルクリックが動く。
- 折りたたみが動く。
- スクロールしても行の高さが変わらない。

- [ ] **Step 9: Commit**

```bash
git add crates/ui/src/ui/gallery.rs
git commit -m "feat(ui): render session groups in the photo gallery"
```

---

### Task 7: 年レールのセッション対応（Q5 が「表示する」の場合）

**Files:**
- Modify: `crates/ui/src/ui/year_rail.rs`
- Modify: `crates/ui/src/state/year_rail.rs`
- Modify: `crates/ui/src/ui/gallery.rs`（呼び出し）
- Test: `crates/ui/src/state/year_rail.rs` の `mod tests`

**Interfaces:**
- Consumes: `SessionRows`, `VirtualListScrollHandle`, `UniformListScrollHandle`。
- Produces:
  - `RailList`（または同等の enum）
  - `session_rail_scroll_offset(rows, photo_index, max_offset) -> f32`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn session_rail_jump_uses_session_row_offsets() {
    let entries = vec![
        entry_with_session(10, 20261109, Some(1)),
        entry_with_session(9, 20261109, Some(1)),
        entry_with_session(8, 20261108, Some(2)),
        entry_with_session(7, 20261108, Some(2)),
    ];
    let rows = SessionRows::build(&entries, 2, 200.0, 92.0, 46.0, &HashSet::new());
    let offset = session_rail_scroll_offset(&rows, rows.offset_for_photo(2).unwrap(), 10_000.0);
    assert_eq!(offset, -rows.offset_for_photo(2).unwrap());
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run: `cargo test -p vrc-photon-ui session_rail_jump_uses_session_row_offsets`

Expected: FAIL.

- [ ] **Step 3: Implement `RailList`**

`ui/year_rail.rs` の `YearRail` のフィールドを次のように抽象化する。

```rust
pub(crate) enum RailList {
    Timeline {
        scroll: UniformListScrollHandle,
        columns: usize,
        row_height: f32,
    },
    Sessions {
        scroll: VirtualListScrollHandle,
        rows: Arc<SessionRows>,
    },
}
```

- `scroll()` は base handle を返す。
- `first_visible_photo()` は Timeline なら既存の `first_visible_photo`、
  Sessions なら `rows.top_photo_index(scroll_top)`。
- `scroll_offset_for_photo(photo_index, max_offset)` は Timeline なら既存の
  `rail_scroll_offset`、Sessions なら `rows.offset_for_photo(photo_index)` を
  `-max_offset..0` に clamp する。
- `prepaint` / `paint` / `jump_to` / drag ハンドラは `RailList` 経由に置き換える。
- `gallery.rs` の YearRail 呼び出しを、時系列・検索は `Timeline`、
  セッションモードは `Sessions` にする。
- 既存の `year_rail` テストを `RailList` 経由に更新する。

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-ui year_rail`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/ui/year_rail.rs crates/ui/src/state/year_rail.rs crates/ui/src/ui/gallery.rs
git commit -m "feat(ui): map the year rail to session rows"
```

---

### Task 8: i18n と QA 用デモデータ

**Files:**
- Modify: `crates/i18n/locales/ja.yml`, `en.yml`
- Modify: `crates/i18n/tests/placeholders.rs`
- Modify: `crates/ui/src/app/demo.rs`, `crates/ui/src/app/demo_fixture.rs`
- Test: `crates/i18n/tests/catalog.rs`, `source_keys.rs`, `placeholders.rs`、`crates/ui/src/app/demo.rs`

**Interfaces:**
- Consumes: 既存の `demo_fixture_at`, `DEMO_WORLDS`, `DEMO_NAMES`, `db.create_session`。
- Produces: セッションモードで表示できるデモ用セッション。

- [ ] **Step 1: Add i18n keys**

`ja.yml` / `en.yml` に追加する。

```yaml
session.mode.timeline: "時系列"
session.mode.sessions: "セッション"
session.photo_count: "写真 %{count}枚"
session.ongoing: "滞在中"
session.players: "同席 %{names}"
session.players.none: "同席 なし（ソロ）"
session.players.more: "%{names} 他%{count}名"
session.unlinked_date: "%{date} ・ セッションに属さない写真 %{count}枚"
session.unknown_world: "不明なワールド"
session.loading: "読み込み中"
session.summary_failed: "セッション情報を取得できません"
session.collapse: "折りたたむ"
session.expand: "展開する"
date.unknown_day: "日付不明"
```

英語側は意味を合わせた自然な文言にする（例: `"%{count} photos"`,
`"Solo"`, `"With %{names}"`, `"%{names} +%{count}"`）。

`tr_args!` を使うキーを `crates/i18n/tests/placeholders.rs` の `CALL_SITES` に追加する。

- [ ] **Step 2: Run the i18n tests**

Run: `cargo test -p vrc-photon-i18n`

Expected: PASS。`source_keys` が `tr!` / `tr_args!` のキーをカタログで解決できる。

- [ ] **Step 3: Add demo sessions**

`demo_fixture.rs` に `session` フィールドを追加するか、`demo.rs` のシード時に
連続するリンク済み写真をグループ化してセッションを作る。

推奨: `demo.rs` のシード時に、次の条件でセッションを作る。

- 連続する `photo.world == Some(w)` の写真をまとめる。
- 前の写真との撮影間隔が 2 時間以内なら同じセッション。
- `started_at = 最初の写真 - 5分`、`ended_at = 最後の写真 + 5分`。
- `world` は既存の `world_ids[w]`。
- `session_players` はグループ内の `photo.players` の和集合（`user_id` で重複除去）。
- 各写真の `PhotoContext.session_id` に作ったセッション ID を入れる。

これにより `--demo` でセッションモードを確認できる。デモ DB の再生成は
`--demo-reset` で行う。

- [ ] **Step 4: Add a demo test**

```rust
#[test]
fn demo_seed_creates_sessions_with_players() {
    let (_dir, db, _assets) = seed_demo_db_for_test(); // 既存のテストヘルパーに合わせる
    let entries = db.photo_index(PhotoSort::Newest).unwrap();
    let ids: Vec<i64> = entries.iter().filter_map(|entry| entry.session_id).collect();
    assert!(!ids.is_empty());
    let summaries = db.session_summaries(&ids).unwrap();
    assert!(summaries.iter().any(|summary| !summary.players.is_empty()));
    assert!(summaries.iter().all(|summary| summary.photo_count > 0));
}
```

既存のデモテストのセットアップに合わせてヘルパー名を調整する。

- [ ] **Step 5: Run the tests**

Run: `cargo test -p vrc-photon-ui demo && cargo test -p vrc-photon-i18n`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/i18n/locales/ja.yml crates/i18n/locales/en.yml crates/i18n/tests/placeholders.rs crates/ui/src/app/demo.rs crates/ui/src/app/demo_fixture.rs
git commit -m "feat(demo): seed sessions and add session i18n"
```

---

### Task 9: 検証・手動 QA

**Files:**
- No production changes. QA notes may be added to `docs/ui.md` if needed.

- [ ] **Step 1: Run the local verification**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] **Step 2: Run the app with demo data**

```bash
cargo run -p vrc-photon-ui --bin vrc-photon -- --demo --demo-reset
```

- [ ] **Step 3: Check the UI at 1280×820 and 1440×900**

- 時系列 → セッション → 時系列の切替で先頭写真が飛ばない。
- セッション見出しのワールド名・時刻・枚数・同席者が正しい。
- 滞在中、日付またぎ、未紐付け、ソロ、1 枚だけ、多人数の見出し。
- 折りたたみ／展開、折りたたみ中のキーボード移動。
- 選択・Shift 範囲選択・Ctrl/Cmd 複数選択・右クリック・ダブルクリック。
- 年レールのクリック・ドラッグ・月ピル。
- カードサイズ変更・再読込・再起動後のモード復元。
- 検索結果画面が従来どおり。

- [ ] **Step 4: Capture screenshots (if the environment supports it)**

Use the `gnome-wayland-visual-qa` skill for this machine if needed. Attach
1280×820 and 1440×900 screenshots to the PR or the review message.

- [ ] **Step 5: Commit QA fixes**

```bash
git add -A
git commit -m "fix(ui): polish session group layout"
```

（修正がなければこのコミットは不要。）

---

### Task 10: PR・CI・マージ

**Files:**
- No code changes.

- [ ] **Step 1: Push the branch and open a PR**

```bash
git push -u origin feat/452-session-groups
gh pr create --title "feat: 写真をセッション単位でグループ表示する" \
  --body "Closes #452" --label enhancement
```

- [ ] **Step 2: Watch CI**

Follow the `ci-watch` skill. Do not merge while CI is red.

- [ ] **Step 3: Merge and clean up**

```bash
gh pr merge <n> --merge
git checkout main && git pull
git branch -d feat/452-session-groups
git push origin --delete feat/452-session-groups
```

- [ ] **Step 4: Update the plan checkboxes**

`docs/superpowers/plans/2026-11-09-session-groups.md` の完了項目を `- [x]` に更新する
（AGENTS.md §4）。

---

## 検証コマンド一覧

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p vrc-photon-ui --bin vrc-photon -- --demo --demo-reset
```

## 受け入れ条件（再掲）

- Photos 画面で「時系列 / セッション」を切り替えられる。
- セッションモードで見出しと写真行が新しい順に混在する。
- 見出しにワールド名、滞在時刻（日付またぎ・滞在中を含む）、枚数、
  同席プレイヤー（3 名＋他 n 名）が出る。
- 未紐付け写真は時系列の位置のまま日付ラベル付きで表示される。
- 折りたたみ・展開が動く。
- 写真の選択・右クリック・詳細表示・年レールが時系列モードと同じように動く。
- モードの選択が再起動後も復元される。
- 検索結果画面の表示・操作に変化がない。
- `cargo fmt` / `clippy` / `test` が通る。
