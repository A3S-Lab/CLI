//! Machine-checkable ACCEPTANCE.md criteria for durable `/goal` latching.
//!
//! Checkbox progress alone is not enough: `kind:command` and `kind:file_exists`
//! criteria are re-verified by the host before GoalAchieved can close the loop.
//! `kind:file_exists` requires a regular file (`Path::is_file`), matching Core's
//! synthesized `test -f` predicate — directories must not latch. Resolved paths must
//! stay inside the workspace (absolute/`../` escapes and outbound symlinks
//! cannot latch). `kind:manual` (default) still requires `- [x]` only.
//!
//! Passing machine criteria also produce **evidence fingerprints** written into
//! STATE.md so makers can skip re-scanning unchanged work. Fingerprints never
//! replace the host latch re-check.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

const ACCEPTANCE_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const VERIFIED_EVIDENCE_HEADING: &str = "## Verified Evidence";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CriterionKind {
    Manual,
    Command { command: String, expect_exit: i32 },
    FileExists { path: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AcceptanceCriterion {
    pub(super) checked: bool,
    pub(super) kind: CriterionKind,
}

/// Count Markdown task-list criteria in ACCEPTANCE.md.
///
/// Only `- [ ]` / `- [x]` (any case) items count. Prose bullets are ignored so
/// guidance text cannot inflate progress.
pub(crate) fn acceptance_criteria_progress(acceptance: &str) -> (usize, usize) {
    let criteria = parse_acceptance_criteria(acceptance);
    let total = criteria.len();
    let done = criteria.iter().filter(|c| c.checked).count();
    (done, total)
}

pub(super) fn parse_acceptance_criteria(acceptance: &str) -> Vec<AcceptanceCriterion> {
    let mut out = Vec::new();
    for line in acceptance.lines() {
        let trimmed = line.trim_start();
        let rest = if let Some(rest) = trimmed.strip_prefix("- [") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("* [") {
            rest
        } else {
            continue;
        };
        let Some((mark, body)) = rest.split_once(']') else {
            continue;
        };
        let mark = mark.trim();
        let checked = mark.eq_ignore_ascii_case("x");
        if !(checked || mark.is_empty() || mark == " ") {
            continue;
        }
        let body = body.trim().trim_start_matches(':').trim();
        out.push(AcceptanceCriterion {
            checked,
            kind: parse_criterion_kind(body),
        });
    }
    out
}

fn parse_criterion_kind(body: &str) -> CriterionKind {
    let lower = body.to_ascii_lowercase();
    if lower.starts_with("kind:command") {
        let command = extract_assert_command(body).unwrap_or_default();
        let expect_exit = extract_expect_exit(body).unwrap_or(0);
        if command.is_empty() {
            return CriterionKind::Manual;
        }
        return CriterionKind::Command {
            command,
            expect_exit,
        };
    }
    if lower.starts_with("kind:file_exists") || lower.starts_with("kind:file-exists") {
        let path = extract_assert_path(body).unwrap_or_default();
        if path.is_empty() {
            return CriterionKind::Manual;
        }
        return CriterionKind::FileExists { path };
    }
    CriterionKind::Manual
}

fn extract_assert_command(body: &str) -> Option<String> {
    let idx = body.to_ascii_lowercase().find("assert:")?;
    let after = body[idx + "assert:".len()..].trim_start();
    if let Some(rest) = after.strip_prefix('`') {
        let end = rest.find('`')?;
        let command = rest[..end].trim();
        if command.is_empty() {
            return None;
        }
        return Some(command.to_string());
    }
    let command = after
        .split_whitespace()
        .next()
        .filter(|token| !token.to_ascii_lowercase().starts_with("expect:"))?;
    Some(command.to_string())
}

fn extract_assert_path(body: &str) -> Option<String> {
    let idx = body.to_ascii_lowercase().find("assert:")?;
    let after = body[idx + "assert:".len()..].trim_start();
    if let Some(rest) = after.strip_prefix('`') {
        let end = rest.find('`')?;
        let path = rest[..end].trim();
        if path.is_empty() {
            return None;
        }
        return Some(path.to_string());
    }
    let path = after
        .split_whitespace()
        .next()
        .filter(|token| !token.to_ascii_lowercase().starts_with("expect:"))?;
    Some(path.to_string())
}

fn extract_expect_exit(body: &str) -> Option<i32> {
    let lower = body.to_ascii_lowercase();
    let idx = lower.find("expect:exit=")?;
    let after = &body[idx + "expect:exit=".len()..];
    let digits: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse().ok()
}

/// Workspace root that owns `.a3s/loops/<id>/`.
pub(super) fn workspace_root_from_loop_dir(loop_dir: &Path) -> Option<PathBuf> {
    // <cwd>/.a3s/loops/<id>
    Some(loop_dir.parent()?.parent()?.parent()?.to_path_buf())
}

/// Re-run machine-checkable checked criteria. Manual `- [x]` items are trusted
/// as checkbox progress only (already required by the host latch).
///
/// Fail-closed: ACCEPTANCE must include at least one `kind:command` or
/// `kind:file_exists` criterion. Manual-only contracts cannot latch — otherwise
/// a Core workspace preset report (e.g. `cargo test`) plus checked manuals
/// would false-complete the durable goal.
pub(super) fn verify_checked_machine_criteria(
    acceptance: &str,
    workspace: &Path,
) -> Result<(), String> {
    let criteria = parse_acceptance_criteria(acceptance);
    if !criteria
        .iter()
        .any(|criterion| !matches!(criterion.kind, CriterionKind::Manual))
    {
        return Err(
            "ACCEPTANCE needs at least one kind:command or kind:file_exists criterion (manual-only cannot latch)"
                .to_string(),
        );
    }
    for (index, criterion) in criteria.into_iter().enumerate() {
        if !criterion.checked {
            continue;
        }
        verify_one_machine_criterion(&criterion, workspace)
            .map_err(|error| format!("criterion #{} {error}", index + 1))?;
    }
    Ok(())
}

fn verify_one_machine_criterion(
    criterion: &AcceptanceCriterion,
    workspace: &Path,
) -> Result<(), String> {
    match &criterion.kind {
        CriterionKind::Manual => Ok(()),
        CriterionKind::Command {
            command,
            expect_exit,
        } => run_acceptance_command(workspace, command, *expect_exit)
            .map_err(|error| format!("command re-check failed: {error}")),
        CriterionKind::FileExists { path } => {
            // Durable goals prove workspace outcomes: absolute/`../` escapes and
            // symlinks that resolve outside the workspace cannot latch.
            let full = resolve_workspace_file(workspace, path)?;
            // Match Core's synthesized `test -f` (regular file only; dirs fail).
            if full.is_file() {
                Ok(())
            } else {
                Err(format!("file_exists missing regular file: {path}"))
            }
        }
    }
}

/// Resolve a `kind:file_exists` assert path to a regular file under `workspace`.
///
/// Rejects absolute paths and relative `..` chains that leave the workspace, and
/// rejects symlinks whose canonical target is outside the workspace.
pub(super) fn resolve_workspace_file(workspace: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("file_exists path is empty".to_string());
    }
    if path_lexically_escapes_workspace(path) {
        return Err(format!("file_exists path escapes workspace: {path}"));
    }
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        workspace.join(path)
    };
    let workspace_canon = workspace
        .canonicalize()
        .map_err(|error| format!("file_exists workspace canonicalize failed: {error}"))?;
    if !candidate.is_file() {
        return Err(format!("file_exists missing regular file: {path}"));
    }
    let file_canon = candidate
        .canonicalize()
        .map_err(|error| format!("file_exists canonicalize failed for {path}: {error}"))?;
    if !file_canon.starts_with(&workspace_canon) {
        return Err(format!("file_exists path escapes workspace: {path}"));
    }
    Ok(file_canon)
}

