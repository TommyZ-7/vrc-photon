# Final Review Follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the eight final pre-release review findings (#745, #747, #748, #755, #766, #770, #771, #772) in one branch without changing UI behavior.

**Architecture:** Each task fixes one boundary or invariant in place. The core changes chunk every dynamic `IN (...)` list below SQLite's 32,766-parameter limit, make the link writes atomic, reject malformed sidecar results at the trust boundary, reject non-UTF-8 paths instead of lossy-converting them, and remove ambiguous internal helpers. Documentation changes state the existing contracts rather than widening them.

**Tech Stack:** Rust edition 2024, rusqlite, serde, thiserror, tempfile, image/png, gpui-kit (untouched).

**Spec:** GitHub issues #745, #747, #748, #755, #766, #770, #771, #772 (final pre-release review, 2026-11-19), https://github.com/TommyZ7/vrc-photon/issues/745 etc.

## Global Constraints

- Keep the change set to the files named in each task; no UI code changes.
- Use the existing 500-row chunk size for SQLite `IN (...)` lists; never bind more than 1,000 parameters in one statement.
- No new dependencies and no schema migration.
- User-facing errors stay localized where the surrounding code is localized.
- Every task ends with a focused test run and a commit; the branch must finish with `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` green.
- One PR on `fix/745-772-final-review-followups`; label it both `bug` and `enhancement`.

## Review Focus

- Selecting more than 32,766 photos must return the intersection of tags instead of a SQLite parameter-limit error.
- A failed `photo_players` write after a context upsert must leave no context row.
- Sidecar results containing NaN, infinity, a probability outside 0..=1, an inverted people interval, or a count outside that interval must be rejected before they reach the database.
- A non-UTF-8 photo root or directory component must produce a visible failure, never a lossy `abs_path`.
- A pre-existing plugin root with mode 0755 must become 0700 after install; plugin binaries must remain private.
- A real translation equal to the missing-translation sentinel must not take a different code path than any other translation.

---

### Task 1: Chunk bulk tag queries and inserts (#745, #766.5)

**Files:**
- Modify: `crates/core/src/db/tags.rs:120-236`
- Test: `crates/core/src/db/tests.rs`

**Interfaces:**
- Consumes: existing `Db::unique_photo_ids`, `Db::ensure_photos_exist`, `HashSet<String>` from `super::*`.
- Produces: private `const BULK_TAG_CHUNK: usize = 500;`; no public signature changes.

- [x] **Step 1: Write the failing regression test**

Append to `crates/core/src/db/tests.rs`:

```rust
#[test]
fn common_manual_tags_handles_more_photos_than_sqlite_bind_limit() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photos = (0..32_770)
        .map(|index| {
            sample_photo(
                root,
                &format!("/photos/common-{index}.png"),
                1_700_000_000 + index as i64,
            )
        })
        .collect::<Vec<_>>();
    let ids = db.upsert_photos(&photos).unwrap();

    db.add_tags_to_photos(&ids, &["shared".to_string(), "extra".to_string()])
        .unwrap();
    db.set_photo_tags(
        ids[0],
        &[
            "shared".to_string(),
            "extra".to_string(),
            "only-first".to_string(),
        ],
    )
    .unwrap();
    db.set_photo_tags(
        *ids.last().unwrap(),
        &[
            "shared".to_string(),
            "extra".to_string(),
            "only-last".to_string(),
        ],
    )
    .unwrap();

    assert_eq!(
        db.common_manual_tags(&ids).unwrap(),
        vec!["extra".to_string(), "shared".to_string()]
    );
}
```

- [x] **Step 2: Run the test and verify it fails for the parameter limit**

Run: `cargo test -p vrc-photon-core common_manual_tags_handles_more_photos_than_sqlite_bind_limit`
Expected: FAIL with a rusqlite/SQLite error mentioning too many SQL variables (the current query binds all 32,770 photo IDs).

- [x] **Step 3: Implement chunking**

In `crates/core/src/db/tags.rs`, add near the top of `impl Db`:

```rust
/// Keeps every dynamic `IN (...)` list well below SQLite's 32,766-value
/// bound-parameter limit.
const BULK_TAG_CHUNK: usize = 500;
```

