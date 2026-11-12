# 同梱アセットのライセンス

このアプリは、サードパーティのアイコンとフォントをバイナリに埋め込み、依存クレートのライセンス文を含むライセンス通知をパッケージに同梱する。

## 同梱物

| アセット | ライセンス | リポジトリ内の通知 | 通知のパッケージ同梱 |
|---|---|---|---|
| Lucide アイコン（`gpui-kit-assets` 0.9.1 経由） | ISC（Feather 由来アイコンは MIT） | `packaging/licenses/LICENSE-LUCIDE` | あり（`LICENSE-LUCIDE`） |
| アプリ本体 | GPL-3.0-or-later | `LICENSE` | あり（`LICENSE`） |
| Noto Sans JP | SIL Open Font License 1.1 | `crates/ui/assets/LICENSE-NotoSansJP.txt` | なし（リポジトリのみ） |
| Intel ISPC Texture Compressor（`intel_tex_2` 経由） | MIT / Apache-2.0 | `packaging/licenses/THIRD-PARTY-NOTICES.md` | あり（`THIRD-PARTY-NOTICES.md`） |
| 依存クレートのライセンス文 | 各クレートのライセンス（MIT / Apache-2.0 など15種） | `packaging/licenses/THIRD-PARTY-NOTICES.md` | あり（`THIRD-PARTY-NOTICES.md`） |

## Lucide

- 取得元: `gpui-kit-assets` 0.9.1（<https://github.com/longbridge/gpui-kit>）
- Lucide: 1.43.0
- ファイル: `packaging/licenses/LICENSE-LUCIDE`（上流のファイルを改変せずコピー）
- sha256: `b495047bd93a9b06913511076f504daba17d5bbeb3e0650f3bb53a4220329c57`

`gpui-kit` を更新した場合は、新しい `gpui-kit-assets` の `LICENSE-LUCIDE` と上記ハッシュを比較し、変更があれば差し替える。

## Intel ISPC Texture Compressor

- 取得元: `intel_tex_2` 0.5.0（<https://github.com/Traverse-Research/intel-tex-rs-2>）
- 上流: Intel ISPC Texture Compressor（<https://github.com/GameTechDev/ISPCTextureCompressor>）
- ライセンス: MIT（`intel_tex_2` は MIT / Apache-2.0）
- 用途: サムネイルの BC7 エンコード。プラットフォーム別の事前ビルド済み ISPC 静的ライブラリをリンクする。
- `intel_tex_2` は `cargo-about` の依存クレート一覧に含まれる。上流 ISPC コードの帰属表示は
  `packaging/licenses/THIRD-PARTY-NOTICES.md` に含める。依存を更新したら再生成して確認する。

## 依存クレート

`packaging/licenses/THIRD-PARTY-NOTICES.md` は cargo-about で生成する（設定は `about.toml`、テンプレートは `about.hbs`）。手で編集しないこと。

依存クレートを追加・更新した場合は、次で再生成してコミットする。

```
cargo about generate about.hbs -o packaging/licenses/THIRD-PARTY-NOTICES.md
```

## パッケージ内の配置

`Packager.toml` の `resources` で `LICENSE`、`LICENSE-LUCIDE`、`THIRD-PARTY-NOTICES.md` を同梱する。

- deb / AppImage / pacman: `/usr/lib/vrc-photon/`
- Windows（NSIS）: 実行ファイルと同じディレクトリ

`Release` ワークフローの `Verify third-party notices` ステップが、Linux の各成果物に3ファイルが含まれることを検査する。
