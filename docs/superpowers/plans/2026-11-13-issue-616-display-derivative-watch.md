# 表示用派生ファイルの検証をイベント駆動化する (#616) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 表示用派生ファイルの 30 秒ごとの全件 stat を廃止し、notify の削除・移動イベントで該当写真だけを missing 化する。全件確認は起動時・6時間ごと・イベント溢れ時に限定する。

**Architecture:** notify の監視対象を `settings.photo_roots` に加えて DB の `library_roots`（取り込み先を含む）へ広げる。`poll_watchers` は `compressed/` 配下の作成・更新を従来どおり索引から除外しつつ、削除・移動イベントは `dirty_paths` に積む。`index_dirty_paths` はパスが表示用派生なら `Db::display_derivative_owner` で写真を引き、所有ルートがオンラインかつファイルが消えている場合だけ missing 化と表示用紐付け解除を行う。`verify_replaced_assets` はフォールバックとして起動時・6時間ごと・notify の rescan 要求時にだけ実行する。

**Tech Stack:** Rust 2024, notify 8, rusqlite, SQLite

**Spec:** https://github.com/TommyZ-7/vrc-photon/issues/616

**Status:** 決定事項 Q1–Q4 確定（2026-11-13）。計画レビュー待ち。

---

## Global Constraints

