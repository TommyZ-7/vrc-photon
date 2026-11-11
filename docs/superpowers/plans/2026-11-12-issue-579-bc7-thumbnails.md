# サムネイルの BC7 化と Texture2DArray 表示 (#579) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** サムネイルを 16:9 の BC7 ファイルとして保存し、gpui-fast の 512 レイヤー Texture2DArray へ直接アップロードして表示する。CPU のデコード済み画像キャッシュと画質設定を削除する。

**Architecture:** core は EXIF 回転適用後の画像を必要なら 90°回転し、16:9 キャンバスへ引き伸ばして `intel_tex_2` で BC7 へエンコードする。UI は `.bc7` ブロックを 512 レイヤーの単一配列へアップロードし、写真 ID ごとにレイヤーを LRU で再利用する。gpui-fast には専用の Texture2DArray リソース、プリミティブ、wgpu / Direct3D 11 の描画経路を追加する。macOS はバックエンド未登録のエラー経路を使う。

**Tech Stack:** Rust 2024, gpui-kit 0.9.1, gpui-fast fork, wgpu, Direct3D 11, `intel_tex_2` 0.5.0, SQLite, rust-i18n

**Spec:** `docs/superpowers/specs/2026-11-12-issue-579-bc7-thumbnails-design.md`

**Mock:** `docs/ui-579-thumbnail-settings-mockup.html`

**Status:** 承認済み（2026-11-12）

## Global Constraints

- キャンバスは 256×144 / 384×216（既定）/ 512×288。高さは `width * 9 / 16`。
- BC7 は 1 B/px。1 枚は 36,864 / 82,944 / 147,456 B。512 レイヤー配列は 18 / 40.5 / 72 MiB。
- 配列は 512 レイヤー固定の 1 本。動的に増やさない。超過分は LRU で再利用する。
- UI はデコード済みサムネイル画像を保持しない。evict されたら `.bc7` を読み直す。
- サムネイル画質設定は削除する。旧 WebP からの移行は行わない。
- BC7 非対応環境のフォールバックは作らない。起動時に明確なエラーで停止する。
- グリッドカードは 3:2 固定。表示比率と回転は DB の `photo.effective_dimensions()` を基準にする。
- fork の変更は `TommyZ-7/gpui-fast` の独立 PR。docs と実装は別 PR。
- 既存の起動時サムネイルバッチ（最新 5,000 件）は維持する。
- `thumbnail_cache_mb` の既定を 8 GB へ変更する。384×216 で約 100,000 枚を保持できる。
- 設定・UI 文言は `crates/i18n/locales/ja.yml` と `en.yml` の両方へ追加・削除する。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- CI が green になるまで merge しない。計画のチェックボックスは作業完了時に `- [x]` へ更新する。

## Review Focus

1. **縦向きの向き**: 9:16 の写真がグリッド、ビューア、ミニマップで上下反転・左右反転せず正しい向きに出るか。
2. **非 16:9 の比率**: 4:3 / 3:4 / パノラマがグリッドの `Cover` とビューアのフレームで引き伸ばされないか。
3. **レイヤー再利用**: 512 枚を超えてスクロールした後、古いカード位置に別の写真が出ないか。
4. **解像度変更**: 旧寸法のファイル・配列・キャッシュが残らず、表示分が新しい寸法で再生成されるか。
5. **非対応環境**: BC7 非対応時に通常 UI へ落ちず、GPU 情報付きのエラー画面で停止するか。

---

## Phase 0: 設計・モック・docs PR

### Task 1: 設計・モック・計画を docs PR にまとめる

**Files:**
- Create: `docs/superpowers/specs/2026-11-12-issue-579-bc7-thumbnails-design.md`
- Create: `docs/superpowers/plans/2026-11-12-issue-579-bc7-thumbnails.md`
- Create: `docs/ui-579-thumbnail-settings-mockup.html`
- Modify: `docs/ui.md`
- Modify: `docs/licenses.md`

**Interfaces:**
- Consumes: なし
- Produces: 承認対象の設計・モック・計画、更新済み `docs/ui.md` / `docs/licenses.md`

- [x] **Step 1: 設計書を書く**

`docs/superpowers/specs/2026-11-12-issue-579-bc7-thumbnails-design.md` を作成する。決定事項 Q1–Q8、I1–I12、保存形式、表示、fork API、リスク、計測を含める。

- [x] **Step 2: 設定画面のモックを作る**

`docs/ui-579-thumbnail-settings-mockup.html` を作成する。変更前の画質スライダー付きカードと、変更後の 256×144 / 384×216 / 512×288 ボタン、再生成ヒントを並べる。

- [x] **Step 3: `docs/ui.md` を更新する**

