# Session Bulk Collapse Implementation Plan (#660)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add "すべて格納" / "すべて展開" header actions that collapse or expand every displayed session in the Photos session view.

**Architecture:** Keep `PhotoBrowser` as the owner of collapse state. Cache the distinct session IDs of the current index there, expose a summary and a bulk mutation that return the changed IDs, then let `App` apply the same layout/animation/scroll-anchor flow used by the per-session toggle. The gallery header renders the two small ghost buttons only in session mode.

**Tech Stack:** Rust 2024, gpui-kit 0.9.1, SQLite (unchanged), rust-i18n YAML locales.

**Spec:** `docs/superpowers/specs/2026-11-16-issue-660-collapse-all-design.md`

## Global Constraints

- Buttons appear only for the library Photos tab in `PhotosViewMode::Sessions`; timeline and search results keep their current header.
- Labels are exactly `すべて格納` and `すべて展開` (ja) / `Collapse all` and `Expand all` (en).
- "Displayed sessions" means distinct `PhotoIndexEntry.session_id` values in the current library index. Unlinked photos are never counted or mutated.
- Collapse-all is disabled when every displayed session is collapsed or there are no sessions.
- Expand-all is disabled when no displayed session is collapsed.
- Collapse state remains in-memory only; no DB column or setting is added.
- Animate only IDs whose state actually changes; reduce motion settles immediately.
- Keep the top visible session header at its viewport position when one exists.
- Do not mix docs files into the implementation PR. Update `docs/ui.md` in a separate docs PR after the code is verified.

## Review Focus

- A filtered/reloaded index changes the displayed session set: the summary must not count stale `collapsed_sessions` IDs.
- A large library (100k photos) must not recompute the summary by scanning every entry on each header render.
- The top visible row may be a photo inside a session or an unlinked photo: the anchor must pick the corresponding header only in the first case.
- Rapid repeated clicks must be no-ops when no session changes state.
- 1080×700 with the inspector open must not clip the new actions; the sort label yields first.

---

### Task 1: PhotoBrowser summary and bulk mutation

**Files:**
- Modify: `crates/ui/src/state/photo_browser.rs`
- Test: `crates/ui/src/state/photo_browser.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub(crate) struct SessionCollapseSummary { pub(crate) total: usize, pub(crate) collapsed: usize }`
  - `impl SessionCollapseSummary { pub(crate) fn can_collapse(&self) -> bool; pub(crate) fn can_expand(&self) -> bool }`
  - `PhotoBrowser::session_collapse_summary(&self) -> SessionCollapseSummary`
  - `PhotoBrowser::set_displayed_sessions_collapsed(&mut self, collapsed: bool) -> Vec<i64>` returns the changed IDs sorted ascending.
- Field: `displayed_session_ids: HashSet<i64>` initialized from the index in `PhotoBrowser::new` and rebuilt in `replace_index`; use it for the existing `collapsed_sessions.retain` prune.

- [x] **Step 1: Write failing tests in `photo_browser.rs`**

Add helper coverage using `session_index()` and `index_entry_with_session`:

