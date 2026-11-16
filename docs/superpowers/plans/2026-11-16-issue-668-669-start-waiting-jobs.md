# 待機中の送信・タグ付けジョブ開始 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 手動の Discord 送信・AI タグ付けで、既存の待機行を新規作成せず開始する。

**Architecture:** core に「待機行を pending へ昇格する」DB メソッドを追加し、UI の手動操作は「新規投入を試す → 失敗したら昇格」の順で呼ぶ。行は作り直さないため、送信元・種別・作成順は維持される。

**Tech Stack:** Rust (edition 2024), rusqlite, gpui-kit

**Spec:** `docs/superpowers/specs/2026-11-16-issue-668-669-start-waiting-jobs-design.md`

## Global Constraints

- DB スキーマは変更しない。
- 自動送信・自動タグ付けの投入条件と一括リリースは変更しない。
- 新しいトースト文言・翻訳キーは追加しない。
- 1PR に両 Issue を含める。

## Review Focus

- 待機行が他プラグイン・他写真に影響しないこと。
- 昇格後も送信行の `origin` / `kind` / `created_at` が変わらないこと。
- すべて `pending` / `processing` の写真で従来の案内が維持されること。
- 待機と新規が混在する複数選択で件数が正しいこと。

---

### Task 1: 送信行の昇格 API

**Files:**
- Modify: `crates/core/src/db/send.rs` (`release_waiting_sends` の付近)
- Test: `crates/core/src/db/tests.rs` (`enqueue_send_dedupes_active_rows_and_allows_resend_after_terminal` の付近)

**Interfaces:**
- Produces: `pub fn start_waiting_send(&self, photo_id: i64) -> Result<bool>`

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn start_waiting_send_promotes_the_photo_row_and_keeps_its_origin() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(photo, SendKind::Photo, SendOrigin::Auto, SendStatus::Waiting)
        .unwrap()
        .unwrap();

    assert!(db.start_waiting_send(photo).unwrap());
    let entry = db.send_entry_by_id(id).unwrap().unwrap();
    assert_eq!(entry.status, SendStatus::Pending);
    assert_eq!(entry.origin, SendOrigin::Auto);
    assert!(!db.start_waiting_send(photo).unwrap());
}

#[test]
fn start_waiting_send_ignores_pending_rows_and_other_photos() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let other = seed_other_photo(&db);
    let pending = db
        .enqueue_send(photo, SendKind::Photo, SendOrigin::Manual, SendStatus::Pending)
        .unwrap()
        .unwrap();
    let waiting = db
        .enqueue_send(other, SendKind::Photo, SendOrigin::Auto, SendStatus::Waiting)
        .unwrap()
        .unwrap();

    assert!(!db.start_waiting_send(photo).unwrap());
    assert_eq!(
        db.send_entry_by_id(pending).unwrap().unwrap().status,
        SendStatus::Pending
    );
    assert_eq!(
        db.send_entry_by_id(waiting).unwrap().unwrap().status,
        SendStatus::Waiting
    );
}
```

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test -p vrc-photon-core start_waiting_send -- --nocapture`
Expected: FAIL (method not found)

- [x] **Step 3: Implement `start_waiting_send`**

`crates/core/src/db/send.rs` の `release_waiting_sends` の前に追加する。1写真だけを対象に `waiting` を `pending` へ更新し、更新行数を `bool` で返す。

```rust
/// Promotes the photo's waiting send so the worker can pick it up.
/// Only `waiting` rows change; pending and processing rows are untouched.
pub fn start_waiting_send(&self, photo_id: i64) -> Result<bool> {
    let changed = self.conn.execute(
        "UPDATE send_outbox SET status = 'pending' \
         WHERE photo_id = ?1 AND status = 'waiting'",
        params![photo_id],
    )?;
    Ok(changed == 1)
}
```

- [x] **Step 4: Run tests to verify they pass**

Run: `cargo test -p vrc-photon-core start_waiting_send`
Expected: PASS

- [x] **Step 5: Commit**

```bash
git add crates/core/src/db/send.rs crates/core/src/db/tests.rs
git commit -m "feat(core): promote a waiting send when starting it manually"
```

---

### Task 2: タグ付けジョブの昇格 API