- 30 秒周期の全件 stat を廃止する。何も変更がない状態では表示用派生の stat を発生させない。
- 全件確認は daemon 起動時、6時間ごと、notify の `need_rescan` 時だけ実行する。スキャン完了時には実行しない。
- 表示用派生の所有ルート（`library_roots.path`）がディレクトリでない場合は検証しない。オフラインの写真ルートを missing にしない現行挙動を維持する。
- 表示用派生の削除・移動イベントでは、保存済みパスが存在しない場合に `photos.status='missing'` と `photos.display_derivative_id=NULL` を同一トランザクションで行い、`library_revision` を 1 回だけ進める。
- `compressed/` 配下の作成・更新イベントは写真索引に使わない。派生ファイルを原画として索引しない。
- 既存の `verify_replaced_assets` 関連テストの観点（生存ファイル維持、削除検出、オフラインスキップ、DB ロック非保持）を維持する。
- DB schema は変更しない。UI / i18n は変更しない。
- この計画は docs-only PR とし、コード変更は承認後の別 PR で行う。各 PR は最新の `main` から `git worktree` で切る。`main` へ直接 push しない。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` と `./scripts/check-docs-drift.sh`。
- 計画ファイルをコミットする場合、実行完了時にチェックボックスを `- [x]` にする。

## Review Focus

1. **取り込み先の表示用派生**: `settings.photo_roots` に含まれない取り込み先でも、削除イベントで missing になり、表示用紐付けが外れる。Task 2 / Task 3 のテストで固定する。
2. **オフラインルート**: マウント解除中や未接続のルートで削除イベントが届いても missing にしない。Task 3 のテストで固定する。
3. **同一フォルダ内リネーム**: 表示用派生を同じ `compressed/` 内で改名した場合、新しい場所へ自動再紐付けせず、保存済み旧パスが消えた時点で missing になる。Task 3 のテストで固定する。
4. **再作成・空スタット**: 削除イベントの処理時点でファイルが存在すれば missing にせず、`library_revision` も動かさない。Task 3 のテストで固定する。
5. **イベント溢れ**: `EventKind::Other` + `Flag::Rescan` で 6 時間を待たず全件確認が走る。Task 4 のテストで固定する。

## 決定事項（確定）

Q1–Q4 は 2026-11-13 に確定した。実装時はこの表の「確定案」に従う。

| # | 判断項目 | 確定案 | 理由 | 状態 |
|---|---|---|---|---|
| Q1 | 監視対象の広げ方 | `settings.photo_roots` と DB の `library_roots` を再帰監視し、入れ子のルートは親に集約する。派生フォルダを個別に列挙しない | 取り込み先は `ScanRoot` で `library_roots` に追加されるが `photo_roots` には入らない。新規 `compressed/` も再帰監視で自動的に拾える | ユーザー決定済み（2026-11-13） |
| Q2 | 全件確認の実行点 | 起動時（watcher 開始後）+ 6時間ごと + `need_rescan` 時。スキャン完了時の呼び出しと 30 秒周期は削除する。`run_once` も 1 回実行する | 「起動時と 6 時間に 1 回まで」を満たし、手動スキャンの回数に依存しない | ユーザー決定済み（2026-11-13） |
| Q3 | notify エラーの扱い | `need_rescan` だけ全件確認を要求する。その他の `notify` エラーはログのみ | 全件確認の連打を避ける。溢れは rescan フラグで確実に拾える | ユーザー決定済み（2026-11-13） |
| Q4 | 派生フォルダ自体の移動・削除 | ファイル単位のイベントだけ即時処理し、フォルダ自体の移動は 6 時間フォールバックに任せる | 受け入れ条件はファイル削除。フォルダ内の一括削除は再帰監視のファイルイベントで拾える。フォルダ rename は稀で、フォールバックで回復できる | ユーザー決定済み（2026-11-13） |

## 対象外

- 外部で削除された表示用派生の復元・再生成。
- 同一フォルダ内でリネームされた派生ファイルの再紐付け（旧パス消失として missing にする）。
- 派生フォルダ自体の rename を即時検出すること（6 時間フォールバックで回復する）。
- `settings.photo_roots` 外にある取り込み先の原画削除を即時 missing にすること。今回は表示用派生だけを対象にする。
- サムネイルキャッシュの即時削除。
- 手動圧縮の出力（表示用派生にならない）。

---

## File Structure

- `crates/core/src/db/photos.rs`: 表示用派生パスから所有写真とルートを引く `display_derivative_owner`。
- `crates/core/src/db/tests.rs`: `display_derivative_owner` の DB テスト。
- `crates/daemon/src/lib.rs`: `FULL_ASSET_CHECK_INTERVAL`、`is_compressed_path`、`Daemon` の新フィールド初期化。
- `crates/daemon/src/lifecycle.rs`: 監視ルートの決定、watcher 構築、イベント振り分け、全件確認のスケジュール。
- `crates/daemon/src/scan.rs`: `index_dirty_paths` の削除処理、スキャン完了時の全件確認削除。
- `crates/daemon/src/tests.rs`: watcher ルート、イベント処理、スケジュールのテスト。
- `docs/architecture.md`: watcher と表示用派生検証のデータフロー。
- `docs/features.md`: 外部削除時の表示用派生検証とフォールバックの説明。

---

### Task 1: 表示用派生パスから所有写真を引く DB メソッド

**Files:**
- Modify: `crates/core/src/db/photos.rs`（`display_derivative_paths` の近く）
- Test: `crates/core/src/db/tests.rs`

**Interfaces:**
- Produces: `Db::display_derivative_owner(&self, path: &str) -> Result<Option<(i64, String)>>`
  - 戻り値は `(photo_id, library_roots.path)`。
  - `photos.status = 'replaced'` かつ `photos.display_derivative_id` が `path` を指す行だけを返す。
  - `derivatives.path` は UNIQUE インデックスを持つため、1 パス検索は軽量。

- [ ] **Step 1: 失敗する DB テストを書く**

`crates/core/src/db/tests.rs` の `photo_effective_path_prefers_the_display_derivative` の近くに追加する。

```rust
#[test]
fn display_derivative_owner_matches_only_the_active_display_derivative() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let first = db
        .add_derivative(
            photo,
            "/photos/a_1.webp",
            "webp",
            16,
            10,
            8,
            Some("p"),
            CompressionOrigin::Manual,
            "keep",
        )
        .unwrap();
    let second = db
        .add_derivative(
            photo,
            "/photos/a_2.webp",
            "webp",
            16,
            10,
            8,
            Some("p"),
            CompressionOrigin::Manual,
            "keep",
        )
        .unwrap();
    assert!(db.set_replaced(photo, second).unwrap());

    assert_eq!(
        db.display_derivative_owner("/photos/a_2.webp").unwrap(),
        Some((photo, "/photos".to_string()))
    );
    assert_eq!(db.display_derivative_owner("/photos/a_1.webp").unwrap(), None);
    assert_eq!(
        db.display_derivative_owner("/photos/missing.webp").unwrap(),
        None
    );

    db.clear_display_derivative(photo).unwrap();
    assert_eq!(
        db.display_derivative_owner("/photos/a_2.webp").unwrap(),
        None
    );
}
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core display_derivative_owner_matches_only_the_active_display_derivative`

Expected: FAIL（`display_derivative_owner` が存在しない）

- [ ] **Step 3: `Db::display_derivative_owner` を実装する**

`crates/core/src/db/photos.rs` の `display_derivative_paths` の直後に追加する。

```rust
/// Returns the photo that currently displays `path` and the library root
/// that owns it. Only replaced photos with that display derivative match.
pub fn display_derivative_owner(&self, path: &str) -> Result<Option<(i64, String)>> {
    let mut stmt = self.conn.prepare_cached(
        "SELECT p.id, r.path FROM photos p \
         JOIN derivatives d ON d.id = p.display_derivative_id \
         JOIN library_roots r ON r.id = p.root_id \
         WHERE d.path = ?1 AND p.status = 'replaced' LIMIT 1",
    )?;
    stmt.query_row(params![path], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()
        .map_err(Into::into)
}
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core display_derivative_owner_matches_only_the_active_display_derivative`

Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/core/src/db/photos.rs crates/core/src/db/tests.rs
git commit -m "feat(db): look up the photo that displays a derivative"
```

---

### Task 2: 監視ルートをライブラリルートへ広げ、削除イベントを振り分ける

**Files:**
- Modify: `crates/daemon/src/lib.rs`（定数・ヘルパー・`Daemon` フィールド）
- Modify: `crates/daemon/src/lifecycle.rs`（`watched_roots` / `setup_watchers` / `rebuild_watchers` / `poll_watchers` / `poll_commands_with_initial`）
- Test: `crates/daemon/src/tests.rs`

**Interfaces:**
- Produces: `pub(crate) fn is_compressed_path(path: &Path) -> bool`
- Produces: `Daemon::watched_roots(&self) -> Result<Vec<PathBuf>>`
- Produces: `Daemon::setup_watchers(&mut self, roots: Vec<PathBuf>) -> Result<()>`
- Consumes: Task 1 の `Db::display_derivative_owner` はまだ使わない。

- [ ] **Step 1: 失敗する watcher ルートのテストを書く**

`crates/daemon/src/tests.rs` の `command_reloads_settings_and_rebuilds_watchers` の近くに追加する。

```rust
#[test]
fn watcher_roots_include_library_roots_outside_settings_roots() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let imported = dir.path().join("imported");
    std::fs::create_dir_all(&imported).unwrap();
    add_replaced_photo(
        &daemon,
        &imported.join("VRChat_imported.png"),
        &imported.join("compressed/imported.webp"),
    );

    daemon.restart_watchers();

    assert!(daemon.watcher_roots.contains(&imported));
    assert!(daemon.watcher_roots.contains(&dir.path().join("photos")));
}
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon watcher_roots_include_library_roots_outside_settings_roots`

Expected: FAIL（`imported` が監視対象に入らない）

- [ ] **Step 3: 失敗するイベント振り分けテストを書く**

```rust
#[test]
fn poll_watchers_queues_removals_and_ignores_derivative_creates() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.restart_watchers();
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.watcher_rx = Some(rx);
    let photo_dir = dir.path().join("photos");
    let compressed = photo_dir.join("compressed");

    let created = compressed.join("created.webp");
    let removed = compressed.join("removed.webp");
    let old = compressed.join("old.webp");
    let new = compressed.join("new.webp");
    let source = photo_dir.join("VRChat_new.png");

    tx.send(Ok(notify::Event::new(notify::EventKind::Create(
        notify::event::CreateKind::File,
    ))
    .add_path(created)))
        .unwrap();
    tx.send(Ok(notify::Event::new(notify::EventKind::Remove(
        notify::event::RemoveKind::File,
    ))
    .add_path(removed.clone())))
        .unwrap();
    tx.send(Ok(notify::Event::new(notify::EventKind::Modify(
        notify::event::ModifyKind::Name(notify::event::RenameMode::Both),
    ))
    .add_path(old.clone())
    .add_path(new)))
        .unwrap();
    tx.send(Ok(notify::Event::new(notify::EventKind::Remove(
        notify::event::RemoveKind::File,
    ))
    .add_path(source.clone())))
        .unwrap();

    daemon.poll_watchers();

    assert!(!daemon.dirty_paths.contains(&created));
    assert!(daemon.dirty_paths.contains(&removed));
    assert!(daemon.dirty_paths.contains(&old));
    assert!(daemon.dirty_paths.contains(&source));
}
```

- [ ] **Step 4: 失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon poll_watchers_queues_removals_and_ignores_derivative_creates`

Expected: FAIL（`compressed/` の削除パスが捨てられる）

- [ ] **Step 5: `is_compressed_path` を追加する**

`crates/daemon/src/lib.rs` の定数群の近くに追加する。

```rust
/// True when `path` lives under a compressed derivative directory that the
/// incremental photo indexer must not treat as a source photo.
pub(crate) fn is_compressed_path(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str() == "compressed")
}
```

- [ ] **Step 6: 監視ルートを決定する**

`crates/daemon/src/lifecycle.rs` に追加する。

```rust
/// Roots covered by the current watchers: configured photo roots plus every
/// library root recorded by scans (import destinations are stored only in
/// `library_roots`). Nested roots collapse to their parent because one
/// recursive watch covers the subtree.
pub(crate) fn watched_roots(&self) -> Result<Vec<PathBuf>> {
    let mut roots: Vec<PathBuf> = self.settings.photo_roots();
    {
        let db = lock_db(&self.db);
        roots.extend(db.roots()?.into_iter().map(|root| PathBuf::from(root.path)));
    }
    roots.retain(|root| root.is_dir());
    roots.sort();
    roots.dedup();
    let pruned: Vec<PathBuf> = roots
        .iter()
        .filter(|root| !roots.iter().any(|other| other != *root && root.starts_with(other)))
        .cloned()
        .collect();
    Ok(pruned)
}
```

`setup_watchers` を `pub(crate) fn setup_watchers(&mut self, roots: Vec<PathBuf>) -> Result<()>` に変え、`self.settings.photo_roots()` のループを `roots` のループに置き換える。`watcher_roots = roots` を設定する。

`rebuild_watchers` は次の形にする。

```rust
self.poll_watchers();
let wanted = self.watched_roots()?;
if self.watcher_rx.is_some() && self.watcher_roots == wanted {
    return Ok(());
}
self.watchers.clear();
self.watcher_rx = None;
self.watcher_roots.clear();
let result = self.setup_watchers(wanted);
if result.is_err() {
    self.watchers.clear();
    self.watcher_roots.clear();
}
result
```

`poll_commands_with_initial` のコマンドループの後、`self.refresh_status()` の前に `self.restart_watchers();` を追加する。`ScanRoot` で追加されたルートを次のループを待たずに監視へ反映するため。

- [ ] **Step 7: `poll_watchers` を削除・移動イベント対応にする**

`poll_watchers` のイベント分岐を次の形にする。作成・更新は従来どおり `compressed/` と原画以外を捨て、削除・移動はパス種別を問わず `dirty_paths` に積む。

```rust
let removal = matches!(
    &event.kind,
    notify::EventKind::Remove(_)
        | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
);
let relevant = matches!(
    &event.kind,
    notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_)
);
if relevant {
    for path in event.paths {
        if removal || (!is_compressed_path(&path) && is_source_photo(&path)) {
            self.dirty_paths.push(path);
        }
    }
}
```

- [ ] **Step 8: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon watcher_roots_include_library_roots_outside_settings_roots`

