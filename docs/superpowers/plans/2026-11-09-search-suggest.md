# 検索入力サジェスト Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 検索画面のワールド／プレイヤー／タグ入力欄に、部分一致する候補を写真枚数付きで最大10件表示し、キーボードで選択できるようにする。

**Architecture:** core に3つの候補クエリを追加し、UI は既存の検索と同じく読み取り専用 `Db` をバックグラウンドで引く。候補は `deferred(anchored(...))` のフローティングオーバーレイに描画し、キー操作は `intercept_keystrokes` で入力欄のキーバインドより先に処理する。タグ欄のテキスト検索は手動タグと適用済み AI タグの両方に一致させる。

**Tech Stack:** Rust 2024, rusqlite, gpui-kit 0.9, SQLite

**Spec:** `docs/superpowers/specs/2026-11-09-search-suggest-design.md`

## Global Constraints

- 候補の対象は検索画面のワールド／プレイヤー／タグの3欄のみ。Photos 画面には展開しない。
- 候補は可視写真（`p.status IN ('present', 'replaced')`）だけを数える。
- 表示は最大10件。並びは写真枚数降順、同数は名前の昇順（`COLLATE NOCASE`）。
- 入力が空または空白のみなら候補を出さない。
- タグ候補は手動タグと適用済み AI タグ（`plugin_id = 'vrc-photon-autotag'`、`applied = 1`）の両方。
- タグ欄のテキスト検索も手動タグ名と AI タグの `name` / `name_en` / `tag_key` に一致させる。
- 同名の手動タグと AI タグは候補1行にまとめ、写真の重複を数えない。
- 候補選択後も入力欄にフォーカスを残し、検索は自動実行しない。
- daemon / IPC のメッセージ形は変更しない。
- 新規・変更する UI 文言は `crates/i18n/locales/ja.yml` と `en.yml` の両方に追加する。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## Review Focus

1. **日本語 IME の変換中**: Enter や矢印が候補選択に奪われず、変換確定・カーソル移動が優先される。Task 5 の `ime_keystrokes_are_not_intercepted` で固定する。
2. **LIKE メタ文字**: `%` や `_` を含む名前がワイルドカード扱いされず、文字どおり一致する。Task 1 の `suggestions_escape_like_metacharacters` と Task 3 の `ai_tag_text_search_escapes_like_metacharacters` で固定する。
3. **高速入力 / 画面離脱後の古い結果**: 古い非同期結果が新しい入力を上書きしない。Task 4 の `stale_suggestion_results_are_discarded` で固定する。
4. **画面下端の候補**: 10件表示でも window 外に出ず、必要なら入力欄の上に開く。Task 5 の `popup_flips_above_when_below_would_overflow` で固定する。
5. **同名タグと未適用 AI タグ**: 同名の手動／AI タグが重複行にならず、`applied = 0` は候補にも検索にも出ない。Task 2 の `tag_suggestions_merge_same_name_and_skip_unapplied` で固定する。

---

## File Structure

- `crates/core/src/model.rs`: `SearchSuggestion` 型。
- `crates/core/src/db.rs`: 候補クエリ3種、タグ検索条件の拡張、テスト。
- `crates/ui/src/app/search_suggest.rs`（新規）: 候補状態、デバウンス、非同期取得、選択移動、キー処理。
- `crates/ui/src/app.rs`: App フィールド、input 購読、キー interceptor 登録。
- `crates/ui/src/app/search.rs`: 検索実行／クリア時の候補クローズ、AI タグ更新後の再検索判定。
- `crates/ui/src/app/lifecycle.rs`: 画面遷移時の候補クローズ。
- `crates/ui/src/ui/search_suggest.rs`（新規）: ポップアップ描画、位置計算、行クリック。
- `crates/ui/src/ui/search.rs`: 3入力欄の bounds 取得とポップアップ配置。
- `crates/ui/src/ui.rs`: 新規 UI モジュール宣言。
- `crates/mcp/src/server.rs`: `tag` パラメータの説明更新。
- `crates/i18n/locales/ja.yml`, `en.yml`: AI タグ説明文の修正。

---

### Task 1: ワールド／プレイヤー候補クエリ

**Files:**
- Modify: `crates/core/src/model.rs`（`PhotoTextFilter` の近く）
- Modify: `crates/core/src/db.rs:1183-1202`（`photo_index_text_limited` の後）
- Test: `crates/core/src/db.rs` の `mod tests`

