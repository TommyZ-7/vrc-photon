# Daemon Idle Polling Reduction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** アイドル中のデーモンから固定間隔の重い DB アクセスと `/proc` / ログディレクトリ全走査を除き、変化検知とシグナル起床へ置き換える。

**Architecture:** プロセス検出は PID キャッシュ、最新ログ特定はディレクトリ mtime ゲート、ハッシュワーカーは JobSignal と ID カーソル、ステータスは session（2秒）と counts（30秒＋イベント）に分割する。全ワーカーの空転待機を長いシグナル待ちに統一し、メインループは期限ベースで起床する。IPC の型・プロトコルバージョン・セッション終了時刻の意味は変えない。

**Tech Stack:** Rust stable（edition 2024）、SQLite / rusqlite、notify 8、std の `Mutex` / `Condvar` / `mpsc`、Windows API（`windows` 0.62）。

**Spec:** 本計画の「決定事項」節（2026-11-12 のブレインストーミングでユーザー承認済み）。別途の spec ファイルは作らない。

## 決定事項

| 項目 | 決定 |
|---|---|
| スコープ | 指定4件＋ `poll_commands` / 設定再読込＋ discord / deletion / メインループ＋ compression / tagging / thumbnail の空転待機 |
| 対象 OS | Linux と Windows の両方で PID キャッシュを実装 |
| VRChat 検出 | 5秒間隔＋PIDキャッシュ。3回連続 miss で close（最大15秒遅延） |
| ステータス | session 2秒 / counts 30秒＋イベント時即時 |
| 最新ログ | ディレクトリ mtime ゲート＋60秒フォールバック。追記ポーリングは750ms維持 |
| ハッシュ | JobSignal＋60秒フォールバック＋カーソル方式で先頭詰まりも修正 |
| 進め方 | 計画書のみ作成。承認後、別 worktree で実装 |

## 現状の実測（この環境）

| 対象 | 実測 | 現状の頻度 |
|---|---|---|
| `/proc` 全走査（360 PID） | 1.63ms/scan | 750ms（セッション中） |
| `count_photos`（100k / 300k、covering index） | 2.8ms / 8.3ms | 500ms |
| ログディレクトリ `read_dir`（5,000ファイル） | 0.91ms | 750ms |
| ハッシュ空転 SELECT | 0.003ms | 2秒 |
| `take_daemon_commands`（空キューでも即時トランザクション） | 計測対象外 | 500ms |
| 設定 JSON 再読込 | 計測対象外 | 500ms |
| compression / tagging 空転待機 | — | 400ms |
| discord / deletion 空転待機 | — | 500ms |
| thumbnail 空転待機 | — | 500ms |

## Global Constraints

- 新規クレートを追加しない。`Cargo.toml` の依存追加は Windows API の feature だけにする。
- IPC の `StatusSnapshot` のフィールドと `PROTOCOL_VERSION` を変えない。
- `VRCHAT_MISSING_POLLS_BEFORE_CLOSE = 3` とセッション終了時刻の算出（ログ mtime）を変えない。
- ログ追記の読み取りは750ms間隔を維持する（`docs/log-and-xmp.md`）。
- ワーカーの空転待機を `#[cfg(test)]` で短縮しない。テストは対応するシグナルを明示的に notify する。
- コード変更とドキュメント更新は別 PR にする（AGENTS.md §4）。本計画書は docs-only PR、実装は code PR、最後に docs PR。
- 実装は `../vrc-photo-manager-daemon-idle` の worktree と `fix/daemon-idle-polling` ブランチで行う。`main` では作業しない。
- ローカル検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## Review Focus

実装後に特に壊れやすい入力・状態。各項目のテストは末尾のタスクに含める。

1. **PID 再利用**: キャッシュした PID が別プロセスに再利用されても「VRChat が動いている」と誤判定しない。comm が一致しなければ全走査へ落ちる。
2. **プロセスを読めない**: キャッシュ PID の `comm` が読めないときは全走査へ落ちる。全走査も失敗したときだけ `None` を返し、miss カウントを進めない。
3. **ログディレクトリの mtime が更新されない**: ネットワーク/FAT では新規ファイルを見落とす可能性がある。60秒フォールバックとキャッシュファイルの消失検知で回復する。
4. **未ハッシュ行の先頭詰まり**: 低 ID の行が16件連続で失敗しても、後続の行がハッシュされる。
5. **pause / resume**: 30〜60秒待機中のワーカーが resume 通知で即座に再開する。
6. **サムネイル設定変更**: ワーカーが長く眠っていても、上限・解像度・キャッシュクリアが反映される。
7. **IPC wake なしのコマンド**: `WakeCommands` が届かなくても、フォールバック周期内にコマンドが処理される。
8. **counts の30秒遅延**: UI と MCP がデーモンの `StatusSnapshot` の counts に依存していないことを確認する。

---

## Task 1: VRChat プロセス検出の PID キャッシュと5秒タイマー

**Files:**
- Modify: `crates/daemon/src/vrchat_process.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/Cargo.toml`
- Test: `crates/daemon/src/vrchat_process.rs`（inline tests）、`crates/daemon/src/tests.rs`

**Interfaces:**
- Consumes: 既存の `is_vrchat_comm(&str) -> bool`。
- Produces:
  - `pub(crate) struct VrchatProcessProbe { cached_pid: Option<u32> }`
  - `impl VrchatProcessProbe { pub(crate) fn new() -> Self; pub(crate) fn poll(&mut self) -> Option<bool>; }`
  - `pub fn is_running() -> Option<bool>`（1回だけ probe を生成して `poll()` する互換ラッパー）
  - `const PRESENCE_INTERVAL: Duration = Duration::from_secs(5);`
  - `Daemon.vrchat_probe: VrchatProcessProbe`
  - `Daemon.last_presence_check: Instant`

- [x] **Step 1: PID キャッシュの失敗テストを書く**

`crates/daemon/src/vrchat_process.rs` の tests に、テスト用の `ProcessTable` 実装を追加する。

```rust
struct FakeTable {
    processes: std::sync::Mutex<Vec<(u32, String)>>,
    list_calls: std::sync::atomic::AtomicUsize,
    comm_calls: std::sync::atomic::AtomicUsize,
    fail_comm: Option<std::io::ErrorKind>,
    fail_list: Option<std::io::ErrorKind>,
}

impl FakeTable {
    fn new(processes: &[(u32, &str)]) -> Self { /* ... */ }
}

impl ProcessTable for FakeTable {
    fn list(&self) -> std::io::Result<Vec<(u32, String)>> { /* increment list_calls, clone processes */ }
    fn comm(&self, pid: u32) -> std::io::Result<Option<String>> { /* increment comm_calls; fail_comm なら Err; 見つからなければ Ok(None) */ }
}
```

テスト:

```rust
#[test]
fn probe_reuses_the_cached_pid_without_listing_the_table()
#[test]
fn probe_rescans_when_the_cached_pid_disappears()
#[test]
fn probe_falls_back_to_a_full_scan_when_the_cached_pid_cannot_be_read()
#[test]
fn probe_returns_none_when_the_process_table_cannot_be_read()
#[test]
fn probe_returns_false_when_vrchat_is_absent()
```

