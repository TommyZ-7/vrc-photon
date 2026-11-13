**English** | [日本語](README.ja.md)

<p align="center">
  <img src="docs/images/vrc-photon-social.png" alt="VRC PhotoN">
</p>

<p align="center">
  <a href="https://github.com/TommyZ-7/vrc-photon/releases"><img src="https://img.shields.io/github/v/release/TommyZ-7/vrc-photon" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/TommyZ-7/vrc-photon" alt="License"></a>
  <img src="https://img.shields.io/github/languages/top/TommyZ-7/vrc-photon" alt="Top language">
  <img src="https://img.shields.io/github/languages/count/TommyZ-7/vrc-photon" alt="Languages">
</p>

A desktop app for organizing your VRChat photos together with the world they were taken in, the players you were with, and when they were taken.

VRChat screenshots tell you nothing from their file names alone. VRC PhotoN watches your photo folder and reads "when, where, and who with" from VRChat's output logs and the PNG XMP metadata, then links that context to each photo. The library, thumbnails, and settings stay on your PC. Network access is limited to update checks, optional Discord posting, optional VRChat world-name lookup, and AI model downloads that you start yourself.


<!-- TODO: Place a screenshot of the actual app at docs/images/main.png and enable the line below
![Main screen](docs/images/main.png)
-->

## Features

**Browse**

- Timeline and session views. Sessions group photos by the world visit recorded in the logs.
- Virtual scrolling, a year rail, and paged loading keep large libraries responsive.
- Full-screen viewer with zoom and pan. Duplicate photos are marked using their BLAKE3 hash.
- Search by world, player, manual tags, applied AI tags, AI people count, date, and favorites, with suggestions while you type.
- Photos are linked to their world and the players present from the logs. Photos without logs fall back to XMP metadata. Unresolved world names can be looked up through the VRChat API when enabled.

**Organize**

- Tags, notes, favorites, multi-select, and a right-click menu for compression, Discord, tag editing, path copy, folder opening, and trash.
- Copy or move import from files and folders, with an option to preserve the source folder hierarchy.
- Deletion goes to the OS trash. The deletion queue waits for running compression, uploads, and AI tagging.

**Compress**

- Convert to JPEG / WebP / AVIF / JPEG XL, with resizing and lossless output where the format supports it.
- Profiles are for manual compression. Automatic compression, the clipboard, and Discord uploads each have their own compression settings.
- Automatic compression writes to `compressed/` next to the original. The clipboard copies a PNG with resize settings only, and Discord compresses only for the upload.
- Manual compression always keeps the original and registers the compressed file as a derivative.

**Connect**

- Optional Discord webhook with manual and automatic posting, send history, and retries.
- A bundled read-only MCP server lets MCP clients search the library, fetch photo details and images, and read world and library statistics.
- AI tagging (paid plugin, separately distributed) infers character presence, people count, indoor/outdoor, and atmosphere/scene tags locally. See the [AI tagging documentation](docs/ai-tagging/README.md) (in Japanese).
- Japanese / English UI, theme colors, system tray residency, autostart, and signed automatic updates.

## Installation

Download the file for your platform from [Releases](https://github.com/TommyZ-7/vrc-photon/releases).

| Platform | File |
|---|---|
| Windows | `vrc-photon_<version>_x64-setup.exe` |
| Debian / Ubuntu | `vrc-photon_<version>_amd64.deb` |
| Arch-based | `vrc-photon-<version>-1-x86_64.pkg.tar.zst` |
| Other Linux | `vrc-photon_<version>_x86_64.AppImage`, or `.tar.gz` |

The photo folder and the log folder are detected automatically on first launch. On systems where detection fails, specify them when starting the daemon with `--photo-dir` and `--log-dir`.

## How it works

The app runs a UI and a background daemon. The daemon watches folders, imports logs, links photos, compresses, and generates thumbnails; the UI handles display and editing. Both share the same SQLite database (`library.db`) and use local IPC for immediate operations. The MCP server is a separate read-only process that talks to the daemon over the same IPC.

```text
crates/
  core/       Database, photo indexing, log/XMP parsing, linking, compression, thumbnails
  daemon/     Watchers, log import, workers, tray, autostart, IPC server
  ui/         GPUI-based UI, IPC client
  mcp/        Read-only stdio MCP server
  ipc/        Local IPC protocol and transport
  ai-protocol/  AI plugin protocol, catalog types, JSONL framing
  ai-host/      Plugin verification, model handling, sidecar client
  update/       Signed update checks, downloads, apply jobs, helper
  signing/      Shared minisign verification
  i18n/         Japanese / English catalogs
  release-tool/ Release manifest signing and verification
```

See [docs/architecture.md](docs/architecture.md) for the full picture.

## Building from source

You need Rust stable plus a few platform-specific packages (on Linux, `libxkbcommon-dev`, `libwayland-dev`, `libvulkan-dev`, and so on; building JPEG XL requires CMake / Ninja / a C++ compiler). See [docs/development.md](docs/development.md) for details.

```bash
cargo build --workspace
```

To try just the UI with sample data, use `cargo run -p vrc-photon-ui -- --demo`.

## Documentation

Documentation is currently written in Japanese.

- [Documentation index](docs/README.md)
- [Features](docs/features.md)
- [Architecture](docs/architecture.md)
- [Data model](docs/data-model.md)
- [UI specification](docs/ui.md)
- [Development setup](docs/development.md)
- [Testing](docs/testing.md)
- [Release and packaging](docs/release.md)
- [AI tagging plugin](docs/ai-tagging/README.md)
- [MCP server](docs/mcp.md)
- [Contributing](CONTRIBUTING.md)

## License

The app itself is GPL-3.0-or-later. The AI tagging plugin is provided under a separate license.
