
const DEFAULT_PATTERN: &str = "daily-triage";
pub(super) const LOOP_CONFIG: &str = "loop.toml";
pub(super) const STATE_FILE: &str = "STATE.md";
pub(super) const RUN_LOG_FILE: &str = "RUN_LOG.md";

fn loop_line(rendered: &str, width: usize) -> String {
    if width == 0 {
        String::new()
    } else {
        pad_to(&truncate(rendered, width), width)
    }
}

fn loop_columns(width: usize) -> (usize, usize) {
    if width == 0 {
        return (0, 0);
    }
    let left = if width < 64 {
        width.saturating_sub(1) / 2
    } else {
        (width / 2).clamp(34, 58).min(width.saturating_sub(28))
    };
    let right = width.saturating_sub(left + 1);
    (left, right)
}

fn loop_header_lines(has_loops: bool, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let mut header = SectionHeader::new("loop engineering")
        .show_separator(false)
        .indent(2)
        .title_color(TN_CYAN)
        .metadata_color(TN_GRAY)
        .muted_color(TN_GRAY)
        .metadata("Enter/r run · a audit · l logs · p runtime · i init · Esc");
    if !has_loops {
        header = header.muted("no loops · press i or run /loop init daily-triage");
    }

    header
        .view(
            width.min(u16::MAX as usize) as u16,
            if has_loops { 2 } else { 3 },
        )
        .lines()
        .map(str::to_string)
        .collect()
}