サムネイルの保存形式、16:9 キャンバス、512 レイヤー、CPU キャッシュなし、設定画面の画質削除、デモの BC7 化を反映する。

- [x] **Step 4: ライセンス通知を更新する**

`docs/licenses.md` に `intel_tex_2` / Intel ISPC Texture Compressor の行と取得元・ライセンスを追加する。`THIRD-PARTY-NOTICES.md` は実装 PR で `cargo about` から再生成する。

- [x] **Step 5: この計画を書く**

Phase 1–4 のタスク、ファイル、インターフェース、テスト、検証コマンドを記載する。

- [x] **Step 6: docs をコミットして PR を出す**

```bash
git add docs/superpowers/specs/2026-11-12-issue-579-bc7-thumbnails-design.md \
  docs/superpowers/plans/2026-11-12-issue-579-bc7-thumbnails.md \
  docs/ui-579-thumbnail-settings-mockup.html docs/ui.md docs/licenses.md
git commit -m "docs: #579 サムネイルの BC7 化と Texture2DArray 表示の設計・モック"
git push -u origin docs/579-bc7-thumbnails
gh pr create --title "docs: #579 サムネイルの BC7 化と Texture2DArray 表示の設計・モック" \
  --body "Refs #579" --label documentation
```

- [x] **Step 7: モックと設計の承認を得る**

2026-11-12 に承認。I10–I12 の回答を反映し、docs PR を CI green 後に merge する。

### Task 2: 承認ゲート

**Files:** なし

**Interfaces:**
- Consumes: Task 1 の設計・モック
- Produces: 実装開始の承認

- [x] **Step 1: ユーザーの承認を確認する**

Q1–Q8、I10–I12、設定画面モック、設計、計画の承認を確認した。

- [x] **Step 2: 設計変更があれば反映する**

I12（`thumbnail_cache_mb` の既定 8 GB）を設計・計画へ反映した。追加の変更はない。

---

## Phase 1: gpui-fast fork

### Task 3: Texture2DArray の公開 API と CPU メタデータ

**Files:**
- Create: `gpui/src/fast/texture_arrays.rs`
- Modify: `gpui/src/fast/mod.rs`
- Modify: `gpui/src/window.rs`
- Modify: `gpui/src/lib.rs`
- Test: `gpui/src/fast/texture_arrays.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: なし
- Produces:
  - `pub struct TextureArrayId(u32)`
  - `pub enum TextureArrayFormat { Bc7RgbaUnorm }`
  - `pub struct TextureArray { id, size, layers, format }`
  - `pub enum TextureArrayUv { Normal, Rotate90 }`
  - `Window::create_texture_array(size, layers, format) -> Result<TextureArray>`
  - `Window::upload_texture_array_layer(array, layer, blocks) -> Result<()>`
  - `Window::paint_texture_array_layer(bounds, image_bounds, array, layer, uv, corner_radii, opacity) -> Result<()>`
  - `Window::remove_texture_array(array)`
  - `set_texture_array_platform(Arc<dyn TextureArrayPlatform>)`（テストと各バックエンド用）

- [ ] **Step 1: 失敗するテストを書く**

テスト名と検証:
- `create_rejects_zero_layers`
- `create_rejects_zero_size`
- `upload_rejects_layer_out_of_range`
- `upload_rejects_wrong_bc7_length`
- `upload_advances_only_the_target_layer_version`
- `create_without_a_platform_reports_unsupported`

`TextureArrayPlatform` のフェイクをテスト内で登録し、`create` / `upload` / `remove` の呼び出し回数を確認する。

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p gpui texture_arrays -- --nocapture`
Expected: FAIL（型・関数が未定義）

- [ ] **Step 3: 最小実装を書く**

レイヤー数・寸法・BC7 期待長を検証する。BC7 の期待長は `ceil(width/4) * ceil(height/4) * 16`。レイヤー版番号を `AtomicU32` で保持し、アップロード成功時に対象レイヤーだけ進める。プラットフォーム未登録時は `"texture arrays are not supported on this platform"` を返す。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p gpui texture_arrays -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add gpui/src/fast/texture_arrays.rs gpui/src/fast/mod.rs gpui/src/window.rs gpui/src/lib.rs
git commit -m "feat(gpui): add texture array resources and window API"
```

### Task 4: プリミティブ、バッチ、タイル処理への統合

**Files:**
- Modify: `gpui/src/scene.rs`
- Modify: `gpui/src/fast/layers/paint.rs`
- Modify: `gpui/src/fast/layers/tiles.rs`
- Modify: `gpui/src/fast/layers/scene.rs`
- Modify: `gpui/src/fast/rounded_clip_scene.rs`
- Test: 各ファイルの `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 3 の `TextureArrayId` / `TextureArrayUv`
- Produces:
  - `pub struct TextureArraySprite { order, array_id, layer, version, bounds, content_mask, uv_origin, uv_size, uv_transform, corner_radii, opacity }`
  - `Primitive::TextureArraySprite`
  - `PrimitiveBatch::TextureArraySprites { array_id, range }`

