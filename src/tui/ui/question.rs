//! Structured-question overlay. Not a permission approval.
//!
//! Options use the existing [`ChoicePrompt`]. Selecting a row answers the
//! parked `ask_user` oneshot with that option text. Free text stays on the
//! composer only when the question allows it or offers no options.

use a3s_tui::components::{ChoicePrompt, ChoicePromptItem, ChoicePromptMsg};
use a3s_tui::event::{KeyEvent, MouseEvent};
use a3s_tui::style::{fit_visible, Style};
use a3s_tui::KeyCode;

use super::{App, Cmd, Msg, State, TN_CYAN, TN_FG, TN_SUBTLE};

/// Last row when the model allowed a typed reply. It is not an option.
pub(super) const TYPE_A_REPLY: &str = "Type a reply";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum QuestionPromptMsg {
    Selected(String),
    Compose,
    Cancelled,
}

/// Single-select question picker. The answer is still one string.
#[derive(Clone, Debug)]
pub(super) struct QuestionPrompt {
    prompt: ChoicePrompt,
    options: Vec<String>,
    allow_free_text: bool,
}

impl QuestionPrompt {
    pub(super) fn new(
        question: &str,
        options: &[String],
        allow_free_text: bool,
        selected: usize,
    ) -> Self {
        let mut choices: Vec<ChoicePromptItem> = options
            .iter()
            .map(|option| ChoicePromptItem::new(option.clone()))
            .collect();
        if allow_free_text {
            choices.push(ChoicePromptItem::new(TYPE_A_REPLY));
        }
        let prompt = ChoicePrompt::new(question, choices)
            .selected(selected)
            .marker("→")
            .title_color(TN_CYAN)
            .text_color(TN_FG)
            .muted_color(TN_SUBTLE)
            .selected_colors(TN_FG, TN_CYAN)
            .hint(if allow_free_text {
                "↑/↓ navigate · Enter select · last row types a reply · Esc dismiss · not an approval"
            } else {
                "↑/↓ navigate · Enter select · Esc dismiss · this is not an approval"
            });
        Self {
            prompt,
            options: options.to_vec(),
            allow_free_text,
        }
    }

    pub(super) fn lines(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let height = self.prompt.choices().len().saturating_add(4).max(1);
        self.prompt
            .lines(width.min(u16::MAX as usize) as u16, height)
    }

    pub(super) fn selected_index(&self) -> usize {
        self.prompt.selected_index()
    }

    pub(super) fn set_y_offset(&mut self, y_offset: u16) {
        self.prompt.set_y_offset(y_offset);
    }

    pub(super) fn handle_key(&mut self, key: &KeyEvent) -> Option<QuestionPromptMsg> {
        self.prompt.handle_key(key).map(|msg| self.map_msg(msg))
    }

    pub(super) fn handle_mouse(&mut self, mouse: &MouseEvent) -> Option<QuestionPromptMsg> {
        self.prompt.handle_mouse(mouse).map(|msg| self.map_msg(msg))
    }

    fn map_msg(&self, msg: ChoicePromptMsg) -> QuestionPromptMsg {
        match msg {
            ChoicePromptMsg::Cancelled => QuestionPromptMsg::Cancelled,
            ChoicePromptMsg::Selected(index) => {
                if let Some(option) = self.options.get(index) {
                    QuestionPromptMsg::Selected(option.clone())
                } else if self.allow_free_text && index == self.options.len() {
                    QuestionPromptMsg::Compose
                } else {
                    QuestionPromptMsg::Cancelled
                }
            }
        }
    }
}

pub(super) fn question_banner_lines(question: &str, composing: bool, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let title = Style::new().fg(TN_CYAN).bold().render("  Question");
    let body = Style::new().fg(TN_FG).render(&format!("  {question}"));
    let hint = if composing {
        "  type a reply · Esc dismiss · this is not an approval"
    } else {
        "  reply below · Esc dismiss · this is not an approval"
    };
    let hint = Style::new().fg(TN_SUBTLE).render(hint);
    [title, body, hint]
        .into_iter()
        .map(|line| fit_visible(&line, width))
        .collect()
}

/// What the picker did. Answer and dismiss already resumed the parked oneshot.
#[derive(Clone, Debug, PartialEq, Eq)]
enum OverlayEffect {
    Answered { accepted: bool, text: String },
    Dismissed,
    Compose,
    Moved(usize),
    Unchanged,
}

/// Shared by the overlay key path and its end-to-end test. Selecting a row
/// answers the parked question here, so the test cannot pass by calling
/// `ask_user::answer` itself.
fn drive_overlay_key(
    question_id: &str,
    question: &str,
    options: &[String],
    allow_free_text: bool,
    selected: usize,
    key: &KeyEvent,
) -> OverlayEffect {
    let mut prompt = QuestionPrompt::new(question, options, allow_free_text, selected);
    match prompt.handle_key(key) {
        Some(msg) => commit_prompt_msg(question_id, msg),
        None => {
            let index = prompt.selected_index();
            if index == selected {
                OverlayEffect::Unchanged
            } else {
                OverlayEffect::Moved(index)
            }
        }
    }
}