期待する assertion:
- 1回目の `poll` で `list_calls == 1`。2回目は `list_calls == 1` のまま、`comm_calls == 1`（キャッシュ PID の確認だけ）。
- キャッシュ PID を消して別 PID に VRChat を置くと `poll == Some(true)`、キャッシュが新しい PID になる。
- `comm` が `PermissionDenied` を返し、一覧に VRChat がいれば `poll == Some(true)`（アクセス拒否で見落とさない）。
- `comm` と `list` の両方がエラーなら `poll == None`、`list_calls == 1`。
- プロセス一覧が空なら `Some(false)`。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon vrchat_process::tests -- --nocapture`

Expected: `VrchatProcessProbe` / `ProcessTable` が未定義でコンパイルエラー。

- [x] **Step 3: `VrchatProcessProbe` と `ProcessTable` を実装する**

共通ロジック:

```rust
impl VrchatProcessProbe {
    pub(crate) fn poll(&mut self) -> Option<bool> {
        #[cfg(target_os = "linux")]
        return self.poll_with(&LinuxProcessTable);

        #[cfg(target_os = "windows")]
        return self.poll_with(&WindowsProcessTable);

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        None
    }

    fn poll_with(&mut self, table: &impl ProcessTable) -> Option<bool> {
        if let Some(pid) = self.cached_pid {
            match table.comm(pid) {
                Ok(Some(comm)) if is_vrchat_comm(&comm) => return Some(true),
                Ok(_) => self.cached_pid = None,
                // アクセス拒否でも一覧には名前が出る（Windows の昇格プロセス等）。
                // 誤って「動いていない」としないため、全走査へ落ちる。
                Err(_) => self.cached_pid = None,
            }
        }
        match table.list() {
            Ok(processes) => match processes.iter().find(|(_, comm)| is_vrchat_comm(comm)) {
                Some((pid, _)) => {
                    self.cached_pid = Some(*pid);
                    Some(true)
                }
                None => {
                    self.cached_pid = None;
                    Some(false)
                }
            },
            Err(_) => None,
        }
    }
}
```

- Linux `ProcessTable`:
  - `list`: `/proc` を `read_dir` し、数字 PID の `comm` を読む。`NotFound` は読み飛ばし、それ以外のエラーは `Err`。`(pid, comm)` の Vec を返す。
  - `comm(pid)`: `/proc/<pid>/comm` を読む。`NotFound` は `Ok(None)`、それ以外は `Err`。
- Windows `ProcessTable`:
  - `list`: 既存の `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)` を列挙し、`(th32ProcessID, szExeFile)` を返す。終端エラーは `Ok`。
  - `comm(pid)`: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)` → `QueryFullProcessImageNameW` → ファイル名を返す。`ERROR_INVALID_PARAMETER` は `Ok(None)`。アクセス拒否は `Err`。
  - `CloseHandle` を必ず1回呼ぶ。既存の unsafe コメント様式に合わせる。
- `crates/daemon/Cargo.toml` の Windows features に `Win32_System_Threading` を追加する。
- 互換ラッパーは次の形にする:

```rust
pub fn is_running() -> Option<bool> {
    let mut probe = VrchatProcessProbe::new();
    probe.poll()
}
```

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon vrchat_process::tests -- --nocapture`

Expected: PASS。`cargo check -p vrc-photon-daemon` も通る。

- [x] **Step 5: 5秒タイマーの失敗テストを書く**

`crates/daemon/src/tests.rs`:

```rust
#[test]
fn presence_check_runs_only_after_its_interval() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    daemon.last_presence_check = Instant::now() - PRESENCE_INTERVAL;
    daemon.run_loop_iteration();
    let after_due = daemon.last_presence_check;
    assert!(after_due.elapsed() < Duration::from_secs(1));

    daemon.run_loop_iteration();
    assert_eq!(daemon.last_presence_check, after_due);
}
```

- [x] **Step 6: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon presence_check_runs_only_after_its_interval -- --nocapture`

Expected: FAIL（`last_presence_check` が存在しない、または毎回更新される）。

- [x] **Step 7: タイマーと probe をメインループへ組み込む**

`crates/daemon/src/lib.rs` に追加:

```rust
const PRESENCE_INTERVAL: Duration = Duration::from_secs(5);
```

`Daemon` に `vrchat_probe: VrchatProcessProbe` と `last_presence_check: Instant` を追加し、`open_inner` で初期化する。`last_presence_check` は `Instant::now() - PRESENCE_INTERVAL`。

`run_loop_iteration` のログブロックから `poll_vrchat_presence` を外し、独立したブロックにする:

```rust
if self.last_log_poll.elapsed() >= LOG_INTERVAL {
    if let Err(error) = self.ingest_logs() { /* ... */ }
    self.last_log_poll = Instant::now();
}
if self.last_presence_check.elapsed() >= PRESENCE_INTERVAL {
    if let Err(error) = self.poll_vrchat_presence() { /* ... */ }
    self.last_presence_check = Instant::now();
}
```

`poll_vrchat_presence` は `vrchat_process::is_running()` ではなく、次の形にする:

```rust
let running = self.vrchat_probe.poll();
self.observe_vrchat_presence(running)
```

`repair_open_sessions` は起動時の1回だけなので、既存の `vrchat_process::is_running()` をそのまま使う。

- [x] **Step 8: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon presence_check_runs_only_after_its_interval vrchat_process::tests -- --nocapture`

Expected: PASS。

- [x] **Step 9: コミット**

```bash
git add crates/daemon/src/vrchat_process.rs crates/daemon/src/lifecycle.rs crates/daemon/src/lib.rs crates/daemon/Cargo.toml crates/daemon/src/tests.rs
git commit -m "fix(daemon): cache the VRChat process id between presence polls"
```

---

## Task 2: 最新ログファイルの mtime ゲート

**Files:**
- Modify: `crates/daemon/src/logs.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/daemon/src/logs.rs`（inline tests）、`crates/daemon/src/tests.rs`

**Interfaces:**
- Consumes: 既存の `latest_log_file(dir: &Path) -> Result<Option<PathBuf>>`。
- Produces:
  - `pub(crate) struct LogLocator { dir: PathBuf, dir_mtime: Option<SystemTime>, latest: Option<PathBuf>, last_scan: Instant, #[cfg(test)] scans: usize }`
  - `impl LogLocator { pub(crate) fn new(dir: PathBuf) -> Self; pub(crate) fn latest(&mut self) -> Result<Option<PathBuf>>; pub(crate) fn dir(&self) -> &Path; pub(crate) fn invalidate(&mut self); }`
  - `const LOG_DIR_RESCAN_FALLBACK: Duration = Duration::from_secs(60);`
  - `Daemon.log_locator: Option<LogLocator>`

- [x] **Step 1: `LogLocator` の失敗テストを書く**

`crates/daemon/src/logs.rs` の tests:

```rust
#[test]
fn locator_reuses_the_cached_latest_without_rescanning()
#[test]
fn locator_detects_a_new_log_file()
#[test]
fn locator_rescans_when_the_cached_file_disappears()
#[test]
fn locator_rescans_after_the_fallback_interval()
```

`#[cfg(test)] scans` を `latest()` の `read_dir` 回数として使う。assertion:
- 変更なしで2回呼ぶと `scans == 1`。
- 新しい `output_log_*.txt` を作ると2回目で `scans == 2`、新しいファイルを返す。
- キャッシュしたファイルを削除すると `scans == 3`、`None` を返す。
- `last_scan = Instant::now() - LOG_DIR_RESCAN_FALLBACK - 1s` にすると `scans` が増える。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon logs::tests -- --nocapture`

Expected: `LogLocator` が未定義でコンパイルエラー。

- [x] **Step 3: `LogLocator` を実装する**

```rust
pub(crate) fn latest(&mut self) -> Result<Option<PathBuf>> {
    let metadata = match std::fs::metadata(&self.dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            self.invalidate();
            return Ok(None);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read log folder {}", self.dir.display()));
        }
    };
    let mtime = metadata.modified().ok();
    let cached_ok = self.dir_mtime.is_some()
        && mtime == self.dir_mtime
        && self.latest.as_ref().is_none_or(|path| path.is_file());
    let fallback_due = self.last_scan.elapsed() >= LOG_DIR_RESCAN_FALLBACK;
    if cached_ok && !fallback_due {
        return Ok(self.latest.clone());
    }

    #[cfg(test)]
    {
        self.scans += 1;
    }
    let latest = latest_log_file(&self.dir)?;
    self.latest = latest.clone();
    self.dir_mtime = mtime;
    self.last_scan = Instant::now();
    Ok(latest)
}
```

- `invalidate`: `latest = None`、`dir_mtime = None`、`last_scan = Instant::now() - LOG_DIR_RESCAN_FALLBACK`。
- `new`: `last_scan = Instant::now()`、`#[cfg(test)] scans = 0`。
- `dir()` は `&self.dir`。
- `LOG_DIR_RESCAN_FALLBACK` は `lib.rs` に置く。

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon logs::tests -- --nocapture`

Expected: PASS。

- [x] **Step 5: `ingest_logs` と設定変更へ組み込む**

`Daemon` に `log_locator: Option<LogLocator>` を追加（初期値 `None`）。

`ingest_logs`:

```rust
if self
    .log_locator
    .as_ref()
    .is_none_or(|locator| locator.dir() != log_dir.as_path())
{
    self.log_locator = Some(LogLocator::new(log_dir.clone()));
}
let path = self.log_locator.as_mut().unwrap().latest();
```

`reload_settings` で `log_dir` が変わったら `self.log_locator = None;`。`settings.log_dir` の比較は `self.settings = settings` の前に行う。

`ingest_logs` の既存の `log_dir.is_dir()` エラー報告は残す。`latest()` が `Err` のときの `daemon.log_unreadable` 報告も残す。

- [x] **Step 6: 設定変更のテストを追加する**

`crates/daemon/src/tests.rs`:

```rust
#[test]
fn changing_the_log_dir_invalidates_the_locator() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.ingest_logs().unwrap();
    assert!(daemon.log_locator.is_some());

    save_daemon_settings(&daemon, |settings| {
        settings.log_dir = Some(dir.path().join("logs2").to_string_lossy().to_string());
    });
    daemon.reload_settings().unwrap();
    assert!(daemon.log_locator.is_none());
}
```

- [x] **Step 7: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon changing_the_log_dir_invalidates_the_locator logs::tests -- --nocapture`

