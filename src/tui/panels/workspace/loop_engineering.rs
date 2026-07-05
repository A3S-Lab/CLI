//! `/loop` engineered loops: persisted loop specs, state, run logs, audit, and
//! an OS-aware run directive.

use super::super::*;
use super::agent::{self, AgentDevSession};
use a3s_tui::components::{DetailPanel, DetailRow, SectionHeader};
use std::path::{Path, PathBuf};

const DEFAULT_PATTERN: &str = "daily-triage";
const LOOP_CONFIG: &str = "loop.toml";
const STATE_FILE: &str = "STATE.md";
const RUN_LOG_FILE: &str = "RUN_LOG.md";

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
        let detail = DetailPanel::new(summary.spec.id.clone())
            .show_separator(false)
            .indent(2)
            .title_color(TN_FG)
            .label_color(TN_GRAY)
            .value_color(TN_GRAY)
            .label_width(8)
            .unlimited_rows()
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
        lines.extend(
            detail
                .view(width.min(u16::MAX as usize) as u16, 7)
                .lines()
                .map(str::to_string),
        );
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
const BUDGET_FILE: &str = "budget.toml";

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
            denylist: &[".env*", "secrets/**", "infra/prod/**", ".github/workflows/*deploy*"],
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
            denylist: &[".env*", "secrets/**", "infra/prod/**", "lockfiles-without-tests/**"],
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

