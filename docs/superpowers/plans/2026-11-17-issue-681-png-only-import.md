# 取り込み対象を PNG のみに絞る (#681) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 写真の索引・監視・手動取り込みの対象を PNG のみにし、JPG/JPEG を取り込まない（#681）。

**Architecture:** 共通判定の `photo_filename::is_source_photo` を PNG のみに変更する。走査・監視・取り込みは既にこの関数を共有しているため、各経路の拡張子判定は増やさない。UI のファイル選択フィルタと文言だけを別途 PNG に合わせる。JPG/JPEG のデコード・表示・圧縮は変更しない。

**Tech Stack:** Rust stable（edition 2024）、`notify`、`rfd`、rust-i18n、gpui-kit 0.9.1、SQLite

**Spec:** `docs/superpowers/specs/2026-11-17-issue-681-png-only-import-design.md`

## Global Constraints

- 対象形式は拡張子 `png` のみ（大文字拡張子も可）。`jpg` / `jpeg` は走査・監視・手動取り込みのすべてで対象外。
- 既存 DB の JPG/JPEG 行に対する移行・削除・一括クリーンアップは追加しない。
- XMP 読み取りは現行どおり PNG 専用。JPG 用のフォールバックは追加しない。
- JPG/JPEG のデコード・表示・サムネイル・MCP 配信・タグ付けスナップショット・圧縮は変更しない。
- 圧縮形式と出力先は変更しない。PNG 生成物の derivative パス除外も今回は追加しない。
- `compressed/`、シンボリックリンク、寸法を読めないファイルの既存挙動は変えない。
- 監視の削除イベントは拡張子で絞らない（表示用派生の削除検出を維持する）。
- この計画ファイルと spec はコード PR に含めない（`AGENTS.md` §4）。実装は別 worktree・別 PR で行う。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## Review Focus

1. **判定の一元化**: 走査・監視・取り込みが同じ `is_source_photo` を通り、JPG を索引しない。
2. **失敗扱いにしない**: 対象外の JPG を `ScanFailure` や取り込み失敗として数えない。
3. **手動取り込み**: ファイル選択・フォルダ走査・1ファイル検査の3経路すべてが PNG のみ。
4. **既存 PNG の回帰**: 走査、増分索引、XMP 紐付け、圧縮、表示が従来どおり動く。
5. **表示機能の維持**: JPG のデコード・表示テストが残り、通る。

---

### Task 0: 実装用 worktree

- [x] `main` を最新化し、`origin/main` から実装 worktree を作る。
  ```bash
  git fetch origin
  git worktree add ../vrc-photo-manager-681-impl -b feat/681-png-only-import origin/main
  ```
- [x] main ワークツリーと docs 用ワークツリーの未コミット変更には触れない。

---

### Task 1: 共通判定を PNG のみにする

**Files:**
- Modify: `crates/core/src/photo_filename.rs`
- Test: `crates/core/src/photo_filename.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- `pub fn is_source_photo(path: &Path) -> bool` の意味を「PNG の原本写真」に変更する。シグネチャは変えない。

- [x] **Step 1: 失敗するテストへ更新する**

既存の `extension_detection_is_case_insensitive` を次の内容に置き換える。

```rust
#[test]
fn extension_detection_accepts_only_png() {
    assert!(is_source_photo(Path::new("a.png")));
    assert!(is_source_photo(Path::new("a.PNG")));
    assert!(!is_source_photo(Path::new("a.jpg")));
    assert!(!is_source_photo(Path::new("a.JPEG")));
    assert!(!is_source_photo(Path::new("a.webp")));
    assert!(!is_source_photo(Path::new("a")));
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core photo_filename::tests::extension_detection_accepts_only_png`
Expected: FAIL（`a.jpg` が true になる）

- [x] **Step 3: `is_source_photo` と doc コメントを変更する**

```rust
/// Returns true when the file extension is the PNG format VRChat produces.
///
/// JPG/JPEG files are not VRChat photos (for example Steam screenshots) and
/// are outside scanning, watching, and importing.
pub fn is_source_photo(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("png")
    )
}
```

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core photo_filename::tests`
Expected: PASS

---

### Task 2: 走査のテストを PNG 前提に更新する