Expected: PASS。

- [x] **Step 8: コミット**

```bash
git add crates/daemon/src/logs.rs crates/daemon/src/lifecycle.rs crates/daemon/src/lib.rs crates/daemon/src/tests.rs
git commit -m "fix(daemon): avoid rescanning the log directory every poll"
```

---

## Task 3: ハッシュワーカーのシグナル化と ID カーソル

**Files:**
- Modify: `crates/core/src/db/photos.rs`
- Modify: `crates/daemon/src/hash.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Modify: `crates/daemon/src/scan.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/core/src/db/tests.rs`、`crates/daemon/src/tests.rs`

**Interfaces:**
- Consumes: 既存の `JobSignal`、`hashing::file_blake3`、`Db::set_photo_blake3`、`Db::bump_library_revision`。
- Produces:
  - `Db::photos_without_hash_after(&self, limit: i64, after_id: i64) -> Result<Vec<(i64, String)>>`
  - `hash_loop(db: Arc<Mutex<Db>>, running: Arc<AtomicBool>, paused: Arc<AtomicBool>, wake: Arc<JobSignal>)`
  - `Daemon.hash_wake: Arc<JobSignal>`
  - `const HASH_IDLE_WAIT: Duration = Duration::from_secs(60);`

- [x] **Step 1: ID カーソルの失敗テストを書く**

`crates/core/src/db/tests.rs`:

```rust
#[test]
fn photos_without_hash_after_skips_earlier_ids() {
    // id 1,2,3 を blake3 NULL で作る
    assert_eq!(db.photos_without_hash_after(2, 0).unwrap().iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(db.photos_without_hash_after(2, 2).unwrap().iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![3]);
    assert!(db.photos_without_hash_after(2, 3).unwrap().is_empty());
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core photos_without_hash_after_skips_earlier_ids -- --nocapture`

Expected: メソッド未定義でコンパイルエラー。

- [x] **Step 3: `photos_without_hash_after` を実装する**

```rust
pub fn photos_without_hash_after(&self, limit: i64, after_id: i64) -> Result<Vec<(i64, String)>> {
    let mut stmt = self.conn.prepare_cached(
        "SELECT id, abs_path FROM photos \
         WHERE status = 'present' AND blake3 IS NULL AND id > ?2 \
         ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![clamp_limit(limit), after_id], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
```

既存の `photos_without_hash` は `photos_without_hash_after(limit, 0)` を呼ぶ互換ラッパーにする。

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core photos_without_hash_after_skips_earlier_ids -- --nocapture`

Expected: PASS。

- [x] **Step 5: 先頭詰まりの回帰テストを書く**

`crates/daemon/src/tests.rs`:

```rust
#[test]
fn hash_worker_passes_missing_rows_and_hashes_later_photos() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    let valid_path = dir.path().join("photos/valid.bin");
    std::fs::write(&valid_path, b"hash me").unwrap();

    {
        let db = daemon.db.lock().unwrap();
        let root = db.add_root(dir.path().join("photos").to_str().unwrap(), "reference").unwrap();
        for index in 0..16 {
            db.upsert_photo(&NewPhoto {
                root_id: root,
                rel_path: format!("missing-{index}.bin"),
                abs_path: dir.path().join(format!("photos/missing-{index}.bin")).to_string_lossy().to_string(),
                file_name: format!("missing-{index}.bin"),
                captured_at_utc: index,
                captured_at_local: "2026-10-21 21:24:00.000".to_string(),
                tz_offset_sec: 9 * 3600,
                width: 1,
                height: 1,
                file_size: 1,
                mtime: index,
                blake3: None,
            }).unwrap();
        }
        db.upsert_photo(&NewPhoto {
            root_id: root,
            rel_path: "valid.bin".to_string(),
            abs_path: valid_path.to_string_lossy().to_string(),
            file_name: "valid.bin".to_string(),
            captured_at_utc: 100,
            captured_at_local: "2026-10-21 21:24:00.000".to_string(),
            tz_offset_sec: 9 * 3600,
            width: 1,
            height: 1,
            file_size: 7,
            mtime: 100,
            blake3: None,
        }).unwrap();
    }

    daemon.start_workers();
    wait_for(
        || daemon.db.lock().unwrap().photo_by_path(valid_path.to_str().unwrap()).unwrap().unwrap().blake3.is_some(),
        Duration::from_secs(2),
    );
    daemon.stop_workers();
}
```

補足: `wait_for` は既存のテストヘルパー。`NewPhoto` のフィールドは既存の `seed_photo_with_link_backoff` に合わせる。元の実装では最初の16件が失敗し続けるため、このテストは修正前ならタイムアウトする。

- [x] **Step 6: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon hash_worker_passes_missing_rows_and_hashes_later_photos -- --nocapture`

Expected: FAIL（2秒以内に `valid.bin` がハッシュされない）。

- [x] **Step 7: `hash_loop` をシグナル＋カーソル方式へ変更する**

`hash_loop` のシグネチャに `wake: Arc<JobSignal>` を追加する。ループの骨子:

```rust
let mut seen_generation = 0;
let mut after_id = 0;
let mut progressed_since_reset = false;

while running.load(Ordering::Relaxed) {
    if paused.load(Ordering::Relaxed) {
        wake.wait(&mut seen_generation, HASH_IDLE_WAIT);
        continue;
    }

    let rows = { /* db.photos_without_hash_after(16, after_id) */ };
    if rows.is_empty() {
        if after_id != 0 {
            after_id = 0;
            if !progressed_since_reset {
                wake.wait(&mut seen_generation, HASH_IDLE_WAIT);
            }
            progressed_since_reset = false;
            continue;
        }
        if hashed_since_idle { revision_pending = true; hashed_since_idle = false; }
        if revision_pending { /* revision bump */ }
        wake.wait(&mut seen_generation, HASH_IDLE_WAIT);
        continue;
    }

    let mut progressed = false;
    for (photo_id, path) in rows {
        after_id = photo_id;
        if !running.load(Ordering::Relaxed) { break; }
        match hashing::file_blake3(Path::new(&path)) {
            Ok(Some(hash)) => { /* set_photo_blake3; progressed = true; hashed_since_idle = true */ }
            Ok(None) => tracing::debug!(photo_id, path, "photo disappeared before hashing"),
            Err(error) => tracing::warn!(%error, photo_id, path, "cannot hash photo"),
        }
    }
    progressed_since_reset |= progressed;
    if progressed {
        thread::sleep(Duration::from_millis(200));
    }
}
```

注意:
- `rows.is_empty()` で `after_id != 0` のときは一度リセットして再走査する。失敗行しかない場合は `progressed_since_reset == false` なので、リセット後に60秒待つ（タイトループにしない）。
- 成功が1件でもあれば `progressed_since_reset = true` にして、リセット後すぐに再走査する。
- 失敗行は次の60秒フォールバックで再試行される。
- 既存の `hashed_since_idle` と `revision_pending` の扱いは引き継ぐ。DB 読み取りエラー時は従来どおり `Vec::new()` として扱い、待機へ落ちる。

- [x] **Step 8: 通知経路を追加する**

`Daemon` に `hash_wake: Arc<JobSignal>` を追加し、`open_inner` で `Arc::new(JobSignal::default())`。`start_workers` の `hash_loop` 呼び出しに `self.hash_wake.clone()` を渡す。`notify_workers` と `stop_workers` に `self.hash_wake.notify()` を追加する。`stop_workers` へ入れ忘れると、シャットダウンが最大60秒待つ。

さらに `IpcControl`（`crates/daemon/src/ipc_server.rs`）に `hash: Arc<JobSignal>` を追加し、`start_ipc` で `self.hash_wake.clone()` を渡す。`IpcControl::set_paused` で `self.hash.notify()` を呼ぶ。これがないと、UI からの resume が hash ワーカーに届かず、最大60秒遅れる。`ipc_server.rs` のテスト用 `IpcControl` リテラルにも `hash` を追加する。

`scan.rs` の次のメソッドの末尾で `self.hash_wake.notify()` を呼ぶ:
- `index_dirty_paths`
- `scan_photos_with_initial`
- `scan_root_photos`

- [x] **Step 9: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon hash_worker_passes_missing_rows_and_hashes_later_photos -- --nocapture`

Run: `cargo test -p vrc-photon-core photos_without_hash_after_skips_earlier_ids -- --nocapture`

Expected: PASS。

- [x] **Step 10: コミット**

```bash
git add crates/core/src/db/photos.rs crates/core/src/db/tests.rs crates/daemon/src/hash.rs crates/daemon/src/lifecycle.rs crates/daemon/src/scan.rs crates/daemon/src/lib.rs crates/daemon/src/tests.rs
git commit -m "fix(daemon): wake the hash worker and skip unreadable rows"
```

---

## Task 4: ステータス更新の session / counts 分割と変更時 publish

**Files:**
- Modify: `crates/daemon/src/status.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Modify: `crates/daemon/src/logs.rs`
- Modify: `crates/daemon/src/scan.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/daemon/src/tests.rs`

**Interfaces:**
- Consumes: 既存の `session_at`、`world_by_id`、`players_at`、`count_photos`、`count_jobs`、`count_pending_sends`、`tagging_status`、`status_snapshot`。
- Produces:
  - `Daemon::refresh_session_status(&self) -> Result<()>`
  - `Daemon::refresh_counts(&self) -> Result<()>`
  - `Daemon::refresh_status(&self) -> Result<()>`（両方を呼ぶ互換メソッド）
  - `Daemon::publish_status_if_changed(&mut self)`
  - `Daemon::invalidate_counts(&mut self)`
  - `const SESSION_STATUS_INTERVAL: Duration = Duration::from_secs(2);`
  - `const COUNTS_STATUS_INTERVAL: Duration = Duration::from_secs(30);`
  - `Daemon.last_session_status: Instant`
  - `Daemon.last_counts_status: Instant`
  - `Daemon.last_published_status: Option<StatusSnapshot>`（`vrc_photon_ipc::StatusSnapshot` を import する）

- [x] **Step 1: 分割と publish の失敗テストを書く**

`crates/daemon/src/tests.rs`:

```rust
#[test]
fn session_refresh_does_not_touch_counts() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = test_daemon(&dir);
    {
        let mut status = daemon.status.lock().unwrap();
        status.photo_count = 123;
        status.pending_jobs = 4;
        status.pending_sends = 5;
        status.pending_tagging_jobs = 6;
    }
    daemon.refresh_session_status().unwrap();
    let status = daemon.status.lock().unwrap();
    assert_eq!(status.photo_count, 123);
    assert_eq!(status.pending_jobs, 4);
    assert_eq!(status.pending_sends, 5);
    assert_eq!(status.pending_tagging_jobs, 6);
}

#[test]
fn counts_refresh_does_not_touch_session_fields() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = test_daemon(&dir);
    {
        let mut status = daemon.status.lock().unwrap();
        status.world_name = Some("keep".to_string());
        status.players = 7;
    }
    daemon.refresh_counts().unwrap();
    let status = daemon.status.lock().unwrap();
    assert_eq!(status.world_name.as_deref(), Some("keep"));
    assert_eq!(status.players, 7);
}

#[test]
fn status_publish_is_skipped_when_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let events = daemon.events.subscribe();
    daemon.publish_status_if_changed();
    daemon.publish_status_if_changed();
    assert_eq!(events.try_iter().filter(|event| matches!(event, Event::StatusChanged { .. })).count(), 1);
}

#[test]
fn status_publish_fires_when_a_field_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let events = daemon.events.subscribe();
    daemon.publish_status_if_changed();
    daemon.set_paused(true);
    while events.try_recv().is_ok() {}
    daemon.publish_status_if_changed();
    assert!(events.try_iter().any(|event| matches!(event, Event::StatusChanged { .. })));
}
```

`set_paused(true)` は既に publish するため、テストではいったんキューを空にしてから `publish_status_if_changed` の挙動を確認する。`last_published_status` の初期値が `None` なので最初の呼び出しは publish される。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon session_refresh_does_not_touch_counts counts_refresh_does_not_touch_session_fields status_publish_is_skipped_when_nothing_changed -- --nocapture`

