//! Demo database selection for app startup.
//!
//! Demo mode writes a temporary photo library and thumbnails so the UI can run
//! without a real VRChat library. The fixture builder lives in
//! [`super::demo_build`] and is behind the `demo` feature; release builds
//! refuse both `--demo` and the database-failure fallback, keeping only the
//! path helpers in this module.

use std::path::PathBuf;

use vrc_photon_core::db::{Db, DbProfile};
use vrc_photon_i18n::tr_args;

use crate::app::cli::LaunchOptions;

pub(crate) struct DemoAssets {
    #[allow(dead_code)] // Fixture root, kept for inspection.
    pub(super) photo_dir: PathBuf,
    pub(super) thumb_dir: PathBuf,
}

#[cfg(feature = "demo")]
fn prepare_demo(reset: bool) -> Result<(Db, DemoAssets), String> {
    super::demo_build::prepare_demo_at(&demo_root(), reset, chrono::Local::now())
}

#[cfg(not(feature = "demo"))]
fn prepare_demo(_reset: bool) -> Result<(Db, DemoAssets), String> {
    Err(vrc_photon_i18n::tr!("demo.unavailable"))
}

/// Root of the throwaway demo library. The UID keeps two users on a shared
/// machine from fighting over one temporary directory.
fn demo_root() -> PathBuf {
    #[cfg(unix)]
    // SAFETY: `getuid` has no preconditions and does not touch memory.
    let name = format!("vrc-photon-demo-{}", unsafe { libc::getuid() });
    #[cfg(not(unix))]
    let name = "vrc-photon-demo".to_string();
    std::env::temp_dir().join(name)
}

fn demo_db_path() -> PathBuf {
    demo_root().join("library.db")
}

pub(crate) fn demo_thumb_path(assets: &DemoAssets, photo_id: i64) -> PathBuf {
    assets.thumb_dir.join(format!("{photo_id}-s384x216.bc7"))
}

/// Demo thumbnails are generated at this fixed canvas size.
pub(crate) const DEMO_THUMBNAIL_WIDTH: u32 = 384;
pub(crate) const DEMO_THUMBNAIL_HEIGHT: u32 = 216;

/// Resolves the database the UI will open, before any window exists, so the
/// single-instance lock can be scoped to the same library. Demo mode uses the
/// generated fixture database; otherwise `VRC_PHOTON_DB`, the per-user default,
/// and finally a relative path in the working directory are tried in order.
pub(crate) fn resolve_db_path(options: &LaunchOptions) -> PathBuf {
    resolve_db_path_from(
        options.demo,
        std::env::var("VRC_PHOTON_DB").ok().map(PathBuf::from),
        vrc_photon_core::default_db_path(),
    )
}

fn resolve_db_path_from(
    demo: bool,
    env_db: Option<PathBuf>,
    default_db: Option<PathBuf>,
) -> PathBuf {
    if demo {
        return demo_db_path();
    }
    env_db
        .or(default_db)
        .unwrap_or_else(|| PathBuf::from("vrc-photon.db"))
}

pub(crate) struct OpenedDatabase {
    pub(crate) db: Db,
    pub(crate) demo: bool,
    pub(crate) database_ok: bool,
    pub(crate) message: Option<String>,
    pub(crate) demo_assets: Option<DemoAssets>,
}

pub(crate) fn open_database(options: &LaunchOptions) -> Result<OpenedDatabase, String> {
    if options.demo {
        let (db, assets) = prepare_demo(options.demo_reset)?;
        return Ok(OpenedDatabase {
            db,
            demo: true,
            database_ok: false,
            message: None,
            demo_assets: Some(assets),
        });
    }
    let path = resolve_db_path(options);
    match Db::open_with_profile(&path, DbProfile::Ui) {
        Ok(db) => Ok(OpenedDatabase {
            db,
            demo: false,
            database_ok: true,
            message: None,
            demo_assets: None,
        }),
        Err(error) => {
            let message = tr_args!("demo.db_error", error = error);
            #[cfg(feature = "demo")]
            {
                match prepare_demo(true) {
                    Ok((db, assets)) => Ok(OpenedDatabase {
                        db,
                        demo: true,
                        database_ok: false,
                        message: Some(message),
                        demo_assets: Some(assets),
                    }),
                    Err(demo_error) => Err(tr_args!(
                        "demo.db_and_prepare_error",
                        error = error,
                        demo_error = demo_error
                    )),
                }
            }
            #[cfg(not(feature = "demo"))]
            {
                Err(message)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_db_path_uses_demo_database_in_demo_mode() {
        let options = LaunchOptions {
            demo: true,
            ..LaunchOptions::default()
        };
        assert_eq!(resolve_db_path(&options), demo_db_path());
    }

    #[test]
    fn resolve_db_path_prefers_demo_then_env_then_default() {
        let env_path = PathBuf::from("/tmp/env/library.db");
        let default_path = PathBuf::from("/tmp/default/library.db");

        assert_eq!(
            resolve_db_path_from(true, Some(env_path.clone()), Some(default_path.clone())),
            demo_db_path()
        );
        assert_eq!(
            resolve_db_path_from(false, Some(env_path.clone()), Some(default_path.clone())),
            env_path
        );
        assert_eq!(
            resolve_db_path_from(false, None, Some(default_path.clone())),
            default_path
        );
        assert_eq!(
            resolve_db_path_from(false, None, None),
            PathBuf::from("vrc-photon.db")
        );
    }

    #[cfg(unix)]
    #[test]
    fn demo_root_is_scoped_to_the_user() {
        let name = demo_root()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.ends_with(&format!("-{}", {
            // SAFETY: `getuid` has no preconditions and does not touch memory.
            unsafe { libc::getuid() }
        })));
    }

    #[test]
    fn demo_thumb_path_uses_png_in_demo_dir() {
        let assets = DemoAssets {
            photo_dir: std::path::PathBuf::from("/tmp/vrc-demo/photos"),
            thumb_dir: std::path::PathBuf::from("/tmp/vrc-demo/thumbnails"),
        };
        let path = demo_thumb_path(&assets, 42);
        assert!(path.ends_with("42-s384x216.bc7"));
        assert_eq!(path.parent(), Some(assets.thumb_dir.as_path()));
    }
}
