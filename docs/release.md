# リリースと配布

## 1. バージョン

- 単一の情報源: ワークスペース `Cargo.toml` の `[workspace.package] version`
- 各クレートは `version.workspace = true` を継承する。
- バンプは `Cargo.toml`／`Packager.toml`／`Cargo.lock`／`packaging/licenses/THIRD-PARTY-NOTICES.md` の4ファイルを同じPRで更新する。`THIRD-PARTY-NOTICES.md` は `cargo about generate about.hbs --locked -o <file>` で再生成する。
- 版番号は CalVer `YY.MM.MICRO` とする。タグは `v<YY>.<MM>.<MICRO>`（例: 2026年11月の最初のリリースは `v26.11.0`）。
  - `YY` は西暦の下2桁、`MM` は月（ゼロ埋めしない。1〜9月は1桁）、`MICRO` はその月内の通し番号（0始まり。同月内は +1、月が変われば 0 に戻す）。
  - 形式は SemVer として解釈可能な3要素の数値を維持し、Cargo／Packager／アップデータの版比較は従来どおり動作する。
  - 破壊的変更（DB 再構築等）の警告は版番号では表さず、リリースノートで明示する。

## 2. リリースフロー

1. `main` を最新にする。
2. バージョンを更新する。
3. 作業ブランチ → PR。
4. CI（`test`）とRelease（パッケージビルド）が緑になる。
5. `main` へマージ。
6. **ユーザーの明示指示がある場合のみ**タグを作成・pushする。

```bash
git tag v0.1.0
git push origin v0.1.0
```

タグpushでReleaseワークフローが走り、GitHub ReleaseにLinux 3形式とWindowsインストーラが添付される。

### リリースノート

- ノートは GitHub の自動生成（generate-notes API）で**マージ済み PR** から作られる。分類は `.github/release.yml` のカテゴリ定義に従う（`enhancement` → ✨ Added、`bug` → 🐛 Fixed、それ以外 → 🔧 Improved）。PR にラベルが付いていない場合は 🔧 Improved に入る
- バージョン bump の PR 自体はノートに載らない
- どの PR にも属さない直接 push のコミットは「その他の変更」として末尾に追記される
- リリースを先に手動作成した場合も、publish ジョブがノート本文を自動生成の内容で上書きする（アセットは従来どおりアップロードされる）

ラベルとカテゴリの対応（`.github/release.yml`）:

| PR のラベル | カテゴリ |
|---|---|
| `enhancement` | ✨ Added |
| `bug` | 🐛 Fixed |
| 上記以外・ラベルなし | 🔧 Improved |
| `duplicate` / `invalid` / `wontfix` | 掲載されない |

- ラベルは PR 作成時に付ける（AGENTS.md §1 の規則）。マージ後に付けたラベルも、その PR を含むリリースのノートが生成される前であれば反映される

## 3. 生成物

`cargo-packager` が `dist/` に以下を生成する。`.pkg.tar.zst` だけは
Release ワークフローが `makepkg` で追加生成する。

| 形式 | 例 | 内容 |
|---|---|---|
| Debian | `vrc-photon_0.1.0_amd64.deb` | `/usr/bin` にUI・デーモン・更新ヘルパー・MCPサーバー、desktop entry、アイコン、systemdユーザーunit、polkit policy |
| AppImage | `vrc-photon_0.1.0_x86_64.AppImage` | UI・デーモン・更新ヘルパー・MCPサーバーを同梱した単一ファイル（polkit policy は含まない） |
| pacman（手動 / AUR 用） | `vrc-photon_0.1.0_x86_64.tar.gz` + `PKGBUILD` | Arch Linux向け。更新ヘルパー・MCPサーバー、systemdユーザーunit、polkit policyを含む |
| pacman（`pacman -U` 用） | `vrc-photon-0.1.0-1-x86_64.pkg.tar.zst` | Releaseワークフローが `makepkg` で追加生成する実パッケージ。アプリ内更新はこれを使う |
| NSIS | `vrc-photon_0.1.0_x64-setup.exe` | Windowsインストーラ（UI・デーモン・更新ヘルパー・MCPサーバー） |

