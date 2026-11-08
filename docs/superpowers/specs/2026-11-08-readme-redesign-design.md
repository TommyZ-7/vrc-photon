# README 改訂案

## 目的

README を、初めて見る人が「何のアプリか」「どう動かすか」「詳細がどこにあるか」の順に追える構成にする。機能・ビルド手順・ドキュメントへのリンクといった現行の事実は変えず、並び順と文言だけを整える。

## 対象

- `README.md` の見出し構成、箇条書きの分類、セクションの順序。
- 導入文と各項目の文言。

## 対象外

- 機能の追加・削除、仕様の変更。
- スクリーンショット、バッジ、ロゴの追加。
- README の分割や英語化。
- `README.md` 本体の差し替え。この PR では提案ファイルだけを追加する。

## 方針

- 機能の羅列を「写真の整理」「圧縮」「VRChat の記録と連携」「アプリの運用」に分ける。目的の項目を見出しから探しやすくするため。
- ビルドと起動を機能一覧の後ろから前へ移す。触ってみるまでの手順を追いやすくするため。
- 環境変数と保存先を「設定とデータ」にまとめ、リファレンスとして引ける位置に置く。
- 既知の制約は削らず、末尾近くにまとめて残す。
- 見出しと箇条書きの粒度を揃え、1項目につき1つの話題を書く。
- 絵文字、宣伝文句、誇張した表現は使わない。

### 検討した構成

1. 現行の順序を保ち、文言だけ整える。差分は小さいが、機能の羅列は読みにくいまま。
2. 機能を分類し、ビルドと起動を前倒しする（採用）。読み順は良くなるが、現行とは見出しの位置が変わる。
3. 利用者向けと開発者向けに README を分割する。情報が2ファイルに散らばり、現行の1ファイル構成から外れる。

## 提案する README

````markdown
# VRC PhotoN

VRChat で撮った写真を、ワールド・同接プレイヤー・撮影日時と一緒に整理するデスクトップアプリです。

写真フォルダを監視し、VRChat の出力ログと PNG の XMP メタデータから「いつ・どこで・誰と撮ったか」を読み取って、ローカルの SQLite に記録します。既存の圧縮エンジンを移植し、JPEG / WebP / AVIF / JPEG XL への変換と自動圧縮に対応しています。

## 主な機能

### 写真の整理

- 写真一覧は仮想スクロールで表示し、サムネイル、日付、圧縮済み/未紐付け、お気に入りが分かる。
- 取り込み時は、コピーか移動を都度選ぶ。
- 詳細画面では、プレビュー、ワールド、インスタンス、撮影者、同接プレイヤー、タグ、メモ、圧縮履歴を確認できる。
- 検索は、ワールド、プレイヤー、タグ、日時、お気に入り、AI タグ、人数に対応する。
- 一覧や検索結果で複数の写真を選び、まとめて圧縮できる。
- 写真は OS のゴミ箱へ移動できる。実行前には2段階の確認がある。
- 外部で削除された写真は `missing` として検出する。

### 圧縮

- プロファイルごとに、形式、品質、ロスレス、リサイズ、出力フォルダ、自動適用を設定する。
- 手動圧縮では、プロファイルを選び、指定フォルダ・プロファイルのフォルダ・クリップボードのいずれかへ出力する。
- 自動圧縮は設定で、手動・一括圧縮は実行のたびに、元画像を残すか置き換えるかを選ぶ。置き換えた元画像は OS のゴミ箱に入る。
- ダッシュボードでは、共有の圧縮キュー、待機中のジョブの一括実行、ログの監視をまとめて扱う。
- サムネイルは 256px の WebP をバックグラウンドで生成する。

### VRChat の記録と連携