fn loop_detail_lines(selected: Option<&LoopSummary>, note: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let mut lines: Vec<String> = DetailPanel::without_title()
        .show_separator(false)
        .indent(2)
        .muted_color(TN_GRAY)
        .row(DetailRow::muted(note))
        .view(width.min(u16::MAX as usize) as u16, 1)
        .lines()
        .map(str::to_string)
        .collect();
    lines.push(String::new());

    if let Some(summary) = selected {
        let runtime = if summary.spec.os_runtime {
            "Runtime + RemoteUI enabled"
        } else {
            "disabled"
        };
        let render_width = width.min(u16::MAX as usize) as u16;
        lines.extend(
            SectionHeader::new(summary.spec.id.clone())
                .show_separator(false)
                .indent(2)
                .title_color(TN_FG)
                .metadata_color(TN_GRAY)
                .muted_color(TN_GRAY)
                .view(render_width, 1)
                .lines()
                .map(str::to_string),
        );
        let detail = KeyValue::new()
            .indent(2)
            .key_width(8)
            .separator(" ")
            .key_color(TN_GRAY)
            .value_color(TN_GRAY)
            .pair("pattern", summary.spec.pattern.clone())
            .pair(
                "level",
                format!("{} · cadence: {}", summary.spec.level, summary.spec.cadence),
            )
            .pair(
                "score",
                format!("{} · {}", summary.audit.score, summary.audit.level),
            )
            .pair("OS", runtime)
            .pair("goal", summary.spec.goal.clone())
            .pair("dir", summary.spec.dir.display().to_string());
        lines.extend(detail.lines(render_width));
        lines.push(String::new());

        let mut missing = DetailPanel::new("Missing")
            .show_separator(false)
            .indent(2)
            .title_color(TN_YELLOW)
            .value_color(TN_YELLOW)
            .muted_color(TN_GREEN)
            .unlimited_rows();
        if summary.audit.missing.is_empty() {
            missing = missing.row(DetailRow::muted("none"));
        } else {
            for item in summary.audit.missing.iter().take(6) {
                missing = missing.row(DetailRow::text(format!("- {item}")).color(TN_YELLOW));
            }
        }
        let missing_height = summary.audit.missing.len().min(6).saturating_add(1).max(2);
        lines.extend(
            missing
                .view(width.min(u16::MAX as usize) as u16, missing_height)
                .lines()
                .map(str::to_string),
        );
    } else {
        lines.extend(
            DetailPanel::without_title()
                .show_separator(false)
                .indent(2)
                .muted_color(TN_GRAY)
                .row(DetailRow::muted(
                    "/loop init daily-triage creates the first loop",
                ))
                .view(width.min(u16::MAX as usize) as u16, 1)
                .lines()
                .map(str::to_string),
        );
    }

    lines
}
pub(super) const BUDGET_FILE: &str = "budget.toml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoopSpec {
    pub(crate) id: String,
    pub(crate) pattern: String,
    pub(crate) goal: String,
    pub(crate) level: String,
    pub(crate) cadence: String,
    pub(crate) os_runtime: bool,
    pub(crate) worktree: bool,
    pub(crate) maker_agent: String,
    pub(crate) checker_agent: String,
    pub(crate) budget_tokens_per_day: u64,
    pub(crate) max_iterations_per_run: usize,
    pub(crate) denylist: Vec<String>,
    pub(crate) connectors: Vec<String>,
    pub(crate) dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoopAudit {
    pub(crate) score: u8,
    pub(crate) level: String,
    pub(crate) passed: Vec<String>,
    pub(crate) missing: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoopSummary {
    pub(crate) spec: LoopSpec,
    pub(crate) audit: LoopAudit,
    pub(crate) last_run: String,
}

pub(crate) struct LoopPanel {
    pub(crate) loops: Vec<LoopSummary>,
    pub(crate) sel: usize,
    pub(crate) note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoopCommand {
    Dashboard,
    Init(String),
    Run(String),
    Audit(String),
    Logs(String),
    Quick(String),
    Usage(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoopRuntimeMode {
    OsAvailable,
    LocalNoOs,
    LocalAgentDev,
}

/// Build the transient maker/checker contract used by DeepResearch. Persistent
/// `/loop` specs and transient research runs intentionally share the same loop
/// engineering vocabulary; DeepResearch stores its live state in the research
/// event journal instead of creating another user-managed loop directory.
pub(crate) fn deep_research_loop_contract(
    query: &str,
    current_date: &str,
    evidence_scope: &str,
    max_parallel_tasks: usize,
    max_child_steps: usize,
) -> serde_json::Value {
    let track_schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": { "type": "string", "maxLength": 64 },
            "focus": { "type": "string", "maxLength": 480 }
        },
        "required": ["title", "focus"]
    });
    let plan_schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "answer_shape": {
                "type": "string",
                "enum": ["lookup", "briefing", "investigation"]
            },
            "report_title": { "type": "string", "maxLength": 100 },
            "freshness_required": { "type": "boolean" },
            "workspace_evidence_required": { "type": "boolean" },
            "execution_route": {
                "type": "string",
                "enum": ["direct_only", "direct_then_review", "maker_first"]
            },
            "phases": {
                "type": "array",
                "maxItems": 3,
                "items": { "type": "string", "maxLength": 100 }
            },
            "tracks": {
                "type": "array",
                "maxItems": max_parallel_tasks.clamp(1, 4),
                "items": { "type": "string", "maxLength": 140 }
            },
            "search_queries": {
                "type": "array",
                "maxItems": 4,
                "items": { "type": "string", "maxLength": 180 }
            },
            "seed_urls": {
                "type": "array",
                "maxItems": 3,
                "items": { "type": "string", "maxLength": 500 }
            },
            "budget": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "retrieval_timeout_secs": { "type": "integer", "minimum": 30, "maximum": 150 },
                    "synthesis_timeout_secs": { "type": "integer", "minimum": 45, "maximum": 90 },
                    "max_iterations": { "type": "integer", "minimum": 1, "maximum": 4 },
                    "max_parallel_tasks": { "type": "integer", "minimum": 1, "maximum": max_parallel_tasks.max(1) },
                    "max_steps_per_task": { "type": "integer", "minimum": 1, "maximum": max_child_steps.clamp(1, 2) },
                    "direct_searches": { "type": "integer", "minimum": 0, "maximum": 4 },
                    "direct_fetches": { "type": "integer", "minimum": 0, "maximum": 8 }
                },
                "required": [
                    "retrieval_timeout_secs", "synthesis_timeout_secs", "max_iterations",
                    "max_parallel_tasks", "max_steps_per_task", "direct_searches",
                    "direct_fetches"
                ]
            },
            "stop_conditions": {
                "type": "array",
                "maxItems": 3,
                "items": { "type": "string", "maxLength": 100 }
            }
        },
        "required": [
            "answer_shape", "report_title", "freshness_required",
            "workspace_evidence_required", "execution_route", "phases", "tracks",
            "search_queries", "seed_urls", "budget", "stop_conditions"
        ]
    });
    let checker_schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "decision": { "type": "string", "enum": ["finalize", "continue", "degrade"] },
            "coverage_summary": { "type": "string", "maxLength": 800 },
            "report_summary": { "type": "string", "maxLength": 1200 },
            "verified_findings": {
                "type": "array",
                "maxItems": 5,
                "items": { "type": "string", "maxLength": 500 }
            },
            "unresolved_gaps": {
                "type": "array",
                "maxItems": 4,
                "items": { "type": "string", "maxLength": 400 }
            },
            "contradictions": {
                "type": "array",
                "maxItems": 3,
                "items": { "type": "string", "maxLength": 400 }
            },
            "next_action": {
                "type": "string",
                "enum": ["none", "direct_retrieval", "maker"]
            },
            "search_queries": {
                "type": "array",
                "maxItems": 3,
                "items": { "type": "string", "maxLength": 300 }
            },
            "seed_urls": {
                "type": "array",
                "maxItems": 4,
                "items": { "type": "string", "maxLength": 1000 }
            },
            "next_tracks": {
                "type": "array",
                "maxItems": max_parallel_tasks.clamp(1, 4),
                "items": track_schema
            },
            "reason": { "type": "string", "maxLength": 500 }
        },
        "required": [
            "decision", "coverage_summary", "report_summary", "verified_findings",
            "unresolved_gaps", "next_action"
        ]
    });
    let planner_prompt = format!(
        "Plan this DeepResearch run semantically; return only the required object. Never infer stages, depth, route, or budget from keyword counts, query length, answer shape, track count, or a task-specific template.\n\nQuery: {query}\nDate: {current_date}\nEvidence scope: {evidence_scope}\n\nWrite a concise reader-facing report_title and choose the smallest useful set of phases and independent evidence tracks. Keep each phase and track as one execution objective; do not repeat the query or add prose rationales. Choose execution_route semantically: direct_only for a bounded question that direct search/fetch plus an independent checker can answer; direct_then_review when bounded multi-query web retrieval can gather the evidence and one structured review can both synthesize it across the planned tracks and check coverage; maker_first only when useful initial evidence requires workspace inspection, evidence production, or multi-step tool work that direct retrieval cannot establish. A substantial public-source investigation normally needs direct_then_review, while a narrow lookup normally needs direct_only, but make the decision from the requested work rather than labels or surface features. Evidence scope describes available tools, not required tracks. Set workspace_evidence_required only when the query explicitly asks about this repository, a local codebase, or attached/local artifacts; general product, technology, migration, or deployment research is web evidence even when a workspace happens to be available. Search queries must each target one evidence question. Include only seed URLs you confidently know. Make the plan internally executable: allocate enough direct searches and fetches for every consequential public evidence track and observable stop condition to receive a real retrieval opportunity within the hard caps; do not create a multi-track investigation whose own retrieval budget can sample only one track. Set independent retrieval and synthesis clocks plus checked iteration, parallelism, and per-task evidence depth; the execution runtime separately gives each maker enough wall-clock time for tool selection, retrieval, and schema finalization. Stop conditions must be observable. The checker decides sufficiency after the planned route has run. Do not expose reasoning."
    );

    serde_json::json!({
        "pattern": "adaptive-deep-research",
        "goal": query,
        "maker_role": "evidence-researcher",
        "checker_role": "evidence-coverage-checker",
        "planner": {
            "agent": "loop-planner",
            "description": "Plan research",
            "max_steps": 1,
            // v5.2.2 streams and repairs structured output inside this single
            // call. Keep planning independent from retrieval while allowing
            // reasoning models to finish the schema-valid plan they already
            // started instead of cancelling them at the old workflow deadline.
            "timeout_ms": 120000,
            "prompt": planner_prompt,
            "output_schema": plan_schema
        },
        "checker": {
            "agent": "loop-checker",
            "description": "Check evidence",
            "max_steps": 2,
            // Optional routing fields keep a first-pass checker decision valid
            // across providers that omit empty arrays. The workflow normalizes
            // those fields and never needs a second model call just to repair
            // syntactic boilerplate.
            // Checker calls are independent from planning, retrieval, maker,
            // and report clocks. Slow reasoning providers can exceed 120s on
            // evidence review even when the 300s workflow fuse has ample room.
            "timeout_ms": 180000,
            "output_schema": checker_schema
        },
        "hard_caps": {
            "max_iterations": 4,
            "max_parallel_tasks": max_parallel_tasks.max(1),
            "max_steps_per_task": max_child_steps.clamp(1, 2),
            "retrieval_timeout_ms": 150000,
            "synthesis_timeout_ms": 90000
        }
    })
}

