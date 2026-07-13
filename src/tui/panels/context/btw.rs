//! `/btw` side-chat overlay and its in-memory request/history state.

use super::super::*;
use a3s_tui::components::SideNotePanel;
use tokio_util::sync::CancellationToken;

pub(crate) const BTW_HISTORY_LIMIT: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BtwHistoryEntry {
    pub(crate) question: String,
    pub(crate) answer: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BtwAnswer {
    Loading,
    Ready(String),
    Failed(String),
}

pub(crate) struct BtwPanelState {
    request_id: u64,
    entries: Vec<(String, BtwAnswer)>,
    selected: usize,
    cancellation: CancellationToken,
    copied: bool,
}

impl BtwPanelState {
    pub(crate) fn start(
        request_id: u64,
        question: String,
        history: &[BtwHistoryEntry],
        cancellation: CancellationToken,
    ) -> Self {
        let history_start = history
            .len()
            .saturating_sub(BTW_HISTORY_LIMIT.saturating_sub(1));
        let mut entries = history[history_start..]
            .iter()
            .map(|entry| {
                (
                    entry.question.clone(),
                    BtwAnswer::Ready(entry.answer.clone()),
                )
            })
            .collect::<Vec<_>>();
        entries.push((question, BtwAnswer::Loading));
        let selected = entries.len().saturating_sub(1);
        Self {
            request_id,
            entries,
            selected,
            cancellation,
            copied: false,
        }
    }

    pub(crate) fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub(crate) fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.copied = false;
    }

    pub(crate) fn select_next(&mut self) {
        self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
        self.copied = false;
    }

    pub(crate) fn mark_copied(&mut self) {
        self.copied = true;
    }

    pub(crate) fn copy_text(&self) -> Option<&str> {
        match self.entries.get(self.selected).map(|(_, answer)| answer) {
            Some(BtwAnswer::Ready(answer)) => Some(answer),
            _ => None,
        }
    }

    pub(crate) fn finish(
        &mut self,
        request_id: u64,
        result: Result<String, String>,
    ) -> Option<BtwHistoryEntry> {
        if request_id != self.request_id {
            return None;
        }
        let (question, answer) = self.entries.last_mut()?;
        if !matches!(answer, BtwAnswer::Loading) {
            return None;
        }
        let history_entry = match result {
            Ok(answer_text) => {
                let answer_text = answer_text.trim().to_string();
                if answer_text.is_empty() {
                    *answer =
                        BtwAnswer::Failed("side question ended without an answer".to_string());
                    None
                } else {
                    *answer = BtwAnswer::Ready(answer_text.clone());
                    Some(BtwHistoryEntry {
                        question: question.clone(),
                        answer: answer_text,
                    })
                }
            }
            Err(error) => {
                *answer = BtwAnswer::Failed(error);
                None
            }
        };
        self.selected = self.entries.len().saturating_sub(1);
        self.copied = false;
        history_entry
    }

    fn current(&self) -> Option<(&str, Option<&str>)> {
        self.entries.get(self.selected).map(|(question, answer)| {
            let answer = match answer {
                BtwAnswer::Loading => None,
                BtwAnswer::Ready(answer) | BtwAnswer::Failed(answer) => Some(answer.as_str()),
            };
            (question.as_str(), answer)
        })
    }

    fn title(&self) -> String {
        if self.copied {
            return "↘ by the way · copied · Esc close".to_string();
        }
        format!(
            "↘ by the way · {}/{} · ←/→ history · c copy · Esc close",
            self.selected.saturating_add(1),
            self.entries.len()
        )
    }
}