- VRChat のログを監視し、ワールド参加、プレイヤーの入退室、セッションを記録する。
- XMP メタデータから撮影者とワールド名/ID を読み取る。ログが残っていない写真は、この情報で補完する。
- AI タグ付けに対応する（有償プラグイン、ローカル推論）。人物の有無、人数、場所、シーンをタグにできる。
- Discord Webhook へ写真を送れる（オプトイン、既定はオフ）。手動送信、送信履歴、自動送信に対応し、原画を優先する。20 MiB を超える場合は、連携用プロファイルの圧縮版を使う。

### アプリの運用

- 設定画面では、起動、トレイ、自動圧縮、即時/待機、自動圧縮時の元画像の保持/置換を切り替えられる。
- トレイに常駐し、自動起動とデーモンのハートビート表示に対応する。
- UI とデーモンはローカル IPC でつながる。即時再スキャン、一時停止/再開、スキャン/圧縮の進捗、終了をやり取りする。
- 署名付きの自動アップデートに対応する（AppImage / Windows の current-user NSIS）。deb と pacman は通知のみ。更新確認は GitHub Releases への HTTPS GET だけで、写真やログは送らない。
- deb / pacman では、任意で systemd ユーザーサービスとして動かせる。ログは journalctl で確認できる。

## ビルド

### 前提

- Rust stable（edition 2024）
- CMake / Ninja / C++ コンパイラ（JPEG XL の vendored ビルド用）
- Linux: `libxkbcommon-dev`、`libwayland-dev`、`libvulkan-dev`、`pkg-config`、`clang`
- Windows: MSVC Build Tools、Windows SDK、CMake、Ninja。DirectX 12 対応 GPU を推奨

```bash
cargo build -p vrc-photon-ui
cargo build -p vrc-photon-daemon
```

## 起動

### デモ画面

サンプルデータで UI だけを確認する。

```bash
cargo run -p vrc-photon-ui -- --demo
```

### 実データ

デーモンと UI を起動する。写真フォルダとログフォルダを自動検出できない場合は、デーモンに明示する。

```bash
cargo run -p vrc-photon-daemon -- --photo-dir "/path/to/Pictures/VRChat" --log-dir "/path/to/AppData/LocalLow/VRChat/VRChat"
```

別のターミナルで UI を起動する。

```bash
cargo run -p vrc-photon-ui
```

デーモンと UI は同じ DB を参照する。UI の「再読込」で最新状態を取得できる。

### 1回だけスキャン

```bash
cargo run -p vrc-photon-daemon -- --once --photo-dir ... --log-dir ...
```

`--compress-pending` を付けると、キューにある圧縮ジョブを処理して終了する。

```bash
cargo run -p vrc-photon-daemon -- --once --compress-pending --photo-dir ...
```

### ヘッドレス

```bash
cargo run -p vrc-photon-daemon -- --no-tray
```

## 設定とデータ

### 環境変数

| 変数 | 用途 |
|---|---|
| `VRC_PHOTON_DIR` | 写真フォルダの上書き |
| `VRC_LOG_DIR` | VRChat ログフォルダの上書き |
| `VRC_PHOTON_DB` | SQLite ファイルの上書き |
| `VRC_PHOTON_UPDATE_FEED` | 更新マニフェスト URL の上書き（ミラー・QA 用） |
| `RUST_LOG` | ログレベル（例: `debug`） |

### データの保存先