Expected: PASS

Run: `cargo test -p vrc-photon-daemon poll_watchers_queues_removals_and_ignores_derivative_creates`

Expected: PASS

- [ ] **Step 9: 既存の watcher テストを確認する**

Run: `cargo test -p vrc-photon-daemon watcher`

Expected: PASS（`poll_commands_preserves_buffered_watcher_events` などの既存観点を壊さない）

- [ ] **Step 10: コミット**

```bash
git add crates/daemon/src/lib.rs crates/daemon/src/lifecycle.rs crates/daemon/src/tests.rs
git commit -m "feat(daemon): watch library roots and route derivative events"
```

---

### Task 3: 削除・移動イベントで表示用派生を検証する

**Files:**
- Modify: `crates/daemon/src/scan.rs`（`index_dirty_paths`）
- Test: `crates/daemon/src/tests.rs`

**Interfaces:**
- Consumes: Task 1 の `Db::display_derivative_owner`、Task 2 の `is_compressed_path` / `poll_watchers`。
- Consumes: 既存の `missing_display_derivatives(rows, root_is_dir, file_is_file) -> Vec<i64>`（`crates/daemon/src/logs.rs`）。
- Produces: なし（`index_dirty_paths` の内部挙動）。

- [ ] **Step 1: 失敗する削除イベントのテストを書く**