Change `ensure_photos_exist` to iterate `photo_ids.chunks(BULK_TAG_CHUNK)`.

Replace the per-row insert loop in `add_tags_in_transaction` with one multi-row statement per tag per photo chunk:

```rust
for tag_id in &tag_ids {
    for chunk in photo_ids.chunks(BULK_TAG_CHUNK) {
        let values_sql = (0..chunk.len())
            .map(|index| format!("(?{}, ?{})", index * 2 + 1, index * 2 + 2))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "INSERT OR IGNORE INTO photo_tags(photo_id, tag_id) VALUES {values_sql}"
        );
        let mut values = Vec::with_capacity(chunk.len() * 2);
        for photo_id in chunk {
            values.push(*photo_id);
            values.push(*tag_id);
        }
        self.conn.execute(&sql, params_from_iter(values))?;
    }
}
```

Replace the body of `common_manual_tags` after `ensure_photos_exist` with a chunked intersection. `tags.name` is `UNIQUE`, so intersecting names is equivalent to intersecting IDs and avoids a second unbounded `IN (...)`:

```rust
let mut common: Option<HashSet<String>> = None;
for chunk in photo_ids.chunks(BULK_TAG_CHUNK) {
    let placeholders = (2..=chunk.len() + 1)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT t.name FROM tags t JOIN photo_tags pt ON pt.tag_id = t.id \
         WHERE pt.photo_id IN ({placeholders}) \
         GROUP BY t.id, t.name \
         HAVING COUNT(DISTINCT pt.photo_id) = ?1"
    );
    let mut values = Vec::with_capacity(chunk.len() + 1);
    values.push(chunk.len() as i64);
    values.extend(chunk.iter().copied());
    let mut stmt = self.conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(values), |row| row.get::<_, String>(0))?;
    let chunk_names = rows.collect::<rusqlite::Result<HashSet<_>>>()?;
    common = Some(match common {
        Some(previous) => previous.intersection(&chunk_names).cloned().collect(),
        None => chunk_names,
    });
    if common.as_ref().is_some_and(HashSet::is_empty) {
        return Ok(Vec::new());
    }
}

let mut names = common.unwrap_or_default().into_iter().collect::<Vec<_>>();
names.sort_by(|left, right| {
    left.to_ascii_lowercase()
        .cmp(&right.to_ascii_lowercase())
        .then_with(|| left.cmp(right))
});
Ok(names)
```

- [x] **Step 4: Run the test and verify it passes**

Run: `cargo test -p vrc-photon-core common_manual_tags_handles_more_photos_than_sqlite_bind_limit`
Expected: PASS.

- [x] **Step 5: Run the core tag tests**

Run: `cargo test -p vrc-photon-core tags`
Expected: PASS, including the existing bulk add/remove tests.

- [x] **Step 6: Commit**

```bash
git add crates/core/src/db/tags.rs crates/core/src/db/tests.rs
git commit -m "fix: chunk bulk tag queries and inserts (#745, #766)"
```

---

### Task 2: Make context and player writes atomic (#747)

**Files:**
- Modify: `crates/core/src/linker.rs:85-136`
- Test: `crates/core/src/linker.rs` (existing `mod tests`, starting around line 229)

**Interfaces:**
- Consumes: `Db::in_transaction`, `Db::set_photo_context`, `Db::set_photo_players`, existing `insert_photo` test helper.
- Produces: private `fn store_link(db: &Db, context: &PhotoContext, photo_id: i64, players: &[PhotoPlayer]) -> Result<()>`.

- [x] **Step 1: Write the failing atomicity test**

Add this test inside the existing `mod tests` in `crates/core/src/linker.rs`:

```rust
#[test]
fn xmp_link_rolls_back_context_when_players_write_fails() {
    let db = Db::open_in_memory().unwrap();
    let photo_id = insert_photo(&db, 1_700_000_000);
    let photo = db.photo_by_id(photo_id).unwrap().unwrap();

    db.connection()
        .execute_batch("DROP TABLE photo_players")
        .unwrap();

    let xmp = Some(XmpMetadata {
        world_name: Some("Test World".to_string()),
        world_id: None,
        author: None,
    });
    assert!(link_photo_with_xmp(&db, &photo, xmp, false).is_err());
    assert!(db.photo_context(photo_id).unwrap().is_none());
}
```

