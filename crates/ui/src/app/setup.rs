//! First-run setup state and page navigation.
//!
//! The wizard only appears for a brand-new install. Page order, persistence,
//! and the page-leave commits live here; rendering lives in `ui::setup`.

use gpui_kit::*;

use crate::app::App;

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
    !demo
        && database_ok
        && !settings_load_failed
        && !setup_completed
        && library_photo_count == 0
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