- [ ] **Step 1: 失敗するテストを書く**

- `scene_batches_adjacent_texture_array_sprites_by_array`
- `texture_array_tile_hash_changes_when_the_layer_version_changes`
- `texture_array_primitive_translates_with_the_scene`
- `texture_array_primitive_clips_to_the_content_mask`
- `rounded_clip_scene_includes_texture_array_sprites`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p gpui texture_array -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

`Primitive` の `bounds` / `content_mask`、`BatchIterator`、`hash_primitive`、`visible_bounds` / `drawn_bounds`、`translate_primitive`、`clip_primitive`、`rounded_clip_scene` に腕を追加する。`version` をタイルハッシュへ含める。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p gpui texture_array -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add gpui/src/scene.rs gpui/src/fast/layers gpui/src/fast/rounded_clip_scene.rs
git commit -m "feat(gpui): add texture array sprite primitive and batching"
```

### Task 5: wgpu バックエンド（Linux）

**Files:**
- Create: `gpui_wgpu/src/fast/texture_arrays.rs`
- Modify: `gpui_wgpu/src/fast/mod.rs`
- Modify: `gpui_wgpu/src/wgpu_renderer.rs`
- Modify: `gpui_wgpu/src/shaders.wgsl`
- Modify: `gpui_wgpu/src/fast/frame.rs`
- Test: `gpui_wgpu/src/fast/texture_arrays.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 3–4 の API とプリミティブ
- Produces: Linux で BC7 配列を作成・アップロード・描画できる `TextureArrayPlatform` 実装

- [ ] **Step 1: 失敗するテストを書く**

- `upload_writes_bc7_blocks_to_the_requested_layer`
- `render_samples_the_requested_array_layer`
- `render_applies_rotate90_uv_transform`
- `render_keeps_corner_radius_and_opacity`

ヘッドレスレンダラで 2 レイヤーに既知の BC7 ブロックを入れ、指定座標のピクセルを確認する。

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p gpui_wgpu texture_arrays --features test-support -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

- デバイス作成時に `TEXTURE_COMPRESSION_BC` が利用可能なら要求する。
- `Bc7RgbaUnorm` の配列テクスチャを作る。
- `Queue::write_texture` で `origin.z = layer`、`bytes_per_row = ceil(width/4) * 16`、`rows_per_image = ceil(height/4)`。
- 新しい WGSL の頂点／フラグメントシェーダ、パイプライン、配列ごとのバインドグループを追加する。
- `fast/frame.rs` のインスタンスアップロード、バッチ計画、描画に `TextureArraySprites` を追加する。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p gpui_wgpu texture_arrays --features test-support -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add gpui_wgpu/src
git commit -m "feat(wgpu): render BC7 texture arrays"
```

### Task 6: Direct3D 11 バックエンド（Windows）

**Files:**
- Create: `gpui_windows/src/directx_texture_arrays.rs`
- Modify: `gpui_windows/src/directx_renderer.rs`
- Modify: `gpui_windows/src/shaders.hlsl`
- Modify: `gpui_windows/src/fast/frame.rs`
- Test: `gpui_windows/src/directx_texture_arrays.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 3–4 の API とプリミティブ
- Produces: Windows で BC7 配列を作成・アップロード・描画できる `TextureArrayPlatform` 実装

- [ ] **Step 1: 失敗するテストを書く**

- `bc7_row_and_depth_pitch_match_the_canvas`
- `layer_box_covers_one_array_slice`
- `format_support_rejects_adapters_without_bc7`