- [x] **Step 2: Run the test and verify it fails**

Run: `cargo test -p vrc-photon-core xmp_link_rolls_back_context_when_players_write_fails`
Expected: FAIL because the context row was committed before `set_photo_players` failed.

- [x] **Step 3: Add `store_link` and use it in all three branches**

Add near the top of `crates/core/src/linker.rs`:

```rust
/// Writes a link's context and players in one transaction.
///
/// `set_photo_context` clears the retry counter, so a crash after the
/// context write would make the photo ineligible for `unlinked_photos_due`
/// while leaving it without players. The transaction makes both writes
/// visible together.
fn store_link(
    db: &Db,
    context: &PhotoContext,
    photo_id: i64,
    players: &[PhotoPlayer],
) -> Result<()> {
    db.in_transaction(|db| {
        db.set_photo_context(context)?;
        db.set_photo_players(photo_id, players)?;
        Ok(())
    })
}
```

In the log branch, build the `Vec<PhotoPlayer>` first, then call
`store_link(db, &context, photo.id, &players)?;`. In the estimated branch,
replace the two calls with `store_link(db, &context, photo.id, &players)?;`.
In the XMP-only branch, replace the two calls with
`store_link(db, &context, photo.id, &[])?;`.

- [x] **Step 4: Run the test and verify it passes**

Run: `cargo test -p vrc-photon-core xmp_link_rolls_back_context_when_players_write_fails`
Expected: PASS, and `photo_context` is `None`.

- [x] **Step 5: Run all linker tests**

Run: `cargo test -p vrc-photon-core linker`
Expected: PASS, including the existing XMP/log/estimated tests.

- [x] **Step 6: Commit**

```bash
git add crates/core/src/linker.rs
git commit -m "fix: store link context and players atomically (#747)"
```

---

### Task 3: Document the raw connection contract (#748)

**Files:**
- Modify: `crates/core/src/db/mod.rs:248-250`

**Interfaces:**
- Consumes: nothing.
- Produces: doc comment on `Db::connection`; no signature change.

- [x] **Step 1: Add the contract documentation**

Replace the bare `pub fn connection` with:

```rust
/// Borrows the underlying SQLite connection.
///
/// `rusqlite::Connection` is `!Sync`. Callers must keep the `Db` on the
/// thread that owns the connection, or hold it behind a mutex as the
/// daemon does. Do not hold this borrow across slow I/O: it blocks every
/// other `Db` user, including UI reads.
///
/// Statements issued directly through this connection bypass the typed
/// `Db` methods and the revision counter that wakes UI pollers. After a
/// direct write, call [`Self::bump_library_revision`] (or the matching
/// AI/session bump) so snapshots refresh. Prefer the typed methods for
/// writes.
pub fn connection(&self) -> &Connection {
    &self.conn
}
```

- [x] **Step 2: Verify the crate still builds and tests**

Run: `cargo test -p vrc-photon-core --lib`
Expected: PASS.

- [x] **Step 3: Commit**

```bash
git add crates/core/src/db/mod.rs
git commit -m "docs: document the raw Db connection contract (#748)"
```

---

### Task 4: Validate sidecar numeric results (#755, #770.3)

**Files:**
- Modify: `crates/ai-host/src/sidecar.rs:1080-1110`
- Modify: `crates/ai-protocol/src/protocol.rs:143-176`
- Test: `crates/ai-host/src/sidecar.rs` (existing `mod tests`, starting around line 1177)

**Interfaces:**
- Consumes: `SidecarResult`, `Op`, `AiHostError::SidecarProtocol`.
- Produces: private `fn validate_probability(name: &str, value: f32) -> Result<()>`.

- [x] **Step 1: Write the failing probability validation test**

Add to the sidecar test module (and add
`use vrc_photon_ai_protocol::{PlaceBranch, PlaceProbabilities, TagProbability};`
inside `mod tests`):

