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

pub(crate) fn focus_should_query(dismissed: bool) -> bool {
    !dismissed
}

fn suggestion_key_action_for_key(
    key: &str,
    ime_in_progress: bool,
) -> Option<SuggestionKeyAction> {
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
        SuggestionKind::Tag => {
            db.tag_name_suggestions(query, SUGGESTION_LIMIT, prefer_english)
        }
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
                    normalized_suggestion_query(&self.suggestion_input_value(kind, &**cx))
                else {
                    self.close_suggestion(cx);
                    return;
                };
                let reuse = self.suggestion.as_ref().is_some_and(|state| {
                    state.kind == kind && state.query == query && !state.dismissed
                });
                if !reuse {
                    self.begin_suggestion_query(kind, query, cx);
                }
            }
            InputEvent::Blur => self.close_suggestion(cx),
            InputEvent::PressEnter { .. } => {}
        }
    }

    fn on_suggestion_input_changed(&mut self, kind: SuggestionKind, cx: &mut Context<Self>) {
        let Some(query) = normalized_suggestion_query(&self.suggestion_input_value(kind, &**cx))
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
        self.suggestion = Some(SuggestionState {
            kind,
            query: query.clone(),
            items: Vec::new(),
            selected: 0,
            dismissed: false,
        });
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
                        let fallback =
                            query_suggestions(&this.db, kind, &query, prefer_english);
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
            .read(&**cx)
            .focus_handle(&**cx)
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
            SuggestionKeyAction::Dismiss => {
                self.dismiss_suggestion(cx);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        focus_should_query, move_suggestion_selection, normalized_suggestion_query,
        suggestion_key_action_for_key, suggestion_result_is_current,
    };
    use crate::ui::search_suggest::SuggestionKeyAction;

    #[test]
    fn ime_keystrokes_are_not_intercepted() {
        assert_eq!(suggestion_key_action_for_key("enter", true), None);
        assert_eq!(
            suggestion_key_action_for_key("enter", false),
            Some(SuggestionKeyAction::Confirm)
        );
        assert_eq!(suggestion_key_action_for_key("up", true), None);
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
    fn dismissed_state_suppresses_focus_until_the_next_change() {
        assert!(!focus_should_query(true));
        assert!(focus_should_query(false));
    }
}
