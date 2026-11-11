# サムネイルの BC7 化と Texture2DArray 表示 (#579) 設計

> 状態: ドラフト（承認待ち）
> モック: `docs/ui-579-thumbnail-settings-mockup.html`
> 計画: `docs/superpowers/plans/2026-11-12-issue-579-bc7-thumbnails.md`
> 関連: #575（アトラスの BC7 化・不採用）、#578（メモリキャッシュの BC7 化・不採用）

## 目的

サムネイルを WebP から BC7 に変え、保存時点で GPU がそのまま読める形式にする。
表示は 512 レイヤーの Texture2DArray に BC7 ブロックを直接アップロードし、
デコード・再圧縮・CPU 側の画像キャッシュをなくす。

これにより、グリッドをスクロールしたときの CPU デコードと、RGBA 画像キャッシュ／
アトラスが占めていた VRAM を削減する。代わりに、表示中のサムネイルは
512 枚固定のレイヤーを LRU で再利用する。

## 決定事項

ユーザーが選んだ方針（2026-11-12）。

| # | 判断項目 | 決定 | 補足 |
|---|---|---|---|
| Q1 | gpui-fast 側の方式 | 専用 Texture2DArray リソース＋専用プリミティブ | macOS/Metal は未実装スタブ。fork は独立 PR |
| Q2 | BC7 エンコーダ | `intel_tex_2`（ISPC） | 事前ビルド済み静的ライブラリを使う。C++/ISPC の実行時依存は増やさない |
| Q3 | レイヤー管理 | 512 レイヤー固定の単一配列。超過分は LRU で再利用 | 配列の動的増減はしない |
| Q4 | 解像度変更時 | 旧キャッシュを全削除し、表示分から遅延再生成 | 既存 WebP からの移行支援は不要。旧ファイルは破棄してよい |
| Q5 | 保存形式 | 生の BC7 ブロック＋`.meta` サイドカー | ヘッダなしでゼロコピーアップロード |
| Q6 | レイアウト基準 | グリッドカードは 3:2 固定。DB の表示寸法から回転・カバー矩形を決める | カードの可変アスペクト化はしない |
| Q7 | BC7 非対応環境 | 起動時に明確なエラーで停止 | フォールバックなし |
| Q8 | fork のブランチ運用 | `vrc-0.1.14-atlas-1536` から `vrc-0.1.14-texture2d-array` を切り、そのブランチへ PR | 本体はマージ後の rev をピン留め |

実装者が決めた事項。

| # | 判断項目 | 決定 | 理由 |
|---|---|---|---|
| I1 | キャンバス寸法 | 幅 256 / 384 / 512、高さは `width * 9 / 16` | 仕様の 256×144 / 384×216 / 512×288。既定は 384×216 |
| I2 | BC7 エンコード設定 | `intel_tex_2::bc7::opaque_basic_settings()` | 写真は不透明。速度と画質のバランスを取る。遅ければ fast へ切り替える |
| I3 | 回転方向 | 縦向きは反時計回りに 90°回転して保存 | 表示側は時計回りの UV 変換で戻す。回転フラグはメタデータにも保存する |
| I4 | メタデータ | `v2 <mtime_secs> <mtime_nanos> <size> <color> <rotated>` | 既存の source stamp に平均色と回転を追加する。旧 `v1` は再生成対象 |
| I5 | 平均色 | エンコード前の最終キャンバスから計算して保存 | 再起動後に BC7 をデコードせずプレースホルダー色を出せる |
| I6 | レイヤー再利用の単位 | 写真 ID 単位。選択中の写真は evict しない | 現在の `pinned` と同じ考え方 |
| I7 | 配列の作成タイミング | 最初のサムネイル描画時。設定変更で破棄し、次の描画で再作成 | 起動時にデバイス能力を確認できる |
| I8 | デモデータ | バージョンを上げ、デモサムネイルも BC7 で生成 | `--demo` でも同じ表示経路を通す |
| I9 | ISPC のライセンス | `docs/licenses.md` と `packaging/licenses` に通知を追加 | 事前ビルド済み ISPC コードを同梱するため |
| I10 | ビューアの遷移 | 遷移元のサムネイルも一時的に pin し、配列レイヤーでクロスフェードする | フル画像が未到着の連続送りでも現行の遷移を維持する |
| I11 | 起動時プリ生成 | デーモンの既存の「最新 5,000 件」バッチは維持する | 初回スクロールの待ちを悪化させない。解像度変更直後は表示分から生成する |
| I12 | ディスク上限 | `thumbnail_cache_mb` の既定 2 GB は変更しない | BC7 では 384×216 で約 25,000 枚。超過分は LRU で削除され、再表示時に再生成される |

