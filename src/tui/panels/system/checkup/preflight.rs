//! Typed, read-only host inspection for `/checkup`.
//!
//! Potentially blocking filesystem and component probes run outside Tokio's
//! async workers. Only bounded, secret-free summaries cross into the planning
//! prompt; raw ACL contents, component paths, MCP errors, and credentials do
//! not.

use super::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use a3s::components::{ComponentHealthReport, ComponentHealthStatus, ComponentPaths};
use a3s_code_core::mcp::McpServerStatus;
use a3s_code_core::CodeConfig;

const MAX_ACL_BYTES: u64 = 1024 * 1024;
const LARGE_SKILL_BYTES: u64 = 128 * 1024;
const LARGE_INSTRUCTION_BYTES: u64 = 256 * 1024;
const MAX_REPORTED_COMPONENTS: usize = 24;
const MAX_REPORTED_CONFIG_ISSUES: usize = 12;
const MCP_STATUS_TIMEOUT: Duration = Duration::from_secs(2);

pub(in crate::tui) struct CheckupPreflight {
    components: ComponentAudit,
    path: PathAudit,
    config: ConfigAudit,
    skills: SkillAudit,
    instructions: InstructionAudit,
    mcp: McpAudit,
}

impl CheckupPreflight {
    pub(super) fn render(&self) -> String {
        [
            self.components.render(),
            self.path.render(),
            self.config.render(),
            self.skills.render(),
            self.instructions.render(),
            self.mcp.render(),
        ]
        .join("\n")
    }
}

struct CheckupPreflightInput {
    component_paths: ComponentPaths,
    config_path: PathBuf,
    code_config: Arc<CodeConfig>,
    workspace: PathBuf,
    configured_skill_dir: PathBuf,
    indexed_instruction_files: Vec<String>,
}

impl CheckupPreflightInput {
    fn from_app(app: &App) -> Self {
        Self {
            component_paths: app.component_paths.clone(),
            config_path: app.config_path.clone(),
            code_config: Arc::clone(&app.code_config),
            workspace: PathBuf::from(&app.cwd),
            configured_skill_dir: app.asset_directories.skill.clone(),
            indexed_instruction_files: app
                .files
                .iter()
                .filter(|path| {
                    Path::new(path)
                        .file_name()
                        .is_some_and(|name| name == "AGENTS.md")
                })
                .cloned()
                .collect(),
        }
    }

    fn inspect(self) -> HostPreflight {
        let components = ComponentAudit::inspect(&self.component_paths);
        let path = PathAudit::inspect(
            &self.component_paths.current_exe,
            self.component_paths.path_env.as_ref(),
        );
        let config = ConfigAudit::inspect(
            &self.config_path,
            &self.workspace,
            self.component_paths.home.as_deref(),
            self.code_config.as_ref(),
        );
        let skill_dirs = agent_skill_dirs_with_configured(
            &self.workspace.to_string_lossy(),
            &self.configured_skill_dir,
        );
        let skills = SkillAudit::inspect(&skill_dirs);
        let instructions =
            InstructionAudit::inspect(&self.workspace, &self.indexed_instruction_files);
        HostPreflight {
            components,
            path,
            config,
            skills,
            instructions,
            configured_mcp: self.code_config.mcp_servers.len(),
        }
    }
}

struct HostPreflight {
    components: ComponentAudit,
    path: PathAudit,
    config: ConfigAudit,
    skills: SkillAudit,
    instructions: InstructionAudit,
    configured_mcp: usize,
}