```rust
#[test]
fn validate_result_rejects_out_of_range_probabilities() {
    let presence = SidecarResult::Presence {
        present: true,
        p_yes: f32::NAN,
    };
    assert!(matches!(
        validate_result(&presence, Op::Presence, None),
        Err(AiHostError::SidecarProtocol(_))
    ));

    let class = SidecarResult::PlaceClass {
        branch: PlaceBranch::Indoor,
        probabilities: PlaceProbabilities {
            indoor: 1.1,
            outdoor: 0.0,
            other: 1.0,
        },
    };
    assert!(validate_result(&class, Op::PlaceClass, None).is_err());

    let tags = SidecarResult::AtmosphereTags {
        tags: vec![TagProbability {
            key: "rain".to_string(),
            p_yes: -0.1,
        }],
    };
    assert!(validate_result(&tags, Op::AtmosphereTags, None).is_err());
}
```

- [x] **Step 2: Write the failing people-count interval test**

```rust
#[test]
fn validate_result_rejects_impossible_people_counts() {
    let inverted = SidecarResult::PeopleCount {
        count: 2,
        interval: [3, 1],
    };
    assert!(validate_result(&inverted, Op::PeopleCount, None).is_err());

    let outside = SidecarResult::PeopleCount {
        count: 4,
        interval: [1, 3],
    };
    assert!(validate_result(&outside, Op::PeopleCount, None).is_err());

    let valid = SidecarResult::PeopleCount {
        count: 2,
        interval: [1, 3],
    };
    assert!(validate_result(&valid, Op::PeopleCount, None).is_ok());
}
```

- [x] **Step 3: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-ai-host validate_result_rejects`
Expected: FAIL; `validate_result` currently accepts both results.

- [x] **Step 4: Implement the validation**

Add before `validate_result`:

```rust
fn validate_probability(name: &str, value: f32) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(AiHostError::SidecarProtocol(format!(
            "{name} must be a finite probability in 0..=1, got {value}"
        )));
    }
    Ok(())
}
```

After the existing branch check in `validate_result`, add:

```rust
match result {
    SidecarResult::Presence { p_yes, .. } => {
        validate_probability("presence.p_yes", *p_yes)?;
    }
    SidecarResult::PeopleCount { count, interval } => {
        if interval[0] > interval[1] {
            return Err(AiHostError::SidecarProtocol(format!(
                "people_count.interval is inverted: [{}, {}]",
                interval[0], interval[1]
            )));
        }
        if *count < interval[0] || *count > interval[1] {
            return Err(AiHostError::SidecarProtocol(format!(
                "people_count.count {count} is outside interval [{}, {}]",
                interval[0], interval[1]
            )));
        }
    }
    SidecarResult::PlaceClass { probabilities, .. } => {
        validate_probability("place_class.probabilities.indoor", probabilities.indoor)?;
        validate_probability("place_class.probabilities.outdoor", probabilities.outdoor)?;
        validate_probability("place_class.probabilities.other", probabilities.other)?;
    }
    SidecarResult::PlaceTags { tags, .. } | SidecarResult::AtmosphereTags { tags } => {
        for tag in tags {
            validate_probability(&format!("tag '{}' p_yes", tag.key), tag.p_yes)?;
        }
    }
}
```

- [x] **Step 5: Document the wire constraints**

In `crates/ai-protocol/src/protocol.rs`, add doc comments:

- On `Presence::p_yes`: “Probability in 0.0..=1.0. The host rejects non-finite or out-of-range values.”
- On `PeopleCount::count`: “Point estimate (median) of the number of people.”
- On `PeopleCount::interval`: “Model p10/p90 interval. `interval[0] <= interval[1]`, and `count` must lie inside the interval; the host rejects violations.”
- On each field of `PlaceProbabilities`: “Probability in 0.0..=1.0; the host validates it.”
- On `TagProbability::p_yes`: same probability rule.

- [x] **Step 6: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-ai-host validate_result_rejects`
Expected: PASS.

- [x] **Step 7: Run the ai-host and ai-protocol tests**

Run: `cargo test -p vrc-photon-ai-host -p vrc-photon-ai-protocol`
Expected: PASS.

- [x] **Step 8: Commit**