pub(crate) async fn run_btw_request(
    agent: Arc<Agent>,
    workspace: String,
    options: SessionOptions,
    question: String,
    history: Vec<a3s_code_core::Message>,
    cancellation: CancellationToken,
) -> Result<String, String> {
    let session = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err("side question cancelled".to_string()),
        session = agent.session_async(workspace, Some(options)) => {
            session.map_err(|error| format!("could not start side question: {error}"))?
        }
    };
    let (mut rx, join) = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err("side question cancelled".to_string()),
        stream = session.stream(&question, Some(&history)) => {
            stream.map_err(|error| format!("could not stream side question: {error}"))?
        }
    };
    let mut join = Some(join);
    let mut answer = String::new();
    let mut stream_error = None;
    let mut completed = false;

    loop {
        let event = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                session.cancel().await;
                if let Some(join) = join.take() {
                    let _ = join.await;
                }
                return Err("side question cancelled".to_string());
            }
            event = rx.recv() => event,
        };
        let Some(event) = event else {
            break;
        };
        match event {
            AgentEvent::TextDelta { text } => answer.push_str(&text),
            AgentEvent::End { text, .. } => {
                if answer.trim().is_empty() {
                    answer = text;
                }
                completed = true;
                break;
            }
            AgentEvent::Error { message } => {
                stream_error = Some(message);
                session.cancel().await;
                break;
            }
            _ => {}
        }
    }

    if let Some(join) = join.take() {
        let _ = join.await;
    }
    if let Some(error) = stream_error {
        return Err(error);
    }
    if !completed {
        return Err("side question stream ended before completion".to_string());
    }
    let answer = answer.trim().to_string();
    if answer.is_empty() {
        Err("side question ended without an answer".to_string())
    } else {
        Ok(answer)
    }
}

fn btw_panel_lines(state: &BtwPanelState, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let Some((question, answer)) = state.current() else {
        return Vec::new();
    };

    let mut panel = SideNotePanel::new(state.title())
        .question(question)
        .loading_text("thinking…")
        .max_body_lines(12)
        .indent(2)
        .title_color(TN_YELLOW)
        .question_color(TN_YELLOW)
        .answer_color(TN_YELLOW)
        .muted_color(TN_GRAY);
    if let Some(answer) = answer {
        panel = panel.answer(answer);
    }

    panel
        .view(width.min(u16::MAX as usize) as u16, usize::MAX)
        .lines()
        .map(str::to_string)
        .collect()
}