## 現状

- `crates/core/src/thumbnail.rs`
  - `ThumbnailSpec { quality, size }`。ファイル名は `<id>-q80-s384.webp`。
  - `ensure_thumbnail_in_with_status` が WebP を生成し、生成時だけ `RgbaImage` を返す。
  - キャッシュの使用量・LRU 削除・全削除・写真単位の削除は `.webp` を前提にしている。
- `crates/core/src/image_decode.rs`
  - `decode_thumbnail_image` は EXIF 回転を適用し、正方形ボックスへ `thumbnail()` する。
- `crates/ui/src/state/thumbnail.rs`
  - デコード済み `RenderImage` を最大 128 件・64 MiB の LRU で保持する。
  - 同時デコードは 1 件。失敗は 30 秒後に再試行する。
  - 生成時に平均色を計算し、プレースホルダー色として保持する。
- `crates/ui/src/state/photo_browser.rs`
  - `PAGE_SIZE = 256`、`MAX_PAGES = 24`。
- `crates/ui/src/ui/theme.rs`
  - グリッドのカードは 3:2 固定（`image_height = card_width * 2 / 3`）。
- `crates/ui/src/ui/gallery.rs` / `library.rs` / `viewer.rs`
  - サムネイルは `gpui_kit::img(...).object_fit(...)` で表示する。
  - ビューアのフル画像は別経路（`RenderImage`）で、サムネイルはフル画像到着までの
    フォールバック、ミニマップ、モーフに使う。
- `crates/ui/src/ui/settings.rs`
  - サムネイル画質スライダーと 256 / 384 / 512 の解像度ボタンがある。
- `Cargo.toml`
  - `gpui-fast` は `TommyZ-7/gpui-fast` の `a3c3ac5` に固定。
  - `gpui-fast-wgpu` と `gpui-fast-windows` も同じ rev。
- gpui-fast フォーク
  - Linux は `gpui_wgpu`、Windows は Direct3D 11 の別レンダラ。
  - Texture2DArray / BC7 の経路はない。
  - macOS/Metal はこのアプリのリリース対象外。

## 仕様

### キャンバスと生成

- 解像度プリセットは次の 3 つ。

  | 設定値（幅） | キャンバス | BC7 1 枚（1 B/px） | 512 レイヤー配列 |
  |---|---|---|---|
  | 256 | 256×144 | 36,864 B（36 KiB） | 18 MiB |
  | 384（既定） | 384×216 | 82,944 B（81 KiB） | 40.5 MiB |
  | 512 | 512×288 | 147,456 B（144 KiB） | 72 MiB |

- 生成手順:
  1. 既存の `decode_thumbnail_image` で EXIF 回転を適用した RGBA を得る。
  2. 表示向きで縦長（`height > width`）なら、反時計回りに 90°回転する。
  3. キャンバス寸法へ引き伸ばす。切り抜きも余白追加もしない。
  4. 最終キャンバスから平均色を計算する。
  5. `intel_tex_2` で BC7 ブロックへエンコードする。
- キャンバス寸法はすべて 4 の倍数なので、BC7 の部分ブロック処理は不要。
- 回転の判定は EXIF 適用後の表示向きで行う。DB の `photo.width/height` も
  EXIF 適用後の表示寸法なので、表示側の判断と一致する。
- 正方形は回転しない。

### 保存形式

- パス: `thumbnails/<photo_id>-s<width>x<height>.bc7`
- 中身: BC7 ブロックを画像の行順に連結したもの。ヘッダなし。
- サイドカー: `<path>.meta`
  - `v2 <mtime_secs> <mtime_nanos> <size> <color> <rotated>`
  - `color` は `0xRRGGBB`。`rotated` は `0` / `1`。
- 一時ファイルは既存の `write_bytes_no_clobber` と `unique_temp_path` を使う。
- キャッシュの fresh 判定は source の mtime と size。サイドカーが `v1` または
  欠落なら再生成する。
