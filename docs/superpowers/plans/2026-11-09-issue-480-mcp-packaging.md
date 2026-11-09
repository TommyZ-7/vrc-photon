# MCP サーバーの配布パッケージ同梱 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** `vrc-photon-mcp` を deb / AppImage / pacman / NSIS の全配布物に同梱し、インストール後に MCP クライアントから起動できる状態にする（#480）。

**Architecture:** 既存の `vrc-photon-updater` と同じ経路を使う。`Packager.toml` の `beforePackagingCommand` に `-p vrc-photon-mcp` を追加し、`[[binaries]]` に `vrc-photon-mcp` を追加する。Release ワークフローで全成果物にバイナリが入っていることと、同梱バイナリが MCP の `initialize` / `tools/list` に応答することを検査する。

**Tech Stack:** Rust stable（edition 2024）, cargo-packager 0.11.8, GitHub Actions, rmcp 3.9

**Spec:** GitHub issue #480（https://github.com/TommyZ-7/vrc-photo-manager/issues/480）

## Global Constraints

- 変更対象は配布設定とその検証に限る。MCP のツール、プロトコル、UI、デーモン IPC は変更しない。
- `Packager.toml` は `cargo-packager` 0.11.8 の既存書式に合わせる。
- 配布物のバイナリ名は `vrc-photon-mcp`（Windows は `vrc-photon-mcp.exe`）。
- Release ワークフローの検証は、既存の `vrc-photon-updater` 検証と同じ成果物（deb / AppImage / pacman `.tar.gz` / NSIS）を対象にする。
- AppImage は単一ファイルのため、MCP バイナリは内部に同梱されるが、クライアントから直接起動するには `--appimage-extract` が必要。この制約は `docs/release.md` に明記する。
- `packaging/licenses/THIRD-PARTY-NOTICES.md` は cargo-about の生成物であり、手で編集しない。`rmcp` を同梱するため再生成する。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- パッケージ自体の最終検証は PR の `Release` ワークフロー（`Packager.toml` 変更で自動実行）で行う。

## Review Focus

1. **deb / AppImage / pacman / NSIS のすべてに MCP バイナリが入る**。Task 1 の `packaging_builds_and_ships_the_mcp_binary` と Task 2 の Release 検証で固定する。
2. **同梱バイナリが実際に MCP サーバーとして起動する**。Task 2 の `initialize` / `tools/list` スモークテストで固定する。
3. **AppImage だけ「入っているのに起動できない」状態にならない**。抽出手順を `docs/release.md` に書き、Task 4 で確認する。
4. **ライセンス通知が同梱クレートを網羅する**。Task 3 で `rmcp` が通知に含まれることを確認する。
5. **既存の更新ヘルパー検証を壊さない**。Task 2 は `vrc-photon-updater` の検査を残したままループ化する。

---

## File Structure

- `Packager.toml`: MCP のビルド対象と同梱バイナリを追加。
- `crates/mcp/tests/packaging.rs`（新規）: 配布設定の回帰テスト。
- `.github/workflows/release.yml`: Linux / Windows の同梱検証と MCP スモークテスト。
- `packaging/licenses/THIRD-PARTY-NOTICES.md`: cargo-about で再生成。
- `docs/release.md`: 生成物、設定、ワークフロー、リリース前チェックを更新。
- `packaging/README.md`: 同梱バイナリの一覧を更新。

---

### Task 1: Packager.toml に MCP バイナリを追加する

**Files:**
- Create: `crates/mcp/tests/packaging.rs`
- Modify: `Packager.toml`

**Interfaces:**
- Consumes: `crates/mcp/Cargo.toml` の `[[bin]] name = "vrc-photon-mcp"`（既存）。
- Produces: `target/release/vrc-photon-mcp` が `cargo build --release -p vrc-photon-mcp` で生成される。

- [x] **Step 1: 失敗する回帰テストを書く**

Create `crates/mcp/tests/packaging.rs`:

```rust
#[test]
fn packaging_builds_and_ships_the_mcp_binary() {
    let packager = include_str!("../../../Packager.toml");

    assert!(
        packager.contains("-p vrc-photon-mcp"),
        "beforePackagingCommand must build vrc-photon-mcp"
    );
    assert!(
        packager
            .lines()
            .any(|line| line.trim() == "path = \"vrc-photon-mcp\""),
        "[[binaries]] must include vrc-photon-mcp"
    );
}
```