**Files:**
- Modify: `crates/core/src/scanner.rs`
- Test: `crates/core/src/scanner.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- 本番コードは変更しない。`walk` と `scan_one_detailed` は Task 1 の判定に従う。

- [x] **Step 1: `scans_and_indexes_vrchat_photos` に JPG を混ぜる**

ルート直下と `compressed/` の PNG に加えて、ルート直下に JPG を作る。

```rust
crate::image_decode::test_support::write_jpeg_with_orientation(
    &root.join("Steam_2026-10-21_21-14-04.221_4x3.jpg"),
    &RgbaImage::from_pixel(4, 3, Rgba([10, 20, 30, 255])),
    1,
);
```

既存の `photos.len() == 1`、`report.found == 1`、`report.indexed == 1`、
`db.count_photos() == 1` はそのまま維持する。

- [x] **Step 2: JPG 専用の走査テストを追加する**

```rust
#[test]
fn scan_root_ignores_jpeg_sources() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write_png(&root.join("VRChat_2026-10-21_21-14-03.221_4x3.png"));
    crate::image_decode::test_support::write_jpeg_with_orientation(
        &root.join("Steam_2026-10-21_21-14-04.221_4x3.jpg"),
        &RgbaImage::from_pixel(4, 3, Rgba([10, 20, 30, 255])),
        1,
    );
    crate::image_decode::test_support::write_jpeg_with_orientation(
        &root.join("Steam_2026-10-21_21-14-05.221_4x3.jpeg"),
        &RgbaImage::from_pixel(4, 3, Rgba([10, 20, 30, 255])),
        1,
    );

    let photos = scan_root(root).unwrap();
    assert_eq!(photos.len(), 1);
    assert_eq!(photos[0].file_name, "VRChat_2026-10-21_21-14-03.221_4x3.png");
}
```

- [x] **Step 3: `scan_one_reports_orientation_aware_dimensions` を JPG 非対象のテストへ置き換える**

```rust
#[test]
fn scan_one_ignores_jpeg_sources() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("Steam_2026-10-21_21-14-03.221_200x100.jpg");
    crate::image_decode::test_support::write_jpeg_with_orientation(
        &path,
        &RgbaImage::from_pixel(200, 100, Rgba([10, 20, 30, 255])),
        6,
    );

    assert!(scan_one(dir.path(), &path).is_none());
    assert!(scan_one_detailed(dir.path(), &path).is_none());
}
```

EXIF 回転を含む寸法の検証は `crates/core/src/image_decode.rs` の
`image_info_reports_oriented_dimensions_and_orientation` が既に担っているため、
走査側のテストは削除してよい。

- [x] **Step 4: 走査テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core scanner::tests`
Expected: PASS

---

### Task 3: 手動取り込みのテストを追加する

**Files:**
- Modify: `crates/core/src/importer.rs`
- Test: `crates/core/src/importer.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- 本番コードは変更しない。`collect_folder_sources` と `import_one` は Task 1 の判定に従う。

- [x] **Step 1: 直接指定された JPG/JPEG がスキップされるテストを追加する**

```rust
#[test]
fn skips_jpeg_sources() {
    let dir = tempdir().unwrap();
    let png = dir.path().join("VRChat_2026-10-21_21-14-03.221_4x3.png");
    write_png(&png);
    let jpg = dir.path().join("Steam_2026-10-21_21-14-04.221_4x3.jpg");
    crate::image_decode::test_support::write_jpeg_with_orientation(
        &jpg,
        &RgbaImage::from_pixel(4, 3, Rgba([10, 20, 30, 255])),
        1,
    );
    let jpeg = dir.path().join("Steam_2026-10-21_21-14-05.221_4x3.jpeg");
    crate::image_decode::test_support::write_jpeg_with_orientation(
        &jpeg,
        &RgbaImage::from_pixel(4, 3, Rgba([10, 20, 30, 255])),
        1,
    );
    let dest = dir.path().join("dest");

    let result =
        import_files(&[png.clone(), jpg, jpeg], &dest, ImportMode::Copy).unwrap();

    assert_eq!(result.imported.len(), 1);
    assert_eq!(result.skipped, 2);
    assert!(result.issues.is_empty());
    assert!(dest.join(png.file_name().unwrap()).exists());
}

#[test]
fn folder_scan_ignores_jpeg_sources() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("photos");
    fs::create_dir_all(&root).unwrap();
    let png = root.join("VRChat_2026-10-21_21-14-03.221_4x3.png");
    write_png(&png);
    crate::image_decode::test_support::write_jpeg_with_orientation(
        &root.join("Steam_2026-10-21_21-14-04.221_4x3.jpg"),
        &RgbaImage::from_pixel(4, 3, Rgba([10, 20, 30, 255])),
        1,
    );

    let scan = collect_folder_sources(&root, None).unwrap();
    assert_eq!(scan.files, vec![png]);
    assert!(scan.issues.is_empty());
}
```

- [x] **Step 2: 取り込みテストが通ることを確認する**

Run: `cargo test -p vrc-photon-core importer::tests`
Expected: PASS

---

### Task 4: 監視が JPG を拾わないことを固定する

**Files:**
- Modify: `crates/daemon/src/tests.rs`
- Test: `poll_watchers_queues_removals_and_ignores_derivative_creates` を拡張する

- [x] **Step 1: 監視テストに JPG の作成イベントを追加する**

`photo_dir` に次のパスを足し、作成イベントを送る。

```rust
let jpeg = photo_dir.join("Steam_2026-10-21_21-14-03.221_4x3.jpg");
tx.send(Ok(notify::Event::new(notify::EventKind::Create(
    notify::event::CreateKind::File,
))
.add_path(jpeg.clone())))
    .unwrap();
