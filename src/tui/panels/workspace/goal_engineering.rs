//! Durable `/goal` execution: an Ultracode session backed by a Loop
//! Engineering workspace and a host-owned achievement latch.

#[path = "goal_acceptance.rs"]
mod goal_acceptance;

use self::goal_acceptance::{
    acceptance_criteria_progress as acceptance_progress_from_body,
    collect_passing_machine_fingerprints, upsert_verified_evidence_section,
    verify_checked_machine_criteria, workspace_root_from_loop_dir,
};
use super::super::*;
use super::loop_engineering::{self, LoopSpec, BUDGET_FILE, LOOP_CONFIG, RUN_LOG_FILE, STATE_FILE};
use a3s_code_core::planning::AgentGoal;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

pub(crate) fn acceptance_criteria_progress(acceptance: &str) -> (usize, usize) {
    acceptance_progress_from_body(acceptance)
}

const GOAL_RUNTIME_START: &str = "<!-- a3s-goal-runtime:start -->";
const GOAL_RUNTIME_END: &str = "<!-- a3s-goal-runtime:end -->";
const GOAL_BUDGET_TOKENS_PER_DAY: u64 = 500_000;
const GOAL_PROGRESS_PERSIST_STEP_PERCENT: u8 = 5;
/// Soft stall gate: after this many consecutive iterations without a matching
/// verified GoalAchieved, pause for the user instead of burning unbounded retries.
const MAX_UNVERIFIED_STREAK: usize = 5;
/// Host admission ceiling for `/goal` parallel fan-out (plan: prefer 2–4).
/// Ultracode's interactive budget may request more; durable goals clamp here.
pub(crate) const GOAL_MAX_PARALLEL_TASKS: usize = 4;
const ACCEPTANCE_FILE: &str = "ACCEPTANCE.md";

/// Cap session `max_parallel_tasks` while a durable `/goal` is active.
pub(crate) fn goal_capped_parallel_tasks(requested: usize) -> usize {
    requested.min(GOAL_MAX_PARALLEL_TASKS).max(1)
}

/// Session rebuild parallel admission: clamp only when a goal run is armed.
///
/// Call sites must pass `goal_run.is_some()` — this is the host branch that
/// keeps Ultracode's wider interactive budget from applying during `/goal`.
pub(crate) fn goal_session_max_parallel_tasks(goal_active: bool, requested: usize) -> usize {
    if goal_active {
        goal_capped_parallel_tasks(requested)
    } else {
        requested
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GoalRunState {
    pub(crate) generation: u64,
    pub(crate) spec: LoopSpec,
    pub(crate) iteration: usize,
    pub(crate) progress: f32,
    pub(crate) achieved: bool,
    pub(crate) failures: usize,
    pub(crate) phase: GoalPhase,
    pub(crate) unverified_streak: usize,
    accepting_achievement: bool,
    extracted_goal: Option<String>,
}

impl GoalRunState {
    fn new(generation: u64, spec: LoopSpec) -> Self {
        Self {
            generation,
            spec,
            iteration: 1,
            progress: 0.0,
            achieved: false,
            failures: 0,
            phase: GoalPhase::Maker,
            unverified_streak: 0,
            accepting_achievement: true,
            extracted_goal: None,
        }
    }

    pub(crate) fn is_generation(&self, generation: u64) -> bool {
        self.generation == generation
    }

    pub(crate) fn paused_state(&self) -> PausedGoalState {
        PausedGoalState {
            loop_id: self.spec.id.clone(),
            goal: self.spec.goal.clone(),
            iteration: self.iteration,
            progress: self.progress,
            failures: self.failures,
            phase: self.phase,
            unverified_streak: self.unverified_streak,
        }
    }

    pub(crate) fn from_paused(
        cwd: &str,
        generation: u64,
        paused: &PausedGoalState,
    ) -> Result<Self, String> {
        if !paused.progress.is_finite() {
            return Err("saved goal progress is not finite".to_string());
        }
        let spec = init_goal_loop(cwd, &paused.goal)?;
        if spec.id != paused.loop_id {
            return Err(format!(
                "saved loop id `{}` does not match goal loop `{}`",
                paused.loop_id, spec.id
            ));
        }
        Ok(Self {
            generation,
            spec,
            iteration: paused.iteration.max(1),
            progress: paused.progress.clamp(0.0, 1.0),
            achieved: false,
            failures: paused.failures,
            phase: paused.phase,
            unverified_streak: paused.unverified_streak,
            accepting_achievement: false,
            extracted_goal: None,
        })
    }

    fn record_extracted_goal(&mut self, goal: &AgentGoal) {
        if !self.accepting_achievement {
            return;
        }
        // Canonicalize to the durable user goal. Divergent extracts cannot latch.
        if normalize_goal(&goal.description) != normalize_goal(&self.spec.goal) {
            return;
        }
        self.extracted_goal = Some(self.spec.goal.clone());
        self.progress = goal.progress.clamp(0.0, 1.0);
    }

    fn record_progress(&mut self, progress: f32) {
        if !self.accepting_achievement {
            return;
        }
        self.progress = progress.clamp(0.0, 1.0);
    }

    fn record_achievement(&mut self, event_goal: &str) -> bool {
        if self.achievement_reject_reason(event_goal).is_some() {
            return false;
        }
        // ACCEPTANCE.md + verifier phase + matching user goal already checked.
        self.achieved = true;
        self.progress = 1.0;
        self.unverified_streak = 0;
        true
    }

    /// Why a Core `GoalAchieved` cannot close this host loop right now.
    fn achievement_reject_reason(&self, event_goal: &str) -> Option<String> {
        if !self.accepting_achievement {
            return Some("ignored during user-turn pause".to_string());
        }
        if self.phase != GoalPhase::Verifier {
            return Some(format!(
                "ignored in {} phase; only verifier can latch",
                self.phase.as_str()
            ));
        }
        let event_goal = normalize_goal(event_goal);
        let user_goal = normalize_goal(&self.spec.goal);
        let matches_user = event_goal == user_goal;
        let matches_extract = self
            .extracted_goal
            .as_deref()
            .is_some_and(|goal| normalize_goal(goal) == event_goal);
        if !(matches_user && matches_extract) {
            return Some("ignored; event goal does not match durable user goal".to_string());
        }
        let (done, total) = read_acceptance_progress(self);
        if total == 0 || done != total {
            return Some(format!(
                "ignored; ACCEPTANCE criteria still open ({done}/{total})"
            ));
        }
        if let Err(reason) = reverify_acceptance_contract(self) {
            return Some(reason);
        }
        None
    }

    fn begin_next_iteration(&mut self, failed: bool) {
        self.iteration = self.iteration.saturating_add(1);
        self.progress = 0.0;
        self.accepting_achievement = true;
        self.extracted_goal = None;
        self.phase = self.phase.next();
        if failed {
            self.failures = self.failures.saturating_add(1);
        } else {
            self.failures = 0;
        }
        self.unverified_streak = self.unverified_streak.saturating_add(1);
    }

    fn begin_resumed_iteration(&mut self) {
        self.iteration = self.iteration.saturating_add(1);
        self.progress = 0.0;
        self.accepting_achievement = true;
        self.extracted_goal = None;
        // Resume continues the stalled phase; do not skip verifier.
        self.unverified_streak = 0;
    }

    pub(crate) fn pause_achievement_for_user_turn(&mut self) {
        self.accepting_achievement = false;
        self.extracted_goal = None;
    }

    pub(crate) fn pause_for_exit(&mut self) {
        self.accepting_achievement = false;
        self.extracted_goal = None;
        let _ = persist_runtime_state(self, "paused", "TUI exited; waiting for session resume");
        let _ = append_goal_log(self, "paused", "TUI exited; waiting for session resume");
    }

    fn should_pause_for_stall(&self) -> bool {
        self.unverified_streak >= MAX_UNVERIFIED_STREAK
    }

    /// Primary completion meter: ACCEPTANCE criteria coverage (not plan %).
    pub(crate) fn criteria_progress(&self) -> (usize, usize) {
        read_acceptance_progress(self)
    }
}

pub(crate) fn mark_paused_goal_cancelled(cwd: &str, paused: &PausedGoalState, reason: &str) {
    let Ok(run) = GoalRunState::from_paused(cwd, 0, paused) else {
        return;
    };
    let _ = persist_runtime_state(&run, "cancelled", reason);
    let _ = append_goal_log(&run, "cancelled", reason);
}

/// Cancel on-disk `goal-*` loops still marked running/retrying/paused, except
/// `keep_id`. Prevents orphaned active STATUS from polluting Core ACCEPTANCE
/// emit when the user starts a replacement `/goal`.
fn cancel_orphaned_active_goal_loops(cwd: &Path, keep_id: Option<&str>, reason: &str) {
    const ACTIVE: &[&str] = &["running", "retrying", "paused"];
    let loops = cwd.join(".a3s").join("loops");
    let Ok(entries) = std::fs::read_dir(&loops) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !id.starts_with("goal-") {
            continue;
        }
        if keep_id == Some(id) {
            continue;
        }
        let state_path = path.join(STATE_FILE);
        let Ok(state) = std::fs::read_to_string(&state_path) else {
            continue;
        };
        let Some(status) = state.lines().find_map(|line| {
            line.trim()
                .strip_prefix("Status:")
                .map(|value| value.trim().to_string())
        }) else {
            continue;
        };
        if !ACTIVE
            .iter()
            .any(|allowed| status.eq_ignore_ascii_case(allowed))
        {
            continue;
        }
        let updated = rewrite_goal_runtime_status(&state, "cancelled", reason);
        let _ = std::fs::write(&state_path, updated);
        let log_path = path.join(RUN_LOG_FILE);
        let _ = append_plain_goal_log(&log_path, "cancelled", reason);
    }
}

fn rewrite_goal_runtime_status(state: &str, status: &str, event: &str) -> String {
    let event = event.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(state.len() + 32);
    let mut wrote_status = false;
    let mut wrote_event = false;
    for line in state.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Status:") {
            out.push_str(&format!("Status: {status}\n"));
            wrote_status = true;
            continue;
        }
        if trimmed.starts_with("Last event:") {
            out.push_str(&format!("Last event: {event}\n"));
            wrote_event = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !wrote_status {
        out.push_str(&format!("Status: {status}\n"));
    }
    if !wrote_event {
        out.push_str(&format!("Last event: {event}\n"));
    }
    out
}

fn append_plain_goal_log(log_path: &Path, status: &str, event: &str) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| error.to_string())?;
    writeln!(file, "- `{status}` {event}").map_err(|error| error.to_string())
}