- 読み込み時はファイル長が `ceil(width/4) * ceil(height/4) * 16` と一致することを
  検証する。不一致は破損として削除・再生成する。

### 設定

- `AppSettings.thumbnail_quality` を削除する。JSON に残っていても無視される。
- `thumbnail_size` は幅（256 / 384 / 512）として維持する。
  - 既存値 256 → 256×144、384 → 384×216、512 → 512×288。
  - 未知の値は 384 に正規化する。
  - キーがない場合の既定値は 384（従来は 256）。
- 設定画面から画質スライダーを削除する。解像度は 3 ボタンのまま、
  表示を `256×144` / `384×216` / `512×288` にする。
- 解像度を変更したら:
  1. UI はメモリ上のサムネイル管理状態と Texture2DArray を破棄する。
  2. デーモンは旧キャッシュを全削除する。
  3. 以後、グリッドに表示された写真から新しい寸法で再生成する。
- 既存 WebP は移行しない。デーモン起動時に旧 `.webp` とそのサイドカー、
  一時ファイルを削除する。

### 表示

- 単一の Texture2DArray を 512 レイヤーで作成する。配列は増やさない。
- レイヤーは写真 ID に割り当てる。512 枚を超えたら、選択中の写真を除く
  最も古いレイヤーを再利用する。
- UI はデコード済み画像を保持しない。レイヤーが evict された写真を再表示するときは、
  `.bc7` ファイルを読み直してレイヤーへアップロードする。
- 平均色は小さな `HashMap<i64, u32>` に残す。これは画像データではない。
- グリッドのカードは現行の 3:2 固定。表示要素には次の情報を渡す。
  - Texture2DArray とレイヤー番号
  - DB の表示寸法（`photo.effective_dimensions()`）
  - 生成時に回転したか（サイドカーの `rotated`）
  - `ObjectFit`（グリッド・インスペクタ・セッション表紙は `Cover`）
- `ObjectFit` の計算は「写真の表示寸法」を基準にする。16:9 キャンバスは写真を
  引き伸ばしたものなので、写真のアスペクトの矩形へ描くことで比率を戻す。
- 縦向きは UV を 90°回して戻す。
- ビューアのフル画像表示は従来どおり `RenderImage`。サムネイルを使うのは
  フル画像到着までのフォールバック、ミニマップ、モーフ。
  - フレームのアスペクトは `photo.effective_dimensions()` から決まる。
  - サムネイルはフレーム全体へ `Fill` 相当で描き、レターボックスを作らない。
  - モーフはグリッドの 3:2 カバーからビューアの写真アスペクトへ補間する。
- セッション表紙（104×58）、インスペクタのプレビュー、カードは `Cover`。
- どの表示面も、サムネイルの 16:9 キャンバス寸法ではなく DB の表示寸法で
  レイアウトを決める。
- ビューアの前後送りクロスフェードは、遷移元がフル画像なら従来どおり
  `RenderImage` を使う。遷移元がサムネイルしかない場合は、その写真の
  レイヤーを遷移中だけ pin し、`Texture2DArray` のスプライトで表示する。
  - `previous_image` と `ViewerMorph` は「フル画像」または
    「サムネイル（配列、レイヤー、回転、DB 寸法）」のどちらかを保持する。
  - モーフ終了時に遷移元の pin を外す。pin が外れた後に evict されても、
    次に必要になった時点でファイルから再アップロードする。
  - pin が間に合わずレイヤーが消えた場合は、クロスフェードを省略して
    新しい画像だけを表示する。フォールバック画像は作らない。

### エラー

- 最初の配列作成時に BC7 Texture2DArray が使えるか確認する。
- 非対応なら通常 UI を描画せず、次のエラー画面を出す。
  - 「この環境ではサムネイル用の BC7 テクスチャ配列を利用できません」
  - 検出した GPU / バックエンド / 不足している機能
  - 終了ボタン
- サムネイル 1 枚の読み込み失敗は、従来どおりプレースホルダーと再試行に留める。
- フォールバック（RGBA アトラス、WebP、非圧縮配列）は行わない。

### キャッシュ管理

- ディスク:
  - `thumbnail_cache_usage` は `.bc7` のみを数える。
  - `prune_thumbnails_in` は `.bc7` を最終更新順に削除する。
  - `clear_thumbnail_cache_in` は `.bc7`、`.bc7.meta`、`.bc7.tmp-*`、
    旧 `.webp` 系を削除する。
  - 写真単位の削除は全バリアントとサイドカーを消す。
