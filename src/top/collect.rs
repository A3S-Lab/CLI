//! Process model + collector shared by `a3s top` and the `/top` panel in
//! `a3s code`. Kept independent from the TUI layer so the same `ProcessRow`
//! snapshot feeds both renderers (and `--json`).

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use a3s_tui::style::Color;
use futures::stream::{self, StreamExt};
use tokio::process::Command;

use super::{ACCENT, GREEN, ORANGE, RED, YELLOW};

/// A coding agent recognised from a process command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentKind {
    A3sCode,
    ClaudeCode,
    Codex,
    Cursor,
    Gemini,
}

impl AgentKind {
    pub(crate) const ALL: [AgentKind; 5] = [
        AgentKind::A3sCode,
        AgentKind::ClaudeCode,
        AgentKind::Codex,
        AgentKind::Cursor,
        AgentKind::Gemini,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            AgentKind::A3sCode => "a3s-code",
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Cursor => "cursor",
            AgentKind::Gemini => "gemini",
        }
    }

    pub(crate) fn color(self) -> Color {
        match self {
            AgentKind::A3sCode => ACCENT,
            AgentKind::ClaudeCode => ORANGE,
            AgentKind::Codex => Color::Rgb(16, 163, 127),
            AgentKind::Cursor => Color::Rgb(180, 182, 200),
            AgentKind::Gemini => Color::Rgb(124, 137, 245),
        }
    }
}

/// Coarse risk classification for a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "med",
            Risk::High => "high",
        }
    }

    pub(crate) fn color(self) -> Color {
        match self {
            Risk::Low => GREEN,
            Risk::Medium => YELLOW,
            Risk::High => RED,
        }
    }
}

/// One host process row.
#[derive(Debug, Clone)]
pub(crate) struct ProcessRow {
    pub(crate) pid: u32,
    pub(crate) ppid: u32,
    pub(crate) cpu_pct: f32,
    pub(crate) mem_pct: f32,
    pub(crate) elapsed: String,
    pub(crate) cwd: Option<String>,
    pub(crate) command: String,
    pub(crate) agent: Option<AgentKind>,
    pub(crate) risk: Risk,
}

/// Snapshot the host process table via `ps`, sorted agents-first then CPU desc.
pub(crate) async fn collect_processes() -> anyhow::Result<Vec<ProcessRow>> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,pcpu=,pmem=,etime=,args="])
        .output()
        .await?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("ps exited with status {}", output.status));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut rows = text
        .lines()
        .filter_map(parse_process_line)
        .collect::<Vec<_>>();
    enrich_agent_process_cwds(&mut rows).await;
    rows.sort_by(|a, b| {
        b.agent.is_some().cmp(&a.agent.is_some()).then(
            b.cpu_pct
                .partial_cmp(&a.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    Ok(rows)
}

pub(crate) fn parse_process_line(line: &str) -> Option<ProcessRow> {
    let mut it = line.split_whitespace();
    let pid = it.next()?.parse().ok()?;
    let ppid = it.next()?.parse().ok()?;
    let cpu_pct = it.next()?.parse().ok()?;
    let mem_pct = it.next()?.parse().ok()?;
    let elapsed = it.next()?.to_string();
    // Command is the verbatim remainder after the 5 fixed columns — slice it
    // from the original line instead of collect()+join() to avoid a Vec alloc
    // per process per tick (and to preserve the command's internal spacing).
    let command = remainder_after_fields(line, 5)?.trim_end();
    if command.is_empty() {
        return None;
    }
    let command = command.to_string();
    let agent = detect_agent(&command);
    Some(ProcessRow {
        pid,
        ppid,
        cpu_pct,
        mem_pct,
        elapsed,
        cwd: None,
        risk: process_risk(&command, agent),
        command,
        agent,
    })
}

/// Return the slice of `line` after skipping `n` whitespace-separated fields.
fn remainder_after_fields(line: &str, n: usize) -> Option<&str> {
    let mut rest = line.trim_start();
    for _ in 0..n {
        let end = rest.find(char::is_whitespace)?;
        rest = rest[end..].trim_start();
    }
    Some(rest)
}

/// Process CWDs change rarely, so cache them by pid across refreshes; only
/// brand-new agent pids pay the `lsof`/proc lookup. Pruned to live pids each
/// pass, so the map stays bounded by the agent processes on the host.
// Process-global cache shared by both `top` callers; pid cwd is stable enough
// for display and the map is pruned to live pids every call.
static CWD_CACHE: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();

async fn enrich_agent_process_cwds(rows: &mut [ProcessRow]) {
    let cache = CWD_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let live_pids: HashSet<u32> = rows.iter().map(|row| row.pid).collect();

    // Serve cached cwds and collect the misses (capped at 16 lookups/pass).
    let mut misses = Vec::new();
    {
        let map = cache.lock().unwrap();
        for row in rows.iter_mut() {
            if row.agent.is_some() {
                match map.get(&row.pid) {
                    Some(cwd) => row.cwd = Some(cwd.clone()),
                    None => misses.push(row.pid),
                }
            }
        }
    }
    misses.truncate(16);

    if !misses.is_empty() {
        let resolved: Vec<(u32, Option<String>)> = stream::iter(misses)
            .map(|pid| async move { (pid, process_cwd(pid).await) })
            .buffer_unordered(8)
            .collect()
            .await;
        let mut map = cache.lock().unwrap();
        for (pid, cwd) in resolved {
            if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
                map.insert(pid, cwd);
            }
        }
        for row in rows.iter_mut() {
            if row.agent.is_some() && row.cwd.is_none() {
                row.cwd = map.get(&row.pid).cloned();
            }
        }
    }

    cache
        .lock()
        .unwrap()
        .retain(|pid, _| live_pids.contains(pid));
}

async fn process_cwd(pid: u32) -> Option<String> {
    // `/proc/<pid>/cwd` only exists on Linux; macOS/BSD fall straight to lsof.
    #[cfg(target_os = "linux")]
    {
        if let Ok(path) = tokio::fs::read_link(format!("/proc/{pid}/cwd")).await {
            return Some(path.display().to_string());
        }
    }
    process_cwd_from_lsof(pid).await
}

async fn process_cwd_from_lsof(pid: u32) -> Option<String> {
    let mut command = Command::new("lsof");
    let pid = pid.to_string();
    command.args(["-a", "-p", &pid, "-d", "cwd", "-Fn"]);
    let output = tokio::time::timeout(Duration::from_millis(200), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_lsof_cwd(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn parse_lsof_cwd(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix('n'))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn detect_agent(command: &str) -> Option<AgentKind> {
    let l = command.to_lowercase();
    if l.contains("a3s-code") || l.contains("a3s code") || l.ends_with("/a3s") {
        Some(AgentKind::A3sCode)
    } else if l.contains("claude") {
        Some(AgentKind::ClaudeCode)
    } else if l.contains("codex") {
        Some(AgentKind::Codex)
    } else if l.contains("cursor-agent") || l.contains("cursor") {
        Some(AgentKind::Cursor)
    } else if l.contains("gemini") {
        Some(AgentKind::Gemini)
    } else {
        None
    }
}

pub(crate) fn process_risk(command: &str, agent: Option<AgentKind>) -> Risk {
    let lower = command.to_lowercase();
    if lower.contains("sudo ")
        || lower.contains(" rm -rf ")
        || lower.contains("ptrace")
        || lower.contains("nmap ")
    {
        Risk::High
    } else if agent.is_some()
        || lower.contains("docker ")
        || lower.contains("curl ")
        || lower.contains("bash -c")
    {
        Risk::Medium
    } else {
        Risk::Low
    }
}