ピッチ計算と `D3D11_BOX` の生成を純粋関数へ切り出してテストする。

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p gpui_windows directx_texture_arrays -- --nocapture`
Expected: FAIL（Windows 実機または Windows CI で実行）

- [ ] **Step 3: 最小実装を書く**

- `DXGI_FORMAT_BC7_UNORM`、`ArraySize = layers`、`MipLevels = 1` の `Texture2D` を作る。
- `CheckFormatSupport` で BC7 / Texture2D を確認する。
- `UpdateSubresource` で `D3D11_BOX { front: layer, back: layer + 1 }` を書き込む。
- HLSL に `Texture2DArray<float4>` の頂点／ピクセルシェーダと `PipelineState<TextureArraySprite>` を追加する。
- `fast/frame.rs` のバッチ計画と描画へ追加する。

- [ ] **Step 4: コンパイルとテストを確認する**

Run（Windows 実機または CI）: `cargo test -p gpui_windows directx_texture_arrays -- --nocapture`
Expected: PASS

Linux からは `rustup target add x86_64-pc-windows-msvc` と Windows SDK が必要になるため、
本体 PR の `windows-latest` CI で `cargo check --workspace --all-targets` を通す。

- [ ] **Step 5: コミット**

```bash
git add gpui_windows/src
git commit -m "feat(windows): render BC7 texture arrays"
```

### Task 7: 未対応プラットフォーム、fork PR、マージ

**Files:**
- Modify: `gpui/src/fast/texture_arrays.rs`（未対応エラーのテスト）
- Modify: `docs/`（fork 側の必要な説明があれば）

**Interfaces:**
- Consumes: Task 3–6
- Produces: マージ済み fork コミット SHA

- [ ] **Step 1: 未対応プラットフォームのテストを追加する**

macOS/Metal ではバックエンドを登録しないため、`create_texture_array` が
`"texture arrays are not supported on this platform"` を返すことを確認する。
`gpui_apple` にシェーダやパイプラインは追加しない。

- [ ] **Step 2: fork の全体検証を実行する**

```bash
cargo fmt --all -- --check
cargo clippy -p gpui -p gpui_wgpu --all-targets -- -D warnings
cargo test -p gpui
cargo test -p gpui_wgpu --features test-support
```

- [ ] **Step 3: ブランチを push して PR を出す**

`vrc-0.1.14-atlas-1536` から `vrc-0.1.14-texture2d-array` を切る。

```bash
git push -u origin vrc-0.1.14-texture2d-array
gh pr create -R TommyZ-7/gpui-fast --base vrc-0.1.14-atlas-1536 \
  --title "feat: add BC7 texture array rendering" \
  --body "Adds a 512-layer BC7 Texture2DArray path for thumbnail rendering. wgpu + Direct3D 11; unsupported platforms report an error."
```

- [ ] **Step 4: CI / レビューを確認してマージする**

PR のチェックと差分を確認し、ユーザーの指示に従ってマージする。マージコミット SHA を記録する。

- [ ] **Step 5: コミット**

```bash
git commit -m "docs: record the merged gpui-fast texture array revision"
```

---

## Phase 2: core / daemon

### Task 8: `intel_tex_2` と BC7 エンコーダ

**Files:**
- Modify: `crates/core/Cargo.toml`
- Create: `crates/core/src/bc7.rs`
- Modify: `crates/core/src/lib.rs`
- Test: `crates/core/src/bc7.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: なし
- Produces:
  - `pub fn encoded_len(width: u32, height: u32) -> Option<usize>`
  - `pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, Bc7Error>`
  - `pub fn is_valid_canvas(width: u32, height: u32) -> bool`

- [ ] **Step 1: 失敗するテストを書く**

- `bc7_length_is_one_byte_per_pixel`
- `encode_rejects_non_multiple_of_four_dimensions`
- `encode_rejects_a_short_rgba_buffer`
- `encode_returns_the_expected_block_count`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core bc7 -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

`intel_tex_2::bc7::{compress_blocks, opaque_basic_settings}` と `RgbaSurface` を使う。
`stride = width * 4`。入力長と 4 の倍数を検証し、失敗は `Bc7Error` で返す。
`unsafe` は `intel_tex_2` の安全ラッパー内に閉じる。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core bc7 -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/core/Cargo.toml crates/core/src/bc7.rs crates/core/src/lib.rs
git commit -m "feat(core): add BC7 thumbnail encoder"
```

### Task 9: 16:9 キャンバス生成（回転・引き伸ばし）

**Files:**
- Modify: `crates/core/src/thumbnail.rs`
- Modify: `crates/core/src/image_decode.rs`
- Test: `crates/core/src/thumbnail.rs` の `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 8 の `encode_rgba`
- Produces:
  - `pub fn thumbnail_canvas(width: u32) -> (u32, u32)`
  - `fn build_thumbnail_canvas(image: RgbaImage) -> (RgbaImage, bool)`（回転フラグ付き）

- [ ] **Step 1: 失敗するテストを書く**

- `canvas_dimensions_are_16_by_9`
- `portrait_photo_is_rotated_and_stretched_to_the_canvas`
- `landscape_photo_is_stretched_without_cropping`
- `square_photo_is_not_rotated`
- `stretch_keeps_the_full_photo_at_the_corners`

4:3 の四隅に異なる色を置き、保存後の四隅が同じ位置関係で残ることを確認する。

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core thumbnail_canvas -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