- メモリ:
  - `ThumbnailCache` から `images`、`image_bytes`、`total_bytes`、
    `take_removed` を削除する。
  - 代わりに `layers: Vec<Option<LayerEntry>>`（512）、`free_layers`、
    `recency`、`by_photo` を持つ。
  - `pending` / `failed` / `pinned` / `photo_generations` は維持する。
  - 設定変更・キャッシュ全削除では配列を破棄し、次の描画で再作成する。
- 転送:
  - 背景処理は BC7 ブロックと平均色を返す。UI は最大 1 件のアップロード待ちを持つ。
  - 描画の先頭で配列を作成済みならアップロードし、レイヤーを確定する。
  - アップロード中に設定が変わった場合は世代番号で破棄する。
- 表示側の再描画:
  - レイヤーへ書き込むたびに、そのレイヤーの版番号を進める。
  - スプライトのプリミティブは版番号を含み、タイルキャッシュのハッシュに含める。
  - `Window::upload_texture_array_layer` はウィンドウへ再描画を要求する。
- 解像度変更後の生成:
  - UI が表示した写真から要求する。全件の一括生成は行わない。
  - デーモンの既存の起動時バッチ（最新 5,000 件）はそのまま維持する。
    解像度変更でワーカーを再起動しない限り、このバッチは走らない。
  - デーモン再起動時は、新しい寸法で最新 5,000 件が先行生成される。
- ディスク上限:
  - 既定の `thumbnail_cache_mb = 2048` は変更しない。
  - 384×216 では約 25,000 枚で 2 GB に達する。超過分は LRU で削除され、
    再表示時に BC7 を再生成する。この再生成コストは計測対象に含める。

## gpui-fast フォーク設計

### 公開 API

`gpui-fast` の `gpui` クレートに次を追加する。名前は実装時に調整してよい。

```rust
pub struct TextureArrayId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureArrayFormat {
    Bc7RgbaUnorm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureArray {
    pub id: TextureArrayId,
    pub size: Size<DevicePixels>,
    pub layers: u32,
    pub format: TextureArrayFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureArrayUv {
    Normal,
    Rotate90,
}

impl Window {
    pub fn create_texture_array(
        &mut self,
        size: Size<DevicePixels>,
        layers: u32,
        format: TextureArrayFormat,
    ) -> Result<TextureArray>;

    pub fn upload_texture_array_layer(
        &mut self,
        array: TextureArray,
        layer: u32,
        blocks: &[u8],
    ) -> Result<()>;

    pub fn paint_texture_array_layer(
        &mut self,
        bounds: Bounds<Pixels>,
        image_bounds: Bounds<Pixels>,
        array: TextureArray,
        layer: u32,
        uv: TextureArrayUv,
        corner_radii: Corners<Pixels>,
        opacity: f32,
    ) -> Result<()>;

    pub fn remove_texture_array(&mut self, array: TextureArray);
}
```

- `create_texture_array` はレイヤー数と寸法を検証し、プロセス内のメタデータと
  プラットフォームの GPU リソースを作る。
- `upload_texture_array_layer` は `layer < layers`、BC7 の期待バイト長、
  配列のフォーマットを検証する。成功時にレイヤーの版番号を進め、再描画を要求する。
- `paint_texture_array_layer` は `bounds` と `image_bounds` から可視部分と UV 部分矩形を
  計算し、新しいプリミティブをシーンへ積む。
- `remove_texture_array` はメタデータと GPU リソースを解放する。
- 要素側は `texture_array_img(array, layer)` を追加し、`content_size`（DB の表示寸法）、
  `rotated`、`ObjectFit`、角丸、不透明度を受ける。`ObjectFit` は少なくとも
  `Cover` / `Contain` / `Fill` を実装する。
- gpui-kit は `pub use ::gpui_fast::*;` なので、新しい公開型と要素は
  `gpui_kit::*` からそのまま使える。

### プリミティブ

- `Primitive::TextureArraySprite(TextureArraySprite)` を追加する。
- フィールド:
  - `order`
  - `array_id`
  - `layer`
  - `version`（タイルハッシュ用）
  - `bounds`（可視矩形）
  - `content_mask`
  - `uv_origin` / `uv_size`（可視部分のテクスチャ座標）
  - `uv_transform`（Normal / Rotate90）
  - `corner_radii`
  - `opacity`
