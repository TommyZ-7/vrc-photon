# MCP サーバー

`vrc-photon-mcp` は、VRC PhotoN のライブラリを MCP（Model Context Protocol）クライアントから検索するためのローカル stdio サーバーです。読み取り専用で、写真の削除、タグの編集、圧縮などライブラリを変更する操作は提供しません。

MCP サーバーはデーモンが提供するローカル IPC を経由します。利用中は VRC PhotoN デーモンが起動している必要があります。MCP サーバー自身がデーモンを起動することはありません。

## 1. 実行ファイルの場所

| 配布形式 | 実行ファイル |
|---|---|
| deb / pacman | `/usr/bin/vrc-photon-mcp` |
| AppImage | `--appimage-extract` で展開した `squashfs-root/usr/bin/vrc-photon-mcp` |
| NSIS（Windows） | `%LOCALAPPDATA%\VRC PhotoN\vrc-photon-mcp.exe`（既定のインストール先） |

deb / pacman では UI やデーモンと同じ `/usr/bin` にインストールされます。

NSIS の既定のインストール先はユーザーごとの `%LOCALAPPDATA%\VRC PhotoN` です。インストール時に別のフォルダを選んだ場合は、UI（`vrc-photon.exe`）と同じフォルダに `vrc-photon-mcp.exe` があります。

AppImage は実行ファイルを内包しているため、先に展開します。

```bash
mkdir -p ~/apps/vrc-photon
cd ~/apps/vrc-photon
/path/to/vrc-photon_<version>_x86_64.AppImage --appimage-extract
```

展開後の実行ファイルは `~/apps/vrc-photon/squashfs-root/usr/bin/vrc-photon-mcp` です。`squashfs-root` は AppImage のバージョンごとに内容が変わるため、AppImage を更新したら展開し直してください。

## 2. デーモンを起動する

UI を起動すると、デーモンが未起動の場合は同じディレクトリのデーモンが自動で起動します。MCP サーバーを使う間は UI を起動したままにするか、以下のいずれかでデーモンを起動してください。

- deb / pacman で systemd ユーザーサービスを有効にしている場合:

  ```bash
  systemctl --user enable --now vrc-photon-daemon.service
  ```

- AppImage で UI を開かずにデーモンだけ動かす場合:

  ```bash
  ./vrc-photon_<version>_x86_64.AppImage --background
  ```

- 開発ビルドなどで直接起動する場合:

  ```bash
  vrc-photon-daemon --no-tray
  ```

デーモンが起動しているかは、Linux では次のコマンドで確認できます。

```bash
pgrep -a vrc-photon-daemon
```

systemd ユーザーサービスを使っている場合は `systemctl --user status vrc-photon-daemon.service` でも確認できます。自動起動の設定と systemd サービスを併用するときの注意は [troubleshooting.md](troubleshooting.md) を参照してください。

## 3. MCP クライアントの設定

MCP クライアントには stdio サーバーとして登録します。`command` に実行ファイルのパスを指定し、通常は引数なしで起動します。

deb / pacman の例:

```json
{
  "mcpServers": {
    "vrc-photon": {
      "command": "/usr/bin/vrc-photon-mcp",
      "args": []
    }
  }
}
```

AppImage の例:

```json
{
  "mcpServers": {
    "vrc-photon": {
      "command": "/home/your-name/apps/vrc-photon/squashfs-root/usr/bin/vrc-photon-mcp",
      "args": []
    }
  }
}
```

NSIS（Windows）の例:

```json
{
  "mcpServers": {
    "vrc-photon": {
      "command": "C:\\Users\\your-name\\AppData\\Local\\VRC PhotoN\\vrc-photon-mcp.exe",
      "args": []
    }
  }
}
```

`your-name` は実際のユーザー名に置き換えてください。設定ファイルの場所やキー名は MCP クライアントによって異なります。上記は `mcpServers` 形式を使うクライアントの例です。利用するクライアントのドキュメントに従って同じコマンドと引数を登録してください。

MCP サーバーは標準入出力で通信するため、クライアント側で stdio トランスポートを選びます。環境変数の設定は必須ではありません。デバッグ時にログを詳しく出す場合は、クライアントの設定で `RUST_LOG=debug` を指定できます。ログは stderr に出力されます。

## 4. データディレクトリを上書きする

MCP サーバーがデーモンのソケットを解決するために使うデータディレクトリは、`--data-dir <PATH>` で上書きできます。省略時は次の既定値を使います。デーモン側と同じパスを指定してください。

| 環境 | データディレクトリ |
|---|---|
| Linux | `$XDG_DATA_HOME/vrc-photon`（未設定なら `~/.local/share/vrc-photon`） |
| Windows | `%LOCALAPPDATA%\vrc-photon` |

```json
{
  "mcpServers": {
    "vrc-photon": {
      "command": "/usr/bin/vrc-photon-mcp",
      "args": ["--data-dir", "/home/your-name/.local/share/vrc-photon"]
    }
  }
}
```