`crates/daemon/src/tests.rs` の `verify_replaced_assets_*` テスト群の近くに追加する。

```rust
#[test]
fn removed_display_derivative_marks_photo_missing_and_clears_display() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo_dir = dir.path().join("photos");
    let compressed = photo_dir.join("compressed");
    std::fs::create_dir_all(&compressed).unwrap();
    let derivative = compressed.join("live.webp");
    std::fs::write(&derivative, b"live").unwrap();
    let id = add_replaced_photo(
        &daemon,
        &photo_dir.join("VRChat_live.png"),
        &derivative,
    );
    let revision = daemon.db.lock().unwrap().library_revision().unwrap();
    daemon.restart_watchers();
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.watcher_rx = Some(rx);

    std::fs::remove_file(&derivative).unwrap();
    tx.send(Ok(notify::Event::new(notify::EventKind::Remove(
        notify::event::RemoveKind::File,
    ))
    .add_path(derivative)))
        .unwrap();
    daemon.poll_watchers();
    daemon.index_dirty_paths().unwrap();

    let db = daemon.db.lock().unwrap();
    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Missing);
    assert!(photo.display_derivative_id.is_none());
    assert!(photo.display_path.is_none());
    assert_eq!(db.library_revision().unwrap(), revision + 1);
    assert_eq!(db.count_photos().unwrap(), 0);
}
```