EXIF 適用後の `RgbaImage` を受け、`height > width` なら
`image::imageops::rotate270`（反時計回り）で横向きにする。
`image::imageops::resize(..., FilterType::Triangle)` でキャンバス寸法へ引き伸ばす。
切り抜き・余白追加はしない。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core thumbnail_canvas -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/core/src/thumbnail.rs crates/core/src/image_decode.rs
git commit -m "feat(core): render thumbnails onto a 16:9 canvas"
```

### Task 10: `.bc7` 保存、メタデータ、読み込み

**Files:**
- Modify: `crates/core/src/thumbnail.rs`
- Modify: `crates/core/src/cache_metadata.rs`
- Test: `crates/core/src/thumbnail.rs` / `crates/core/src/cache_metadata.rs`

**Interfaces:**
- Consumes: Task 8–9
- Produces:
  - `ThumbnailSpec { size: u32 }`（品質なし）
  - `EnsuredThumbnail { path, created, bytes, bc7: Option<Vec<u8>>, color: u32, rotated: bool }`
  - `pub fn read_thumbnail_blocks(path: &Path, spec: ThumbnailSpec) -> Result<Vec<u8>>`
  - `thumbnail_path_in` は `<id>-s<w>x<h>.bc7`

- [ ] **Step 1: 失敗するテストを書く**

- `thumbnail_file_name_uses_the_canvas_dimensions`
- `generated_file_has_the_bc7_block_length`
- `metadata_v2_round_trips_color_and_rotation`
- `metadata_v1_forces_one_regeneration`
- `corrupt_bc7_length_is_regenerated_once`
- `cache_hit_does_not_return_blocks_from_the_generator`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core thumbnail -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

- `.bc7` はヘッダなしの BC7 ブロック列。
- `.meta` は `v2 <mtime_secs> <mtime_nanos> <size> <color> <rotated>`。
- 生成時だけ `bc7: Some(blocks)` を返す。キャッシュヒットは `None`。
- `read_thumbnail_blocks` はファイル長を期待値と照合し、不一致をエラーにする。
- 既存の `write_bytes_no_clobber` と temp 名を使う。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core thumbnail -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/core/src/thumbnail.rs crates/core/src/cache_metadata.rs
git commit -m "feat(core): store thumbnails as raw BC7 blocks"
```

### Task 11: キャッシュの使用量・LRU・全削除・旧 WebP

**Files:**
- Modify: `crates/core/src/thumbnail.rs`
- Test: `crates/core/src/thumbnail.rs`

**Interfaces:**
- Consumes: Task 10
- Produces: `.bc7` を対象にした `thumbnail_cache_usage_in` / `prune_thumbnails_in` / `clear_thumbnail_cache_in` / `remove_thumbnail_files_in`

- [ ] **Step 1: 失敗するテストを書く**

- `usage_counts_only_bc7_entries`
- `prune_removes_oldest_bc7_and_its_meta`
- `clear_removes_bc7_legacy_webp_and_temps`
- `remove_for_one_photo_removes_every_bc7_variant`
- `legacy_webp_is_not_counted_as_an_active_entry`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core thumbnail_cache -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

ファイル名の判定を `.bc7` / `.bc7.meta` / `.bc7.tmp-*` へ広げる。
旧 `.webp` は全削除の対象にだけ含める。使用量と LRU の対象は `.bc7` のみ。
写真単位の削除は全バリアントとサイドカーを消す。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core thumbnail_cache -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/core/src/thumbnail.rs
git commit -m "feat(core): manage the BC7 thumbnail cache"
```

### Task 12: 設定から画質を削除し、寸法を正規化する

**Files:**
- Modify: `crates/core/src/settings.rs`
- Test: `crates/core/src/settings.rs`

**Interfaces:**
- Consumes: Task 10 の `ThumbnailSpec`
- Produces:
  - `AppSettings` から `thumbnail_quality` を削除
  - `thumbnail_size` は幅 256 / 384 / 512、既定 384
  - `thumbnail_spec()` は `ThumbnailSpec { size }`

- [ ] **Step 1: 失敗するテストを書く**

- `thumbnail_quality_is_no_longer_a_setting`
- `thumbnail_size_defaults_to_384`
- `thumbnail_size_normalizes_unknown_values_to_384`
- `legacy_thumbnail_size_values_map_to_widths`
- `unknown_quality_key_in_json_is_ignored`
- `thumbnail_cache_default_is_8_gb`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-core settings:: -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

`thumbnail_quality` と `THUMBNAIL_QUALITY_*` を削除する。`default_thumbnail_size` を 384 にする。既存の 256 / 384 / 512 は幅として維持し、未知値は 384。`thumbnail_cache_mb` の既定を 8192 にする。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-core settings:: -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/core/src/settings.rs
git commit -m "feat(core): remove the thumbnail quality setting"
```

### Task 13: デーモンの spec・解像度変更・旧 WebP 削除

