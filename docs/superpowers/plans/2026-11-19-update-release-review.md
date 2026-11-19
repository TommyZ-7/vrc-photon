# 更新・リリース系レビュー対応 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Issue #753 / #756 / #769 のレビュー指摘を update クレートと release-tool に反映する。

**Architecture:** `ApplyJob` の manifest ペアを全適用種別で必須化し、ユーザーモードヘルパーでも
signed manifest との結合検証を行う。補助的に liveness probe を syscall 化し、Windows の
状態ファイル書き込みを原子的にし、release-tool の manifest 生成時に署名を再検証する。

**Tech Stack:** Rust (edition 2024), minisign, libc, windows 0.62, GitHub Actions

**Spec:** Issue #753, #756, #769 (本文をこの計画の仕様とする)

## Global Constraints

- 既存の API 形状とエラーバリアント (`UpdateError`) を維持する。
- 新規依存を追加しない。`windows` の feature 追加のみ許可。
- `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` を通す。
- リリース成果物の挙動を変えるため、UI 側の job 生成も同じ PR で更新する。

## Review Focus

- manifest を持たない旧形式 job: 適用前に明確な `InvalidJob` で失敗する。
- manifest の asset sha256 と staged ファイルが不一致: 適用前に拒否する。
- クエリ付き feed URL: `.minisig` を同じパスの末尾拡張として取得する。
- Windows で rename 直前にクラッシュ: `job.json` / `last-result.json` が消えない。
- release-tool の stale 署名: generate が出版前に失敗する。

---

### Task 1: process_alive を syscall 化する (#756)