/// Lexical `..` / absolute-outside check before the file needs to exist.
fn path_lexically_escapes_workspace(path: &str) -> bool {
    let p = Path::new(path);
    if p.is_absolute() {
        // Absolute paths are allowed only after canonicalize proves they sit
        // under the workspace; lexical pass-through here, fail closed later.
        return false;
    }
    let mut depth = 0i32;
    for component in p.components() {
        match component {
            std::path::Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            std::path::Component::Normal(_) => depth += 1,
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return true,
            std::path::Component::CurDir => {}
        }
    }
    false
}

fn short_digest(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn file_mtime_nanos(path: &Path) -> Option<u128> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos(),
    )
}

/// Stable fingerprint for a checked machine criterion that currently passes.
/// Returns `None` for manual criteria or when the check fails / cannot be hashed.
pub(super) fn machine_criterion_fingerprint(
    criterion: &AcceptanceCriterion,
    workspace: &Path,
) -> Option<String> {
    if !criterion.checked {
        return None;
    }
    verify_one_machine_criterion(criterion, workspace).ok()?;
    match &criterion.kind {
        CriterionKind::Manual => None,
        CriterionKind::Command {
            command,
            expect_exit,
        } => Some(format!(
            "fp:command:sha256={}:expect={expect_exit}:ok",
            short_digest(command)
        )),
        CriterionKind::FileExists { path } => {
            let full = resolve_workspace_file(workspace, path).ok()?;
            let meta = std::fs::metadata(&full).ok()?;
            let mtime = file_mtime_nanos(&full).unwrap_or(0);
            Some(format!(
                "fp:file:path={path}:len={}:mtime_ns={mtime}",
                meta.len()
            ))
        }
    }
}