AppImage に同梱した MCP サーバーを MCP クライアントから使う場合は、
`./vrc-photon_<version>_x86_64.AppImage --appimage-extract` で展開し、
`squashfs-root/usr/bin/vrc-photon-mcp` を実行する。deb / pacman / NSIS では
`vrc-photon-mcp`（Windows は `vrc-photon-mcp.exe`）が他のバイナリと同じ場所に
インストールされる。クライアントからの設定方法と提供ツールは
[mcp.md](mcp.md) を参照。

`Packager.toml` の設定:

- `productName = "VRC PhotoN"`
- `identifier = "com.tommyz.vrc-photon"`
- `formats = ["deb", "appimage", "pacman"]`（Windowsジョブは `-f nsis` で上書き）
- `beforePackagingCommand = "cargo build --release -p vrc-photon-ui -p vrc-photon-daemon -p vrc-photon-update -p vrc-photon-mcp"`
- `[[binaries]]`: `vrc-photon`（main）、`vrc-photon-daemon`、`vrc-photon-updater`、`vrc-photon-mcp`
- `binariesDir = "target/release"` / `outDir = "dist"`
- アイコン: `packaging/icons/icon.png`（512px、Linuxパッケージ）/ `packaging/icons/icon.ico`（Windows実行ファイルに埋め込み）
- `resources = ["LICENSE", "packaging/licenses/LICENSE-LUCIDE"]`
- deb依存: `libxkbcommon0`, `libxkbcommon-x11-0`, `libwayland-client0`, `libvulkan1`, `libx11-6`, `libxcb1`, `libfontconfig1`, `libfreetype6`
- pacman依存: `libxkbcommon`, `wayland`, `vulkan-icd-loader`, `libx11`, `libxcb`, `fontconfig`, `freetype2`
- `[deb.files]` / `[pacman.files]`: `packaging/systemd/vrc-photon-daemon.service` を `usr/lib/systemd/user/vrc-photon-daemon.service` に配置
- `[deb.files]` / `[pacman.files]`: `packaging/polkit/com.tommyz.vrc-photon.update.policy` を `usr/share/polkit-1/actions/com.tommyz.vrc-photon.update.policy` に配置

Windows の4実行ファイル（UI・デーモン・更新ヘルパー・MCPサーバー）は、
各クレートの `build.rs` が `build-support/windows_icon.rs` を共有して
`packaging/icons/icon.ico` をリソースID 1 で埋め込む。GPUI は同じIDを
ウィンドウアイコンとして読み込む。ICOの再生成手順は
[packaging/README.md](../packaging/README.md) を参照。

`resources` のライセンス通知は、Linux（deb / AppImage / pacman）では
`/usr/lib/vrc-photon/`、Windows（NSIS）では実行ファイルと同じディレクトリに配置される。
由来と更新手順は [licenses.md](licenses.md) を参照。

## 4. パッケージ起動時の挙動

- UIバイナリとデーモンバイナリは同じディレクトリに配置される。
- UI起動時、ハートビートが古ければ同じディレクトリのデーモンを自動起動する。
- デーモンは `single-instance` で二重起動を防ぐ。
- 自動起動:
  - deb/pacman: `/usr/bin/vrc-photon-daemon` を登録。
  - AppImage: `$APPIMAGE --background` を登録。UIの `--background` がデーモンを子プロセスで起動し、終了まで待ってAppImageのマウントを維持する。
- deb/pacman は任意の systemd ユーザーサービス `vrc-photon-daemon.service` を同梱する。インストール時に enable はされない。アプリ管理の自動起動はフォールバックとして残る。
- LinuxのトレイはStatusNotifierItem（`ksni`）。トレイ非対応環境ではヘッドレスで動作する。

### systemdユーザーサービス（deb / pacman、任意）

unit は `/usr/lib/systemd/user/vrc-photon-daemon.service` に配置される。

アプリ内更新でデーモンを再起動するとき、UIは `app_settings.daemon_supervisor` が
`systemd` なら `systemctl --user daemon-reload` と
`systemctl --user restart vrc-photon-daemon.service` を使う。失敗時は従来の
プロセス起動にフォールバックする。これにより、更新後のデーモンが unit 外の
プロセスにならない。