- `PrimitiveBatch::TextureArraySprites { array_id, range }` を追加する。
  同じ配列が連続する範囲を 1 バッチにする。
- 次の共通処理に腕を追加する。
  - `Primitive::bounds` / `content_mask`
  - `BatchIterator`
  - `fast::layers::tiles::hash_primitive`
  - `fast::layers::scene::{visible_bounds, drawn_bounds, translate_primitive}`
  - `fast::layers::paint::{clip_primitive, ...}`
  - `fast::rounded_clip_scene`
  - wgpu / Direct3D のフレーム計画と描画
- タイルハッシュに `version` を含める。レイヤー再利用後に古いタイルが
  新しい内容を表示しないようにする。

### wgpu バックエンド（Linux）

- デバイス作成時に `TEXTURE_COMPRESSION_BC` がアダプタで利用可能なら要求する。
  利用不可でもデバイス作成は成功させ、`create_texture_array` が明示エラーを返す。
- 配列テクスチャ:
  - `wgpu::TextureFormat::Bc7RgbaUnorm`
  - `TextureUsages::TEXTURE_BINDING | COPY_DST`
  - `size.depth_or_array_layers = layers`
  - mip 1
- アップロード:
  - `Queue::write_texture` で `origin.z = layer`、`depth_or_array_layers = 1`。
  - `bytes_per_row = ceil(width / 4) * 16`
  - `rows_per_image = ceil(height / 4)`
- 新しいシェーダ:
  - `texture_2d_array<f32>` と `layer` を受け取る。
  - UV 変換（Normal / Rotate90）を適用する。
  - 角丸 SDF、クリップ、不透明度は既存のポリクロームスプライトと同じ。
- 新しいパイプラインとバインドグループ（配列ごとにテクスチャビュー＋サンプラー）。
- インスタンスデータは既存のストレージバッファ経路に新しい配列を追加する。
- ヘッドレスレンダラのテストで、既知の BC7 ブロックをアップロードし、
  回転あり／なしのサンプル位置を検証する。

### Direct3D 11 バックエンド（Windows）

- `DXGI_FORMAT_BC7_UNORM` の `Texture2D` を `ArraySize = layers`、`MipLevels = 1` で作る。
- `CheckFormatSupport` で BC7 と Texture2D のサポートを確認する。
- `UpdateSubresource` でレイヤーを書き込む。
  - `D3D11_BOX` の `front/back` をレイヤーに合わせる。
  - `RowPitch = ceil(width / 4) * 16`
  - `DepthPitch = RowPitch * ceil(height / 4)`
- HLSL に `Texture2DArray<float4>` と `layer` を受け取る頂点／ピクセルシェーダを追加する。
- `PipelineState<TextureArraySprite>` を追加し、既存のポリクロームスプライトと同じ
  インスタンスバッファ方式で描画する。

### macOS / Metal

- バックエンドを登録しない。`create_texture_array` は
  `Err("texture arrays are not supported on this platform")` を返す。
- `gpui_apple` にシェーダやパイプラインは追加しない。既存の
  dynamic texture と同じ「プラットフォーム未対応」経路を使う。
- このアプリは macOS をリリースしないため、実行時には到達しない。

## 変更ファイル（本体）

- `crates/core/Cargo.toml`
  - `intel_tex_2` を追加。
- `crates/core/src/bc7.rs`（新規）
  - `encode_rgba` / 期待バイト長 / 4 の倍数検証。
- `crates/core/src/thumbnail.rs`
  - `ThumbnailSpec` から品質を削除。
  - キャンバス寸法、回転、引き伸ばし、BC7 保存。
  - `EnsuredThumbnail` に `bc7` / `color` / `rotated` を追加。
  - `.bc7` の走査、使用量、削除、旧 WebP の一括削除。
- `crates/core/src/cache_metadata.rs`
  - `v2` の source stamp＋平均色＋回転。
- `crates/core/src/settings.rs`
  - `thumbnail_quality` 削除、`thumbnail_size` の正規化、既定 384。
- `crates/core/src/image_decode.rs`
  - キャンバス用の回転・引き伸ばしヘルパー（または `thumbnail.rs` 側）。