**Interfaces:**
- Consumes: 既存の `Db`, `VISIBLE_STATUS`, `escape_like`, `clamp_limit`。
- Produces:
  - `pub struct SearchSuggestion { pub name: String, pub photo_count: i64 }`
  - `Db::world_name_suggestions(&self, query: &str, limit: usize) -> Result<Vec<SearchSuggestion>>`
  - `Db::player_name_suggestions(&self, query: &str, limit: usize) -> Result<Vec<SearchSuggestion>>`

- [ ] **Step 1: Write the failing tests**

Add these tests to `mod tests` in `crates/core/src/db.rs`. Use the existing
`photo_at`, `set_photo_context`, `set_photo_players`, and `PhotoPlayer`
helpers.

```rust
#[test]
fn world_suggestions_match_partial_names_with_visible_counts() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let a = db.upsert_photo(&photo_at(root, "/photos/a.png", 1, "2025-04-30 09:00:00")).unwrap();
    let b = db.upsert_photo(&photo_at(root, "/photos/b.png", 2, "2025-04-30 09:00:00")).unwrap();
    let c = db.upsert_photo(&photo_at(root, "/photos/c.png", 3, "2025-04-30 09:00:00")).unwrap();
    let missing = db.upsert_photo(&photo_at(root, "/photos/m.png", 4, "2025-04-30 09:00:00")).unwrap();
    for (photo_id, name) in [
        (a, "Sunset Pier"),
        (b, "Sunset Pier"),
        (c, "Sunset Bar"),
        (missing, "Sunset Missing"),
    ] {
        set_context(&db, photo_id, Some(name));
    }
    db.mark_missing(missing).unwrap();

    let rows = db.world_name_suggestions("sun", 10).unwrap();
    assert_eq!(
        rows,
        vec![
            SearchSuggestion { name: "Sunset Pier".into(), photo_count: 2 },
            SearchSuggestion { name: "Sunset Bar".into(), photo_count: 1 },
        ]
    );
    assert!(db.world_name_suggestions("  ", 10).unwrap().is_empty());
}

#[test]
fn player_suggestions_group_names_and_count_distinct_photos() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let a = db.upsert_photo(&photo_at(root, "/photos/a.png", 1, "2025-04-30 09:00:00")).unwrap();
    let b = db.upsert_photo(&photo_at(root, "/photos/b.png", 2, "2025-04-30 09:00:00")).unwrap();
    let c = db.upsert_photo(&photo_at(root, "/photos/c.png", 3, "2025-04-30 09:00:00")).unwrap();
    db.set_photo_players(a, &[PhotoPlayer { user_id: "usr_1".into(), display_name: "Mio".into() }]).unwrap();
    db.set_photo_players(b, &[PhotoPlayer { user_id: "usr_2".into(), display_name: "Mio".into() }]).unwrap();
    db.set_photo_players(c, &[PhotoPlayer { user_id: "usr_3".into(), display_name: "Mika".into() }]).unwrap();

    let rows = db.player_name_suggestions("mi", 10).unwrap();
    assert_eq!(
        rows,
        vec![
            SearchSuggestion { name: "Mio".into(), photo_count: 2 },
            SearchSuggestion { name: "Mika".into(), photo_count: 1 },
        ]
    );
}

#[test]
fn suggestions_escape_like_metacharacters_and_honor_the_limit() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    for (index, name) in ["100%_World", "100XWorld", "Percent"].iter().enumerate() {
        let photo = db.upsert_photo(&photo_at(
            root,
            &format!("/photos/{index}.png"),
            index as i64,
            "2025-04-30 09:00:00",
        )).unwrap();
        set_context(&db, photo, Some(name));
    }

    let rows = db.world_name_suggestions("100%", 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "100%_World");
    assert_eq!(db.world_name_suggestions("o", 1).unwrap().len(), 1);
}
```

`set_context` is a local helper used by these tests:

```rust
fn set_context(db: &Db, photo_id: i64, world_name: Option<&str>) {
    db.set_photo_context(&PhotoContext {
        photo_id,
        session_id: None,
        world_id: None,
        world_name: world_name.map(str::to_string),
        instance_id: None,
        link_method: "manual".to_string(),
        confidence: 1.0,
        author: None,
        linked_at: now_string(),
    })
    .unwrap();
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-core world_suggestions_match_partial_names_with_visible_counts player_suggestions_group_names_and_count_distinct_photos suggestions_escape_like_metacharacters_and_honor_the_limit`
Expected: FAIL with "no method named `world_name_suggestions`".