有効化の前に、UIの「設定 → 起動 → 自動起動」をOFFにする。ONのままだとログイン時にXDG側が先にデーモンを起動し、systemdサービスがinactiveのまま残る競合が起こり得る。

```bash
systemctl --user daemon-reload
systemctl --user enable --now vrc-photon-daemon.service
systemctl --user is-active vrc-photon-daemon.service
journalctl --user -u vrc-photon-daemon.service -f
```

- サービスは `graphical-session.target` に紐付く。GNOME / KDE など、この target を管理するデスクトップ環境でログイン時に起動する。
- `graphical-session.target` を管理しないデスクトップ環境では、代わりに `default.target` へリンクする:

```bash
systemctl --user add-wants default.target vrc-photon-daemon.service
```

- 既にデーモンが動いている状態で `enable --now` すると、サービスは単一インスタンス判定で終了して inactive になる。既存のデーモンを終了してから `systemctl --user restart vrc-photon-daemon.service` を実行する。ログイン後に `systemctl --user is-active` が `active` でない場合も同じ手順で復旧する。
- アンインストール前に無効化する:

```bash
systemctl --user disable --now vrc-photon-daemon.service
systemctl --user remove-wants default.target vrc-photon-daemon.service
systemctl --user daemon-reload
```

パッケージマネージャは unit ファイルを削除するが、`systemctl --user enable` が作るシンボリックリンクは `~/.config/systemd/user/graphical-session.target.wants/` に残る。`default.target` へ `add-wants` した場合は `default.target.wants/` にも残るため、先に disable / remove-wants しておくことで、アンインストール後にサービス定義が残らない。

## 5. ローカルでのパッケージビルド

```bash
cargo install cargo-packager --locked --version 0.11.8
cargo packager --release
ls -lh dist
```

前提:

- CMake / Ninja / Clang / pkg-config
- `libxkbcommon-x11-dev` などのX11開発パッケージ
- `cargo-packager` 0.11.8
- AppImageビルド時に `linuxdeploy` を自動取得するためネットワークが必要

## 6. Releaseワークフロー

`.github/workflows/release.yml`:

- `push: tags: ['v*.*.*']`
- `pull_request`（release-relevant pathsのみ）
- `workflow_dispatch`
- Ubuntuランナー: 依存を入れ、`cargo packager --release`
- `Build pacman package`: `archlinux:base-devel` コンテナ内で非 root ユーザーが
  `dist/PKGBUILD` と `dist/vrc-photon_<version>_x86_64.tar.gz` から
  `makepkg -f --nodeps` を実行し、`dist/*.pkg.tar.zst` を生成する。
  `LC_ALL=C pacman -Qip` の出力から `Name: vrc-photon`、
  `Version: <version>-1`、`Architecture: x86_64` を検証する
- `Verify third-party notices`: deb / AppImage / pacman（`.tar.gz`）に
  `LICENSE` と `LICENSE-LUCIDE` が含まれること、および
  `usr/bin/vrc-photon-updater` と `usr/bin/vrc-photon-mcp` が含まれることを検査する
- `Smoke-test the packaged MCP server`: AppImage から展開した
  `vrc-photon-mcp` に `initialize` と `tools/list` を送り、
  `library_stats` が返ることを検査する
- `Verify systemd user unit`: deb / pacman に unit が入り、`ExecStart` が
  `/usr/bin/vrc-photon-daemon` であることを検査する
- `Verify polkit policy`: deb / pacman（`.pkg.tar.zst`）の両方に
  `com.tommyz.vrc-photon.update.policy` が入り、
  `org.freedesktop.policykit.exec.path` が `/usr/bin/vrc-photon-updater` で
  あることを検査する
- Windowsランナー: `cargo packager --release -f nsis`（NSISは自動取得）
- Windows の `Verify bundled binaries in the installer`: NSIS に
  `vrc-photon-updater.exe` と `vrc-photon-mcp.exe` が含まれることを検査する