- DB: OS のユーザーデータディレクトリ配下 `vrc-photon/library.db`
  - Linux: `~/.local/share/vrc-photon/`
  - Windows: `%LOCALAPPDATA%\vrc-photon\`
- サムネイル: 同ディレクトリの `thumbnails/`
- 更新ファイル・適用ログ・旧版バックアップ: 同ディレクトリの `updates/`
- Discord Webhook URL: 同 DB の `app_settings` テーブル（`discord_webhook_url`）

写真ファイル自体は移動しない。圧縮版は既定で元写真の `compressed/` に生成する。

Discord 連携を有効にしない限り、写真・ログを外部へ送信しない。Webhook URL は認証情報なので、DB バックアップを共有するときは注意する。

## 既知の制約

- 写真の取り込みはコピー/移動を都度選択する。取り込み先の既定はユーザーデータディレクトリの `imported/`。
- 一覧は4列固定の仮想スクロールで、現在は最大300枚を読み込んで表示する。100k枚のページングは未実装。検索も最大300枚/回。
- UI は IPC イベントで監視状態・キュー・写真枚数を即時更新する。IPC が使えない場合は3秒ごとの自動更新にフォールバックする。
- デーモンと UI は SQLite で耐久状態を共有し、ローカル IPC で即時 wake とイベントを補完する（ADR-0011）。
- トレイ/自動起動は Linux でコンパイル確認済み。Windows 実機は未検証。
- VRChat の出力ログは24時間で削除される。過去分の復元は実行時に残っているログのみ。
- プライベートワールドは XMP に名前が入らないことがある。その場合はワールド ID 表示になる。
- AVIF の「ロスレス」は `ravif` の制約で品質100の近似。JPEG XL は現状αを保持しない。

## ドキュメント

開発や運用の詳細は `docs/` にまとめている。

- [docs/README.md](docs/README.md) — 開発ドキュメントの索引
- [docs/architecture.md](docs/architecture.md) — 全体構成
- [docs/development.md](docs/development.md) — 環境構築・実行
- [docs/release.md](docs/release.md) — リリースとパッケージ
- [docs/ai-tagging/README.md](docs/ai-tagging/README.md) — AI タグ付けプラグインの導入・GPU 要件・プライバシー
- [docs/roadmap.md](docs/roadmap.md) — 残作業と優先度
- [CONTRIBUTING.md](CONTRIBUTING.md) — 開発フロー

## 構成

リポジトリは次のクレートで構成する。

```text
crates/
  core/     # DB、写真索引、ログ解析、XMP、紐付け、圧縮、サムネイル
  ipc/      # ローカルIPCのプロトコルとトランスポート
  daemon/   # 監視、自動圧縮、トレイ、自動起動、IPCサーバー
  ui/       # gpui-kit アプリ、IPCクライアント
docs/
  開発企画書.md
  ui-mockup.html   # ブラウザで開ける画面案
```

UI とデーモンは同じ SQLite（WAL）を共有する。重い書き込みはデーモン、UI は短い読み書きを行う。即時コマンドとイベント配信にはローカル IPC（Unix domain socket / Windows 名前付きパイプ、改行区切り JSON）を使う。IPC は wake-up とイベントだけを担い、耐久データの正は SQLite のまま。

## 開発

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo check` / `build` は WSL で負荷が高いため、並列実行しないこと。

## ライセンス

GPL-3.0-or-later（0.9.x 以前は MIT）。同梱アセットの通知とパッケージ内の配置は [docs/licenses.md](docs/licenses.md) を参照。

- Noto Sans JP: SIL Open Font License 1.1（`crates/ui/assets/LICENSE-NotoSansJP.txt`）
- Lucide アイコン（`gpui-kit-assets` 経由）: ISC、Feather 由来分は MIT（`packaging/licenses/LICENSE-LUCIDE`）
````

## 事実の維持

現行 README の内容が提案内に残っていることを、次の単位で確認した。

- 機能 21 項目（写真一覧から `missing` 検出まで）。
- ビルドの前提 4 項目と、UI / デーモンのビルドコマンド。
- 起動の各パターン（デモ、実データ、1回だけスキャン、`--compress-pending`、ヘッドレス）。
- 環境変数 5 つ。
- 保存先 4 つと、写真を外部送信しない条件。
- 既知の制約 8 項目。
- ドキュメントへのリンク 7 つ。
- ライセンス本文と同梱アセット 2 件。

## 受け入れ条件

- 提案文を `README.md` に置き換えたとき、現行の機能・ビルド手順・ドキュメント構成がすべて残っている。
- 見出しだけで内容の見当がつき、機能一覧を目的別に追える。
- 絵文字、宣伝文句、誇張した表現がない。
- この PR では `README.md` 本体を変更していない。
