//! Sticky reply verifier: Core Gate admission + independent host executor.
//!
//! Product contract (aligned with Desktop Science Auto-review for coding replies):
//! - Input is a [`TurnEvidenceBundle`] only (claim↔record).
//! - Incomplete host evidence → fail-closed chrome; never publish a clean pass.
//! - Complete evidence → Gate-mode `AuxiliaryRunService`.
//! - When a forked side-path [`LlmClient`] is available → `StructuredAuxiliaryExecutor`
//!   (Desktop independent ReviewAgent). Protocol rubric is hermetic/offline only.
//! - No `AgentStyle::CodeReview`, no main session.

use a3s_code_core::llm::structured::{StructuredMode, StructuredRequest};
use a3s_code_core::{
    AgentEvent, AuxiliaryCapabilityProfileV1, AuxiliaryExecutor, AuxiliaryModeV1,
    AuxiliaryRunContextV1, AuxiliaryRunError, AuxiliaryRunService, AuxiliaryRunSpecV1,
    EvidenceReadRequestV1, ExecutionFrameV1, ExecutionTargetV1, InMemoryAuxiliaryRunService,
    InMemoryRunStore, LlmClient, RunEvidenceReader, StructuredAuxiliaryExecutor,
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
    async fn execute(&self, context: AuxiliaryRunContextV1) -> Result<Value, AuxiliaryRunError> {
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

fn coding_review_findings_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["decision", "findings"],
        "properties": {
            "decision": { "type": "string", "enum": ["clean", "findings"] },
            "findings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["finding_id", "category", "severity", "message", "evidence"],
                    "properties": {
                        "finding_id": { "type": "string" },
                        "category": {
                            "type": "string",
                            "enum": [
                                "claim_record",
                                "false_pass",
                                "contradiction",
                                "incomplete_evidence",
                                "other"
                            ]
                        },
                        "severity": {
                            "type": "string",
                            "enum": ["info", "warning", "error", "blocker"]
                        },
                        "message": { "type": "string" },
                        "claim": { "type": "string" },
                        "evidence": { "type": "string" },
                        "quote": { "type": "string" },
                        "suggestion": { "type": "string" },
                        "verdict": {
                            "type": "string",
                            "enum": ["pass", "warn", "fail", "inconclusive"]
                        },
                        "evidence_refs": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    }
                }
            }
        }
    })
}

fn build_coding_structured_request(bundle: &TurnEvidenceBundle) -> StructuredRequest {
    let mut tools = String::new();
    for tool in &bundle.tools {
        tools.push_str(&format!(
            "tool:{} name={} state={} exit={:?}\nargs: {}\noutput:\n{}\n---\n",
            tool.index, tool.name, tool.state, tool.exit_code, tool.args, tool.output
        ));
    }
    let prompt = format!(
        "Review the assistant reply against turn tool evidence only (claim↔record).\n\n\
         ## User request\n{}\n\n\
         ## Assistant reply\n{}\n\n\
         ## Tool evidence\n{}\n\n\
         Return decision=clean with empty findings when the reply is sound. \
         Prefer false_pass / claim_record when the reply claims success that tools contradict.",
        bundle.user, bundle.assistant, tools
    );
    StructuredRequest {
        prompt,
        system: Some(
            "You are an independent coding reply verifier. You never edit files or run tools. \
             Judge only claim↔record fidelity between the assistant message and the tool evidence."
                .to_owned(),
        ),
        schema: coding_review_findings_schema(),
        schema_name: "cli_sticky_reply_review_findings".to_owned(),
        schema_description: Some(
            "Independent sticky reply verifier findings for A3S Code TUI.".to_owned(),
        ),
        mode: StructuredMode::Auto,
        max_repair_attempts: 2,
    }
}

fn severity_wire_to_review(severity: &str) -> &'static str {
    match severity.trim().to_ascii_lowercase().as_str() {
        "blocker" | "error" | "high" => "high",
        "warning" | "medium" | "warn" => "medium",
        _ => "low",
    }
}