- `crates/daemon/src/thumbnails.rs`
  - spec の pack/unpack を幅のみに。
- `crates/daemon/src/lifecycle.rs`
  - 解像度変更検出時のキャッシュ全削除。
  - 起動時の旧 WebP 削除。
- `crates/daemon/src/tests.rs`
  - `.bc7` 前提へ更新。
- `crates/ui/src/state/thumbnail.rs`
  - レイヤー LRU、アップロード待ち、`ThumbnailHandle`。
  - 画像キャッシュと `take_removed` を削除。
- `crates/ui/src/app.rs`
  - Texture2DArray の所有、作成失敗時のエラー状態、破棄の遅延。
- `crates/ui/src/app/photos.rs`
  - BC7 読み込み・アップロード要求。
- `crates/ui/src/ui/gallery.rs`
  - カードを `texture_array_img` へ。
- `crates/ui/src/ui/library.rs`
  - インスペクタのサムネイルを `texture_array_img` へ。
- `crates/ui/src/ui/viewer.rs`
  - フォールバック、ミニマップ、モーフを `texture_array_img` へ。
- `crates/ui/src/app/viewer.rs` / `crates/ui/src/state/viewer.rs`
  - 前後送りのクロスフェード元を「フル画像またはサムネイル」の enum へ。
  - 遷移中のサムネイル pin と解放。
- `crates/ui/src/ui/settings.rs`
  - 画質スライダー削除、解像度ボタンの表示変更、再生成ヒント。
- `crates/ui/src/app/demo.rs`
  - デモサムネイルを `.bc7` で生成。`DEMO_DATA_VERSION` を上げる。
- `crates/i18n/locales/ja.yml` / `en.yml`
  - `settings.thumbnail.quality` を削除。
  - 再生成ヒントを追加。
- `docs/ui.md`
  - サムネイルの保存形式・寸法・キャッシュ・表示経路を更新。
- `docs/licenses.md` / `packaging/licenses`
  - ISPC の通知を追加。
- `Cargo.toml` / `Cargo.lock`
  - gpui-fast の rev を fork マージ後に更新。
  - `intel_tex_2` を追加。

## 非対象

- WebP から BC7 への移行ツール。既存キャッシュは破棄する。
- BC7 非対応環境のフォールバック（RGBA アトラス、WebP 表示、非圧縮配列）。
- サムネイル画質設定。
- グリッドカードの可変アスペクト（メイソンリー）。
- フル画像の BC7 化・Texture2DArray 化。
- macOS/Metal の実装。
- 512 レイヤーを超える同時表示の最適化。可視枚数は 512 未満を前提とする。

## リスク

1. **fork の変更量**  
   wgpu と Direct3D 11 の 2 バックエンド、シェーダ、バッチ処理、タイルキャッシュに
   跨る。独立 PR にし、まず wgpu でヘッドレステストを通してから Windows へ展開する。

2. **BC7 非対応 GPU**  
   Vulkan / D3D11 の古い環境では配列作成が失敗する。起動時に明示エラーを出す。
   CI の Windows ランナーが BC7 をサポートしない場合は、コンパイル確認と
   ユニットテストに留め、実機確認は手動にする。

3. **レイヤー再利用中の表示**  
   512 枚を超えて evict した直後に、古いタイルが新しい内容を表示する可能性がある。
   プリミティブに版番号を持たせ、タイルハッシュへ含める。選択中は evict しない。

4. **BC7 エンコード時間とピークメモリ**  
   384×216 の ISPC エンコードは軽量だが、元画像のデコードと合わせたピークを
   計測する。`decode_budget` の見積もりを更新する。

5. **ISPC のライセンスとビルド**  
   `intel_tex_2` はプラットフォーム別の事前ビルド静的ライブラリを同梱する。
   `cargo about` の対象外のコードなので、通知を手動で追加する。
   Windows の MSVC で MD/MT の組み合わせが合うか CI で確認する。

6. **ウィンドウ再作成・デバイスロスト**  
   配列はウィンドウの GPU リソースに紐づく。ウィンドウが作り直されたら
   配列も作り直す。デバイスロスト時はアプリの再起動を要求する。

7. **ディスク使用量の増加**  
   BC7 は固定 1 B/px（384×216 で 81 KiB）。WebP より大きくなる可能性がある。
   既定 2 GB の上限では 384×216 で約 25,000 枚しか保持できず、それを超えると
   スクロール中に再生成が発生する。実測して記録し、必要なら上限の見直しを別途提案する。