- [x] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p vrc-photon-mcp --test packaging`
Expected: FAIL（`beforePackagingCommand must build vrc-photon-mcp`）

- [x] **Step 3: `Packager.toml` を更新する**

`beforePackagingCommand` を次のように変更する:

```toml
beforePackagingCommand = "cargo build --release -p vrc-photon-ui -p vrc-photon-daemon -p vrc-photon-update -p vrc-photon-mcp"
```

`[[binaries]]` の末尾に追加する:

```toml
[[binaries]]
path = "vrc-photon-mcp"
```

- [x] **Step 4: テストが通ることを確認する**

Run: `cargo test -p vrc-photon-mcp --test packaging`
Expected: PASS

- [x] **Step 5: リリースビルドでバイナリが生成されることを確認する**

Run: `cargo build --release -p vrc-photon-mcp && test -x target/release/vrc-photon-mcp && target/release/vrc-photon-mcp --version`
Expected: `vrc-photon-mcp 0.13.0` が表示される。

- [x] **Step 6: コミット**

```bash
git add Packager.toml crates/mcp/tests/packaging.rs
git commit -m "fix: build the MCP server in release packages"
```

---

### Task 2: Release ワークフローで同梱と起動を検証する

**Files:**
- Modify: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: Task 1 の `target/release/vrc-photon-mcp`。
- Produces: 各成果物の `usr/bin/vrc-photon-mcp`（Windows は `vrc-photon-mcp.exe`）の存在検査と、Linux での MCP ハンドシェイク検査。

- [x] **Step 1: Linux の同梱検証をループ化する**

`Verify third-party notices` ステップの更新ヘルパー検査を、次のループに置き換える（ライセンス通知の検査はそのまま残す）:

```yaml
          for binary in vrc-photon-updater vrc-photon-mcp; do
            path="usr/bin/$binary"
            dpkg-deb --contents "$deb" | grep "$path\$" >/dev/null \
              || { echo "missing $path in deb"; exit 1; }
            test -f "squashfs-root/$path" \
              || { echo "missing $path in AppImage"; exit 1; }
            tar -tzf "$pacman" | grep "$path\$" >/dev/null \
              || { echo "missing $path in pacman package"; exit 1; }
          done
          echo "third-party notices and bundled binaries verified"
```

- [x] **Step 2: Linux の MCP スモークテストを追加する**

`Verify third-party notices` の直後に新しいステップを追加する。`squashfs-root` は前のステップで展開済みのものを使う:

```yaml
      - name: Smoke-test the packaged MCP server
        run: |
          set -euo pipefail
          mcp="squashfs-root/usr/bin/vrc-photon-mcp"
          test -x "$mcp"
          {
            printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"release-smoke","version":"0"}}}'
            printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
            printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
          } | timeout 10 "$mcp" > /tmp/mcp-smoke.jsonl 2>/tmp/mcp-smoke.err
          cat /tmp/mcp-smoke.jsonl
          grep -q '"serverInfo"' /tmp/mcp-smoke.jsonl
          grep -q '"library_stats"' /tmp/mcp-smoke.jsonl
          echo "packaged MCP server responded with library_stats"
```

- [x] **Step 3: Windows の同梱検証をループ化する**

`Verify update helper in the installer` ステップを次のように変更する（ステップ名も `Verify bundled binaries in the installer` に変更する）:

```yaml
      - name: Verify bundled binaries in the installer
        shell: pwsh
        run: |
          $installer = Get-ChildItem dist\*-setup.exe | Select-Object -First 1
          if (-not $installer) { throw "no NSIS installer found" }
          $sevenZip = Join-Path $env:ProgramFiles "7-Zip\7z.exe"
          if (-not (Test-Path $sevenZip)) { throw "7-Zip not found" }
          $listing = & $sevenZip l $installer.FullName
          foreach ($binary in @('vrc-photon-updater.exe', 'vrc-photon-mcp.exe')) {
            if (-not ($listing | Select-String -Quiet ([regex]::Escape($binary)))) {
              throw "$binary is missing from $($installer.Name)"
            }
          }
          Write-Host "update helper and MCP server found in $($installer.Name)"