**Files:**
- Modify: `crates/core/src/db/ai.rs` (`release_waiting_tagging_jobs` の付近)
- Test: `crates/core/src/db/tests.rs` (`release_waiting_moves_only_waiting_jobs_for_the_plugin` の付近)

**Interfaces:**
- Produces: `pub fn start_waiting_tagging_jobs(&self, photo_ids: &[i64], plugin_id: &str) -> Result<usize>`

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn start_waiting_tagging_jobs_promotes_only_the_requested_photos() {
    let db = Db::open_in_memory().unwrap();
    let waiting = seed_photo(&db);
    let other_waiting = seed_other_photo(&db);
    let other_plugin = seed_photos(&db, 3)[2];
    let pending = seed_photos(&db, 4)[3];
    db.enqueue_tagging_jobs(&[waiting], AI_PLUGIN, TaggingJobStatus::Waiting)
        .unwrap();
    db.enqueue_tagging_jobs(&[other_waiting], AI_PLUGIN, TaggingJobStatus::Waiting)
        .unwrap();
    db.enqueue_tagging_jobs(&[other_plugin], "other-plugin", TaggingJobStatus::Waiting)
        .unwrap();
    db.enqueue_tagging_jobs(&[pending], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();

    assert_eq!(
        db.start_waiting_tagging_jobs(&[waiting, pending], AI_PLUGIN)
            .unwrap(),
        1
    );
    assert_eq!(
        db.tagging_jobs_for_photo(waiting).unwrap()[0].status,
        TaggingJobStatus::Pending
    );
    assert_eq!(
        db.tagging_jobs_for_photo(other_waiting).unwrap()[0].status,
        TaggingJobStatus::Waiting
    );
    assert_eq!(
        db.tagging_jobs_for_photo(other_plugin).unwrap()[0].status,
        TaggingJobStatus::Waiting
    );
    assert_eq!(
        db.tagging_jobs_for_photo(pending).unwrap()[0].status,
        TaggingJobStatus::Pending
    );
    assert_eq!(
        db.start_waiting_tagging_jobs(&[waiting], AI_PLUGIN)
            .unwrap(),
        0
    );
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p vrc-photon-core start_waiting_tagging_jobs -- --nocapture`
Expected: FAIL (method not found)

- [x] **Step 3: Implement `start_waiting_tagging_jobs`**

`crates/core/src/db/ai.rs` の `release_waiting_tagging_jobs` の後に追加する。`enqueue_tagging_jobs` と同じく即時トランザクションで写真ごとに更新し、進捗・ステージ・エラー・開始/終了時刻を初期化する。

```rust
/// Promotes `waiting` jobs for the given photos and plugin to `pending`.
/// Other photos, other plugins, and terminal jobs are left untouched.
pub fn start_waiting_tagging_jobs(
    &self,
    photo_ids: &[i64],
    plugin_id: &str,
) -> Result<usize> {
    let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
    let mut changed = 0;
    for photo_id in photo_ids {
        changed += tx.execute(
            "UPDATE tagging_jobs SET status = 'pending', progress = 0, stage = NULL, \
                error = NULL, started_at = NULL, finished_at = NULL \
             WHERE photo_id = ?1 AND plugin_id = ?2 AND status = 'waiting'",
            params![photo_id, plugin_id],
        )?;
    }
    tx.commit()?;
    Ok(changed)
}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test -p vrc-photon-core start_waiting_tagging_jobs`
Expected: PASS

- [x] **Step 5: Commit**

```bash
git add crates/core/src/db/ai.rs crates/core/src/db/tests.rs
git commit -m "feat(core): promote waiting tagging jobs for manual retagging"
```

---

### Task 3: UI の手動操作を昇格に対応させる

**Files:**
- Modify: `crates/ui/src/app/discord.rs` (`enqueue_photo_send`)
- Modify: `crates/ui/src/app/ai.rs` (`enqueue_manual_retag`)
- Test: `crates/ui/src/app/ai.rs` (`manual_retag_enqueues_one_job_and_wakes` の付近)

**Interfaces:**
- Consumes: `Db::start_waiting_send`, `Db::start_waiting_tagging_jobs`
- Produces: 変更なし（`ManualRetagOutcome` の既存契約を維持）

- [x] **Step 1: Write the failing UI tests**

`crates/ui/src/app/ai.rs` の tests モジュールへ追加する。

```rust
#[test]
fn manual_retag_starts_a_waiting_job_without_creating_another() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.enqueue_tagging_jobs(&[photo], AI_PLUGIN_ID, TaggingJobStatus::Waiting)
        .unwrap();

    let outcome = enqueue_manual_retag(&db, true, true, &[photo]).unwrap();
    assert_eq!(outcome, ManualRetagOutcome::Queued(1));

    let jobs = db.tagging_jobs_for_photo(photo).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, TaggingJobStatus::Pending);
}