- [ ] **Step 3: Implement the model and queries**

Add to `crates/core/src/model.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchSuggestion {
    pub name: String,
    pub photo_count: i64,
}
```

Add to `impl Db` after `photo_index_text_limited`:

```rust
pub fn world_name_suggestions(&self, query: &str, limit: usize) -> Result<Vec<SearchSuggestion>>;
pub fn player_name_suggestions(&self, query: &str, limit: usize) -> Result<Vec<SearchSuggestion>>;
```

Implementation rules:

- Trim `query`; return `Ok(Vec::new())` when empty.
- Build `like = format!("%{}%", escape_like(query))`.
- Use the SQL from the spec (`world_name_suggestions` and
  `player_name_suggestions` sections), bind `limit as i64`, and map rows to
  `SearchSuggestion`.
- Exclude `NULL` and empty names.
- Use `ORDER BY photo_count DESC, name COLLATE NOCASE ASC`.

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-core world_suggestions player_suggestions suggestions_escape_like`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/model.rs crates/core/src/db.rs
git commit -m "feat(core): add world and player search suggestions"
```

---

### Task 2: タグ候補クエリ（手動 + AI）

**Files:**
- Modify: `crates/core/src/db.rs`（Task 1 のメソッドの後）
- Test: `crates/core/src/db.rs` の `mod tests`

**Interfaces:**
- Consumes: `SearchSuggestion`, `AI_PLUGIN_ID`, `install_test_catalog`, `seed_photo`, `set_photo_tags`, `seed_ai_tag`。
- Produces: `Db::tag_name_suggestions(&self, query: &str, limit: usize, prefer_english: bool) -> Result<Vec<SearchSuggestion>>`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn tag_suggestions_merge_manual_and_applied_ai_tags() {
    let db = Db::open_in_memory().unwrap();
    install_test_catalog(&db);
    let a = seed_photo(&db);
    let b = seed_photo_at(&db, "/photos/b.png");
    let c = seed_photo_at(&db, "/photos/c.png");
    db.set_photo_tags(a, &["夜景".into()]).unwrap();
    db.set_photo_tags(b, &["夜景".into()]).unwrap();
    seed_ai_tag(&db, c, AI_PLUGIN, "sea", 0.97, true);
    seed_ai_tag(&db, c, AI_PLUGIN, "park", 0.97, false);

    assert_eq!(
        db.tag_name_suggestions("夜", 10, false).unwrap(),
        vec![SearchSuggestion { name: "夜景".into(), photo_count: 2 }]
    );
    assert_eq!(
        db.tag_name_suggestions("海", 10, false).unwrap(),
        vec![SearchSuggestion { name: "海".into(), photo_count: 1 }]
    );
    assert_eq!(
        db.tag_name_suggestions("Sea", 10, true).unwrap(),
        vec![SearchSuggestion { name: "Sea".into(), photo_count: 1 }]
    );
    assert!(db.tag_name_suggestions("公", 10, false).unwrap().is_empty());
}

