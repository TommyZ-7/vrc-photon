# UI レビュー対応 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Issue #757 / #758 / #759 / #760 / #767 / #768 のレビュー指摘を UI に反映する。

**Architecture:** レンダーパスからディスク I/O を除去し、起動時の systemctl と DB シードを
UI スレッドから外す。完了イベントは既存の coalesce パスへ寄せ、demo は cargo feature で
リリースから除外する。settings 書き込みは単一フライトのデバウンスに集約する。

**Tech Stack:** Rust (edition 2024), gpui-kit 0.9.1, SQLite

**Spec:** Issue #757, #758, #759, #760, #767, #768 (本文をこの計画の仕様とする)

## Global Constraints

- UI の見た目・文言を変えない（エラー終了時のメッセージを除く）。
- レンダー関数内で DB・ファイルシステム・プロセス起動を実行しない。
- 既存のエラー通知経路 (`toast_error` / `toast_warning`) を使う。
- `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` を通す。
- update 系 PR (#753) が `update_view.rs` を変更するため、このブランチは update PR のマージ後に rebase する。

## Review Focus

- 更新カードの再描画: ディスクプローブが 1 フレームも走らない。
- systemd が遅い環境: 最初のフレームが `systemctl` を待たない。
- DB が壊れたリリース環境: demo ライブラリへ黙って切り替わらず、原因を出して終了する。
- 1,000 件の圧縮完了バースト: UI スレッドの `photo_by_id` が 0 回、一括解決が 1 回。
- 複数ユーザーの demo: 互いの `demo_root` を共有しない。

---

### Task 1: 更新カードの書き込み可否をキャッシュする (#757)

**Files:**
- Modify: `crates/ui/src/update_view.rs` (`UpdateState` 定義 :671-750, レンダー :1508, :1580, アクション :948, :1079)
- Test: `crates/ui/src/update_view.rs` のテストモジュール

**Interfaces:**
- Produces: `fn cached_appimage_writable(install: &InstallKind, cached: Option<bool>) -> bool`
- Produces: `impl UpdateState { fn refresh_appimage_writable(&mut self); fn appimage_writable(&self) -> bool }`

- [ ] **Step 1: 純粋 helper の失敗テストを書く**

```rust
#[test]
fn cached_writability_is_only_used_for_appimages() {
    let appimage = appimage();
    assert!(cached_appimage_writable(&appimage, Some(true)));
    assert!(!cached_appimage_writable(&appimage, Some(false)));
    assert!(!cached_appimage_writable(&appimage, None));
    assert!(cached_appimage_writable(
        &InstallKind::Development,
        Some(false)
    ));
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p vrc-photon-ui cached_writability -- --nocapture`
Expected: `cached_appimage_writable` 未定義でコンパイル失敗。

- [ ] **Step 3: キャッシュを実装**

- `UpdateState` に `appimage_writable: Option<bool>` を追加。
- `refresh_appimage_writable` は AppImage のときだけ `appimage_writable(&self.install)` を保存。
- `UpdateState::new` の構築後に呼ぶ。
- `UpdatePhase::Available` / `Ready` へ遷移する箇所（`Checked`, `Downloaded`）で呼ぶ。
- `update_available_view` / `update_ready_view` は `self.update.appimage_writable()` を使う。
- `start_update_download` / `apply_update_confirmed` の即時プローブは残し、失敗時に
  `self.update.appimage_writable = Some(false)` を設定する。
- `cached_appimage_writable` は `install` が AppImage なら `cached.unwrap_or(false)`、それ以外は `true`。

- [ ] **Step 4: テストと clippy**

Run: `cargo test -p vrc-photon-ui cached_writability && cargo clippy -p vrc-photon-ui --all-targets -- -D warnings`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/update_view.rs
git commit -m "fix: cache AppImage writability instead of probing during render (#757)"
```

---

### Task 2: systemctl を background executor へ移す (#758)

**Files:**
- Modify: `crates/ui/src/app.rs:1480-1490`
- Modify: `crates/ui/src/update_view.rs:1210-1220`, `:1240-1250`
- Test: 既存の `should_use_systemd` テストを維持（I/O 自体は単体テストしない）

**Interfaces:**
- Consumes: `pub(crate) fn start_daemon(supervisor: Option<&str>) -> Result<(), String>`
- Consumes: `pub(crate) fn restart_daemon_after_update(supervisor: Option<&str>) -> Result<(), String>`

- [ ] **Step 1: App::new の同期呼び出しを置換**

```rust
if !app.demo && !app.daemon_active && !updating {
    let supervisor = supervisor.clone();
    cx.spawn(async move |this, cx| {
        let result = cx
            .background_executor()
            .spawn(async move { start_daemon(supervisor.as_deref()) })
            .await;
        if let Err(error) = result {
            this.update(cx, |this, cx| {
                this.toast_error(tr_args!("app.daemon_start_error", error = error), cx);
            })
            .ok();
        }
    })
    .detach();
}
```

- [ ] **Step 2: 更新適用失敗時の再起動を置換**

`apply_update_confirmed` の `if this.daemon_active { let _ = start_daemon(...) }` を、
`cx.background_executor().spawn` + `this.update` の非同期再起動へ置換する。

- [ ] **Step 3: 更新後再起動を置換**

`complete_update_startup` の `restart_daemon_after_update` を同じ形で `cx.spawn` に移す。
失敗時のトーストは既存の `update.error.daemon_start` を使う。

- [ ] **Step 4: 検証**

Run: `cargo test -p vrc-photon-ui daemon && cargo clippy -p vrc-photon-ui --all-targets -- -D warnings`
Expected: PASS。`rg "start_daemon\\(|restart_daemon_after_update\\(" crates/ui/src` で
UI スレッド直呼びが残っていないことを確認する。

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/app.rs crates/ui/src/update_view.rs
git commit -m "fix: run systemctl daemon startup off the UI thread (#758)"
```

---

### Task 3: 圧縮完了を一括解決する (#760)

**Files:**
- Modify: `crates/ui/src/app.rs` (`App` の `compression_replaced_photos` を置換)
- Modify: `crates/ui/src/app/lifecycle.rs:299-310`, `:447-500`
- Test: `crates/ui/src/app/lifecycle.rs` のテストモジュール

**Interfaces:**
- Produces: `fn replaced_photo_rows(photos: Vec<Photo>) -> Vec<Photo>`
- Consumes: `Db::photos_by_ids(&[i64]) -> Result<Vec<Photo>>`

- [ ] **Step 1: フィルタの失敗テストを書く**

```rust
#[test]
fn only_replaced_rows_are_refreshed_after_compression() {
    let rows = vec![
        photo_with_status(1, PhotoStatus::Keep),
        photo_with_status(2, PhotoStatus::Replaced),
    ];
    let kept = replaced_photo_rows(rows);
    assert_eq!(kept.iter().map(|photo| photo.id).collect::<Vec<_>>(), vec![2]);
}
```

`photo_with_status` はテスト内で最小の `Photo` を組み立てる helper。

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p vrc-photon-ui only_replaced_rows -- --nocapture`
Expected: 未定義でコンパイル失敗。

- [ ] **Step 3: 実装**

- `App` の `compression_replaced_photos` を `compression_replaced_candidates: HashSet<i64>` に置換。
- `CompressionProgress` の `completed` 分岐は `photo_by_id` を呼ばず、`photo_id` を候補集合へ入れる。
- `refresh_after_compression` は候補を drain し、空でなければ
  `self.db.photos_by_ids(&candidates)` を 1 回呼び、`replaced_photo_rows` で `is_replaced()` のみに絞る。
  その行だけ `update_photo_rows` とメディア無効化を行う。
- quick look の再要求判定は、絞った行の id を使う。

- [ ] **Step 4: テストを実行**

Run: `cargo test -p vrc-photon-ui only_replaced_rows && cargo test -p vrc-photon-ui`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/app.rs crates/ui/src/app/lifecycle.rs
git commit -m "fix: coalesce compression row lookups into one batch query (#760)"
```

---

### Task 4: demo を feature 化し、リリースのフォールバックを拒否する (#759, #767)

**Files:**
- Modify: `crates/ui/Cargo.toml` (`[features] demo = []`, unix `libc`)
- Modify: `crates/ui/src/app.rs:10-11`, `:734`, `:1480`
- Modify: `crates/ui/src/app/demo.rs`
- Modify: `crates/ui/src/main.rs:43-47`, `:67`
- Modify: `.github/workflows/ci.yml`（demo feature のテスト）
- Test: `crates/ui/src/app/demo.rs`

**Interfaces:**
- Produces: `pub(crate) fn open_database(options: &LaunchOptions) -> Result<(Db, bool, bool, Option<String>, Option<DemoAssets>), String>`
- Produces: `#[cfg(feature = "demo")] fn prepare_demo(reset: bool) -> Result<(Db, DemoAssets), String>`
- Produces: `#[cfg(not(feature = "demo"))] fn prepare_demo(_: bool) -> Result<(Db, DemoAssets), String>`（常に `Err`）
- Produces: `fn demo_root() -> PathBuf`（unix は UID 入り）

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[cfg(unix)]
#[test]
fn demo_root_is_scoped_to_the_user() {
    let root = demo_root();
    let name = root.file_name().unwrap().to_string_lossy();
    assert!(name.ends_with(&format!("-{}", unsafe { libc::getuid() })));
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p vrc-photon-ui demo_root_is_scoped -- --nocapture`
Expected: 現在の `vrc-photon-demo` に UID が無いため失敗。

- [ ] **Step 3: feature と startuperror を実装**

- `Cargo.toml` に `[features] demo = []`、`[target.'cfg(unix)'.dependencies] libc.workspace = true` を追加。
- `demo_fixture` と `seed_demo` / `demo_image` / `seed_demo_sessions` / `prepare_demo_at` を
  `#[cfg(feature = "demo")]` で囲む。`prepare_demo` の非 feature 版は
  `Err(tr!("demo.unavailable"))` を返す。
- `open_database` の戻り値を `Result<... , String>` に変更:
  - 明示 `--demo` で feature 無効 → `Err(tr!("demo.unavailable"))`
  - 通常 DB オープン失敗で feature 有効 → 従来の `prepare_demo(true)` フォールバック
  - 通常 DB オープン失敗で feature 無効 → `Err(tr_args!("demo.db_error", error = error))`
- `App::new` は `open_database(options).unwrap_or_else(|error| crate::exit_startup_failure("cannot open the library database", error))`。
- `exit_startup_failure` を `pub(crate)` にする。
- `demo_root` を `temp_dir().join(format!("vrc-photon-demo-{}", unsafe { libc::getuid() }))` に変更。
- i18n に `demo.unavailable` を ja/en 両方追加。
- `ci.yml` の test ステップに `cargo test -p vrc-photon-ui --features demo` を追加。

- [ ] **Step 4: テストを実行**

Run: `cargo test -p vrc-photon-ui demo_root_is_scoped && cargo test -p vrc-photon-ui --features demo`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/Cargo.toml crates/ui/src/app.rs crates/ui/src/app/demo.rs crates/ui/src/main.rs crates/ui/src/i18n .github/workflows/ci.yml Cargo.lock
git commit -m "fix: gate demo content and reject the release fallback (#759, #767)"
```

---

### Task 5: monitor 参照と settings 保存を整理する (#768-1, #768-2)

**Files:**
- Modify: `crates/ui/src/app/daemon.rs:320-346`
- Modify: `crates/ui/src/app/lifecycle.rs:202, 228, 332, 529, 789`
- Modify: `crates/ui/src/app.rs` (settings 保存 helper とフィールド)
- Modify: `crates/ui/src/ui/settings.rs`, `crates/ui/src/ui/setup.rs` の保存呼び出し
- Test: `crates/ui/src/app/lifecycle.rs` / `daemon.rs`

**Interfaces:**
- Produces: `impl App { pub(crate) fn schedule_settings_save(&mut self, cx: &mut Context<Self>); pub(crate) fn flush_settings_save(&mut self); }`
- Produces: `fn refresh_monitor_sync(&mut self)`（demo / インメモリ専用）

- [ ] **Step 1: monitor の同期参照を削除**

- `daemon::refresh_monitor` を `refresh_monitor_sync` に改名し、呼び出しを `refresh_live` (lifecycle.rs:529) のみに残す。
- Connected / Disconnected / StatusChanged / reload_aux_ui の呼び出しを削除する。
  monitor フィールドは `apply_live_snapshot` が背景スナップショットから更新する。
- コメントで「同期版は demo / インメモリのみ」と明記。

- [ ] **Step 2: settings 保存の失敗テストを書く**

デバウンスは時間依存のため、純粋 helper を切り出す:

```rust
#[test]
fn a_settings_save_batch_supersedes_earlier_values() {
    let first = AppSettings { language: "en".into(), ..Default::default() };
    let second = AppSettings { language: "ja".into(), ..Default::default() };
    let batch = SettingsSaveBatch::new(first).merge(second);
    assert_eq!(batch.settings().language, "ja");
    assert_eq!(batch.generation(), 2);
}
```

`SettingsSaveBatch` は `App` 内の小構造体でよい。

- [ ] **Step 3: デバウンスを実装**

- `App` に `settings_save: SettingsSaveBatch` 相当の状態を追加。
- `schedule_settings_save(cx)` は 350ms の timer を 1 回だけ spawn し、発火時に最新値だけ
  `settings.save(&db)` する。失敗は `tracing::warn` と既存のトーストで通知。
- `flush_settings_save` を追加し、`cx.quit()` 直前（更新適用）とアプリ終了経路で呼ぶ。
- `ui/settings.rs` / `ui/setup.rs` / `app.rs` のハンドラ内 `settings.save(&self.db)` を
  `self.schedule_settings_save(cx)` に置換する。起動時の初期保存など `cx` を持たない箇所は同期のまま。

- [ ] **Step 4: 検証**

Run: `cargo test -p vrc-photon-ui && cargo clippy -p vrc-photon-ui --all-targets -- -D warnings`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/app.rs crates/ui/src/app/daemon.rs crates/ui/src/app/lifecycle.rs crates/ui/src/ui/settings.rs crates/ui/src/ui/setup.rs
git commit -m "fix: read monitor state from the snapshot and debounce settings saves (#768)"
```

---

### Task 6: 小規模整理 (#768-3, #768-4, #768-5, #768-6)

**Files:**
- Modify: `crates/ui/src/ui/search_suggest.rs:333-363`
- Modify: `crates/ui/src/clipboard.rs:294-330`, `:151`, `:612`, `:625`
- Modify: `crates/ui/src/update_view.rs`（テストモジュールを末尾へ移動）
- Modify: `crates/ui/src/app/lifecycle.rs`（batch 合成テスト）

**Interfaces:**
- Produces: `pub(crate) fn dib_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String>`

- [ ] **Step 1: search_suggest の check-then-unwrap を let-else 化**

`as_ref()` ガード後の `as_mut().unwrap()` / `as_ref().unwrap()` を
`let Some(value) = ... else { return ...; };` に置換する。挙動は変えない。

- [ ] **Step 2: clipboard の長さ契約を Result 化**

- `dib_from_rgba` は `rgba.len() != stride * height` のとき `Err("...")` を返す。
- 呼び出し元 3 箇所で `?` または `unwrap_or_else` を使い、既存のエラー表示へ流す。
- テストを追加: 長さ不一致で `Err`、正しい長さで先頭 `biSize == 40`。

- [ ] **Step 3: update_view のテストモジュールを末尾へ移動**

ファイル先頭の `#[cfg(test)] mod tests` を本番コードの下へ移動する。内容は変更しない。

- [ ] **Step 4: batch 合成テストを追加**

```rust
#[test]
fn a_mixed_ipc_batch_is_applied_in_order_with_one_repaint() {
    let (tx, rx) = async_channel::bounded(8);
    tx.try_send(Incoming::Connected { /* 既存テストの値 */ }).unwrap();
    tx.try_send(Incoming::Event(IpcEvent::ScanStarted { .. })).unwrap();
    tx.try_send(Incoming::Disconnected("closed".into())).unwrap();
    let first = rx.try_recv().unwrap();
    let batch = drain_ipc_batch(first, &rx);
    assert_eq!(batch.len(), 3);
    assert!(matches!(batch[0], Incoming::Connected { .. }));
    assert!(matches!(batch[2], Incoming::Disconnected(_)));
}
```

`Connected` / `ScanStarted` の必須フィールドは既存の IPC テストからコピーする。

- [ ] **Step 5: 検証とコミット**

Run: `cargo test -p vrc-photon-ui && cargo clippy -p vrc-photon-ui --all-targets -- -D warnings`
Expected: PASS

```bash
git add crates/ui/src/ui/search_suggest.rs crates/ui/src/clipboard.rs crates/ui/src/update_view.rs crates/ui/src/app/lifecycle.rs
git commit -m "chore: address UI review follow-ups (#768)"
```

---

### Task 7: 最終検証

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo test -p vrc-photon-ui --features demo`
- [ ] 計画のチェックボックスを `- [x]` に更新し、PR を作成する