Expected: メソッド未定義でコンパイルエラー。

- [x] **Step 3: `status.rs` を分割する**

`refresh_session_status`:
- DB ロックを取り、`session_at(now)`、`world_by_id`、`players_at` を読む。DB ロックを解放してから status ロックを取る（現在のロック順を維持）。
- 更新するフィールド: `watching`、`world_name`、`players`、`last_log`。

`refresh_counts`:
- DB ロックを取り、`count_photos`、`count_jobs(Pending)`、`count_pending_sends` を読む。DB ロックを解放。
- `tagging_status(&self.db, &self.plugin)` を呼ぶ。
- status ロックを取り、`photo_count`、`pending_jobs`、`pending_sends`、`pending_tagging_jobs`、`tagging_ready` を更新する。

`refresh_status` は `self.refresh_session_status()?; self.refresh_counts()`。

`invalidate_counts`:
```rust
pub(crate) fn invalidate_counts(&mut self) {
    self.last_counts_status = Instant::now() - COUNTS_STATUS_INTERVAL;
}
```

`publish_status_if_changed`:
```rust
pub(crate) fn publish_status_if_changed(&mut self) {
    let snapshot = {
        let mut status = lock_status(&self.status);
        status_snapshot(&mut status)
    };
    let changed = self
        .last_published_status
        .as_ref()
        .is_none_or(|previous| !same_status_fields(previous, &snapshot));
    if changed {
        self.last_published_status = Some(snapshot.clone());
        self.events.publish(Event::StatusChanged { status: snapshot });
    }
}
```
`StatusSnapshot` は `PartialEq` だが `version` が毎回増えるため、比較用のヘルパーを `status.rs` に置く:
```rust
fn same_status_fields(a: &StatusSnapshot, b: &StatusSnapshot) -> bool {
    a.paused == b.paused
        && a.watching == b.watching
        && a.scanning == b.scanning
        && a.scan_phase == b.scan_phase
        && a.scan_processed == b.scan_processed
        && a.scan_total == b.scan_total
        && a.scan_indexed == b.scan_indexed
        && a.scan_root == b.scan_root
        && a.scan_roots == b.scan_roots
        && a.photo_count == b.photo_count
        && a.pending_jobs == b.pending_jobs
        && a.pending_sends == b.pending_sends
        && a.pending_tagging_jobs == b.pending_tagging_jobs
        && a.tagging_ready == b.tagging_ready
}
```
`publish_status_if_changed` はこのヘルパーで比較する。

