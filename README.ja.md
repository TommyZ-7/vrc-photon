[English](README.md) | **日本語**

<p align="center">
  <img src="packaging/icons/icon.png" width="96" alt="VRC PhotoN">
</p>

# VRC PhotoN

<p align="center">
  <a href="https://github.com/TommyZ-7/vrc-photon/releases"><img src="https://img.shields.io/github/v/release/TommyZ-7/vrc-photon" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/TommyZ-7/vrc-photon" alt="License"></a>
  <img src="https://img.shields.io/github/languages/top/TommyZ-7/vrc-photon" alt="Top language">
  <img src="https://img.shields.io/github/languages/count/TommyZ-7/vrc-photon" alt="Languages">
</p>

VRChatの写真を、撮ったときのワールド・一緒にいたプレイヤー・日時とあわせて管理するデスクトップアプリです。

VRChatのスクリーンショットは、ファイル名だけでは中身が分かりません。VRC PhotoNは写真フォルダを監視し、VRChatの出力ログとPNGのXMPメタデータから「いつ、どこで、誰と撮ったか」を読み取って写真に紐付けます。記録はすべて手元のSQLiteに保存され、写真やログが外部に送信されることはありません。

<p align="center">
  <img src="docs/images/vrc-photon-social.png" alt="VRC PhotoN">
</p>

<!-- TODO: 実画面のスクリーンショットを docs/images/main.png に置き、下の行を有効化する
![メイン画面](docs/images/main.png)
-->

## できること

**探す**

- 写真一覧は仮想スクロールで、大量の写真でも一息にスクロールできます
- ワールド名、プレイヤー名、タグ、日時、お気に入りで検索できます
- 写真ごとにワールドと同接プレイヤーをログから自動で紐付けます。ログが無い写真はXMPメタデータで補います

**圧縮する**

- JPEG / WebP / AVIF / JPEG XL への変換とリサイズができます
- プロファイルを作っておけば、取り込み時の自動圧縮も、選んだ写真の一括圧縮も同じ設定で回せます
- 元画像を残すか、置き換える（OSのゴミ箱へ送る）かは実行ごとに選べます

**その他**

- 写真の詳細で、プレビュー、ワールド、インスタンス、同接プレイヤー、タグ、メモ、圧縮履歴を確認・編集できます
- Discord Webhookへ写真を送れます（任意設定で、初期状態はオフ）
- トレイ常駐と自動起動に対応し、署名付きの自動アップデートがあります
- UIは日本語と英語に対応しています

**AIタグ付け（有償プラグイン）**

人物の有無と人数、場所、雰囲気をローカル推論でタグ付けするプラグインを別売しています。推論は手元のPCで完結します。導入方法と動作要件は [AIタグ付けのドキュメント](docs/ai-tagging/README.md) を参照してください。

## インストール

[Releases](https://github.com/TommyZ-7/vrc-photon/releases) から、使っている環境のファイルをダウンロードしてください。

| 環境 | ファイル |
|---|---|
| Windows | `vrc-photon_<バージョン>_x64-setup.exe` |
| Debian / Ubuntu | `vrc-photon_<バージョン>_amd64.deb` |
| Arch系 | `vrc-photon-<バージョン>-1-x86_64.pkg.tar.zst` |
| その他のLinux | `vrc-photon_<バージョン>_x86_64.AppImage`、または `.tar.gz` |

写真フォルダとログフォルダは初回起動時に自動で検出されます。検出できない環境では、デーモンの起動時に `--photo-dir` と `--log-dir` で指定してください。

## 仕組み

UIとデーモンの2プロセス構成です。デーモンがフォルダ監視、ログ解析、圧縮、サムネイル生成を担当し、UIは表示と操作を担当します。両者は同じSQLiteデータベース（`library.db`）を共有し、即時の操作にはローカルIPCを使います。

```text
crates/
  core/     DB、写真索引、ログ解析、XMP、紐付け、圧縮、サムネイル
  ipc/      ローカルIPCのプロトコルとトランスポート
  daemon/   監視、自動圧縮、トレイ、自動起動、IPCサーバー
  ui/       GPUI製のUI、IPCクライアント
```

詳しい構成は [docs/architecture.md](docs/architecture.md) にあります。

## ソースからビルド

前提はRust stableのほか、プラットフォームごとにいくつか要ります（Linuxは `libxkbcommon-dev`、`libwayland-dev`、`libvulkan-dev` など。JPEG XLのビルドには CMake / Ninja / C++コンパイラ）。詳細は [docs/development.md](docs/development.md) を参照してください。

```bash
cargo build -p vrc-photon-ui
cargo build -p vrc-photon-daemon
```

サンプルデータでUIだけ試す場合は `cargo run -p vrc-photon-ui -- --demo` が使えます。

## ドキュメント

- [ドキュメント索引](docs/README.md)
- [アーキテクチャ](docs/architecture.md)
- [開発環境の構築](docs/development.md)
- [リリースとパッケージ](docs/release.md)
- [AIタグ付けプラグイン](docs/ai-tagging/README.md)
- [ロードマップ](docs/roadmap.md)
- [コントリビューション](CONTRIBUTING.md)

## ライセンス

本体は GPL-3.0-or-later です。AIタグ付けプラグインは、本体とは別のライセンスで提供しています。