**Files:**
- Modify: `crates/daemon/src/thumbnails.rs`
- Modify: `crates/daemon/src/lifecycle.rs`
- Modify: `crates/daemon/src/tests.rs`
- Test: `crates/daemon/src/tests.rs`

**Interfaces:**
- Consumes: Task 10–12
- Produces:
  - `pack_thumbnail_spec` / `unpack_thumbnail_spec` は幅のみ
  - 解像度変更時に `clear_thumbnail_cache_in` を呼ぶ
  - 起動時に旧 `.webp` を削除する

- [ ] **Step 1: 失敗するテストを書く**

- `thumbnail_spec_pack_round_trips_the_width`
- `changing_the_thumbnail_size_clears_the_cache`
- `unchanged_size_does_not_clear_the_cache`
- `startup_removes_legacy_webp_entries`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-daemon thumbnail -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

`refresh_compression_settings` で `thumbnail_size` の変化を検出し、DB ロックを離してから全削除する。`thumbnail_cache_epoch` を進める。起動時の旧 WebP 削除は失敗をログして続行する。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-daemon thumbnail -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/daemon/src/thumbnails.rs crates/daemon/src/lifecycle.rs crates/daemon/src/tests.rs
git commit -m "feat(daemon): reset thumbnail caches on resolution changes"
```

---

## Phase 3: UI

### Task 14: ThumbnailCache を 512 レイヤー LRU へ置き換える

**Files:**
- Modify: `crates/ui/src/state/thumbnail.rs`
- Test: `crates/ui/src/state/thumbnail.rs`

**Interfaces:**
- Consumes: Task 10 の `read_thumbnail_blocks`、Task 3 の `TextureArray`
- Produces:
  - `pub(crate) struct ThumbnailHandle { array: TextureArray, layer: u32, rotated: bool }`
  - `ThumbnailCache::get(id) -> Option<ThumbnailHandle>`
  - `ThumbnailCache::insert_layer(id, array, layer, color, rotated) -> bool`
  - `ThumbnailCache::take_pending_upload() -> Option<PendingThumbnailUpload>`
  - `ThumbnailCache::clear() / invalidate(id) / remove_photo(id) / pin(id) / unpin(id)`

- [ ] **Step 1: 失敗するテストを書く**

- `allocates_layers_from_zero_to_511`
- `evicts_the_least_recently_used_layer_after_512_entries`
- `never_evicts_a_pinned_layer`
- `reuses_a_freed_layer`
- `clear_drops_every_layer_and_bumps_the_generation`
- `invalidate_and_remove_photo_release_the_layer`
- `a_stale_pending_upload_is_rejected_by_generation`
- `cache_holds_no_render_images`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui state::thumbnail -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

`images` / `image_bytes` / `total_bytes` / `take_removed` を削除する。`layers: Vec<Option<LayerEntry>>` を 512 で初期化し、free list と recency を持つ。`pinned` は `HashSet<i64>` に広げる。`THUMB_CACHE_BYTES` / `THUMB_CACHE_ENTRIES` を削除する。`THUMB_IN_FLIGHT` と失敗再試行は維持する。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ui state::thumbnail -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/state/thumbnail.rs
git commit -m "refactor(ui): cache thumbnails as texture array layers"
```

### Task 15: 配列のライフサイクルと起動エラー

**Files:**
- Modify: `crates/ui/src/app.rs`
- Modify: `crates/ui/src/app/photos.rs`
- Test: `crates/ui/src/app/photos.rs` / `crates/ui/src/app.rs`

**Interfaces:**
- Consumes: Task 14
- Produces:
  - `App::thumbnail_array: Option<TextureArray>`
  - `App::thumbnail_array_error: Option<String>`
  - `App::pending_array_drop: Option<TextureArray>`
  - 最初の描画で配列を作成し、失敗したら `App::render_fatal_thumbnail_error`

- [ ] **Step 1: 失敗するテストを書く**

- `thumbnail_array_is_created_once_for_the_current_spec`
- `resolution_change_drops_the_old_array`
- `array_creation_failure_sets_the_fatal_error`
- `pending_bc7_upload_is_applied_on_the_next_render`
- `a_pending_upload_from_the_old_spec_is_discarded`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui thumbnail_array -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