- [x] **Step 4: メインループとイベント経路を更新する**

`run_loop_iteration`:
```rust
if self.last_session_status.elapsed() >= SESSION_STATUS_INTERVAL {
    if let Err(error) = self.refresh_session_status() { /* warn */ }
    self.last_session_status = Instant::now();
    self.publish_status_if_changed();
}
if self.last_counts_status.elapsed() >= COUNTS_STATUS_INTERVAL {
    if let Err(error) = self.refresh_counts() { /* warn */ }
    self.last_counts_status = Instant::now();
    self.publish_status_if_changed();
}
```

`index_dirty_paths` が成功したら `self.invalidate_counts()` を呼ぶ。`scan_photos_with_initial` / `scan_root_photos` の末尾でも `self.invalidate_counts()` を呼ぶ。これにより、写真枚数が変わった直後のメインループで counts が更新される（既存テスト `paused_loop_collects_watcher_events_and_indexes_them_after_resume` の `photo_count == 1` を維持する）。

`ingest_log_pass` の末尾は `self.refresh_status()` ではなく `self.refresh_session_status()?; self.publish_status_if_changed();` にする。

`observe_vrchat_presence` の close 後も `refresh_session_status()?; publish_status_if_changed();` にする。

`poll_commands_with_initial` と `run_once` は `refresh_status()` のまま（両方更新）。

初期化: `last_session_status = Instant::now() - SESSION_STATUS_INTERVAL`、`last_counts_status = Instant::now() - COUNTS_STATUS_INTERVAL`、`last_published_status = None`。

- [x] **Step 5: counts の周期テストを追加する**

```rust
#[test]
fn loop_refreshes_counts_only_when_due() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    daemon.last_counts_status = Instant::now();
    {
        let mut status = daemon.status.lock().unwrap();
        status.photo_count = 42;
    }
    daemon.run_loop_iteration();
    assert_eq!(daemon.status.lock().unwrap().photo_count, 42);

    daemon.last_counts_status = Instant::now() - COUNTS_STATUS_INTERVAL;
    daemon.run_loop_iteration();
    let expected = daemon.db.lock().unwrap().count_photos().unwrap();
    assert_eq!(daemon.status.lock().unwrap().photo_count, expected);
}
```

- [x] **Step 6: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon session_refresh_does_not_touch_counts counts_refresh_does_not_touch_session_fields status_publish_is_skipped_when_nothing_changed loop_refreshes_counts_only_when_due -- --nocapture`

Expected: PASS。

- [x] **Step 7: 既存テストを確認する**

Run: `cargo test -p vrc-photon-daemon`

Expected: PASS。失敗するテストがあれば、`refresh_status()` を明示的に呼ぶか、`invalidate_counts()` を追加して直す。テスト専用の短い周期は導入しない。

- [x] **Step 8: コミット**

```bash
git add crates/daemon/src/status.rs crates/daemon/src/lifecycle.rs crates/daemon/src/logs.rs crates/daemon/src/scan.rs crates/daemon/src/lib.rs crates/daemon/src/tests.rs
git commit -m "fix(daemon): split status refresh cadence and publish changes"
```

---

## Task 5: 空のコマンドキューと設定再読込の高速化

**Files:**
- Modify: `crates/core/src/db/daemon.rs`
- Modify: `crates/core/src/db/settings.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Test: `crates/core/src/db/tests.rs`、`crates/daemon/src/tests.rs`

**Interfaces:**
- Produces:
  - `Db::has_daemon_commands(&self) -> Result<bool>`
  - `Db::data_version(&self) -> Result<i64>`
  - `Daemon.last_data_version: Option<i64>`
  - `SETTINGS_INTERVAL` を `Duration::from_secs(1)` にする（`data_version` ゲートで JSON パースは変更時だけ。1秒＋ループ待機でも #188 の約2.5秒以内）

- [x] **Step 1: `has_daemon_commands` の失敗テストを書く**

`crates/core/src/db/tests.rs`:

```rust
#[test]
fn has_daemon_commands_reports_only_queued_rows() {
    // 空なら false
    assert!(!db.has_daemon_commands().unwrap());
    // request_daemon_command 後は true、take 後は false
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core has_daemon_commands_reports_only_queued_rows -- --nocapture`

Expected: メソッド未定義でコンパイルエラー。

- [x] **Step 3: `has_daemon_commands` を実装する**

```rust
pub fn has_daemon_commands(&self) -> Result<bool> {
    let exists: bool = self.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM daemon_commands)",
        [],
        |row| row.get(0),
    )?;
    Ok(exists)
}
```

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core has_daemon_commands_reports_only_queued_rows -- --nocapture`

Expected: PASS。

- [x] **Step 5: `data_version` の失敗テストを書く**

`crates/core/src/db/tests.rs`:

```rust
#[test]
fn data_version_changes_after_another_connection_commits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let reader = Db::open(&path).unwrap();
    let writer = Db::open(&path).unwrap();

    let before = reader.data_version().unwrap();
    writer.set_setting("version_probe", "1").unwrap();
    assert!(reader.data_version().unwrap() > before);
}
```

- [x] **Step 6: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core data_version_changes_after_another_connection_commits -- --nocapture`

Expected: `data_version` 未定義でコンパイルエラー。

- [x] **Step 7: `Db::data_version` を実装する**

`crates/core/src/db/settings.rs` に追加:

```rust
pub fn data_version(&self) -> Result<i64> {
    let version = self.conn.query_row("PRAGMA data_version", [], |row| row.get(0))?;
    Ok(version)
}
```

