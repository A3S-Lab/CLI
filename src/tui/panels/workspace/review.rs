//! Repository-scoped, read-only code-review target parsing and prompts.

use std::path::Path;

use super::review::review_report_contract;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceReviewTarget {
    WorkingTree,
    Commit(String),
    Branch(String),
}

impl WorkspaceReviewTarget {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::WorkingTree => "working tree".to_string(),
            Self::Commit(revision) => format!("commit {revision}"),
            Self::Branch(base) => format!("branch against {base}"),
        }
    }

    fn inspection(&self) -> String {
        match self {
            Self::WorkingTree => "Review all tracked staged and unstaged changes plus relevant untracked files. Use `git status --short`, `git diff --cached`, and `git diff`; inspect untracked source files directly. Do not review unrelated unchanged code except where needed to prove an issue.".to_string(),
            Self::Commit(revision) => format!(
                "Treat the revision in the data block below as a Git commit-ish. Resolve it as a commit, then review exactly its patch and the surrounding code needed to prove findings (equivalent scope: `git show --find-renames --find-copies <revision>`).\n\n```review-target\n{revision}\n```"
            ),
            Self::Branch(base) => format!(
                "Treat the value in the data block below as the comparison base. Resolve it as a commit, compute its merge base with HEAD, and review the complete merge-base-to-HEAD patch, including staged and unstaged working-tree changes.\n\n```review-target\n{base}\n```"
            ),
        }
    }
}

pub(crate) fn parse_workspace_review_target(
    input: &str,
) -> Result<WorkspaceReviewTarget, &'static str> {
    let parts = input.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        [] | ["working-tree"] | ["uncommitted"] => Ok(WorkspaceReviewTarget::WorkingTree),
        ["commit", revision] if valid_revision(revision) => {
            Ok(WorkspaceReviewTarget::Commit((*revision).to_string()))
        }
        ["branch", base] if valid_revision(base) => {
            Ok(WorkspaceReviewTarget::Branch((*base).to_string()))
        }
        _ => Err("usage: /review [working-tree|commit <revision>|branch <base>]"),
    }
}

fn valid_revision(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._/@{}~^:+-".contains(&byte))
}

pub(crate) fn workspace_review_prompt(cwd: &Path, target: &WorkspaceReviewTarget) -> String {
    format!(
        "Perform a deep, read-only code review of the Git repository at {workspace}.\n\n\
         Scope:\n{inspection}\n\n\
         Find concrete correctness, security, reliability, performance, and regression risks introduced by the scoped change. Read repository instructions and relevant tests before judging behavior. Prioritize findings that the author would act on. Every finding must identify a precise file and line in the reviewed change when possible, explain the failure mode, and avoid style-only commentary. Do not edit files, run formatting, install dependencies, commit, or perform any other mutation. If the target cannot be resolved or the directory is not a Git repository, explain that clearly and return an empty report.\
         {contract}",
        workspace = cwd.display(),
        inspection = target.inspection(),
        contract = review_report_contract(cwd),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_review_targets() {
        assert_eq!(
            parse_workspace_review_target("").unwrap(),
            WorkspaceReviewTarget::WorkingTree
        );
        assert_eq!(
            parse_workspace_review_target("uncommitted").unwrap(),
            WorkspaceReviewTarget::WorkingTree
        );
        assert_eq!(
            parse_workspace_review_target("commit HEAD~2").unwrap(),
            WorkspaceReviewTarget::Commit("HEAD~2".to_string())
        );
        assert_eq!(
            parse_workspace_review_target("branch origin/main").unwrap(),
            WorkspaceReviewTarget::Branch("origin/main".to_string())
        );
    }

    #[test]
    fn rejects_ambiguous_or_option_like_targets() {
        for input in [
            "commit",
            "branch",
            "commit --all",
            "branch main extra",
            "other",
        ] {
            assert!(parse_workspace_review_target(input).is_err(), "{input}");
        }
    }

    #[test]
    fn prompt_is_read_only_and_carries_the_report_contract() {
        let prompt = workspace_review_prompt(
            Path::new("/workspace"),
            &WorkspaceReviewTarget::Branch("main".to_string()),
        );
        assert!(prompt.contains("read-only code review"));
        assert!(prompt.contains("Do not edit files"));
        assert!(prompt.contains("merge base"));
        assert!(prompt.contains("```a3s-review"));
        assert!(prompt.contains("\"asset_dir\": \"/workspace\""));
    }
}
