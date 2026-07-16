//! Source-backed transcript entries that can be re-rendered after resize.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use a3s_tui::style::truncate_visible;

use super::design_markdown::Markdown;
use super::message_chrome::{
    message_branch, message_marker, message_title, render_notice, sanitize_message_source,
    subagent_message_tone, tool_message_tone, MessageBranch, MessageTone, NoticeKind,
};
use super::render::{
    arg_summary_for_tool, render_live_tool_activity, render_tool_terminal, render_tool_transcript,
    ToolTranscriptInput,
};
use super::runtime_projection::{SubagentOutcome, ToolCallState};
use super::tool_style::highlight_explore_detail;
#[cfg(test)]
use super::TN_CYAN;
use super::{gutter, user_bubble, wrap_words, Style, TN_FG, TN_GRAY};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TranscriptEntry {
    /// Already-rendered UI notices and non-reflowable terminal artifacts.
    Preformatted(String),
    /// Source-backed system feedback with stable severity and responsive layout.
    Notice { kind: NoticeKind, source: String },
    /// Raw user text, rendered into the transcript bubble at the current width.
    User { source: String },
    /// Raw assistant Markdown, rendered canonically at the current width.
    AssistantMarkdown { source: String },
    /// Completed model reasoning. Hidden from normal history but retained for
    /// the full semantic Ctrl+T transcript after the live thinking pane clears.
    Reasoning { source: String },
    /// Semantic tool call, retained from preparation through completion.
    Tool(ToolTranscriptEntry),
    /// Terminal delegated child result. Foreground children may be retained
    /// invisibly because their parent task tool owns the same rendered result.
    Subagent(SubagentTranscriptEntry),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolTranscriptEntry {
    call_id: Option<String>,
    name: String,
    state: ToolCallState,
    args_json: String,
    args: Option<serde_json::Value>,
    output: String,
    metadata: Option<serde_json::Value>,
    exit_code: Option<i32>,
    started_at: Option<Instant>,
    duration: Option<Duration>,
    visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SubagentTranscriptEntry {
    task_id: String,
    agent: String,
    task: String,
    outcome: SubagentOutcome,
    output: String,
    visible: bool,
}

impl ToolTranscriptEntry {
    fn args(&self) -> Option<serde_json::Value> {
        self.args
            .clone()
            .or_else(|| serde_json::from_str(&self.args_json).ok())
    }

    fn is_groupable_explore(&self) -> bool {
        self.visible
            && matches!(
                self.state,
                ToolCallState::Preparing | ToolCallState::Running | ToolCallState::Succeeded
            )
            && matches!(
                self.name.as_str(),
                "read" | "cat" | "grep" | "search" | "ls" | "glob" | "find"
            )
    }
}

#[derive(Debug)]
struct StoredTranscriptEntry {
    id: TranscriptEntryId,
    revision: u64,
    entry: TranscriptEntry,
    render_cache: Option<EntryRenderCache>,
}

/// Stable identity for a transcript entry whose presentation may evolve from
/// an in-progress notice into one terminal result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TranscriptEntryId(u64);

#[derive(Clone, Debug)]
struct EntryRenderCache {
    revision: u64,
    screen_width: u16,
    content_width: usize,
    activity_phase: Option<bool>,
    block: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TranscriptAnchor {
    entry_id: TranscriptEntryId,
    row_in_entry: usize,
}

#[derive(Clone, Copy, Debug)]
struct LayoutSpan {
    entry_id: TranscriptEntryId,
    start_row: usize,
    row_count: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Transcript {
    entries: Vec<StoredTranscriptEntry>,
    tool_positions: HashMap<String, usize>,
    latest_input_tool_id: Option<String>,
    next_id: u64,
    layout: Vec<LayoutSpan>,
}

impl TranscriptEntry {
    pub(crate) fn preformatted(value: impl Into<String>) -> Self {
        Self::Preformatted(value.into())
    }

    pub(crate) fn user(source: impl Into<String>) -> Self {
        Self::User {
            source: source.into(),
        }
    }

    pub(crate) fn notice(kind: NoticeKind, source: impl Into<String>) -> Self {
        Self::Notice {
            kind,
            source: source.into(),
        }
    }

    pub(crate) fn assistant_markdown(source: impl Into<String>) -> Self {
        Self::AssistantMarkdown {
            source: source.into(),
        }
    }

    pub(crate) fn reasoning(source: impl Into<String>) -> Self {
        Self::Reasoning {
            source: source.into(),
        }
    }

    #[cfg(test)]
    pub(crate) fn tool(
        name: impl Into<String>,
        exit_code: i32,
        output: impl Into<String>,
        metadata: Option<serde_json::Value>,
        args: Option<serde_json::Value>,
    ) -> Self {
        Self::Tool(ToolTranscriptEntry {
            call_id: None,
            name: name.into(),
            state: if exit_code == 0 {
                ToolCallState::Succeeded
            } else {
                ToolCallState::Failed
            },
            args_json: String::new(),
            args,
            output: output.into(),
            metadata,
            exit_code: Some(exit_code),
            started_at: None,
            duration: None,
            visible: true,
        })
    }

    #[cfg(test)]
    pub(crate) fn render(&self, screen_width: u16, content_width: usize) -> String {
        self.render_with_activity(screen_width, content_width, true)
    }

    fn render_with_activity(
        &self,
        _screen_width: u16,
        content_width: usize,
        activity_phase: bool,
    ) -> String {
        match self {
            Self::Preformatted(value) => value.clone(),
            Self::Notice { kind, source } => render_notice(*kind, source, content_width),
            Self::User { source } => user_bubble(&sanitize_message_source(source), content_width),
            Self::AssistantMarkdown { source } => {
                let source = sanitize_message_source(source);
                let rendered = Markdown::new()
                    .with_width(content_width.saturating_sub(2).max(1))
                    .render(&source);
                gutter(TN_GRAY, &rendered)
            }
            Self::Reasoning { .. } => String::new(),
            Self::Subagent(subagent) if !subagent.visible => String::new(),
            Self::Subagent(subagent) => render_subagent_result(subagent, content_width, false),
            Self::Tool(tool) if !tool.visible => String::new(),
            Self::Tool(tool) if tool.state.is_terminal() => render_tool_terminal(
                &tool.name,
                tool.state,
                tool.exit_code.unwrap_or(1),
                &tool.output,
                tool.metadata.as_ref(),
                tool.args().as_ref(),
                content_width,
            ),
            Self::Tool(tool) => render_live_tool_activity(
                &tool.name,
                tool.args().as_ref(),
                &tool.output,
                content_width,
                activity_phase,
                tool.state,
            ),
        }
    }

    fn render_transcript_with_activity(
        &self,
        screen_width: u16,
        content_width: usize,
        activity_phase: bool,
    ) -> String {
        match self {
            Self::Tool(tool) if !tool.visible => String::new(),
            Self::Tool(tool) => render_tool_transcript(ToolTranscriptInput {
                name: &tool.name,
                state: tool.state,
                exit_code: tool.exit_code,
                output: &tool.output,
                metadata: tool.metadata.as_ref(),
                args: tool.args().as_ref(),
                duration: tool.duration,
                width: content_width,
            }),
            Self::Subagent(subagent) if !subagent.visible => String::new(),
            Self::Subagent(subagent) => render_subagent_result(subagent, content_width, true),
            Self::Reasoning { source } => render_reasoning(source, content_width),
            _ => self.render_with_activity(screen_width, content_width, activity_phase),
        }
    }

    /// Tool and delegated-agent cells form one execution cluster until prose,
    /// user input, or a notice establishes a new narrative boundary.
    fn is_activity_cell(&self) -> bool {
        match self {
            Self::Tool(tool) => tool.visible,
            Self::Subagent(subagent) => subagent.visible,
            _ => false,
        }
    }
}

fn render_reasoning(source: &str, width: usize) -> String {
    let source = sanitize_message_source(source);
    if width == 0 || source.trim().is_empty() {
        return String::new();
    }
    let bullet = message_marker(MessageTone::Reasoning);
    let title = message_title("Reasoning", false);
    let mut rows = vec![format!("{bullet} {title}")];
    let mut first = true;
    for line in source.lines() {
        rows.extend(
            wrap_words(line, width.saturating_sub(4).max(1))
                .into_iter()
                .map(|line| {
                    let prefix = message_branch(if first {
                        MessageBranch::Last
                    } else {
                        MessageBranch::Indent
                    });
                    first = false;
                    format!(
                        "{prefix}{}",
                        Style::new().fg(TN_GRAY).italic().render(&line)
                    )
                }),
        );
    }
    rows.into_iter()
        .map(|line| truncate_visible(&line, width))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_subagent_result(
    subagent: &SubagentTranscriptEntry,
    width: usize,
    full_output: bool,
) -> String {
    if width == 0 {
        return String::new();
    }
    let status = match subagent.outcome {
        SubagentOutcome::Succeeded => "completed",
        SubagentOutcome::Failed => "failed",
        SubagentOutcome::Cancelled => "cancelled",
        SubagentOutcome::TrackingLost => "tracking lost",
    };
    let title = message_title(&format!("Agent {status}"), false);
    let bullet = message_marker(subagent_message_tone(subagent.outcome));
    let agent_name = sanitize_message_source(&subagent.agent);
    let agent = Style::new().fg(TN_GRAY).bold().render(&agent_name);
    let separator = Style::new().fg(super::TN_SUBTLE).render("·");
    let task_id = sanitize_message_source(&subagent.task_id);
    let id = full_output.then(|| {
        Style::new()
            .fg(super::TN_SUBTLE)
            .render(&format!("({task_id})"))
    });
    let header = match id {
        Some(id) => format!("{bullet} {title} {separator} {agent} {id}"),
        None => format!("{bullet} {title} {separator} {agent}"),
    };
    let mut rows = vec![truncate_visible(&header, width)];

    let task_source = sanitize_message_source(&subagent.task);
    let output_source = sanitize_message_source(&subagent.output);
    let task = task_source.trim();
    let output = output_source.trim();
    if !task.is_empty() {
        for (index, line) in wrap_words(task, width.saturating_sub(4).max(1))
            .into_iter()
            .enumerate()
        {
            let branch = match (index, output.is_empty()) {
                (0, false) => MessageBranch::Fork,
                (0, true) => MessageBranch::Last,
                (_, false) => MessageBranch::Pipe,
                (_, true) => MessageBranch::Indent,
            };
            rows.push(format!(
                "{}{}",
                message_branch(branch),
                Style::new().fg(TN_FG).render(&line)
            ));
        }
    }

    if !output.is_empty() {
        let mut output_rows = output
            .lines()
            .flat_map(|line| wrap_words(line, width.saturating_sub(4).max(1)))
            .collect::<Vec<_>>();
        let omitted = if full_output || output_rows.len() <= 8 {
            0
        } else {
            let omitted = output_rows.len() - 8;
            output_rows.truncate(8);
            omitted
        };
        rows.extend(output_rows.into_iter().enumerate().map(|(index, line)| {
            format!(
                "{}{}",
                message_branch(if index == 0 {
                    MessageBranch::Last
                } else {
                    MessageBranch::Indent
                }),
                Style::new().fg(TN_GRAY).render(&line)
            )
        }));
        if omitted > 0 {
            rows.push(truncate_visible(
                &format!(
                    "{}{}",
                    message_branch(MessageBranch::Indent),
                    Style::new()
                        .fg(TN_GRAY)
                        .render(&format!("… +{omitted} lines · Ctrl+T"))
                ),
                width,
            ));
        }
    }
    rows.join("\n")
}

impl Transcript {
    pub(crate) fn from_entries(entries: Vec<TranscriptEntry>) -> Self {
        let mut transcript = Self::default();
        transcript.extend(entries);
        transcript
    }

    pub(crate) fn into_entries(self) -> Vec<TranscriptEntry> {
        self.entries
            .into_iter()
            .map(|stored| stored.entry)
            .collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Drop provisional entries appended after an LLM attempt began. Tool
    /// positions and cached layout are rebuilt so a replacement stream can
    /// reuse the same transcript surface without leaving stale call drafts.
    pub(crate) fn truncate(&mut self, len: usize) {
        if len >= self.entries.len() {
            return;
        }
        self.entries.truncate(len);
        self.rebuild_tool_positions();
        self.latest_input_tool_id = None;
        self.layout.clear();
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.tool_positions.clear();
        self.latest_input_tool_id = None;
        self.layout.clear();
    }

    pub(crate) fn push(&mut self, entry: TranscriptEntry) {
        self.push_tracked(entry);
    }

    pub(crate) fn push_tracked(&mut self, entry: TranscriptEntry) -> TranscriptEntryId {
        let id = TranscriptEntryId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        if let TranscriptEntry::Tool(tool) = &entry {
            if let Some(call_id) = &tool.call_id {
                self.tool_positions
                    .insert(call_id.clone(), self.entries.len());
            }
        }
        self.entries.push(StoredTranscriptEntry {
            id,
            revision: 0,
            entry,
            render_cache: None,
        });
        id
    }

    /// Replace a tracked notice without assuming it is still the last entry.
    /// Streaming finalization and tool settlement may append other semantic
    /// entries while the asynchronous operation is completing.
    pub(crate) fn replace_preformatted(
        &mut self,
        id: TranscriptEntryId,
        value: impl Into<String>,
    ) -> bool {
        let Some(stored) = self.entries.iter_mut().find(|stored| stored.id == id) else {
            return false;
        };
        let TranscriptEntry::Preformatted(current) = &mut stored.entry else {
            return false;
        };
        *current = value.into();
        stored.revision = stored.revision.wrapping_add(1);
        stored.render_cache = None;
        self.layout.clear();
        true
    }

    pub(crate) fn extend(&mut self, entries: impl IntoIterator<Item = TranscriptEntry>) {
        for entry in entries {
            self.push(entry);
        }
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &TranscriptEntry> {
        self.entries.iter().map(|stored| &stored.entry)
    }

    pub(crate) fn start_tool(&mut self, call_id: String, name: String, visible: bool) {
        let index = self.ensure_tool(call_id.clone(), name, visible);
        self.mutate_tool_at(index, |tool| {
            if !tool.state.is_terminal() {
                tool.state = ToolCallState::Preparing;
                tool.started_at.get_or_insert_with(Instant::now);
            }
            tool.visible |= visible;
        });
        self.latest_input_tool_id = Some(call_id);
    }

    pub(crate) fn push_tool_input(&mut self, call_id: Option<&str>, delta: &str) -> bool {
        let call_id = call_id
            .map(str::to_string)
            .or_else(|| self.latest_input_tool_id.clone());
        let Some(index) = call_id
            .as_deref()
            .and_then(|id| self.tool_positions.get(id).copied())
        else {
            return false;
        };
        self.mutate_tool_at(index, |tool| tool.args_json.push_str(delta));
        true
    }

    pub(crate) fn await_tool_approval(
        &mut self,
        call_id: String,
        name: String,
        args: serde_json::Value,
    ) {
        let index = self.ensure_tool(call_id.clone(), name, true);
        self.mutate_tool_at(index, |tool| {
            tool.args = Some(args);
            tool.state = ToolCallState::AwaitingApproval;
            tool.started_at.get_or_insert_with(Instant::now);
            tool.visible = true;
        });
        self.latest_input_tool_id = Some(call_id);
    }

    pub(crate) fn start_tool_execution(
        &mut self,
        call_id: String,
        name: String,
        args: serde_json::Value,
        visible: bool,
    ) {
        self.start_tool_execution_inner(call_id, name, args, visible, true);
    }

    /// Restore a persisted tool call whose original wall-clock timing was not
    /// stored. Keeping `started_at` empty makes Ctrl+T omit elapsed time rather
    /// than inventing a near-zero duration during replay.
    pub(crate) fn restore_tool_execution(
        &mut self,
        call_id: String,
        name: String,
        args: serde_json::Value,
        visible: bool,
    ) {
        self.start_tool_execution_inner(call_id, name, args, visible, false);
    }

    fn start_tool_execution_inner(
        &mut self,
        call_id: String,
        name: String,
        args: serde_json::Value,
        visible: bool,
        track_duration: bool,
    ) {
        let index = self.ensure_tool(call_id.clone(), name, visible);
        self.mutate_tool_at(index, |tool| {
            tool.args = Some(args);
            tool.state = ToolCallState::Running;
            if track_duration {
                tool.started_at.get_or_insert_with(Instant::now);
            }
            tool.visible |= visible;
        });
        self.latest_input_tool_id = Some(call_id);
    }

    pub(crate) fn push_tool_output(
        &mut self,
        call_id: &str,
        name: String,
        delta: &str,
        visible: bool,
    ) {
        let index = self.ensure_tool(call_id.to_string(), name, visible);
        self.mutate_tool_at(index, |tool| {
            tool.output.push_str(delta);
            tool.visible |= visible;
            if !tool.state.is_terminal() {
                tool.state = ToolCallState::Running;
                tool.started_at.get_or_insert_with(Instant::now);
            }
        });
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish_tool(
        &mut self,
        call_id: &str,
        name: String,
        args: Option<serde_json::Value>,
        output: String,
        exit_code: i32,
        metadata: Option<serde_json::Value>,
        visible: bool,
    ) -> Option<serde_json::Value> {
        let state = if exit_code == 0 {
            ToolCallState::Succeeded
        } else {
            ToolCallState::Failed
        };
        self.finish_tool_with_state(
            call_id, name, args, output, exit_code, metadata, state, visible,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish_tool_with_state(
        &mut self,
        call_id: &str,
        name: String,
        args: Option<serde_json::Value>,
        output: String,
        exit_code: i32,
        metadata: Option<serde_json::Value>,
        state: ToolCallState,
        visible: bool,
    ) -> Option<serde_json::Value> {
        let index = self.ensure_tool(call_id.to_string(), name, visible);
        self.mutate_tool_at(index, |tool| {
            if args.is_some() {
                tool.args = args;
            }
            tool.metadata = metadata;
            let protected_state = matches!(
                tool.state,
                ToolCallState::Denied | ToolCallState::TimedOut | ToolCallState::Interrupted
            );
            if !protected_state {
                tool.output = output;
                tool.exit_code = Some(exit_code);
                tool.state = state;
                if tool.duration.is_none() {
                    tool.duration = tool.started_at.map(|started| started.elapsed());
                }
            } else if tool.output.trim().is_empty() {
                tool.output = output;
            }
            tool.visible |= visible;
        });
        if self.latest_input_tool_id.as_deref() == Some(call_id) {
            self.latest_input_tool_id = None;
        }
        self.tool_at(index).and_then(ToolTranscriptEntry::args)
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn finish_subagent(
        &mut self,
        task_id: String,
        agent: String,
        task: String,
        success: bool,
        output: String,
        visible: bool,
    ) {
        let outcome = if success {
            SubagentOutcome::Succeeded
        } else {
            SubagentOutcome::Failed
        };
        self.finish_subagent_with_outcome(task_id, agent, task, outcome, output, visible);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish_subagent_with_outcome(
        &mut self,
        task_id: String,
        agent: String,
        task: String,
        outcome: SubagentOutcome,
        output: String,
        visible: bool,
    ) {
        if let Some(index) = self.entries.iter().position(|stored| {
            matches!(
                &stored.entry,
                TranscriptEntry::Subagent(subagent) if subagent.task_id == task_id
            )
        }) {
            let Some(stored) = self.entries.get_mut(index) else {
                return;
            };
            let TranscriptEntry::Subagent(subagent) = &mut stored.entry else {
                return;
            };
            subagent.agent = agent;
            if !task.trim().is_empty() {
                subagent.task = task;
            }
            // Cancellation is authoritative when a late generic failure for
            // the same task arrives from another completion channel. Preserve
            // its explanatory output as well, otherwise the card would say
            // "cancelled" while displaying a contradictory failure message.
            let preserve_cancelled = subagent.outcome == SubagentOutcome::Cancelled
                && outcome != SubagentOutcome::Cancelled;
            if !preserve_cancelled {
                subagent.outcome = outcome;
                subagent.output = output;
            } else if subagent.output.trim().is_empty() {
                subagent.output = output;
            }
            subagent.visible |= visible;
            stored.revision = stored.revision.wrapping_add(1);
            stored.render_cache = None;
            return;
        }
        self.push(TranscriptEntry::Subagent(SubagentTranscriptEntry {
            task_id,
            agent,
            task,
            outcome,
            output,
            visible,
        }));
    }

    pub(crate) fn discard_tool(&mut self, call_id: &str) -> bool {
        let Some(index) = self.tool_positions.remove(call_id) else {
            return false;
        };
        self.entries.remove(index);
        self.rebuild_tool_positions();
        if self.latest_input_tool_id.as_deref() == Some(call_id) {
            self.latest_input_tool_id = None;
        }
        true
    }

    pub(crate) fn interrupt_unfinished_tools(&mut self) {
        let indices = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, stored)| match &stored.entry {
                TranscriptEntry::Tool(tool) if !tool.state.is_terminal() => Some(index),
                _ => None,
            })
            .collect::<Vec<_>>();
        for index in indices {
            self.mutate_tool_at(index, |tool| {
                tool.state = ToolCallState::Interrupted;
                tool.exit_code = Some(130);
                if tool.duration.is_none() {
                    tool.duration = tool.started_at.map(|started| started.elapsed());
                }
                tool.visible = true;
                if tool.output.trim().is_empty() {
                    tool.output = "Interrupted before the tool call completed.".to_string();
                }
            });
        }
        self.latest_input_tool_id = None;
    }

    #[cfg(test)]
    pub(crate) fn render(&mut self, screen_width: u16, content_width: usize) -> Vec<String> {
        self.render_with_activity(screen_width, content_width, true)
    }

    pub(crate) fn render_with_activity(
        &mut self,
        screen_width: u16,
        content_width: usize,
        activity_phase: bool,
    ) -> Vec<String> {
        let mut blocks = Vec::new();
        let mut layout = Vec::new();
        let mut next_block_row = 0usize;
        let mut activity_cluster = String::new();
        let mut index = 0usize;
        while index < self.entries.len() {
            if self.is_groupable_explore(index) {
                let start = index;
                index += 1;
                while index < self.entries.len() && self.is_groupable_explore(index) {
                    index += 1;
                }
                let tools = self.entries[start..index]
                    .iter()
                    .filter_map(|stored| match &stored.entry {
                        TranscriptEntry::Tool(tool) => Some(tool),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let block = render_explore_group(&tools, content_width, activity_phase);
                if !block.is_empty() {
                    let row_count = block.lines().count();
                    append_activity_cell(&mut activity_cluster, &block);
                    for stored in &self.entries[start..index] {
                        layout.push(LayoutSpan {
                            entry_id: stored.id,
                            start_row: next_block_row,
                            row_count,
                        });
                    }
                    next_block_row += row_count;
                }
                continue;
            }

            let is_activity_cell = self.entries[index].entry.is_activity_cell();
            let block = self.render_entry(index, screen_width, content_width, activity_phase);
            if !block.is_empty() {
                let row_count = block.lines().count();
                if is_activity_cell {
                    append_activity_cell(&mut activity_cluster, &block);
                    layout.push(LayoutSpan {
                        entry_id: self.entries[index].id,
                        start_row: next_block_row,
                        row_count,
                    });
                    next_block_row += row_count;
                } else {
                    flush_activity_cluster(&mut blocks, &mut activity_cluster);
                    layout.push(LayoutSpan {
                        entry_id: self.entries[index].id,
                        start_row: next_block_row,
                        row_count,
                    });
                    next_block_row += row_count;
                    blocks.push(block);
                }
            }
            index += 1;
        }
        flush_activity_cluster(&mut blocks, &mut activity_cluster);
        self.layout = layout;
        blocks
    }

    /// Render every semantic entry for Ctrl+T without the compact-history
    /// grouping or output bounds. This deliberately does not mutate the main
    /// viewport layout/anchor cache.
    pub(crate) fn render_transcript_with_activity(
        &self,
        screen_width: u16,
        content_width: usize,
        activity_phase: bool,
    ) -> Vec<String> {
        self.entries
            .iter()
            .map(|stored| {
                stored.entry.render_transcript_with_activity(
                    screen_width,
                    content_width,
                    activity_phase,
                )
            })
            .filter(|block| !block.is_empty())
            .collect()
    }

    pub(crate) fn anchor_for_row(&self, row: usize) -> Option<TranscriptAnchor> {
        let span = self
            .layout
            .iter()
            .find(|span| row >= span.start_row && row < span.start_row + span.row_count)
            .or_else(|| self.layout.iter().rev().find(|span| span.start_row <= row))?;
        Some(TranscriptAnchor {
            entry_id: span.entry_id,
            row_in_entry: row
                .saturating_sub(span.start_row)
                .min(span.row_count.saturating_sub(1)),
        })
    }

    pub(crate) fn row_for_anchor(&self, anchor: TranscriptAnchor) -> Option<usize> {
        let span = self
            .layout
            .iter()
            .find(|span| span.entry_id == anchor.entry_id)?;
        Some(span.start_row + anchor.row_in_entry.min(span.row_count.saturating_sub(1)))
    }

    fn ensure_tool(&mut self, call_id: String, name: String, visible: bool) -> usize {
        if let Some(index) = self.tool_positions.get(&call_id).copied() {
            return index;
        }
        let index = self.entries.len();
        self.push(TranscriptEntry::Tool(ToolTranscriptEntry {
            call_id: Some(call_id.clone()),
            name,
            state: ToolCallState::Preparing,
            args_json: String::new(),
            args: None,
            output: String::new(),
            metadata: None,
            exit_code: None,
            started_at: None,
            duration: None,
            visible,
        }));
        self.tool_positions.insert(call_id, index);
        index
    }

    fn mutate_tool_at(&mut self, index: usize, mutate: impl FnOnce(&mut ToolTranscriptEntry)) {
        let Some(stored) = self.entries.get_mut(index) else {
            return;
        };
        let TranscriptEntry::Tool(tool) = &mut stored.entry else {
            return;
        };
        mutate(tool);
        stored.revision = stored.revision.wrapping_add(1);
        stored.render_cache = None;
    }

    fn tool_at(&self, index: usize) -> Option<&ToolTranscriptEntry> {
        match &self.entries.get(index)?.entry {
            TranscriptEntry::Tool(tool) => Some(tool),
            _ => None,
        }
    }

    fn is_groupable_explore(&self, index: usize) -> bool {
        self.tool_at(index)
            .is_some_and(ToolTranscriptEntry::is_groupable_explore)
    }

    fn render_entry(
        &mut self,
        index: usize,
        screen_width: u16,
        content_width: usize,
        activity_phase: bool,
    ) -> String {
        let stored = &mut self.entries[index];
        let cache_phase = match &stored.entry {
            TranscriptEntry::Tool(tool) if !tool.state.is_terminal() => Some(activity_phase),
            _ => None,
        };
        if let Some(cache) = &stored.render_cache {
            if cache.revision == stored.revision
                && cache.screen_width == screen_width
                && cache.content_width == content_width
                && cache.activity_phase == cache_phase
            {
                return cache.block.clone();
            }
        }
        let block = stored
            .entry
            .render_with_activity(screen_width, content_width, activity_phase);
        stored.render_cache = Some(EntryRenderCache {
            revision: stored.revision,
            screen_width,
            content_width,
            activity_phase: cache_phase,
            block: block.clone(),
        });
        block
    }

    fn rebuild_tool_positions(&mut self) {
        self.tool_positions.clear();
        for (index, stored) in self.entries.iter().enumerate() {
            if let TranscriptEntry::Tool(tool) = &stored.entry {
                if let Some(call_id) = &tool.call_id {
                    self.tool_positions.insert(call_id.clone(), index);
                }
            }
        }
    }
}

fn append_activity_cell(cluster: &mut String, cell: &str) {
    if !cluster.is_empty() {
        cluster.push('\n');
    }
    cluster.push_str(cell);
}

fn flush_activity_cluster(blocks: &mut Vec<String>, cluster: &mut String) {
    if cluster.is_empty() {
        return;
    }
    blocks.push(std::mem::take(cluster));
}

/// Codex history cells own their internal spacing. Stacking them with one row
/// boundary keeps tool runs cohesive and lets the user-authored surface provide
/// its own top/bottom breathing room without an extra global blank row.
pub(crate) fn join_transcript_blocks(blocks: &[String]) -> String {
    blocks.join("\n")
}

#[derive(Debug)]
enum ExploreAction {
    Read(Vec<String>),
    Other(String),
}

fn render_explore_group(
    tools: &[&ToolTranscriptEntry],
    width: usize,
    activity_phase: bool,
) -> String {
    if tools.is_empty() || width == 0 {
        return String::new();
    }
    let mut actions = Vec::<ExploreAction>::new();
    for tool in tools {
        let args = tool.args().unwrap_or(serde_json::Value::Null);
        match tool.name.as_str() {
            "read" | "cat" => {
                let path = args
                    .get("file_path")
                    .or_else(|| args.get("path"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("file")
                    .to_string();
                match actions.last_mut() {
                    Some(ExploreAction::Read(paths)) => paths.push(path),
                    _ => actions.push(ExploreAction::Read(vec![path])),
                }
            }
            "grep" | "search" => {
                let query = args
                    .get("pattern")
                    .or_else(|| args.get("query"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pattern");
                let path = args
                    .get("path")
                    .or_else(|| args.get("file_path"))
                    .and_then(serde_json::Value::as_str);
                actions.push(ExploreAction::Other(match path {
                    Some(path) => format!("Search {query} in {path}"),
                    None => format!("Search {query}"),
                }));
            }
            "ls" | "glob" | "find" => {
                let detail = arg_summary_for_tool(&tool.name, &args).unwrap_or_default();
                actions.push(ExploreAction::Other(if detail.is_empty() {
                    "List files".to_string()
                } else {
                    format!("List {detail}")
                }));
            }
            _ => {}
        }
    }

    let live = tools.iter().any(|tool| !tool.state.is_terminal());
    let tone = if live {
        tool_message_tone(ToolCallState::Running, activity_phase)
    } else {
        MessageTone::Inactive
    };
    let bullet = message_marker(tone);
    let title = message_title(if live { "Exploring" } else { "Explored" }, false);
    let mut rows = vec![format!("{bullet} {title}")];
    for (action_index, action) in actions.into_iter().enumerate() {
        let text = match action {
            ExploreAction::Read(paths) => format!("Read {}", paths.join(", ")),
            ExploreAction::Other(text) => text,
        };
        let styled = highlight_explore_detail(&text);
        let wrapped = wrap_words(&styled, width.saturating_sub(4).max(1));
        for (line_index, line) in wrapped.into_iter().enumerate() {
            let prefix = message_branch(if action_index == 0 && line_index == 0 {
                MessageBranch::Last
            } else {
                MessageBranch::Indent
            });
            rows.push(format!("{prefix}{line}"));
        }
    }
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::tool_style::{TOOL_ARGUMENT_COLOR, TOOL_PATH_COLOR};
    use super::super::ACCENT;
    use super::*;

    fn assert_bounded(rendered: &str, width: usize) {
        for line in rendered.lines() {
            assert!(
                a3s_tui::style::visible_len(line) <= width,
                "line exceeds width {width}: {:?}",
                a3s_tui::style::strip_ansi(line)
            );
        }
    }

    #[test]
    fn assistant_markdown_is_source_backed_across_resize() {
        let entry = TranscriptEntry::assistant_markdown(
            "A paragraph with **formatting** and enough words to wrap at narrow widths.",
        );
        let narrow = entry.render(28, 27);
        let wide = entry.render(80, 79);
        assert!(narrow.lines().count() > wide.lines().count());
        assert_bounded(&narrow, 27);
        assert_bounded(&wide, 79);
    }

    #[test]
    fn user_message_owns_codex_surface_spacing_at_product_widths() {
        for width in [24_u16, 48, 80] {
            let mut transcript = Transcript::from_entries(vec![
                TranscriptEntry::user("Review the message hierarchy."),
                TranscriptEntry::assistant_markdown("The hierarchy is now calmer."),
            ]);
            let blocks = transcript.render(width, width as usize);
            assert_eq!(blocks.len(), 2);
            let joined = join_transcript_blocks(&blocks);
            let rendered = a3s_tui::style::strip_ansi(&joined);
            let rows = rendered.lines().collect::<Vec<_>>();
            let assistant_row = rows
                .iter()
                .position(|row| row.contains("The hierarchy"))
                .expect("assistant row");
            assert!(
                rendered
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .contains("The hierarchy is now calmer."),
                "width {width}: {rendered:?}"
            );

            assert!(rows.first().is_some_and(|row| row.trim().is_empty()));
            assert!(rows.iter().any(|row| row.starts_with("› Review")));
            assert!(rows[assistant_row - 1].trim().is_empty());
            assert_eq!(
                rows.iter().filter(|row| row.trim().is_empty()).count(),
                2,
                "width {width}: {rendered:?}"
            );
            assert!(joined.lines().take(1).all(
                |row| row.contains(&format!("\x1b[{}m", super::super::SURFACE_USER.bg_ansi()))
            ));
            assert!(!rendered.contains("\n\n\n"), "width {width}: {rendered:?}");
            assert_bounded(&blocks[0], width as usize);
            assert_bounded(&blocks[1], width as usize);
        }
    }

    #[test]
    fn truncate_removes_provisional_stream_attempt_entries_and_tool_indexes() {
        let mut transcript = Transcript::from_entries(vec![TranscriptEntry::user(
            "Keep this user message exactly once.",
        )]);
        let checkpoint = transcript.len();
        transcript.push(TranscriptEntry::assistant_markdown(
            "discarded partial answer",
        ));
        transcript.start_tool("partial-tool".into(), "bash".into(), true);
        assert!(transcript.push_tool_input(Some("partial-tool"), r#"{"command":"car"#));

        transcript.truncate(checkpoint);

        assert_eq!(transcript.len(), 1);
        assert!(matches!(
            transcript.iter().next(),
            Some(TranscriptEntry::User { source }) if source == "Keep this user message exactly once."
        ));
        assert!(!transcript.push_tool_input(Some("partial-tool"), "go"));
    }

    #[test]
    fn completed_reasoning_is_hidden_from_history_but_retained_for_ctrl_t() {
        let mut transcript = Transcript::from_entries(vec![TranscriptEntry::reasoning(
            "Inspect the event ordering, then preserve the semantic boundary.",
        )]);

        assert!(transcript.render(80, 79).is_empty());
        let complete = transcript.render_transcript_with_activity(80, 79, true);
        assert_eq!(complete.len(), 1);
        let plain = a3s_tui::style::strip_ansi(&complete[0]);
        assert!(plain.contains("• Reasoning"), "{plain}");
        assert!(plain.contains("  └ Inspect the event ordering"), "{plain}");
        assert!(plain.contains("Inspect the event ordering"), "{plain}");
        assert!(complete[0].contains(&message_marker(MessageTone::Reasoning)));
        assert_bounded(&complete[0], 79);
    }

    #[test]
    fn background_subagent_result_is_durable_and_ctrl_t_keeps_full_output() {
        let mut transcript = Transcript::default();
        let output = (0..12)
            .map(|index| format!("result line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        transcript.finish_subagent(
            "task-bg".into(),
            "review".into(),
            "audit the implementation".into(),
            true,
            output,
            true,
        );

        let compact_rendered = transcript.render(80, 79).join("\n");
        let compact = a3s_tui::style::strip_ansi(&compact_rendered);
        assert!(compact.contains("Agent completed · review"), "{compact}");
        assert!(!compact.contains("(task-bg)"), "{compact}");
        assert!(
            compact_rendered.contains(&message_marker(MessageTone::Success)),
            "{compact_rendered:?}"
        );
        assert!(
            compact_rendered.contains(&message_title("Agent completed", false)),
            "{compact_rendered:?}"
        );
        assert!(
            compact_rendered.contains(&Style::new().fg(TN_GRAY).bold().render("review")),
            "{compact_rendered:?}"
        );
        assert!(
            !compact_rendered.contains(&Style::new().fg(TN_CYAN).bold().render("review")),
            "agent identity should not compete with the outcome marker: {compact_rendered:?}"
        );
        assert!(compact.contains("audit the implementation"), "{compact}");
        assert!(
            compact.contains("  ├ audit the implementation"),
            "{compact}"
        );
        assert!(compact.contains("  └ result line 0"), "{compact}");
        assert!(!compact.contains("Output:"), "{compact}");
        assert!(compact.contains("result line 0"), "{compact}");
        assert!(compact.contains("… +4 lines"), "{compact}");
        assert!(!compact.contains("result line 11"), "{compact}");

        let complete_rendered = transcript
            .render_transcript_with_activity(80, 79, true)
            .join("\n");
        let complete = a3s_tui::style::strip_ansi(&complete_rendered);
        assert!(complete.contains("(task-bg)"), "{complete}");
        assert!(complete.contains("result line 11"), "{complete}");
        assert!(!complete.contains("… +"), "{complete}");
    }

    #[test]
    fn semantic_messages_strip_untrusted_terminal_controls_before_styling() {
        let mut transcript = Transcript::from_entries(vec![
            TranscriptEntry::user("\x1b[2Juser\0 message"),
            TranscriptEntry::assistant_markdown("\x1b]0;title\x07assistant **message**"),
            TranscriptEntry::reasoning("\x1b[31mreasoning\x1b[0m\0 message"),
        ]);
        transcript.finish_subagent(
            "\x1b[2Jtask-id".into(),
            "\x1b[31mreview\x1b[0m".into(),
            "audit\0 task".into(),
            true,
            "safe\x1b]0;title\x07 output".into(),
            true,
        );

        let compact = transcript.render(80, 79).join("\n");
        let complete = transcript
            .render_transcript_with_activity(80, 79, true)
            .join("\n");
        for rendered in [&compact, &complete] {
            assert!(!rendered.contains("\x1b[2J"), "{rendered:?}");
            assert!(!rendered.contains("\x1b]0;title"), "{rendered:?}");
            assert!(!rendered.contains('\0'), "{rendered:?}");
        }
        let compact_plain = a3s_tui::style::strip_ansi(&compact);
        let complete_plain = a3s_tui::style::strip_ansi(&complete);
        assert!(compact_plain.contains("user message"), "{compact_plain}");
        assert!(
            compact_plain.contains("assistant message"),
            "{compact_plain}"
        );
        assert!(compact_plain.contains("audit task"), "{compact_plain}");
        assert!(
            complete_plain.contains("reasoning message"),
            "{complete_plain}"
        );
        assert!(complete_plain.contains("(task-id)"), "{complete_plain}");
    }

    #[test]
    fn foreground_subagent_semantic_cell_stays_hidden_behind_parent_result() {
        let mut transcript = Transcript::default();
        transcript.finish_subagent(
            "task-fg".into(),
            "explore".into(),
            "inspect".into(),
            true,
            "same output as parent task".into(),
            false,
        );

        assert_eq!(transcript.iter().count(), 1);
        assert!(transcript.render(80, 79).is_empty());
        assert!(transcript
            .render_transcript_with_activity(80, 79, true)
            .is_empty());
    }

    #[test]
    fn duplicate_subagent_terminal_delivery_updates_one_durable_cell() {
        let mut transcript = Transcript::default();
        for output in ["event output", "tracker output"] {
            transcript.finish_subagent(
                "task-bg".into(),
                "review".into(),
                "audit".into(),
                true,
                output.into(),
                true,
            );
        }

        assert_eq!(transcript.iter().count(), 1);
        let plain = a3s_tui::style::strip_ansi(&transcript.render(80, 79).join("\n"));
        assert!(plain.contains("tracker output"), "{plain}");
        assert!(!plain.contains("event output"), "{plain}");
    }

    #[test]
    fn cancelled_subagent_resists_a_late_generic_failure() {
        let mut transcript = Transcript::default();
        transcript.finish_subagent_with_outcome(
            "task-bg".into(),
            "review".into(),
            "audit".into(),
            SubagentOutcome::Cancelled,
            "Stopped by user.".into(),
            true,
        );
        transcript.finish_subagent_with_outcome(
            "task-bg".into(),
            "review".into(),
            "audit".into(),
            SubagentOutcome::Failed,
            "Late watcher failure.".into(),
            true,
        );

        let plain = a3s_tui::style::strip_ansi(&transcript.render(80, 79).join("\n"));
        assert!(plain.contains("Agent cancelled · review"), "{plain}");
        assert!(plain.contains("Stopped by user."), "{plain}");
        assert!(!plain.contains("Agent failed"), "{plain}");
        assert!(!plain.contains("Late watcher failure."), "{plain}");
    }

    #[test]
    fn tool_entry_reflows_from_semantic_fields() {
        let entry = TranscriptEntry::tool(
            "bash",
            0,
            "first output line\nsecond output line",
            None,
            Some(serde_json::json!({
                "command": "cargo test a-very-long-filter-name -- --nocapture"
            })),
        );
        assert_bounded(&entry.render(36, 35), 35);
        assert_bounded(&entry.render(80, 79), 79);
    }

    #[test]
    fn tool_completion_updates_start_position_not_completion_order() {
        let mut transcript = Transcript::default();
        transcript.push(TranscriptEntry::assistant_markdown("before"));
        transcript.start_tool("t1".into(), "bash".into(), true);
        transcript.start_tool("t2".into(), "grep".into(), true);
        transcript.finish_tool(
            "t2",
            "grep".into(),
            Some(serde_json::json!({"pattern": "TODO"})),
            "match".into(),
            0,
            None,
            true,
        );
        transcript.finish_tool(
            "t1",
            "bash".into(),
            Some(serde_json::json!({"command": "echo ok"})),
            "ok".into(),
            0,
            None,
            true,
        );
        transcript.push(TranscriptEntry::assistant_markdown("after"));

        let kinds = transcript
            .iter()
            .map(|entry| match entry {
                TranscriptEntry::AssistantMarkdown { source } => source.clone(),
                TranscriptEntry::Tool(tool) => tool.call_id.clone().unwrap(),
                _ => "other".to_string(),
            })
            .collect::<Vec<_>>();
        assert_eq!(kinds, ["before", "t1", "t2", "after"]);
    }

    #[test]
    fn authoritative_end_args_render_without_streamed_input() {
        let mut transcript = Transcript::default();
        transcript.start_tool("t1".into(), "bash".into(), true);
        transcript.finish_tool(
            "t1",
            "bash".into(),
            Some(serde_json::json!({"command": "cargo test"})),
            "ok".into(),
            0,
            None,
            true,
        );
        let plain = a3s_tui::style::strip_ansi(&transcript.render(80, 79).join("\n"));
        assert!(plain.contains("cargo test"), "{plain}");
    }

    #[test]
    fn adjacent_explore_calls_group_without_erasing_meaningful_rereads() {
        let mut transcript = Transcript::default();
        for (id, name, args) in [
            ("r1", "read", serde_json::json!({"file_path": "auth.rs"})),
            ("r2", "read", serde_json::json!({"file_path": "auth.rs"})),
            (
                "g1",
                "grep",
                serde_json::json!({"pattern": "TODO", "path": "src"}),
            ),
        ] {
            transcript.start_tool(id.into(), name.into(), true);
            transcript.finish_tool(id, name.into(), Some(args), String::new(), 0, None, true);
        }
        let blocks = transcript.render(80, 79);
        assert_eq!(blocks.len(), 1);
        let rendered = &blocks[0];
        let plain = a3s_tui::style::strip_ansi(rendered);
        assert_eq!(
            plain.lines().collect::<Vec<_>>(),
            [
                "• Explored",
                "  └ Read auth.rs, auth.rs",
                "    Search TODO in src"
            ],
            "{plain}"
        );
        assert_eq!(plain.matches("auth.rs").count(), 2, "{plain}");
        assert!(
            rendered.contains(&Style::new().fg(TN_CYAN).render("Read")),
            "{rendered:?}"
        );
        assert!(
            rendered.contains(&Style::new().fg(TOOL_ARGUMENT_COLOR).render("TODO")),
            "{rendered:?}"
        );
        assert!(
            rendered.contains(&Style::new().fg(TOOL_PATH_COLOR).render("src")),
            "{rendered:?}"
        );
    }

    #[test]
    fn adjacent_explore_calls_are_grouped_while_live_then_finish_in_place() {
        let mut transcript = Transcript::default();
        transcript.start_tool_execution(
            "read-live".into(),
            "read".into(),
            serde_json::json!({"file_path":"src/lib.rs"}),
            true,
        );
        transcript.start_tool_execution(
            "grep-live".into(),
            "grep".into(),
            serde_json::json!({"pattern":"TODO", "path":"src"}),
            true,
        );

        let live = transcript.render_with_activity(80, 79, true);
        assert_eq!(live.len(), 1);
        let live = a3s_tui::style::strip_ansi(&live[0]);
        assert!(live.starts_with("• Exploring\n"), "{live}");
        assert!(live.contains("Read src/lib.rs"), "{live}");
        assert!(live.contains("Search TODO in src"), "{live}");

        for (id, name) in [("read-live", "read"), ("grep-live", "grep")] {
            transcript.finish_tool(id, name.into(), None, String::new(), 0, None, true);
        }
        let completed = transcript.render_with_activity(80, 79, true);
        assert_eq!(completed.len(), 1);
        let completed = a3s_tui::style::strip_ansi(&completed[0]);
        assert!(completed.starts_with("• Explored\n"), "{completed}");
    }

    #[test]
    fn consecutive_tool_cells_share_one_dense_activity_cluster() {
        let mut transcript = Transcript::from_entries(vec![TranscriptEntry::assistant_markdown(
            "I will verify both layers.",
        )]);
        for (id, command, output) in [
            ("shell-1", "cargo check", "check passed"),
            ("shell-2", "cargo test focused", "test passed"),
        ] {
            let args = serde_json::json!({"command": command});
            transcript.start_tool_execution(id.into(), "bash".into(), args.clone(), true);
            transcript.finish_tool(id, "bash".into(), Some(args), output.into(), 0, None, true);
        }
        transcript.push(TranscriptEntry::assistant_markdown(
            "Both verification layers passed.",
        ));

        let blocks = transcript.render(80, 79);

        assert_eq!(blocks.len(), 3, "activity cells should share one block");
        let activity = a3s_tui::style::strip_ansi(&blocks[1]);
        assert!(activity.contains("cargo check"), "{activity}");
        assert!(activity.contains("cargo test focused"), "{activity}");
        assert!(!activity.contains("\n\n"), "{activity}");

        let tool_spans = transcript
            .entries
            .iter()
            .filter_map(|stored| match &stored.entry {
                TranscriptEntry::Tool(_) => transcript
                    .layout
                    .iter()
                    .find(|span| span.entry_id == stored.id)
                    .copied(),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(tool_spans.len(), 2);
        assert_eq!(
            tool_spans[1].start_row,
            tool_spans[0].start_row + tool_spans[0].row_count,
            "adjacent activity cells must not reserve a blank layout row"
        );

        let flow = a3s_tui::style::strip_ansi(&join_transcript_blocks(&blocks));
        assert!(
            flow.contains("both layers.\n• Ran")
                && flow.contains("test passed\n• Both verification"),
            "Codex-style history cells should stack without global blank rows: {flow}"
        );
    }

    #[test]
    fn hidden_tool_placeholder_can_be_discarded_without_reordering_neighbors() {
        let mut transcript = Transcript::from_entries(vec![TranscriptEntry::user("before")]);
        transcript.start_tool("hidden".into(), "write".into(), false);
        transcript.push(TranscriptEntry::assistant_markdown("after"));
        assert!(transcript.discard_tool("hidden"));
        assert_eq!(transcript.iter().count(), 2);
    }

    #[test]
    fn duplicate_terminal_delivery_updates_one_tool_entry() {
        let mut transcript = Transcript::default();
        transcript.start_tool_execution(
            "host-1".into(),
            "dynamic_workflow".into(),
            serde_json::json!({"run_id": "run-1"}),
            true,
        );
        for output in ["raw", "sanitized"] {
            transcript.finish_tool(
                "host-1",
                "dynamic_workflow".into(),
                Some(serde_json::json!({"run_id": "run-1"})),
                output.to_string(),
                0,
                None,
                true,
            );
        }

        assert_eq!(transcript.iter().count(), 1);
        let TranscriptEntry::Tool(tool) = transcript.iter().next().unwrap() else {
            panic!("expected tool entry");
        };
        assert_eq!(tool.output, "sanitized");
    }

    #[test]
    fn duplicate_tool_end_preserves_denied_terminal_state() {
        let mut transcript = Transcript::default();
        transcript.await_tool_approval(
            "denied-1".into(),
            "bash".into(),
            serde_json::json!({"command": "dangerous"}),
        );
        transcript.finish_tool_with_state(
            "denied-1",
            "bash".into(),
            None,
            "Denied by user".into(),
            1,
            None,
            ToolCallState::Denied,
            true,
        );
        transcript.finish_tool(
            "denied-1",
            "bash".into(),
            Some(serde_json::json!({"command": "dangerous"})),
            "tool execution denied".into(),
            1,
            None,
            true,
        );

        let TranscriptEntry::Tool(tool) = transcript.iter().next().unwrap() else {
            panic!("expected tool entry");
        };
        assert_eq!(tool.state, ToolCallState::Denied);
        assert_eq!(tool.output, "Denied by user");
        let plain = a3s_tui::style::strip_ansi(&transcript.render(80, 79).join("\n"));
        assert!(plain.contains("Denied dangerous"), "{plain}");
        assert!(!plain.contains("Ran dangerous"), "{plain}");
    }

    #[test]
    fn preformatted_entries_are_preserved_verbatim() {
        let value = format!("{}notice", ACCENT.fg_ansi());
        assert_eq!(TranscriptEntry::preformatted(&value).render(40, 39), value);
    }

    #[test]
    fn semantic_notices_reflow_and_keep_severity_across_widths() {
        let entry = TranscriptEntry::notice(
            NoticeKind::Error,
            "无法连接 provider because the configured endpoint did not respond",
        );
        let narrow = entry.render(28, 27);
        let wide = entry.render(80, 79);

        assert!(narrow.lines().count() > wide.lines().count());
        assert!(narrow.contains(&message_marker(MessageTone::Error)));
        assert!(wide.contains(&message_marker(MessageTone::Error)));
        assert_bounded(&narrow, 27);
        assert_bounded(&wide, 79);
    }

    #[test]
    fn mixed_message_gallery_preserves_hierarchy_at_product_widths() {
        let mut transcript = Transcript::from_entries(vec![
            TranscriptEntry::notice(NoticeKind::Info, "Context auto-compacted at 85%"),
            TranscriptEntry::user("请检查 src/tui/ui/render.rs and preserve the visual hierarchy"),
            TranscriptEntry::assistant_markdown(
                "I’ll inspect the rendering path, then verify `cargo test`.",
            ),
            TranscriptEntry::reasoning(
                "Compare semantic state, message density, and responsive wrapping.",
            ),
        ]);
        transcript.start_tool_execution(
            "read-live".into(),
            "read".into(),
            serde_json::json!({"file_path": "src/tui/ui/render.rs"}),
            true,
        );
        transcript.await_tool_approval(
            "write-awaiting".into(),
            "write".into(),
            serde_json::json!({
                "file_path": "src/tui/ui/message_chrome.rs",
                "content": "semantic message chrome"
            }),
        );
        transcript.finish_tool(
            "exec-done",
            "bash".into(),
            Some(serde_json::json!({"command": "cargo test --bin a3s"})),
            "1234 tests passed".into(),
            0,
            None,
            true,
        );
        let diff_before = (0..60)
            .map(|index| format!("old-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let diff_after = (0..60)
            .map(|index| format!("new-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        transcript.finish_tool(
            "edit-large",
            "edit".into(),
            Some(serde_json::json!({"file_path": "src/large.rs"})),
            "Updated src/large.rs".into(),
            0,
            Some(serde_json::json!({
                "file_path": "src/large.rs",
                "before": diff_before,
                "after": diff_after
            })),
            true,
        );
        transcript.finish_tool(
            "lookup-failed",
            "custom_lookup".into(),
            Some(serde_json::json!({"path": "./fixtures/研究.json", "count": 2})),
            "provider did not respond".into(),
            1,
            None,
            true,
        );
        transcript.finish_tool(
            "batch-partial",
            "batch".into(),
            Some(serde_json::json!({
                "invocations": [
                    {"tool": "read", "args": {"file_path": "README.md"}},
                    {"tool": "bash", "args": {"command": "cargo test"}}
                ]
            })),
            "--- [1: read] ---\ncontents\n--- [2: bash] ---\nERROR: failed".into(),
            0,
            Some(serde_json::json!({
                "execution_mode": "parallel",
                "applied_concurrency": 2,
                "success_count": 1,
                "failure_count": 1,
                "results": [
                    {"index": 0, "tool": "read", "success": true, "exit_code": 0},
                    {"index": 1, "tool": "bash", "success": false, "exit_code": 101}
                ]
            })),
            true,
        );
        transcript.finish_tool(
            "mcp-json",
            "mcp__docs__find".into(),
            Some(serde_json::json!({"query": "terminal UX"})),
            serde_json::json!({
                "documents": [
                    {"title": "Message hierarchy", "score": 0.98},
                    {"title": "Streaming stability", "score": 0.91}
                ]
            })
            .to_string(),
            0,
            None,
            true,
        );
        transcript.finish_tool_with_state(
            "exec-denied",
            "bash".into(),
            Some(serde_json::json!({"command": "rm -rf protected"})),
            "Denied by user policy.".into(),
            1,
            None,
            ToolCallState::Denied,
            true,
        );
        transcript.finish_tool_with_state(
            "fetch-timeout",
            "web_fetch".into(),
            Some(serde_json::json!({"url": "https://example.com/slow"})),
            "Request exceeded the tool deadline.".into(),
            124,
            None,
            ToolCallState::TimedOut,
            true,
        );
        transcript.finish_tool_with_state(
            "lookup-interrupted",
            "custom_lookup".into(),
            Some(serde_json::json!({"query": "cancelled lookup"})),
            "Stopped by user.".into(),
            130,
            None,
            ToolCallState::Interrupted,
            true,
        );
        transcript.finish_subagent(
            "agent-1".into(),
            "reviewer".into(),
            "Audit the tool state matrix".into(),
            true,
            "State transitions are consistent.".into(),
            true,
        );
        transcript.finish_subagent_with_outcome(
            "agent-2".into(),
            "planner".into(),
            "Stop a superseded planning branch".into(),
            SubagentOutcome::Cancelled,
            "Stopped after the primary branch completed.".into(),
            true,
        );
        transcript.finish_subagent_with_outcome(
            "agent-3".into(),
            "auditor".into(),
            "Verify an unavailable provider".into(),
            SubagentOutcome::Failed,
            "Provider authentication was unavailable.".into(),
            true,
        );

        for width in [24_u16, 32, 48, 80, 120] {
            let content_width = usize::from(width.saturating_sub(1));
            let compact = transcript
                .render_with_activity(width, content_width, true)
                .join("\n\n");
            let compact_plain = a3s_tui::style::strip_ansi(&compact);
            let compact_flow = compact_plain
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let compact_dense = compact_plain
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>();
            assert_bounded(&compact, content_width);
            assert!(
                compact_flow.contains("Context auto-compacted"),
                "{compact_plain}"
            );
            assert!(compact_flow.contains("Exploring"), "{compact_plain}");
            assert!(
                compact_flow.contains("Awaiting approval"),
                "{compact_plain}"
            );
            assert!(
                compact_flow.contains("1234 tests passed"),
                "{compact_plain}"
            );
            assert!(compact_flow.contains("diff · Ctrl+T"), "{compact_plain}");
            assert!(!compact_plain.contains("new-59"), "{compact_plain}");
            assert!(
                compact_flow.contains("Batch partially completed"),
                "{compact_plain}"
            );
            assert!(compact_flow.contains("exit 101"), "{compact_plain}");
            assert!(compact_flow.contains("Called docs.find"), "{compact_plain}");
            assert!(
                compact_dense.contains("providerdidnotrespond"),
                "{compact_plain}"
            );
            assert!(compact_flow.contains("Agent completed"), "{compact_plain}");
            assert!(compact_flow.contains("Denied"), "{compact_plain}");
            assert!(compact_flow.contains("Timed out"), "{compact_plain}");
            assert!(compact_flow.contains("Interrupted"), "{compact_plain}");
            assert!(compact_flow.contains("Agent cancelled"), "{compact_plain}");
            assert!(compact_flow.contains("Agent failed"), "{compact_plain}");
            assert!(!compact_flow.contains("Reasoning"), "{compact_plain}");

            let full = transcript
                .render_transcript_with_activity(width, content_width, true)
                .join("\n\n");
            let full_plain = a3s_tui::style::strip_ansi(&full);
            let full_flow = full_plain.split_whitespace().collect::<Vec<_>>().join(" ");
            let full_dense = full_plain
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>();
            assert_bounded(&full, content_width);
            assert!(full_flow.contains("Reasoning"), "{full_plain}");
            assert!(full_flow.contains("Input"), "{full_plain}");
            assert!(full_flow.contains("⊘ denied"), "{full_plain}");
            assert!(full_flow.contains("◷ timed out"), "{full_plain}");
            assert!(full_flow.contains("■ interrupted"), "{full_plain}");
            assert!(full_dense.contains("new-59"), "{full_plain}");
            assert!(full_dense.contains("Streamingstability"), "{full_plain}");
            assert!(full_flow.contains("! partial · 1 failed"), "{full_plain}");
        }
    }

    #[test]
    fn tracked_notice_finishes_in_place_after_unrelated_entries_arrive() {
        let mut transcript = Transcript::default();
        let status = transcript.push_tracked(TranscriptEntry::preformatted("interrupting…"));
        transcript.push(TranscriptEntry::assistant_markdown("partial answer"));
        transcript.start_tool("tool-after-status".into(), "bash".into(), true);

        assert!(transcript.replace_preformatted(status, "interrupted"));

        let entries = transcript.iter().collect::<Vec<_>>();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], &TranscriptEntry::preformatted("interrupted"));
        let plain = a3s_tui::style::strip_ansi(&transcript.render(80, 79).join("\n"));
        assert_eq!(plain.matches("interrupted").count(), 1, "{plain}");
        assert!(!plain.contains("interrupting"), "{plain}");
        assert!(plain.contains("partial answer"), "{plain}");
    }

    #[test]
    fn replacing_tracked_notice_invalidates_its_render_cache_and_layout() {
        let mut transcript = Transcript::default();
        let status = transcript.push_tracked(TranscriptEntry::preformatted("working…"));
        transcript.render(80, 79);
        assert!(transcript.entries[0].render_cache.is_some());
        assert!(!transcript.layout.is_empty());

        assert!(transcript.replace_preformatted(status, "done"));

        assert!(transcript.entries[0].render_cache.is_none());
        assert!(transcript.layout.is_empty());
        assert_eq!(transcript.render(80, 79), ["done"]);
    }

    #[test]
    fn cleared_transcript_does_not_reuse_a_late_status_id() {
        let mut transcript = Transcript::default();
        let stale = transcript.push_tracked(TranscriptEntry::preformatted("old operation…"));
        transcript.clear();
        let current = transcript.push_tracked(TranscriptEntry::preformatted("new session"));

        assert_ne!(stale, current);
        assert!(!transcript.replace_preformatted(stale, "late result"));
        assert_eq!(transcript.render(80, 79), ["new session"]);
    }

    #[test]
    fn transcript_entry_cache_survives_unrelated_tool_mutation() {
        let mut transcript = Transcript::from_entries(vec![TranscriptEntry::user("one")]);
        transcript.render(40, 39);
        let first_id = transcript.entries[0].id;
        assert!(transcript.entries[0].render_cache.is_some());
        transcript.start_tool("t".into(), "bash".into(), true);
        transcript.push_tool_input(Some("t"), r#"{"command":"echo"}"#);
        assert_eq!(transcript.entries[0].id, first_id);
        assert!(transcript.entries[0].render_cache.is_some());
    }

    #[test]
    fn semantic_anchor_survives_reflow_above_the_entry() {
        let mut transcript = Transcript::from_entries(vec![
            TranscriptEntry::assistant_markdown(
                "A deliberately long paragraph above the reading position that wraps very differently at narrow and wide terminal widths.",
            ),
            TranscriptEntry::assistant_markdown("anchor target\nsecond target row"),
        ]);
        transcript.render(28, 27);
        let target_id = transcript.entries[1].id;
        let old_span = transcript
            .layout
            .iter()
            .find(|span| span.entry_id == target_id)
            .copied()
            .unwrap();
        let anchor = transcript.anchor_for_row(old_span.start_row + 1).unwrap();

        transcript.render(100, 99);
        let restored = transcript.row_for_anchor(anchor).unwrap();
        let new_span = transcript
            .layout
            .iter()
            .find(|span| span.entry_id == target_id)
            .copied()
            .unwrap();

        assert!(restored >= new_span.start_row);
        assert!(restored < new_span.start_row + new_span.row_count);
        assert_ne!(old_span.start_row, new_span.start_row);
    }

    #[test]
    fn ctrl_t_render_keeps_full_tool_output_and_each_call_in_start_order() {
        let mut transcript = Transcript::default();
        let output = (0..18)
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        for (id, path) in [("read-1", "first.rs"), ("read-2", "second.rs")] {
            let args = serde_json::json!({"file_path": path});
            transcript.start_tool_execution(id.into(), "read".into(), args.clone(), true);
            transcript.finish_tool(id, "read".into(), Some(args), output.clone(), 0, None, true);
        }

        let compact = transcript.render(80, 79);
        assert_eq!(
            compact.len(),
            1,
            "successful explore calls compact in history"
        );

        let complete = transcript.render_transcript_with_activity(80, 79, true);
        assert_eq!(complete.len(), 2, "Ctrl+T retains each semantic tool call");
        let plain = complete
            .iter()
            .map(|block| a3s_tui::style::strip_ansi(block))
            .collect::<Vec<_>>()
            .join("\n\n");
        assert!(plain.find("first.rs").unwrap() < plain.find("second.rs").unwrap());
        assert_eq!(plain.matches("line-17").count(), 2, "{plain}");
        assert!(!plain.contains("… +"), "{plain}");
    }

    #[test]
    fn restored_tool_does_not_invent_a_replay_duration() {
        let mut transcript = Transcript::default();
        transcript.restore_tool_execution(
            "restored-1".into(),
            "bash".into(),
            serde_json::json!({"command":"echo restored"}),
            true,
        );
        transcript.finish_tool(
            "restored-1",
            "bash".into(),
            None,
            "restored output".into(),
            0,
            None,
            true,
        );

        let TranscriptEntry::Tool(tool) = transcript.iter().next().unwrap() else {
            panic!("expected tool entry");
        };
        assert_eq!(tool.duration, None);
        let plain = a3s_tui::style::strip_ansi(
            &transcript
                .render_transcript_with_activity(80, 79, true)
                .join("\n"),
        );
        assert!(plain.ends_with('✓'), "{plain}");
        assert!(!plain.contains("unknown"), "{plain}");
    }
}