## テスト

### core

- 256×144 / 384×216 / 512×288 のキャンバスが生成される。
- 横向き・縦向き・正方形・16:9 / 9:16 / 4:3 / 3:4 で、
  回転フラグと保存後の寸法が正しい。
- 引き伸ばしで切り抜き・余白が入らない（四隅の色で確認）。
- BC7 のファイル長が期待値と一致する。
- `v2` メタデータの往復、`v1` の再生成、破損長の再生成。
- 使用量・LRU 削除・全削除・写真単位削除が `.bc7` と旧 `.webp` を扱う。
- 設定の正規化（256/384/512、未知値、キー欠落、旧品質キーの無視）。

### ui

- 512 レイヤーの割り当て、解放、LRU 再利用、pinned の保護。
- レイヤー evict 後の再要求でファイルを読み直す。
- 設定変更・全削除で配列が破棄され、次の描画で再作成される。
- アップロード待ちが 1 件に制限され、古い世代の結果が捨てられる。
- 失敗の再試行、写真削除、置換後の invalidate。
- 画像キャッシュ（`RenderImage`）を持たないこと。

### fork

- 寸法・レイヤー数・BC7 バイト長の検証。
- wgpu ヘッドレスレンダラで、既知の BC7 ブロックを 2 レイヤーへアップロードし、
  レイヤー選択と UV 回転をピクセルで確認する。
- 角丸・クリップ・不透明度が既存スプライトと同じ結果になる。
- レイヤー再アップロードでタイルハッシュが変わる。
- Windows のコンパイル（`cargo check` / `clippy`）と、可能なら D3D11 の
  ヘッドレス相当テスト。

### 手動確認

- `--demo` でグリッド・検索・インスペクタ・ビューア・ミニマップを確認する。
- 縦向き写真、16:9、9:16、4:3、3:4、パノラマを混在させ、
  グリッドとビューアで比率が崩れないことを確認する。
- 解像度を 256 → 384 → 512 と変更し、全サムネイルが新しい寸法で再生成されること、
  配列が作り直されることを確認する。
- 512 枚を超えてスクロールし、レイヤー再利用後も古い画像が残らないことを確認する。
- 設定画面に画質スライダーがないこと。

## 計測

- ディスク:
  - 同一の 1,000 枚（またはデモ 18 枚＋実ライブラリ）で、
    変更前の WebP と変更後の BC7 の合計バイト数と 1 枚平均を記録する。
  - `du -sb` と `thumbnail_cache_usage` の両方で確認する。
- VRAM:
  - 計算値: `512 * ceil(w/4) * ceil(h/4) * 16`。
  - 実測: Linux の AMD GPU では
    `/sys/class/drm/card*/device/mem_info_vram_used` の起動前後差を記録する。
  - 比較のため、変更前の RGBA LRU（最大 64 MiB）＋アトラスの使用量も記録する。
- 結果は `docs/superpowers/plans/2026-11-12-issue-579-bc7-thumbnails.md` の
  完了報告に追記する。

## PR 分割

1. **docs PR（このブランチ）**
   - 設計、モック、計画、`docs/ui.md`、ライセンス通知。
   - モック承認後に実装へ進む。
2. **gpui-fast fork PR**
   - `vrc-0.1.14-atlas-1536` から `vrc-0.1.14-texture2d-array`。
   - wgpu → Windows D3D11 → macOS スタブの順。
   - マージ後に rev を確定する。
3. **本体実装 PR**
   - fork の rev 更新、core / daemon / UI / demo / i18n / docs。
   - `CI` と `Release`（bundle-check）を green にしてマージ。
4. **計測・完了報告**
   - 実装 PR に計測結果を追記するか、完了コメントとして残す。

## 参考

- `crates/core/src/thumbnail.rs`
- `crates/ui/src/state/thumbnail.rs`
- `crates/ui/src/ui/theme.rs`
- `crates/ui/src/ui/gallery.rs`
- `crates/ui/src/ui/viewer.rs`
- `gpui-fast`: `gpui/src/scene.rs`, `gpui_wgpu/src/shaders.wgsl`,
  `gpui_windows/src/shaders.hlsl`
- `intel_tex_2`: <https://docs.rs/intel_tex_2>