fn slug(s: &str) -> String {
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

fn spec_text(spec: &LoopSpec) -> String {
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
        "Iteratively improve A3S Code agent `{}` ({}) for its stated purpose: {}. Keep work scoped to {} and maintain a valid agent definition.",
        agent.name,
        agent.rel,
        agent.description,
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
            "# Loop State: {}\n\nStatus: ready\n\n## Current Focus\n- {}\n\n## Target Agent\n- Name: {}\n- Definition: {}\n- Root: {}\n\n## Open Items\n- None yet.\n\n## Human Handoff\n- None.\n",
            spec.id,
            spec.goal,
            agent.name,
            agent.path.display(),
            agent.root.display()
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        spec.dir.join("skills").join("triage.md"),
        format!(
            "# Agent Triage Skill\n\nRead `{}` and identify the highest-value improvements for `{}`. Focus on trigger description, tools, workflow, constraints, verification, and examples.\n",
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

fn read_spec(path: &Path) -> Result<LoopSpec, String> {
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
    add(
        10,
        spec.worktree || agent_loop,
        if agent_loop {
            "agent asset scope requested"
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
        if agent_loop {
            !spec.os_runtime && !spec.connectors.iter().any(|c| c == "os-runtime")
        } else {
            spec.os_runtime && spec.connectors.iter().any(|c| c == "os-runtime")
        },
        if agent_loop {
            "local agent loop runtime"
        } else {
            "OS Runtime connector enabled"
        },
        if agent_loop {
            "disable OS Runtime for local /agent loops"
        } else {
            "enable os_runtime/connectors=[\"os-runtime\"]"
        },
    );
    add(
        15,
        exists_nonempty(&spec.dir.join("skills").join("triage.md"))
            && exists_nonempty(&spec.dir.join("skills").join("verifier.md")),
        "triage and verifier skills",
        "add skills/triage.md and skills/verifier.md",
    );
    if spec.level == "L3" && score < 90 {
        warnings.push("L3 requested but readiness is below unattended threshold".to_string());
    }
    if spec.level != "L1" && spec.level != "A2" && !spec.worktree {
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

fn last_run_label(dir: &Path) -> String {
    let path = dir.join(RUN_LOG_FILE);
    let Ok(text) = std::fs::read_to_string(path) else {
        return "never".to_string();
    };
    text.lines()
        .rev()
        .find(|l| l.trim_start().starts_with("- "))
        .map(|l| l.trim_start().trim_start_matches("- ").to_string())
        .unwrap_or_else(|| "never".to_string())
}

pub(crate) fn append_run_start(spec: &LoopSpec, os_available: bool) -> Result<(), String> {
    let line = format!(
        "- {} start · level={} · os_runtime={} · status=running\n",
        chrono::Utc::now().to_rfc3339(),
        spec.level,
        os_available && spec.os_runtime
    );
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(spec.dir.join(RUN_LOG_FILE))
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.as_bytes())
        })
        .map_err(|e| e.to_string())
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn loop_run_prompt(spec: &LoopSpec, cwd: &str, os_available: bool) -> String {
    let mode = if os_available {
        LoopRuntimeMode::OsAvailable
    } else {
        LoopRuntimeMode::LocalNoOs
    };
    loop_run_prompt_with_runtime(spec, cwd, mode)
}

pub(crate) fn loop_run_prompt_with_runtime(
    spec: &LoopSpec,
    cwd: &str,
    runtime_mode: LoopRuntimeMode,
) -> String {
    let state = path_text(&spec.dir.join(STATE_FILE));
    let log = path_text(&spec.dir.join(RUN_LOG_FILE));
    let reports = path_text(&spec.dir.join("reports"));
    let skills = path_text(&spec.dir.join("skills"));
    let os_directive = match runtime_mode {
        LoopRuntimeMode::OsAvailable if spec.os_runtime => {
            format!(
                "OS IS AVAILABLE AND MUST BE USED. Use the signed-in A3S OS capabilities and A3S Runtime instead of doing the whole loop serially in the local shell. Split independent discovery/checker work into 3-6 parallel `parallel_task` branches or OS Runtime `runtime` tasks. Use shaped progressive API calls (`shaped:true`) when creating/reporting OS views so the TUI can surface RemoteUI. Create a Markdown report and standalone HTML report, then return the OS `.view`/`viewUrl` response; if no view can be created, explain the missing OS capability explicitly. Runtime evidence must include both fan-out (`runtime` or `parallel_task`) and the report view. Runtime work should be visible through the asset-scoped runtime activity panel; do not hide all execution in one long local command. {}",
                RuntimePolicy::Required.directive()
            )
        }
        LoopRuntimeMode::LocalAgentDev => {
            format!(
                "This loop is running inside local /agent development mode. Stay local even if OS is signed in. Do not open OS, WebIDE, RemoteUI, browser pages, or the OS workflow designer. Do not claim an OS RemoteUI view exists. Use local maker/checker passes; update the target agent definition only when the loop goal asks for agent improvements, and always update the loop state/report artifacts. {}",
                RuntimePolicy::LocalOnly.directive()
            )
        }
        LoopRuntimeMode::OsAvailable => {
            "This loop has os_runtime disabled. Run it locally and do not claim an OS RemoteUI view exists unless a later explicit OS tool call returns one.".to_string()
        }
        LoopRuntimeMode::LocalNoOs => {
            "OS is not signed in for this TUI session. Run the loop locally, but keep the same report/state artifacts. Do not claim an OS RemoteUI view exists; tell the user `/login` enables Runtime parallelism and RemoteUI.".to_string()
        }
    };
    let action_policy = match spec.level.as_str() {
        "L1" => {
            "L1 report-only: do not modify project source files. Only update this loop's STATE.md, RUN_LOG.md, and reports/ artifacts unless the user explicitly asks for a fix."
        }
        "A2" => {
            "A2 agent-development loop: the selected agent definition is the target asset. You may edit that agent definition and this loop's STATE.md, RUN_LOG.md, and reports/ artifacts when the goal asks for improvements. Keep changes local, validate the agent definition, and do not deploy."
        }
        "L2" => {
            "L2 assisted: use an isolated git worktree for any code edits, run verifier checks, and stop at a patch/branch plus human handoff. Do not merge or deploy."
        }
        "L3" => {
            "L3 unattended: act only within allowlisted low-risk scope, obey denylist and budget, and escalate to human on ambiguity, repeated failures, secrets, auth, infra, or product decisions."
        }
        _ => "Draft loop: produce a report and improve the loop state before taking action.",
    };
    format!(
        "Run this A3S Code engineered loop.\n\n\
         Loop id: {id}\n\
         Pattern: {pattern}\n\
         Level: {level}\n\
         Cadence target: {cadence}\n\
         Workspace: {cwd}\n\
         Goal: {goal}\n\n\
         Files you must read first:\n\
         - Config: {config}\n\
         - State: {state}\n\
         - Run log: {log}\n\
         - Skills directory: {skills}\n\n\
         Loop contract:\n\
         1. Read the state, run log, and skills before deciding work.\n\
         2. Respect denylist paths: {denylist}.\n\
         3. Respect budget: {budget} tokens/day, max {max_iter} iterations this run.\n\
         4. Use maker/checker split: maker `{maker}`, checker `{checker}`. The maker cannot declare its own work verified.\n\
         5. {action_policy}\n\
         6. {os_directive}\n\
         7. End by updating {state}, appending a finished entry to {log}, and creating both a Markdown and HTML report under {reports}.\n\
         8. Final answer: summarize what changed, list report paths, mention any OS RemoteUI view surfaced by the host, and say what needs human input.\n\n\
         Start now.",
        id = spec.id,
        pattern = spec.pattern,
        level = spec.level,
        cadence = spec.cadence,
        cwd = cwd,
        goal = spec.goal,
        config = path_text(&spec.dir.join(LOOP_CONFIG)),
        state = state,
        log = log,
        skills = skills,
        denylist = spec.denylist.join(", "),
        budget = spec.budget_tokens_per_day,
        max_iter = spec.max_iterations_per_run,
        maker = spec.maker_agent,
        checker = spec.checker_agent,
        reports = reports,
    )
}

fn audit_note(audit: &LoopAudit) -> String {
    let mut note = format!("score {} · {}", audit.score, audit.level);
    if !audit.missing.is_empty() {
        note.push_str(&format!(" · missing {}", audit.missing.len()));
    }
    note
}

impl App {
    pub(crate) fn handle_loop_command(&mut self, rest: &str) -> Option<Cmd<Msg>> {
        match parse_loop_command(rest) {
            LoopCommand::Dashboard => {
                self.textarea.clear();
                self.open_loop_panel(None);
                None
            }
            LoopCommand::Init(arg) => {
                self.textarea.clear();
                let agent = self.agent_dev.clone();
                let result = match agent.as_ref() {
                    Some(dev) => init_agent_loop(&self.cwd, &arg, dev),
                    None => init_loop(&self.cwd, &arg),
                };
                match result {
                    Ok(spec) => {
                        self.push_line(&gutter(
                            TN_GREEN,
                            &format!(
                                "loop `{}` initialized · {} · /loop run {}",
                                spec.id,
                                spec.dir.display(),
                                spec.id
                            ),
                        ));
                        let note = match agent.as_ref() {
                            Some(dev) => {
                                format!("created agent loop `{}` for `{}`", spec.id, dev.name)
                            }
                            None => format!("created `{}`", spec.id),
                        };
                        self.open_loop_panel(Some(note));
                    }
                    Err(e) => self.push_line(
                        &Style::new()
                            .fg(TN_RED)
                            .render(&format!("  /loop init failed: {e}")),
                    ),
                }
                None
            }
            LoopCommand::Run(name) => {
                self.textarea.clear();
                match find_loop(&self.cwd, &name) {
                    Ok(spec) => self.start_engineered_loop(spec),
                    Err(e) => {
                        self.push_line(
                            &Style::new()
                                .fg(TN_YELLOW)
                                .render(&format!("  {e} · create one with /loop init {name}")),
                        );
                        None
                    }
                }
            }
            LoopCommand::Audit(name) => {
                self.textarea.clear();
                match find_loop(&self.cwd, &name) {
                    Ok(spec) => {
                        let audit = audit_loop(&spec);
                        self.push_line(&gutter(
                            TN_CYAN,
                            &format!("loop audit `{}` · {}", spec.id, audit_note(&audit)),
                        ));
                        for item in audit.missing.iter().take(4) {
                            self.push_line(
                                &Style::new()
                                    .fg(TN_YELLOW)
                                    .render(&format!("  missing: {item}")),
                            );
                        }
                        self.open_loop_panel(Some(format!(
                            "audit `{}` · {}",
                            spec.id,
                            audit_note(&audit)
                        )));
                    }
                    Err(e) => self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  {e}"))),
                }
                None
            }
            LoopCommand::Logs(name) => {
                self.textarea.clear();
                match find_loop(&self.cwd, &name) {
                    Ok(spec) => {
                        let path = spec.dir.join(RUN_LOG_FILE);
                        match std::fs::read_to_string(&path) {
                            Ok(text) => self.open_readonly_in_ide(
                                &format!("loop-{}-run-log.md", spec.id),
                                &text,
                            ),
                            Err(e) => self.push_line(
                                &Style::new()
                                    .fg(TN_YELLOW)
                                    .render(&format!("  run log unavailable: {e}")),
                            ),
                        }
                    }
                    Err(e) => self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  {e}"))),
                }
                None
            }
            LoopCommand::Quick(task) => {
                self.textarea.clear();
                if let Some(dev) = &self.agent_dev {
                    self.push_line(&gutter(
                        TN_GREEN,
                        &format!("agent loop `{}` · local auto-continue", dev.name),
                    ));
                }
                self.engage_autonomy(8);
                Some(cmd::msg(Msg::Submit(task)))
            }
            LoopCommand::Usage(usage) => {
                self.textarea.clear();
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!("  {usage}")));
                None
            }
        }
    }

    fn start_engineered_loop(&mut self, spec: LoopSpec) -> Option<Cmd<Msg>> {
        let agent = self.agent_dev.clone();
        let runtime_mode = if agent.is_some() {
            LoopRuntimeMode::LocalAgentDev
        } else if self.os_session.is_some() {
            LoopRuntimeMode::OsAvailable
        } else {
            LoopRuntimeMode::LocalNoOs
        };
        let os_available = matches!(runtime_mode, LoopRuntimeMode::OsAvailable);
        if let Err(e) = append_run_start(&spec, os_available) {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render(&format!("  loop run log could not be updated: {e}")),
            );
        }
        self.goal = Some(match agent.as_ref() {
            Some(dev) => agent::agent_goal_label(dev, &spec.goal),
            None => spec.goal.clone(),
        });
        self.goal_since = Some(Instant::now());
        self.push_line(&gutter(
            TN_CYAN,
            &format!(
                "loop `{}` running · {} · {}",
                spec.id,
                spec.level,
                if matches!(runtime_mode, LoopRuntimeMode::LocalAgentDev) {
                    "local agent engineering"
                } else if os_available && spec.os_runtime {
                    "OS Runtime + RemoteUI required"
                } else {
                    "local fallback"
                }
            ),
        ));
        if os_available && spec.os_runtime {
            self.push_line(&Style::new().fg(TN_GRAY).render(
                "  OS connected: use A3S Runtime parallel workers; inspect them with asset activity",
            ));
        } else if let Some(dev) = &agent {
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  /agent active: loop stays local and targets {} ({})",
                dev.name, dev.rel
            )));
        }
        let prompt = if matches!(runtime_mode, LoopRuntimeMode::LocalAgentDev) {
            loop_run_prompt_with_runtime(&spec, &self.cwd, runtime_mode)
        } else {
            loop_run_prompt(&spec, &self.cwd, os_available)
        };
        let (prompt, display) = match agent.as_ref() {
            Some(dev) => (
                agent::agent_loop_prompt(dev, &prompt),
                format!("◇ loop {}: {}", dev.name, truncate(&spec.goal, 48)),
            ),
            None => (
                prompt,
                format!("loop {}: {}", spec.id, truncate(&spec.goal, 54)),
            ),
        };
        self.engage_autonomy(8);
        let runtime_expectation = (os_available && spec.os_runtime)
            .then(|| RuntimeExpectation::required_report_view(format!("loop {}", spec.id)));
        self.start_stream_inner_with_runtime(
            prompt,
            display,
            true,
            true,
            false,
            runtime_expectation,
        )
    }

    pub(crate) fn open_loop_panel(&mut self, note: Option<String>) {
        let loops = list_loops(&self.cwd);
        let note = note.unwrap_or_else(|| {
            if let Some(dev) = &self.agent_dev {
                format!(
                    "agent dev `{}` active · /loop init creates an agent-scoped local loop",
                    dev.name
                )
            } else if loops.is_empty() {
                "no loops yet · /loop init daily-triage".to_string()
            } else if self.os_session.is_some() {
                "OS connected · runs use A3S Runtime + RemoteUI when enabled".to_string()
            } else {
                "sign in with /login to use OS Runtime + RemoteUI".to_string()
            }
        });
        self.loop_panel = Some(LoopPanel {
            loops,
            sel: 0,
            note,
        });
    }

    pub(crate) fn handle_loop_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        if key.code == KeyCode::Esc {
            self.loop_panel = None;
            return None;
        }
        let last = self
            .loop_panel
            .as_ref()
            .map(|p| p.loops.len().saturating_sub(1))
            .unwrap_or(0);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = self.loop_panel.as_mut() {
                    p.sel = p.sel.saturating_sub(1);
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(p) = self.loop_panel.as_mut() {
                    p.sel = (p.sel + 1).min(last);
                }
                None
            }
            KeyCode::Char('r') | KeyCode::Enter => {
                let spec = self
                    .loop_panel
                    .as_ref()
                    .and_then(|p| p.loops.get(p.sel))
                    .map(|s| s.spec.clone())?;
                self.loop_panel = None;
                self.start_engineered_loop(spec)
            }
            KeyCode::Char('a') => {
                if let Some(p) = self.loop_panel.as_mut() {
                    if let Some(summary) = p.loops.get(p.sel) {
                        p.note = format!(
                            "audit `{}` · {}",
                            summary.spec.id,
                            audit_note(&summary.audit)
                        );
                    }
                }
                None
            }
            KeyCode::Char('l') => {
                let spec = self
                    .loop_panel
                    .as_ref()
                    .and_then(|p| p.loops.get(p.sel))
                    .map(|s| s.spec.clone())?;
                let path = spec.dir.join(RUN_LOG_FILE);
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        self.loop_panel = None;
                        self.open_readonly_in_ide(&format!("loop-{}-run-log.md", spec.id), &text);
                    }
                    Err(e) => {
                        if let Some(p) = self.loop_panel.as_mut() {
                            p.note = format!("run log unavailable: {e}");
                        }
                    }
                }
                None
            }
            KeyCode::Char('p') => {
                let query = self
                    .loop_panel
                    .as_ref()
                    .and_then(|p| p.loops.get(p.sel))
                    .map(|s| s.spec.id.clone())
                    .unwrap_or_default();
                self.loop_panel = None;
                self.open_runtime_activity_panel(query)
            }
            KeyCode::Char('i') => {
                let agent = self.agent_dev.clone();
                let result = match agent.as_ref() {
                    Some(dev) => init_agent_loop(&self.cwd, "", dev),
                    None => init_loop(&self.cwd, DEFAULT_PATTERN),
                };
                match result {
                    Ok(spec) => {
                        let note = match agent.as_ref() {
                            Some(dev) => {
                                format!("created agent loop `{}` for `{}`", spec.id, dev.name)
                            }
                            None => format!("created `{}`", spec.id),
                        };
                        self.open_loop_panel(Some(note));
                    }
                    Err(e) => {
                        if let Some(p) = self.loop_panel.as_mut() {
                            p.note = e;
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(crate) fn render_loop_panel(&self, panel: &LoopPanel) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let (left_w, right_w) = loop_columns(width);
        let mut left = Vec::new();
        left.extend(loop_header_lines(!panel.loops.is_empty(), left_w));
        if !panel.loops.is_empty() {
            for (idx, item) in panel.loops.iter().enumerate() {
                let mark = if idx == panel.sel { ">" } else { " " };
                let row = format!(
                    "{mark} {:<18} {:<3} {:>3}  {}",
                    truncate(&item.spec.id, 18),
                    item.spec.level,
                    item.audit.score,
                    truncate(&item.last_run, left_w.saturating_sub(31))
                );
                let style = if idx == panel.sel {
                    Style::new().fg(TN_CYAN).bold()
                } else {
                    Style::new().fg(TN_FG)
                };
                left.push(loop_line(&style.render(&row), left_w));
            }
        }
        while left.len() < h {
            left.push(" ".repeat(left_w));
        }
        let selected = panel.loops.get(panel.sel);
        let mut right = loop_detail_lines(selected, &panel.note, right_w);
        while right.len() < h {
            right.push(" ".repeat(right_w));
        }
        let mut rows = Vec::new();
        let sep = if width == 0 {
            String::new()
        } else {
            Style::new().fg(TN_GRAY).render("│")
        };
        for i in 0..h {
            rows.push(format!(
                "{}{}{}",
                left.get(i).cloned().unwrap_or_else(|| " ".repeat(left_w)),
                sep,
                right.get(i).cloned().unwrap_or_else(|| " ".repeat(right_w))
            ));
        }
        rows.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "a3s-loop-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file_text(path: impl AsRef<Path>) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    fn agent_dev_session(root: &Path) -> AgentDevSession {
        AgentDevSession {
            name: "code-reviewer".into(),
            description: "Review code changes carefully".into(),
            rel: "review/code-reviewer.md".into(),
            path: root.join("agents/review/code-reviewer.md"),
            root: root.join("agents"),
        }
    }

    #[test]
    fn loop_columns_keep_narrow_panels_usable() {
        let (left, right) = loop_columns(40);

        assert!(left > 0, "left column should not collapse");
        assert!(right > 0, "right column should not collapse");
        assert!(left + 1 + right <= 40);
    }

    #[test]
    fn loop_lines_are_width_bounded_with_styles() {
        let line = loop_line(
            &Style::new()
                .fg(TN_GRAY)
                .render("  Enter/r run · a audit · l logs · p runtime · i init · Esc"),
            22,
        );

        assert!(
            a3s_tui::style::visible_len(&line) <= 22,
            "{}",
            a3s_tui::style::strip_ansi(&line)
        );
    }

    #[test]
    fn loop_header_lines_use_shared_section_header_and_fit_width() {
        let lines = loop_header_lines(false, 34);
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(lines.len(), 3);
        assert!(plain.contains("loop engineering"), "{plain}");
        assert!(plain.contains("no loops"), "{plain}");
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 34),
            "{plain}"
        );
    }

    #[test]
    fn loop_detail_lines_use_shared_detail_panel_and_fit_width() {
        let root = temp_root("detail-lines");
        let summary = LoopSummary {
            spec: LoopSpec {
                id: "daily-triage".into(),
                pattern: "daily-triage".into(),
                goal: "Inspect workspace state, tests, and risks before reporting".into(),
                level: "L1".into(),
                cadence: "1d".into(),
                os_runtime: true,
                worktree: true,
                maker_agent: "triage".into(),
                checker_agent: "verifier".into(),
                budget_tokens_per_day: 120_000,
                max_iterations_per_run: 1,
                denylist: vec![".env*".into()],
                connectors: vec!["os-runtime".into()],
                dir: root.join(".a3s/loops/daily-triage"),
            },
            audit: LoopAudit {
                score: 72,
                level: "L1-ready".into(),
                passed: vec![],
                missing: vec![
                    "budget.toml is missing a daily cap".into(),
                    "os_runtime connector has not been verified".into(),
                ],
                warnings: vec![],
            },
            last_run: "never".into(),
        };

        let lines = loop_detail_lines(Some(&summary), "ready", 38);
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(plain.contains("daily-triage"), "{plain}");
        assert!(plain.contains("pattern"), "{plain}");
        assert!(plain.contains("Missing"), "{plain}");
        assert!(plain.contains("- budget.toml"), "{plain}");
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 38),
            "{plain}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parse_loop_command_keeps_quick_loop_form() {
        assert_eq!(parse_loop_command(""), LoopCommand::Dashboard);
        assert_eq!(
            parse_loop_command("run daily-triage"),
            LoopCommand::Run("daily-triage".into())
        );
        assert_eq!(
            parse_loop_command("fix the tests"),
            LoopCommand::Quick("fix the tests".into())
        );
    }

    #[test]
    fn parse_loop_command_requires_names_for_targeted_subcommands() {
        assert_eq!(
            parse_loop_command("run"),
            LoopCommand::Usage("usage: /loop run <name>")
        );
        assert_eq!(
            parse_loop_command("audit"),
            LoopCommand::Usage("usage: /loop audit <name>")
        );
        assert_eq!(
            parse_loop_command("logs"),
            LoopCommand::Usage("usage: /loop logs <name>")
        );
        assert_eq!(
            parse_loop_command("log daily-triage"),
            LoopCommand::Quick("log daily-triage".into()),
            "/loop log must not stay as a hidden /loop logs alias"
        );
        assert_eq!(
            parse_loop_command("help"),
            LoopCommand::Quick("help".into()),
            "/loop help must not stay as a hidden dashboard alias"
        );
    }

    #[test]
    fn init_loop_scaffolds_state_budget_skills_and_audits_l1_ready() {
        let root = temp_root("init");
        let cwd = root.to_string_lossy();
        let spec = init_loop(&cwd, "daily-triage").unwrap();
        assert!(spec.dir.join(LOOP_CONFIG).is_file());
        assert!(spec.dir.join(STATE_FILE).is_file());
        assert!(spec.dir.join(RUN_LOG_FILE).is_file());
        assert!(spec.dir.join(BUDGET_FILE).is_file());
        assert!(spec.dir.join("skills").join("triage.md").is_file());
        let audit = audit_loop(&spec);
        assert!(audit.score >= 75, "{audit:?}");
        assert!(audit.passed.iter().any(|p| p.contains("OS Runtime")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn init_loop_rejects_duplicate_id_without_overwriting_existing_files() {
        let root = temp_root("duplicate");
        let cwd = root.to_string_lossy();
        let spec = init_loop(&cwd, "daily-triage").unwrap();
        let config = spec.dir.join(LOOP_CONFIG);
        let state = spec.dir.join(STATE_FILE);
        let original_config = file_text(&config);
        std::fs::write(&state, "sentinel state\n").unwrap();

        let err = init_loop(&cwd, "daily-triage").unwrap_err();

        assert!(err.contains("already exists"), "{err}");
        assert_eq!(file_text(&config), original_config);
        assert_eq!(file_text(&state), "sentinel state\n");
        assert_eq!(list_loops(&cwd).len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn init_loop_custom_name_uses_default_pattern() {
        let root = temp_root("custom-name");
        let cwd = root.to_string_lossy();
        let spec = init_loop(&cwd, "nightly-check").unwrap();

        assert_eq!(spec.id, "nightly-check");
        assert_eq!(spec.pattern, DEFAULT_PATTERN);
        assert_eq!(spec.cadence, "1d");
        assert!(spec.dir.ends_with(".a3s/loops/nightly-check"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn init_loop_custom_name_can_select_known_pattern() {
        let root = temp_root("custom-pattern");
        let cwd = root.to_string_lossy();
        let spec = init_loop(&cwd, "ci-watch ci-sweeper").unwrap();

        assert_eq!(spec.id, "ci-watch");
        assert_eq!(spec.pattern, "ci-sweeper");
        assert_eq!(spec.cadence, "15m");
        assert!(spec.goal.contains("CI/test failures"));
        assert!(spec.budget_tokens_per_day > 120_000);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn init_agent_loop_scopes_to_active_agent_and_disables_os_runtime() {
        let root = temp_root("agent-init");
        let cwd = root.to_string_lossy();
        let agent = agent_dev_session(&root);
        let spec = init_agent_loop(&cwd, "", &agent).unwrap();

        assert_eq!(spec.id, "agent-code-reviewer");
        assert_eq!(spec.pattern, "agent-dev");
        assert_eq!(spec.level, "A2");
        assert!(!spec.os_runtime);
        assert!(!spec.worktree);
        assert!(spec.connectors.is_empty());
        assert!(spec.goal.contains("code-reviewer"));
        assert!(spec.goal.contains("review/code-reviewer.md"));
        assert!(file_text(spec.dir.join(LOOP_CONFIG)).contains("os_runtime = false"));
        assert!(file_text(spec.dir.join(STATE_FILE)).contains("Target Agent"));

        let audit = audit_loop(&spec);
        assert_eq!(audit.score, 100, "{audit:?}");
        assert!(audit.passed.iter().any(|p| p == "local agent loop runtime"));
        assert!(audit
            .passed
            .iter()
            .any(|p| p == "agent asset scope requested"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn init_agent_loop_accepts_custom_id_and_pattern() {
        let root = temp_root("agent-custom");
        let cwd = root.to_string_lossy();
        let agent = agent_dev_session(&root);
        let spec = init_agent_loop(&cwd, "review-watch ci-sweeper", &agent).unwrap();

        assert_eq!(spec.id, "review-watch");
        assert_eq!(spec.pattern, "ci-sweeper");
        assert_eq!(spec.level, "A2");
        assert!(spec.goal.contains("code-reviewer"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn list_and_find_loops_sort_and_match_slug() {
        let root = temp_root("list");
        let cwd = root.to_string_lossy();
        init_loop(&cwd, "zeta-check").unwrap();
        init_loop(&cwd, "nightly-check").unwrap();
        init_loop(&cwd, "alpha-check").unwrap();

        let ids = list_loops(&cwd)
            .into_iter()
            .map(|summary| summary.spec.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["alpha-check", "nightly-check", "zeta-check"]);

        let spec = find_loop(&cwd, "Nightly Check").unwrap();
        assert_eq!(spec.id, "nightly-check");
        assert!(find_loop(&cwd, "missing")
            .unwrap_err()
            .contains("not found"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn audit_loop_downgrades_when_required_artifacts_are_missing() {
        let root = temp_root("audit-missing");
        let cwd = root.to_string_lossy();
        let mut spec = init_loop(&cwd, "daily-triage").unwrap();
        std::fs::remove_file(spec.dir.join(BUDGET_FILE)).unwrap();
        std::fs::remove_file(spec.dir.join("skills").join("verifier.md")).unwrap();
        spec.connectors.clear();

        let audit = audit_loop(&spec);

        assert!(audit.score < 90, "{audit:?}");
        assert!(audit.level == "L2-ready" || audit.level == "L1-ready");
        assert!(
            audit
                .missing
                .iter()
                .any(|m| m.contains("budget.toml") || m.contains("daily caps")),
            "{audit:?}"
        );
        assert!(
            audit
                .missing
                .iter()
                .any(|m| m.contains("os_runtime") || m.contains("os-runtime")),
            "{audit:?}"
        );
        assert!(
            audit
                .missing
                .iter()
                .any(|m| m.contains("triage.md") && m.contains("verifier.md")),
            "{audit:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn append_run_start_records_os_runtime_flag_and_last_run() {
        let root = temp_root("run-log");
        let cwd = root.to_string_lossy();
        let spec = init_loop(&cwd, "daily-triage").unwrap();

        append_run_start(&spec, true).unwrap();

        let log = file_text(spec.dir.join(RUN_LOG_FILE));
        assert!(log.contains("start · level=L1 · os_runtime=true"), "{log}");
        let summaries = list_loops(&cwd);
        assert_eq!(summaries.len(), 1);
        assert!(
            summaries[0]
                .last_run
                .contains("start · level=L1 · os_runtime=true"),
            "{summaries:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parse_spec_reads_lists_and_flags() {
        let spec = parse_spec(
            r#"
id = "ci"
goal = "Fix CI"
level = "L2"
os_runtime = true
worktree = false
denylist = [".env*", "infra/**"]
connectors = ["os-runtime"]
"#,
            PathBuf::from("/tmp/ci"),
        )
        .unwrap();
        assert_eq!(spec.id, "ci");
        assert_eq!(spec.level, "L2");
        assert!(!spec.worktree);
        assert_eq!(spec.denylist, vec![".env*", "infra/**"]);
    }

    #[test]
    fn loop_run_prompt_falls_back_locally_without_os() {
        let root = temp_root("prompt-local");
        let cwd = root.to_string_lossy();
        let spec = init_loop(&cwd, "daily-triage").unwrap();
        let p = loop_run_prompt(&spec, &cwd, false);

        assert!(p.contains("OS is not signed in"), "{p}");
        assert!(
            p.contains("`/login` enables Runtime parallelism and RemoteUI"),
            "{p}"
        );
        assert!(p.contains("Do not claim an OS RemoteUI view exists"), "{p}");
        assert!(!p.contains("OS IS AVAILABLE AND MUST BE USED"), "{p}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn loop_run_prompt_stays_local_for_agent_dev_runtime_mode() {
        let root = temp_root("prompt-agent");
        let cwd = root.to_string_lossy();
        let agent = agent_dev_session(&root);
        let spec = init_agent_loop(&cwd, "", &agent).unwrap();
        let p = loop_run_prompt_with_runtime(&spec, &cwd, LoopRuntimeMode::LocalAgentDev);

        assert!(p.contains("local /agent development mode"), "{p}");
        assert!(p.contains("Stay local even if OS is signed in"), "{p}");
        assert!(p.contains("Do not open OS, WebIDE, RemoteUI"), "{p}");
        assert!(p.contains("Local-only runtime policy"), "{p}");
        assert!(p.contains("A2 agent-development loop"), "{p}");
        assert!(!p.contains("OS IS AVAILABLE AND MUST BE USED"), "{p}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn loop_run_prompt_requires_os_runtime_and_remoteui_when_available() {
        let root = temp_root("prompt");
        let cwd = root.to_string_lossy();
        let spec = init_loop(&cwd, "daily-triage").unwrap();
        let p = loop_run_prompt(&spec, &cwd, true);
        assert!(p.contains("OS IS AVAILABLE AND MUST BE USED"), "{p}");
        assert!(p.contains("Runtime evidence is required"), "{p}");
        assert!(p.contains("parallel_task") && p.contains("RemoteUI"), "{p}");
        assert!(p.contains("A3S Runtime"), "{p}");
        assert!(p.contains("shaped:true"), "{p}");
        assert!(p.contains(".view") && p.contains("viewUrl"), "{p}");
        assert!(p.contains("must include both fan-out"), "{p}");
        assert!(
            p.contains("Markdown report") && p.contains("HTML report"),
            "{p}"
        );
        assert!(
            p.contains("visible through the asset-scoped runtime activity panel"),
            "{p}"
        );
        assert!(p.contains("STATE.md") && p.contains("RUN_LOG.md"), "{p}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn loop_run_prompt_enforces_l1_report_only_policy() {
        let root = temp_root("prompt-l1");
        let cwd = root.to_string_lossy();
        let spec = init_loop(&cwd, "daily-triage").unwrap();
        let p = loop_run_prompt(&spec, &cwd, true);

        assert!(p.contains("L1 report-only"), "{p}");
        assert!(p.contains("do not modify project source files"), "{p}");
        assert!(
            p.contains("Only update this loop's STATE.md, RUN_LOG.md, and reports/ artifacts"),
            "{p}"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