- [x] **Step 8: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core data_version_changes_after_another_connection_commits -- --nocapture`

Expected: PASS。

- [x] **Step 9: `poll_commands_with_initial` に高速パスを追加する**

```rust
pub(crate) fn poll_commands_with_initial(&mut self, initial_scan: Option<bool>) -> Result<()> {
    let mut commands = std::mem::take(&mut self.deferred_commands);
    {
        let db = lock_db(&self.db);
        if commands.is_empty() && !db.has_daemon_commands()? {
            return Ok(());
        }
        commands.extend(db.take_daemon_commands()?);
    }
    // 既存の処理
}
```

- [x] **Step 10: 設定再読込を `data_version` でゲートする**

`Daemon` に `last_data_version: Option<i64>` を追加し、`open_inner` で `None` に初期化する。

`refresh_periodic_settings` を次の形にする:

```rust
if self.last_settings_check.elapsed() >= SETTINGS_INTERVAL {
    let version = {
        let db = lock_db(&self.db);
        db.data_version().ok()
    };
    let changed = version.is_none() || version != self.last_data_version;
    self.last_data_version = version;
    self.last_settings_check = Instant::now();
    if changed {
        self.refresh_compression_settings();
    }
}
```

- `version` が `None`（読み取り失敗）のときは `changed == true` になり、従来どおり再読込する。
- `refresh_autostart_setting` の10秒周期は変えない。
- デーモン自身が `settings.save` した場合は `self.settings` を直接更新しているため、`data_version` が変わらなくても問題ない。

- [x] **Step 11: コマンドの回帰テストを追加する**

`crates/daemon/src/tests.rs`:

```rust
#[test]
fn command_inserted_without_wake_is_still_processed() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    write_photo(&dir.path().join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png"));

    {
        let db = Db::open(&dir.path().join("library.db")).unwrap();
        db.request_daemon_command(DaemonCommand::Rescan).unwrap();
    }

    daemon.poll_commands().unwrap();
    assert_eq!(daemon.db.lock().unwrap().count_photos().unwrap(), 1);
}
```

- [x] **Step 12: 設定再読込のゲートテストを追加する**

```rust
#[test]
fn settings_reload_is_skipped_when_data_version_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    let version = { let db = daemon.db.lock().unwrap(); db.data_version().unwrap() };
    daemon.last_data_version = Some(version);
    daemon.thumbnail_cache_limit.store(999, Ordering::Relaxed);
    daemon.last_settings_check = Instant::now() - SETTINGS_INTERVAL;
    daemon.refresh_periodic_settings();
    assert_eq!(daemon.thumbnail_cache_limit.load(Ordering::Relaxed), 999);
}

#[test]
fn settings_reload_applies_an_external_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    let db_path = dir.path().join("library.db");
    let before = {
        let db = daemon.db.lock().unwrap();
        db.data_version().unwrap()
    };
    daemon.last_data_version = Some(before);

    let mut settings = AppSettings::load(&daemon.db.lock().unwrap()).unwrap();
    settings.thumbnail_cache_mb = 1;
    {
        let other = Db::open(&db_path).unwrap();
        settings.save(&other).unwrap();
    }

    daemon.last_settings_check = Instant::now() - SETTINGS_INTERVAL;
    daemon.refresh_periodic_settings();
    assert_eq!(daemon.thumbnail_cache_limit.load(Ordering::Relaxed), 1024 * 1024);
}
```

- [x] **Step 13: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon command_inserted_without_wake_is_still_processed settings_reload_is_skipped_when_data_version_is_unchanged settings_reload_applies_an_external_change -- --nocapture`

Run: `cargo test -p vrc-photon-core has_daemon_commands_reports_only_queued_rows data_version_changes_after_another_connection_commits -- --nocapture`

Expected: PASS。

- [x] **Step 14: コミット**

```bash
git add crates/core/src/db/daemon.rs crates/core/src/db/settings.rs crates/core/src/db/tests.rs crates/daemon/src/lifecycle.rs crates/daemon/src/tests.rs
git commit -m "fix(daemon): avoid idle command transactions and settings reads"
```

---

## Task 6: ワーカー空転待機の統一

**Files:**
- Modify: `crates/daemon/src/compression.rs`
- Modify: `crates/daemon/src/tagging.rs`
- Modify: `crates/daemon/src/discord.rs`
- Modify: `crates/daemon/src/deletion.rs`
- Modify: `crates/daemon/src/thumbnails.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/daemon/src/tests.rs`

**Interfaces:**
- Produces:
  - `const COMPRESSION_IDLE_WAIT: Duration = Duration::from_secs(30);`
  - `const TAGGING_IDLE_WAIT: Duration = Duration::from_secs(30);`
  - `const DISCORD_IDLE_WAIT: Duration = Duration::from_secs(30);`
  - `const DELETION_IDLE_WAIT: Duration = Duration::from_secs(5);`
  - `const THUMBNAIL_IDLE_WAIT: Duration = Duration::from_secs(2);`
  - `#[derive(Debug, PartialEq, Eq)] pub(crate) enum ThumbnailRequest { Photo(i64), Refresh }`
  - `Daemon.thumbnail_tx: Option<mpsc::Sender<ThumbnailRequest>>`

**待機値の理由:**
- compression / tagging / discord は enqueue 時に必ず signal が飛ぶため、30秒はフォールバック。
- deletion は compression / tagging / send の完了で解除されることがあり、通知漏れの影響を抑えるため5秒。
- thumbnail は外部から書き込まれたサムネイルの通知を IPC 経由で受けるため、2秒で十分な応答性を確保する（500msから4倍に削減）。設定変更は `Refresh` で即時反映する。

- [x] **Step 1: thumbnail チャネルの失敗テストを書く**

`crates/daemon/src/tests.rs` の `scan_root_sends_new_photos_to_the_thumbnail_queue` を新しい型に合わせ、`Refresh` のテストを追加する:

```rust
#[test]
fn thumbnail_settings_change_wakes_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.thumbnail_tx = Some(tx);
    daemon.thumbnail_cache_limit.store(1, Ordering::Relaxed);
    daemon.refresh_compression_settings();
    assert!(matches!(rx.try_recv().unwrap(), ThumbnailRequest::Refresh));
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon thumbnail_settings_change_wakes_the_worker -- --nocapture`

Expected: `ThumbnailRequest` 未定義、または `Refresh` が送られず FAIL。

- [x] **Step 3: `ThumbnailRequest` と `Refresh` 送信を実装する**

`thumbnails.rs`:
```rust
pub(crate) enum ThumbnailRequest {
    Photo(i64),
    Refresh,
}
```
`thumbnail_loop` の `rx: &Receiver<i64>` を `&Receiver<ThumbnailRequest>` に変更し、`match` を:
```rust
Ok(ThumbnailRequest::Photo(id)) => { /* 既存の生成処理 */ }
Ok(ThumbnailRequest::Refresh) => {
    // ループ先頭の epoch / limit チェックに任せる
}
Err(mpsc::RecvTimeoutError::Timeout) => { /* 既存の prune 処理 */ }
```
`recv_timeout(THUMBNAIL_PRUNE_IDLE)` を `recv_timeout(THUMBNAIL_IDLE_WAIT)` に変更する。

