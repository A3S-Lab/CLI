//! Managed isolated-worktree status, handoff, and cleanup guidance.

use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Error, ErrorKind, Write};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const WORKTREE_LIFECYCLE_SCHEMA_VERSION: u32 = 1;
const MAX_WORKTREE_LIFECYCLE_BYTES: u64 = 256 * 1024;
const MAX_GIT_DIAGNOSTIC_BYTES: usize = 1024 * 1024;
static WORKTREE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ManagedWorktreeState {
    schema_version: u32,
    session_id: String,
    source_repository: PathBuf,
    worktree_root: PathBuf,
    workspace: PathBuf,
    branch: String,
    base_commit: String,
    created_at_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeHandoffManifest<'a> {
    schema: &'static str,
    session_id: &'a str,
    source_repository: &'a Path,
    worktree_root: &'a Path,
    workspace: &'a Path,
    branch: &'a str,
    base_commit: &'a str,
    patch_path: &'a Path,
    patch_sha256: &'a str,
    patch_bytes: usize,
    created_at_ms: u64,
}

pub(super) fn parse_worktree_lifecycle_command(
    rest: &str,
) -> Result<WorktreeLifecycleCommand, &'static str> {
    match rest.trim() {
        "" | "status" => Ok(WorktreeLifecycleCommand::Status),
        "handoff" => Ok(WorktreeLifecycleCommand::Handoff),
        "cleanup" => Ok(WorktreeLifecycleCommand::Cleanup),
        _ => Err("usage: /worktree [status|handoff|cleanup]"),
    }
}

impl ManagedWorktreeState {
    pub(super) fn from_fork(result: &WorktreeForkResult) -> Self {
        Self {
            schema_version: WORKTREE_LIFECYCLE_SCHEMA_VERSION,
            session_id: result.session_id.clone(),
            source_repository: result.source_repository.clone(),
            worktree_root: result.worktree_root.clone(),
            workspace: result.workspace.clone(),
            branch: result.branch.clone(),
            base_commit: result.base_commit.clone(),
            created_at_ms: epoch_ms(),
        }
    }
}

pub(super) fn save_managed_worktree_state(state: &ManagedWorktreeState) -> std::io::Result<()> {
    if state.schema_version != WORKTREE_LIFECYCLE_SCHEMA_VERSION
        || state.session_id.trim().is_empty()
        || !valid_object_id(&state.base_commit)
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "invalid managed worktree lifecycle state",
        ));
    }
    let mut body = serde_json::to_vec_pretty(state)
        .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
    body.push(b'\n');
    if body.len() as u64 > MAX_WORKTREE_LIFECYCLE_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "managed worktree lifecycle state exceeds 256 KiB",
        ));
    }
    write_new_or_identical(
        &managed_worktree_state_path(&state.workspace, &state.session_id),
        &body,
        MAX_WORKTREE_LIFECYCLE_BYTES,
    )
}

fn load_managed_worktree_state(
    workspace: &Path,
    session_id: &str,
) -> Result<ManagedWorktreeState, String> {
    let path = managed_worktree_state_path(workspace, session_id);
    let metadata = fs::metadata(&path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            "this session is not attached to an A3S-managed worktree; create one with `/fork worktree`"
                .to_string()
        } else {
            format!("could not inspect managed worktree state {}: {error}", path.display())
        }
    })?;
    if !metadata.is_file() || metadata.len() > MAX_WORKTREE_LIFECYCLE_BYTES {
        return Err(format!(
            "managed worktree state is not a bounded regular file: {}",
            path.display()
        ));
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("could not read managed worktree state: {error}"))?;
    let state: ManagedWorktreeState = serde_json::from_slice(&bytes)
        .map_err(|error| format!("managed worktree state is invalid: {error}"))?;
    validate_managed_worktree_state(workspace, session_id, state)
}

fn validate_managed_worktree_state(
    workspace: &Path,
    session_id: &str,
    state: ManagedWorktreeState,
) -> Result<ManagedWorktreeState, String> {
    if state.schema_version != WORKTREE_LIFECYCLE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported managed worktree schema {}; expected {}",
            state.schema_version, WORKTREE_LIFECYCLE_SCHEMA_VERSION
        ));
    }
    if state.session_id != session_id {
        return Err("managed worktree state belongs to another session".to_string());
    }
    if !valid_object_id(&state.base_commit) {
        return Err("managed worktree state has an invalid base commit".to_string());
    }

    let workspace = canonical(workspace, "current workspace")?;
    let recorded_workspace = canonical(&state.workspace, "recorded worktree workspace")?;
    let worktree_root = canonical(&state.worktree_root, "recorded worktree root")?;
    let source_repository = canonical(&state.source_repository, "source repository")?;
    if workspace != recorded_workspace || !workspace.starts_with(&worktree_root) {
        return Err("managed worktree state does not match the current workspace".to_string());
    }
    if git_common_directory(&worktree_root)? != git_common_directory(&source_repository)? {
        return Err("managed worktree source is not part of the same Git repository".to_string());
    }
    let current_branch = git_text_bounded(
        &worktree_root,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?;
    if current_branch.trim() != state.branch {
        return Err("managed worktree branch no longer matches its recorded identity".to_string());
    }

    Ok(ManagedWorktreeState {
        source_repository,
        worktree_root,
        workspace,
        ..state
    })
}