```

- [x] **Step 4: YAML の構文を確認する**

Run: `ruby -e 'require "yaml"; YAML.load_file(".github/workflows/release.yml"); puts "yaml ok"'`
Expected: `yaml ok`

Ruby がない場合は `npx --yes yaml-lint .github/workflows/release.yml` を使う。
どちらも使えない場合は、このステップは PR の Release ワークフロー起動で代替し、その旨を報告する。

- [x] **Step 5: コミット**

```bash
git add .github/workflows/release.yml
git commit -m "ci: verify the MCP server in release packages"
```

---

### Task 3: サードパーティ通知を再生成する

**Files:**
- Modify: `packaging/licenses/THIRD-PARTY-NOTICES.md`

**Interfaces:**
- Consumes: `about.toml` / `about.hbs`（既存）。
- Produces: `rmcp 3.9.0` を含む通知ファイル。

- [x] **Step 1: cargo-about で再生成する**

Run: `cargo about generate about.hbs -o packaging/licenses/THIRD-PARTY-NOTICES.md`

- [x] **Step 2: `rmcp` が含まれることを確認する**

Run: `grep -a -n 'rmcp 3.9.0' packaging/licenses/THIRD-PARTY-NOTICES.md | head`
Expected: `rmcp 3.9.0` を含む行が1件以上。

- [x] **Step 3: 差分が生成物だけであることを確認する**

Run: `git status --short && git diff --stat packaging/licenses/THIRD-PARTY-NOTICES.md`
Expected: 変更は `packaging/licenses/THIRD-PARTY-NOTICES.md` のみ。

差分に無関係な大量の変更が含まれる場合は、コミットせずに停止して報告する（手編集はしない）。

- [x] **Step 4: コミット**

```bash
git add packaging/licenses/THIRD-PARTY-NOTICES.md
git commit -m "chore: refresh third-party notices for the MCP server"
```

---

### Task 4: リリース・パッケージ文書を更新する

**Files:**
- Modify: `docs/release.md`
- Modify: `packaging/README.md`

**Interfaces:**
- Consumes: Task 1 の `Packager.toml`、Task 2 のワークフロー検査、Task 3 の通知。
- Produces: 同梱物と検証手順が文書と一致した状態。

- [x] **Step 1: `docs/release.md` の生成物一覧を更新する**

- Debian の内容: `UI・デーモン・更新ヘルパー・MCPサーバー`
- AppImage の内容: `UI・デーモン・更新ヘルパー・MCPサーバー`
- pacman（手動 / AUR 用）: `更新ヘルパー・MCPサーバー、systemdユーザーunit、polkit policy`
- NSIS: `UI・デーモン・更新ヘルパー・MCPサーバー`
- AppImage の直後に、MCP クライアントから使うには
  `./vrc-photon_<version>_x86_64.AppImage --appimage-extract` で展開し、
  `squashfs-root/usr/bin/vrc-photon-mcp` を指定する旨を追記する。

- [x] **Step 2: `docs/release.md` の `Packager.toml` 設定を更新する**

- `beforePackagingCommand` の末尾に `-p vrc-photon-mcp` を追加。
- `[[binaries]]` の一覧に `vrc-photon-mcp` を追加。

- [x] **Step 3: `docs/release.md` のワークフロー説明とリリース前チェックを更新する**

- `Verify third-party notices` の説明に `vrc-photon-mcp` を追加。
- `Smoke-test the packaged MCP server` の説明を追加。
- Windows の検証説明に `vrc-photon-mcp.exe` を追加。
- リリース前チェックに
  `- [x] パッケージ生成物に vrc-photon-mcp が入っている` を追加。

- [x] **Step 4: `packaging/README.md` に同梱バイナリを追記する**

`cargo-packager builds three Linux artifacts into dist/:` の後に、次の一覧を追加する:

```markdown
All artifacts include the UI, daemon, update helper, and MCP server:

- `vrc-photon`
- `vrc-photon-daemon`
- `vrc-photon-updater`
- `vrc-photon-mcp`

The AppImage stores them inside the image. To use the MCP server with an
AppImage, extract it first with `--appimage-extract` and point the MCP
client at `squashfs-root/usr/bin/vrc-photon-mcp`.
```

- [x] **Step 5: 文書の記述が実際の設定と一致することを確認する**

Run: `grep -n "vrc-photon-mcp" Packager.toml docs/release.md packaging/README.md`
Expected: 3ファイルすべてに `vrc-photon-mcp` が含まれる。

- [x] **Step 6: コミット**

```bash
git add docs/release.md packaging/README.md
git commit -m "docs: document the bundled MCP server"
```

---

### Task 5: 全体検証

**Files:**
- なし（検証のみ）

- [x] **Step 1: ローカル検証を実行する**

Run:

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Expected: すべて PASS。

- [x] **Step 2: 配布設定の回帰テストを再実行する**

Run: `cargo test -p vrc-photon-mcp --test packaging`
Expected: PASS。

- [x] **Step 3: ブランチを push して PR を作成する**

PR には `bug` ラベルを付ける。`Packager.toml` と `.github/workflows/release.yml` の変更により
Release ワークフローの bundle ジョブが自動で走る。

- [x] **Step 4: Release ワークフローの成果物を確認する**

`ci-watch` スキルで `CI` と `Release` の両方を監視する。成功後、少なくとも
`Verify third-party notices` と `Smoke-test the packaged MCP server` が
緑であることを確認する。

- [x] **Step 5: 計画のチェックボックスを更新する**

実行完了時にこのファイルのチェックボックスを `- [x]` に更新する。
計画ファイルを PR に含めるかは、PR 作成時にユーザーの指示に従う。

---

## 決定事項（承認済み）

1. **AppImage の扱い**: 同梱のみとし、抽出手順を文書化する。AppImage から
   直接 `--mcp` で起動できる導線が必要なら、別 issue とする。
2. **MCP の利用手順ドキュメント**: 今回の PR には含めず、マージ後に別の
   docs issue とする。
3. **サードパーティ通知の再生成**: `rmcp` 同梱のため Task 3 に含める。
4. **計画ファイル**: この PR に含める。