- 両ジョブの `dist/*` をworkflow artifactとして保存
- タグ時のみ `publish` ジョブが両artifactをまとめて `gh release create/upload` で公開
- `publish` ジョブはタグと `Cargo.toml` / `Packager.toml` のバージョン一致を確認し、
  `MINISIGN_SECRET_KEY` がなければ失敗する
- プレリリースタグ（`-` を含むもの）は安定版フィードに混ざらないよう拒否する
- `vrc-photon-release-tool` は秘密鍵を渡さないステップでビルドし、署名ステップでは
  実行だけ行う
- `vrc-photon-release-tool` で全artifactと `latest.json` を署名・検証し、
  `latest.json` を同じReleaseに添付する。`generate` は AppImage / NSIS に加えて
  `.pkg.tar.zst` と `.deb` が各1個あること、ファイル名にタグバージョンが
  含まれることを要求する
- `generate --ai-plugin packaging/ai-plugin.json` は、最新の AI プラグイン版
  （`version`）とその版が要求する本体の最小版（`host_version_min`）を
  `latest.json` の任意欄 `ai_plugin` に埋め込む
- `jq -e '.schema == 1 and (.platforms | length == 2) and (.system_packages | length == 2) and (.system_packages | has("pacman") and has("deb")) and (.system_packages.pacman.format == "pacman") and (.system_packages.deb.format == "deb") and ((.ai_plugin.version | type) == "string") and ((.ai_plugin.host_version_min | type) == "string")' dist/latest.json`
  で `system_packages` が pacman / deb の2件だけを持ち、`format` が一致し、
  `ai_plugin` の2フィールドが文字列であることを検査する

Releaseノートの分類は `.github/release.yml` と GitHub の generate-notes API に従う。

## 7. CIとブランチ保護

- `.github/workflows/ci.yml`: fmt / clippy / test。
- docs-only PRは `changes` ジョブでfast-pass。
- `main` の保護:
  - 必須チェック: `test`
  - 直接push禁止
  - force push / branch deletion禁止
  - 会話解決必須
  - 管理者にも適用

Required check名を変更した場合は、GitHubのbranch protectionも更新する。

## 8. リリース前チェック

- [ ] `Cargo.toml` と `Cargo.lock` のバージョンが一致
- [ ] CI / Release（PR）が緑
- [ ] 破壊的DB変更がない、または旧DBの扱い（v1のみ・1.0.0未満は自動移行せず明示エラー）がリリースノートに明記されている
- [ ] パッケージ生成物にUIとデーモンの両方が入っている
- [ ] deb / pacman に `usr/lib/systemd/user/vrc-photon-daemon.service` が入っている
- [ ] アイコンとdesktop entryが入っている
- [ ] パッケージ生成物に `LICENSE` と `LICENSE-LUCIDE` が入っている
- [ ] パッケージ生成物に `vrc-photon-updater` が入っている
- [ ] パッケージ生成物に `vrc-photon-mcp`（Windows は `vrc-photon-mcp.exe`）が入っている
- [ ] deb / pacman に `usr/share/polkit-1/actions/com.tommyz.vrc-photon.update.policy` が入り、`exec.path` が `/usr/bin/vrc-photon-updater` である
- [ ] `dist/*.pkg.tar.zst` が存在し、`LC_ALL=C pacman -Qip` で `Name: vrc-photon`、`Version: <version>-1`、`Architecture: x86_64` を返す
- [ ] `latest.json` の `system_packages` が pacman / deb の2件で、両方に署名済みURL・サイズ・SHA-256・署名がある
- [ ] `packaging/ai-plugin.json` が最新の AI プラグイン版と `host_version_min` を持ち、`latest.json` の `ai_plugin` と一致する
- [ ] `MINISIGN_SECRET_KEY` が登録されている
- [ ] `latest.json` と `latest.json.minisig` がReleaseに添付されている
- [ ] リリースノートの分類が正しい

## 9. ロールバック