struct PatternTemplate {
    id: &'static str,
    goal: &'static str,
    cadence: &'static str,
    budget: u64,
    denylist: &'static [&'static str],
    triage_skill: &'static str,
    verifier_skill: &'static str,
}

pub(crate) fn parse_loop_command(rest: &str) -> LoopCommand {
    let arg = rest.trim();
    if arg.is_empty() {
        return LoopCommand::Dashboard;
    }
    let (head, tail) = arg
        .split_once(char::is_whitespace)
        .map(|(h, t)| (h, t.trim()))
        .unwrap_or((arg, ""));
    match head {
        "init" => LoopCommand::Init(tail.to_string()),
        "run" => {
            if tail.is_empty() {
                LoopCommand::Usage("usage: /loop run <name>")
            } else {
                LoopCommand::Run(tail.to_string())
            }
        }
        "audit" => {
            if tail.is_empty() {
                LoopCommand::Usage("usage: /loop audit <name>")
            } else {
                LoopCommand::Audit(tail.to_string())
            }
        }
        "logs" => {
            if tail.is_empty() {
                LoopCommand::Usage("usage: /loop logs <name>")
            } else {
                LoopCommand::Logs(tail.to_string())
            }
        }
        _ => LoopCommand::Quick(arg.to_string()),
    }
}