fn commit_prompt_msg(question_id: &str, msg: QuestionPromptMsg) -> OverlayEffect {
    match msg {
        QuestionPromptMsg::Selected(text) => OverlayEffect::Answered {
            accepted: a3s_code_core::ask_user::answer(question_id, &text),
            text,
        },
        QuestionPromptMsg::Compose => OverlayEffect::Compose,
        QuestionPromptMsg::Cancelled => {
            let _ = a3s_code_core::ask_user::cancel(question_id);
            OverlayEffect::Dismissed
        }
    }
}

impl App {
    pub(super) fn handle_user_question_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let Some(pending) = self.pending_user_question.clone() else {
            return None;
        };
        if !pending.owns_picker() {
            if key.code == KeyCode::Esc {
                if pending.options.is_empty() {
                    return self.dismiss_pending_question();
                }
                self.leave_question_compose();
                return None;
            }
            return None;
        }
        let effect = drive_overlay_key(
            &pending.question_id,
            &pending.question,
            &pending.options,
            pending.allow_free_text,
            pending.selected,
            key,
        );
        self.apply_overlay_effect(effect)
    }

    fn apply_overlay_effect(&mut self, effect: OverlayEffect) -> Option<Cmd<Msg>> {
        match effect {
            OverlayEffect::Answered { accepted, text } => {
                self.finish_answered_question(accepted, &text)
            }
            OverlayEffect::Dismissed => self.finish_dismissed_question(),
            OverlayEffect::Compose => {
                self.begin_question_compose();
                None
            }
            OverlayEffect::Moved(index) => {
                if let Some(current) = self.pending_user_question.as_mut() {
                    current.selected = index;
                }
                None
            }
            OverlayEffect::Unchanged => None,
        }
    }

    pub(super) fn handle_user_question_mouse(&mut self, mouse: &MouseEvent) -> Option<Cmd<Msg>> {
        let Some(pending) = self.pending_user_question.clone() else {
            return None;
        };
        if !pending.owns_picker() {
            return None;
        }
        let width = (self.width as usize).min(u16::MAX as usize);
        if width == 0 {
            return None;
        }
        let mut prompt = QuestionPrompt::new(
            &pending.question,
            &pending.options,
            pending.allow_free_text,
            pending.selected,
        );
        let lines = prompt.lines(width);
        if lines.is_empty() {
            return None;
        }
        let y_offset = question_overlay_y_offset(
            self.height as usize,
            lines.len(),
            self.approval_rows_below(),
        );
        let row = mouse.row as usize;
        let start = y_offset as usize;
        if row < start || row >= start.saturating_add(lines.len()) {
            return None;
        }
        prompt.set_y_offset(y_offset);
        let before = prompt.selected_index();
        let effect = match prompt.handle_mouse(mouse) {
            Some(msg) => commit_prompt_msg(&pending.question_id, msg),
            None => {
                let after = prompt.selected_index();
                if after == before {
                    OverlayEffect::Unchanged
                } else {
                    OverlayEffect::Moved(after)
                }
            }
        };
        self.apply_overlay_effect(effect)
    }

    pub(super) fn overlay_user_question(&self, composed: String) -> String {
        let Some(pending) = self.pending_user_question.as_ref() else {
            return composed;
        };
        if self.state == State::Awaiting && !self.pending_tools.is_empty() {
            return composed;
        }
        let width = self.width as usize;
        let lines = if pending.owns_picker() {
            QuestionPrompt::new(
                &pending.question,
                &pending.options,
                pending.allow_free_text,
                pending.selected,
            )
            .lines(width)
        } else {
            question_banner_lines(&pending.question, pending.composing, width)
        };
        self.overlay_list_with_rows_below(composed, &lines, self.approval_rows_below())
    }

    pub(super) fn answer_pending_question(&mut self, text: &str) -> Option<Cmd<Msg>> {
        let Some(pending) = self.pending_user_question.as_ref() else {
            return None;
        };
        let answered = a3s_code_core::ask_user::answer(&pending.question_id, text);
        self.finish_answered_question(answered, text)
    }

    fn finish_answered_question(&mut self, answered: bool, text: &str) -> Option<Cmd<Msg>> {
        let Some(pending) = self.pending_user_question.take() else {
            return None;
        };
        let line = if answered {
            format!("  answered · {text}")
        } else {
            "  question already closed".to_string()
        };
        self.push_line(&Style::new().fg(TN_CYAN).render(&line));
        self.restore_question_draft(pending.stashed_composer);
        self.relayout();
        None
    }

    pub(super) fn dismiss_pending_question(&mut self) -> Option<Cmd<Msg>> {
        if let Some(pending) = self.pending_user_question.as_ref() {
            let _ = a3s_code_core::ask_user::cancel(&pending.question_id);
        }
        self.finish_dismissed_question()
    }

    fn finish_dismissed_question(&mut self) -> Option<Cmd<Msg>> {
        let Some(pending) = self.pending_user_question.take() else {
            return None;
        };
        if let Some(draft) = pending.stashed_composer {
            self.textarea.set_value(&draft);
        }
        self.push_line(
            &Style::new()
                .fg(TN_SUBTLE)
                .render("  question dismissed · not an approval"),
        );
        self.relayout();
        None
    }

    fn begin_question_compose(&mut self) {
        if self.pending_user_question.is_none() {
            return;
        }
        let draft = self.textarea.value();
        if let Some(pending) = self.pending_user_question.as_mut() {
            if pending.stashed_composer.is_none() {
                pending.stashed_composer = Some(draft);
            }
            pending.composing = true;
        }
        self.textarea.clear();
        self.relayout();
    }

    fn leave_question_compose(&mut self) {
        let draft = self
            .pending_user_question
            .as_ref()
            .and_then(|pending| pending.stashed_composer.clone());
        if let Some(pending) = self.pending_user_question.as_mut() {
            pending.composing = false;
            pending.stashed_composer = None;
        }
        self.restore_question_draft(draft);
        self.relayout();
    }

    fn restore_question_draft(&mut self, draft: Option<String>) {
        match draft {
            Some(draft) => self.textarea.set_value(&draft),
            None => self.textarea.clear(),
        }
    }
}