fn normalize_goal(goal: &str) -> String {
    goal.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn goal_loop_id(goal: &str) -> String {
    let digest = Sha256::digest(goal.as_bytes());
    let suffix = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("goal-{}-{suffix}", loop_engineering::slug(goal))
}

fn write_if_missing(path: &Path, body: &str) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, body).map_err(|error| error.to_string())
}

fn initial_goal_state(spec: &LoopSpec) -> String {
    format!(
        "# Goal Loop State: {}\n\n\
         {GOAL_RUNTIME_START}\n\
         Status: ready\n\
         Iteration: 0\n\
         Phase: maker\n\
         Criteria: 0/1\n\
         Unverified streak: 0\n\
         Plan progress (secondary, not completion): 0%\n\
         Goal: {}\n\
         Last event: waiting to start\n\
         {GOAL_RUNTIME_END}\n\n\
         ## Verified Evidence\n\n\
         - None yet.\n\n\
         ## Remaining Work\n\n\
         - Establish the implementation and verification plan.\n\n\
         ## Human Handoff\n\n\
         - None.\n",
        spec.id, spec.goal
    )
}

fn initial_acceptance(goal: &str) -> String {
    format!(
        "# Acceptance Contract\n\n\
         ## Immutable Goal\n\n\
         {goal}\n\n\
         ## Observable Criteria\n\n\
         Use Markdown task items (`- [ ]` open, `- [x]` proven). Prefer machine-checkable \
         forms so the host can re-verify before latching:\n\
         - kind:command assert:`<shell>` expect:exit=0\n\
         - kind:file_exists assert:<path>\n\
         - kind:manual assert:<text> (checkbox only; cannot latch alone)\n\
         Replace or extend the starter item with at least one `kind:command` or \
         `kind:file_exists` criterion before the host will latch GoalAchieved.\n\
         Do not list host lifecycle events (GoalAchieved, DONE, iteration limits) as criteria.\n\n\
         - [ ] kind:manual assert:{goal}\n\n\
         ## Evidence Log\n\n\
         - None yet.\n"
    )
}

fn read_acceptance_progress(run: &GoalRunState) -> (usize, usize) {
    let path = run.spec.dir.join(ACCEPTANCE_FILE);
    let body = std::fs::read_to_string(path).unwrap_or_default();
    acceptance_criteria_progress(&body)
}

fn reverify_acceptance_contract(run: &GoalRunState) -> Result<(), String> {
    let path = run.spec.dir.join(ACCEPTANCE_FILE);
    let body = std::fs::read_to_string(&path).map_err(|error| {
        format!("ignored; cannot read ACCEPTANCE.md for host re-check ({error})")
    })?;
    let workspace = workspace_root_from_loop_dir(&run.spec.dir).ok_or_else(|| {
        "ignored; cannot resolve workspace root for ACCEPTANCE re-check".to_string()
    })?;
    verify_checked_machine_criteria(&body, &workspace).map_err(|error| format!("ignored; {error}"))
}

/// Maker may mutate under Auto; verifier keeps Plan chrome but host arms
/// goal-verify posture (sandboxed bash + writes only under the loop dir) so
/// Core can collect `verification_reports` without opening implementation writes.
pub(super) fn composer_mode_for_goal_phase(phase: GoalPhase) -> Mode {
    match phase {
        GoalPhase::Maker => Mode::Auto,
        GoalPhase::Verifier => Mode::Plan,
    }
}

/// Core Plan specialty denies bash; verifier must clear specialty so reports can form.
pub(super) fn agent_style_for_goal_phase(phase: GoalPhase) -> Option<a3s_code_core::AgentStyle> {
    match phase {
        GoalPhase::Maker => Mode::Auto.agent_style(),
        GoalPhase::Verifier => None,
    }
}

fn budget_text() -> String {
    format!(
        "# Goal budget (advisory)\n\
         #\n\
         # tokens_per_day is an advisory ceiling for humans and operators.\n\
         # The host does not meter tokens to stop the loop; completion is gated\n\
         # only by a matching verified GoalAchieved during the verifier phase,\n\
         # with a soft pause after {MAX_UNVERIFIED_STREAK} unverified iterations.\n\
         tokens_per_day = {GOAL_BUDGET_TOKENS_PER_DAY}\n\
         max_iterations_per_run = 0\n\
         max_unverified_streak = {MAX_UNVERIFIED_STREAK}\n\
         completion_gate = \"goal_achieved_event\"\n\
         kill_switch = false\n"
    )
}

fn maker_skill(goal: &str) -> String {
    format!(
        "# Goal Maker\n\n\
         Work toward this exact goal: {goal}\n\n\
         Read `../ACCEPTANCE.md`, `../STATE.md`, and `../RUN_LOG.md` first. Inspect the current \
         workspace before editing. Make the smallest coherent change that advances the goal, \
         preserve unrelated user work, refine ACCEPTANCE.md with machine-checkable criteria \
         (`kind:command` / `kind:file_exists` / `kind:manual`), and record concrete evidence in \
         `../STATE.md`. Never mark your own change verified; hand it to the verifier.\n"
    )
}

fn verifier_skill(goal: &str) -> String {
    format!(
        "# Goal Verifier\n\n\
         Independently verify this exact goal: {goal}\n\n\
         Derive observable success criteria from the goal, inspect the maker's actual changes, run \
         proportionate verification commands (so Core can collect structured verification reports), and \
         reject completion when evidence is missing, stale, indirect, or contradicted by the workspace. \
         Update `../ACCEPTANCE.md` with `kind:command` / `kind:file_exists` / `kind:manual` items only \
         when fresh evidence proves them. Prefer running ACCEPTANCE `kind:command` asserts via bash so \
         Core attaches structured verification reports (same predicates the host re-runs at latch). \
         The host re-runs command/file criteria before latching. \
         You may write only under this loop directory; do not mutate implementation sources. Record \
         commands, outcomes, residual risks, and remaining gaps in `../STATE.md`.\n"
    )
}

