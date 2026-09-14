//! Promote or discard the conversation isolation worktree.
//!
//! This is the host transaction for kernel effect isolation. It does not
//! create a worktree and does not replace `/worktree`.

use super::*;
use a3s_code_core::effect_isolation::{self, PromoteOutcome};
use a3s_code_core::outcome_memory::{OutcomeKind, OutcomeLedger};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

const USAGE: &str = "usage: /isolate status|promote|discard|accept|revert|reject";
const CONSTRAINT_USAGE: &str = "usage: /isolate accept|revert|reject <constraint>";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum IsolationCommand {
    Status,
    Promote,
    Discard,
    Accept(String),
    Revert(String),
    Reject(String),
}

pub(super) fn parse_isolation_command(rest: &str) -> Result<IsolationCommand, &'static str> {
    let rest = rest.trim();
    match rest {
        "" | "status" => return Ok(IsolationCommand::Status),
        "promote" => return Ok(IsolationCommand::Promote),
        "discard" => return Ok(IsolationCommand::Discard),
        _ => {}
    }
    if let Some(constraint) = constraint_after(rest, "accept")? {
        return Ok(IsolationCommand::Accept(constraint));
    }
    if let Some(constraint) = constraint_after(rest, "revert")? {
        return Ok(IsolationCommand::Revert(constraint));
    }
    if let Some(constraint) = constraint_after(rest, "reject")? {
        return Ok(IsolationCommand::Reject(constraint));
    }
    Err(USAGE)
}

fn constraint_after(rest: &str, verb: &str) -> Result<Option<String>, &'static str> {
    if rest == verb {
        return Err(CONSTRAINT_USAGE);
    }
    let Some(tail) = rest.strip_prefix(verb) else {
        return Ok(None);
    };
    if !tail.starts_with(|ch: char| ch.is_whitespace()) {
        return Ok(None);
    }
    let constraint = tail.trim();
    if constraint.is_empty() {
        return Err(CONSTRAINT_USAGE);
    }
    Ok(Some(constraint.to_string()))
}

pub(super) fn outcome_ledger_path(workspace: &Path, session_id: &str) -> PathBuf {
    let key = URL_SAFE_NO_PAD.encode(session_id.as_bytes());
    workspace
        .join(".a3s")
        .join("tui")
        .join("outcomes")
        .join("v1")
        .join(format!("id_{key}.json"))
}

pub(super) fn load_outcome_ledger(workspace: &Path, session_id: &str) -> OutcomeLedger {
    OutcomeLedger::load(&outcome_ledger_path(workspace, session_id)).unwrap_or_default()
}

fn isolation_status(session_id: &str) -> String {
    match effect_isolation::binding(session_id) {
        Some(binding) => format!(
            "isolation bound · worktree {} · source revision {}",
            binding.worktree_path.display(),
            binding.source_revision
        ),
        None => "isolation is not bound for this conversation".to_string(),
    }
}

/// Host UX for Core isolation bind refusals on published 8.5.8.
///
/// Published Core returns `source revision is unknown` without naming the
/// repair. Later Core revisions may already include `create an initial
/// commit`; do not duplicate that clause. This is host copy, not a claim
/// that Core's event text changed.
pub(crate) fn annotate_isolation_bind_error(message: &str) -> String {
    let message = message.trim();
    if message.contains("source revision is unknown")
        && !message.contains("create an initial commit")
    {
        format!(
            "{message}; create an initial commit in this Git repository before a coding session can isolate writes"
        )
    } else {
        message.to_string()
    }
}

fn isolation_promote(
    session: &AgentSession,
    workspace: &Path,
    session_id: &str,
) -> Result<String, String> {
    match effect_isolation::promote_current(session_id) {
        Ok(PromoteOutcome::Applied { digest }) => {
            note_promoted(session, workspace, session_id, &digest)?;
            Ok(format!("promoted {digest} onto the source tree"))
        }
        Ok(PromoteOutcome::Idempotent { digest }) => {
            note_promoted(session, workspace, session_id, &digest)?;
            Ok(format!("promote already applied {digest}"))
        }
        Ok(PromoteOutcome::Conflict {
            bound_revision,
            current_revision,
        }) => Ok(format!(
            "promote refused: source revision moved from {bound_revision} to {current_revision}; nothing was applied"
        )),
        Err(error) => Err(error.to_string()),
    }
}

fn note_promoted(
    session: &AgentSession,
    workspace: &Path,
    session_id: &str,
    digest: &str,
) -> Result<(), String> {
    if !session
        .note_promoted_digest(digest)
        .map_err(|error| error.to_string())?
    {
        return Err("promoted digest was not recorded".to_string());
    }
    persist_outcome_ledger(session, workspace, session_id)
}