- [ ] **Step 2: 失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon removed_display_derivative_marks_photo_missing_and_clears_display`

Expected: FAIL（写真が `replaced` のまま）

- [ ] **Step 3: `index_dirty_paths` の削除処理を実装する**

ループの先頭で `path.is_file()` を分岐する形に組み替える。

- ファイル（作成・更新）: 所有ルートが `settings.photo_roots` にある場合だけ処理する。`is_compressed_path(&path)` ならスキップする。既存の `root_id` 取得、`scan_one`、`upsert_photo`、rebind はそのままファイル分岐へ移す。
- 非ファイル（削除・移動）: まず `settings.photo_roots` 配下なら `photo_by_path` で原画を探し、既存どおり `mark_missing_if_path` する。見つからない場合（または設定ルート外）は `display_derivative_owner` を引き、候補を `removed_display_rows: Vec<(i64, Option<String>, String)>` に積む。
- ループ後に `missing_display_derivatives(removed_display_rows, |path| path.is_dir(), |path| path.is_file())` を呼び、オフラインルートと実在ファイルを除外する。
- 返った ID を既存の最終ロックブロックで `in_transaction` にまとめ、`mark_missing` と `clear_display_derivative` を実行する。`changed = true` にする。
- 最終ブロックの `if changed { db.bump_library_revision()?; }` はそのまま 1 回だけ動かす。

`index_dirty_paths` の非ファイル分岐の骨格:

```rust
let source_photo = match source_root {
    Some(_) => {
        let db = lock_db(&self.db);
        db.photo_by_path(&path.to_string_lossy())?
    }
    None => None,
};
if let Some(photo) = source_photo {
    if photo.status == PhotoStatus::Present {
        // existing mark_missing_if_path handling
    }
    continue;
}
let owner = {
    let db = lock_db(&self.db);
    db.display_derivative_owner(&path.to_string_lossy())?
};
if let Some((photo_id, root)) = owner {
    removed_display_rows.push((
        photo_id,
        Some(path.to_string_lossy().into_owned()),
        root,
    ));
}
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon removed_display_derivative_marks_photo_missing_and_clears_display`

Expected: PASS

- [ ] **Step 5: 移動・生存・オフライン・取り込み先のテストを追加する**

```rust
#[test]
fn renamed_display_derivative_marks_photo_missing() {
    // compressed/live.webp を compressed/moved.webp へ rename し、
    // Modify(Name(Both)) に旧パスと新パスを入れて送る。
    // 旧パスが消えているため Missing、display_derivative_id は None。
}

