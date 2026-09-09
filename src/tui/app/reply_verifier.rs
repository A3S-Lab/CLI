//! Sticky reply verifier: Core Gate admission + independent host executor.
//!
//! Product contract (Claude Science Auto-review analogue for coding replies):
//! - Input is a [`TurnEvidenceBundle`] only (claim↔record).
//! - Incomplete host evidence → fail-closed chrome; never publish a clean pass.
//! - Complete evidence → Gate-mode `AuxiliaryRunService` + protocol executor
//!   (hermetic false-pass rubric). No `AgentStyle::CodeReview`, no main session.

use a3s_code_core::{
    AgentEvent, AuxiliaryCapabilityProfileV1, AuxiliaryExecutor, AuxiliaryModeV1,
    AuxiliaryRunContextV1, AuxiliaryRunError, AuxiliaryRunService, AuxiliaryRunSpecV1,
    EvidenceReadRequestV1, ExecutionFrameV1, ExecutionTargetV1, InMemoryAuxiliaryRunService,
    InMemoryRunStore, RunEvidenceReader,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::tui::panels::workspace_review::{mock_sticky_reply_review_report, TurnEvidenceBundle};

const AUX_PURPOSE: &str = "cli.reply-verifier";

/// Prefix for fail-closed sticky finishes (must not be parsed as a clean report).
pub(crate) const REPLY_VERIFIER_FAIL_CLOSED_PREFIX: &str = "reviewer incomplete:";

pub(crate) fn reply_verifier_fail_closed_text() -> String {
    format!(
        "{REPLY_VERIFIER_FAIL_CLOSED_PREFIX} evidence incomplete — review not published (fail-closed)"
    )
}

pub(crate) fn reply_verifier_fail_closed_chrome() -> &'static str {
    crate::tui::panels::review::reviewer_fail_closed_finish_line()
}

/// Sticky Gate identity (purpose + synthetic session id). Must stay distinct from
/// git `/review` (`bg-review-*` + `AgentStyle::CodeReview`).
pub(crate) fn sticky_reply_verifier_identity() -> (&'static str, &'static str) {
    (AUX_PURPOSE, "cli-reply-verifier")
}

struct ProtocolReplyVerifierExecutor {
    cwd: PathBuf,
    bundle: TurnEvidenceBundle,
}

#[async_trait]
impl AuxiliaryExecutor for ProtocolReplyVerifierExecutor {
    async fn execute(
        &self,
        context: AuxiliaryRunContextV1,
    ) -> Result<Value, AuxiliaryRunError> {
        if context.cancellation.is_cancelled() {
            return Err(AuxiliaryRunError::Cancelled);
        }
        if !self.bundle.complete {
            return Err(AuxiliaryRunError::EvidenceIncomplete);
        }
        let report = mock_sticky_reply_review_report(&self.cwd, &self.bundle);
        let decision = if report.contains("\"issues\":[]") {
            "clean"
        } else {
            "findings"
        };
        Ok(json!({
            "decision": decision,
            "report_fence": report,
        }))
    }
}