pub(super) fn command(app: &App, status_entry: TranscriptEntryId) -> Cmd<Msg> {
    let input = CheckupPreflightInput::from_app(app);
    let session = Arc::clone(&app.session);
    cmd::cmd(move || async move {
        let host = tokio::task::spawn_blocking(move || input.inspect());
        let mcp = tokio::time::timeout(MCP_STATUS_TIMEOUT, session.mcp_status());
        let (host, mcp) = tokio::join!(host, mcp);
        let result = host
            .map_err(|error| format!("host inspection task failed: {error}"))
            .map(|host| CheckupPreflight {
                components: host.components,
                path: host.path,
                config: host.config,
                skills: host.skills,
                instructions: host.instructions,
                mcp: match mcp {
                    Ok(status) => McpAudit::available(host.configured_mcp, status),
                    Err(_) => McpAudit::timed_out(host.configured_mcp),
                },
            });
        Msg::CheckupPreflightCompleted {
            status_entry,
            result,
        }
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ComponentAudit {
    available: bool,
    total: usize,
    ready: usize,
    broken: usize,
    missing: usize,
    unknown: usize,
    affected: Vec<String>,
}

impl ComponentAudit {
    fn inspect(paths: &ComponentPaths) -> Self {
        match a3s::components::component_health_report(paths) {
            Ok(report) => Self::from_report(report),
            Err(_) => {
                tracing::warn!("checkup component health inspection failed");
                Self {
                    available: false,
                    total: 0,
                    ready: 0,
                    broken: 0,
                    missing: 0,
                    unknown: 0,
                    affected: Vec::new(),
                }
            }
        }
    }

    fn from_report(report: ComponentHealthReport) -> Self {
        let mut audit = Self {
            available: true,
            total: report.checks.len(),
            ready: 0,
            broken: 0,
            missing: 0,
            unknown: 0,
            affected: Vec::new(),
        };
        for check in report.checks {
            let label = match check.status {
                ComponentHealthStatus::Ready => {
                    audit.ready += 1;
                    continue;
                }
                ComponentHealthStatus::Broken => {
                    audit.broken += 1;
                    "broken"
                }
                ComponentHealthStatus::Missing => {
                    audit.missing += 1;
                    "missing"
                }
                ComponentHealthStatus::Unknown => {
                    audit.unknown += 1;
                    "unknown"
                }
            };
            if audit.affected.len() < MAX_REPORTED_COMPONENTS {
                audit.affected.push(format!("{}={label}", check.component));
            }
        }
        audit
    }

    fn render(&self) -> String {
        if !self.available {
            return "- component health: inspection unavailable (no probe output exposed)"
                .to_string();
        }
        let mut text = format!(
            "- component health: {} checked; {} ready, {} broken, {} missing, {} unknown",
            self.total, self.ready, self.broken, self.missing, self.unknown
        );
        if !self.affected.is_empty() {
            text.push_str("; affected ");
            text.push_str(&self.affected.join(", "));
        }
        text
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivePathPosition {
    First,
    Shadowed { earlier_distinct: usize },
    NotRepresented,
    PathUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PathAudit {
    candidate_locations: usize,
    distinct_binaries: usize,
    active: ActivePathPosition,
}

impl PathAudit {
    fn inspect(current_exe: &Path, path_env: Option<&OsString>) -> Self {
        let Some(path_env) = path_env else {
            return Self {
                candidate_locations: 0,
                distinct_binaries: 0,
                active: ActivePathPosition::PathUnavailable,
            };
        };
        let binary = format!("a3s{}", std::env::consts::EXE_SUFFIX);
        let candidates = std::env::split_paths(path_env)
            .map(|directory| directory.join(&binary))
            .filter(|candidate| is_executable_file(candidate))
            .collect::<Vec<_>>();
        let distinct = candidates
            .iter()
            .map(|candidate| canonical_key(candidate))
            .collect::<BTreeSet<_>>();
        let active_index = candidates
            .iter()
            .position(|candidate| same_file_or_path(candidate, current_exe));
        let active = match active_index {
            Some(0) => ActivePathPosition::First,
            Some(index) => {
                let active_key = canonical_key(current_exe);
                let earlier_distinct = candidates[..index]
                    .iter()
                    .map(|candidate| canonical_key(candidate))
                    .filter(|candidate| candidate != &active_key)
                    .collect::<BTreeSet<_>>()
                    .len();
                if earlier_distinct == 0 {
                    ActivePathPosition::First
                } else {
                    ActivePathPosition::Shadowed { earlier_distinct }
                }
            }
            None => ActivePathPosition::NotRepresented,
        };
        Self {
            candidate_locations: candidates.len(),
            distinct_binaries: distinct.len(),
            active,
        }
    }

    fn render(&self) -> String {
        let relation = match self.active {
            ActivePathPosition::First => "active executable is the first resolved PATH binary"
                .to_string(),
            ActivePathPosition::Shadowed { earlier_distinct } => format!(
                "active executable is shadowed by {earlier_distinct} earlier distinct PATH binary/binaries"
            ),
            ActivePathPosition::NotRepresented => {
                "active executable is not represented on PATH".to_string()
            }
            ActivePathPosition::PathUnavailable => "PATH is unavailable".to_string(),
        };
        format!(
            "- executable/PATH: {} candidate location(s), {} distinct binary/binaries; {relation}",
            self.candidate_locations, self.distinct_binaries
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigFileStatus {
    Valid,
    InvalidSyntax,
    InvalidStructure,
    Unreadable,
    Oversized,
}

impl ConfigFileStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::InvalidSyntax => "invalid ACL syntax",
            Self::InvalidStructure => "invalid A3S configuration structure",
            Self::Unreadable => "unreadable",
            Self::Oversized => "larger than the 1 MiB audit bound",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfigFileAudit {
    labels: Vec<&'static str>,
    path: PathBuf,
    status: ConfigFileStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfigAudit {
    files: Vec<ConfigFileAudit>,
    effective_issues: Vec<String>,
}

impl ConfigAudit {
    fn inspect(
        effective_path: &Path,
        workspace: &Path,
        home: Option<&Path>,
        effective: &CodeConfig,
    ) -> Self {
        let user = home.map(|home| home.join(".a3s/config.acl"));
        let workspace_config = crate::commands::config_resolver::workspace_config_path(workspace);
        let layered = user
            .as_deref()
            .is_some_and(|path| same_file_or_path(path, effective_path))
            || workspace_config
                .as_deref()
                .is_some_and(|path| same_file_or_path(path, effective_path));
        let mut candidates = Vec::<ConfigFileAudit>::new();
        if layered {
            if let Some(path) = user.filter(|path| path.is_file()) {
                push_config_candidate(&mut candidates, "user", path);
            }
            if let Some(path) = workspace_config {
                push_config_candidate(&mut candidates, "workspace", path);
            }
        }
        push_config_candidate(&mut candidates, "effective", effective_path.to_path_buf());
        for candidate in &mut candidates {
            candidate.status = audit_acl_file(&candidate.path);
        }
        let effective_issues = crate::api::code_web::config::validation::validate_config(effective)
            .into_iter()
            .take(MAX_REPORTED_CONFIG_ISSUES)
            .map(|issue| safe_data(&issue, MAX_HOST_FACT_CHARS))
            .collect();
        Self {
            files: candidates,
            effective_issues,
        }
    }

    fn render(&self) -> String {
        let layers = self
            .files
            .iter()
            .map(|file| {
                format!(
                    "{}={} ({})",
                    file.labels.join("+"),
                    safe_data(&file.path.display().to_string(), MAX_HOST_FACT_CHARS),
                    file.status.label()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let semantics = if self.effective_issues.is_empty() {
            "effective semantic validation passed".to_string()
        } else {
            format!(
                "{} effective semantic issue(s): {}",
                self.effective_issues.len(),
                self.effective_issues.join("; ")
            )
        };
        format!(
            "- ACL configuration: {} inspected layer(s): {layers}; {semantics}",
            self.files.len()
        )
    }
}

fn push_config_candidate(
    candidates: &mut Vec<ConfigFileAudit>,
    label: &'static str,
    path: PathBuf,
) {
    if let Some(existing) = candidates
        .iter_mut()
        .find(|candidate| same_file_or_path(&candidate.path, &path))
    {
        if !existing.labels.contains(&label) {
            existing.labels.push(label);
        }
        return;
    }
    candidates.push(ConfigFileAudit {
        labels: vec![label],
        path,
        status: ConfigFileStatus::Unreadable,
    });
}

fn audit_acl_file(path: &Path) -> ConfigFileStatus {
    let Ok(file) = File::open(path) else {
        return ConfigFileStatus::Unreadable;
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_ACL_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return ConfigFileStatus::Unreadable;
    }
    if bytes.len() as u64 > MAX_ACL_BYTES {
        return ConfigFileStatus::Oversized;
    }
    let Ok(source) = String::from_utf8(bytes) else {
        return ConfigFileStatus::InvalidSyntax;
    };
    if a3s_acl::parse_acl(&source).is_err() {
        return ConfigFileStatus::InvalidSyntax;
    }
    if CodeConfig::from_acl(&source).is_err() {
        return ConfigFileStatus::InvalidStructure;
    }
    ConfigFileStatus::Valid
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SkillAudit {
    directories: usize,
    files: usize,
    total_bytes: u64,
    duplicate_names: usize,
    large_files: usize,
    metadata_failures: usize,
}

impl SkillAudit {
    fn inspect(directories: &[PathBuf]) -> Self {
        let mut audit = Self {
            directories: directories.len(),
            files: 0,
            total_bytes: 0,
            duplicate_names: 0,
            large_files: 0,
            metadata_failures: 0,
        };
        let mut names = BTreeMap::<String, usize>::new();
        for directory in directories {
            let Ok(entries) = std::fs::read_dir(directory) else {
                audit.metadata_failures += 1;
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let (name, skill_file) = if path.is_dir() {
                    let skill = path.join("SKILL.md");
                    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                        continue;
                    };
                    (name.to_string(), skill)
                } else if path.extension().and_then(|value| value.to_str()) == Some("md") {
                    let Some(name) = path.file_stem().and_then(|value| value.to_str()) else {
                        continue;
                    };
                    (name.to_string(), path)
                } else {
                    continue;
                };
                if !skill_file.is_file() {
                    continue;
                }
                audit.files += 1;
                *names.entry(name).or_default() += 1;
                match skill_file.metadata() {
                    Ok(metadata) => {
                        audit.total_bytes = audit.total_bytes.saturating_add(metadata.len());
                        if metadata.len() > LARGE_SKILL_BYTES {
                            audit.large_files += 1;
                        }
                    }
                    Err(_) => audit.metadata_failures += 1,
                }
            }
        }
        audit.duplicate_names = names.values().map(|count| count.saturating_sub(1)).sum();
        audit
    }

    fn render(&self) -> String {
        format!(
            "- skill/plugin context: {} file(s) across {} source dir(s), {}; {} duplicate name(s), {} file(s) over 128 KiB, {} metadata failure(s)",
            self.files,
            self.directories,
            human_bytes(self.total_bytes),
            self.duplicate_names,
            self.large_files,
            self.metadata_failures
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InstructionAudit {
    files: usize,
    total_bytes: u64,
    large_files: usize,
    metadata_failures: usize,
}

impl InstructionAudit {
    fn inspect(workspace: &Path, indexed_files: &[String]) -> Self {
        let mut audit = Self {
            files: indexed_files.len(),
            total_bytes: 0,
            large_files: 0,
            metadata_failures: 0,
        };
        for indexed in indexed_files {
            let path = Path::new(indexed);
            if path
                .components()
                .any(|component| component == Component::ParentDir)
            {
                audit.metadata_failures += 1;
                continue;
            }
            let path = if path.is_absolute() {
                if !path.starts_with(workspace) {
                    audit.metadata_failures += 1;
                    continue;
                }
                path.to_path_buf()
            } else {
                workspace.join(path)
            };
            match path.metadata() {
                Ok(metadata) => {
                    audit.total_bytes = audit.total_bytes.saturating_add(metadata.len());
                    if metadata.len() > LARGE_INSTRUCTION_BYTES {
                        audit.large_files += 1;
                    }
                }
                Err(_) => audit.metadata_failures += 1,
            }
        }
        audit
    }

    fn render(&self) -> String {
        format!(
            "- workspace instructions: {} indexed AGENTS.md file(s), {}; {} file(s) over 256 KiB, {} metadata failure(s)",
            self.files,
            human_bytes(self.total_bytes),
            self.large_files,
            self.metadata_failures
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct McpAudit {
    available: bool,
    configured: usize,
    registered: usize,
    enabled: usize,
    connected: usize,
    tools: usize,
    errors: usize,
}

impl McpAudit {
    fn available(configured: usize, status: HashMap<String, McpServerStatus>) -> Self {
        Self {
            available: true,
            configured,
            registered: status.len(),
            enabled: status.values().filter(|entry| entry.enabled).count(),
            connected: status.values().filter(|entry| entry.connected).count(),
            tools: status.values().map(|entry| entry.tool_count).sum(),
            errors: status
                .values()
                .filter(|entry| entry.error.is_some())
                .count(),
        }
    }

    fn timed_out(configured: usize) -> Self {
        Self {
            available: false,
            configured,
            registered: 0,
            enabled: 0,
            connected: 0,
            tools: 0,
            errors: 0,
        }
    }

    fn render(&self) -> String {
        if !self.available {
            return format!(
                "- MCP runtime: {} configured; in-memory status timed out (servers were not started)",
                self.configured
            );
        }
        format!(
            "- MCP runtime: {} configured, {} registered, {} enabled, {} connected, {} tool(s), {} error state(s); error text withheld",
            self.configured,
            self.registered,
            self.enabled,
            self.connected,
            self.tools,
            self.errors
        )
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn same_file_or_path(left: &Path, right: &Path) -> bool {
    left == right || canonical_key(left) == canonical_key(right)
}

fn canonical_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn safe_data(value: &str, max_chars: usize) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut truncated = normalized
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn human_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    if bytes < 1024 * 1024 {
        return format!("{:.1} KiB", bytes as f64 / 1024.0);
    }
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn write_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn path_audit_detects_a_shadowed_active_executable() {
        let temp = tempfile::tempdir().unwrap();
        let shadow = temp.path().join("shadow/a3s");
        let active = temp.path().join("active/a3s");
        write_executable(&shadow);
        write_executable(&active);
        let path =
            std::env::join_paths([shadow.parent().unwrap(), active.parent().unwrap()]).unwrap();

        let audit = PathAudit::inspect(&active, Some(&path));

        assert_eq!(audit.candidate_locations, 2);
        assert_eq!(audit.distinct_binaries, 2);
        assert_eq!(
            audit.active,
            ActivePathPosition::Shadowed {
                earlier_distinct: 1
            }
        );
    }

    #[test]
    fn acl_audit_is_bounded_and_never_needs_secret_values() {
        let temp = tempfile::tempdir().unwrap();
        let valid = temp.path().join("valid.acl");
        std::fs::write(&valid, crate::config::config_template()).unwrap();
        assert_eq!(audit_acl_file(&valid), ConfigFileStatus::Valid);

        let oversized = temp.path().join("oversized.acl");
        std::fs::write(&oversized, vec![b'x'; MAX_ACL_BYTES as usize + 1]).unwrap();
        assert_eq!(audit_acl_file(&oversized), ConfigFileStatus::Oversized);
    }

    #[test]
    fn skill_audit_counts_duplicate_names_and_context_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for root in [&first, &second] {
            let skill = root.join("shared/SKILL.md");
            std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
            std::fs::write(skill, "---\nname: shared\n---\nbody\n").unwrap();
        }

        let audit = SkillAudit::inspect(&[first, second]);

        assert_eq!(audit.files, 2);
        assert_eq!(audit.duplicate_names, 1);
        assert!(audit.total_bytes > 0);
    }

    #[test]
    fn mcp_audit_exposes_counts_but_not_error_text() {
        let status = HashMap::from([(
            "private".to_string(),
            McpServerStatus {
                name: "private".to_string(),
                connected: false,
                enabled: true,
                tool_count: 0,
                error: Some("Bearer top-secret-token".to_string()),
            },
        )]);

        let rendered = McpAudit::available(1, status).render();

        assert!(rendered.contains("1 error state"));
        assert!(!rendered.contains("top-secret-token"));
    }
}
