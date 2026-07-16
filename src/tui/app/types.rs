//! Shared application messages, rebuild state, and asynchronous runtime types.

use super::*;

/// Shared, single-consumer receiver for the active agent run. Wrapped so the
/// pump command can own a clone; pumps run sequentially, so the mutex never
/// actually contends.
pub(super) type SharedRx = Arc<Mutex<mpsc::Receiver<AgentEvent>>>;
pub(super) type SharedManifestRx =
    Arc<Mutex<tokio::sync::broadcast::Receiver<LocalWorkspaceManifestSnapshot>>>;
pub(super) type SharedActiveSession = Arc<std::sync::Mutex<Arc<AgentSession>>>;
pub(super) type StreamJoin = tokio::task::JoinHandle<()>;
pub(super) type HostToolAbort = tokio::task::AbortHandle;

#[derive(Clone)]
pub(super) struct LlmTurnUiCheckpoint {
    pub(super) turn: usize,
    pub(super) transcript_len: usize,
    pub(super) streaming: StreamingMarkdown,
    pub(super) thinking: String,
    pub(super) turn_text: String,
    pub(super) got_delta: bool,
    pub(super) turn_had_agent_activity: bool,
    pub(super) turn_text_after_activity: bool,
    pub(super) runtime_tools: RuntimeToolCheckpoint,
    pub(super) report_tools: ReportPhaseToolBuffer,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum State {
    Idle,
    Streaming,
    Awaiting,
    Rebuilding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InterruptedContinuation {
    DrainQueue,
    RestoreGoalMode,
    SettleDeepResearch,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ViewportAnchor {
    Bottom,
    Transcript(TranscriptAnchor),
    Absolute(usize),
}

#[derive(Clone)]
#[allow(clippy::enum_variant_names)]
pub(super) enum Action {
    ScrollUp,
    ScrollDown,
    ScrollTop,
    ScrollBottom,
}

/// Set by `/update` when an upgrade is available: after the TUI exits (terminal
/// restored), `run` performs the upgrade (Homebrew or standalone download) and
/// re-execs the freshly-installed binary.
pub(super) static UPGRADE_ON_EXIT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// The latest version tag, stashed by `/update` for the post-exit upgrade.
pub(super) static LATEST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AutoReviewKey {
    pub(super) session_id: String,
    pub(super) revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AutoReviewTicket {
    pub(super) id: u64,
    pub(super) key: AutoReviewKey,
}

#[derive(Debug)]
pub(super) struct AutoReviewTracker {
    pub(super) revision: u64,
    pub(super) reviewed: Option<AutoReviewKey>,
    pub(super) inflight: Option<AutoReviewTicket>,
    pub(super) next_ticket_id: u64,
}

impl AutoReviewTracker {
    pub(super) fn new(revision: u64) -> Self {
        Self {
            revision,
            reviewed: None,
            inflight: None,
            next_ticket_id: 0,
        }
    }

    pub(super) fn on_user_turn(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub(super) fn current_key(&self, session_id: &str) -> AutoReviewKey {
        AutoReviewKey {
            session_id: session_id.to_string(),
            revision: self.revision,
        }
    }

    pub(super) fn current_is_reviewed(&self, session_id: &str) -> bool {
        self.reviewed
            .as_ref()
            .is_some_and(|key| key.session_id == session_id && key.revision == self.revision)
    }

    /// Mark the current conversation revision as considered and, when it has a
    /// real user turn, issue a unique ticket for the asynchronous review.
    pub(super) fn begin(
        &mut self,
        session_id: &str,
        has_user_turn: bool,
    ) -> Option<AutoReviewTicket> {
        let key = self.current_key(session_id);
        if self.reviewed.as_ref() == Some(&key) {
            return None;
        }
        self.reviewed = Some(key.clone());
        if !has_user_turn {
            return None;
        }

        self.next_ticket_id = self.next_ticket_id.wrapping_add(1);
        let ticket = AutoReviewTicket {
            id: self.next_ticket_id,
            key,
        };
        // A newer conversation may replace an older in-flight ticket. The old
        // result will fail the exact-ticket check in `accept` and cannot clear it.
        self.inflight = Some(ticket.clone());
        Some(ticket)
    }

    pub(super) fn accept(&mut self, ticket: &AutoReviewTicket, session_id: &str) -> bool {
        if self.inflight.as_ref() != Some(ticket) {
            return false;
        }
        self.inflight = None;
        ticket.key.session_id == session_id
            && ticket.key.revision == self.revision
            && self.reviewed.as_ref() == Some(&ticket.key)
    }
}

pub(super) fn auto_review_history_has_user_turn(history: &[Message]) -> bool {
    history
        .iter()
        .any(|message| message.role == "user" && !message.text().trim().is_empty())
}

pub(super) enum SessionRebuildAction {
    Model {
        model: String,
        source: ModelSelectionSource,
        llm_override: Option<LlmOverride>,
        context_limit: u32,
    },
    Effort {
        selected: usize,
        codex_effort: Option<CodexEffortStatus>,
    },
    GoalStart {
        generation: u64,
        previous_effort: usize,
        previous_goal: Option<String>,
        previous_goal_since: Option<Instant>,
    },
    GoalResume {
        generation: u64,
        paused: PausedGoalState,
    },
    GoalRestore,
    Compact {
        summary: String,
        session_id: String,
    },
    Fork {
        session_id: String,
    },
    Clear {
        session_id: String,
    },
    Reload {
        skill_count: usize,
    },
    Refresh {
        failure_context: Option<&'static str>,
    },
}

pub(super) struct SessionRebuildProfile {
    pub(super) session_id: String,
    pub(super) model: Option<String>,
    pub(super) effort: usize,
    pub(super) context_limit: u32,
    pub(super) llm_override: Option<LlmOverride>,
    pub(super) compact_summary: Option<String>,
}

pub(super) struct IdeIntelligenceResult {
    pub(super) title: String,
    pub(super) rows: Vec<IdeIntelligenceRow>,
    pub(super) truncated: bool,
    pub(super) saved_version: bool,
    pub(super) dirty_buffer: bool,
    pub(super) stale: bool,
    pub(super) workspace_revision: Option<u64>,
}

pub(super) struct IdeIntelligenceJump {
    pub(super) path: PathBuf,
    pub(super) lines: Vec<String>,
    pub(super) row: usize,
    pub(super) col: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionRebuildMode {
    /// Reconfigure an existing persisted session without ever replacing a
    /// failed resume with an empty session using the same id.
    ResumeExisting,
    /// Materialize a deliberately new id for `/clear` or `/compact`.
    CreateFresh,
}

pub(super) enum Msg {
    Term(Event),
    // Boxed: AgentEvent is large; keeps the Msg enum small.
    Agent {
        source: SharedRx,
        event: Box<AgentEvent>,
    },
    Submit(String),
    StreamStarted {
        token: u64,
        session: Arc<AgentSession>,
        rx: SharedRx,
        join: StreamJoin,
        /// Temporary previews retained until stream admission succeeds.
        submitted_images: Vec<PendingImage>,
    },
    StreamEnded(SharedRx),
    StreamJoinSettled {
        token: u64,
        synthesis: Option<(String, String)>,
    },
    DiscardedStreamSettled {
        token: u64,
    },
    /// Session cancellation and the active stream worker have settled enough
    /// for the terminal program to restore the shell without detaching work.
    QuitReady,
    StreamError {
        token: u64,
        error: String,
        /// Admission can be retried without duplicating a persisted user turn.
        retryable_admission: bool,
        /// Restored to the composer when stream admission fails.
        submitted_images: Vec<PendingImage>,
    },
    /// Bounded backoff before retrying a queued turn that raced Core's
    /// single-flight lease release.
    QueueRetry {
        generation: u64,
    },
    WorkspaceManifest(Box<LocalWorkspaceManifestSnapshot>),
    WorkspaceManifestStopped,
    IdeIntelligenceCompleted {
        request_id: u64,
        result: Result<IdeIntelligenceResult, String>,
    },
    IdeIntelligenceJumpCompleted {
        request_id: u64,
        jump_request_id: u64,
        result: Result<IdeIntelligenceJump, String>,
    },
    SpinnerTick,
    /// Advance Codex-style Markdown commit animation independently from the
    /// slower status spinner.
    StreamCommitTick,
    /// Advance the welcome-mascot animation frame.
    BannerTick,
    /// Drive the short, high-frame-rate Ultracode activation transition.
    UltracodeTick {
        epoch: u64,
    },
    ModalConfirm {
        tool_id: String,
        approved: bool,
        approve_all_pending: bool,
    },
    BackgroundSubagentFinished {
        session_id: String,
        generation: u64,
        task_id: String,
        agent: String,
        output: String,
        outcome: SubagentOutcome,
        finished_ms: u64,
    },
    BackgroundSubagentWatchStopped {
        session_id: String,
        generation: u64,
        task_id: String,
    },
    SubagentSnapshots {
        session_id: String,
        generation: u64,
        request_id: u64,
        snapshots: Vec<RestoredSubagentSnapshot>,
    },
    /// The active DeepResearch parent reached a report terminal state. Its
    /// children must be terminal before the report view opens and autonomy is
    /// restored, otherwise the footer advertises work after the parent ended.
    DeepResearchSubagentsSettled {
        session_id: String,
        generation: u64,
        exit: DeepResearchSettlementExit,
        settlements: Vec<DeepResearchSubagentSettlement>,
    },
    DeepResearchJournalFinalized {
        run_id: String,
        exit: DeepResearchSettlementExit,
        result: Result<ResearchRunProjection, String>,
    },
    DeepResearchJournalEventRecorded {
        run_id: String,
        result: Result<ResearchRunProjection, String>,
    },
    Resume,
    Interrupted {
        goal_cancelled: bool,
        status_entry: TranscriptEntryId,
    },
    /// Output of a `!`-prefixed shell command.
    ShellOutput(String),
    ResearchDiagnostic(Result<String, String>),
    /// Host-controlled `?` deep-research workflow finished; next step is synthesis.
    DeepResearchWorkflowCompleted {
        query: String,
        os_runtime: bool,
        args: serde_json::Value,
        result: Result<ToolCallResult, String>,
        convergence: ConvergenceDecision,
        accepted_evidence: Vec<AcceptedEvidence>,
    },
    /// Host-owned structured report generation completed. DeepResearch uses
    /// this closed-evidence path instead of reopening a general agent stream.
    DeepResearchReportGenerated {
        token: u64,
        query: String,
        phase: DeepResearchReportGenerationPhase,
        result: Result<ToolCallResult, String>,
    },
    /// A DeepResearch synthesis/repair stream exceeded its host-side model budget.
    DeepResearchSynthesisTimedOut {
        token: u64,
    },
    /// A timed-out DeepResearch synthesis/repair stream was cancelled at the session layer.
    DeepResearchSynthesisTimedOutAfterCancel {
        token: u64,
        status: String,
        streamed_text: String,
        report_completed: bool,
    },
    /// `/update` version check finished: the latest version tag, if reachable.
    UpdatePlan(Option<String>),
    /// `/update` found no binary upgrade was needed and repaired companion tools.
    UpdateRepair {
        status_entry: TranscriptEntryId,
        result: Result<Vec<String>, String>,
    },
    /// OS login completed.
    OsLogin {
        status_entry: TranscriptEntryId,
        result: Result<String, String>,
    },
    /// Post-login SSH-key sync finished (registers the local pubkey with OS).
    SshKeySynced(crate::a3s_os::SshKeyOutcome),
    /// OS access token was refreshed (or refresh failed) in the background.
    OsRefreshed(Result<crate::a3s_os::StoredOsSession, String>),
    /// OS unified-gateway model ids fetched for the `/model` picker.
    OsGatewayModels {
        login_at_ms: u64,
        result: Result<Vec<crate::a3s_os::GatewayModel>, String>,
    },
    /// Models discovered from a detected local developer-tool account.
    AccountModels {
        provider: crate::account_providers::AccountProvider,
        result: Result<Vec<String>, String>,
    },
    /// Host-owned continuation for an active `/goal`. The generation makes a
    /// delayed retry inert after Esc, `/goal clear`, or a replacement goal.
    GoalContinue {
        generation: u64,
        prompt: String,
    },
    /// A streaming `/goal clear` finished cancelling and joining the old run.
    GoalCleared,
    /// Picker-visible models refreshed through the signed-in Codex CLI.
    CodexModels(Result<Vec<crate::account_providers::codex::CodexModel>, String>),
    /// An async session rebuild for `/model`, `/effort`, or another
    /// session-mutating TUI action completed.
    SessionRebuilt {
        request_id: u64,
        action: SessionRebuildAction,
        result: Box<panels::model::SessionRebuildResult>,
    },
    /// `/fork` copied the session under a new id (Ok) — swap the active session to
    /// it — or failed (Err with a reason).
    Forked {
        request_id: u64,
        result: Result<String, String>,
    },
    /// `/memory` graph data loaded (timeline + details + derived graph).
    MemoryLoaded(MemPanelData),
    /// A `/memory` forget-candidate deletion finished, with fresh graph data.
    MemoryForgotten(Result<(String, MemPanelData), String>),
    /// Asset-scoped OS asset list loaded.
    AssetListLoaded(Result<panels::asset_resources::AssetListFetch, String>),
    /// Runtime activity rows loaded for an asset-scoped activity panel.
    RuntimeActivityLoaded(Result<panels::asset_resources::RuntimeActivityFetch, String>),
    /// `/kb import` finished; carries the one-line summary to show.
    KbAdded(String),
    /// `/ctx <query>` finished: raw `ctx search --json` stdout (or the error).
    CtxResults {
        status_entry: TranscriptEntryId,
        result: Result<String, String>,
    },
    /// `/ctx <n>` finished: (hit title, transcript window) to stage as context.
    CtxWindow {
        status_entry: TranscriptEntryId,
        result: Result<(String, String), String>,
    },
    /// `/ctx save <n>` finished: Ok(hit title) once written to the memory store.
    CtxSaved(Result<String, String>),
    /// `/sleep` finished persisting its consolidated memories (count on Ok).
    SleepSaved(Result<usize, String>),
    /// `/flow` published/opened/inspected an OS Workflow as a Service asset.
    FlowOsCompleted {
        status_entry: TranscriptEntryId,
        result: Result<panels::flow::FlowOsResult, String>,
    },
    /// `/agent` published/opened an OS agent asset through Agent as a Service or Function as a Service.
    AgentOsCompleted {
        status_entry: TranscriptEntryId,
        result: Result<panels::agent::AgentOsResult, String>,
    },
    /// `/mcp` published/ran/tested an OS Function as a Service MCP asset.
    McpOsCompleted {
        status_entry: TranscriptEntryId,
        result: Result<panels::mcp::McpOsResult, String>,
    },
    /// `/skill` published/deployed/inspected an OS Function as a Service skill asset.
    SkillOsCompleted {
        status_entry: TranscriptEntryId,
        result: Result<panels::skill::SkillOsResult, String>,
    },
    /// `/okf` published/deployed an OS Knowledge service package asset.
    OkfOsCompleted {
        status_entry: TranscriptEntryId,
        result: Result<panels::okf::OkfOsResult, String>,
    },
    /// Asset source was cloned into the local asset workspace.
    AssetCloned {
        status_entry: TranscriptEntryId,
        result: Result<asset_clone::AssetCloneResult, String>,
    },
    /// `/memory` → ctx back-jump finished: (ctx event id, transcript window).
    CtxMemorySource(Result<(String, String), String>),
    /// Inactivity auto-review summary text, tagged so stale background results
    /// cannot appear after a new turn, `/clear`, compact, or fork.
    AutoReview {
        ticket: AutoReviewTicket,
        text: String,
    },
    /// `/compact` completed its direct, tool-free summary request.
    Compacted(Result<Option<String>, String>),
    /// Startup update check completed with the latest published version (if any).
    UpdateCheck(Option<String>),
}

pub(super) struct RestoredSubagentSnapshot {
    pub(super) snapshot: a3s_code_core::SubagentTaskSnapshot,
    pub(super) parent_result_expected: bool,
}

pub(super) struct DeepResearchSubagentSettlement {
    pub(super) task_id: String,
    pub(super) agent: String,
    pub(super) output: String,
    pub(super) outcome: SubagentOutcome,
    pub(super) finished_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeepResearchReportGenerationPhase {
    Synthesis,
    Repair,
}

impl DeepResearchReportGenerationPhase {
    pub(super) fn is_repair(self) -> bool {
        matches!(self, Self::Repair)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeepResearchSettlementExit {
    ReportReady,
    Interrupted,
}

impl DeepResearchSettlementExit {
    pub(super) fn opens_report(self) -> bool {
        matches!(self, Self::ReportReady)
    }
}

impl From<Event> for Msg {
    fn from(event: Event) -> Self {
        // Ctrl+C is handled in the key loop as a global graceful quit key.
        Msg::Term(event)
    }
}