```rust
#[test]
fn session_collapse_summary_counts_only_displayed_sessions() {
    let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
    browser.collapsed_sessions.insert(7);
    browser.collapsed_sessions.insert(99); // not in the index
    let summary = browser.session_collapse_summary();
    assert_eq!(summary.total, 2);
    assert_eq!(summary.collapsed, 1);
    assert!(summary.can_collapse());
    assert!(summary.can_expand());
}

#[test]
fn collapsing_all_sessions_changes_only_displayed_ones() {
    let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
    browser.collapsed_sessions.insert(8);
    browser.collapsed_sessions.insert(99);
    let changed = browser.set_displayed_sessions_collapsed(true);
    assert_eq!(changed, vec![7]);
    assert!(browser.collapsed_sessions.contains(&7));
    assert!(browser.collapsed_sessions.contains(&8));
    assert!(browser.collapsed_sessions.contains(&99));
    assert_eq!(browser.set_displayed_sessions_collapsed(true), Vec::<i64>::new());
    assert!(!browser.session_collapse_summary().can_collapse());
    assert!(browser.session_collapse_summary().can_expand());
}

#[test]
fn expanding_all_sessions_changes_only_displayed_ones() {
    let mut browser = PhotoBrowser::new(session_index(), TEST_ROW_HEIGHT);
    browser.collapsed_sessions.extend([7, 8, 99]);
    let changed = browser.set_displayed_sessions_collapsed(false);
    assert_eq!(changed, vec![7, 8]);
    assert!(!browser.collapsed_sessions.contains(&7));
    assert!(!browser.collapsed_sessions.contains(&8));
    assert!(browser.collapsed_sessions.contains(&99));
    assert_eq!(browser.set_displayed_sessions_collapsed(false), Vec::<i64>::new());
    assert_eq!(browser.session_collapse_summary().collapsed, 0);
}
```

Also assert `browser.session_rows` becomes `None` after a non-empty bulk mutation (rebuild required), and that `replace_index` refreshes `displayed_session_ids` by collapsing a session that exists only in the new index.

- [x] **Step 2: Run the tests and verify they fail to compile**

Run: `cargo test -p vrc-photon-ui state::photo_browser::tests::session_collapse_summary_counts_only_displayed_sessions`
Expected: compile error, missing `session_collapse_summary`.

- [x] **Step 3: Implement the state API**

Add the struct near `SessionLayout`:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SessionCollapseSummary {
    pub(crate) total: usize,
    pub(crate) collapsed: usize,
}

impl SessionCollapseSummary {
    pub(crate) fn can_collapse(&self) -> bool {
        self.total > 0 && self.collapsed < self.total
    }

    pub(crate) fn can_expand(&self) -> bool {
        self.collapsed > 0
    }
}
```

Build `displayed_session_ids` once per index in `new` and `replace_index`. Implement:

```rust
pub(crate) fn session_collapse_summary(&self) -> SessionCollapseSummary {
    SessionCollapseSummary {
        total: self.displayed_session_ids.len(),
        collapsed: self
            .collapsed_sessions
            .iter()
            .filter(|id| self.displayed_session_ids.contains(id))
            .count(),
    }
}

pub(crate) fn set_displayed_sessions_collapsed(&mut self, collapsed: bool) -> Vec<i64> {
    let mut changed: Vec<i64> = if collapsed {
        self.displayed_session_ids
            .difference(&self.collapsed_sessions)
            .copied()
            .collect()
    } else {
        self.displayed_session_ids
            .intersection(&self.collapsed_sessions)
            .copied()
            .collect()
    };
    changed.sort_unstable();
    if changed.is_empty() {
        return changed;
    }
    for id in &changed {
        if collapsed {
            self.collapsed_sessions.insert(*id);
        } else {
            self.collapsed_sessions.remove(id);
        }
    }
    self.session_scroll_anchor = None;
    self.pending_photo_anchor = None;
    self.pending_photo_expected_offset = None;
    self.session_rows = None;
    self.session_rows_key = None;
    changed
}
```

- [x] **Step 4: Run the state tests**

Run: `cargo test -p vrc-photon-ui state::photo_browser::tests::session_collapse`
Expected: all new tests pass.

- [x] **Step 5: Commit**

```bash
git add crates/ui/src/state/photo_browser.rs
git commit -m "feat(ui): add bulk session collapse state for #660"
```

---

### Task 2: App bulk action with scroll anchor

**Files:**
- Modify: `crates/ui/src/app/session.rs`
- Test: `crates/ui/src/app/session.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `PhotoBrowser::set_displayed_sessions_collapsed`, `SessionCollapseSummary`.
- Produces:
  - `pub(crate) fn set_all_sessions_collapsed(&mut self, collapsed: bool, viewport_height: f32, cx: &mut Context<Self>)`
  - Private helper `fn bulk_anchor_for_scroll(rows: &SessionRows, entries: &[PhotoIndexEntry], scroll_top: f32, heights: &[f32], viewport_height: f32) -> Option<(i64, f32)>`