#[test]
fn tag_suggestions_merge_same_name_and_skip_unapplied() {
    let db = Db::open_in_memory().unwrap();
    install_test_catalog(&db);
    let a = seed_photo(&db);
    let b = seed_photo_at(&db, "/photos/b.png");
    db.set_photo_tags(a, &["海".into()]).unwrap();
    seed_ai_tag(&db, a, AI_PLUGIN, "sea", 0.97, true);
    seed_ai_tag(&db, b, AI_PLUGIN, "sea", 0.97, true);

    let rows = db.tag_name_suggestions("海", 10, false).unwrap();
    assert_eq!(rows, vec![SearchSuggestion { name: "海".into(), photo_count: 2 }]);
}
```

`seed_photo_at` adds a photo at a unique path:

```rust
fn seed_photo_at(db: &Db, path: &str) -> i64 {
    let root = db.add_root("/photos", "reference").unwrap();
    db.upsert_photo(&sample_photo(root, path, 1_700_000_000)).unwrap()
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-core tag_suggestions`
Expected: FAIL with "no method named `tag_name_suggestions`".

- [ ] **Step 3: Implement `tag_name_suggestions`**

Use the CTE from the spec. Bind `prefer_english` as an integer boolean in
the AI-name `CASE`, bind the LIKE pattern, and bind `limit as i64`.
`GROUP BY name` must be exact so same-name rows merge; `ORDER BY
photo_count DESC, name COLLATE NOCASE ASC`.

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-core tag_suggestions`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/db.rs
git commit -m "feat(core): add tag suggestions across manual and AI tags"
```

---

### Task 3: タグ検索を AI タグ名にも拡張

**Files:**
- Modify: `crates/core/src/db.rs:5618-5683`（`build_text_filter`）
- Modify: `crates/mcp/src/server.rs:28-40`（`SearchPhotosParams.tag` の doc）
- Modify: `crates/mcp/src/server.rs:83`（ツール説明）
- Test: `crates/core/src/db.rs` の `mod tests`
- Test: `crates/mcp/src/server.rs` の既存 forwarding テスト

**Interfaces:**
- Consumes: 既存の `build_text_filter`, `escape_like`, `AI_PLUGIN_ID`。
- Produces: `PhotoTextFilter.tag` が手動タグと適用済み AI タグ名の両方に一致する。

- [ ] **Step 1: Write the failing core tests**

```rust
#[test]
fn tag_text_filter_matches_applied_ai_tag_names_and_keys() {
    let db = Db::open_in_memory().unwrap();
    install_test_catalog(&db);
    let photo = seed_photo(&db);
    seed_ai_tag(&db, photo, AI_PLUGIN, "sea", 0.97, true);

    for query in ["海", "Sea", "sea"] {
        let filter = PhotoTextFilter { tag: Some(query.into()), ..Default::default() };
        assert_eq!(db.photo_index_text(&filter).unwrap().len(), 1, "{query}");
        assert_eq!(db.search_photos_text(&filter, 10, 0).unwrap().len(), 1, "{query}");
    }

    let unapplied = seed_photo_at(&db, "/photos/unapplied.png");
    seed_ai_tag(&db, unapplied, AI_PLUGIN, "park", 0.97, false);
    let filter = PhotoTextFilter { tag: Some("公園".into()), ..Default::default() };
    assert!(db.photo_index_text(&filter).unwrap().is_empty());
}

#[test]
fn ai_tag_text_search_escapes_like_metacharacters() {
    let db = Db::open_in_memory().unwrap();
    let catalog = Catalog {
        version: "2026-11-03.1".into(),
        outdoor: vec![tag("percent", "100%_tag", Some("100%_tag"), 0.5)],
        indoor: Vec::new(),
        atmosphere: Vec::new(),
    };
    db.sync_ai_catalog(AI_PLUGIN, &catalog).unwrap();
    let photo = seed_photo(&db);
    seed_ai_tag(&db, photo, AI_PLUGIN, "percent", 0.97, true);

    let filter = PhotoTextFilter { tag: Some("100%".into()), ..Default::default() };
    assert_eq!(db.photo_index_text(&filter).unwrap().len(), 1);
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-core tag_text_filter_matches_applied_ai_tag_names_and_keys ai_tag_text_search_escapes_like_metacharacters`
Expected: FAIL; the AI-tag queries return 0 photos.

- [ ] **Step 3: Extend the tag clause in `build_text_filter`**

Replace the single manual-tag `EXISTS` with the OR form from the spec:

```sql
(
  EXISTS (... manual tag ...)
  OR EXISTS (
    SELECT 1 FROM photo_ai_tags at
    JOIN ai_tag_definitions d
      ON d.plugin_id = at.plugin_id AND d.tag_key = at.tag_key
    WHERE at.photo_id = p.id
      AND at.applied = 1
      AND at.plugin_id = ?
      AND (d.name LIKE ? ESCAPE '\'
           OR COALESCE(d.name_en, d.name) LIKE ? ESCAPE '\'
           OR d.tag_key LIKE ? ESCAPE '\')
  )
)
```

Bind `AI_PLUGIN_ID` once and the escaped LIKE pattern three times. Keep the
existing manual-tag placeholder first.

- [ ] **Step 4: Update the MCP-facing text and test**

Change the `tag` doc to "Manual tag or applied AI tag name/key substring."
and the tool description to "manual or AI tag". In
`search_photos_clamps_limits_and_forwards_filters`, change the forwarded tag
to a Japanese AI tag name (e.g. `"海"`) and keep asserting it is forwarded
unchanged.

- [ ] **Step 5: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-core tag_text_filter ai_tag_text_search && cargo test -p vrc-photon-mcp search_photos_clamps_limits_and_forwards_filters`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/db.rs crates/mcp/src/server.rs
git commit -m "feat(core): match applied AI tags in tag text search"
```

---

### Task 4: 候補状態と非同期取得

**Files:**
- Create: `crates/ui/src/app/search_suggest.rs`
- Modify: `crates/ui/src/app.rs:1-19`（module 宣言）、`App` フィールド、`App::new`
- Test: `crates/ui/src/app/search_suggest.rs` の `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `SearchSuggestion`, `SuggestionKind`, `Db`, `InputEvent`。
- Produces:
  - `pub(crate) const SUGGESTION_LIMIT: usize = 10;`
  - `pub(crate) const SUGGESTION_DEBOUNCE: Duration = Duration::from_millis(120);`
  - `pub(crate) enum SuggestionKind { World, Player, Tag }`
  - `pub(crate) struct SuggestionState { kind, query, items, selected, dismissed }`
  - `App::on_suggestion_event`, `App::close_suggestion`, `App::select_suggestion`
  - `App` fields: `suggestion`, `suggestion_request`, `suggestion_bounds`,
    `_search_suggestion_subscriptions`

- [ ] **Step 1: Write the failing helper tests**

```rust
#[test]
fn selection_clamps_at_both_ends() {
    assert_eq!(move_suggestion_selection(0, 5, -1), 0);
    assert_eq!(move_suggestion_selection(0, 5, 1), 1);
    assert_eq!(move_suggestion_selection(4, 5, 1), 4);
    assert_eq!(move_suggestion_selection(0, 0, 1), 0);
}

#[test]
fn blank_queries_are_not_searchable() {
    assert_eq!(normalized_suggestion_query("  "), None);
    assert_eq!(normalized_suggestion_query("  sunset  "), Some("sunset".into()));
}

#[test]
fn stale_results_are_rejected() {
    assert!(suggestion_result_is_current(7, 7));
    assert!(!suggestion_result_is_current(6, 7));
}

#[test]
fn dismissed_state_suppresses_focus_until_the_next_change() {
    assert!(!focus_should_query(true));
    assert!(focus_should_query(false));
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-ui search_suggest`
Expected: FAIL; the module and helpers do not exist.

- [ ] **Step 3: Implement the state and helpers**

Add `pub(crate) mod search_suggest;` to `crates/ui/src/app.rs`. In the new
module:

- Define `SuggestionKind`, `SuggestionState`, `SUGGESTION_LIMIT`, and
  `SUGGESTION_DEBOUNCE`.
- Implement `normalized_suggestion_query`, `move_suggestion_selection`,
  `suggestion_result_is_current`, and `focus_should_query`.
- Implement `query_suggestions(db, kind, query, prefer_english)` that dispatches
  to the three core methods with `SUGGESTION_LIMIT`.
- Implement `App::on_suggestion_event(kind, event, cx)`:
  - `Change`: normalize the input value; close on `None`; otherwise set a new
    state, bump `suggestion_request`, and spawn the 120ms debounce.
  - `Focus`: if `focus_should_query(dismissed)` and the value is non-empty,
    start the query without the debounce.
  - `Blur`: close the suggestion.
- Implement `start_suggestion_query`, `apply_suggestion_result` (drop stale
  generations and non-Search screens), `dismiss_suggestion`, `close_suggestion`,
  and `select_suggestion`.
- `select_suggestion` sets the active input's value with `InputState::set_value`,
  keeps focus, and clears the suggestion state without emitting `Change`.
- `query_suggestions` uses `Db::open_read_only` on the background executor and
  falls back to the live `self.db` on failure, mirroring `start_search`.

Add the App fields and initialize them in `App::new`:

```rust
pub(crate) suggestion: Option<SuggestionState>,
pub(crate) suggestion_request: u64,
pub(crate) suggestion_bounds: Rc<RefCell<HashMap<SuggestionKind, Bounds<Pixels>>>>,
pub(crate) _search_suggestion_subscriptions: Vec<Subscription>,
```

Create the input subscriptions after `world_input` / `player_input` /
`tag_input` exist:

```rust
let world_suggestion_subscription = cx.subscribe(&world_input, |this, _, event: &InputEvent, cx| {
    this.on_suggestion_event(SuggestionKind::World, event, cx);
});
```

Repeat for player and tag; store all three in the Vec.

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-ui search_suggest`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/app.rs crates/ui/src/app/search_suggest.rs
git commit -m "feat(ui): add search suggestion state and async queries"
```

---

### Task 5: 候補ポップアップとキーボード操作

**Files:**
- Create: `crates/ui/src/ui/search_suggest.rs`
- Modify: `crates/ui/src/ui.rs`
- Modify: `crates/ui/src/ui/search.rs:100-210`
- Modify: `crates/ui/src/app/search_suggest.rs`（`handle_suggestion_key` の実装）
- Test: `crates/ui/src/ui/search_suggest.rs` の `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `App.suggestion`, `App.suggestion_bounds`, `format_count`, `tr_args!`。
- Produces:
  - `suggestion_popup_origin(input: Bounds<Pixels>, viewport: Size<Pixels>, item_count: usize) -> Point<Pixels>`
  - `suggestion_key_action(key: &str) -> Option<SuggestionKeyAction>`
  - `App::suggestion_popup`, `App::suggestion_field`, `App::handle_suggestion_key`
  - `App` field: `_suggestion_keys`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn popup_opens_below_when_there_is_room() {
    let input = Bounds { origin: point(px(10.), px(10.)), size: size(px(200.), px(34.)) };
    let origin = suggestion_popup_origin(input, size(px(1000.), px(800.)), 5);
    assert_eq!(origin, point(px(10.), px(48.)));
}

#[test]
fn popup_flips_above_when_below_would_overflow() {
    let input = Bounds { origin: point(px(10.), px(740.)), size: size(px(200.), px(34.)) };
    let origin = suggestion_popup_origin(input, size(px(1000.), px(800.)), 5);
    assert!(origin.y < input.origin.y);
}

#[test]
fn suggestion_keys_map_to_actions() {
    assert_eq!(suggestion_key_action("up"), Some(SuggestionKeyAction::Previous));
    assert_eq!(suggestion_key_action("down"), Some(SuggestionKeyAction::Next));
    assert_eq!(suggestion_key_action("enter"), Some(SuggestionKeyAction::Confirm));
    assert_eq!(suggestion_key_action("escape"), Some(SuggestionKeyAction::Dismiss));
    assert_eq!(suggestion_key_action("a"), None);
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-ui search_suggest::tests`
Expected: FAIL; the module and functions do not exist.

- [ ] **Step 3: Implement the popup and field wrappers**

- Add `pub(crate) mod search_suggest;` to `crates/ui/src/ui.rs`.
- `suggestion_popup_origin`: popup height = `item_count * 27 + 8`; below =
  `input.bottom() + 4`; above = `input.top() - height - 4`; choose below when
  `below + height <= viewport.height - 8`, otherwise above clamped to `8`.
- `suggestion_key_action` maps the four keys.
- `App::suggestion_field(kind, label, input)` wraps the existing
  `filter_field` contents in a `div().relative()` and adds an absolute
  `canvas` that stores the input bounds in `self.suggestion_bounds`.
- `App::suggestion_popup` returns `None` unless a non-dismissed state with
  non-empty items exists. Build the surface with the palette (`panel_2` /
  `panel_deep`, `border_strong`, rounded 11, padding 4, shadow), a fixed row
  height of 27, selected-row `accent_soft` + left accent bar, and the count
  from `tr_args!("search.count", total = format_count(item.photo_count))`.
  Rows select on `on_mouse_down` and use `cx.listener`.
- Attach `on_mouse_down_out` to the surface to call `close_suggestion`.
- Render it with `deferred(anchored().position(origin).snap_to_window_with_margin(px(8.)).child(surface)).with_priority(gpui_kit::base::POPUP_PRIORITY)`.
- In `search_filter_panel`, replace the world/player/tag `filter_field` calls
  with `suggestion_field` and append the popup after the action buttons.
- Implement `handle_suggestion_key`:
  - Return `false` when the screen is not Search, no state exists, the state is
    dismissed, the state has no items, or the active input is not focused.
  - Return `false` when `event.keystroke.is_ime_in_progress()`.
  - `Previous` / `Next`: move the selection and return `true`.
  - `Confirm`: call `select_suggestion(selected, window, cx)` and return `true`.
  - `Dismiss`: call `dismiss_suggestion(cx)` and return `true`.
- Add `_suggestion_keys: Subscription` to `App` and register the interceptor in
  `App::new`:

```rust
let suggestion_entity = cx.entity().downgrade();
let suggestion_keys = cx.intercept_keystrokes(move |event, window, cx| {
    let Some(app) = suggestion_entity.upgrade() else { return; };
    if app.update(cx, |app, cx| app.handle_suggestion_key(event, window, cx)).unwrap_or(false) {
        cx.stop_propagation();
    }
});
```

- [ ] **Step 4: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-ui search_suggest`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/ui.rs crates/ui/src/ui/search.rs crates/ui/src/ui/search_suggest.rs crates/ui/src/app/search_suggest.rs
git commit -m "feat(ui): render search suggestions and handle keyboard selection"
```

---

### Task 6: 検索ライフサイクルと再検索判定

**Files:**
- Modify: `crates/ui/src/app/search.rs:83-117,236-266`
- Modify: `crates/ui/src/app/lifecycle.rs:946-980`
- Modify: `crates/i18n/locales/ja.yml:466`
- Modify: `crates/i18n/locales/en.yml:466`
- Test: `crates/ui/src/app/search.rs` の `mod tests`

**Interfaces:**
- Consumes: `has_tag_filter`, `ai_filter_is_active`, `close_suggestion`。
- Produces: 検索実行・クリア・画面遷移で候補が閉じ、AI タグ更新がタグ欄だけの検索も更新する。

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn ai_tag_edits_refresh_tag_text_only_searches() {
    assert!(ai_edit_refreshes_search(&PhotoTextFilter {
        tag: Some("海".into()),
        ..Default::default()
    }));
    assert!(ai_edit_refreshes_search(&PhotoTextFilter {
        ai: AiPhotoFilter { tags: vec!["sea".into()], people_count: None },
        ..Default::default()
    }));
    assert!(!ai_edit_refreshes_search(&PhotoTextFilter {
        world: Some("Sunset".into()),
        ..Default::default()
    }));
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run: `cargo test -p vrc-photon-ui ai_tag_edits_refresh_tag_text_only_searches`
Expected: FAIL; `ai_edit_refreshes_search` does not exist.

- [ ] **Step 3: Implement the lifecycle changes**

- Add `fn ai_edit_refreshes_search(filter: &PhotoTextFilter) -> bool { ai_filter_is_active(filter) || has_tag_filter(filter) }`.
- Use it in `refresh_search_after_ai_tag_edit`.
- Call `close_suggestion(cx)` at the start of `run_search` and in
  `clear_search` (clear_search already receives `window`; use a `&mut Context`
  variant that does not need the window).
- In `switch_screen`, call `close_suggestion(cx)` when leaving Search.
- Change both i18n values to "選択したタグのいずれかに一致（OR）。タグ欄とは別条件です。"
  and "Matches any selected tag (OR). Separate from the tag field."

- [ ] **Step 4: Run the test and verify it passes**

Run: `cargo test -p vrc-photon-ui ai_tag_edits_refresh_tag_text_only_searches`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/app/search.rs crates/ui/src/app/lifecycle.rs crates/i18n/locales/ja.yml crates/i18n/locales/en.yml
git commit -m "feat(ui): close suggestions on search lifecycle and refresh AI tag edits"
```

---

### Task 7: 全体検証と目視確認

**Files:**
- No source changes expected.

- [ ] **Step 1: Run rustfmt**

Run: `cargo fmt --all -- --check`
Expected: exit 0.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: exit 0.

- [ ] **Step 3: Run the workspace tests**

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 4: Build the UI and launch the demo**

Run: `cargo build -p vrc-photo-ui`
Then launch the demo with the GNOME XWayland capture script (from the main
worktree) using the built binary, navigate to Search, and type into the three
fields. Confirm:

- candidates appear under the field without moving the layout;
- counts and ordering match the spec;
- `↑` / `↓` / `Enter` / `Esc` work, and Japanese IME conversion is not broken;
- the popup flips above near the window bottom and closes on outside click;
- selecting a tag and running Search matches manual and AI tags.

- [ ] **Step 5: Commit only if verification changed tracked files**

If no files changed, skip. Otherwise commit the fixes with a `fix:` prefix.