/// Fingerprints for every checked machine criterion that passes right now.
pub(super) fn collect_passing_machine_fingerprints(
    acceptance: &str,
    workspace: &Path,
) -> Vec<String> {
    parse_acceptance_criteria(acceptance)
        .into_iter()
        .filter_map(|criterion| machine_criterion_fingerprint(&criterion, workspace))
        .collect()
}

/// Replace or append the `## Verified Evidence` section with current fingerprints.
pub(super) fn upsert_verified_evidence_section(state: &str, fingerprints: &[String]) -> String {
    let body = if fingerprints.is_empty() {
        "- None yet.\n".to_string()
    } else {
        let mut lines = String::from(
            "Host-recorded fingerprints for passing machine criteria. Makers should \
             skip re-scanning assertions whose fingerprints still match. These do \
             **not** replace host latch re-checks.\n\n",
        );
        for fp in fingerprints {
            lines.push_str("- `");
            lines.push_str(fp);
            lines.push_str("`\n");
        }
        lines
    };
    replace_markdown_section(state, VERIFIED_EVIDENCE_HEADING, &body)
}

fn replace_markdown_section(doc: &str, heading: &str, body: &str) -> String {
    let heading_line = format!("{heading}\n");
    if let Some(start) = doc.find(&heading_line) {
        let after_heading = start + heading_line.len();
        let rest = &doc[after_heading..];
        let end_offset = rest
            .find("\n## ")
            .map(|idx| after_heading + idx + 1)
            .unwrap_or(doc.len());
        let mut out = String::with_capacity(doc.len() + body.len());
        out.push_str(&doc[..after_heading]);
        if !body.ends_with('\n') {
            out.push_str(body);
            out.push('\n');
        } else {
            out.push_str(body);
        }
        if !body.ends_with("\n\n") && end_offset < doc.len() {
            out.push('\n');
        }
        out.push_str(&doc[end_offset..]);
        return out;
    }
    let mut out = doc.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(heading);
    out.push('\n');
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Whether every recorded fingerprint still appears in the live set (unchanged work).
pub(super) fn recorded_fingerprints_still_current(recorded: &[String], live: &[String]) -> bool {
    !recorded.is_empty()
        && recorded
            .iter()
            .all(|fp| live.iter().any(|live_fp| live_fp == fp))
}

fn run_acceptance_command(cwd: &Path, command: &str, expect_exit: i32) -> Result<(), String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("spawn failed for `{command}`: {error}"))?;
    let deadline = Instant::now() + ACCEPTANCE_COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(1);
                if code == expect_exit {
                    return Ok(());
                }
                return Err(format!("`{command}` exited {code}, expected {expect_exit}"));
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("`{command}` timed out"));
            }
            Err(error) => return Err(format!("wait failed for `{command}`: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_command_file_and_manual_criteria() {
        let body = "\
# Acceptance\n\
- [ ] kind:manual assert:ship the feature\n\
- [x] kind:command assert:`true` expect:exit=0\n\
- [ ] kind:file_exists assert:README.md\n\
- prose ignored\n";
        let criteria = parse_acceptance_criteria(body);
        assert_eq!(criteria.len(), 3);
        assert!(!criteria[0].checked);
        assert!(matches!(criteria[0].kind, CriterionKind::Manual));
        assert!(criteria[1].checked);
        assert!(matches!(
            criteria[1].kind,
            CriterionKind::Command {
                ref command,
                expect_exit: 0
            } if command == "true"
        ));
        assert!(matches!(
            criteria[2].kind,
            CriterionKind::FileExists { ref path } if path == "README.md"
        ));
        assert_eq!(acceptance_criteria_progress(body), (1, 3));
    }

    #[test]
    fn host_rechecks_failing_command_criterion() {
        let root =
            std::env::temp_dir().join(format!("a3s-goal-acceptance-fail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let body = "- [x] kind:command assert:`false` expect:exit=0\n";
        let err = verify_checked_machine_criteria(body, &root).unwrap_err();
        assert!(err.contains("re-check failed"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn host_rechecks_passing_command_and_file_criteria() {
        let root =
            std::env::temp_dir().join(format!("a3s-goal-acceptance-ok-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("marker.txt"), "ok").unwrap();
        let body = "\
- [x] kind:command assert:`true` expect:exit=0\n\
- [x] kind:file_exists assert:marker.txt\n\
- [x] kind:manual assert:reviewed\n";
        verify_checked_machine_criteria(body, &root).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn host_rejects_manual_only_acceptance_contract() {
        let root =
            std::env::temp_dir().join(format!("a3s-goal-acceptance-manual-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let body = "- [x] kind:manual assert:reviewed\n";
        let err = verify_checked_machine_criteria(body, &root).unwrap_err();
        assert!(
            err.contains("kind:command") || err.contains("manual-only"),
            "{err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn host_file_exists_rejects_directories_like_test_f() {
        let root =
            std::env::temp_dir().join(format!("a3s-goal-acceptance-dir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(root.join("not-a-file")).unwrap();
        let body = "- [x] kind:file_exists assert:not-a-file\n";
        let err = verify_checked_machine_criteria(body, &root).unwrap_err();
        assert!(
            err.contains("regular file") || err.contains("file_exists"),
            "directory must not satisfy kind:file_exists (align with Core test -f): {err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn host_file_exists_rejects_paths_that_escape_workspace() {
        let root =
            std::env::temp_dir().join(format!("a3s-goal-acceptance-escape-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!(
            "a3s-goal-acceptance-outside-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&outside);
        fs::create_dir_all(&root).unwrap();
        fs::write(&outside, "secret").unwrap();

        let relative = "- [x] kind:file_exists assert:../outside-should-not-count\n";
        // Create sibling file that ../ would reach from a nested join — use explicit parent escape.
        let err = verify_checked_machine_criteria(relative, &root).unwrap_err();
        assert!(
            err.contains("escapes workspace"),
            "relative ../ escape must not latch: {err}"
        );

        // Symlink inside workspace pointing at an outside file.
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = root.join("escape-link.txt");
            let _ = fs::remove_file(&link);
            symlink(&outside, &link).unwrap();
            let body = "- [x] kind:file_exists assert:escape-link.txt\n";
            let err = verify_checked_machine_criteria(body, &root).unwrap_err();
            assert!(
                err.contains("escapes workspace"),
                "symlink-to-outside must not latch: {err}"
            );
            let _ = fs::remove_file(&link);
        }

        let abs = format!("- [x] kind:file_exists assert:`{}`\n", outside.display());
        let err = verify_checked_machine_criteria(&abs, &root).unwrap_err();
        assert!(
            err.contains("escapes workspace"),
            "absolute outside path must not latch: {err}"
        );

        let _ = fs::remove_file(&outside);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fingerprints_record_passing_machine_criteria_and_detect_file_change() {
        let root =
            std::env::temp_dir().join(format!("a3s-goal-acceptance-fp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("marker.txt"), "ok").unwrap();
        let body = "\
- [x] kind:command assert:`true` expect:exit=0\n\
- [x] kind:file_exists assert:marker.txt\n\
- [x] kind:manual assert:reviewed\n";
        let first = collect_passing_machine_fingerprints(body, &root);
        assert_eq!(first.len(), 2, "{first:?}");
        assert!(first.iter().any(|fp| fp.starts_with("fp:command:")));
        assert!(first.iter().any(|fp| fp.starts_with("fp:file:")));
        assert!(recorded_fingerprints_still_current(&first, &first));

        // Ensure mtime can change on fast filesystems.
        std::thread::sleep(Duration::from_millis(20));
        fs::write(root.join("marker.txt"), "changed").unwrap();
        let second = collect_passing_machine_fingerprints(body, &root);
        assert_eq!(second.len(), 2);
        assert!(!recorded_fingerprints_still_current(&first, &second));

        let state =
            "# Goal\n\n## Verified Evidence\n\n- None yet.\n\n## Remaining Work\n\n- ship\n";
        let updated = upsert_verified_evidence_section(state, &second);
        assert!(updated.contains("fp:command:"));
        assert!(updated.contains("fp:file:"));
        assert!(updated.contains("## Remaining Work"));
        assert!(!updated.contains("- None yet."));
        let _ = fs::remove_dir_all(&root);
    }
}