`lifecycle.rs`:
- `thumbnail_tx: Option<mpsc::Sender<ThumbnailRequest>>` に変更。
- `start_workers`、`scan.rs` の `tx.send(...)` を `ThumbnailRequest::Photo(id)` に変更。
- 設定変更後（`reload_settings` の thumbnail 更新、`refresh_compression_settings` の thumbnail 更新）に `self.notify_thumbnail_worker()` を呼ぶ:
```rust
fn notify_thumbnail_worker(&self) {
    if let Some(tx) = &self.thumbnail_tx {
        let _ = tx.send(ThumbnailRequest::Refresh);
    }
}
```
- `Refresh` は上限・spec・epoch のいずれかが実際に変わったときだけ送る。`refresh_compression_settings` はデータ更新のたびに走り得るため、毎回送るとチャネルに溜まる。`thumbnail_size_changed` と同じように、変更前の値を比較してから送る:
```rust
let old_limit = self.thumbnail_cache_limit.load(Ordering::Relaxed);
let old_spec = self.thumbnail_spec.load(Ordering::Relaxed);
let new_limit = thumbnail_cache_limit_bytes(settings.thumbnail_cache_mb);
let new_spec = pack_thumbnail_spec(settings.thumbnail_spec());
self.thumbnail_cache_limit.store(new_limit, Ordering::Relaxed);
self.thumbnail_spec.store(new_spec, Ordering::Relaxed);
if old_limit != new_limit || old_spec != new_spec || thumbnail_size_changed {
    self.notify_thumbnail_worker();
}
```
`reload_settings` も同じ比較で送る。
- テストの `thumbnail_tx.send(999_999)` を `ThumbnailRequest::Photo(999_999)` に変更する。

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon thumbnail_settings_change_wakes_the_worker -- --nocapture`

Expected: PASS。

- [x] **Step 5: 各ワーカーの待機値を変更する**

- `compression.rs`: `Duration::from_millis(400)` を `COMPRESSION_IDLE_WAIT` に。pause 中の `PAUSED_WORKER_POLL` も `COMPRESSION_IDLE_WAIT` に。
- `tagging.rs`: `TaggingWorkerConfig::default().poll_interval` を `TAGGING_IDLE_WAIT` に。pause 中も同じ。
- `discord.rs`: `POLL_INTERVAL` を `DISCORD_IDLE_WAIT` に。
- `deletion.rs`: `Duration::from_millis(500)` と pause 中の待機を `DELETION_IDLE_WAIT` に。
- `lib.rs`: `PAUSED_WORKER_POLL` と `THUMBNAIL_PRUNE_IDLE` が未使用になるなら削除する。
- `notify_workers` に `self.hash_wake.notify()` と `self.notify_thumbnail_worker()` を追加する（Task 3 で hash は追加済み）。

- [x] **Step 6: pause / resume の通知テストを追加する**

```rust
#[test]
fn notify_workers_wakes_hash_and_thumbnail() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.thumbnail_tx = Some(tx);
    let before = daemon.hash_wake.generation();
    daemon.notify_workers();
    assert!(daemon.hash_wake.generation() > before);
    assert!(matches!(rx.try_recv().unwrap(), ThumbnailRequest::Refresh));
}
```

- [x] **Step 7: 既存テストを修正する**

Run: `cargo test -p vrc-photon-daemon`

Expected: PASS。失敗したテストは、ワーカーが直接 DB を読むことを前提にしている。次のいずれかで直す:
- テスト内で `daemon.jobs.notify()` / `daemon.sends.notify()` / `daemon.tagging.notify()` / `daemon.hash_wake.notify()` を呼ぶ。
- ワーカーの1ステップ関数（例: `run_deletions_once`）を直接呼ぶ。
- `wait_for` のタイムアウトを延ばすのではなく、通知を追加する。

- [x] **Step 8: コミット**

```bash
git add crates/daemon/src/compression.rs crates/daemon/src/tagging.rs crates/daemon/src/discord.rs crates/daemon/src/deletion.rs crates/daemon/src/thumbnails.rs crates/daemon/src/lifecycle.rs crates/daemon/src/lib.rs crates/daemon/src/scan.rs crates/daemon/src/tests.rs
git commit -m "fix(daemon): unify worker idle waits"
```

---

## Task 7: メインループの期限ベース起床と watcher wake

**Files:**
- Modify: `crates/daemon/src/lifecycle.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/daemon/src/tests.rs`

**Interfaces:**
- Produces:
  - `pub(crate) fn min_wait(waits: &[Duration], cap: Duration) -> Duration`（純粋関数）
  - `Daemon::next_loop_wait(&self) -> Duration`
  - `const MAIN_LOOP_MAX_WAIT: Duration = Duration::from_millis(500);`
  - `const MAIN_LOOP_HEADLESS_WAIT: Duration = Duration::from_secs(1);`

- [x] **Step 1: `min_wait` の失敗テストを書く**

```rust
#[test]
fn min_wait_uses_the_earliest_deadline_and_caps_at_the_maximum() {
    assert_eq!(crate::lifecycle::min_wait(&[Duration::from_secs(5), Duration::from_secs(2)], Duration::from_secs(1)), Duration::from_secs(1));
    assert_eq!(crate::lifecycle::min_wait(&[Duration::from_millis(200), Duration::from_secs(5)], Duration::from_secs(1)), Duration::from_millis(200));
    assert_eq!(crate::lifecycle::min_wait(&[], Duration::from_secs(1)), Duration::from_secs(1));
    assert_eq!(crate::lifecycle::min_wait(&[Duration::ZERO], Duration::from_secs(1)), Duration::from_millis(10));
}
```

`min_wait` は最小値を `Duration::from_millis(10)` で下限クリップする。

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon min_wait_uses_the_earliest_deadline_and_caps_at_the_maximum -- --nocapture`

Expected: `min_wait` 未定義でコンパイルエラー。

- [x] **Step 3: `min_wait` と `next_loop_wait` を実装する**

```rust
pub(crate) fn min_wait(waits: &[Duration], cap: Duration) -> Duration {
    waits
        .iter()
        .copied()
        .min()
        .unwrap_or(cap)
        .min(cap)
        .max(Duration::from_millis(10))
}

impl Daemon {
    pub(crate) fn next_loop_wait(&self) -> Duration {
        let now = Instant::now();
        // pause 中は asset check を実行しない。期限切れのまま含めると
        // 毎回 10ms で起床するビジーループになる。
        let asset_deadline = if self.is_paused() {
            now + Duration::from_secs(30)
        } else {
            self.last_asset_check + Duration::from_secs(30)
        };
        let waits = [
            self.last_log_poll + LOG_INTERVAL,
            self.last_presence_check + PRESENCE_INTERVAL,
            self.last_session_status + SESSION_STATUS_INTERVAL,
            self.last_counts_status + COUNTS_STATUS_INTERVAL,
            self.last_settings_check + SETTINGS_INTERVAL,
            self.last_autostart_check + Duration::from_secs(10),
            self.last_heartbeat + Duration::from_secs(10),
            asset_deadline,
        ]
        .map(|deadline| deadline.saturating_duration_since(now));

        let cap = if self.tray_enabled {
            MAIN_LOOP_MAX_WAIT
        } else {
            MAIN_LOOP_HEADLESS_WAIT
        };
        min_wait(&waits, cap)
    }
}
```