**Files:**
- Modify: `crates/update/src/helper.rs:1873-1884`
- Test: `crates/update/src/helper.rs` の `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: なし
- Produces: `pub(crate) fn process_alive(pid: u32) -> bool` (シグネチャ不変)

- [ ] **Step 1: 失敗するテストを書く**

`helper.rs` のテストモジュールに追加:

```rust
#[test]
fn process_alive_reports_the_current_process_and_a_missing_pid() {
    assert!(process_alive(std::process::id()));
    assert!(!process_alive(u32::MAX));
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p vrc-photon-update process_alive_reports -- --nocapture`
Expected: コンパイルは通るが `u32::MAX` が Linux の `/proc` 判定で false になるため現状でも通る。
このテストは契約の固定用であり、次の置換の回帰を防ぐ。

- [ ] **Step 3: BSD/macOS 分岐を libc に置換**

```rust
#[cfg(all(unix, not(target_os = "linux")))]
{
    // SAFETY: signal 0 は権限と存在だけを確認し、シグナルを配送しない。
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
```

- [ ] **Step 4: テストと clippy を実行**

Run: `cargo test -p vrc-photon-update process_alive && cargo clippy -p vrc-photon-update --all-targets -- -D warnings`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/update/src/helper.rs
git commit -m "fix: probe process liveness without PATH-dependent kill (#756)"
```

---

### Task 2: manifest を全適用種別で必須化し staged と結合する (#753)

**Files:**
- Modify: `crates/update/src/manifest.rs` (`asset_for_apply` 追加)
- Modify: `crates/update/src/install.rs:431-530` (`ApplyJob::validate`)
- Modify: `crates/update/src/helper.rs:359-390` (`verify_staged`), `:541-620` (`verify_manifest`)
- Modify: `crates/ui/src/update_view.rs:1141-1155` (job 生成)
- Test: `crates/update/src/install.rs`, `crates/update/src/helper.rs`

**Interfaces:**
- Produces: `impl Manifest { pub fn asset_for_apply(&self, kind: &ApplyKind) -> Option<&PlatformAsset> }`
- Produces: `fn verify_manifest(job: &ApplyJob, options: &HelperOptions) -> Result<(Manifest, Version, PlatformAsset)>`（全 kind 対応）
- Consumes: `ApplyJob { signed_manifest: Option<String>, manifest_signature: Option<String>, .. }`

- [ ] **Step 1: 失敗するテストを書く**

`install.rs` のテストで、manifest を持たない AppImage job が validate に失敗することを固定:

```rust
#[test]
fn every_apply_kind_requires_manifest_and_signature() {
    let dir = tempfile::tempdir().unwrap();
    let mut job = test_job(dir.path());
    job.signed_manifest = None;
    assert!(job.validate().is_err());

    let mut job = test_job(dir.path());
    job.manifest_signature = None;
    assert!(job.validate().is_err());
}
```

既存の `test_job` は `signed_manifest: Some("{\"schema\":1}".to_string())`,
`manifest_signature: Some("signature".to_string())` を持つよう更新する。

`helper.rs` に AppImage fixture と結合検証テストを追加:

```rust
#[cfg(unix)]
fn appimage_fixture(dir: &Path, version: &str) -> (ApplyJob, HelperOptions) { /* Task 2 Step 3 の fixture */ }

#[cfg(unix)]
#[test]
fn verify_staged_accepts_a_matching_signed_manifest() { /* Ok(()) */ }

#[cfg(unix)]
#[test]
fn verify_staged_rejects_a_sha256_that_differs_from_the_manifest() { /* UpdateError::InvalidJob */ }

#[cfg(unix)]
#[test]
fn verify_staged_rejects_a_downgrade_manifest() { /* UpdateError::Downgrade */ }
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p vrc-photon-update verify_staged_ -- --nocapture`
Expected: `verify_staged_accepts_a_matching_signed_manifest` が manifest 未検証のため失敗。

- [ ] **Step 3: manifest 選択と結合検証を実装**

`manifest.rs`:

```rust
pub fn asset_for_apply(&self, kind: &ApplyKind) -> Option<&PlatformAsset> {
    let (platform, format) = match kind {
        ApplyKind::AppImage => (LINUX_PLATFORM, UpdateFormat::AppImage),
        ApplyKind::Windows => (WINDOWS_PLATFORM, UpdateFormat::Nsis),
        ApplyKind::SystemPackage { family } => return self.system_package_for(*family),
    };
    self.platforms.get(platform).filter(|asset| asset.format == format)
}
```

`verify_manifest` は `ApplyKind::SystemPackage` の `family` 分岐を削除し、`asset_for_apply` を使う。
`verify_staged` の先頭で `verify_manifest(job, options)?` を呼び、次を検査:

```rust
if !asset.sha256.eq_ignore_ascii_case(&job.sha256) {
    return Err(UpdateError::InvalidJob(
        "manifest asset SHA-256 does not match the job".to_string(),
    ));
}
```

その後に既存の staged sha256 と artifact 署名検証を続ける。

`install.rs` の `validate`: manifest ペアの presence/size 検査を
`ApplyKind::SystemPackage` 分岐の外へ移す。system package 固有の staging パス検査は残す。

`ui/update_view.rs`: `let (signed_manifest, manifest_signature) = match &kind { ... }` を削除し、
常に UTF-8 変換した manifest と署名を job に入れる。

- [ ] **Step 4: テストを実行**

Run: `cargo test -p vrc-photon-update && cargo test -p vrc-photon-ui --lib`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/update/src/manifest.rs crates/update/src/install.rs crates/update/src/helper.rs crates/ui/src/update_view.rs
git commit -m "fix: bind staged artifacts to the signed manifest on every apply path (#753)"
```

---

### Task 3: `.minisig` URL を Url::join で組み立てる (#769-2)

**Files:**
- Modify: `crates/update/src/check.rs:82`
- Test: `crates/update/src/check.rs` のテストモジュール

**Interfaces:**
- Produces: `fn signature_url(manifest_url: &str) -> Result<Url>`

- [ ] **Step 1: 失敗するテストを書く**

```rust
#[test]
fn signature_url_appends_the_suffix_and_drops_the_query() {
    let url = signature_url("https://example.com/feed/latest.json?token=1").unwrap();
    assert_eq!(url.as_str(), "https://example.com/feed/latest.json.minisig");
    assert!(signature_url("https://example.com/feed/").is_err());
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p vrc-photon-update signature_url -- --nocapture`
Expected: `signature_url` が未定義でコンパイル失敗。

- [ ] **Step 3: helper を実装して check() から使う**

```rust
fn signature_url(manifest_url: &str) -> Result<Url> {
    let url = Url::parse(manifest_url)
        .map_err(|error| UpdateError::InvalidManifest(format!("invalid feed URL: {error}")))?;
    let name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| UpdateError::InvalidManifest("feed URL has no file name".to_string()))?;
    url.join(&format!("{name}.minisig"))
        .map_err(|error| UpdateError::InvalidManifest(format!("invalid signature URL: {error}")))
}
```

`check()` の `format!("{}.minisig", feed.manifest_url)` を `signature_url(&feed.manifest_url)?` に置換。

- [ ] **Step 4: テストを実行**

Run: `cargo test -p vrc-photon-update signature_url`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/update/src/check.rs
git commit -m "fix: build the manifest signature URL with Url::join (#769)"
```

---

### Task 4: Windows の write_atomic を MoveFileExW にする (#769-1)

**Files:**
- Modify: `crates/update/Cargo.toml:43-44` (`Win32_Storage_FileSystem` feature)
- Modify: `crates/update/src/helper.rs` の `write_atomic` Windows 分岐
- Test: Windows CI のコンパイルで確認（Linux では cfg 外）

**Interfaces:**
- Consumes: `std::path::Path`
- Produces: `fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()>` (不変)

- [ ] **Step 1: Windows 用の feature を追加**

`crates/update/Cargo.toml`:

```toml
windows = { version = "0.62", features = ["Win32_Foundation", "Win32_Storage_FileSystem", "Win32_System_Threading"] }
```

- [ ] **Step 2: remove_file + rename を置換**

```rust
#[cfg(windows)]
{
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MoveFileExW, MOVE_FILE_FLAGS,
    };
    use windows::core::PCWSTR;

    let wide = |path: &Path| -> Vec<u16> {
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    };
    let from = wide(&temp);
    let to = wide(path);
    // SAFETY: 両方のバッファは NUL 終端の UTF-16 で、呼び出し中有効。
    let result = unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0),
        )
    };
    return result.map_err(|error| UpdateError::io(path, std::io::Error::from(error)));
}
#[cfg(not(windows))]
std::fs::rename(&temp, path).map_err(|error| UpdateError::io(path, error))
```

実際の windows 0.62 の戻り値型に合わせ、`result` が `BOOL` の場合は
`result.as_bool()` で判定する。Windows ジョブのコンパイルで確定する。

- [ ] **Step 3: Linux で回帰確認**

Run: `cargo test -p vrc-photon-update && cargo clippy -p vrc-photon-update --all-targets -- -D warnings`
Expected: PASS（Windows 分岐は cfg 外）

- [ ] **Step 4: コミット**

```bash
git add crates/update/Cargo.toml crates/update/src/helper.rs
git commit -m "fix: replace state files atomically on Windows (#769)"
```

---

### Task 5: release-tool の generate で署名を検証する (#769-3)

**Files:**
- Modify: `crates/release-tool/src/lib.rs:29-100`
- Modify: `crates/release-tool/src/main.rs:60-85`
- Modify: `.github/workflows/release.yml:377-383`
- Test: `crates/release-tool/src/lib.rs` のテスト

**Interfaces:**
- Produces: `pub fn build_manifest(dist: &Path, tag: &str, repo: &str, notes_url: &str, ai_plugin: Option<AiPluginRelease>, public_keys: &[String]) -> Result<Manifest>`

- [ ] **Step 1: 失敗するテストを書く**

`build_manifest` の既存テストを public key 付きに更新し、stale 署名のテストを追加:

```rust
#[test]
fn generate_rejects_an_artifact_changed_after_signing() {
    // 既存 fixture を署名後に artifact へ 1 バイト追記し、
    // build_manifest(..., &[public_key]) が Err になることを確認する。
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p vrc-photon-release-tool generate_rejects -- --nocapture`
Expected: 署名検証がないため Ok が返り失敗。

- [ ] **Step 3: 実装**

`build_manifest` の各 `asset(...)` 呼び出し前に `verify_file(path, public_keys)?` を実行する。
`public_keys` が空なら `bail!("at least one --public-key is required")`。

`main.rs` の `Generate` に `#[arg(long = "public-key")] public_keys: Vec<String>` を追加。
`release.yml` の generate 呼び出しへ `"${PUB_ARGS[@]}"` を渡す。

- [ ] **Step 4: テストを実行**

Run: `cargo test -p vrc-photon-release-tool`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/release-tool/src/lib.rs crates/release-tool/src/main.rs .github/workflows/release.yml
git commit -m "fix: verify artifact signatures when generating the update manifest (#769)"
```

---

### Task 6: 最終検証

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] 計画のチェックボックスを `- [x]` に更新し、PR を作成する