/// Run sticky claim↔record verification. Fail-closed when evidence is incomplete.
pub(crate) async fn run_reply_verifier(cwd: &Path, bundle: TurnEvidenceBundle) -> String {
    if !bundle.complete {
        return reply_verifier_fail_closed_text();
    }

    let (purpose, session_id) = sticky_reply_verifier_identity();
    let run_id = format!("reply-{}", uuid_lite());
    let runs = Arc::new(InMemoryRunStore::new());
    let run = runs
        .create_run_with_id(run_id.clone(), session_id, purpose)
        .await;
    if runs
        .record_event(
            &run.id,
            AgentEvent::TextDelta {
                text: bundle.assistant.clone(),
            },
        )
        .await
        .is_none()
    {
        return format!("reviewer failed: failed to record turn evidence for Gate admission");
    }

    let target = ExecutionTargetV1::new(session_id, &run.id);
    let evidence = match RunEvidenceReader::new(Arc::clone(&runs))
        .read(EvidenceReadRequestV1::new(target.clone()))
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => return format!("reviewer failed: evidence read: {error}"),
    };
    if let Err(error) = evidence.validate() {
        return format!("reviewer failed: evidence validate: {error}");
    }
    if !evidence.complete || evidence.retention_gap {
        return reply_verifier_fail_closed_text();
    }

    let executor: Arc<dyn AuxiliaryExecutor> = Arc::new(ProtocolReplyVerifierExecutor {
        cwd: cwd.to_path_buf(),
        bundle,
    });
    let service = InMemoryAuxiliaryRunService::new(executor);
    let auxiliary_run_id = format!("aux-reply-{run_id}");
    let spec = AuxiliaryRunSpecV1::new(
        ExecutionFrameV1::root(target),
        purpose,
        "Independent claim-vs-record review of the assistant reply against turn evidence.",
        evidence.snapshot_digest.clone(),
    )
    .with_id(auxiliary_run_id)
    .with_mode(AuxiliaryModeV1::Gate)
    .with_capabilities(AuxiliaryCapabilityProfileV1::tool_free());

    match service.spawn(spec, evidence, None).await {
        Ok(handle) => match handle.wait().await {
            Ok(output) => output
                .value
                .get("report_fence")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!("reviewer failed: Gate executor returned no report_fence")
                }),
            Err(error) => match error {
                AuxiliaryRunError::EvidenceIncomplete => reply_verifier_fail_closed_text(),
                other => format!("reviewer failed: {other}"),
            },
        },
        Err(AuxiliaryRunError::EvidenceIncomplete) => reply_verifier_fail_closed_text(),
        Err(error) => format!("reviewer failed: {error}"),
    }
}

fn uuid_lite() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::panels::workspace_review::TurnEvidenceTool;
    use crate::tui::panels::review::{
        classify_background_review_finish, parse_review_report, BackgroundReviewFinishKind,
    };

    fn false_pass_bundle(complete: bool) -> TurnEvidenceBundle {
        TurnEvidenceBundle {
            user: "Did the tests pass?".into(),
            assistant: "Yes — all tests passed.".into(),
            tools: vec![TurnEvidenceTool {
                index: 1,
                name: "bash".into(),
                state: "failed".into(),
                args: "{\"command\":\"cargo test\"}".into(),
                output: "test result: FAILED. 2 failed".into(),
                exit_code: Some(1),
                truncated: false,
            }],
            complete,
        }
    }

    #[tokio::test]
    async fn reply_verifier_gate_denies_incomplete_evidence() {
        let text = run_reply_verifier(Path::new("/ws"), false_pass_bundle(false)).await;
        assert!(
            text.starts_with(REPLY_VERIFIER_FAIL_CLOSED_PREFIX),
            "{text}"
        );
        assert_eq!(
            classify_background_review_finish(&text),
            BackgroundReviewFinishKind::FailClosed
        );
        assert!(parse_review_report(&text).is_none());
        assert!(!text.contains("\"issues\":[]"));
    }

    #[tokio::test]
    async fn reply_verifier_complete_false_pass_publishes_fence() {
        let text = run_reply_verifier(Path::new("/ws"), false_pass_bundle(true)).await;
        assert_eq!(
            classify_background_review_finish(&text),
            BackgroundReviewFinishKind::Capture
        );
        let (_, kind, issues) = parse_review_report(&text).expect("fence");
        assert_eq!(kind, crate::tui::panels::review::ReviewReportKind::Reply);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].verdict, "fail");
    }

    #[test]
    fn sticky_reply_verifier_does_not_use_code_review_style() {
        let (purpose, session_id) = sticky_reply_verifier_identity();
        assert_eq!(purpose, "cli.reply-verifier");
        assert_eq!(session_id, "cli-reply-verifier");
        assert!(
            !purpose.contains("code-review") && !session_id.starts_with("bg-review-"),
            "sticky identity must not look like git CodeReview side-session"
        );
    }
}