- [x] **Step 1: Write failing tests for the anchor helper**

In `app/session.rs` tests, build a `SessionRows` from entries using `SessionRows::build(entries, 2, 200.0, 92.0, 46.0, 44.0, &HashSet::new(), &HashSet::new())`. Verify:

```rust
#[test]
fn bulk_anchor_uses_the_top_visible_session_header() {
    // entries: session 7 photos then session 8 photos
    // scroll into session 8's photo row; expect (8, viewport_y) where the
    // header offset and scroll top produce the relative y.
}

#[test]
fn bulk_anchor_clamps_a_partially_hidden_header_into_view() {
    // scroll below the header by less than the header height; expect y == 0.0
}

#[test]
fn bulk_anchor_is_none_for_unlinked_photos() {
    // top photo has session_id None; expect None
}
```

- [x] **Step 2: Run the anchor tests and verify they fail**

Run: `cargo test -p vrc-photon-ui app::session::tests::bulk_anchor`
Expected: compile error, missing helper.

- [x] **Step 3: Implement the helper and `set_all_sessions_collapsed`**

Helper:
```rust
fn bulk_anchor_for_scroll(
    rows: &SessionRows,
    entries: &[PhotoIndexEntry],
    scroll_top: f32,
    heights: &[f32],
    viewport_height: f32,
) -> Option<(i64, f32)> {
    let photo_index = rows.top_photo_index_with_heights(scroll_top, heights)?;
    let session_id = entries.get(photo_index)?.session_id?;
    let header_offset = rows.offset_for_session_with_heights(session_id, heights)?;
    let viewport_y = keep_header_viewport_y(
        header_offset - scroll_top,
        SESSION_HEADER_ROW_HEIGHT,
        viewport_height,
    );
    Some((session_id, viewport_y))
}
```

`set_all_sessions_collapsed` mirrors `toggle_session_collapsed`:
1. Capture `now`, columns, row height.
2. Run `session_layout` before the mutation and compute `bulk_anchor_for_scroll` from `session_scroll_top`.
3. Call `set_displayed_sessions_collapsed`; return early when the changed list is empty.
4. Animate each changed ID (or settle each under reduce motion).
5. Run `session_layout` after; when an anchor exists, set `header_scroll_offset(after_header_offset, viewport_y)` and set the session scroll anchor only while animating.
6. `drop(browser); cx.notify();`

- [x] **Step 4: Run the app tests**

Run: `cargo test -p vrc-photon-ui app::session::tests`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/ui/src/app/session.rs
git commit -m "feat(ui): apply bulk session collapse with a scroll anchor for #660"
```

---

### Task 3: Header buttons, narrow-width rule, and translations

**Files:**
- Modify: `crates/ui/src/ui/gallery.rs`
- Modify: `crates/ui/src/ui/theme.rs` (header width helper)
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`
- Test: `crates/ui/src/ui/gallery.rs`, `crates/ui/src/ui/theme.rs`

**Interfaces:**
- Consumes: `PhotoBrowser::session_collapse_summary`, `App::set_all_sessions_collapsed`.
- Produces:
  - `pub(crate) fn gallery_header_width(window_width: f32, sidebar_collapsed: bool, inspector_collapsed: bool) -> f32` in `theme.rs`
  - `fn shows_session_bulk_actions(mode: PhotosViewMode) -> bool` in `gallery.rs`
  - `fn shows_sort_label(mode: PhotosViewMode, header_width: f32) -> bool` in `gallery.rs`
  - `const SESSION_BULK_HEADER_MIN_WIDTH: f32 = 640.0;`

- [x] **Step 1: Write failing tests**