```bash
git add crates/ai-host/src/sidecar.rs crates/ai-protocol/src/protocol.rs
git commit -m "fix: validate sidecar probabilities and people intervals (#755, #770)"
```

---

### Task 5: Harden the verification stamp docs and plugin root (#770.1, #770.2)

**Files:**
- Modify: `crates/ai-host/src/model.rs:517-553`
- Modify: `crates/ai-host/src/install.rs:353-360`
- Test: `crates/ai-host/src/install.rs` (existing `mod tests`, starting around line 817)

**Interfaces:**
- Consumes: `plugins_root`, `TestPackage`, `install()` test helper.
- Produces: no signature changes; `plugins_root` is mode 0700 after install on Unix.

- [x] **Step 1: Write the failing plugin-root mode test**

Add inside the install test module:

```rust
#[cfg(unix)]
#[test]
fn install_restricts_the_plugins_root_to_the_owner() {
    use std::os::unix::fs::PermissionsExt;

    let data_dir = tempdir().unwrap();
    let root = plugins_root(data_dir.path());
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();

    let package = TestPackage::new();
    install(&package, data_dir.path()).unwrap();

    let mode = fs::metadata(&root).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}
```

- [x] **Step 2: Run the test and verify it fails**

Run: `cargo test -p vrc-photon-ai-host install_restricts_the_plugins_root_to_the_owner`
Expected: FAIL; the root stays 0755 because install only calls `create_dir_all`.

- [x] **Step 3: Set the plugin root mode**

In `crates/ai-host/src/install.rs`, immediately after
`fs::create_dir_all(&root)?;`, add:

```rust
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
}
```

- [x] **Step 4: Run the test and verify it passes**

Run: `cargo test -p vrc-photon-ai-host install_restricts_the_plugins_root_to_the_owner`
Expected: PASS with mode 0700.

- [x] **Step 5: Document the stamp threat model**

Add this doc comment above `fn stamp_matches` in `crates/ai-host/src/model.rs`:

```rust
/// Returns true when the on-disk verification stamp matches the manifest
/// and both model files.
///
/// The stamp is a fast path, not a cryptographic guarantee. It trusts the
/// file size and mtime recorded when a full SHA-256 verification last
/// passed. An attacker with write access as the same user (or root) can
/// preserve both values with `utimensat`, so this check does not defend
/// against a same-uid compromise. The model is trusted-sidecar data and
/// untrusted input is validated at the sidecar boundary; a full SHA-256
/// runs on install, the first verification, and whenever the stamp is
/// missing or does not match.
```

- [x] **Step 6: Run the ai-host tests**

Run: `cargo test -p vrc-photon-ai-host`
Expected: PASS, including `matching_stamp_is_trusted_without_rehashing`.

- [x] **Step 7: Commit**

```bash
git add crates/ai-host/src/model.rs crates/ai-host/src/install.rs
git commit -m "fix: keep plugin binaries private and document stamp limits (#770)"
```

---

### Task 6: Remove the remaining core edge-case hazards (#766.1-#766.4)

**Files:**
- Modify: `crates/core/src/bc7.rs:48`
- Modify: `crates/core/src/thumbnail.rs:513`
- Modify: `crates/core/src/db/photos.rs:564`
- Modify: `crates/core/src/settings.rs:697-699`
- Test: `crates/core/src/bc7.rs` (existing `mod tests`)

**Interfaces:**
- Consumes: existing `CoreError::Invalid`, `Option<usize>` return of `encoded_len`.
- Produces: private `fn checked_stride(width: u32) -> Option<u32>`; `is_under` removed.

- [x] **Step 1: Write the failing stride test**

Add to the `bc7` test module:

```rust
#[test]
fn checked_stride_rejects_overflow() {
    assert_eq!(checked_stride(1), Some(4));
    assert_eq!(checked_stride(u32::MAX), None);
}
```

- [x] **Step 2: Run the test and verify it fails**

Run: `cargo test -p vrc-photon-core checked_stride_rejects_overflow`
Expected: FAIL with “cannot find function `checked_stride`”.

- [x] **Step 3: Implement the checked stride**

Add above `encode_rgba`:

```rust
fn checked_stride(width: u32) -> Option<u32> {
    width.checked_mul(4)
}
```

Replace `stride: width * 4` with:

```rust
stride: checked_stride(width)
    .ok_or_else(|| CoreError::Invalid("BC7 stride overflow".to_string()))?,
```

- [x] **Step 4: Run the test and verify it passes**

Run: `cargo test -p vrc-photon-core checked_stride_rejects_overflow`
Expected: PASS.

- [x] **Step 5: Apply the behavior-preserving fixes**

In `crates/core/src/thumbnail.rs`, replace:

```rust
expected_bytes == Some(bytes as usize)
```

with:

```rust
expected_bytes == usize::try_from(bytes).ok()
```

In `crates/core/src/db/photos.rs`, inside `find_rebind_candidate`, replace:

```rust
let candidate = matches.pop().unwrap();
```

with:

```rust
let Some(candidate) = matches.pop() else {
    return Ok(None);
};
```

In `crates/core/src/settings.rs`, delete the whole function:

```rust
pub fn is_under(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).is_ok()
}
```

- [x] **Step 6: Run the core tests**

Run: `cargo test -p vrc-photon-core`
Expected: PASS.

- [x] **Step 7: Commit**

```bash
git add crates/core/src/bc7.rs crates/core/src/thumbnail.rs crates/core/src/db/photos.rs crates/core/src/settings.rs
git commit -m "fix: remove core overflow and unwrap hazards (#766)"
```

---

### Task 7: Reject non-UTF-8 roots and directory components (#771)

**Files:**
- Modify: `crates/core/src/scanner.rs:28-43, 124-141, 231-277`
- Modify: `crates/core/src/settings.rs:180-183, 554-556`
- Test: `crates/core/src/scanner.rs` (existing `mod tests`, starting around line 805)

**Interfaces:**
- Consumes: `ContainmentError::Unverifiable`, `CoreError::Invalid`.
- Produces: `scan_root_report` returns `CoreError::Invalid` for a non-UTF-8 root; `contained_rel_path` returns `ContainmentError::Unverifiable` for any non-UTF-8 component; `ScannedPhoto::into_new_photo` stays lossless because the scanner rejects those paths first.

- [x] **Step 1: Write the failing non-UTF-8 root test**

Add to the scanner test module:

```rust
#[cfg(unix)]
#[test]
fn scan_root_rejects_a_non_utf8_root() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = tempdir().unwrap();
    let root = dir
        .path()
        .join(OsString::from_vec(b"root-\xff".to_vec()));
    std::fs::create_dir(&root).unwrap();
    write_png(&root.join("photo.png"));

    let error = scan_root_report(&root).unwrap_err();
    assert!(error.to_string().contains("UTF-8"));
}
```

- [x] **Step 2: Write the failing non-UTF-8 directory test**

```rust
#[cfg(unix)]
#[test]
fn scan_reports_a_non_utf8_directory_component() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = tempdir().unwrap();
    let bad_dir = dir
        .path()
        .join(OsString::from_vec(b"dir-\xff".to_vec()));
    std::fs::create_dir(&bad_dir).unwrap();
    write_png(&bad_dir.join("photo.png"));

    let (photos, failures) = scan_root_report(dir.path()).unwrap();
    assert!(photos.is_empty());
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("UTF-8"));
}
```