fn record_constraint(
    session: &AgentSession,
    workspace: &Path,
    session_id: &str,
    outcome: OutcomeKind,
    constraint: &str,
) -> Result<String, String> {
    let digest = outcome_digest(session, workspace, session_id, outcome)?;
    let stored = session
        .record_outcome(outcome, &digest, constraint)
        .map_err(|error| error.to_string())?;
    if !stored {
        return Err("constraint was not stored".to_string());
    }
    persist_outcome_ledger(session, workspace, session_id)?;
    let verb = match outcome {
        OutcomeKind::Accept => "accepted",
        OutcomeKind::Revert => "reverted",
        OutcomeKind::Reject => "rejected",
    };
    Ok(format!("{verb} {digest}"))
}

fn outcome_digest(
    session: &AgentSession,
    workspace: &Path,
    session_id: &str,
    outcome: OutcomeKind,
) -> Result<String, String> {
    if let Some(digest) = session
        .outcome_ledger_snapshot()
        .last_promoted_digest()
        .map(str::to_string)
        .or_else(|| {
            load_outcome_ledger(workspace, session_id)
                .last_promoted_digest()
                .map(str::to_string)
        })
    {
        return Ok(digest);
    }
    if outcome == OutcomeKind::Accept {
        return Err("promote a change set before accept".to_string());
    }
    match effect_isolation::current_change_digest(session_id).map_err(|error| error.to_string())? {
        Some(digest) => Ok(digest),
        None => Err("nothing to record".to_string()),
    }
}

fn persist_outcome_ledger(
    session: &AgentSession,
    workspace: &Path,
    session_id: &str,
) -> Result<(), String> {
    session
        .outcome_ledger_snapshot()
        .save(&outcome_ledger_path(workspace, session_id))
        .map_err(|error| error.to_string())
}

async fn isolation_discard(session_id: &str) -> Result<String, String> {
    effect_isolation::discard(session_id)
        .await
        .map(|()| "discarded the isolation worktree; source files were not deleted".to_string())
        .map_err(|error| error.to_string())
}

impl App {
    pub(super) fn submit_isolation_command(&mut self, rest: &str) -> Option<Cmd<Msg>> {
        let command = match parse_isolation_command(rest) {
            Ok(command) => command,
            Err(usage) => {
                self.textarea.clear();
                self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  {usage}")));
                return None;
            }
        };
        self.textarea.clear();
        let request_id = self.reserve_fork_request();
        let session_id = self.session_id.clone();
        let workspace = PathBuf::from(&self.cwd);
        let session = Arc::clone(&self.session);
        self.push_line(&Style::new().fg(TN_GRAY).render("  isolation transaction…"));
        Some(cmd::cmd(move || async move {
            let result = match command {
                IsolationCommand::Status => Ok(isolation_status(&session_id)),
                IsolationCommand::Promote => {
                    let session_id = session_id.clone();
                    let workspace = workspace.clone();
                    let session = Arc::clone(&session);
                    tokio::task::spawn_blocking(move || {
                        isolation_promote(&session, &workspace, &session_id)
                    })
                    .await
                    .map_err(|error| format!("isolation task failed: {error}"))
                    .and_then(|result| result)
                }
                IsolationCommand::Discard => isolation_discard(&session_id).await,
                IsolationCommand::Accept(constraint) => record_constraint(
                    &session,
                    &workspace,
                    &session_id,
                    OutcomeKind::Accept,
                    &constraint,
                ),
                IsolationCommand::Revert(constraint) => record_constraint(
                    &session,
                    &workspace,
                    &session_id,
                    OutcomeKind::Revert,
                    &constraint,
                ),
                IsolationCommand::Reject(constraint) => record_constraint(
                    &session,
                    &workspace,
                    &session_id,
                    OutcomeKind::Reject,
                    &constraint,
                ),
            };
            Msg::IsolationTransactionFinished { request_id, result }
        }))
    }

