//! Settings screen: library paths, display options, startup, AI plugin
//! management, and compress policies.
//!
//! The switch, card-size, and field widgets come from `ui::components`; screen
//! dispatch lives in `ui::shell`.

use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::StatefulInteractiveElement as _;
use gpui_kit::*;
use std::path::Path;
use vrc_photon_ai_protocol::ModelState;
use vrc_photon_core::ai::PluginInstallation;
use vrc_photon_core::model::DaemonCommand;
use vrc_photon_core::settings::{AppSettings, CardSize};
use vrc_photon_core::thumbnail;
use vrc_photon_i18n::{tr, tr_args};
use vrc_photon_ipc::Response;

use crate::app::App;
use crate::app::ai::{
    AI_PLUGIN_ID, AiErrorInfo, AiErrorKind, AiManifestView, AiPluginAction, AiPluginCardState,
    AiPluginCardView, ModelDownloadView, UninstallTarget, ai_error_code_is_stable,
    ai_error_detail_in, ai_error_summary_in, ai_manifest_view, ai_plugin_update_notice,
    installation_row_id, is_active_installation, model_progress_ratio, variant_backend_label,
};
use crate::app::daemon::open_path;
use crate::ipc_bridge;
use crate::ui::components::{card_divider, dialog_panel, glass_panel, status_progress_bar};
use crate::ui::palette::{
    accent_colors, find_accent, find_main, main_colors, preview_accent, preview_main, readable_on,
    resolve,
};
use crate::ui::theme::*;

mod ai;
mod ai_overlay;
mod developer;
mod display;
mod language;
mod library;
mod theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemeAxis {
    Main,
    Accent,
}

impl ThemeAxis {
    fn key(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Accent => "accent",
        }
    }

    fn title(self) -> String {
        match self {
            Self::Main => tr!("theme.color.main.title"),
            Self::Accent => tr!("theme.color.accent.title"),
        }
    }
}

/// Whether a library path edit changed the photo root, and whether anything
/// changed at all. The photo root needs a library snapshot refresh; any path
/// change needs a daemon rescan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LibraryPathChanges {
    pub(crate) root: bool,
    pub(crate) any: bool,
}

pub(crate) const DEVELOPER_REPOSITORY_LABEL: &str = "github.com/TommyZ-7/vrc-photon";
pub(crate) const DEVELOPER_REPOSITORY_URL: &str = "https://github.com/TommyZ-7/vrc-photon";
pub(crate) const DEVELOPER_X_LABEL: &str = "x.com/TommyZ794685";
pub(crate) const DEVELOPER_X_URL: &str = "https://x.com/TommyZ794685";

/// The four developer-card rows, in display order: label then value.
pub(crate) fn developer_info_rows() -> Vec<(String, String)> {
    vec![
        (
            tr!("settings.developer.version"),
            vrc_photon_update::CURRENT_VERSION.to_string(),
        ),
        (
            tr!("settings.developer.license"),
            "GPL-3.0-or-later".to_string(),
        ),
        (
            tr!("settings.developer.repository"),
            DEVELOPER_REPOSITORY_LABEL.to_string(),
        ),
        (tr!("settings.developer.x"), DEVELOPER_X_LABEL.to_string()),
    ]
}

/// Applies the three path fields to the settings with the same trimming and
/// blank-clearing rules as the settings screen.
pub(crate) fn apply_library_path_inputs(
    settings: &mut AppSettings,
    photo_root: &str,
    log_dir: &str,
    import_dir: &str,
) -> LibraryPathChanges {
    let previous_root = std::mem::take(&mut settings.photo_roots);
    let previous_log = settings.log_dir.take();
    let previous_import = settings.managed_dir.take();

    let root = photo_root.trim();
    settings.photo_roots = if root.is_empty() {
        Vec::new()
    } else {
        vec![root.to_string()]
    };
    let log = log_dir.trim();
    settings.log_dir = (!log.is_empty()).then(|| log.to_string());
    let imported = import_dir.trim();
    settings.managed_dir = (!imported.is_empty()).then(|| imported.to_string());

    let root_changed = settings.photo_roots != previous_root;
    let any =
        root_changed || settings.log_dir != previous_log || settings.managed_dir != previous_import;
    LibraryPathChanges {
        root: root_changed,
        any,
    }
}