- [x] **Step 3: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-core non_utf8`
Expected: FAIL; both paths are currently lossily converted.

- [x] **Step 4: Reject non-UTF-8 roots and components**

In `scan_root_report_with_progress_running`, after the `root.is_dir()` check, add:

```rust
if root.to_str().is_none() {
    return Err(CoreError::Invalid(format!(
        "撮影ルートに UTF-8 以外の文字が含まれています: {}",
        root.display()
    )));
}
```

In `contained_rel_path`, after computing `path_abs`, add:

```rust
if path_abs.to_str().is_none() {
    return Err(ContainmentError::Unverifiable(format!(
        "パスに UTF-8 以外の文字が含まれています: {}",
        path.display()
    )));
}
```

Keep the final `relative.to_string_lossy().replace('\\', "/")`; the check
above guarantees it is now lossless.

- [x] **Step 5: Run the tests and verify they pass**

Run: `cargo test -p vrc-photon-core non_utf8`
Expected: PASS.

- [x] **Step 6: Document the String storage constraint**

In `crates/core/src/settings.rs`, extend the `photo_roots` field doc:

```rust
/// Photo roots to index. The first entry is the primary VRChat folder.
/// Paths are stored as strings and must be valid UTF-8; scanning rejects a
/// non-UTF-8 root instead of replacing bytes with U+FFFD.
```

Add a doc comment to `ScannedPhoto::into_new_photo`:

```rust
/// Converts a scanned photo into a database row.
///
/// `abs_path` is stored as a `String`. The scanner rejects non-UTF-8 roots
/// and path components before this point, so `to_string_lossy` cannot
/// substitute replacement characters here.
```

- [x] **Step 7: Run the scanner and settings tests**

Run: `cargo test -p vrc-photon-core scanner`
Expected: PASS.

- [x] **Step 8: Commit**

```bash
git add crates/core/src/scanner.rs crates/core/src/settings.rs
git commit -m "fix: reject non-UTF-8 photo paths instead of rewriting them (#771)"
```

---

### Task 8: Remove the translation sentinel special case (#772)

**Files:**
- Modify: `crates/i18n/src/lib.rs:128-158`

**Interfaces:**
- Consumes: `lookup`, `lookup_exact`, `DEFAULT_LOCALE`, `MISSING`.
- Produces: private `fn translate_in(locale: &str, key: &str) -> Option<String>`; `tr_in` and `tr_args_in` keep their signatures.

- [x] **Step 1: Refactor the lookup chain**

Replace `tr_in` and `tr_args_in` with:

```rust
fn translate_in(locale: &str, key: &str) -> Option<String> {
    let locale = normalize(locale);
    lookup(&locale, key).or_else(|| lookup_exact(DEFAULT_LOCALE, key))
}

fn missing_translation(locale: &str, key: &str) -> String {
    tracing::warn!(locale = %normalize(locale), key, "missing translation");
    MISSING.to_string()
}

/// Translate a key in an explicit locale.
///
/// Lookup order: exact locale, base language (`pt-BR` → `pt`), Japanese.
/// A key missing everywhere renders [`MISSING`], never the key itself.
pub fn tr_in(locale: &str, key: &str) -> String {
    translate_in(locale, key).unwrap_or_else(|| missing_translation(locale, key))
}

/// Translate a key in an explicit locale and replace `%{name}` placeholders.
pub fn tr_args_in(locale: &str, key: &str, args: &[(&str, String)]) -> String {
    let Some(text) = translate_in(locale, key) else {
        return missing_translation(locale, key);
    };
    let names: Vec<&str> = args.iter().map(|(name, _)| *name).collect();
    let values: Vec<String> = args.iter().map(|(_, value)| value.clone()).collect();
    rust_i18n::replace_patterns(&text, &names, &values)
}
```

- [x] **Step 2: Run the i18n tests**

Run: `cargo test -p vrc-photon-i18n`
Expected: PASS. No new behavioral test is possible: a catalog value exactly equal to `MISSING` has no placeholders, so the old early return produced the same output. The refactor removes the internal special case while preserving output.

- [x] **Step 3: Commit**

```bash
git add crates/i18n/src/lib.rs
git commit -m "refactor: resolve missing translations without a sentinel check (#772)"
```

---

### Final verification and handoff

- [x] **Step 1: Mark all plan checkboxes complete**

Replace every `- [ ]` in this file with `- [x]` after the final verification passes.

- [x] **Step 2: Run the full local gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all three commands exit 0.

- [x] **Step 3: Commit the completed plan**

```bash
git add docs/superpowers/plans/2026-11-19-final-review-followups.md
git commit -m "docs: mark the final review follow-up plan complete"
```

- [x] **Step 4: Open the PR and wait for CI**

Push the branch, open one PR titled
“Fix final review follow-ups (#745, #747, #748, #755, #766, #770, #771, #772)”,
apply the `bug` and `enhancement` labels, and follow the `ci-watch` skill.
Merge only after every required check is green.
