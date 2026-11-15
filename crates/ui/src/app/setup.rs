//! First-run setup state and page navigation.
//!
//! The wizard only appears for a brand-new install. Page order, persistence,
//! and the page-leave commits live here; rendering lives in `ui::setup`.

use std::path::PathBuf;

use gpui_kit::*;
use vrc_photon_core::discord::validate_webhook_url;
use vrc_photon_core::settings::save_discord_webhook;
use vrc_photon_i18n::{tr, tr_args};

use crate::app::App;
use crate::app::discord::webhook_error_key;
use crate::app::file_picker::{self, DialogOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetupPathField {
    PhotoRoot,
    LogDir,
    ImportDir,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetupPage {
    Welcome,
    Startup,
    Folders,
    Display,
    Discord,
    Compression,
    Theme,
    AiTagging,
}

impl SetupPage {
    pub(crate) fn next(self) -> Option<Self> {
        match self {
            Self::Welcome => Some(Self::Startup),
            Self::Startup => Some(Self::Folders),
            Self::Folders => Some(Self::Display),
            Self::Display => Some(Self::Discord),
            Self::Discord => Some(Self::Compression),
            Self::Compression => Some(Self::Theme),
            Self::Theme => Some(Self::AiTagging),
            Self::AiTagging => None,
        }
    }

    pub(crate) fn prev(self) -> Option<Self> {
        match self {
            Self::Welcome => None,
            Self::Startup => Some(Self::Welcome),
            Self::Folders => Some(Self::Startup),
            Self::Display => Some(Self::Folders),
            Self::Discord => Some(Self::Display),
            Self::Compression => Some(Self::Discord),
            Self::Theme => Some(Self::Compression),
            Self::AiTagging => Some(Self::Theme),
        }
    }

    pub(crate) fn is_last(self) -> bool {
        self.next().is_none()
    }
}

/// Whether the first-run wizard should open.
///
/// A missing settings row is the only fresh-install signal. Existing installs
/// carry `setup_completed = true` through the serde default, and a library
/// with photos also means the user already has a working install.
pub(crate) fn should_show_setup(
    demo: bool,
    database_ok: bool,
    settings_load_failed: bool,
    setup_completed: bool,
    library_photo_count: i64,
) -> bool {
    !demo && database_ok && !settings_load_failed && !setup_completed && library_photo_count == 0
}

pub(crate) struct SetupState {
    pub(crate) open: bool,
    pub(crate) page: SetupPage,
    pub(crate) focus: FocusHandle,
    pub(crate) focus_pending: bool,
    pub(crate) error: Option<SharedString>,
}

impl SetupState {
    pub(crate) fn new(open: bool, cx: &mut Context<App>) -> Self {
        Self {
            open,
            page: SetupPage::Welcome,
            focus: cx.focus_handle(),
            focus_pending: open,
            error: None,
        }
    }
}

impl App {
    pub(crate) fn advance_setup(&mut self, cx: &mut Context<Self>) {
        if !self.commit_setup_page(cx) {
            cx.notify();
            return;
        }
        match self.setup.page.next() {
            Some(page) => {
                self.setup.page = page;
                self.setup.error = None;
                self.setup.focus_pending = true;
                cx.notify();
            }
            None => self.finish_setup(cx),
        }
    }

    pub(crate) fn back_setup(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.setup.page.prev() {
            self.setup.page = page;
            self.setup.error = None;
            self.setup.focus_pending = true;
            cx.notify();
        }
    }

    pub(crate) fn dismiss_setup(&mut self, cx: &mut Context<Self>) {
        if self.commit_setup_page(cx) {
            self.finish_setup(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn pick_setup_folder(
        &mut self,
        field: SetupPathField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = match field {
            SetupPathField::PhotoRoot => self.photo_root_input.read(cx).value().to_string(),
            SetupPathField::LogDir => self.log_dir_input.read(cx).value().to_string(),
            SetupPathField::ImportDir => self.import_dir_input.read(cx).value().to_string(),
        };
        let directory = PathBuf::from(current);
        let options = DialogOptions {
            directory: directory.is_dir().then_some(directory),
            ..DialogOptions::default()
        };
        let dialog = options.dialog(Some(&*window));
        cx.spawn_in(window, async move |this, cx| {
            let Some(dir) = file_picker::pick_folder(dialog).await else {
                return;
            };
            let path = dir.to_string_lossy().to_string();
            cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    let input = match field {
                        SetupPathField::PhotoRoot => &this.photo_root_input,
                        SetupPathField::LogDir => &this.log_dir_input,
                        SetupPathField::ImportDir => &this.import_dir_input,
                    };
                    input.update(cx, |state, cx| state.set_value(path, window, cx));
                    cx.notify();
                })
                .ok();
            })
            .ok();
        })
        .detach();
    }

    fn save_setup_webhook(&mut self, cx: &mut Context<Self>) -> bool {
        let raw = self.discord_url_input.read(cx).value().trim().to_string();
        if raw.is_empty() {
            return true;
        }
        match validate_webhook_url(&raw) {
            Ok(parsed) => match save_discord_webhook(&self.db, parsed.as_str()) {
                Ok(()) => {
                    self.discord_url = Some(parsed.as_str().to_string());
                    self.discord_url_editing = false;
                    true
                }
                Err(error) => {
                    self.setup.error = Some(tr_args!("settings.save_failed", error = error).into());
                    false
                }
            },
            Err(error) => {
                self.setup.error = Some(tr(webhook_error_key(error)).into());
                false
            }
        }
    }

    pub(crate) fn set_setup_immediate(&mut self, immediate: bool, cx: &mut Context<Self>) {
        if self.settings.auto_compress_immediate == immediate {
            return;
        }
        self.settings.auto_compress_immediate = immediate;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save immediate compression setting");
        }
        cx.notify();
    }

    pub(crate) fn set_setup_keep_original(&mut self, keep: bool, cx: &mut Context<Self>) {
        if self.settings.auto_compress_keep_original == keep {
            return;
        }
        self.settings.auto_compress_keep_original = keep;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save keep-original setting");
        }
        cx.notify();
    }

    fn commit_setup_page(&mut self, cx: &mut Context<Self>) -> bool {
        match self.setup.page {
            SetupPage::Folders => self.save_library_paths(cx),
            SetupPage::Discord => self.save_setup_webhook(cx),
            _ => true,
        }
    }

    fn finish_setup(&mut self, cx: &mut Context<Self>) {
        self.settings.setup_completed = true;
        if let Err(error) = self.settings.save(&self.db) {
            tracing::warn!(%error, "cannot save the setup completion flag");
            self.toast_error(tr_args!("settings.save_failed", error = error), cx);
        }
        self.setup.open = false;
        self.initial_focus_pending = true;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{SetupPage, should_show_setup};

    #[test]
    fn setup_pages_advance_and_return_in_order() {
        use SetupPage::*;
        let pages = [
            Welcome,
            Startup,
            Folders,
            Display,
            Discord,
            Compression,
            Theme,
            AiTagging,
        ];
        for pair in pages.windows(2) {
            assert_eq!(pair[0].next(), Some(pair[1]));
            assert_eq!(pair[1].prev(), Some(pair[0]));
        }
        assert_eq!(AiTagging.next(), None);
        assert!(AiTagging.is_last());
        assert_eq!(Welcome.prev(), None);
    }

    #[test]
    fn setup_only_shows_for_a_fresh_install() {
        assert!(should_show_setup(false, true, false, false, 0));
        assert!(!should_show_setup(true, true, false, false, 0));
        assert!(!should_show_setup(false, false, false, false, 0));
        assert!(!should_show_setup(false, true, true, false, 0));
        assert!(!should_show_setup(false, true, false, true, 0));
        assert!(!should_show_setup(false, true, false, false, 3));
    }
}