- `request_thumbnail` は `read_thumbnail_blocks` の結果を `PendingThumbnailUpload` として保持する。
- `Render for App` の先頭で配列を作成・アップロードする。
- `create_texture_array` のエラーは GPU 名・バックエンド・不足機能を含む文字列にする。
- エラー時は通常のシェルを描画せず、エラー画面と終了ボタンだけを出す。
- `clear` 時は旧配列を `pending_array_drop` に積み、次の描画で `remove_texture_array` する。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ui thumbnail_array -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/app.rs crates/ui/src/app/photos.rs
git commit -m "feat(ui): own the thumbnail texture array lifecycle"
```

### Task 16: グリッド、インスペクタ、ビューアの表示置き換え

**Files:**
- Modify: `crates/ui/src/ui/gallery.rs`
- Modify: `crates/ui/src/ui/library.rs`
- Modify: `crates/ui/src/ui/viewer.rs`
- Modify: `crates/ui/src/app/viewer.rs`
- Modify: `crates/ui/src/state/viewer.rs`
- Test: `crates/ui/src/state/viewer.rs` / `crates/ui/src/ui/theme.rs`

**Interfaces:**
- Consumes: Task 14–15 の `ThumbnailHandle`
- Produces:
  - `texture_array_img(array, layer)` を使う表示
  - `ViewerStill { Full(Arc<RenderImage>), Thumbnail { handle, dimensions } }`
  - `ViewerMorph::thumbnail: Option<ViewerStill>`
  - 遷移中の写真 ID を pin / unpin する

- [ ] **Step 1: 失敗するテストを書く**

- `thumbnail_fit_uses_the_photo_aspect_not_the_canvas_aspect`
- `portrait_thumbnail_rotates_the_uv_rectangle`
- `viewer_still_thumbnail_pins_and_unpins_its_photo`
- `viewer_morph_keeps_a_thumbnail_still_when_the_full_image_is_missing`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui viewer -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

- `render_card` は `texture_array_img(handle.array, handle.layer).content_size(photo.effective_dimensions()).rotated(handle.rotated).object_fit(Cover)` を使う。
- インスペクタ、セッション表紙、ミニマップは `Cover`。
- ビューアのフォールバックは `Fill` 相当で写真アスペクトのフレームへ描く。
- `ViewerMorph` の `thumbnail` を `ViewerStill` に変える。
- 前後送りの開始時に遷移元 ID を `pin` し、モーフ終了時に `unpin` する。
- pin が間に合わない場合はクロスフェードを省略する。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ui viewer -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/ui/gallery.rs crates/ui/src/ui/library.rs crates/ui/src/ui/viewer.rs \
  crates/ui/src/app/viewer.rs crates/ui/src/state/viewer.rs
git commit -m "feat(ui): draw thumbnails from the BC7 texture array"
```

### Task 17: 設定画面と i18n

**Files:**
- Modify: `crates/ui/src/ui/settings.rs`
- Modify: `crates/ui/src/app.rs`
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`
- Test: `crates/ui/src/ui/settings.rs` / `crates/i18n/tests`

**Interfaces:**
- Consumes: Task 12–13
- Produces: 画質スライダーのないサムネイルカード、寸法ボタン、再生成ヒント

- [ ] **Step 1: 失敗するテストを書く**

- `thumbnail_quality_slider_is_not_constructed`
- `thumbnail_size_buttons_show_width_and_height`
- `changing_the_thumbnail_size_clears_the_local_cache`
- `ja_and_en_have_the_regeneration_hint`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui settings -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

`thumbnail_quality_slider` と subscription を削除する。サイズボタンは `256×144` / `384×216` / `512×288`。ヒント `settings.thumbnail.regenerate_hint` を追加し、`settings.thumbnail.quality` を削除する。変更時はローカルキャッシュと配列を破棄する。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ui settings -- --nocapture && cargo test -p vrc-photon-i18n`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/ui/settings.rs crates/ui/src/app.rs crates/i18n/locales/ja.yml crates/i18n/locales/en.yml
git commit -m "feat(ui): remove thumbnail quality and show canvas sizes"
```

### Task 18: デモデータを BC7 へ

**Files:**
- Modify: `crates/ui/src/app/demo.rs`
- Test: `crates/ui/src/app/demo.rs`

**Interfaces:**
- Consumes: Task 8–10
- Produces: `.bc7` のデモサムネイル、`DEMO_DATA_VERSION = 5`

- [ ] **Step 1: 失敗するテストを書く**

- `demo_thumb_path_uses_bc7`
- `demo_fixture_writes_bc7_files`
- `demo_data_version_bump_rebuilds_old_fixtures`

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-ui demo -- --nocapture`
Expected: FAIL

- [ ] **Step 3: 最小実装を書く**