    pub(super) fn finish_isolation_transaction(
        &mut self,
        request_id: u64,
        result: Result<String, String>,
    ) -> Option<Cmd<Msg>> {
        if self.session_rebuild_pending != Some(request_id) {
            return None;
        }
        self.session_rebuild_pending = None;
        match result {
            Ok(message) => {
                self.push_line(&Style::new().fg(TN_GREEN).render(&format!("  {message}")))
            }
            Err(error) => self.push_line(
                &Style::new()
                    .fg(TN_RED)
                    .render(&format!("  /isolate: {error}")),
            ),
        }
        self.relayout();
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{annotate_isolation_bind_error, parse_isolation_command};

    #[test]
    fn published_core_unknown_revision_gets_host_repair_copy() {
        let annotated = annotate_isolation_bind_error(
            "isolation unavailable: source revision is unknown",
        );
        assert!(annotated.contains("source revision is unknown"), "{annotated}");
        assert!(
            annotated.contains("create an initial commit"),
            "{annotated}"
        );
    }

    #[test]
    fn richer_core_message_is_not_duplicated() {
        let core = "isolation unavailable: source revision is unknown; create an initial commit in this Git repository before a coding session can isolate writes";
        assert_eq!(annotate_isolation_bind_error(core), core);
    }

    use super::IsolationCommand;

    #[test]
    fn isolation_command_accepts_only_the_transaction_verbs() {
        assert_eq!(
            parse_isolation_command("").unwrap(),
            IsolationCommand::Status
        );
        assert_eq!(
            parse_isolation_command("promote").unwrap(),
            IsolationCommand::Promote
        );
        assert_eq!(
            parse_isolation_command("discard").unwrap(),
            IsolationCommand::Discard
        );
        assert_eq!(
            parse_isolation_command("accept name the evidence").unwrap(),
            IsolationCommand::Accept("name the evidence".to_string())
        );
        assert!(parse_isolation_command("accept").is_err());
        assert!(parse_isolation_command("cleanup").is_err());
    }

    #[test]
    fn accept_reloads_only_a_kept_promoted_digest() {
        let root = tempfile::tempdir().unwrap();
        let path = super::outcome_ledger_path(root.path(), "session/1");
        let mut ledger = a3s_code_core::outcome_memory::OutcomeLedger::default();
        assert!(ledger.note_promoted("digest-kept"));
        assert!(ledger.reject("digest-drop", "do not unwrap"));
        ledger.save(&path).unwrap();
        let loaded = super::load_outcome_ledger(root.path(), "session/1");
        assert_eq!(loaded.last_promoted_digest(), Some("digest-kept"));
        assert!(loaded.active_recall().is_empty());
        let digest = loaded.last_promoted_digest().unwrap().to_string();
        let mut loaded = loaded;
        assert!(loaded.accept(&digest, "name the missing evidence"));
        assert_eq!(loaded.active_recall().len(), 1);
        assert!(!loaded.accept("digest-kept", "token=sk-live-secret-value"));
    }

    /// `/isolate promote` returns Core's refusal and does not record a digest
    /// or copy the refused file onto the source tree.
    ///
    /// Git refuses to store `../escape`, so this Darwin host cannot feed the
    /// unsafe-path classifier through a real index. The reachable refusal is a
    /// credential file. The host must surface that Core error unchanged and
    /// must not apply it. The unsafe-path classifier itself stays in Core.
    #[tokio::test]
    async fn isolate_promote_surfaces_a_core_refusal_and_does_not_apply() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("README.md"), "source\n").unwrap();
        std::fs::write(root.path().join(".env"), "TOKEN=source-secret-91aa\n").unwrap();
        git(root.path(), &["init"]);
        git(root.path(), &["config", "user.email", "a3s@example.com"]);
        git(root.path(), &["config", "user.name", "a3s"]);
        git(root.path(), &["add", "README.md", ".env"]);
        git(root.path(), &["commit", "-m", "init"]);
        let config = root.path().join("config.acl");
        std::fs::write(
            &config,
            "default_model = \"openai/x\"\n\
             providers \"openai\" {\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
             models \"x\" { name = \"x\" }\n}\n",
        )
        .unwrap();
        let agent = a3s_code_core::Agent::new(config.to_string_lossy().to_string())
            .await
            .expect("agent");
        let session = agent
            .session_async(root.path().to_string_lossy().to_string(), None)
            .await
            .expect("session");
        let session_id = session.session_id().to_string();
        let binding = a3s_code_core::effect_isolation::bind(&session_id, root.path(), true)
            .await
            .expect("bind");
        std::fs::write(
            binding.worktree_path.join(".env"),
            "TOKEN=must-not-land\n",
        )
        .unwrap();

        let error = super::isolation_promote(&session, root.path(), &session_id)
            .expect_err("promote must refuse");

        assert!(
            error.contains("credential boundary") || error.contains("refusing to promote"),
            "{error}"
        );
        assert!(
            !error.contains("promoted "),
            "a refusal must not be reported as applied: {error}"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join(".env")).unwrap(),
            "TOKEN=source-secret-91aa\n"
        );
        assert!(
            super::load_outcome_ledger(root.path(), &session_id)
                .last_promoted_digest()
                .is_none()
        );
        a3s_code_core::effect_isolation::discard(&session_id)
            .await
            .expect("discard");
    }

    fn git(root: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }
}