#[test]
fn removed_display_derivative_with_a_live_file_keeps_the_photo() {
    // ファイルを消さずに Remove イベントだけ送る。
    // Replaced のまま、display_derivative_id と library_revision が変わらない。
}

#[test]
fn removed_display_derivative_skips_an_offline_root() {
    // 存在しない offline-drive 配下の派生行を作り、Remove イベントを送る。
    // Replaced のまま、revision も変わらない（既存 verify テストと同じ観点）。
}

#[test]
fn removed_display_derivative_outside_settings_roots_is_handled() {
    // imported ディレクトリだけを library_root に持ち、settings.photo_roots は photos のまま。
    // imported/compressed/live.webp を削除して Remove イベントを送る。
    // Missing になり、表示用紐付けが外れる。
}

#[test]
fn compressed_derivative_file_is_not_indexed_as_a_source_photo() {
    // compressed/generated.jpg を実ファイルとして置き、dirty_paths に直接積む。
    // index_dirty_paths 後も photo_by_path は None、count_photos は変わらない。
}

#[test]
fn removed_unknown_path_does_not_bump_library_revision() {
    // 表示用派生でも原画でもないファイルの Remove イベントを送る。
    // revision が変わらない。
}
```

各テストは Step 1 と同じく手動チャネルへイベントを送り、`poll_watchers` → `index_dirty_paths` の順で検証する。

- [ ] **Step 6: 追加テストと既存の増分索引テストを通す**

Run: `cargo test -p vrc-photon-daemon index_dirty_paths`

Expected: PASS（原画の削除・移動・再結合を含む既存テストを壊さない）

Run: `cargo test -p vrc-photon-daemon removed_display_derivative`

Expected: PASS

- [ ] **Step 7: コミット**

```bash
git add crates/daemon/src/scan.rs crates/daemon/src/tests.rs
git commit -m "feat(daemon): mark photos missing when display derivatives disappear"
```

---

### Task 4: 全件確認を起動時・6時間ごと・rescan 時に限定する

**Files:**
- Modify: `crates/daemon/src/lib.rs`（`FULL_ASSET_CHECK_INTERVAL`）
- Modify: `crates/daemon/src/lifecycle.rs`（`run_loop_iteration` / `next_loop_wait` / `run` / `run_once` / `run_full_asset_check`）
- Modify: `crates/daemon/src/scan.rs`（スキャン完了時の `verify_replaced_assets` 呼び出し削除）
- Test: `crates/daemon/src/tests.rs`

**Interfaces:**
- Produces: `const FULL_ASSET_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);`
- Produces: `Daemon::full_asset_check_due: bool`（`Daemon::open` で `false`）
- Produces: `Daemon::run_full_asset_check(&mut self)`
  - `verify_replaced_assets` を呼び、成否にかかわらず `last_asset_check` を現在時刻へ更新し、`full_asset_check_due` を false にする。

- [ ] **Step 1: 失敗するスケジュールテストを書く**

```rust
#[test]
fn full_asset_check_does_not_run_before_six_hours() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo_dir = dir.path().join("photos");
    let compressed = photo_dir.join("compressed");
    std::fs::create_dir_all(&compressed).unwrap();
    let derivative = compressed.join("missing.webp");
    let id = add_replaced_photo(
        &daemon,
        &photo_dir.join("VRChat_missing.png"),
        &derivative,
    );
    daemon.last_asset_check = Instant::now();

    daemon.run_loop_iteration();

    let db = daemon.db.lock().unwrap();
    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Replaced);
}