impl App {
    pub(crate) fn settings_screen(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .relative()
            .flex_1()
            .min_h_0()
            .items_stretch()
            .child(
                v_flex()
                    .id("settings-sections")
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.settings.scroll)
                    .gap(px(CARD_GAP))
                    .child(self.language_card(cx))
                    .child(self.theme_card(cx))
                    .child(self.library_card(cx))
                    .child(self.ai_plugin_card(cx))
                    .child(self.world_name_card(cx))
                    .child(self.display_card(cx))
                    .child(self.performance_card(cx))
                    .child(self.thumbnail_card(cx))
                    .child(self.startup_card(cx))
                    .child(self.update_card(cx))
                    .child(self.log_card(cx))
                    .child(
                        h_flex().gap(px(8.)).child(
                            Button::new("settings-reload")
                                .label(tr!("settings.reload"))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reload_aux(cx);
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(self.developer_card(cx)),
            )
            .vertical_scrollbar(&self.settings.scroll)
    }
}

#[cfg(test)]
mod path_input_tests {
    use super::apply_library_path_inputs;
    use vrc_photon_core::settings::AppSettings;

    #[test]
    fn library_path_inputs_trim_and_report_changes() {
        let mut settings = AppSettings::default();
        let changes = apply_library_path_inputs(
            &mut settings,
            "  /tmp/photos  ",
            " /tmp/logs ",
            " /tmp/imported ",
        );
        assert!(changes.root);
        assert!(changes.any);
        assert_eq!(settings.photo_roots, vec!["/tmp/photos".to_string()]);
        assert_eq!(settings.log_dir.as_deref(), Some("/tmp/logs"));
        assert_eq!(settings.managed_dir.as_deref(), Some("/tmp/imported"));
    }

    #[test]
    fn blank_library_paths_clear_the_fields() {
        let mut settings = AppSettings {
            photo_roots: vec!["/tmp/photos".to_string()],
            log_dir: Some("/tmp/logs".to_string()),
            managed_dir: Some("/tmp/imported".to_string()),
            ..Default::default()
        };
        let changes = apply_library_path_inputs(&mut settings, " ", "", "\t");
        assert!(changes.root);
        assert!(changes.any);
        assert!(settings.photo_roots.is_empty());
        assert_eq!(settings.log_dir, None);
        assert_eq!(settings.managed_dir, None);
    }

    #[test]
    fn unchanged_library_paths_report_no_change() {
        let mut settings = AppSettings {
            photo_roots: vec!["/tmp/photos".to_string()],
            log_dir: Some("/tmp/logs".to_string()),
            managed_dir: None,
            ..Default::default()
        };
        let changes = apply_library_path_inputs(&mut settings, "/tmp/photos", "/tmp/logs", "");
        assert!(!changes.root);
        assert!(!changes.any);
    }

    #[test]
    fn developer_info_rows_use_the_unprefixed_version_and_public_links() {
        let rows = super::developer_info_rows();
        assert_eq!(rows[0].1, vrc_photon_update::CURRENT_VERSION);
        assert!(!rows[0].1.starts_with('v'));
        assert_eq!(rows[1].1, "GPL-3.0-or-later");
        assert_eq!(rows[2].1, super::DEVELOPER_REPOSITORY_LABEL);
        assert_eq!(rows[3].1, super::DEVELOPER_X_LABEL);
        assert_eq!(
            super::DEVELOPER_REPOSITORY_URL,
            "https://github.com/TommyZ-7/vrc-photon"
        );
        assert_eq!(super::DEVELOPER_X_URL, "https://x.com/TommyZ794685");
    }

    #[test]
    fn developer_avatar_asset_is_a_jpeg() {
        let bytes = include_bytes!("../../../assets/devinfo-avatar.jpg");
        assert!(bytes.len() > 2);
        assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    }
}