Linux で `XDG_RUNTIME_DIR` が設定されている場合、デーモンのソケットは `$XDG_RUNTIME_DIR/vrc-photon/daemon.sock` に置かれます。この場合は通常 `--data-dir` を指定する必要はありません。`XDG_RUNTIME_DIR` が未設定の場合、ソケットは `<data-dir>/run/daemon.sock` になります。

Linux で `XDG_RUNTIME_DIR` が設定されている場合、デーモンのソケットは `$XDG_RUNTIME_DIR/vrc-photon/daemon.sock` に置かれます。この場合は通常 `--data-dir` を指定する必要はありません。`XDG_RUNTIME_DIR` が未設定の場合、ソケットは `<data-dir>/run/daemon.sock` になります。

Windows はユーザーごとの名前付きパイプを使うため、`--data-dir` は接続先の切り替えには影響しません。

## 5. 提供ツール

すべてのツールは読み取り専用です。写真の削除、タグやメモの編集、圧縮の開始などは行いません。編集は VRC PhotoN の UI から行ってください。

| ツール | 内容 |
|---|---|
| `search_photos` | 条件に合う写真をページ単位で検索する |
| `random_search_photos` | 同じ条件で写真をランダムに抽出する。`seed` を指定すると同じ結果を再現できる |
| `get_photo` | 1 枚分の詳細メタデータを返す |
| `get_photo_image` | 写真画像を返す。既定は長辺 1024px の JPEG プレビューで、`original: true` を指定すると原本を返す |
| `list_worlds` | ライブラリに含まれるワールドと写真数を返す |
| `library_stats` | 写真数、合計バイト数、ワールド数、圧縮済み枚数、派生ファイルの容量を返す |

`search_photos` と `random_search_photos` は同じ絞り込み引数を受け取ります。

| 引数 | 内容 |
|---|---|
| `world` | ワールド名、ワールド ID、インスタンス ID の部分一致 |
| `player` | プレイヤーの表示名、ユーザー ID の部分一致 |
| `tag` | 手動タグ、または AI タグの名前・キーの部分一致 |
| `ai_tags` | 付与済み AI タグキーの配列。複数指定した場合は OR で一致する |
| `captured_from` / `captured_to` | 撮影日時の範囲。RFC 3339 形式で指定し、境界を含む |
| `favorite` | `true` の場合はお気に入りのみ |
| `limit` | 取得件数。既定は 50 で、1〜200 に丸められる |
| `offset` | 読み飛ばす件数。既定は 0（`search_photos` のみ） |
| `seed` | 乱数シード。省略すると呼び出しごとに結果が変わる（`random_search_photos` のみ） |

検索結果は `photos`、`total`、`limit`、`offset`（ランダム抽選では `offset` なし）を返します。各写真には ID、表示用ファイルのパス、撮影日時、ワールド、プレイヤーが含まれます。

`get_photo` は解像度、ファイルサイズ、ワールド、インスタンス、プレイヤー、手動タグ、AI タグ、人数の推定区間、お気に入り、メモ、ファイル状態を返します。`get_photo_image` のプレビューは表示用の派生ファイルから生成され、`original: true` の場合は原本のバイト列をそのまま返します。

## 6. 接続できない場合

### デーモンが起動していない

ツールが `daemon_unavailable` を返す場合は、デーモンが起動していないか、MCP サーバーからソケットに到達できていません。`pgrep -a vrc-photon-daemon` でプロセスを確認し、UI または systemd ユーザーサービスで起動してください。

### ソケットを確認する

Linux ではソケットのパスと権限を確認します。

```bash
ls -l "$XDG_RUNTIME_DIR/vrc-photon/daemon.sock"
```

`XDG_RUNTIME_DIR` が未設定の場合は `<data-dir>/run/daemon.sock` を確認してください。ディレクトリは `0700`、ソケットは `0600` が正常です。デーモンが異常終了してソケットが残っている場合は、次回起動時に接続確認のうえ削除されます。手動で削除してもかまいません。

### データディレクトリが一致しているか確認する

`vrc-photon-daemon --print-data-dir` は、デーモンが既定で使うデータディレクトリを表示します。MCP クライアントの `--data-dir` と異なる場合は、どちらかを合わせてください。

AppImage を使っている場合は、展開した `squashfs-root` のパスが変わっていないこと、AppImage を更新したあとに展開し直したことを確認してください。

### ツールごとのエラー

| エラー | 意味 |
|---|---|
| `daemon_unavailable` | デーモンに接続できない |
| `photo_not_found` | 指定した写真 ID がライブラリに存在しない |
| `image_unavailable` | 写真ファイルが移動・削除されている、または画像を読み取れない |
| `daemon_error` | デーモンが返したその他のエラー |

写真 ID は `search_photos` の結果に含まれる値をそのまま使ってください。