デモ画像を 384×216 へ引き伸ばし、`bc7::encode_rgba` で `.bc7` を書く。`DEMO_DATA_VERSION` を 5 に上げる。旧 PNG サムネイルは再生成時に消える。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-ui demo -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/ui/src/app/demo.rs
git commit -m "feat(demo): generate BC7 demo thumbnails"
```

---

## Phase 4: 統合・計測・リリース

### Task 19: gpui-fast の rev 更新と全体ビルド

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: Task 7 のマージ済み fork SHA
- Produces: 新しい `gpui-fast` / `gpui-fast-wgpu` / `gpui-fast-windows` の rev

- [ ] **Step 1: rev を更新する**

`[patch.crates-io]` の 3 つの `rev` を fork のマージコミットへ揃える。

- [ ] **Step 2: lock を更新してビルドする**

Run: `cargo update -p gpui-fast -p gpui-fast-wgpu -p gpui-fast-windows`
Expected: `Cargo.lock` の 3 エントリが同じ rev になる。

- [ ] **Step 3: コンパイルを確認する**

Run: `cargo check --workspace --all-targets`
Expected: PASS

- [ ] **Step 4: コミット**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: update gpui-fast for texture array rendering"
```

### Task 20: 検証、計測、完了報告

**Files:**
- Modify: `docs/superpowers/plans/2026-11-12-issue-579-bc7-thumbnails.md`
- Modify: `packaging/licenses/THIRD-PARTY-NOTICES.md`

**Interfaces:**
- Consumes: Task 1–19
- Produces: CI green、計測値、チェック済み計画、完了報告

- [ ] **Step 1: ローカル検証を実行する**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] **Step 2: ライセンス通知を再生成する**

```bash
cargo about generate about.hbs -o packaging/licenses/THIRD-PARTY-NOTICES.md
```

`intel_tex_2`、`ispc_rt`、ISPC の帰属が含まれることを確認する。

- [ ] **Step 3: デモと実画面を確認する**

`cargo run -- --demo` でグリッド、検索、インスペクタ、ビューア、ミニマップを確認する。
縦向き、16:9、9:16、4:3、3:4、パノラマを混在させる。解像度を 256 → 384 → 512 と変更し、配列が作り直されることを確認する。512 枚を超えてスクロールし、レイヤー再利用後の残像がないことを確認する。

- [ ] **Step 4: ディスクと VRAM を計測する**

- `du -sb` と `thumbnail_cache_usage` で 1,000 枚あたりのバイト数を記録する。
- 計算値 `512 * ceil(w/4) * ceil(h/4) * 16` を記録する。
- Linux AMD では `/sys/class/drm/card*/device/mem_info_vram_used` の起動前後差を記録する。
- 変更前の WebP + RGBA LRU（64 MiB）との比較を完了報告へ書く。

- [ ] **Step 5: 計画のチェックボックスを更新する**

完了したタスクを `- [x]` にし、計測値を「計測結果」節へ追記する。

- [ ] **Step 6: コミット**

```bash
git add docs/superpowers/plans/2026-11-12-issue-579-bc7-thumbnails.md \
  packaging/licenses/THIRD-PARTY-NOTICES.md
git commit -m "docs: record #579 verification and measurements"
```

### Task 21: 実装 PR、CI、マージ

**Files:**
- すべての変更

**Interfaces:**
- Consumes: Task 1–20
- Produces: マージ済み PR、削除済み作業ブランチ

- [ ] **Step 1: PR を出す**

```bash
git push -u origin feat/579-bc7-thumbnails
gh pr create --title "perf: サムネイルを BC7 化し Texture2DArray で表示する" \
  --body "Closes #579" --label enhancement
```

- [ ] **Step 2: CI と Release bundle-check を監視する**

`ci-watch` の手順で `CI` と `Release` を監視する。`CI` が green になるまでマージしない。
Windows の `cargo check` が通ることを確認する。

- [ ] **Step 3: マージ後の後片付け**

`main` へ戻して pull し、作業ブランチと worktree を削除する。

```bash
git worktree remove ../vrc-photo-manager-579-bc7
git branch -d feat/579-bc7-thumbnails
git push origin --delete feat/579-bc7-thumbnails
```

- [ ] **Step 4: Issue #579 の受け入れ条件を確認する**

- 生成されるサムネイルがすべて設定寸法の BC7 である
- 解像度変更で全サムネイルが再生成され、新しい寸法で表示される
- 縦向き写真がグリッド／クイックルックで正しい向き・比率で表示される
- 16:9 / 9:16 以外の写真が正しい比率で表示される
- ディスク使用量と VRAM が実測で記録されている
- 画質設定が設定画面から消えている

---

## 計測結果

実装後に追記する。

| 項目 | 変更前（WebP + RGBA） | 変更後（BC7 + 512 レイヤー） |
|---|---|---|
| サムネイル 1 枚（384×216） | 未計測 | 81 KiB |
| 1,000 枚のディスク | 未計測 | 未計測 |
| 表示 VRAM（512 レイヤー） | 未計測 | 計算値 40.5 MiB / 実測 未計測 |
| 初回表示の体感 | 未計測 | 未計測 |