pub(crate) fn loops_root(cwd: &str) -> PathBuf {
    Path::new(cwd).join(".a3s").join("loops")
}

fn pattern(name: &str) -> Option<PatternTemplate> {
    match name {
        "daily-triage" => Some(PatternTemplate {
            id: "daily-triage",
            goal: "Inspect the workspace, recent git activity, CI hints, TODOs, and open risks; update state and produce a concise report.",
            cadence: "1d",
            budget: 120_000,
            denylist: &[".env*", "secrets/**", "infra/prod/**"],
            triage_skill: "Find meaningful changes, failures, TODOs, stale work, and risk signals. Return a short prioritized report with evidence and next actions.",
            verifier_skill: "Check the report against git status, tests mentioned by the project, and the loop state. Flag unsupported claims and missing evidence.",
        }),
        "ci-sweeper" => Some(PatternTemplate {
            id: "ci-sweeper",
            goal: "Watch CI/test failures, isolate the likely cause, and propose the smallest safe fix with verifier evidence.",
            cadence: "15m",
            budget: 220_000,
            denylist: &[
                ".env*",
                "secrets/**",
                "infra/prod/**",
                ".github/workflows/*deploy*",
            ],
            triage_skill: "Collect failing checks, logs, changed files, and suspected root causes. Separate flakes from deterministic failures.",
            verifier_skill: "Run the exact failing checks where possible, inspect the proposed diff, and reject broad or unverified fixes.",
        }),
        "pr-babysitter" => Some(PatternTemplate {
            id: "pr-babysitter",
            goal: "Monitor active PR work, summarize blockers, answer review comments, and prepare safe follow-up actions.",
            cadence: "15m",
            budget: 200_000,
            denylist: &[".env*", "secrets/**", "infra/prod/**"],
            triage_skill: "Summarize PR state, review comments, unresolved threads, CI status, and files that changed since the last run.",
            verifier_skill: "Check proposed replies/fixes against the diff and project rules; require human handoff for ambiguous product choices.",
        }),
        "dependency-sweeper" => Some(PatternTemplate {
            id: "dependency-sweeper",
            goal: "Review dependency drift and suggest low-risk upgrades with changelog and test evidence.",
            cadence: "1d",
            budget: 160_000,
            denylist: &[
                ".env*",
                "secrets/**",
                "infra/prod/**",
                "lockfiles-without-tests/**",
            ],
            triage_skill: "Find outdated dependencies, security advisories, changelog risks, and package-manager constraints.",
            verifier_skill: "Verify upgrade scope, lockfile changes, tests, and rollback notes before recommending action.",
        }),
        "changelog-drafter" => Some(PatternTemplate {
            id: "changelog-drafter",
            goal: "Draft a human-reviewable changelog from recent commits, PRs, and release notes.",
            cadence: "1d",
            budget: 80_000,
            denylist: &[".env*", "secrets/**"],
            triage_skill: "Group recent changes by user-facing impact, breaking changes, fixes, and internal maintenance.",
            verifier_skill: "Check every changelog item against commits or PR evidence and remove unsupported claims.",
        }),
        "agent-dev" => Some(PatternTemplate {
            id: "agent-dev",
            goal: "Iteratively improve one selected A3S Code agent definition with local maker/checker passes and validation evidence.",
            cadence: "manual",
            budget: 100_000,
            denylist: &[".env*", "secrets/**"],
            triage_skill: "Read the active agent definition, identify unclear trigger text, unsafe tool scope, missing workflow guidance, and weak success criteria. Propose the smallest high-value improvements.",
            verifier_skill: "Validate Markdown frontmatter or YAML shape, stable name, one-line description, conservative tools, prompt clarity, and evidence that the revised agent still matches its intended use.",
        }),
        _ => None,
    }
}