/// Map Desktop/Gate structured findings JSON into the existing `a3s-review` fence
/// so checklist capture and Core upsert share one payload.
pub(crate) fn report_fence_from_structured_findings(cwd: &Path, value: &Value) -> String {
    let asset = cwd.display().to_string();
    let findings = value
        .get("findings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut issues = Vec::new();
    for finding in findings {
        let finding_id = finding
            .get("finding_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let category = finding
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("other")
            .to_string();
        let message = finding
            .get("message")
            .or_else(|| finding.get("claim"))
            .and_then(Value::as_str)
            .unwrap_or("finding")
            .to_string();
        let evidence = finding
            .get("evidence")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let suggestion = finding
            .get("suggestion")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let detail = if suggestion.is_empty() {
            evidence.clone()
        } else if evidence.is_empty() {
            suggestion
        } else {
            format!("{evidence} · suggestion: {suggestion}")
        };
        let severity = severity_wire_to_review(
            finding
                .get("severity")
                .and_then(Value::as_str)
                .unwrap_or("warning"),
        );
        let verdict = finding
            .get("verdict")
            .and_then(Value::as_str)
            .unwrap_or("fail")
            .to_string();
        let evidence_refs = finding
            .get("evidence_refs")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let title = if category.is_empty() {
            message.clone()
        } else {
            format!("{category}: {message}")
        };
        issues.push(json!({
            "finding_id": finding_id,
            "severity": severity,
            "file": "assistant-reply",
            "line": Value::Null,
            "title": title,
            "detail": detail,
            "verdict": verdict,
            "evidence_refs": evidence_refs,
            "status": "open",
        }));
    }
    format!(
        "{}\n{{\"asset_dir\":{},\"kind\":\"reply\",\"issues\":{}}}\n```",
        crate::tui::panels::review::REVIEW_FENCE,
        serde_json::to_string(&asset).unwrap_or_else(|_| "\"\"".into()),
        Value::Array(issues),
    )
}

fn independent_reply_executor(
    cwd: PathBuf,
    bundle: TurnEvidenceBundle,
    llm: Option<Arc<dyn LlmClient>>,
) -> Arc<dyn AuxiliaryExecutor> {
    if let Some(client) = llm {
        let evidence_bundle = bundle.clone();
        Arc::new(StructuredAuxiliaryExecutor::new(client, move |_context| {
            build_coding_structured_request(&evidence_bundle)
        }))
    } else {
        Arc::new(ProtocolReplyVerifierExecutor { cwd, bundle })
    }
}

/// Run sticky claim↔record verification. Fail-closed when evidence is incomplete.
///
/// Pass a forked side-path LLM for the Desktop-aligned product path. Omit LLM
/// only for hermetic/offline protocol rubric (tests).
pub(crate) async fn run_reply_verifier(
    cwd: &Path,
    bundle: TurnEvidenceBundle,
    llm: Option<Arc<dyn LlmClient>>,
) -> String {
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
        return "reviewer failed: failed to record turn evidence for Gate admission".to_string();
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

    let used_llm = llm.is_some();
    let executor = independent_reply_executor(cwd.to_path_buf(), bundle, llm);
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
            Ok(output) => {
                if let Some(fence) = output
                    .value
                    .get("report_fence")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                {
                    return fence;
                }
                if used_llm {
                    return report_fence_from_structured_findings(cwd, &output.value);
                }
                "reviewer failed: Gate executor returned no report_fence".to_string()
            }
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
    use crate::tui::panels::review::{
        classify_background_review_finish, parse_review_report, BackgroundReviewFinishKind,
    };
    use crate::tui::panels::workspace_review::TurnEvidenceTool;

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
        let text = run_reply_verifier(Path::new("/ws"), false_pass_bundle(false), None).await;
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
        let text = run_reply_verifier(Path::new("/ws"), false_pass_bundle(true), None).await;
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

    #[test]
    fn structured_findings_map_into_a3s_review_fence() {
        let value = json!({
            "decision": "findings",
            "findings": [{
                "finding_id": "f1",
                "category": "false_pass",
                "severity": "error",
                "message": "claimed pass",
                "evidence": "tool:1 failed",
                "verdict": "fail",
                "evidence_refs": ["tool:1"]
            }]
        });
        let fence = report_fence_from_structured_findings(Path::new("/ws"), &value);
        let (_, kind, issues) = parse_review_report(&fence).expect("fence");
        assert_eq!(kind, crate::tui::panels::review::ReviewReportKind::Reply);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].finding_id, "f1");
        assert!(issues[0].title.contains("false_pass"));
    }
}