`SCAN_INTERVAL` を `MAIN_LOOP_MAX_WAIT` に置き換える。`run` の待機:
```rust
if self.running.load(Ordering::Relaxed) {
    let wait = self.next_loop_wait();
    self.wake.wait(wait);
}
```

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon min_wait_uses_the_earliest_deadline_and_caps_at_the_maximum -- --nocapture`

Expected: PASS。

- [x] **Step 5: watcher からメインループを起こす**

`setup_watchers`:
```rust
let wake = self.wake.clone();
let mut watcher = notify::recommended_watcher(move |event| {
    if let Err(error) = tx.send(event) {
        tracing::debug!(%error, "photo watcher receiver closed; dropping event");
    }
    wake.notify();
})?;
```
これにより、写真イベントの索引付けが最大待機時間まで遅れない。

- [x] **Step 6: 待機時間の境界テストを追加する**

```rust
#[test]
fn headless_idle_wait_is_at_most_one_second() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    daemon.last_log_poll = Instant::now() - LOG_INTERVAL;
    // すべての期限を現在にすると下限10ms、未来にすると cap に張り付く
    assert!(daemon.next_loop_wait() <= Duration::from_secs(1));
}

#[test]
fn tray_idle_wait_is_at_most_five_hundred_milliseconds() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(true);
    assert!(daemon.next_loop_wait() <= Duration::from_millis(500));
}

#[test]
fn paused_idle_wait_does_not_spin_on_an_overdue_asset_check() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    let now = Instant::now();
    daemon.last_log_poll = now;
    daemon.last_presence_check = now;
    daemon.last_session_status = now;
    daemon.last_counts_status = now;
    daemon.last_settings_check = now;
    daemon.last_autostart_check = now;
    daemon.last_heartbeat = now;
    daemon.last_asset_check = now - Duration::from_secs(60);
    daemon.set_paused(true);
    assert!(daemon.next_loop_wait() >= Duration::from_millis(100));
}
```

`last_*` を未来にする方法がないため、初期化直後の期限で境界だけを確認する。厳密な期限一致は `min_wait` の単体テストで担保する。

- [x] **Step 7: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon min_wait_uses_the_earliest_deadline_and_caps_at_the_maximum headless_idle_wait_is_at_most_one_second tray_idle_wait_is_at_most_five_hundred_milliseconds -- --nocapture`

Expected: PASS。

- [x] **Step 8: コミット**

```bash
git add crates/daemon/src/lifecycle.rs crates/daemon/src/lib.rs crates/daemon/src/tests.rs
git commit -m "fix(daemon): wake the main loop from watcher events and deadlines"
```

---

## Task 8: ドキュメント更新（docs-only PR）

**Files:**
- Modify: `docs/architecture.md`
- Modify: `docs/log-and-xmp.md`
- Modify: `docs/features.md`（ポーリング周期の記述がある場合）
- Modify: `docs/development.md`（検証手順がある場合）

**注意:** このタスクはコード PR とは別の docs-only PR にする（AGENTS.md §4）。

- [x] **Step 1: architecture.md を更新する**

次の内容を追記・修正する:
- メインループは期限ベースで起床し、アイドル時はトレイ有効500ms / ヘッドレス1秒を上限にする。
- VRChat プロセス検出は5秒間隔で、PIDキャッシュを使う。close は3回連続 miss。
- ステータスは session 2秒 / counts 30秒＋イベント時。
- ハッシュ・compression・tagging・discord はシグナル起床＋長いフォールバック、deletion は5秒、thumbnail は2秒＋Refresh。
- IPC の型とプロトコルバージョンは不変。

- [x] **Step 2: log-and-xmp.md を更新する**

「追記は750ms間隔のポーリングと notify を併用する」の後に追記:
- 最新ログファイルの特定はログディレクトリの mtime をゲートにし、変更時と60秒フォールバック時だけ `read_dir` する。
- 追記の読み取りは750msのまま。

- [x] **Step 3: features.md / development.md を確認する**

Run: `grep -n "750\|500ms\|400ms\|3秒ポーリング" docs/features.md docs/development.md docs/architecture.md`

期待: 変更対象の記述をすべて洗い出す。UI の3秒ポーリングは今回のスコープ外なので変更しない。

- [x] **Step 4: ドキュメントの差分を確認する**

Run: `git diff -- docs/architecture.md docs/log-and-xmp.md docs/features.md docs/development.md`

Expected: 実装と矛盾する記述がない。

- [x] **Step 5: コミット（docs-only PR）**

```bash
git add docs/architecture.md docs/log-and-xmp.md docs/features.md docs/development.md
git commit -m "docs: document daemon idle polling cadence"
```

---

## 検証

### 自動テスト

各タスクの後に:

```bash
cargo test -p vrc-photon-daemon
cargo test -p vrc-photon-core
```

最後に:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### アイドル計測（手動）

1. 100k 写真・5,000 ログのテスト環境でデーモンを起動する（本番ライブラリでも可）。
2. 60秒間アイドルにする。
3. `strace -f -c -p $(pgrep -f vrc-photon-daemon)` で syscall 回数を記録する。ptrace が使えない場合は `/proc/<pid>/io` と `perf stat -p <pid> sleep 60` を使う。
4. 次の項目を確認する:
   - `/proc/[0-9]*/comm` の open が定常的に発生しない（起動直後と VRChat 終了時を除く）。
   - ログディレクトリの `getdents64` が60秒あたり1回以下。
   - `count_photos` 相当の `SCAN` が30秒あたり1回以下。
   - ハッシュの `SELECT` が60秒あたり1回以下。
   - `BEGIN IMMEDIATE` がアイドル時に発生しない（コマンド投入時を除く）。
5. アイドル CPU がベースラインより低下していることを確認する。絶対値ではなく、同じ環境の before/after で比較する。

### 挙動確認

- VRChat を終了 → 最大15秒でセッションが close され、終了時刻がログ mtime と一致する。
- 新しいログファイルを作成 → 750ms以内に検出される。
- pause → resume で compression / tagging / deletion / discord / hash / thumbnail が即座に再開する。
- サムネイル上限を設定画面で変更 → 2秒以内に prune が反映される。
- 既存の UI 3秒ポーリング / IPC イベントが変化しない。

### 完了条件

- [x] `cargo fmt --all -- --check` が通る。
- [x] `cargo clippy --workspace --all-targets -- -D warnings` が通る。
- [x] `cargo test --workspace` が通る。
- [ ] 手動計測で上記の頻度を満たす。
- [x] `StatusSnapshot` のフィールドと `PROTOCOL_VERSION` が変わっていない。
- [x] セッション終了時刻が従来と同じロジックで計算されている。
- [x] docs-only PR が分離されている。

> 手動の strace 計測は未実施。頻度の根拠は自動テスト、コードレビュー、CI（Linux / Windows ビルド）で確認した。実機でのアイドル計測は別途行う。

## 実行結果

- 実装: PR #619 `fix/daemon-idle-polling`（merge commit `81c9192`）
- 自動検証: `cargo fmt`、workspace clippy、workspace tests、audit、IPC Windows build、Release bundle（Linux / Windows）がすべて成功
- ドキュメント: この docs-only PR

## 実行上の注意

- 各タスクは独立してコミットする。タスク間でテストが壊れた場合は、次のタスクへ進む前に直す。
- Task 6 は既存テストへの影響が最も大きい。`cargo test -p vrc-photon-daemon` をタスク内で必ず実行し、通知漏れをテスト専用の短縮で隠さない。
- Windows の `ProcessTable` は Linux ではコンパイルされない。`cargo check --target x86_64-pc-windows-gnu -p vrc-photon-daemon` が使える環境なら実行し、使えなければ Release ワークフローの Windows ビルドで確認する。
- 計画書を docs-only PR で `main` にマージしてから、実装用 worktree を切る。
- 実装開始前に worktree を作成する:

```bash
REPO=$(basename "$(git rev-parse --show-toplevel)")
git worktree add ../${REPO}-daemon-idle -b fix/daemon-idle-polling
```

その後、新しいセッションで `../vrc-photo-manager-daemon-idle` を開く。
