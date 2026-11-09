//! Floating search-suggestion popup.

use gpui_kit::base::POPUP_PRIORITY;
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vrc_photon_i18n::tr_args;

use crate::app::App as PhotoApp;
use crate::app::search_suggest::SuggestionKind;
use crate::ui::theme::format_count;

const ROW_HEIGHT: f32 = 27.0;
const SURFACE_PADDING: f32 = 4.0;
const ANCHOR_GAP: f32 = 4.0;
const VIEWPORT_MARGIN: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SuggestionKeyAction {
    Previous,
    Next,
    Confirm,
}

pub(crate) fn suggestion_key_action(key: &str) -> Option<SuggestionKeyAction> {
    match key {
        "up" => Some(SuggestionKeyAction::Previous),
        "down" => Some(SuggestionKeyAction::Next),
        "enter" => Some(SuggestionKeyAction::Confirm),
        _ => None,
    }
}

pub(crate) fn suggestion_popup_origin(
    input: Bounds<Pixels>,
    viewport: gpui_kit::Size<Pixels>,
    item_count: usize,
) -> Point<Pixels> {
    let height = px(item_count as f32 * ROW_HEIGHT + SURFACE_PADDING * 2.0);
    let margin = px(VIEWPORT_MARGIN);
    let below = input.bottom() + px(ANCHOR_GAP);
    let y = if below + height <= viewport.height - margin {
        below
    } else {
        let above = input.top() - height - px(ANCHOR_GAP);
        if above < margin { margin } else { above }
    };

    let right_limit = viewport.width - margin;
    let x = if input.left() + input.size.width > right_limit {
        let clamped = right_limit - input.size.width;
        if clamped < margin { margin } else { clamped }
    } else {
        input.left()
    };

    point(x, y)
}

impl PhotoApp {
    pub(crate) fn suggestion_field(
        &self,
        kind: SuggestionKind,
        label: impl Into<String>,
        input: Input,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette;
        let label: String = label.into();
        let bounds_map = self.suggestion_bounds.clone();
        v_flex()
            .gap(px(6.))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                // Escape is left to the input so an active IME composition is
                // cancelled by the input before the popup is dismissed.
                if event.keystroke.key == "escape" {
                    this.dismiss_suggestion(cx);
                }
            }))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted_2))
                    .child(label),
            )
            .child(
                div().relative().child(input.w_full()).child(
                    canvas(
                        move |bounds, _, _| {
                            bounds_map.borrow_mut().insert(kind, bounds);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                ),
            )
    }

    pub(crate) fn suggestion_popup(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.suggestion.as_ref()?;
        if state.dismissed || state.items.is_empty() {
            return None;
        }
        let bounds = *self.suggestion_bounds.borrow().get(&state.kind)?;
        let items = state.items.clone();
        let selected = state.selected;
        let kind = state.kind;
        let p = self.palette;

        let mut rows = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let name = item.name.clone();
            let count = tr_args!("search.count", total = format_count(item.photo_count));
            let hover = move |style: StyleRefinement| style.bg(rgb(p.panel_inner));
            rows.push(
                div()
                    .id(SharedString::from(format!(
                        "search-suggestion-{kind:?}-{index}"
                    )))
                    .relative()
                    .flex()
                    .items_center()
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .pl(px(11.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .when(index == selected, |this| this.bg(rgb(p.accent_soft)))
                    .when(index == selected, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(3.))
                                .top(px(6.))
                                .bottom(px(6.))
                                .w(px(2.))
                                .rounded_full()
                                .bg(rgb(p.accent)),
                        )
                    })
                    .when(index != selected, |this| this.hover(hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.select_suggestion(index, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(10.5))
                            .text_color(rgb(if index == selected { p.text } else { p.muted }))
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(9.5))
                            .text_color(rgb(if index == selected {
                                p.accent
                            } else {
                                p.muted_2
                            }))
                            .child(count),
                    ),
            );
        }

        let surface = div()
            .w(bounds.size.width)
            .p(px(SURFACE_PADDING))
            .rounded(px(11.))
            .bg(rgb(p.panel_2))
            .border_1()
            .border_color(rgb(p.border_strong))
            .shadow(vec![
                BoxShadow::new(px(0.), px(18.), rgba(p.shadow).into()).blur_radius(px(42.)),
            ])
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_suggestion(cx)))
            .children(rows);

        let origin = suggestion_popup_origin(bounds, window.viewport_size(), items.len());
        Some(
            deferred(
                anchored()
                    .position(origin)
                    .snap_to_window_with_margin(px(VIEWPORT_MARGIN))
                    .child(surface),
            )
            .with_priority(POPUP_PRIORITY)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{SuggestionKeyAction, suggestion_key_action, suggestion_popup_origin};
    use gpui_kit::{Bounds, point, px, size};

    #[test]
    fn popup_opens_below_when_there_is_room() {
        let input = Bounds {
            origin: point(px(10.), px(10.)),
            size: size(px(200.), px(34.)),
        };
        let origin = suggestion_popup_origin(input, size(px(1000.), px(800.)), 5);
        assert_eq!(origin, point(px(10.), px(48.)));
    }

    #[test]
    fn popup_flips_above_when_below_would_overflow() {
        let input = Bounds {
            origin: point(px(10.), px(740.)),
            size: size(px(200.), px(34.)),
        };
        let origin = suggestion_popup_origin(input, size(px(1000.), px(800.)), 5);
        assert!(origin.y < input.origin.y);
    }

    #[test]
    fn suggestion_keys_map_to_actions() {
        assert_eq!(
            suggestion_key_action("up"),
            Some(SuggestionKeyAction::Previous)
        );
        assert_eq!(
            suggestion_key_action("down"),
            Some(SuggestionKeyAction::Next)
        );
        assert_eq!(
            suggestion_key_action("enter"),
            Some(SuggestionKeyAction::Confirm)
        );
        assert_eq!(suggestion_key_action("a"), None);
    }

    #[test]
    fn escape_is_left_to_the_input_and_ime() {
        assert_eq!(suggestion_key_action("escape"), None);
    }
}