pub(crate) fn init_goal_loop(cwd: &str, raw_goal: &str) -> Result<LoopSpec, String> {
    let goal = normalize_goal(raw_goal);
    if goal.is_empty() {
        return Err("goal cannot be empty".to_string());
    }
    let id = goal_loop_id(&goal);
    let dir = Path::new(cwd).join(".a3s").join("loops").join(&id);
    let config = dir.join(LOOP_CONFIG);

    if config.exists() {
        let spec = loop_engineering::read_spec(&config)?;
        if normalize_goal(&spec.goal) != goal {
            return Err(format!("goal loop id collision for `{id}`"));
        }
        std::fs::create_dir_all(dir.join("skills")).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(dir.join("reports")).map_err(|error| error.to_string())?;
        write_if_missing(&dir.join(STATE_FILE), &initial_goal_state(&spec))?;
        write_if_missing(&dir.join(ACCEPTANCE_FILE), &initial_acceptance(&goal))?;
        write_if_missing(
            &dir.join(RUN_LOG_FILE),
            &format!("# Goal Run Log: {}\n\n", spec.id),
        )?;
        write_if_missing(&dir.join(BUDGET_FILE), &budget_text())?;
        write_if_missing(&dir.join("skills").join("maker.md"), &maker_skill(&goal))?;
        write_if_missing(
            &dir.join("skills").join("verifier.md"),
            &verifier_skill(&goal),
        )?;
        return Ok(spec);
    }

    std::fs::create_dir_all(dir.join("skills")).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(dir.join("reports")).map_err(|error| error.to_string())?;
    let spec = LoopSpec {
        id,
        pattern: "goal-engineering".to_string(),
        goal,
        level: "G1".to_string(),
        cadence: "continuous-until-achieved".to_string(),
        os_runtime: false,
        worktree: false,
        maker_agent: "goal-maker".to_string(),
        checker_agent: "goal-verifier".to_string(),
        budget_tokens_per_day: GOAL_BUDGET_TOKENS_PER_DAY,
        // Zero is deliberately unbounded: only GoalAchieved closes this loop.
        max_iterations_per_run: 0,
        denylist: vec![".env*".to_string(), "secrets/**".to_string()],
        connectors: Vec::new(),
        dir: dir.clone(),
    };
    std::fs::write(&config, loop_engineering::spec_text(&spec))
        .map_err(|error| error.to_string())?;
    std::fs::write(dir.join(STATE_FILE), initial_goal_state(&spec))
        .map_err(|error| error.to_string())?;
    std::fs::write(dir.join(ACCEPTANCE_FILE), initial_acceptance(&spec.goal))
        .map_err(|error| error.to_string())?;
    std::fs::write(
        dir.join(RUN_LOG_FILE),
        format!("# Goal Run Log: {}\n\n", spec.id),
    )
    .map_err(|error| error.to_string())?;
    std::fs::write(dir.join(BUDGET_FILE), budget_text()).map_err(|error| error.to_string())?;
    std::fs::write(dir.join("skills").join("maker.md"), maker_skill(&spec.goal))
        .map_err(|error| error.to_string())?;
    std::fs::write(
        dir.join("skills").join("verifier.md"),
        verifier_skill(&spec.goal),
    )
    .map_err(|error| error.to_string())?;
    Ok(spec)
}

fn runtime_section(run: &GoalRunState, status: &str, event: &str) -> String {
    let event = event.split_whitespace().collect::<Vec<_>>().join(" ");
    let (criteria_done, criteria_total) = read_acceptance_progress(run);
    let criteria = if criteria_total == 0 {
        "none".to_string()
    } else {
        format!("{criteria_done}/{criteria_total}")
    };
    format!(
        "{GOAL_RUNTIME_START}\n\
         Status: {status}\n\
         Iteration: {}\n\
         Phase: {}\n\
         Criteria: {criteria}\n\
         Unverified streak: {}\n\
         Plan progress (secondary, not completion): {:.0}%\n\
         Goal: {}\n\
         Last event: {event}\n\
         {GOAL_RUNTIME_END}",
        run.iteration,
        run.phase.as_str(),
        run.unverified_streak,
        run.progress * 100.0,
        run.spec.goal,
    )
}

fn goal_progress_checkpoint(progress: f32) -> u8 {
    let percent = (progress.clamp(0.0, 1.0) * 100.0).floor() as u8;
    if percent == 100 {
        return 100;
    }
    (percent / GOAL_PROGRESS_PERSIST_STEP_PERCENT) * GOAL_PROGRESS_PERSIST_STEP_PERCENT
}

fn runtime_document_with_section(mut current: String, loop_id: &str, replacement: &str) -> String {
    let runtime_range = current.find(GOAL_RUNTIME_START).and_then(|start| {
        current[start..].find(GOAL_RUNTIME_END).map(|relative_end| {
            let end = start + relative_end + GOAL_RUNTIME_END.len();
            start..end
        })
    });
    if let Some(range) = runtime_range {
        current.replace_range(range, replacement);
    } else if current.trim().is_empty() {
        current = format!("# Goal Loop State: {loop_id}\n\n{replacement}\n");
    } else {
        current.push_str("\n\n");
        current.push_str(replacement);
        current.push('\n');
    }
    current
}

fn persist_runtime_state(run: &GoalRunState, status: &str, event: &str) -> Result<(), String> {
    let path = run.spec.dir.join(STATE_FILE);
    let previous = std::fs::read_to_string(&path).unwrap_or_default();
    let replacement = runtime_section(run, status, event);
    let with_runtime = runtime_document_with_section(previous.clone(), &run.spec.id, &replacement);
    let current = sync_verified_evidence_fingerprints(run, with_runtime);
    if current == previous {
        return Ok(());
    }
    std::fs::write(path, current).map_err(|error| error.to_string())
}

/// Refresh STATE.md `## Verified Evidence` from currently-passing machine criteria.
/// Fingerprints guide makers to skip unchanged work; they never skip latch re-checks.
fn sync_verified_evidence_fingerprints(run: &GoalRunState, state: String) -> String {
    let acceptance_path = run.spec.dir.join(ACCEPTANCE_FILE);
    let acceptance = std::fs::read_to_string(acceptance_path).unwrap_or_default();
    let Some(workspace) = workspace_root_from_loop_dir(&run.spec.dir) else {
        return state;
    };
    let fingerprints = collect_passing_machine_fingerprints(&acceptance, &workspace);
    upsert_verified_evidence_section(&state, &fingerprints)
}

fn append_goal_log(run: &GoalRunState, status: &str, detail: &str) -> Result<(), String> {
    let detail = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = format!(
        "- {} iteration={} · status={} · {}\n",
        chrono::Utc::now().to_rfc3339(),
        run.iteration,
        status,
        detail
    );
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run.spec.dir.join(RUN_LOG_FILE))
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|error| error.to_string())
}

pub(crate) fn goal_run_prompt(run: &GoalRunState) -> String {
    let phase_contract = match run.phase {
        GoalPhase::Maker => {
            "Phase: maker. Implement the highest-value remaining gap. Update ACCEPTANCE.md and \
             STATE.md with concrete evidence and open criteria. Do not treat this phase as \
             completion — the host only accepts GoalAchieved during a later verifier phase."
        }
        GoalPhase::Verifier => {
            "Phase: verifier (Plan chrome + host goal-verify fence). Independently inspect the \
             workspace and run fresh verification checks for every open acceptance criterion. Prefer \
             `kind:command` / `kind:file_exists` criteria. Run ACCEPTANCE `kind:command` asserts via \
             sandboxed bash so Core can collect structured verification reports that match those \
             predicates. You may update files only under this loop \
             directory (ACCEPTANCE.md / STATE.md / RUN_LOG.md). Do not mutate implementation sources. \
             Mark proven items `- [x]`. The host re-verifies machine criteria before latching \
             GoalAchieved; Core structured verification is still required; prose and DONE are not \
             enough."
        }
    };
    format!(
        "Run this durable A3S `/goal` Loop Engineering iteration.\n\n\
         Exact goal: {goal}\n\
         Loop id: {id}\n\
         Iteration: {iteration}\n\
         {phase_contract}\n\
         Workspace: {cwd}\n\n\
         Read these files before acting:\n\
         - {config}\n\
         - {acceptance}\n\
         - {state}\n\
         - {log}\n\
         - {maker}\n\
         - {verifier}\n\n\
         Completion contract:\n\
         1. Work on the exact goal text above, not a smaller substitute. Inspect the current workspace first and preserve unrelated user changes.\n\
         2. Use the maker/verifier split as host-enforced phases. The maker may implement; the verifier must independently inspect and run fresh checks. Parallelize only genuinely independent work inside a phase; never run overlapping writes or maker and verifier concurrently.\n\
         3. Update ACCEPTANCE.md and STATE.md with concrete evidence, commands, outcomes, remaining gaps, and any genuine blocker. Append this iteration to RUN_LOG.md.\n\
         4. Do not treat a normal answer, the word DONE, a plan ending, or an iteration limit as goal completion. The host latches GoalAchieved only in the verifier phase after Core evaluates structured verification evidence, every ACCEPTANCE.md task item is marked proven (`- [x]`), and the host re-checks `kind:command` / `kind:file_exists` criteria; never fabricate that event, but never list the host event itself as a success criterion or remaining user work.\n\
         5. If the goal is not yet fully verified, make the maximum safe progress and report exact remaining work; the host will start another iteration automatically.\n\
         6. Never fabricate test results or completion evidence. When the underlying goal is proven, report the concrete evidence and say that no underlying work remains; the host owns the completion signal.\n\
         7. Treat parallelism as a bounded admission window, not a target. Prefer 2-4 focused read-only branches per wave. For evidence fan-out use allow_partial_failure=true, retain successful results, and retry only failed branches; never replay a completed or potentially mutating branch.\n\
         8. Reuse still-valid evidence already recorded in STATE.md `## Verified Evidence` (host fingerprints such as `fp:command:` / `fp:file:`). Skip re-scanning assertions whose fingerprints still match the workspace. Fingerprints never replace host latch re-checks of machine criteria. Spend this iteration on the highest-value unresolved gap.\n\n\
         Begin iteration {iteration} now.",
        goal = run.spec.goal,
        id = run.spec.id,
        iteration = run.iteration,
        cwd = run.spec.dir.parent().and_then(Path::parent).and_then(Path::parent).unwrap_or(Path::new(".")).display(),
        config = run.spec.dir.join(LOOP_CONFIG).display(),
        acceptance = run.spec.dir.join(ACCEPTANCE_FILE).display(),
        state = run.spec.dir.join(STATE_FILE).display(),
        log = run.spec.dir.join(RUN_LOG_FILE).display(),
        maker = run.spec.dir.join("skills").join("maker.md").display(),
        verifier = run.spec.dir.join("skills").join("verifier.md").display(),
    )
}