#[test]
fn full_asset_check_runs_after_six_hours() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo_dir = dir.path().join("photos");
    let compressed = photo_dir.join("compressed");
    std::fs::create_dir_all(&compressed).unwrap();
    let derivative = compressed.join("missing.webp");
    let id = add_replaced_photo(
        &daemon,
        &photo_dir.join("VRChat_missing.png"),
        &derivative,
    );
    daemon.last_asset_check = Instant::now() - FULL_ASSET_CHECK_INTERVAL - Duration::from_secs(1);

    daemon.run_loop_iteration();

    let db = daemon.db.lock().unwrap();
    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Missing);
}
```

- [ ] **Step 2: 失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon full_asset_check_`

Expected: FAIL（`FULL_ASSET_CHECK_INTERVAL` が存在しない。直しても 30 秒周期のままで挙動が合わない）

- [ ] **Step 3: 定数と実行ラッパーを追加する**

`crates/daemon/src/lib.rs` に追加する。

```rust
/// Full display-derivative verification runs at startup, every six hours,
/// and when the watcher reports lost events. Event handling covers deletions
/// in between.
const FULL_ASSET_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
```

`crates/daemon/src/lifecycle.rs` に追加する。

```rust
pub(crate) fn run_full_asset_check(&mut self) {
    if let Err(error) = self.verify_replaced_assets() {
        tracing::warn!(%error, "cannot verify replaced photo assets");
    }
    self.last_asset_check = Instant::now();
    self.full_asset_check_due = false;
}
```

`Daemon` に `pub(crate) full_asset_check_due: bool` を追加し、`Daemon::open` で `false` に初期化する。

- [ ] **Step 4: 実行点を差し替える**

- `poll_watchers` のイベント分岐の先頭で `need_rescan` を記録する。

```rust
if event.need_rescan() {
    self.full_asset_check_due = true;
}
```


- `run_loop_iteration` の `if !paused && self.last_asset_check.elapsed() >= Duration::from_secs(30)` を次に置き換える。

```rust
let asset_check_due =
    self.full_asset_check_due || self.last_asset_check.elapsed() >= FULL_ASSET_CHECK_INTERVAL;
if !paused && asset_check_due {
    self.run_full_asset_check();
}
```

- `next_loop_wait` の asset deadline を次に置き換える。

```rust
let asset_deadline = if self.is_paused() {
    now + FULL_ASSET_CHECK_INTERVAL
} else if self.full_asset_check_due {
    now
} else {
    self.last_asset_check + FULL_ASSET_CHECK_INTERVAL
};
```