fn known_pattern(name: &str) -> bool {
    pattern(name).is_some()
}

pub(super) fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in s.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        DEFAULT_PATTERN.to_string()
    } else {
        out
    }
}

fn list_literal(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn spec_text(spec: &LoopSpec) -> String {
    format!(
        "id = \"{}\"\npattern = \"{}\"\ngoal = \"{}\"\nlevel = \"{}\"\ncadence = \"{}\"\nos_runtime = {}\nworktree = {}\nmaker_agent = \"{}\"\nchecker_agent = \"{}\"\nbudget_tokens_per_day = {}\nmax_iterations_per_run = {}\ndenylist = {}\nconnectors = {}\n",
        spec.id,
        spec.pattern,
        spec.goal.replace('"', "\\\""),
        spec.level,
        spec.cadence,
        spec.os_runtime,
        spec.worktree,
        spec.maker_agent,
        spec.checker_agent,
        spec.budget_tokens_per_day,
        spec.max_iterations_per_run,
        list_literal(&spec.denylist),
        list_literal(&spec.connectors),
    )
}

pub(crate) fn init_loop(cwd: &str, arg: &str) -> Result<LoopSpec, String> {
    let parts = arg.split_whitespace().collect::<Vec<_>>();
    let first = parts.first().copied().unwrap_or(DEFAULT_PATTERN);
    let second = parts.get(1).copied();
    let pattern_id = if let Some(p) = second.filter(|p| known_pattern(p)) {
        p
    } else if known_pattern(first) {
        first
    } else {
        DEFAULT_PATTERN
    };
    let id = if parts.is_empty() || known_pattern(first) {
        pattern_id.to_string()
    } else {
        slug(first)
    };
    let template =
        pattern(pattern_id).ok_or_else(|| format!("unknown loop pattern: {pattern_id}"))?;
    let dir = loops_root(cwd).join(&id);
    let config = dir.join(LOOP_CONFIG);
    if config.exists() {
        return Err(format!("loop `{id}` already exists"));
    }
    std::fs::create_dir_all(dir.join("skills")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir.join("reports")).map_err(|e| e.to_string())?;
    let spec = LoopSpec {
        id,
        pattern: template.id.to_string(),
        goal: template.goal.to_string(),
        level: "L1".to_string(),
        cadence: template.cadence.to_string(),
        os_runtime: true,
        worktree: true,
        maker_agent: "implementer".to_string(),
        checker_agent: "verifier".to_string(),
        budget_tokens_per_day: template.budget,
        max_iterations_per_run: 3,
        denylist: template.denylist.iter().map(|s| s.to_string()).collect(),
        connectors: vec!["os-runtime".to_string()],
        dir: dir.clone(),
    };
    std::fs::write(&config, spec_text(&spec)).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join(STATE_FILE),
        format!(
            "# Loop State: {}\n\nStatus: ready\n\n## Current Focus\n- {}\n\n## Open Items\n- None yet.\n\n## Human Handoff\n- None.\n",
            spec.id, spec.goal
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join(RUN_LOG_FILE),
        format!("# Loop Run Log: {}\n\n", spec.id),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join(BUDGET_FILE),
        format!(
            "tokens_per_day = {}\nmax_iterations_per_run = {}\nkill_switch = false\n",
            spec.budget_tokens_per_day, spec.max_iterations_per_run
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("skills").join("triage.md"),
        format!("# Triage Skill\n\n{}\n", template.triage_skill),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("skills").join("verifier.md"),
        format!("# Verifier Skill\n\n{}\n", template.verifier_skill),
    )
    .map_err(|e| e.to_string())?;
    Ok(spec)
}

fn agent_loop_default_id(agent: &AgentDevSession) -> String {
    format!("agent-{}", slug(&agent.name))
}

fn agent_loop_init_arg(agent: &AgentDevSession, arg: &str) -> String {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return format!("{} agent-dev", agent_loop_default_id(agent));
    }
    let parts = trimmed.split_whitespace().collect::<Vec<_>>();
    let first = parts.first().copied().unwrap_or_default();
    if parts.len() == 1 && known_pattern(first) {
        format!("{} {first}", agent_loop_default_id(agent))
    } else if parts.len() == 1 {
        format!("{} agent-dev", slug(first))
    } else {
        trimmed.to_string()
    }
}

fn agent_loop_goal(agent: &AgentDevSession) -> String {
    format!(
        "Iteratively improve A3S Code agent package `{}` ({}) for its stated purpose: {}. Keep work scoped to {} with entrypoint {} and maintain a valid agent definition.",
        agent.name,
        agent.rel,
        agent.description,
        agent.package_path.display(),
        agent.path.display()
    )
}

pub(crate) fn init_agent_loop(
    cwd: &str,
    arg: &str,
    agent: &AgentDevSession,
) -> Result<LoopSpec, String> {
    let init_arg = agent_loop_init_arg(agent, arg);
    let mut spec = init_loop(cwd, &init_arg)?;
    spec.goal = agent_loop_goal(agent);
    spec.level = "A2".to_string();
    spec.os_runtime = false;
    spec.worktree = false;
    spec.connectors.clear();

    std::fs::write(spec.dir.join(LOOP_CONFIG), spec_text(&spec)).map_err(|e| e.to_string())?;
    std::fs::write(
        spec.dir.join(STATE_FILE),
        format!(
            "# Loop State: {}\n\nStatus: ready\n\n## Current Focus\n- {}\n\n## Target Agent\n- Name: {}\n- Package: {}\n- Entrypoint: {}\n- Root: {}\n\n## Open Items\n- None yet.\n\n## Human Handoff\n- None.\n",
            spec.id,
            spec.goal,
            agent.name,
            agent.package_path.display(),
            agent.path.display(),
            agent.root.display()
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        spec.dir.join("skills").join("triage.md"),
        format!(
            "# Agent Triage Skill\n\nRead package `{}` and entrypoint `{}`. Identify the highest-value improvements for `{}`. Focus on trigger description, tools, workflow, package resources, constraints, verification, and examples.\n",
            agent.package_path.display(),
            agent.path.display(),
            agent.name
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        spec.dir.join("skills").join("verifier.md"),
        "# Agent Verifier Skill\n\nVerify the agent definition remains valid Markdown/YAML, keeps a stable name, has a one-line trigger description, uses conservative tools, and contains actionable workflow and success criteria.\n",
    )
    .map_err(|e| e.to_string())?;
    Ok(spec)
}

fn unquote(v: &str) -> String {
    v.trim()
        .trim_matches('"')
        .replace("\\\"", "\"")
        .replace("\\\\", "\\")
}

fn parse_list(v: &str) -> Vec<String> {
    v.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(unquote)
        .filter(|s| !s.trim().is_empty())
        .collect()
}

pub(super) fn read_spec(path: &Path) -> Result<LoopSpec, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse_spec(&text, path.parent().unwrap_or(Path::new(".")).to_path_buf())
}

pub(crate) fn parse_spec(text: &str, dir: PathBuf) -> Result<LoopSpec, String> {
    let mut id = String::new();
    let mut pattern = DEFAULT_PATTERN.to_string();
    let mut goal = String::new();
    let mut level = "L1".to_string();
    let mut cadence = "1d".to_string();
    let mut os_runtime = true;
    let mut worktree = true;
    let mut maker_agent = "implementer".to_string();
    let mut checker_agent = "verifier".to_string();
    let mut budget_tokens_per_day = 100_000;
    let mut max_iterations_per_run = 3;
    let mut denylist = Vec::new();
    let mut connectors = Vec::new();

    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some((key, value)) = t.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "id" => id = unquote(value),
            "pattern" => pattern = unquote(value),
            "goal" => goal = unquote(value),
            "level" => level = unquote(value),
            "cadence" => cadence = unquote(value),
            "os_runtime" => os_runtime = value == "true",
            "worktree" => worktree = value == "true",
            "maker_agent" => maker_agent = unquote(value),
            "checker_agent" => checker_agent = unquote(value),
            "budget_tokens_per_day" => {
                budget_tokens_per_day = value.parse().unwrap_or(budget_tokens_per_day)
            }
            "max_iterations_per_run" => {
                max_iterations_per_run = value.parse().unwrap_or(max_iterations_per_run)
            }
            "denylist" => denylist = parse_list(value),
            "connectors" => connectors = parse_list(value),
            _ => {}
        }
    }
    if id.trim().is_empty() {
        return Err("loop.toml missing id".to_string());
    }
    if goal.trim().is_empty() {
        goal = format!("Run the {id} loop and keep state current.");
    }
    Ok(LoopSpec {
        id,
        pattern,
        goal,
        level,
        cadence,
        os_runtime,
        worktree,
        maker_agent,
        checker_agent,
        budget_tokens_per_day,
        max_iterations_per_run,
        denylist,
        connectors,
        dir,
    })
}