fn goal_continuation_prompt(run: &GoalRunState, failure: Option<&str>) -> String {
    let reason = failure
        .map(|failure| {
            format!(
                "The previous iteration failed before verification completed: {}\n\
                 Diagnose or work around that failure, then continue. An error is not completion.\n",
                failure.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        })
        .unwrap_or_else(|| {
            "The previous iteration ended without a matching GoalAchieved event. The goal is still active.\n"
                .to_string()
        });
    let phase_line = match run.phase {
        GoalPhase::Maker => {
            "Current phase: maker. Advance the highest-value remaining gap; completion cannot latch in this phase."
        }
        GoalPhase::Verifier => {
            "Current phase: verifier. Independently verify open acceptance criteria with fresh structured evidence."
        }
    };
    format!(
        "Continue durable `/goal` loop `{}` at iteration {}.\n\n\
         Exact goal: {}\n\
         {phase_line}\n\
         {reason}\n\
         Re-read {}, {}, and {}. Preserve verified work instead of repeating it: treat STATE.md `## Verified Evidence` fingerprints as skip-hints for unchanged assertions, but never treat them as latch substitutes. Identify the highest-value remaining gap, and respect the host maker/verifier phase. Keep read-only fan-out to 2-4 focused branches with allow_partial_failure=true; retain successful branches and never replay completed or potentially mutating work. Update the loop artifacts and continue until the exact goal is proven. Do not use DONE as a completion signal. GoalAchieved is latched by the host only in the verifier phase after structured verification evidence supports it: never fabricate it and never list that host event as a success criterion or remaining user work. If the underlying goal is proven, report the concrete evidence and state that no underlying work remains.",
        run.spec.id,
        run.iteration,
        run.spec.goal,
        run.spec.dir.join(ACCEPTANCE_FILE).display(),
        run.spec.dir.join(STATE_FILE).display(),
        run.spec.dir.join(RUN_LOG_FILE).display(),
    )
}

fn retry_delay(failures: usize) -> Duration {
    let exponent = failures.saturating_sub(1).min(5) as u32;
    Duration::from_secs((1u64 << exponent).min(30))
}

/// P3 failure-storm classification: provider errors back off; unmet criteria do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GoalStallClass {
    ProviderError,
    CriterionFail,
}

impl GoalStallClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::ProviderError => "provider_error",
            Self::CriterionFail => "criterion_fail",
        }
    }

    fn from_failure(failure: Option<&str>) -> Self {
        if failure.is_some() {
            Self::ProviderError
        } else {
            Self::CriterionFail
        }
    }

    fn should_backoff(self) -> bool {
        matches!(self, Self::ProviderError)
    }
}

/// Host continue schedule for the next `/goal` iteration (backoff vs immediate).
///
/// `continue_goal_run` must use this so provider storms sleep while unmet
/// criteria keep iterating without artificial delay.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GoalContinueSchedule {
    stall: GoalStallClass,
    runtime_status: &'static str,
    log_event: &'static str,
    delay: Option<Duration>,
}

fn schedule_goal_continue(failure: Option<&str>, failures: usize) -> GoalContinueSchedule {
    let stall = GoalStallClass::from_failure(failure);
    if stall.should_backoff() {
        GoalContinueSchedule {
            stall,
            runtime_status: "retrying",
            log_event: "retrying",
            delay: Some(retry_delay(failures)),
        }
    } else {
        GoalContinueSchedule {
            stall,
            runtime_status: "running",
            log_event: "continuing",
            delay: None,
        }
    }
}

impl App {
    /// Apply maker/verifier host posture: mode chrome + goal-verify fence + Core style.
    fn apply_goal_phase_posture(&mut self, phase: GoalPhase) {
        let loop_dir = self.goal_run.as_ref().map(|run| run.spec.dir.clone());
        match phase {
            GoalPhase::Maker => self.execution_policy.set_goal_verify(false, None),
            GoalPhase::Verifier => self.execution_policy.set_goal_verify(true, loop_dir),
        }
        self.set_composer_mode(composer_mode_for_goal_phase(phase));
        // Plan specialty denies bash; verifier clears specialty so reports can form.
        if let Err(error) = self
            .session
            .set_agent_style(agent_style_for_goal_phase(phase))
        {
            tracing::warn!(%error, "could not update Core agent style for goal phase");
        }
    }

    fn clear_goal_verify_posture(&mut self) {
        self.execution_policy.set_goal_verify(false, None);
    }