- 不具合のあるリリースは、修正版を新しいバージョン（CalVer では次の MICRO）で出す。
- 既存タグの付け替えはしない。
- GitHub Releaseを非公開/削除する場合も、タグは残す。ユーザーの明示指示が必要。
- DB は v1 のみをサポートする。1.0.0 未満の DB は自動移行せず、明示エラーで開かない。
  アップグレード前にバックアップを案内し、ダウングレードは保証しない。

## 10. 自動アップデートの署名と配信

### 10.1 フィード

- 更新確認は `https://github.com/TommyZ-7/vrc-photon/releases/latest/download/latest.json`
  を取得する。GitHub API は使わない。
- `latest.json` の隣に `latest.json.minisig` を置く。
- `latest.json` の `platforms` は `linux-x86_64`（AppImage）と
  `windows-x86_64`（NSIS）を持つ。各エントリは `url`、`format`、`size`、
  `sha256`、`signature`（`.minisig` 全体のbase64）を含む。
- `latest.json` の `system_packages` は pacman / deb のシステムパッケージ更新に
  使う任意のマップ。キーは `pacman` / `deb` で、各エントリは `platforms` と
  同じ `url`、`format`、`size`、`sha256`、`signature` を持つ。
  - `pacman` の `url` は `*.pkg.tar.zst`、`deb` の `url` は `*.deb`。
  - `schema` は 1 のまま。`system_packages` を追加しても `platforms` の
    検証は変わらないため、この機能より前のクライアントは更新確認を続けられる。
  - 新しいクライアントは `system_packages` がない旧フィードでは
    パッケージマネージャのコマンド表示にフォールバックする。
- `latest.json` の `ai_plugin` は任意のオブジェクトで、`version`（最新の AI
  プラグイン版）と `host_version_min`（その版が要求する本体の最小版）を持つ。
  - 単一情報源は `packaging/ai-plugin.json`。autotag のリリース時に、リリースした
    プラグインの `plugin_version` と manifest の `host_version_min` でこのファイルを
    更新し、photon のリリース前に `main` へマージする。初期値は
    `version: 0.5.1`、`host_version_min: 1.0.0`。
  - `schema` は 1 のまま。`ai_plugin` を知らない旧クライアントは未知フィールドとして
    無視する。
  - `ai_plugin` が semver でない、型が違う、キーが欠落している場合は、プラグイン
    通知だけを出さず、本体の更新確認は継続する。
- クライアントは本体の更新確認で取得した `ai_plugin` を
  `updates/ai-plugin.json` にキャッシュする。再起動後や24時間スロットル中も
  追加通信なしで比較し、フィードから `ai_plugin` が消えたらキャッシュを削除する。
- 通知条件は、`ai_plugin.version` がアクティブな導入版より新しいこと、かつ
  `ai_plugin.host_version_min` が現在の本体版以下であること。設定画面の
  AI タグ付けカードに表示するだけで、プラグインのダウンロード・自動更新は行わない。
  導入済みの `.vpp` ボタンは「プラグインを更新」に切り替わり、未導入では
  「プラグイン (.vpp) を取り込む」のまま。
- クライアントはマニフェスト署名をJSONパース前に検証し、成果物は
  ダウンロードしながら署名とSHA-256を検証する。システムパッケージの適用時は
  特権ヘルパーが同じマニフェスト署名と成果物を再検証する。

### 10.2 署名鍵

- 秘密鍵は GitHub Actions の repository secret `MINISIGN_SECRET_KEY` にだけ置く。
  値は `crates/update/examples/generate_update_key.rs` が生成する
  minisign secret key box のファイル内容。
- 公開鍵は `packaging/update/keys.txt` に1行1鍵で置く。アプリはこのファイルを
  `include_str!` で埋め込み、複数鍵のどれかで検証できれば成功とする。
- 秘密鍵のオフラインバックアップを必ず保管する。GitHub secretは再表示できない。
- 鍵の生成:

```bash
cargo run -p vrc-photon-update --example generate_update_key -- /secure/path/minisign.key
```

  出力された `RW...` 行を `packaging/update/keys.txt` に追記し、秘密鍵を
  `gh secret set MINISIGN_SECRET_KEY < /secure/path/minisign.key` で登録する。
  秘密鍵はリポジトリに置かない。

