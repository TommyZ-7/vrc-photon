//! First-run setup state and page navigation.
//!
//! The wizard only appears for a brand-new install. Page order, persistence,
//! and the page-leave commits live here; rendering lives in `ui::setup`.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use vrc_photon_core::discord::validate_webhook_url;
use vrc_photon_core::settings::save_discord_webhook;
use vrc_photon_i18n::{tr, tr_args};

use crate::app::App;
use crate::app::discord::webhook_error_key;
use crate::app::file_picker::{self, DialogOptions};
use crate::state::motion::ToggleMotion;

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
/// A missing settings row is the fresh-install signal; without a row, a
/// library with photos belongs to an older install. A present row with
/// `setup_completed = false` is an interrupted wizard and resumes regardless
/// of the current photo count.
pub(crate) fn should_show_setup(
    demo: bool,
    database_ok: bool,
    settings_load_failed: bool,
    settings_row_present: bool,
    setup_completed: bool,
    library_photo_count: i64,
) -> bool {
    if demo || !database_ok || settings_load_failed {
        return false;
    }
    if settings_row_present {
        // A row with false means the wizard was interrupted after it saved a
        // setting. Resume it even when the daemon has already indexed photos.
        !setup_completed
    } else {
        // No row is a fresh install. A library with photos but no settings
        // belongs to an older install and must not show the wizard.
        library_photo_count == 0
    }
}

/// Trims a webhook field and validates it. `Ok(None)` means "leave the stored
/// webhook unchanged"; `Err(key)` is the catalog key for the inline error.
pub(crate) fn setup_webhook_value(raw: &str) -> Result<Option<String>, &'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    validate_webhook_url(raw)
        .map(|parsed| Some(parsed.as_str().to_string()))
        .map_err(webhook_error_key)
}

/// Horizontal offset for a page that slides in from the side. `value` goes
/// from 1 (fully offset) to 0 (settled).
pub(crate) fn setup_page_offset(direction: f32, value: f32) -> f32 {
    direction * 28.0 * value.clamp(0.0, 1.0)
}

/// The preview card's text body exists only when at least one text option is
/// on; otherwise the card is just its image and must not keep empty padding.
pub(crate) fn display_preview_has_body(world: bool, date: bool, size: bool) -> bool {
    world || date || size
}

pub(crate) struct SetupState {
    pub(crate) open: bool,
    pub(crate) page: SetupPage,
    pub(crate) focus: FocusHandle,
    pub(crate) focus_pending: bool,
    pub(crate) error: Option<SharedString>,
    pub(crate) motion: ToggleMotion,
    pub(crate) direction: f32,
}

impl SetupState {
    pub(crate) fn new(open: bool, cx: &mut Context<App>) -> Self {
        Self {
            open,
            page: SetupPage::Welcome,
            focus: cx.focus_handle(),
            focus_pending: open,
            error: None,
            motion: ToggleMotion::settled(0.0),
            direction: 1.0,
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
                self.start_setup_page_motion(1.0);
                cx.notify();
            }
            None => self.finish_setup(cx),
        }
    }

    pub(crate) fn back_setup(&mut self, cx: &mut Context<Self>) {
        if matches!(self.setup.page, SetupPage::Folders | SetupPage::Discord)
            && !self.commit_setup_page(cx)
        {
            cx.notify();
            return;
        }
        if let Some(page) = self.setup.page.prev() {
            self.setup.page = page;
            self.setup.error = None;
            self.setup.focus_pending = true;
            self.start_setup_page_motion(-1.0);
            cx.notify();
        }
    }

    fn start_setup_page_motion(&mut self, direction: f32) {
        self.setup.direction = direction;
        self.setup.motion = ToggleMotion::settled(1.0);
        self.setup
            .motion
            .animate_to(0.0, Duration::from_millis(180));
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
        let raw = self.discord_url_input.read(cx).value().to_string();
        match setup_webhook_value(&raw) {
            Ok(None) => true,
            Ok(Some(url)) => match save_discord_webhook(&self.db, &url) {
                Ok(()) => {
                    self.discord_url = Some(url);
                    self.discord_url_editing = false;
                    true
                }
                Err(error) => {
                    self.setup.error = Some(tr_args!("settings.save_failed", error = error).into());
                    false
                }
            },
            Err(key) => {
                self.setup.error = Some(tr(key).into());
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
    use super::{
        SetupPage, display_preview_has_body, setup_page_offset, setup_webhook_value,
        should_show_setup,
    };

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
        assert!(should_show_setup(false, true, false, false, false, 0));
        assert!(!should_show_setup(true, true, false, false, false, 0));
        assert!(!should_show_setup(false, false, false, false, false, 0));
        assert!(!should_show_setup(false, true, true, false, false, 0));
        assert!(!should_show_setup(false, true, false, true, true, 0));
        assert!(!should_show_setup(false, true, false, false, false, 3));
    }

    #[test]
    fn an_interrupted_setup_resumes_even_after_photos_are_indexed() {
        assert!(should_show_setup(false, true, false, true, false, 0));
        assert!(should_show_setup(false, true, false, true, false, 3));
        assert!(!should_show_setup(false, true, false, true, true, 3));
    }

    #[test]
    fn setup_page_offset_slides_from_the_side_and_settles_at_zero() {
        assert_eq!(setup_page_offset(1.0, 1.0), 28.0);
        assert_eq!(setup_page_offset(-1.0, 1.0), -28.0);
        assert_eq!(setup_page_offset(1.0, 0.5), 14.0);
        assert_eq!(setup_page_offset(1.0, 0.0), 0.0);
    }

    #[test]
    fn display_preview_body_hides_when_all_text_options_are_off() {
        assert!(!display_preview_has_body(false, false, false));
        assert!(display_preview_has_body(true, false, false));
        assert!(display_preview_has_body(false, true, false));
        assert!(display_preview_has_body(false, false, true));
    }

    #[test]
    fn setup_webhook_values_are_validated() {
        assert_eq!(setup_webhook_value("   "), Ok(None));
        assert!(setup_webhook_value("http://discord.com/api/webhooks/1/t").is_err());
        let url = "https://discord.com/api/webhooks/123/token";
        assert_eq!(setup_webhook_value(url), Ok(Some(url.to_string())));
    }
}
