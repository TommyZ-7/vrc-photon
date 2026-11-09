//! Search-screen autocomplete state and background queries.

use std::path::Path;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use vrc_photon_core::db::Db;
use vrc_photon_core::model::SearchSuggestion;

use crate::app::{App as PhotoApp, Screen};
use crate::ui::search_suggest::{SuggestionKeyAction, suggestion_key_action};

pub(crate) const SUGGESTION_LIMIT: usize = 10;
pub(crate) const SUGGESTION_DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SuggestionKind {
    World,
    Player,
    Tag,
}

#[derive(Clone, Debug)]
pub(crate) struct SuggestionState {
    pub(crate) kind: SuggestionKind,
    pub(crate) query: String,
    pub(crate) items: Vec<SearchSuggestion>,
    pub(crate) selected: usize,
    pub(crate) dismissed: bool,
}

pub(crate) fn normalized_suggestion_query(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(crate) fn move_suggestion_selection(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    current.saturating_add_signed(delta).min(len - 1)
}

pub(crate) fn suggestion_result_is_current(result: u64, current: u64) -> bool {
    result == current
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlurAction {
    Keep,
    Close,
}

pub(crate) fn blur_action(state: Option<&SuggestionState>, kind: SuggestionKind) -> BlurAction {
    match state {
        Some(state) if state.kind == kind && !state.dismissed => BlurAction::Close,
        _ => BlurAction::Keep,
    }
}

pub(crate) fn focus_should_start(
    state: Option<&SuggestionState>,
    kind: SuggestionKind,
    query: &str,
) -> bool {
    !state.is_some_and(|state| state.kind == kind && state.query == query)
}

/// Builds the state for a new query while keeping the previous candidates
/// visible until its result arrives. Candidates only carry over between
/// queries of the same field, so one field never shows another field's list.
fn begin_suggestion_state(
    previous: Option<&SuggestionState>,
    kind: SuggestionKind,
    query: String,
) -> SuggestionState {
    let previous = previous.filter(|state| state.kind == kind);
    SuggestionState {
        kind,
        query,
        items: previous
            .map(|state| state.items.clone())
            .unwrap_or_default(),
        selected: previous.map(|state| state.selected).unwrap_or(0),
        dismissed: false,
    }
}

// gpui-kit 0.9 exposes no composition flag on `InputState`. Printable
// composition keys are covered by `is_ime_in_progress`; arrow keys during
// composition rely on the platform IME consuming them before GPUI sees them.
// Escape is deliberately not intercepted here: the input handles IME cancel
// first, and the field wrapper dismisses the popup from the propagated event.
fn suggestion_key_action_for_key(key: &str, ime_in_progress: bool) -> Option<SuggestionKeyAction> {
    if ime_in_progress {
        return None;
    }
    suggestion_key_action(key)
}

fn query_suggestions(
    db: &Db,
    kind: SuggestionKind,
    query: &str,
    prefer_english: bool,
) -> vrc_photon_core::error::Result<Vec<SearchSuggestion>> {
    match kind {
        SuggestionKind::World => db.world_name_suggestions(query, SUGGESTION_LIMIT),
        SuggestionKind::Player => db.player_name_suggestions(query, SUGGESTION_LIMIT),
        SuggestionKind::Tag => db.tag_name_suggestions(query, SUGGESTION_LIMIT, prefer_english),
    }
}

impl PhotoApp {
    fn suggestion_input(&self, kind: SuggestionKind) -> &Entity<InputState> {
        match kind {
            SuggestionKind::World => &self.world_input,
            SuggestionKind::Player => &self.player_input,
            SuggestionKind::Tag => &self.tag_input,
        }
    }

    fn suggestion_input_value(&self, kind: SuggestionKind, cx: &gpui_kit::App) -> String {
        self.suggestion_input(kind).read(cx).value().to_string()
    }

    pub(crate) fn on_suggestion_event(
        &mut self,
        kind: SuggestionKind,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => self.on_suggestion_input_changed(kind, cx),
            InputEvent::Focus => {
                let Some(query) =
                    normalized_suggestion_query(&self.suggestion_input_value(kind, cx))
                else {
                    self.close_suggestion(cx);
                    return;
                };
                if focus_should_start(self.suggestion.as_ref(), kind, &query) {
                    self.begin_suggestion_query(kind, query, cx);
                }
            }
            InputEvent::Blur => {
                if blur_action(self.suggestion.as_ref(), kind) == BlurAction::Close {
                    self.close_suggestion(cx);
                }
            }
            InputEvent::PressEnter { .. } => {}
        }
    }

    fn on_suggestion_input_changed(&mut self, kind: SuggestionKind, cx: &mut Context<Self>) {
        let Some(query) = normalized_suggestion_query(&self.suggestion_input_value(kind, cx))
        else {
            self.close_suggestion(cx);
            return;
        };
        self.begin_suggestion_query(kind, query, cx);
    }

    fn begin_suggestion_query(
        &mut self,
        kind: SuggestionKind,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let request = self.suggestion_request.wrapping_add(1);
        self.suggestion_request = request;
        self.suggestion = Some(begin_suggestion_state(
            self.suggestion.as_ref(),
            kind,
            query.clone(),
        ));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SUGGESTION_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                if !suggestion_result_is_current(request, this.suggestion_request) {
                    return;
                }
                let still_wanted = this.suggestion.as_ref().is_some_and(|state| {
                    state.kind == kind && state.query == query && !state.dismissed
                });
                if still_wanted {
                    this.start_suggestion_query(request, kind, query, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn start_suggestion_query(
        &mut self,
        request: u64,
        kind: SuggestionKind,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let prefer_english = vrc_photon_i18n::current_locale().starts_with("en");
        let Some(path) = self.db.path().map(Path::to_path_buf) else {
            let result = query_suggestions(&self.db, kind, &query, prefer_english);
            self.apply_suggestion_result(request, result, cx);
            return;
        };

        cx.spawn(async move |this, cx| {
            let background_query = query.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    Db::open_read_only(path).and_then(|db| {
                        query_suggestions(&db, kind, &background_query, prefer_english)
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                if !suggestion_result_is_current(request, this.suggestion_request) {
                    return;
                }
                match result {
                    Ok(items) => this.apply_suggestion_result(request, Ok(items), cx),
                    Err(_) => {
                        let fallback = query_suggestions(&this.db, kind, &query, prefer_english);
                        this.apply_suggestion_result(request, fallback, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply_suggestion_result(
        &mut self,
        request: u64,
        result: vrc_photon_core::error::Result<Vec<SearchSuggestion>>,
        cx: &mut Context<Self>,
    ) {
        if !suggestion_result_is_current(request, self.suggestion_request)
            || self.screen != Screen::Search
        {
            return;
        }
        let Some(state) = self.suggestion.as_mut() else {
            return;
        };
        if state.dismissed {
            return;
        }
        match result {
            Ok(items) => {
                state.items = items;
                state.selected = 0;
            }
            Err(_) => {
                self.suggestion = None;
            }
        }
        cx.notify();
    }

    pub(crate) fn dismiss_suggestion(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.suggestion.as_mut() else {
            return;
        };
        state.dismissed = true;
        state.items.clear();
        cx.notify();
    }

    pub(crate) fn close_suggestion(&mut self, cx: &mut Context<Self>) {
        let had_suggestion = self.suggestion.take().is_some();
        self.suggestion_request = self.suggestion_request.wrapping_add(1);
        if had_suggestion {
            cx.notify();
        }
    }

    /// Closes the popup when the filter panel scrolls, because the popup is
    /// anchored to the input's cached bounds and would otherwise drift away
    /// from its field.
    pub(crate) fn close_suggestion_on_scroll(&mut self, cx: &mut Context<Self>) {
        if self.suggestion.is_some() {
            self.close_suggestion(cx);
        }
    }

    pub(crate) fn select_suggestion(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.suggestion.as_ref() else {
            return;
        };
        let Some(item) = state.items.get(index) else {
            return;
        };
        let kind = state.kind;
        let name = item.name.clone();
        let input = self.suggestion_input(kind).clone();
        self.close_suggestion(cx);
        input.update(cx, |input, cx| input.set_value(name, window, cx));
        window.focus(&input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(crate) fn handle_suggestion_key(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.screen != Screen::Search {
            return false;
        }
        let Some(action) = suggestion_key_action_for_key(
            &event.keystroke.key,
            event.keystroke.is_ime_in_progress(),
        ) else {
            return false;
        };
        let Some(state) = self.suggestion.as_ref() else {
            return false;
        };
        if state.dismissed || state.items.is_empty() {
            return false;
        }
        let kind = state.kind;
        let focused = self
            .suggestion_input(kind)
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !focused {
            return false;
        }

        match action {
            SuggestionKeyAction::Previous | SuggestionKeyAction::Next => {
                let delta = if action == SuggestionKeyAction::Previous {
                    -1
                } else {
                    1
                };
                let state = self.suggestion.as_mut().unwrap();
                state.selected =
                    move_suggestion_selection(state.selected, state.items.len(), delta);
                cx.notify();
                true
            }
            SuggestionKeyAction::Confirm => {
                let index = self.suggestion.as_ref().unwrap().selected;
                self.select_suggestion(index, window, cx);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BlurAction, SuggestionKind, SuggestionState, begin_suggestion_state, blur_action,
        focus_should_start, move_suggestion_selection, normalized_suggestion_query,
        suggestion_key_action_for_key, suggestion_result_is_current,
    };
    use crate::ui::search_suggest::SuggestionKeyAction;
    use vrc_photon_core::model::SearchSuggestion;

    fn state(kind: SuggestionKind, query: &str, dismissed: bool) -> SuggestionState {
        SuggestionState {
            kind,
            query: query.into(),
            items: Vec::new(),
            selected: 0,
            dismissed,
        }
    }

    fn with_items(
        kind: SuggestionKind,
        query: &str,
        names: &[&str],
        selected: usize,
    ) -> SuggestionState {
        let mut state = state(kind, query, false);
        state.items = names
            .iter()
            .map(|name| SearchSuggestion {
                name: (*name).into(),
                photo_count: 1,
            })
            .collect();
        state.selected = selected;
        state
    }

    #[test]
    fn ime_keystrokes_are_not_intercepted() {
        assert_eq!(suggestion_key_action_for_key("enter", true), None);
        assert_eq!(
            suggestion_key_action_for_key("enter", false),
            Some(SuggestionKeyAction::Confirm)
        );
    }

    #[test]
    fn blur_only_affects_its_own_field_and_keeps_dismissed_state() {
        let tag = state(SuggestionKind::Tag, "夜", false);
        let dismissed = state(SuggestionKind::Tag, "夜", true);
        assert_eq!(
            blur_action(Some(&tag), SuggestionKind::Tag),
            BlurAction::Close
        );
        assert_eq!(
            blur_action(Some(&tag), SuggestionKind::Player),
            BlurAction::Keep
        );
        assert_eq!(
            blur_action(Some(&dismissed), SuggestionKind::Tag),
            BlurAction::Keep
        );
        assert_eq!(blur_action(None, SuggestionKind::Tag), BlurAction::Keep);
    }

    #[test]
    fn focus_starts_only_for_a_new_field_or_query() {
        let tag = state(SuggestionKind::Tag, "夜", false);
        let dismissed = state(SuggestionKind::Tag, "夜", true);
        assert!(!focus_should_start(Some(&tag), SuggestionKind::Tag, "夜"));
        assert!(!focus_should_start(
            Some(&dismissed),
            SuggestionKind::Tag,
            "夜"
        ));
        assert!(focus_should_start(Some(&tag), SuggestionKind::Tag, "夜桜"));
        assert!(focus_should_start(Some(&tag), SuggestionKind::Player, "夜"));
        assert!(focus_should_start(None, SuggestionKind::Tag, "夜"));
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        assert_eq!(move_suggestion_selection(0, 5, -1), 0);
        assert_eq!(move_suggestion_selection(0, 5, 1), 1);
        assert_eq!(move_suggestion_selection(4, 5, 1), 4);
        assert_eq!(move_suggestion_selection(0, 0, 1), 0);
    }

    #[test]
    fn blank_queries_are_not_searchable() {
        assert_eq!(normalized_suggestion_query("  "), None);
        assert_eq!(
            normalized_suggestion_query("  sunset  "),
            Some("sunset".into())
        );
    }

    #[test]
    fn stale_results_are_rejected() {
        assert!(suggestion_result_is_current(7, 7));
        assert!(!suggestion_result_is_current(6, 7));
    }

    #[test]
    fn pending_query_keeps_candidates_from_the_same_field() {
        let previous = with_items(SuggestionKind::World, "夜", &["夜の街", "夜桜"], 1);
        let next = begin_suggestion_state(Some(&previous), SuggestionKind::World, "夜桜".into());
        assert_eq!(next.query, "夜桜");
        assert_eq!(next.items, previous.items);
        assert_eq!(next.selected, 1);
        assert!(!next.dismissed);
    }

    #[test]
    fn pending_query_starts_empty_when_the_field_changes() {
        let previous = with_items(SuggestionKind::World, "夜", &["夜の街"], 0);
        let next = begin_suggestion_state(Some(&previous), SuggestionKind::Player, "night".into());
        assert!(next.items.is_empty());
        assert_eq!(next.selected, 0);
    }
}