- ローテーション:
  1. 新しい鍵を生成し、公開鍵を `keys.txt` に追記する。
  2. そのバージョンを旧鍵で署名してリリースする。
  3. 次のリリースから `MINISIGN_SECRET_KEY` を新鍵に切り替える。
  4. 旧鍵での更新が行き渡った後、旧公開鍵を `keys.txt` から外す。
  旧鍵が漏洩した場合は 2〜4 を待たず、手動更新を案内する。

### 10.3 ローカルでの署名・検証

`vrc-photon-release-tool` はRelease専用で、アプリには同梱しない。

```bash
cargo run -p vrc-photon-release-tool --bin vrc-photon-update-manifest -- \
  sign --key /secure/path/minisign.key --dist dist
PUB_ARGS=()
while read -r key; do PUB_ARGS+=(--public-key "$key"); done \
  < <(grep '^RW' packaging/update/keys.txt)
cargo run -p vrc-photon-release-tool --bin vrc-photon-update-manifest -- \
  verify "${PUB_ARGS[@]}" --dist dist
cargo run -p vrc-photon-release-tool --bin vrc-photon-update-manifest -- \
  generate --dist dist --tag v0.4.0 --repo TommyZ-7/vrc-photon \
  --notes-url https://github.com/TommyZ-7/vrc-photon/releases/tag/v0.4.0 \
  --output dist/latest.json
cargo run -p vrc-photon-release-tool --bin vrc-photon-update-manifest -- \
  sign --key /secure/path/minisign.key --file dist/latest.json
cargo run -p vrc-photon-release-tool --bin vrc-photon-update-manifest -- \
  verify "${PUB_ARGS[@]}" --file dist/latest.json
```

`sign` / `verify` / `generate` は `dist` 内の `*.minisig` を除く全ファイルを
対象にする。`generate` は AppImage と NSIS に加えて `.pkg.tar.zst` と `.deb` が
各1個あること、ファイル名にタグバージョンが含まれることを要求する。

pacman の `.pkg.tar.zst` は `cargo packager` の直後、署名の前に生成する。
`makepkg` は root で実行できないため、Arch Linux コンテナ内の非 root ユーザーを
使う。

```bash
docker run --rm -v "$PWD/dist:/dist" -w /dist archlinux:base-devel bash -c '
  set -euo pipefail
  owner=$(stat -c "%u:%g" /dist)
  trap "chown -R $owner /dist" EXIT
  useradd -m builder
  chown -R builder:builder /dist
  su builder -c "makepkg -f --nodeps"
  LC_ALL=C pacman -Qip /dist/*.pkg.tar.zst
  rm -rf /dist/src /dist/pkg
'
```

- `dist/PKGBUILD` と `dist/vrc-photon_<version>_x86_64.tar.gz` は
  `cargo packager` が生成する。`.tar.gz` は `makepkg` の入力であり、
  `pacman -U` では直接インストールできない。
- 生成された `.pkg.tar.zst` も `sign --dist` の対象になる。
- `.tar.gz` と `PKGBUILD` は手動インストール / AUR 用として Release に残す。

### 10.4 polkit policy とシステムパッケージ更新

設計判断は [ADR-0016](adr/0016-system-package-update.md) を参照。

deb / pacman には次の polkit policy を同梱する。

- ソース: `packaging/polkit/com.tommyz.vrc-photon.update.policy`
- 配置先: `/usr/share/polkit-1/actions/com.tommyz.vrc-photon.update.policy`
- action: `com.tommyz.vrc-photon.update-package`
- `allow_any` / `allow_inactive` / `allow_active`: `auth_admin`
- `org.freedesktop.policykit.exec.path`: `/usr/bin/vrc-photon-updater`

AppImage には同梱しない。policy がない環境、`pkexec` がない環境、
`/usr/bin/vrc-photon-updater` がない環境では、アプリは自動適用を隠して
パッケージマネージャのコマンドを表示する。

アプリ内更新の流れ:

1. UI が署名済み manifest の `system_packages` から pacman / deb の成果物を
   選び、`<data_dir>/updates/downloads/<version>/` にダウンロードする。