In `gallery.rs` tests:
```rust
#[test]
fn session_bulk_actions_are_only_shown_in_session_mode() {
    assert!(!shows_session_bulk_actions(PhotosViewMode::Timeline));
    assert!(shows_session_bulk_actions(PhotosViewMode::Sessions));
}

#[test]
fn the_sort_label_yields_before_bulk_actions_in_a_narrow_session_header() {
    assert!(shows_sort_label(PhotosViewMode::Timeline, 400.0));
    assert!(!shows_sort_label(PhotosViewMode::Sessions, 600.0));
    assert!(shows_sort_label(PhotosViewMode::Sessions, 700.0));
}
```

In `theme.rs`, assert `gallery_header_width(1080.0, false, false) < 640.0` and `gallery_header_width(1280.0, false, false) >= 640.0` for the implemented threshold; keep values aligned with the actual helper result.

- [x] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p vrc-photon-ui ui::gallery::tests::session_bulk_actions_are_only_shown_in_session_mode ui::theme::tests::the_gallery_header_width_matches_the_responsive_columns`
Expected: compile error.

- [x] **Step 3: Implement theme and header**

Extract the gallery width calculation from `grid_available_width` so the header helper and the grid share one source. `gallery_header_width` returns gallery width minus `GALLERY_PAD_X * 2.0` (do not subtract the rail).

In `render_header`:
- Read `summary = browser.session_collapse_summary()`.
- Compute `header_width = gallery_header_width(width, app.sidebar_collapsed, app.inspector_collapsed)`.
- Keep the existing month/count child and spacer.
- Insert the bulk actions only when `shows_session_bulk_actions(mode)`.
- Render the sort label only when `shows_sort_label(mode, header_width)`.

Action buttons:
```rust
fn bulk_button(
    id: &'static str,
    label: String,
    disabled: bool,
    collapsed: bool,
    viewport_height: f32,
    app: WeakEntity<App>,
) -> Button {
    Button::new(SharedString::from(id))
        .label(label)
        .small()
        .ghost()
        .disabled(disabled)
        .on_click(move |_, _, cx| {
            if let Some(app) = app.upgrade() {
                app.update(cx, |app, cx| {
                    app.set_all_sessions_collapsed(collapsed, viewport_height, cx);
                });
            }
        })
}
```

Place the two buttons in an `h_flex().flex_shrink_0().items_center().gap(px(2.))` immediately before `view_mode_toggle`. Use `session_grid_viewport_height(window)` for the viewport height.

Add translations:
```yaml
session.collapse_all: "すべて格納"
session.expand_all: "すべて展開"
```
and
```yaml
session.collapse_all: "Collapse all"
session.expand_all: "Expand all"
```

- [x] **Step 4: Run the unit tests**

Run: `cargo test -p vrc-photon-ui ui::gallery::tests ui::theme::tests`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/ui/src/ui/gallery.rs crates/ui/src/ui/theme.rs crates/i18n/locales/ja.yml crates/i18n/locales/en.yml
git commit -m "feat(ui): add collapse-all and expand-all session header actions for #660"
```

---

### Task 4: Workspace verification and visual check

**Files:** none (verification only)

- [x] **Step 1: Format**

Run: `cargo fmt --all -- --check`
Expected: no output, exit 0.

- [x] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: exit 0.

- [x] **Step 3: Tests**

Run: `cargo test --workspace`
Expected: exit 0.

- [x] **Step 4: Visual check**

Use the `gnome-wayland-visual-qa` skill to launch `cargo run -p vrc-photon-ui -- --demo` and capture the Photos session view. Confirm:
- Both buttons sit left of the mode toggle.
- Disabled states match the state table.
- Timeline mode has no buttons.
- 1080×700 with the inspector open keeps both buttons visible and hides the sort label.

- [x] **Step 5: Commit any verification fixes**

Only if Step 4 required code changes.

---

### Task 5: Docs PR (separate)

**Files:**
- Modify: `docs/ui.md`

- [x] After the implementation PR is merged, create a docs-only branch and update §2.4.2 with the header actions, disabled conditions, scroll rule, animation rule, and narrow-width sort hiding.
- [x] Run CI and merge the docs PR separately.