fn managed_worktree_state_path(workspace: &Path, session_id: &str) -> PathBuf {
    let key = URL_SAFE_NO_PAD.encode(session_id.as_bytes());
    workspace
        .join(".a3s")
        .join("tui")
        .join("worktrees")
        .join("v1")
        .join(format!("id_{key}.json"))
}

impl App {
    pub(super) fn submit_worktree_lifecycle_command(&mut self, rest: &str) -> Option<Cmd<Msg>> {
        let command = match parse_worktree_lifecycle_command(rest) {
            Ok(command) => command,
            Err(usage) => {
                self.textarea.clear();
                self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  {usage}")));
                return None;
            }
        };
        self.textarea.clear();
        let request_id = self.reserve_fork_request();
        let workspace = PathBuf::from(&self.cwd);
        let session_id = self.session_id.clone();
        self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
            "  inspecting managed worktree {}…",
            command.label()
        )));

        Some(cmd::cmd(move || async move {
            let result = tokio::task::spawn_blocking(move || {
                run_worktree_lifecycle_command(command, &workspace, &session_id)
            })
            .await
            .map_err(|error| format!("worktree task failed: {error}"))
            .and_then(|result| result);
            Msg::WorktreeLifecycleFinished {
                request_id,
                command,
                result,
            }
        }))
    }

    pub(super) fn finish_worktree_lifecycle_command(
        &mut self,
        request_id: u64,
        command: WorktreeLifecycleCommand,
        result: Result<WorktreeLifecycleResult, String>,
    ) -> Option<Cmd<Msg>> {
        if self.session_rebuild_pending != Some(request_id) {
            return None;
        }
        self.session_rebuild_pending = None;
        match result {
            Ok(result) => {
                self.push_line(&gutter(TN_CYAN, &result.title));
                for line in result.lines {
                    self.push_line(&Style::new().fg(TN_FG).render(&format!("  {line}")));
                }
            }
            Err(error) => self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render(&format!("  /worktree {}: {error}", command.label())),
            ),
        }
        self.drain_queue()
    }
}

fn run_worktree_lifecycle_command(
    command: WorktreeLifecycleCommand,
    workspace: &Path,
    session_id: &str,
) -> Result<WorktreeLifecycleResult, String> {
    let state = load_managed_worktree_state(workspace, session_id)?;
    match command {
        WorktreeLifecycleCommand::Status => worktree_status(&state),
        WorktreeLifecycleCommand::Handoff => create_worktree_handoff(&state),
        WorktreeLifecycleCommand::Cleanup => Ok(worktree_cleanup_guidance(&state)),
    }
}