2. UI は `updates/bin/vrc-photon-updater --apply <job.json>` を起動する。
   job には署名対象の `latest.json` 生テキストと `.minisig` も入る。
3. ユーザー権限ヘルパーが staging を再検証し、
   `pkexec --disable-internal-agent /usr/bin/vrc-photon-updater --apply-system-package <job.json>`
   を実行する。
4. 特権ヘルパーが staging の所有者・symlink・署名・バージョン・パッケージ
   メタデータを再検証し、固定コマンドでインストールする。
   - pacman: `LC_ALL=C /usr/bin/pacman -U --noconfirm <temp>`
   - deb: `LC_ALL=C DEBIAN_FRONTEND=noninteractive /usr/bin/dpkg -i <temp>`
5. ユーザー権限ヘルパーが新 UI を起動し、健康マーカーを確認する。新 UI は
   `complete_update_startup` でデーモンを起動し、systemd 管理なら
   `systemctl --user restart vrc-photon-daemon.service` を使う。
6. 結果は `<data_dir>/updates/last-result.json` と `updater.log` に残る。

システムパッケージは自動ロールバックしない。認証キャンセル・DB ロック・
署名不正などインストール前の失敗では job と staged を掃除し、旧 UI を再起動
する。`pacman -U` / `dpkg -i` の失敗や健康確認の失敗では、調査用に job /
staged / log を残して `target` を一度だけ起動する。

### 10.5 手動復旧

AppImage:

```bash
ls -l "/path/to/VRC PhotoN.AppImage.previous"
cp "/path/to/VRC PhotoN.AppImage.previous" "/path/to/VRC PhotoN.AppImage"
chmod +x "/path/to/VRC PhotoN.AppImage"
```

Windows（current-user NSIS）:

```powershell
$backup = "$env:LOCALAPPDATA\vrc-photon\updates\backup\<version>"
Copy-Item "$backup\vrc-photon.exe" "$env:LOCALAPPDATA\VRC PhotoN\vrc-photon.exe" -Force
Copy-Item "$backup\vrc-photon-daemon.exe" "$env:LOCALAPPDATA\VRC PhotoN\vrc-photon-daemon.exe" -Force
```

pacman / deb（システムパッケージ）:

自動ロールバックはない。失敗した場合は `last-result.json` のコードと
`updater.log` を確認し、パッケージマネージャで手動復旧する。

```bash
# pacman: リリースページから .pkg.tar.zst を取得して再インストール
sudo pacman -U /path/to/vrc-photon-<version>-1-x86_64.pkg.tar.zst

# deb: リリースページから .deb を取得して再インストール
sudo dpkg -i /path/to/vrc-photon_<version>_amd64.deb
sudo apt-get install -f
```

`pacman -U` / `dpkg -i` が途中で失敗した場合は、まずパッケージマネージャの
状態を確認する。pacman は `/var/lib/pacman/db.lck`、deb は
`/var/lib/dpkg/lock-frontend` のロックを確認する。実行中の pacman / dpkg /
apt プロセスがないこと（例: `pgrep -a pacman`、`pgrep -a dpkg`）と、ロックを
保持しているプロセスがないこと（例: `sudo fuser /var/lib/pacman/db.lck`）を
確認してからロックを解放し、再試行する。実行中にロックファイルを削除しない。
健康確認だけが失敗した場合は、新しいバージョンは既にインストールされているため、
`updater.log` を確認して UI を手動で起動する。

`<data_dir>/updates/updater.log` と `last-result.json` に失敗理由が残る。
`last-result.json` はUIが起動時に一度読んで削除する。インストール前の失敗では
job と staged が掃除され、インストール後・健康確認の失敗、および
`install_outcome_unknown`（exit 16 / シグナル）では調査用に残る。

## 11. 未対応

- MSIインストーラ（WiX）
- Windows Authenticode コード署名
- システムパッケージの自動ロールバック
- Flatpak / Snap / AUR
- UIを閉じている間の更新確認

これらは GitHub Issues で優先度を決める。