fn question_overlay_y_offset(screen_height: usize, row_count: usize, rows_below: usize) -> u16 {
    screen_height
        .saturating_sub(rows_below)
        .saturating_sub(row_count)
        .min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_tui::event::KeyEvent;
    use a3s_tui::style::strip_ansi;
    use a3s_tui::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn visible(lines: &[String]) -> String {
        lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn options_render_as_a_picker_not_an_approval_menu() {
        let prompt = QuestionPrompt::new("Which name?", &["left".into(), "right".into()], false, 0);
        let rendered = visible(&prompt.lines(80));
        assert!(rendered.contains("Which name?"));
        assert!(rendered.contains("left"));
        assert!(rendered.contains("right"));
        assert!(rendered.contains("not an approval"));
        assert!(!rendered.to_ascii_lowercase().contains("allow"));
        assert!(!rendered.to_ascii_lowercase().contains("permission"));
    }

    #[test]
    fn arrow_and_enter_select_the_option_text_not_a_typed_reply() {
        let mut prompt =
            QuestionPrompt::new("Which name?", &["left".into(), "right".into()], true, 0);
        assert!(prompt.handle_key(&key(KeyCode::Down)).is_none());
        assert_eq!(
            prompt.handle_key(&key(KeyCode::Enter)),
            Some(QuestionPromptMsg::Selected("right".into()))
        );
    }

    #[test]
    fn free_text_row_opens_the_composer_instead_of_answering_its_label() {
        let mut prompt = QuestionPrompt::new("Which name?", &["left".into()], true, 1);
        assert_eq!(
            prompt.handle_key(&key(KeyCode::Enter)),
            Some(QuestionPromptMsg::Compose)
        );
    }

    #[test]
    fn escape_dismisses_without_selecting() {
        let mut prompt = QuestionPrompt::new("Continue?", &["yes".into()], false, 0);
        assert_eq!(
            prompt.handle_key(&key(KeyCode::Esc)),
            Some(QuestionPromptMsg::Cancelled)
        );
    }

    #[tokio::test]
    async fn overlay_selection_resumes_the_parked_question() {
        let (question, rx) = a3s_code_core::ask_user::begin(
            "overlay-run",
            "ask-overlay",
            "Which token?",
            &["left".into(), "right".into()],
            false,
        )
        .unwrap();
        let moved = drive_overlay_key(
            &question.question_id,
            &question.question,
            &question.options,
            question.allow_free_text,
            0,
            &key(KeyCode::Down),
        );
        assert_eq!(moved, OverlayEffect::Moved(1));
        let selected = drive_overlay_key(
            &question.question_id,
            &question.question,
            &question.options,
            question.allow_free_text,
            1,
            &key(KeyCode::Enter),
        );
        assert_eq!(
            selected,
            OverlayEffect::Answered {
                accepted: true,
                text: "right".into(),
            }
        );
        match rx.await.unwrap() {
            a3s_code_core::ask_user::AskUserResume::Answered { text } => {
                assert_eq!(text, "right");
                let message = a3s_code_core::ask_user::resume_message(
                    &question.question_id,
                    &a3s_code_core::ask_user::AskUserResume::Answered { text },
                );
                assert!(message.contains("\"permission_grant\":false"));
                assert!(message.contains("right"));
            }
            a3s_code_core::ask_user::AskUserResume::Unanswered => {
                panic!("selecting an option must resume the parked question");
            }
        }
    }

    #[tokio::test]
    async fn overlay_escape_dismisses_as_unanswered() {
        let (question, rx) = a3s_code_core::ask_user::begin(
            "overlay-dismiss",
            "ask-dismiss",
            "Continue?",
            &["yes".into()],
            false,
        )
        .unwrap();
        assert_eq!(
            drive_overlay_key(
                &question.question_id,
                &question.question,
                &question.options,
                question.allow_free_text,
                0,
                &key(KeyCode::Esc),
            ),
            OverlayEffect::Dismissed
        );
        assert!(matches!(
            rx.await.unwrap(),
            a3s_code_core::ask_user::AskUserResume::Unanswered
        ));
    }
}
