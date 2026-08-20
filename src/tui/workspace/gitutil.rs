//! Lightweight git context for the status bar.

use std::path::{Path, PathBuf};

#[cfg(test)]
use std::process::Command;

/// Current Git branch of `dir`, including linked worktrees and detached HEAD.
pub(crate) fn git_branch(dir: &str) -> Option<String> {
    let dir = Path::new(dir);
    let git_dir = resolve_git_dir(dir)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    parse_head(&head)
}

#[cfg(test)]
fn git_repository_might_exist(dir: &Path) -> bool {
    resolve_git_dir(dir).is_some()
}

fn resolve_git_dir(dir: &Path) -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("GIT_DIR").filter(|value| !value.is_empty()) {
        let configured = PathBuf::from(configured);
        let configured = if configured.is_absolute() {
            configured
        } else {
            dir.join(configured)
        };
        return configured.join("HEAD").is_file().then_some(configured);
    }

    for ancestor in dir.ancestors() {
        let marker = ancestor.join(".git");
        if marker.is_dir() {
            return Some(marker);
        }
        if marker.is_file() {
            let contents = std::fs::read_to_string(&marker).ok()?;
            let target = contents.trim().strip_prefix("gitdir:")?.trim();
            if target.is_empty() {
                return None;
            }
            let target = PathBuf::from(target);
            let target = if target.is_absolute() {
                target
            } else {
                ancestor.join(target)
            };
            if target.join("HEAD").is_file() {
                return Some(target);
            }
            return None;
        }
        if ancestor.join("HEAD").is_file()
            && ancestor.join("objects").is_dir()
            && ancestor.join("refs").is_dir()
        {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn parse_head(head: &str) -> Option<String> {
    let head = head.trim();
    if head.is_empty() {
        return None;
    }
    if let Some(reference) = head.strip_prefix("ref:").map(str::trim) {
        let branch = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        return (!branch.is_empty()).then(|| branch.to_string());
    }
    if !head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let short_len = head.len().min(8);
    Some(format!("detached@{}", &head[..short_len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
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

    fn repository() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temporary repository");
        git(root.path(), &["init"]);
        git(root.path(), &["config", "user.name", "A3S Test"]);
        git(
            root.path(),
            &["config", "user.email", "a3s-test@example.invalid"],
        );
        std::fs::write(root.path().join("README.md"), "initial\n").expect("write fixture");
        git(root.path(), &["add", "README.md"]);
        git(root.path(), &["commit", "-m", "initial"]);
        root
    }

    #[test]
    fn branch_is_discovered_in_normal_and_linked_worktrees() {
        let root = repository();
        let branch = git_branch(root.path().to_str().unwrap()).expect("normal branch");

        let linked_root = tempfile::tempdir().expect("linked worktree parent");
        let linked = linked_root.path().join("linked");
        let output = Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args(["worktree", "add", "-b", "linked-test"])
            .arg(&linked)
            .arg("HEAD")
            .output()
            .expect("create linked worktree");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert!(!branch.is_empty());
        assert_eq!(
            git_branch(linked.to_str().unwrap()).as_deref(),
            Some("linked-test")
        );
    }

    #[test]
    fn detached_head_is_visible_instead_of_disappearing() {
        let root = repository();
        git(root.path(), &["checkout", "--detach"]);

        let branch = git_branch(root.path().to_str().unwrap()).expect("detached identity");
        assert!(branch.starts_with("detached@"), "{branch}");
        assert!(branch.len() > "detached@".len());
    }

    #[test]
    fn non_repository_has_no_git_identity() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested/workspace");
        std::fs::create_dir_all(&nested).unwrap();

        assert!(!git_repository_might_exist(&nested));
        assert_eq!(git_branch(nested.to_str().unwrap()), None);
    }
}