pub(crate) fn list_loops(cwd: &str) -> Vec<LoopSummary> {
    let root = loops_root(cwd);
    let mut loops = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path().join(LOOP_CONFIG);
            if !path.is_file() {
                continue;
            }
            if let Ok(spec) = read_spec(&path) {
                let audit = audit_loop(&spec);
                let last_run = last_run_label(&spec.dir);
                loops.push(LoopSummary {
                    spec,
                    audit,
                    last_run,
                });
            }
        }
    }
    loops.sort_by(|a, b| a.spec.id.cmp(&b.spec.id));
    loops
}

pub(crate) fn find_loop(cwd: &str, name: &str) -> Result<LoopSpec, String> {
    let needle = slug(name);
    list_loops(cwd)
        .into_iter()
        .find(|s| s.spec.id == name || s.spec.id == needle)
        .map(|s| s.spec)
        .ok_or_else(|| format!("loop `{name}` not found"))
}

fn exists_nonempty(path: &Path) -> bool {
    path.is_file()
        && std::fs::metadata(path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
}

pub(crate) fn audit_loop(spec: &LoopSpec) -> LoopAudit {
    let mut score = 0u8;
    let mut passed = Vec::new();
    let mut missing = Vec::new();
    let mut warnings = Vec::new();
    let mut add = |points: u8, ok: bool, pass: &str, miss: &str| {
        if ok {
            score = score.saturating_add(points);
            passed.push(pass.to_string());
        } else {
            missing.push(miss.to_string());
        }
    };
    add(
        10,
        !spec.goal.trim().is_empty(),
        "single clear goal",
        "add a single-sentence goal",
    );
    add(
        10,
        exists_nonempty(&spec.dir.join(STATE_FILE)),
        "durable STATE.md",
        "create STATE.md",
    );
    add(
        10,
        exists_nonempty(&spec.dir.join(RUN_LOG_FILE)),
        "append-only RUN_LOG.md",
        "create RUN_LOG.md",
    );
    add(
        10,
        exists_nonempty(&spec.dir.join(BUDGET_FILE)) && spec.budget_tokens_per_day > 0,
        "budget and kill-switch file",
        "create budget.toml with daily caps",
    );
    add(
        10,
        !spec.denylist.is_empty(),
        "denylist paths configured",
        "add denylist paths for secrets/infra",
    );
    add(
        15,
        !spec.maker_agent.is_empty()
            && !spec.checker_agent.is_empty()
            && spec.maker_agent != spec.checker_agent,
        "maker/checker split",
        "configure separate maker_agent and checker_agent",
    );
    let agent_loop = spec.level == "A2";
    let goal_loop = spec.level == "G1";
    add(
        10,
        spec.worktree || agent_loop || goal_loop,
        if agent_loop {
            "agent asset scope requested"
        } else if goal_loop {
            "goal scope is guarded by the active session"
        } else {
            "worktree isolation requested"
        },
        if agent_loop {
            "scope the loop to one agent definition"
        } else {
            "enable worktree isolation before L2"
        },
    );
    add(
        10,
        if agent_loop || goal_loop {
            !spec.os_runtime && !spec.connectors.iter().any(|c| c == "os-runtime")
        } else {
            spec.os_runtime && spec.connectors.iter().any(|c| c == "os-runtime")
        },
        if agent_loop {
            "local agent loop runtime"
        } else if goal_loop {
            "local goal loop runtime"
        } else {
            "OS Runtime connector enabled"
        },
        if agent_loop {
            "disable OS Runtime for local /agent loops"
        } else {
            "enable os_runtime/connectors=[\"os-runtime\"]"
        },
    );
    let skill_pair = if goal_loop {
        exists_nonempty(&spec.dir.join("skills").join("maker.md"))
            && exists_nonempty(&spec.dir.join("skills").join("verifier.md"))
    } else {
        exists_nonempty(&spec.dir.join("skills").join("triage.md"))
            && exists_nonempty(&spec.dir.join("skills").join("verifier.md"))
    };
    add(
        15,
        skill_pair,
        if goal_loop {
            "maker and verifier skills"
        } else {
            "triage and verifier skills"
        },
        if goal_loop {
            "add skills/maker.md and skills/verifier.md"
        } else {
            "add skills/triage.md and skills/verifier.md"
        },
    );
    if spec.level == "L3" && score < 90 {
        warnings.push("L3 requested but readiness is below unattended threshold".to_string());
    }
    if spec.level != "L1" && spec.level != "A2" && spec.level != "G1" && !spec.worktree {
        warnings.push("acting loops should use worktree isolation".to_string());
    }
    let level = if score >= 90 {
        "L3-ready"
    } else if score >= 75 {
        "L2-ready"
    } else if score >= 50 {
        "L1-ready"
    } else {
        "L0-draft"
    }
    .to_string();
    LoopAudit {
        score,
        level,
        passed,
        missing,
        warnings,
    }
}