    pub(crate) fn start_goal_run(&mut self, raw_goal: &str) -> Option<Cmd<Msg>> {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  finish the current turn before starting a goal"),
            );
            return None;
        }
        // Replace must clear in-memory and on-disk active contracts so Core
        // ACCEPTANCE emit cannot OR against an orphaned sibling loop.
        if self.goal_run.is_some() {
            let _ = self.cancel_goal_state("replaced by a new /goal");
        }
        self.clear_paused_goal("replaced by a new /goal");
        cancel_orphaned_active_goal_loops(Path::new(&self.cwd), None, "replaced by a new /goal");
        let effective_goal = normalize_goal(raw_goal);
        let spec = match init_goal_loop(&self.cwd, &effective_goal) {
            Ok(spec) => spec,
            Err(error) => {
                self.push_line(&Style::new().fg(TN_RED).render(&format!(
                    "  /goal could not create Loop Engineering: {error}"
                )));
                return None;
            }
        };

        let previous_effort = self.effort;
        let previous_goal = self.goal.clone();
        let previous_goal_since = self.goal_since;
        self.goal_generation = self.goal_generation.wrapping_add(1).max(1);
        let run = GoalRunState::new(self.goal_generation, spec);
        self.goal = Some(run.spec.goal.clone());
        self.goal_since = Some(Instant::now());
        self.goal_run = Some(run);

        let mut profile = self.session_rebuild_profile();
        profile.effort = ULTRACODE;
        self.start_session_rebuild(
            profile,
            SessionRebuildAction::GoalStart {
                generation: self.goal_generation,
                previous_effort,
                previous_goal,
                previous_goal_since,
            },
        )
    }

    pub(crate) fn finish_goal_start(&mut self) -> Option<Cmd<Msg>> {
        self.effort = ULTRACODE;
        let phase = self
            .goal_run
            .as_ref()
            .map(|run| run.phase)
            .unwrap_or(GoalPhase::Maker);
        self.apply_goal_phase_posture(phase);
        self.autonomy_restore = None;
        self.loop_remaining = 0;
        self.gradient_until = Some(Instant::now());
        self.gradient_frame = 0;

        let run = self.goal_run.as_ref().expect("goal run initialized");
        let _ = persist_runtime_state(run, "running", "Ultracode goal run started");
        let _ = append_goal_log(run, "running", "Ultracode + forced planning enabled");
        let prompt = goal_run_prompt(run);
        let display = format!("◎ goal: {}", truncate(&run.spec.goal, 54));
        let id = run.spec.id.clone();
        let dir = run.spec.dir.display().to_string();
        self.push_line(&gutter(
            ACCENT,
            &format!("◎\u{200A}goal loop `{id}` · Ultracode · continues until verified"),
        ));
        self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
            "  Loop Engineering: {dir} · Esc or /goal clear cancels"
        )));
        self.start_stream_inner(prompt, display, true, true, false)
    }

    pub(crate) fn finish_goal_resume(&mut self) -> Option<Cmd<Msg>> {
        self.autonomy_restore = None;
        self.loop_remaining = 0;
        self.gradient_until = Some(Instant::now());
        self.gradient_frame = 0;

        let run = self.goal_run.as_mut().expect("goal run restored");
        run.begin_resumed_iteration();
        let phase = run.phase;
        let _ = persist_runtime_state(run, "running", "paused goal resumed by user");
        let _ = append_goal_log(run, "running", "paused goal resumed by user");
        let prompt = goal_continuation_prompt(run, None);
        let display = format!(
            "◎ resumed goal iteration {}: {}",
            run.iteration,
            truncate(&run.spec.goal, 48)
        );
        let id = run.spec.id.clone();
        self.apply_goal_phase_posture(phase);
        self.push_line(&gutter(
            ACCENT,
            &format!("◎\u{200A}resumed goal loop `{id}` · continues until verified"),
        ));
        self.start_stream_inner(prompt, display, true, true, false)
    }

    pub(crate) fn record_goal_extracted(&mut self, goal: &AgentGoal) {
        if let Some(run) = self.goal_run.as_mut() {
            run.record_extracted_goal(goal);
            let _ = persist_runtime_state(run, "running", "goal extracted; executing plan");
        }
    }

    pub(crate) fn record_goal_progress(&mut self, progress: f32) {
        if let Some(run) = self.goal_run.as_mut() {
            let previous_checkpoint = goal_progress_checkpoint(run.progress);
            run.record_progress(progress);
            if goal_progress_checkpoint(run.progress) != previous_checkpoint {
                let _ = persist_runtime_state(run, "running", "plan progress updated");
            }
        }
    }

    pub(crate) fn record_goal_achieved(&mut self, goal: &str) {
        if let Some(run) = self.goal_run.as_mut() {
            if let Some(reason) = run.achievement_reject_reason(goal) {
                let _ = persist_runtime_state(run, "running", &reason);
                let _ = append_goal_log(run, "blocked", &reason);
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!("  ◎ {reason}")));
                return;
            }
            if run.record_achievement(goal) {
                let _ = persist_runtime_state(run, "verified", "matching GoalAchieved received");
                let _ = append_goal_log(run, "verified", "matching GoalAchieved received");
            }
        }
    }

    pub(crate) fn continue_goal_run(&mut self, failure: Option<String>) -> Option<Cmd<Msg>> {
        if self.goal_run.as_ref().is_some_and(|run| run.achieved) {
            return self.finish_achieved_goal();
        }
        let run = self.goal_run.as_mut()?;
        let failed = failure.is_some();
        run.begin_next_iteration(failed);
        if run.should_pause_for_stall() {
            let paused = run.paused_state();
            let streak = run.unverified_streak;
            let _ = persist_runtime_state(
                run,
                "paused",
                &format!("stalled after {streak} unverified iterations"),
            );
            let _ = append_goal_log(
                run,
                "paused",
                &format!("stalled after {streak} unverified iterations"),
            );
            self.goal_run = None;
            self.goal = None;
            self.goal_since = None;
            self.paused_goal = Some(paused);
            self.loop_remaining = 0;
            self.clear_goal_verify_posture();
            self.push_line(&gutter(
                TN_YELLOW,
                &format!(
                    "◎\u{200A}goal paused after {streak} unverified iterations · /goal resume or /goal clear"
                ),
            ));
            return self.restore_goal_planning_mode();
        }
        let prompt = goal_continuation_prompt(run, failure.as_deref());
        let generation = run.generation;
        let iteration = run.iteration;
        let phase = run.phase;
        let schedule = schedule_goal_continue(failure.as_deref(), run.failures);
        let detail = match failure.as_deref() {
            Some(error) => format!("class={} · {error}", schedule.stall.as_str()),
            None => format!(
                "class={} · GoalAchieved not received; continuing",
                schedule.stall.as_str()
            ),
        };
        let _ = persist_runtime_state(run, schedule.runtime_status, &detail);
        let _ = append_goal_log(run, schedule.log_event, &detail);
        let suffix = schedule
            .delay
            .map(|delay| format!(" · retry in {}s", delay.as_secs()))
            .unwrap_or_default();
        self.apply_goal_phase_posture(phase);
        self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
            "  ↻ goal iteration {iteration} · {} · {} · verification still open{suffix} · Esc stops",
            phase.as_str(),
            schedule.stall.as_str()
        )));
        Some(match schedule.delay {
            Some(delay) => cmd::cmd(move || async move {
                tokio::time::sleep(delay).await;
                Msg::GoalContinue { generation, prompt }
            }),
            None => cmd::msg(Msg::GoalContinue { generation, prompt }),
        })
    }

    pub(crate) fn handle_goal_continue(
        &mut self,
        generation: u64,
        prompt: String,
    ) -> Option<Cmd<Msg>> {
        let run = self.goal_run.as_mut()?;
        if !run.is_generation(generation) || run.achieved || self.state != State::Idle {
            return None;
        }
        run.accepting_achievement = true;
        run.extracted_goal = None;
        let phase = run.phase;
        let display = format!(
            "◎ goal iteration {}: {}",
            run.iteration,
            truncate(&run.spec.goal, 48)
        );
        self.apply_goal_phase_posture(phase);
        self.start_stream_inner(prompt, display, true, false, false)
    }

    pub(crate) fn finish_achieved_goal(&mut self) -> Option<Cmd<Msg>> {
        let run = self.goal_run.take()?;
        if !run.achieved {
            self.goal_run = Some(run);
            return None;
        }
        self.clear_goal_verify_posture();
        let _ = persist_runtime_state(&run, "achieved", "goal completed and run closed");
        let _ = append_goal_log(&run, "achieved", "goal completed and run closed");
        self.goal = None;
        self.goal_since = None;
        self.loop_remaining = 0;
        self.push_line(&gutter(
            TN_GREEN,
            &format!(
                "◎\u{200A}goal achieved · {} iterations · {}",
                run.iteration,
                run.spec.dir.display()
            ),
        ));
        self.restore_goal_planning_mode()
    }

    pub(crate) fn cancel_goal_state(&mut self, reason: &str) -> bool {
        self.goal_generation = self.goal_generation.wrapping_add(1).max(1);
        let Some(run) = self.goal_run.take() else {
            return false;
        };
        self.clear_goal_verify_posture();
        let _ = persist_runtime_state(&run, "cancelled", reason);
        let _ = append_goal_log(&run, "cancelled", reason);
        self.goal = None;
        self.goal_since = None;
        self.loop_remaining = 0;
        true
    }

    pub(crate) fn restore_goal_planning_mode(&mut self) -> Option<Cmd<Msg>> {
        self.clear_goal_verify_posture();
        if self.state != State::Idle {
            return None;
        }
        let profile = self.session_rebuild_profile();
        self.start_session_rebuild(profile, SessionRebuildAction::GoalRestore)
    }

    pub(crate) fn clear_goal_command(&mut self) -> Option<Cmd<Msg>> {
        self.textarea.clear();
        let active = self.cancel_goal_state("cleared by user");
        let paused = self.clear_paused_goal("cleared by user");
        if !active {
            self.goal = None;
            self.goal_since = None;
            self.push_line(&Style::new().fg(TN_GRAY).render(if paused {
                "  paused goal cleared"
            } else {
                "  goal cleared"
            }));
            return None;
        }
        self.push_line(&Style::new().fg(TN_GRAY).render("  goal loop cleared"));
        if self.session_rebuild_pending.is_some() {
            // The atomic build cannot be force-aborted through the TEA command
            // handle. Its completion observes the invalidated generation and
            // immediately restores ordinary Ultracode planning instead of
            // starting the cancelled goal.
            return None;
        }
        if self.state == State::Idle {
            return self.restore_goal_planning_mode();
        }

        self.interrupting = true;
        let session = self.session.clone();
        let join = self.stream_join.take();
        let host_abort = self.host_tool_abort.take();
        Some(cmd::cmd(move || async move {
            if let Some(host_abort) = host_abort {
                host_abort.abort();
            }
            let _ = session
                .cancel_and_settle(
                    Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                )
                .await;
            if let Some(join) = join {
                let _ = settle_stream_join_for_quit(
                    join,
                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                )
                .await;
            }
            Msg::GoalCleared
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "a3s-goal-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn goal_loop_scaffolds_complete_loop_engineering_workspace() {
        let root = temp_root("scaffold");
        let spec = init_goal_loop(
            &root.to_string_lossy(),
            "Implement the feature and verify it end to end",
        )
        .unwrap();

        assert_eq!(spec.level, "G1");
        assert_eq!(spec.max_iterations_per_run, 0);
        assert!(spec.id.starts_with("goal-implement-the-feature"));
        for path in [
            spec.dir.join(LOOP_CONFIG),
            spec.dir.join(STATE_FILE),
            spec.dir.join(ACCEPTANCE_FILE),
            spec.dir.join(RUN_LOG_FILE),
            spec.dir.join(BUDGET_FILE),
            spec.dir.join("skills/maker.md"),
            spec.dir.join("skills/verifier.md"),
        ] {
            assert!(path.is_file(), "missing {}", path.display());
        }
        assert!(spec.dir.join("reports").is_dir());
        assert_eq!(loop_engineering::audit_loop(&spec).score, 100);
        let budget = std::fs::read_to_string(spec.dir.join(BUDGET_FILE)).unwrap();
        assert!(budget.contains("completion_gate = \"goal_achieved_event\""));
        assert!(budget.contains("max_unverified_streak"));
        assert!(budget.contains("advisory"));
        let acceptance = std::fs::read_to_string(spec.dir.join(ACCEPTANCE_FILE)).unwrap();
        assert!(acceptance.contains("- [ ]"));
        assert_eq!(acceptance_criteria_progress(&acceptance), (0, 1));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn start_goal_run_cancels_previous_loop_runtime_status() {
        let root = temp_root("orphan-cancel");
        let orphan = root
            .join(".a3s")
            .join("loops")
            .join("goal-orphan-left-running");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(
            orphan.join(STATE_FILE),
            "Status: running\nPhase: maker\nLast event: left behind\n",
        )
        .unwrap();
        let keep = root.join(".a3s").join("loops").join("goal-keep-me");
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(
            keep.join(STATE_FILE),
            "Status: running\nPhase: maker\nLast event: current\n",
        )
        .unwrap();
        let verified = root.join(".a3s").join("loops").join("goal-already-done");
        std::fs::create_dir_all(&verified).unwrap();
        std::fs::write(
            verified.join(STATE_FILE),
            "Status: verified\nPhase: verifier\nLast event: done\n",
        )
        .unwrap();

        cancel_orphaned_active_goal_loops(&root, Some("goal-keep-me"), "replaced by a new /goal");

        let orphan_state = std::fs::read_to_string(orphan.join(STATE_FILE)).unwrap();
        assert!(
            orphan_state.contains("Status: cancelled"),
            "orphaned running loop must be cancelled: {orphan_state}"
        );
        assert!(orphan_state.contains("replaced by a new /goal"));
        let keep_state = std::fs::read_to_string(keep.join(STATE_FILE)).unwrap();
        assert!(
            keep_state.contains("Status: running"),
            "keep_id must remain running: {keep_state}"
        );
        let verified_state = std::fs::read_to_string(verified.join(STATE_FILE)).unwrap();
        assert!(
            verified_state.contains("Status: verified"),
            "completed loops must stay verified: {verified_state}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn acceptance_criteria_progress_counts_only_task_items() {
        let body = "\
# Acceptance\n\
## Observable Criteria\n\
guidance text with [x] should not count\n\
- [ ] open one\n\
- [x] done one\n\
* [X] done two\n\
- plain bullet\n\
- [y] ignored\n";
        assert_eq!(acceptance_criteria_progress(body), (2, 3));
    }

    #[test]
    fn runtime_section_includes_acceptance_criteria_progress() {
        let root = temp_root("criteria-progress");
        let spec = init_goal_loop(&root.to_string_lossy(), "Track criteria progress").unwrap();
        let run = GoalRunState::new(12, spec);
        let section = runtime_section(&run, "running", "started");
        assert!(section.contains("Criteria: 0/1"), "{section}");
        let acceptance = run.spec.dir.join(ACCEPTANCE_FILE);
        std::fs::write(
            &acceptance,
            "# Acceptance\n\n- [x] kind:manual assert:Track criteria progress\n",
        )
        .unwrap();
        let section = runtime_section(&run, "running", "updated");
        assert!(section.contains("Criteria: 1/1"), "{section}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reopening_goal_loop_preserves_agent_work_notes() {
        let root = temp_root("resume");
        let cwd = root.to_string_lossy();
        let spec = init_goal_loop(&cwd, "Keep this exact goal active").unwrap();
        let state = spec.dir.join(STATE_FILE);
        let mut body = std::fs::read_to_string(&state).unwrap();
        body.push_str("\n## Work Notes\n\n- Preserve this evidence.\n");
        std::fs::write(&state, body).unwrap();

        let reopened = init_goal_loop(&cwd, "Keep this exact goal active").unwrap();

        assert_eq!(reopened.id, spec.id);
        assert!(std::fs::read_to_string(state)
            .unwrap()
            .contains("Preserve this evidence"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn paused_goal_restores_state_and_resumes_at_the_next_iteration() {
        let root = temp_root("paused-resume");
        let cwd = root.to_string_lossy();
        let spec = init_goal_loop(&cwd, "Keep this resumable goal active").unwrap();
        let mut original = GoalRunState::new(7, spec);
        original.iteration = 4;
        original.progress = 0.65;
        original.failures = 2;
        let paused = original.paused_state();

        let mut restored = GoalRunState::from_paused(&cwd, 8, &paused).unwrap();

        assert_eq!(restored.generation, 8);
        assert_eq!(restored.spec.id, paused.loop_id);
        assert_eq!(restored.spec.goal, paused.goal);
        assert_eq!(restored.iteration, 4);
        assert_eq!(restored.progress, 0.65);
        assert_eq!(restored.failures, 2);
        assert!(!restored.accepting_achievement);

        restored.begin_resumed_iteration();
        assert_eq!(restored.iteration, 5);
        assert_eq!(restored.progress, 0.0);
        assert!(restored.accepting_achievement);
        let _ = std::fs::remove_dir_all(root);
    }

    fn mark_all_acceptance_criteria(run: &GoalRunState) {
        let path = run.spec.dir.join(ACCEPTANCE_FILE);
        let body = std::fs::read_to_string(&path).unwrap();
        let updated = body
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                if trimmed.starts_with("- [ ]") || trimmed.starts_with("* [ ]") {
                    line.replacen("[ ]", "[x]", 1)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, updated).unwrap();
    }

    /// Durable latch requires a checked machine criterion — not manuals alone.
    fn write_passing_machine_acceptance(run: &GoalRunState) {
        std::fs::write(
            run.spec.dir.join(ACCEPTANCE_FILE),
            "# Acceptance\n\n\
             - [x] kind:command assert:`true` expect:exit=0\n\
             - [x] kind:manual assert:reviewed\n",
        )
        .unwrap();
    }

    #[test]
    fn only_the_current_extracted_goal_can_latch_achievement() {
        let root = temp_root("latch");
        let spec = init_goal_loop(&root.to_string_lossy(), "Verify the exact target").unwrap();
        let mut run = GoalRunState::new(7, spec);
        run.phase = GoalPhase::Verifier;
        let goal = AgentGoal::new("Verify the exact target");
        run.record_extracted_goal(&goal);

        assert!(!run.record_achievement("Different target"));
        assert!(!run.achieved);
        assert!(
            !run.record_achievement("Verify the exact target"),
            "open ACCEPTANCE criteria must block latch"
        );
        write_passing_machine_acceptance(&run);
        assert!(run.record_achievement("Verify the exact target"));
        assert!(run.achieved);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn open_acceptance_criteria_block_verifier_latch() {
        let root = temp_root("open-acceptance");
        let spec = init_goal_loop(&root.to_string_lossy(), "Require checked criteria").unwrap();
        let mut run = GoalRunState::new(15, spec);
        run.phase = GoalPhase::Verifier;
        run.record_extracted_goal(&AgentGoal::new("Require checked criteria"));
        let reason = run
            .achievement_reject_reason("Require checked criteria")
            .expect("open criteria should explain the block");
        assert!(reason.contains("ACCEPTANCE"), "{reason}");
        assert!(!run.record_achievement("Require checked criteria"));
        write_passing_machine_acceptance(&run);
        assert!(run
            .achievement_reject_reason("Require checked criteria")
            .is_none());
        assert!(run.record_achievement("Require checked criteria"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn manual_only_acceptance_cannot_latch_even_when_checked() {
        let root = temp_root("manual-only");
        let spec = init_goal_loop(&root.to_string_lossy(), "Manual alone is not enough").unwrap();
        let mut run = GoalRunState::new(17, spec);
        run.phase = GoalPhase::Verifier;
        run.record_extracted_goal(&AgentGoal::new("Manual alone is not enough"));
        mark_all_acceptance_criteria(&run);
        let reason = run
            .achievement_reject_reason("Manual alone is not enough")
            .expect("manual-only must not latch");
        assert!(
            reason.contains("kind:command")
                || reason.contains("kind:file_exists")
                || reason.contains("manual-only"),
            "{reason}"
        );
        assert!(!run.record_achievement("Manual alone is not enough"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failing_command_criterion_blocks_verifier_latch_after_checkbox() {
        let root = temp_root("cmd-fail");
        let spec = init_goal_loop(&root.to_string_lossy(), "Command must pass").unwrap();
        let mut run = GoalRunState::new(21, spec);
        run.phase = GoalPhase::Verifier;
        run.record_extracted_goal(&AgentGoal::new("Command must pass"));
        let acceptance = run.spec.dir.join(ACCEPTANCE_FILE);
        std::fs::write(
            &acceptance,
            "# Acceptance\n\n- [x] kind:command assert:`false` expect:exit=0\n",
        )
        .unwrap();
        let reason = run
            .achievement_reject_reason("Command must pass")
            .expect("failing command should block");
        assert!(
            reason.contains("re-check") || reason.contains("command"),
            "{reason}"
        );
        assert!(!run.record_achievement("Command must pass"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn passing_command_and_file_criteria_allow_verifier_latch() {
        let root = temp_root("cmd-ok");
        std::fs::write(root.join("ok.txt"), "present").unwrap();
        let spec = init_goal_loop(&root.to_string_lossy(), "Command and file must pass").unwrap();
        let mut run = GoalRunState::new(22, spec);
        run.phase = GoalPhase::Verifier;
        run.record_extracted_goal(&AgentGoal::new("Command and file must pass"));
        let acceptance = run.spec.dir.join(ACCEPTANCE_FILE);
        std::fs::write(
            &acceptance,
            "# Acceptance\n\n\
             - [x] kind:command assert:`true` expect:exit=0\n\
             - [x] kind:file_exists assert:ok.txt\n\
             - [x] kind:manual assert:reviewed\n",
        )
        .unwrap();
        assert!(run
            .achievement_reject_reason("Command and file must pass")
            .is_none());
        assert!(run.record_achievement("Command and file must pass"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persist_records_evidence_fingerprints_but_stale_fp_cannot_skip_latch_recheck() {
        let root = temp_root("fp-latch");
        std::fs::write(root.join("ok.txt"), "present").unwrap();
        let spec = init_goal_loop(&root.to_string_lossy(), "Fingerprints are skip hints").unwrap();
        let mut run = GoalRunState::new(23, spec);
        run.phase = GoalPhase::Verifier;
        run.record_extracted_goal(&AgentGoal::new("Fingerprints are skip hints"));
        let acceptance = run.spec.dir.join(ACCEPTANCE_FILE);
        std::fs::write(
            &acceptance,
            "# Acceptance\n\n\
             - [x] kind:command assert:`true` expect:exit=0\n\
             - [x] kind:file_exists assert:ok.txt\n",
        )
        .unwrap();
        persist_runtime_state(&run, "running", "fingerprint refresh").unwrap();
        let state = std::fs::read_to_string(run.spec.dir.join(STATE_FILE)).unwrap();
        assert!(
            state.contains("fp:command:") && state.contains("fp:file:"),
            "expected host fingerprints in Verified Evidence, got:\n{state}"
        );
        assert!(
            state.contains("do **not** replace host latch")
                || state.contains("not** replace host latch")
                || state.contains("never replace")
                || state.contains("do not replace")
                || state.contains("**not** replace"),
            "{state}"
        );

        // Stale / optimistic fingerprints in STATE must not weaken the latch.
        std::fs::write(
            run.spec.dir.join(STATE_FILE),
            "# Goal\n\n## Verified Evidence\n\n\
             - `fp:command:sha256=deadbeef:expect=0:ok`\n\
             - `fp:file:path=ok.txt:len=7:mtime_ns=1`\n",
        )
        .unwrap();
        std::fs::write(
            &acceptance,
            "# Acceptance\n\n- [x] kind:command assert:`false` expect:exit=0\n",
        )
        .unwrap();
        let reason = run
            .achievement_reject_reason("Fingerprints are skip hints")
            .expect("failing command must still block despite stale fingerprints");
        assert!(
            reason.contains("re-check") || reason.contains("command"),
            "{reason}"
        );
        assert!(!run.record_achievement("Fingerprints are skip hints"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn maker_phase_cannot_latch_achievement() {
        let root = temp_root("maker-no-latch");
        let spec = init_goal_loop(&root.to_string_lossy(), "Keep maker open").unwrap();
        let mut run = GoalRunState::new(9, spec);
        assert_eq!(run.phase, GoalPhase::Maker);
        run.record_extracted_goal(&AgentGoal::new("Keep maker open"));
        assert!(!run.record_achievement("Keep maker open"));
        assert!(!run.achieved);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn divergent_extracted_goal_cannot_latch_user_goal() {
        let root = temp_root("divergent-extract");
        let spec = init_goal_loop(&root.to_string_lossy(), "Ship the exact feature").unwrap();
        let mut run = GoalRunState::new(10, spec);
        run.phase = GoalPhase::Verifier;
        run.record_extracted_goal(&AgentGoal::new("Ship a smaller substitute"));
        assert!(run.extracted_goal.is_none());
        assert!(!run.record_achievement("Ship the exact feature"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn user_turn_goal_events_cannot_close_the_host_goal_iteration() {
        let root = temp_root("user-turn");
        let spec = init_goal_loop(&root.to_string_lossy(), "Keep the host goal open").unwrap();
        let mut run = GoalRunState::new(8, spec);
        run.phase = GoalPhase::Verifier;
        run.pause_achievement_for_user_turn();
        let unrelated = AgentGoal::new("Answer the queued side request");
        run.record_extracted_goal(&unrelated);

        assert!(!run.record_achievement("Answer the queued side request"));
        assert!(!run.achieved);
        run.begin_next_iteration(false);
        assert_eq!(run.phase, GoalPhase::Maker);
        run.begin_next_iteration(false);
        assert_eq!(run.phase, GoalPhase::Verifier);
        let actual = AgentGoal::new("Keep the host goal open");
        run.record_extracted_goal(&actual);
        write_passing_machine_acceptance(&run);
        assert!(run.record_achievement("Keep the host goal open"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn goal_iterations_have_no_fixed_completion_cap() {
        let root = temp_root("iterations");
        let spec = init_goal_loop(&root.to_string_lossy(), "Continue until verified").unwrap();
        let mut run = GoalRunState::new(11, spec);

        for _ in 0..1_000 {
            run.begin_next_iteration(false);
        }

        assert_eq!(run.iteration, 1_001);
        assert!(!run.achieved);
        assert!(run.is_generation(11));
        assert!(!run.is_generation(12));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unverified_first_iteration_continues_and_second_can_complete() {
        let root = temp_root("two-iterations");
        let spec =
            init_goal_loop(&root.to_string_lossy(), "Finish only after verification").unwrap();
        let mut run = GoalRunState::new(21, spec);
        let first = AgentGoal::new("Finish only after verification");
        run.record_extracted_goal(&first);
        run.record_progress(1.0);

        assert!(!run.achieved, "plan completion is not goal completion");
        assert_eq!(run.phase, GoalPhase::Maker);
        assert!(!run.record_achievement("Finish only after verification"));
        run.begin_next_iteration(false);
        assert_eq!(run.iteration, 2);
        assert_eq!(run.phase, GoalPhase::Verifier);
        assert!(!run.achieved);

        let second = AgentGoal::new("Finish only after verification");
        run.record_extracted_goal(&second);
        write_passing_machine_acceptance(&run);
        assert!(run.record_achievement("Finish only after verification"));
        assert!(run.achieved);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unverified_streak_pauses_instead_of_unbounded_retry() {
        let root = temp_root("stall");
        let spec = init_goal_loop(&root.to_string_lossy(), "Stall after empty waves").unwrap();
        let mut run = GoalRunState::new(33, spec);
        for _ in 0..(MAX_UNVERIFIED_STREAK - 1) {
            run.begin_next_iteration(false);
            assert!(!run.should_pause_for_stall());
        }
        run.begin_next_iteration(false);
        assert!(run.should_pause_for_stall());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn verifier_phase_forces_plan_mode_and_maker_keeps_auto() {
        assert_eq!(composer_mode_for_goal_phase(GoalPhase::Maker), Mode::Auto);
        assert_eq!(
            composer_mode_for_goal_phase(GoalPhase::Verifier),
            Mode::Plan
        );
        assert!(agent_style_for_goal_phase(GoalPhase::Maker).is_none());
        assert!(
            agent_style_for_goal_phase(GoalPhase::Verifier).is_none(),
            "verifier must clear Plan specialty so Core can collect verification_reports via bash"
        );
    }

    #[test]
    fn criteria_progress_is_primary_completion_meter_not_plan_percent() {
        let root = temp_root("criteria-meter");
        let spec = init_goal_loop(&root.to_string_lossy(), "Meter criteria").unwrap();
        let mut run = GoalRunState::new(3, spec);
        run.record_progress(1.0);
        assert_eq!(run.progress, 1.0, "plan progress may still be recorded");
        assert_eq!(run.criteria_progress(), (0, 1), "open ACCEPTANCE stays 0/1");
        let section = runtime_section(&run, "running", "plan complete");
        assert!(section.contains("Criteria: 0/1"), "{section}");
        assert!(
            section.contains("Plan progress (secondary, not completion): 100%"),
            "{section}"
        );
        std::fs::write(
            run.spec.dir.join(ACCEPTANCE_FILE),
            "# Acceptance\n\n\
             - [x] kind:manual assert:one\n\
             - [ ] kind:manual assert:two\n",
        )
        .unwrap();
        assert_eq!(run.criteria_progress(), (1, 2));
        let section = runtime_section(&run, "running", "half proven");
        assert!(section.contains("Criteria: 1/2"), "{section}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn goal_parallel_admission_clamps_ultracode_budget_to_plan_window() {
        use crate::budget::{EFFORT_LEVELS, ULTRACODE_INDEX as ULTRACODE};
        assert_eq!(GOAL_MAX_PARALLEL_TASKS, 4);
        assert_eq!(goal_capped_parallel_tasks(8), 4);
        assert_eq!(goal_capped_parallel_tasks(4), 4);
        assert_eq!(goal_capped_parallel_tasks(2), 2);
        assert_eq!(goal_capped_parallel_tasks(0), 1);
        assert!(
            EFFORT_LEVELS[ULTRACODE].max_parallel_tasks > GOAL_MAX_PARALLEL_TASKS,
            "clamp is only meaningful when Ultracode asks for more than the goal ceiling"
        );
    }

    #[test]
    fn goal_session_parallel_admission_clamps_only_when_goal_active() {
        use crate::budget::{EFFORT_LEVELS, ULTRACODE_INDEX as ULTRACODE};
        let ultra = EFFORT_LEVELS[ULTRACODE].max_parallel_tasks;
        assert!(ultra > GOAL_MAX_PARALLEL_TASKS);
        assert_eq!(
            goal_session_max_parallel_tasks(true, ultra),
            GOAL_MAX_PARALLEL_TASKS,
            "session_options_for_profile must pass goal_run.is_some() here"
        );
        assert_eq!(
            goal_session_max_parallel_tasks(false, ultra),
            ultra,
            "ordinary Ultracode must keep the wider interactive budget"
        );
        assert_eq!(goal_session_max_parallel_tasks(true, 2), 2);
    }

    #[test]
    fn goal_prompt_uses_event_gate_instead_of_done_text() {
        let root = temp_root("prompt");
        let spec = init_goal_loop(&root.to_string_lossy(), "Prove the release is ready").unwrap();
        let mut run = GoalRunState::new(1, spec);
        let prompt = goal_run_prompt(&run);

        assert!(prompt.contains("Ultracode") || prompt.contains("`/goal`"));
        assert!(prompt.contains("host latches GoalAchieved only in the verifier phase"));
        assert!(prompt.contains("every ACCEPTANCE.md task item is marked proven"));
        assert!(prompt.contains("re-checks") || prompt.contains("re-check"));
        run.phase = GoalPhase::Verifier;
        let verifier = goal_run_prompt(&run);
        assert!(
            verifier.contains("goal-verify") || verifier.contains("verification"),
            "{verifier}"
        );
        assert!(
            verifier.contains("Plan chrome") || verifier.contains("loop directory"),
            "{verifier}"
        );
        assert!(prompt.contains("never list the host event itself as a success criterion"));
        assert!(prompt.contains("maker/verifier"));
        assert!(prompt.contains("word DONE"));
        assert!(prompt.contains("Phase: maker"));
        assert!(prompt.contains("ACCEPTANCE.md") || prompt.contains("ACCEPTANCE"));
        assert!(prompt.contains("2-4 focused read-only branches"));
        assert!(prompt.contains("allow_partial_failure=true"));
        assert!(
            prompt.contains("Verified Evidence") && prompt.contains("fp:command:"),
            "{prompt}"
        );
        let continuation = goal_continuation_prompt(&run, None);
        assert!(continuation.contains("structured verification evidence"));
        assert!(continuation.contains("never list that host event as a success criterion"));
        assert!(continuation.contains("Preserve verified work instead of repeating it"));
        assert!(
            continuation.contains("Verified Evidence")
                && continuation.contains("latch substitutes"),
            "{continuation}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn goal_progress_persistence_is_bounded_to_five_percent_checkpoints() {
        assert_eq!(goal_progress_checkpoint(0.0), 0);
        assert_eq!(goal_progress_checkpoint(0.049), 0);
        assert_eq!(goal_progress_checkpoint(0.051), 5);
        assert_eq!(goal_progress_checkpoint(0.10), 10);
        assert_eq!(goal_progress_checkpoint(0.994), 95);
        assert_eq!(goal_progress_checkpoint(1.0), 100);
    }

    #[test]
    fn unchanged_runtime_document_does_not_require_another_write() {
        let root = temp_root("runtime-dedup");
        let spec = init_goal_loop(&root.to_string_lossy(), "Keep runtime writes bounded").unwrap();
        let run = GoalRunState::new(31, spec);
        let replacement = runtime_section(&run, "running", "plan progress updated");
        let initial = runtime_document_with_section(String::new(), &run.spec.id, &replacement);
        let repeated = runtime_document_with_section(initial.clone(), &run.spec.id, &replacement);

        assert_eq!(repeated, initial);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn next_goal_iteration_resets_plan_progress_without_losing_failure_history() {
        let root = temp_root("progress-reset");
        let spec = init_goal_loop(&root.to_string_lossy(), "Reset each plan wave").unwrap();
        let mut run = GoalRunState::new(41, spec);
        run.record_progress(0.85);

        run.begin_next_iteration(true);

        assert_eq!(run.iteration, 2);
        assert_eq!(run.progress, 0.0);
        assert_eq!(run.failures, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn goal_error_retries_back_off_without_becoming_terminal() {
        assert_eq!(retry_delay(1), Duration::from_secs(1));
        assert_eq!(retry_delay(2), Duration::from_secs(2));
        assert_eq!(retry_delay(6), Duration::from_secs(30));
        assert_eq!(retry_delay(100), Duration::from_secs(30));
    }

    #[test]
    fn provider_error_backs_off_criterion_fail_does_not() {
        let provider = GoalStallClass::from_failure(Some("connection reset"));
        let criterion = GoalStallClass::from_failure(None);
        assert_eq!(provider.as_str(), "provider_error");
        assert_eq!(criterion.as_str(), "criterion_fail");
        assert!(provider.should_backoff());
        assert!(!criterion.should_backoff());
    }

    #[test]
    fn continue_schedule_backs_off_provider_errors_not_criterion_misses() {
        let provider = schedule_goal_continue(Some("connection reset"), 2);
        assert_eq!(provider.stall.as_str(), "provider_error");
        assert_eq!(provider.runtime_status, "retrying");
        assert_eq!(provider.log_event, "retrying");
        assert_eq!(provider.delay, Some(Duration::from_secs(2)));

        let criterion = schedule_goal_continue(None, 2);
        assert_eq!(criterion.stall.as_str(), "criterion_fail");
        assert_eq!(criterion.runtime_status, "running");
        assert_eq!(criterion.log_event, "continuing");
        assert!(
            criterion.delay.is_none(),
            "unmet criteria must continue immediately without backoff sleep"
        );
    }

    #[test]
    fn continue_schedule_persists_stall_class_into_runtime_state() {
        let root = temp_root("stall-persist");
        let spec =
            init_goal_loop(&root.to_string_lossy(), "Prove stall class persistence").unwrap();
        let run = GoalRunState::new(55, spec);

        let provider = schedule_goal_continue(Some("connection reset"), 1);
        let detail = format!("class={} · connection reset", provider.stall.as_str());
        persist_runtime_state(&run, provider.runtime_status, &detail).unwrap();
        let state = std::fs::read_to_string(run.spec.dir.join(STATE_FILE)).unwrap();
        assert!(state.contains("Status: retrying"), "{state}");
        assert!(state.contains("class=provider_error"), "{state}");

        let criterion = schedule_goal_continue(None, 1);
        let detail = format!(
            "class={} · GoalAchieved not received; continuing",
            criterion.stall.as_str()
        );
        persist_runtime_state(&run, criterion.runtime_status, &detail).unwrap();
        let state = std::fs::read_to_string(run.spec.dir.join(STATE_FILE)).unwrap();
        assert!(state.contains("Status: running"), "{state}");
        assert!(state.contains("class=criterion_fail"), "{state}");

        let _ = std::fs::remove_dir_all(root);
    }
}
