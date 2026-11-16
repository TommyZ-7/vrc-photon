use anyhow::{Context, Result};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use vrc_photon_i18n::{tr, tr_args};

use crate::DaemonStatus;

pub enum TrayAction {
    Noop,
    TogglePause(bool),
    Quit,
    OpenUi,
}

pub struct Tray {
    icon: TrayIcon,
    open: MenuItem,
    pause: CheckMenuItem,
    status: MenuItem,
    quit: MenuItem,
    last_text: String,
}

impl Tray {
    pub fn new() -> Result<Self> {
        let open = MenuItem::new(tr!("tray.open"), true, None);
        let pause = CheckMenuItem::new(tr!("tray.pause"), true, false, None);
        let status = MenuItem::new(tr!("tray.status.starting"), false, None);
        let quit = MenuItem::new(tr!("tray.quit"), true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &open,
            &pause,
            &status,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .context("cannot build tray menu")?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("VRC PhotoN")
            .with_icon(build_icon()?)
            .build()
            .context("cannot create tray icon")?;

        Ok(Self {
            icon,
            open,
            pause,
            status,
            quit,
            last_text: String::new(),
        })
    }

    /// Refresh the menu labels after the UI changed the language.
    pub fn set_language(&mut self) {
        self.open.set_text(tr!("tray.open"));
        self.pause.set_text(tr!("tray.pause"));
        self.quit.set_text(tr!("tray.quit"));
        self.last_text.clear();
    }

    pub fn poll(&mut self, status: &DaemonStatus) -> TrayAction {
        #[cfg(target_os = "windows")]
        pump_windows_messages();

        let mut action = TrayAction::Noop;
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id() == self.open.id() {
                action = TrayAction::OpenUi;
            } else if event.id() == self.pause.id() {
                action = TrayAction::TogglePause(!status.paused);
            } else if event.id() == self.quit.id() {
                action = TrayAction::Quit;
            }
        }

        let text = if status.paused {
            tr!("tray.status.paused")
        } else if let Some(world) = &status.world_name {
            tr_args!(
                "tray.status.watching",
                world = world,
                players = status.players
            )
        } else {
            tr!("tray.status.idle")
        };
        if text != self.last_text {
            self.status
                .set_text(tr_args!("tray.status.prefix", status = text.clone()));
            let _ = self.icon.set_tooltip(Some(format!("VRC PhotoN — {text}")));
            self.last_text = text;
        }
        self.pause.set_checked(status.paused);
        action
    }
}

pub fn open_ui() {
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let name = if cfg!(target_os = "windows") {
        "vrc-photon.exe"
    } else {
        "vrc-photon"
    };
    let candidate = current
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(name);
    if candidate.exists() {
        // Keep test-build forks out of another test's executable write
        // (#647). Production builds do not take this lock.
        #[cfg(test)]
        let _process_guard = crate::test_support::process_lock();
        if let Err(error) = std::process::Command::new(&candidate).spawn() {
            tracing::warn!(path = %candidate.display(), %error, "cannot open UI");
        }
    } else {
        tracing::info!(path = %candidate.display(), "UI binary not found; build crates/ui");
    }
}

/// Loads the icon group embedded in this executable.
///
/// The group contains every size generated from `packaging/icons/icon.png`,
/// so the notification area can pick a crisp variant for the current DPI.
#[cfg(target_os = "windows")]
fn build_icon() -> Result<Icon> {
    Icon::from_resource(1, None).context("cannot load the embedded tray icon")
}

/// Renders the bundled application icon at tray size.
///
/// Linux panels take a single RGBA image rather than a multi-size icon
/// resource, so downscale the same artwork the Windows resource is built from.
#[cfg(not(target_os = "windows"))]
fn build_icon() -> Result<Icon> {
    let rgba = bundled_icon_rgba()?;
    Icon::from_rgba(rgba.into_raw(), TRAY_ICON_SIZE, TRAY_ICON_SIZE).context("invalid tray icon")
}

#[cfg(not(target_os = "windows"))]
const TRAY_ICON_SIZE: u32 = 32;

#[cfg(not(target_os = "windows"))]
fn bundled_icon_rgba() -> Result<image::RgbaImage> {
    const ICON_PNG: &[u8] = include_bytes!("../../../packaging/icons/icon.png");
    let source = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .context("cannot decode the bundled tray icon")?;
    Ok(image::imageops::resize(
        &source.to_rgba8(),
        TRAY_ICON_SIZE,
        TRAY_ICON_SIZE,
        image::imageops::FilterType::Lanczos3,
    ))
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::*;

    #[test]
    fn bundled_icon_is_resized_with_transparent_corners() {
        let icon = bundled_icon_rgba().expect("bundled tray icon decodes");
        assert_eq!(icon.dimensions(), (TRAY_ICON_SIZE, TRAY_ICON_SIZE));
        assert!(
            icon.get_pixel(0, 0)[3] < 16,
            "the rounded corner must stay transparent"
        );
        assert!(
            icon.get_pixel(TRAY_ICON_SIZE / 2, TRAY_ICON_SIZE / 2)[3] > 200,
            "the center of the asterisk must stay opaque"
        );
    }
}

#[cfg(target_os = "windows")]
fn pump_windows_messages() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };
    // SAFETY: this runs on the thread that owns the tray window and its message
    // queue. `message` is a stack buffer that PeekMessageW initializes before
    // TranslateMessage/DispatchMessageW read it, and the loop handles only this
    // thread's messages.
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