- `Daemon::open` の `last_asset_check` を `Instant::now()` に変える。
- `run()` の `self.restart_watchers();` の直後に `self.run_full_asset_check();` を追加する。
- `run_once()` の `self.link_unlinked()?;` の後、`self.refresh_status()?;` の前に `self.run_full_asset_check();` を追加する。
- `crates/daemon/src/scan.rs` の `scan_roots_inner` から `self.verify_replaced_assets()?;` を削除する。
- `verify_replaced_assets` の doc コメントを「イベント駆動のフォールバック。起動時・6時間ごと・rescan 要求時だけ実行する」に更新する。

- [ ] **Step 5: スケジュールテストを通す**

Run: `cargo test -p vrc-photon-daemon full_asset_check_`

Expected: PASS

- [ ] **Step 6: 起動時と rescan のテストを追加する**

```rust
#[test]
fn run_once_runs_the_full_asset_check() {
    // 派生ファイルが存在しない replaced 写真を用意して run_once。
    // Missing になり、表示用紐付けが外れる。
}

#[test]
fn rescan_flag_forces_the_full_asset_check() {
    // last_asset_check = Instant::now() のまま、
    // EventKind::Other + Flag::Rescan を送って run_loop_iteration。
    // 6 時間を待たず Missing になる。
}
```

- [ ] **Step 7: 既存テストを更新する**

`paused_idle_wait_does_not_spin_on_an_overdue_asset_check` の `last_asset_check` を `now - FULL_ASSET_CHECK_INTERVAL - Duration::from_secs(1)` に更新する。

- [ ] **Step 8: daemon のテストを通す**

Run: `cargo test -p vrc-photon-daemon`

Expected: PASS

- [ ] **Step 9: コミット**

```bash
git add crates/daemon/src/lib.rs crates/daemon/src/lifecycle.rs crates/daemon/src/scan.rs crates/daemon/src/tests.rs
git commit -m "feat(daemon): limit full asset checks to startup and six-hour intervals"
```

---

### Task 5: ドキュメント更新と全体検証

**Files:**
- Modify: `docs/architecture.md`（4.2 写真の索引）
- Modify: `docs/features.md`（1. 写真の索引）

- [ ] **Step 1: `docs/architecture.md` を更新する**

4.2 の手順を次の内容に合わせる。

- 1: notify は写真ルートとライブラリルート（取り込み先を含む）を監視する。
- 2: 作成・更新イベントは `compressed/` 配下と原画以外を除外して `dirty_paths` に積む。削除・移動イベントは表示用派生ファイルも対象にする。
- 6 の後: 表示用派生が外部で削除・移動された場合、所有ルートがオンラインで保存済みパスが存在しないときだけ `status='missing'` と表示用紐付け解除を行う。全件確認は起動時・6時間ごと・notify の rescan 要求時だけ行う。

- [ ] **Step 2: `docs/features.md` を更新する**

- `compressed/` の行を「索引の走査対象外。削除・移動イベントは表示用派生の検証に使う」に更新する。
- 外部削除の行に、表示用派生は notify の削除・移動イベントで検出し、全件確認は起動時と 6 時間ごと、イベント溢れ時だけ行うことを追記する。

- [ ] **Step 3: ドキュメントのドリフト検査を通す**

Run: `./scripts/check-docs-drift.sh`

Expected: PASS

- [ ] **Step 4: 全体検証を通す**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add docs/architecture.md docs/features.md
git commit -m "docs: describe display derivative event verification"
```

---

## Self-Review

- 仕様の各項目: 30 秒周期の廃止（Task 4）、watcher 対象への派生フォルダ追加（Task 2）、削除・移動イベントだけの存在確認と missing 化（Task 3）、起動時・6 時間・溢れ時の全件確認（Task 4）、オフラインスキップ（Task 3）、既存テスト観点の維持（Task 4 のテスト含む）。
- 型の一貫性: `display_derivative_owner` は Task 1 で定義し Task 3 で使う。`is_compressed_path` は Task 2 で定義し Task 3 で使う。`full_asset_check_due` は Task 4 で追加して同じタスクで消費する。
- 計画の長さ: 実装コードはシグネチャと分岐骨格に留め、テスト名と主要な assert を固定した。
