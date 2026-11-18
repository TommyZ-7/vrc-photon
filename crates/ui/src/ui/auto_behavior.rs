//! Shared automatic-mode and bulk-action controls used by the three
//! integration cards. These are deliberately plain sections, not nested
//! cards, so the card keeps one visual hierarchy.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::*;
use gpui_kit::*;
use vrc_photon_core::settings::{AutoBehavior, AutoMode, AutoScope};
use vrc_photon_i18n::tr;

use crate::app::App;
use crate::ui::palette::Palette;

pub(crate) fn auto_mode_segmented(
    palette: &Palette,
    id_prefix: &'static str,
    mode: AutoMode,
    cx: &mut Context<App>,
    on_mode: fn(&mut App, AutoMode, &mut Window, &mut Context<App>),
) -> AnyElement {
    let p = palette;
    v_flex()
        .gap(px(7.))
        .child(
            div()
                .text_size(px(12.))
                .font_semibold()
                .text_color(rgb(p.text))
                .child(tr!("auto.behavior.mode")),
        )
        .child(
            h_flex()
                .gap(px(2.))
                .p(px(3.))
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.panel_inner))
                .child(mode_segment(
                    p,
                    format!("{id_prefix}-mode-off"),
                    tr!("auto.behavior.mode.off"),
                    mode == AutoMode::Off,
                    cx,
                    AutoMode::Off,
                    on_mode,
                ))
                .child(mode_segment(
                    p,
                    format!("{id_prefix}-mode-immediate"),
                    tr!("auto.behavior.mode.immediate"),
                    mode == AutoMode::Immediate,
                    cx,
                    AutoMode::Immediate,
                    on_mode,
                ))
                .child(mode_segment(
                    p,
                    format!("{id_prefix}-mode-waiting"),
                    tr!("auto.behavior.mode.waiting"),
                    mode == AutoMode::Waiting,
                    cx,
                    AutoMode::Waiting,
                    on_mode,
                )),
        )
        .into_any_element()
}

fn mode_segment(
    palette: &Palette,
    id: String,
    label: String,
    active: bool,
    cx: &mut Context<App>,
    mode: AutoMode,
    on_mode: fn(&mut App, AutoMode, &mut Window, &mut Context<App>),
) -> Stateful<Div> {
    segment(palette, id, label, active)
        .on_click(cx.listener(move |this, _, window, cx| on_mode(this, mode, window, cx)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn bulk_action_block(
    palette: &Palette,
    id_prefix: &'static str,
    behavior: AutoBehavior,
    enabled: bool,
    button_label: String,
    cx: &mut Context<App>,
    on_scope: fn(&mut App, AutoScope, &mut Window, &mut Context<App>),
    on_bulk: fn(&mut App, &mut Window, &mut Context<App>),
) -> AnyElement {
    let p = palette;
    v_flex()
        .gap(px(7.))
        .child(
            div()
                .text_size(px(12.))
                .font_semibold()
                .text_color(rgb(p.text))
                .child(tr!("auto.bulk.title")),
        )
        .child(
            h_flex()
                .gap(px(2.))
                .p(px(3.))
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.panel_inner))
                .child(scope_segment(
                    p,
                    format!("{id_prefix}-scope-new"),
                    tr!("auto.bulk.scope.new_only"),
                    behavior.scope == AutoScope::NewOnly,
                    cx,
                    AutoScope::NewOnly,
                    on_scope,
                ))
                .child(scope_segment(
                    p,
                    format!("{id_prefix}-scope-backlog"),
                    tr!("auto.bulk.scope.include_backlog"),
                    behavior.scope == AutoScope::IncludeBacklog,
                    cx,
                    AutoScope::IncludeBacklog,
                    on_scope,
                )),
        )
        .child(
            Button::new(SharedString::from(format!("{id_prefix}-bulk")))
                .label(button_label)
                .small()
                .primary()
                .w_full()
                .disabled(!enabled)
                .on_click(cx.listener(move |this, _, window, cx| on_bulk(this, window, cx))),
        )
        .into_any_element()
}

fn scope_segment(
    palette: &Palette,
    id: String,
    label: String,
    active: bool,
    cx: &mut Context<App>,
    scope: AutoScope,
    on_scope: fn(&mut App, AutoScope, &mut Window, &mut Context<App>),
) -> Stateful<Div> {
    segment(palette, id, label, active)
        .on_click(cx.listener(move |this, _, window, cx| on_scope(this, scope, window, cx)))
}

pub(crate) fn segment(palette: &Palette, id: String, label: String, active: bool) -> Stateful<Div> {
    let p = palette;
    div()
        .id(SharedString::from(id))
        .flex_1()
        .h(px(27.))
        .flex()
        .items_center()
        .justify_center()
        .px(px(8.))
        .rounded(px(7.))
        .text_size(px(10.5))
        .font_medium()
        .text_center()
        .cursor_pointer()
        .bg(rgb(if active { p.accent_soft } else { p.panel_2 }))
        .text_color(rgb(if active { p.accent } else { p.muted }))
        .child(label)
}

pub(crate) fn bulk_action_available(master_enabled: bool, feature_ready: bool) -> bool {
    master_enabled && feature_ready
}

#[cfg(test)]
mod tests {
    use super::bulk_action_available;
    use vrc_photon_core::settings::{AutoBehavior, AutoScope};

    #[test]
    fn bulk_action_requires_both_the_master_switch_and_feature_readiness() {
        assert!(bulk_action_available(true, true));
        assert!(!bulk_action_available(false, true));
        assert!(!bulk_action_available(true, false));
    }

    #[test]
    fn bulk_since_uses_the_activation_time_for_new_only() {
        let mut behavior = AutoBehavior::waiting();
        behavior.activated_at = Some(42);
        behavior.scope = AutoScope::NewOnly;
        assert_eq!(behavior.bulk_since(), Some(42));
        behavior.scope = AutoScope::IncludeBacklog;
        assert_eq!(behavior.bulk_since(), None);
    }
}