impl App {
    /// `/btw` side-chat panel above the input: the question and its answer.
    pub(crate) fn overlay_btw(&self, composed: String) -> String {
        let Some(state) = &self.btw else {
            return composed;
        };
        let width = self.width as usize;
        let lines = btw_panel_lines(state, width);
        self.overlay_list(composed, &lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestWorkspace(std::path::PathBuf);

    impl TestWorkspace {
        fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("a3s-{label}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&path).expect("create temporary workspace");
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn panel(question: &str, answer: Option<&str>) -> BtwPanelState {
        let mut panel =
            BtwPanelState::start(7, question.to_string(), &[], CancellationToken::new());
        if let Some(answer) = answer {
            panel.finish(7, Ok(answer.to_string()));
        }
        panel
    }

    #[test]
    fn btw_panel_lines_are_width_bounded_with_styles() {
        let state = panel(
            "Can this long side question stay inside the available width?",
            Some("Yes, the shared side-note panel wraps the compact answer safely."),
        );
        let lines = btw_panel_lines(&state, 24);

        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 24),
            "{:?}",
            lines
                .iter()
                .map(|line| a3s_tui::style::strip_ansi(line))
                .collect::<Vec<_>>()
        );
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("by the way"), "{plain}");
        assert!(plain.contains("Q:"), "{plain}");
        assert!(plain.contains("shared"), "{plain}");
        assert!(plain.contains("side-note"), "{plain}");
        assert!(
            lines.iter().any(|line| line.contains("\x1b[")),
            "side note panel should carry styling"
        );
    }

    #[test]
    fn btw_panel_lines_use_loading_fallback() {
        let plain = btw_panel_lines(&panel("Still working?", None), 40)
            .into_iter()
            .map(|line| a3s_tui::style::strip_ansi(&line))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(plain.contains("Still working"), "{plain}");
        assert!(
            plain.contains("thinking"),
            "loading fallback should render: {plain}"
        );
    }

    #[test]
    fn btw_history_navigation_and_raw_markdown_copy_are_in_memory_only() {
        let history = vec![
            BtwHistoryEntry {
                question: "old one".into(),
                answer: "**first**".into(),
            },
            BtwHistoryEntry {
                question: "old two".into(),
                answer: "`second`".into(),
            },
        ];
        let mut state =
            BtwPanelState::start(9, "new question".into(), &history, CancellationToken::new());
        state.finish(9, Ok("# newest\n\nbody".into()));

        assert_eq!(state.copy_text(), Some("# newest\n\nbody"));
        state.select_previous();
        assert_eq!(state.copy_text(), Some("`second`"));
        state.select_previous();
        assert_eq!(state.copy_text(), Some("**first**"));
        state.select_next();
        assert_eq!(state.copy_text(), Some("`second`"));
    }

    #[test]
    fn stale_btw_completion_cannot_replace_the_active_request() {
        let mut state = BtwPanelState::start(12, "active".into(), &[], CancellationToken::new());

        assert_eq!(state.finish(11, Ok("stale".into())), None);
        assert_eq!(state.copy_text(), None);
        assert_eq!(
            state.finish(12, Ok("current".into())),
            Some(BtwHistoryEntry {
                question: "active".into(),
                answer: "current".into(),
            })
        );
        assert_eq!(state.copy_text(), Some("current"));
        assert_eq!(state.finish(12, Ok("duplicate".into())), None);
        assert_eq!(state.copy_text(), Some("current"));
    }

    #[test]
    fn empty_btw_completion_is_an_error_and_never_enters_history() {
        let mut state = BtwPanelState::start(13, "active".into(), &[], CancellationToken::new());

        assert_eq!(state.finish(13, Ok("  \n".into())), None);
        assert_eq!(state.copy_text(), None);
        let plain = btw_panel_lines(&state, 64)
            .into_iter()
            .map(|line| a3s_tui::style::strip_ansi(&line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("ended without an answer"), "{plain}");
    }

    #[test]
    fn cancelling_a_btw_panel_signals_the_background_request() {
        let token = CancellationToken::new();
        let state = BtwPanelState::start(1, "question".into(), &[], token.clone());

        assert!(!token.is_cancelled());
        state.cancel();
        assert!(token.is_cancelled());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "hits the real configured LLM over the network"]
    async fn btw_real_llm_reads_main_context_without_persisting_the_side_session() {
        use a3s_code_core::store::{MemorySessionStore, SessionStore};
        use std::time::Duration;

        let home = std::env::var("HOME").expect("HOME");
        let config = format!("{home}/.a3s/config.acl");
        assert!(
            std::path::Path::new(&config).exists(),
            "no ~/.a3s/config.acl — configure a model first"
        );

        let workspace = TestWorkspace::new("btw-real-llm");
        let store = Arc::new(MemorySessionStore::new());
        let session_store: Arc<dyn SessionStore> = store.clone();
        let options = tui_session_options(
            a3s_code_core::hitl::ConfirmationPolicy::enabled()
                .with_timeout(500, TimeoutAction::Reject),
        )
        .with_session_store(session_store)
        .with_memory(Arc::new(a3s_memory::InMemoryStore::new()))
        .with_auto_save(false)
        .with_auto_compact(true)
        .with_max_context_tokens(200_000)
        .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
        .with_goal_tracking(false)
        .with_auto_delegation_enabled(false)
        .with_auto_parallel_delegation(false)
        .with_manual_delegation_enabled(false)
        .with_max_parallel_tasks(1)
        .with_max_tool_rounds(4)
        .with_max_continuation_turns(1);
        let history = vec![
            a3s_code_core::Message::user(
                "The verification token for the next side question is A3S_BTW_REAL_OK.",
            ),
            a3s_code_core::Message::assistant(
                "I will return that exact token when the side question asks for it.",
            ),
        ];

        let result = tokio::time::timeout(
            Duration::from_secs(300),
            run_btw_request(
                Arc::new(Agent::new(config).await.expect("configured agent")),
                workspace.path().to_string_lossy().into_owned(),
                options,
                "Return only the verification token from the prior conversation. Do not use tools."
                    .to_string(),
                history,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("real /btw request timed out")
        .expect("real /btw request failed");

        assert_eq!(result.trim(), "A3S_BTW_REAL_OK");
        assert!(
            store.list().await.expect("list side sessions").is_empty(),
            "an ephemeral /btw request must not persist a session"
        );
    }
}
