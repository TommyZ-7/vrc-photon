//! Shared build-script helper that embeds the application icon into a
//! Windows executable.
//!
//! Every crate that ships a `.exe` includes this file from its `build.rs`, so
//! `packaging/icons/icon.ico` stays the single source of truth for the
//! Explorer, taskbar, and window icon. GPUI loads the icon group with resource
//! ID 1, so the ID must not change.

use std::path::PathBuf;

pub fn embed() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"),
    );
    let icon = manifest_dir.join("../../packaging/icons/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=../../build-support/windows_icon.rs");

    // Keep the resource script in OUT_DIR so it can reference the icon by an
    // absolute path outside the crate. RC.EXE treats backslashes in the quoted
    // path as escape sequences (`D:\a\...` loses the `\a`), so hand it forward
    // slashes; Windows resolves them like any other path separator.
    let rc = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"))
        .join("app-icon.rc");
    let icon_path = icon.display().to_string().replace('\\', "/");
    std::fs::write(&rc, format!("1 ICON \"{icon_path}\"\n"))
        .expect("cannot write the Windows resource script");

    // gpui-pre links the RT_MANIFEST resource, so this script only contributes
    // the icon; an embedded manifest would duplicate it. The icon is required
    // for release builds, so a missing resource compiler must fail the build.
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_required()
        .expect("cannot embed the application icon");
}