#[test]
fn manual_retag_counts_started_and_new_jobs_together() {
    let db = Db::open_in_memory().unwrap();
    let waiting = seed_photo(&db);
    let fresh = seed_other_photo(&db);
    db.enqueue_tagging_jobs(&[waiting], AI_PLUGIN_ID, TaggingJobStatus::Waiting)
        .unwrap();

    let outcome = enqueue_manual_retag(&db, true, true, &[waiting, fresh]).unwrap();
    assert_eq!(outcome, ManualRetagOutcome::Queued(2));
    assert_eq!(
        db.tagging_jobs_for_photo(waiting).unwrap()[0].status,
        TaggingJobStatus::Pending
    );
    assert_eq!(
        db.tagging_jobs_for_photo(fresh).unwrap()[0].status,
        TaggingJobStatus::Pending
    );
}

#[test]
fn manual_retag_reports_none_when_every_photo_is_already_active() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.enqueue_tagging_jobs(&[photo], AI_PLUGIN_ID, TaggingJobStatus::Pending)
        .unwrap();

    let outcome = enqueue_manual_retag(&db, true, true, &[photo]).unwrap();
    assert_eq!(outcome, ManualRetagOutcome::Queued(0));
    assert!(!outcome.wakes());
}
```

`crates/ui/src/app/discord.rs` には、トーストの既存テストを変更せず、DB 昇格 API の組み合わせを Task 1 の core テストで担保する。UI 側の分岐は手動確認対象とする。

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test -p vrc-photon-ui manual_retag_`
Expected: FAIL (`Queued(0)` になる、または待機ジョブが pending にならない)

- [x] **Step 3: Update `enqueue_manual_retag`**

`crates/ui/src/app/ai.rs` の `enqueue_manual_retag` で、昇格件数と新規投入件数を合算する。

```rust
let started = db.start_waiting_tagging_jobs(photo_ids, AI_PLUGIN_ID)?;
let inserted =
    db.enqueue_tagging_jobs(photo_ids, AI_PLUGIN_ID, TaggingJobStatus::Pending)?;
Ok(ManualRetagOutcome::Queued(started + inserted))
```

- [x] **Step 4: Update `enqueue_photo_send`**

`crates/ui/src/app/discord.rs` の `enqueue_photo_send` の `Ok(None)` 分岐を、待機行の昇格を試す形にする。成功時は履歴更新・`send_wake_sends`・成功トースト、昇格できなければ従来の info トースト、DB エラーはエラートースト。

```rust
Ok(None) => match self.db.start_waiting_send(photo_id) {
    Ok(true) => {
        self.reload_send_history(cx);
        self.send_wake_sends(cx);
        self.toast_success(manual_send_toast(Ok(true)), cx);
    }
    Ok(false) => self.toast_info(manual_send_toast(Ok(false)), cx),
    Err(error) => {
        tracing::warn!(%error, photo_id, "cannot start a waiting Discord send");
        self.toast_error(
            format!("{}: {error}", tr!("photos.action.discord_error")),
            cx,
        );
    }
},
```

- [x] **Step 5: Run tests to verify they pass**

Run: `cargo test -p vrc-photon-ui manual_retag_`
Expected: PASS

- [x] **Step 6: Commit**

```bash
git add crates/ui/src/app/ai.rs crates/ui/src/app/discord.rs
git commit -m "feat(ui): start waiting rows on manual send and tagging"
```

---

### Task 4: ワークスペース検証

- [x] **Step 1: Format**

Run: `cargo fmt --all -- --check`
Expected: 差分なし

- [x] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: warning/error なし

- [x] **Step 3: Tests**

Run: `cargo test --workspace`
Expected: 全テスト PASS

- [x] **Step 4: Commit (必要な場合のみ)**

検証で生じた修正をコミットする。