```

`daemon.poll_watchers()` の後、次を追加する。

```rust
assert!(!daemon.dirty_paths.contains(&jpeg));
```

既存の削除イベント（`removed` / `old`）が `dirty_paths` に入ることは維持する。

- [x] **Step 2: 監視テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon poll_watchers_queues_removals_and_ignores_derivative_creates`
Expected: PASS

---

### Task 5: 取り込みフィルタと文言を PNG に合わせる

**Files:**
- Modify: `crates/ui/src/app/import.rs`
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`
- Test: `crates/ui/src/app/import.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- Produces: `fn import_dialog_filters() -> Vec<(String, Vec<String>)>`

- [x] **Step 1: フィルタを組み立てる関数を追加し、`pick_import_files` から使う**

```rust
fn import_dialog_filters() -> Vec<(String, Vec<String>)> {
    vec![(tr!("import.dialog.filter.png"), vec!["png".to_string()])]
}
```

`pick_import_files` の `filters:` は `import_dialog_filters(),` に置き換える。

- [x] **Step 2: フィルタの拡張子を固定するテストを追加する**

```rust
#[test]
fn import_filter_accepts_only_png() {
    let filters = import_dialog_filters();
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0].1, vec!["png".to_string()]);
}
```

テストモジュールの `use super::{...}` に `import_dialog_filters` を追加する。

- [x] **Step 3: 翻訳キーを改名する**

`crates/i18n/locales/ja.yml`:

```yaml
import.dialog.filter.png: "PNG 画像"
```

`crates/i18n/locales/en.yml`:

```yaml
import.dialog.filter.png: "PNG images"
```

古い `import.dialog.filter.images` は両方から削除する。`tr_args!` を使わないため
`crates/i18n/tests/placeholders.rs` の変更は不要。

- [x] **Step 4: UI テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ui import_filter_accepts_only_png`
Expected: PASS

---

### Task 6: 全体検証

- [x] **Step 1: フォーマット**

Run: `cargo fmt --all -- --check`
Expected: 差分なし

- [x] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 警告なし

- [x] **Step 3: テスト**

Run: `cargo test --workspace`
Expected: 全件 PASS

- [x] **Step 4: 差分を確認する**

Run: `git diff --stat`
Expected: `photo_filename.rs`、`scanner.rs`、`importer.rs`、`daemon/tests.rs`、
`ui/app/import.rs`、`ja.yml`、`en.yml` のみ。計画・spec ファイルを含めない。

---

### Task 7: PR と CI

- [x] **Step 1: ブランチを push し、`enhancement` ラベル付きの PR を作る**

```bash
git push -u origin feat/681-png-only-import
gh pr create --title "feat: restrict photo import to PNG (#681)" \
  --body-file /tmp/opencode/pr-681.md \
  --label enhancement
```

PR 本文には次を書く。

- 対象を PNG のみにしたこと
- 既存 DB の JPG 行は移行しないこと
- 圧縮生成物の derivative 除外は今回の対象外であること
- 検証コマンドと結果

- [x] **Step 2: `ci-watch` スキルに従って CI を監視する**

- [x] **Step 3: 全チェックが green になったら merge し、`main` に戻って pull する**

- [x] **Step 4: マージ済みブランチと worktree を片付ける**

---

### Task 8: ドキュメント更新（コード PR とは別の docs PR）

**Files:**
- Modify: `docs/features.md`
- Modify: `docs/troubleshooting.md`
- Modify: `docs/architecture.md`
- Modify: `docs/testing.md`
- Modify: この計画ファイル（完了時にチェックボックスを `- [x]` にする）

- [x] **Step 1: `docs/features.md` を更新する**

- §1 の対象を `.png` のみに変更し、JPG/JPEG は走査・監視・取り込みの対象外
  （Steam のスクリーンショットなど）であることを書く。
- §4 に「取り込み対象は PNG のみ。ファイル選択フィルタも PNG に限定し、
  JPG/JPEG はスキップする」を追加する。

- [x] **Step 2: `docs/troubleshooting.md` を更新する**

§2 の「対応拡張子は `.png` / `.jpg` / `.jpeg` のみ」を
「対応拡張子は `.png` のみ。JPG/JPEG は対象外」に変更する。

- [x] **Step 3: `docs/architecture.md` を更新する**

§4.6 に、ファイル選択とフォルダ走査の対象が PNG のみであることを追加する。

- [x] **Step 4: `docs/testing.md` を更新する**

`scanner.rs`、`photo_filename.rs`、`importer.rs` の説明を
「PNG のみ」「JPG/JPEG 除外」を含む内容に合わせる。

- [x] **Step 5: docs PR を作り、CI green を確認して merge する**

タイトル例: `docs: document PNG-only import (#681)`。