fn worktree_status(state: &ManagedWorktreeState) -> Result<WorktreeLifecycleResult, String> {
    let dirty = git_text_bounded(
        &state.worktree_root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    let changed = dirty.lines().filter(|line| !line.trim().is_empty()).count();
    let revision = format!("{}..HEAD", state.base_commit);
    let commits = git_text_bounded(
        &state.worktree_root,
        &["rev-list", "--count", revision.as_str()],
    )?
    .trim()
    .parse::<usize>()
    .map_err(|_| "Git returned an invalid worktree commit count".to_string())?;
    let branch = git_text_bounded(
        &state.worktree_root,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?;
    let branch = branch.trim();

    Ok(WorktreeLifecycleResult {
        title: "managed worktree status".to_string(),
        lines: vec![
            format!("branch: {branch}"),
            format!("base: {}", state.base_commit),
            format!("source: {}", state.source_repository.display()),
            format!("worktree: {}", state.worktree_root.display()),
            format!("changes: {changed} file(s) · {commits} commit(s) since base"),
            "handoff: /worktree handoff".to_string(),
            "cleanup preview: /worktree cleanup".to_string(),
        ],
    })
}

fn create_worktree_handoff(
    state: &ManagedWorktreeState,
) -> Result<WorktreeLifecycleResult, String> {
    let snapshot = GitTreeSnapshot::capture(&state.workspace).map_err(|error| error.to_string())?;
    if snapshot.repository_root() != state.worktree_root {
        return Err("current Git root does not match managed worktree state".to_string());
    }
    let patch = snapshot
        .patch_from_commit(&state.base_commit)
        .map_err(|error| error.to_string())?;
    if patch.is_empty() {
        let cleanup = worktree_cleanup_guidance(state);
        return Ok(WorktreeLifecycleResult {
            title: "managed worktree has no changes to hand off".to_string(),
            lines: cleanup.lines,
        });
    }

    let sha256 = format!("{:x}", Sha256::digest(patch.bytes()));
    let key = URL_SAFE_NO_PAD.encode(state.session_id.as_bytes());
    let directory = state
        .source_repository
        .join(".a3s")
        .join("tui")
        .join("worktree-handoffs")
        .join("v1");
    let stem = format!("id_{key}-{}", &sha256[..12]);
    let patch_path = directory.join(format!("{stem}.patch"));
    let manifest_path = directory.join(format!("{stem}.json"));
    write_new_or_identical(&patch_path, patch.bytes(), MAX_BINARY_HANDOFF_BYTES)
        .map_err(|error| format!("could not persist handoff patch: {error}"))?;

    let manifest = WorktreeHandoffManifest {
        schema: "a3s.code.worktree-handoff.v1",
        session_id: &state.session_id,
        source_repository: &state.source_repository,
        worktree_root: &state.worktree_root,
        workspace: &state.workspace,
        branch: &state.branch,
        base_commit: &state.base_commit,
        patch_path: &patch_path,
        patch_sha256: &sha256,
        patch_bytes: patch.bytes().len(),
        // Keep the manifest deterministic for an unchanged handoff so the
        // digest-derived path can be recreated idempotently.
        created_at_ms: state.created_at_ms,
    };
    let mut body = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("could not encode handoff manifest: {error}"))?;
    body.push(b'\n');
    write_new_or_identical(&manifest_path, &body, MAX_WORKTREE_LIFECYCLE_BYTES).map_err(
        |error| {
            format!(
                "could not persist handoff manifest: {error}; patch retained at {}",
                patch_path.display()
            )
        },
    )?;

    Ok(WorktreeLifecycleResult {
        title: "digest-bound worktree handoff ready".to_string(),
        lines: vec![
            format!("patch: {}", patch_path.display()),
            format!("manifest: {}", manifest_path.display()),
            format!("sha256: {sha256} · {} byte(s)", patch.bytes().len()),
            format!(
                "inspect: git -C {} apply --stat {}",
                shell_single_quote(&state.source_repository.to_string_lossy()),
                shell_single_quote(&patch_path.to_string_lossy())
            ),
            format!(
                "apply: git -C {} apply --3way {}",
                shell_single_quote(&state.source_repository.to_string_lossy()),
                shell_single_quote(&patch_path.to_string_lossy())
            ),
            "After applying and verifying, use `/worktree cleanup` for safe removal commands."
                .to_string(),
        ],
    })
}

fn worktree_cleanup_guidance(state: &ManagedWorktreeState) -> WorktreeLifecycleResult {
    WorktreeLifecycleResult {
        title: "managed worktree cleanup preview".to_string(),
        lines: vec![
            "Exit every process using the isolated workspace, then run from another directory:"
                .to_string(),
            format!(
                "git -C {} worktree remove {}",
                shell_single_quote(&state.source_repository.to_string_lossy()),
                shell_single_quote(&state.worktree_root.to_string_lossy())
            ),
            format!(
                "git -C {} branch -d {}",
                shell_single_quote(&state.source_repository.to_string_lossy()),
                shell_single_quote(&state.branch)
            ),
            "Both commands fail closed when work is uncommitted or the branch is not integrated; no force flag is suggested."
                .to_string(),
        ],
    }
}

fn git_common_directory(workspace: &Path) -> Result<PathBuf, String> {
    let value = git_text_bounded(
        workspace,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    canonical(Path::new(value.trim()), "Git common directory")
}

fn git_text_bounded(workspace: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .map_err(|error| format!("could not launch Git: {error}"))?;
    if output.stdout.len() > MAX_GIT_DIAGNOSTIC_BYTES
        || output.stderr.len() > MAX_GIT_DIAGNOSTIC_BYTES
    {
        return Err("Git diagnostic output exceeded 1 MiB".to_string());
    }
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {} failed: {}", args.join(" "), detail.trim()));
    }
    String::from_utf8(output.stdout).map_err(|_| "Git output was not UTF-8".to_string())
}

fn canonical(path: &Path, label: &str) -> Result<PathBuf, String> {
    canonical_git_path(path)
        .map_err(|error| format!("could not resolve {label} {}: {error}", path.display()))
}

fn valid_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn write_new_or_identical(
    path: &Path,
    body: &[u8],
    max_existing_bytes: u64,
) -> std::io::Result<()> {
    if path.exists() {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() > max_existing_bytes {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!(
                    "existing handoff path is not a bounded regular file: {}",
                    path.display()
                ),
            ));
        }
        if fs::read(path)? == body {
            return Ok(());
        }
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!(
                "refusing to replace different existing content: {}",
                path.display()
            ),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "worktree state path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = temporary_path(path);
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(body)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

const MAX_BINARY_HANDOFF_BYTES: u64 = 128 * 1024 * 1024 + 1024 * 1024;

fn temporary_path(path: &Path) -> PathBuf {
    let sequence = WORKTREE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("worktree-state");
    path.with_file_name(format!(
        ".{file_name}.tmp-{}-{sequence}",
        std::process::id()
    ))
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(repository: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("temporary repository root");
        let repository = root.path().join("repository");
        fs::create_dir_all(&repository).unwrap();
        git(&repository, &["init"]);
        git(&repository, &["config", "user.name", "A3S Test"]);
        git(
            &repository,
            &["config", "user.email", "a3s-test@example.invalid"],
        );
        fs::write(repository.join("tracked.txt"), "base\n").unwrap();
        git(&repository, &["add", "tracked.txt"]);
        git(&repository, &["commit", "-m", "initial"]);
        (root, repository)
    }

    #[test]
    fn parser_exposes_bounded_lifecycle_actions() {
        assert_eq!(
            parse_worktree_lifecycle_command(""),
            Ok(WorktreeLifecycleCommand::Status)
        );
        assert_eq!(
            parse_worktree_lifecycle_command(" handoff"),
            Ok(WorktreeLifecycleCommand::Handoff)
        );
        assert_eq!(
            parse_worktree_lifecycle_command("cleanup "),
            Ok(WorktreeLifecycleCommand::Cleanup)
        );
        assert!(parse_worktree_lifecycle_command("remove --force").is_err());
    }

    #[test]
    fn cleanup_guidance_never_suggests_force() {
        let state = ManagedWorktreeState {
            schema_version: WORKTREE_LIFECYCLE_SCHEMA_VERSION,
            session_id: "session-1".to_string(),
            source_repository: PathBuf::from("/source repo"),
            worktree_root: PathBuf::from("/worktree path"),
            workspace: PathBuf::from("/worktree path"),
            branch: "a3s/fork-session-1".to_string(),
            base_commit: "a".repeat(40),
            created_at_ms: 1,
        };
        let rendered = worktree_cleanup_guidance(&state).lines.join("\n");
        assert!(rendered.contains("worktree remove"));
        assert!(rendered.contains("branch -d"));
        assert!(!rendered.contains("--force"));
        assert!(!rendered.contains("branch -D"));
    }

    #[test]
    fn handoff_captures_committed_uncommitted_and_untracked_content() {
        let (_root, repository) = repository();
        let isolated = GitTreeSnapshot::capture(&repository)
            .unwrap()
            .fork_worktree("handoff-fixture")
            .unwrap();
        let result = WorktreeForkResult {
            session_id: "handoff-session".to_string(),
            workspace: isolated.workspace.clone(),
            worktree_root: isolated.root.clone(),
            branch: isolated.branch.clone(),
            source_repository: isolated.source_repository.clone(),
            base_commit: isolated.base_commit.clone(),
        };
        let state = ManagedWorktreeState::from_fork(&result);
        save_managed_worktree_state(&state).unwrap();

        fs::write(isolated.workspace.join("tracked.txt"), "committed\n").unwrap();
        git(&isolated.root, &["add", "tracked.txt"]);
        git(&isolated.root, &["commit", "-m", "worktree commit"]);
        fs::write(isolated.workspace.join("tracked.txt"), "uncommitted\n").unwrap();
        fs::write(isolated.workspace.join("untracked.txt"), "new\n").unwrap();

        let handoff = run_worktree_lifecycle_command(
            WorktreeLifecycleCommand::Handoff,
            &isolated.workspace,
            &result.session_id,
        )
        .unwrap();
        assert!(handoff.title.contains("handoff ready"));
        let patch_path = handoff.lines[0]
            .strip_prefix("patch: ")
            .map(PathBuf::from)
            .expect("patch path");
        assert!(patch_path.is_file());
        let body = fs::read(&patch_path).unwrap();
        assert!(String::from_utf8_lossy(&body).contains("untracked.txt"));

        let repeated = run_worktree_lifecycle_command(
            WorktreeLifecycleCommand::Handoff,
            &isolated.workspace,
            &result.session_id,
        )
        .unwrap();
        assert_eq!(repeated, handoff);

        let patch = patch_path.to_string_lossy().into_owned();
        git(&repository, &["apply", "--check", &patch]);

        git(
            &repository,
            &[
                "worktree",
                "remove",
                "--force",
                isolated.root.to_string_lossy().as_ref(),
            ],
        );
        git(&repository, &["branch", "-D", &isolated.branch]);
    }
}
