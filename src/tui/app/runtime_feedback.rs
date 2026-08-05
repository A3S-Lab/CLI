//! Typed Core runtime feedback projected into the terminal UI.
//!
//! The event stream contains both high-frequency progress and operational
//! failures. Progress belongs on the single transient activity row; failures
//! and recovery decisions belong in the semantic transcript. Keeping that
//! distinction here prevents the main event reducer from silently discarding
//! important Core events or turning every context/memory event into noise.

use std::collections::BTreeMap;

use a3s_code_core::tools::ToolErrorKind;
use a3s_code_core::verification::{
    format_verification_summary, VerificationStatus, VerificationSummary,
};
use a3s_code_core::AgentEvent;

use super::NoticeKind;

const STATUS_FRAGMENT_CHARS: usize = 48;
const NOTICE_FRAGMENT_CHARS: usize = 240;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CoreRunStatus {
    agent_mode: Option<AgentModeStatus>,
    context_providers: Option<Vec<String>>,
    memory_matches: Option<usize>,
    planning: bool,
    external_tasks: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentModeStatus {
    mode: String,
    agent: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CoreEventNotice {
    pub(crate) kind: NoticeKind,
    pub(crate) message: String,
}

impl CoreRunStatus {
    /// Apply one Core event to the transient run projection and return a
    /// durable notice only when the event requires user-visible attention.
    pub(crate) fn observe(&mut self, event: &AgentEvent) -> Option<CoreEventNotice> {
        match event {
            AgentEvent::Start { .. } => {
                // Core emits AgentModeChanged immediately before Start. Clear
                // only stale per-turn phases so the freshly selected mode is
                // still visible on the activity row.
                self.context_providers = None;
                self.memory_matches = None;
                self.planning = false;
                self.external_tasks.clear();
                None
            }
            AgentEvent::AgentModeChanged { mode, agent, .. } => {
                self.agent_mode = Some(AgentModeStatus {
                    mode: bounded_fragment(mode, STATUS_FRAGMENT_CHARS),
                    agent: bounded_fragment(agent, STATUS_FRAGMENT_CHARS),
                });
                None
            }
            AgentEvent::ContextResolving { providers } => {
                self.context_providers = Some(
                    providers
                        .iter()
                        .take(3)
                        .map(|provider| bounded_fragment(provider, STATUS_FRAGMENT_CHARS))
                        .filter(|provider| !provider.is_empty())
                        .collect(),
                );
                None
            }
            AgentEvent::ContextResolved { .. } => {
                self.context_providers = None;
                self.memory_matches = None;
                None
            }
            AgentEvent::MemoryRecalled { .. } => {
                self.memory_matches =
                    Some(self.memory_matches.unwrap_or_default().saturating_add(1));
                None
            }
            AgentEvent::MemoriesSearched { result_count, .. } => {
                self.memory_matches = Some(*result_count);
                None
            }
            AgentEvent::MemoryStored {
                memory_type,
                importance,
                ..
            } => {
                let memory_type = bounded_fragment(memory_type, STATUS_FRAGMENT_CHARS);
                Some(CoreEventNotice {
                    kind: NoticeKind::Info,
                    message: if memory_type.is_empty() {
                        format!(
                            "Stored memory · importance {}",
                            format_metric(*importance as f64)
                        )
                    } else {
                        format!(
                            "Stored {memory_type} memory · importance {}",
                            format_metric(*importance as f64)
                        )
                    },
                })
            }
            AgentEvent::PlanningStart { .. } => {
                self.planning = true;
                None
            }
            AgentEvent::PlanningEnd { .. } => {
                self.planning = false;
                None
            }
            AgentEvent::ExternalTaskPending {
                task_id,
                command_type,
                ..
            } => {
                self.external_tasks.insert(
                    task_id.clone(),
                    bounded_fragment(command_type, STATUS_FRAGMENT_CHARS),
                );
                None
            }
            AgentEvent::ExternalTaskCompleted {
                task_id, success, ..
            } => {
                let command = self.external_tasks.remove(task_id).unwrap_or_default();
                let task_id = bounded_fragment(task_id, STATUS_FRAGMENT_CHARS);
                (!success).then(|| CoreEventNotice {
                    kind: NoticeKind::Warning,
                    message: if command.is_empty() {
                        format!("External task {task_id} failed")
                    } else {
                        format!("External task {command} ({task_id}) failed")
                    },
                })
            }
            AgentEvent::CommandRetry {
                command_id,
                command_type,
                lane,
                attempt,
                delay_ms,
            } => Some(CoreEventNotice {
                kind: NoticeKind::Info,
                message: format!(
                    "Retrying queued {} {} on {} · attempt {} after {}",
                    bounded_fragment(command_type, STATUS_FRAGMENT_CHARS),
                    bounded_fragment(command_id, STATUS_FRAGMENT_CHARS),
                    bounded_fragment(lane, STATUS_FRAGMENT_CHARS),
                    attempt,
                    format_duration_ms(*delay_ms)
                ),
            }),
            AgentEvent::CommandDeadLettered {
                command_id,
                command_type,
                lane,
                error,
                attempts,
            } => Some(CoreEventNotice {
                kind: NoticeKind::Error,
                message: format!(
                    "Queued {} {} on {} stopped after {} attempts · {}",
                    bounded_fragment(command_type, STATUS_FRAGMENT_CHARS),
                    bounded_fragment(command_id, STATUS_FRAGMENT_CHARS),
                    bounded_fragment(lane, STATUS_FRAGMENT_CHARS),
                    attempts,
                    bounded_fragment(error, NOTICE_FRAGMENT_CHARS)
                ),
            }),
            AgentEvent::QueueAlert {
                level,
                alert_type,
                message,
            } => Some(CoreEventNotice {
                kind: notice_kind_for_level(level),
                message: format!(
                    "Queue {} · {}",
                    bounded_fragment(alert_type, STATUS_FRAGMENT_CHARS),
                    bounded_fragment(message, NOTICE_FRAGMENT_CHARS)
                ),
            }),
            AgentEvent::PersistenceFailed {
                operation, error, ..
            } => Some(CoreEventNotice {
                kind: NoticeKind::Error,
                message: format!(
                    "Session persistence failed during {} · {}",
                    bounded_fragment(operation, STATUS_FRAGMENT_CHARS),
                    bounded_fragment(error, NOTICE_FRAGMENT_CHARS)
                ),
            }),
            AgentEvent::BudgetThresholdHit {
                resource,
                kind,
                consumed,
                limit,
                message,
            } => {
                let severity = if kind.eq_ignore_ascii_case("hard") {
                    NoticeKind::Error
                } else {
                    NoticeKind::Warning
                };
                let mut feedback = format!(
                    "{} budget {} threshold · {} / {}",
                    bounded_fragment(resource, STATUS_FRAGMENT_CHARS),
                    bounded_fragment(kind, STATUS_FRAGMENT_CHARS),
                    format_metric(*consumed),
                    format_metric(*limit)
                );
                if let Some(message) = message {
                    let message = bounded_fragment(message, NOTICE_FRAGMENT_CHARS);
                    if !message.is_empty() {
                        feedback.push_str(" · ");
                        feedback.push_str(&message);
                    }
                }
                Some(CoreEventNotice {
                    kind: severity,
                    message: feedback,
                })
            }
            AgentEvent::MemoryCleared { tier, count } => Some(CoreEventNotice {
                kind: NoticeKind::Warning,
                message: format!(
                    "Cleared {} {} memories",
                    count,
                    bounded_fragment(tier, STATUS_FRAGMENT_CHARS)
                ),
            }),
            AgentEvent::PassivationRequested {
                reason,
                deadline_ms,
            } => {
                let mut message = format!(
                    "Session passivation requested · {}",
                    bounded_fragment(reason, NOTICE_FRAGMENT_CHARS)
                );
                if let Some(deadline_ms) = deadline_ms {
                    message.push_str(" · deadline ");
                    message.push_str(&format_deadline_ms(*deadline_ms));
                }
                Some(CoreEventNotice {
                    kind: NoticeKind::Warning,
                    message,
                })
            }
            AgentEvent::PeerInvocation {
                from_session_id, ..
            } => Some(CoreEventNotice {
                kind: NoticeKind::Info,
                message: format!(
                    "Peer session {} invoked this session",
                    bounded_fragment(from_session_id, STATUS_FRAGMENT_CHARS)
                ),
            }),
            AgentEvent::End { .. } | AgentEvent::Error { .. } => {
                self.clear();
                None
            }
            _ => None,
        }
    }

    pub(crate) fn activity_label(&self) -> String {
        if !self.external_tasks.is_empty() {
            if self.external_tasks.len() == 1 {
                let command = self
                    .external_tasks
                    .values()
                    .next()
                    .filter(|command| !command.is_empty());
                return command.map_or_else(
                    || "Waiting for external task…".to_string(),
                    |command| format!("Waiting for external task · {command}…"),
                );
            }
            return format!("Waiting for {} external tasks…", self.external_tasks.len());
        }

        if let Some(matches) = self.memory_matches {
            return match matches {
                0 => "Searching memory…".to_string(),
                1 => "Recalling memory · 1 match…".to_string(),
                matches => format!("Recalling memory · {matches} matches…"),
            };
        }

        if let Some(providers) = &self.context_providers {
            if providers.is_empty() {
                return "Resolving context…".to_string();
            }
            return format!("Resolving context · {}…", providers.join(", "));
        }

        if self.planning {
            return "Planning…".to_string();
        }

        let Some(mode) = &self.agent_mode else {
            return "Working…".to_string();
        };
        match mode.mode.to_ascii_lowercase().as_str() {
            "explore" | "exploring" => "Exploring…".to_string(),
            "planning" | "plan" => "Planning…".to_string(),
            "general" | "default" => "Working…".to_string(),
            _ if !mode.agent.is_empty() => format!("Working as {}…", mode.agent),
            _ => "Working…".to_string(),
        }
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Convert Core's terminal verification contract into one concise semantic
/// transcript notice. The empty `Skipped` summary is the normal no-verifier
/// case and remains quiet; every meaningful report is visible to the user.
pub(crate) fn verification_notice(summary: &VerificationSummary) -> Option<CoreEventNotice> {
    if summary.status == VerificationStatus::Skipped && summary.report_count == 0 {
        return None;
    }

    let kind = match summary.status {
        VerificationStatus::Passed | VerificationStatus::Skipped => NoticeKind::Info,
        VerificationStatus::NeedsReview => NoticeKind::Warning,
        VerificationStatus::Failed => NoticeKind::Error,
    };
    Some(CoreEventNotice {
        kind,
        message: bounded_fragment(&format_verification_summary(summary), NOTICE_FRAGMENT_CHARS),
    })
}

/// Preserve Core's typed retry semantics in the user-facing tool card.
///
/// Core intentionally stopped guessing whether arbitrary prose is transient.
/// The TUI therefore adds guidance only when the tool supplied a structured
/// `ToolErrorKind`; untyped failures remain unchanged.
pub(crate) fn enrich_tool_failure_output(
    output: &str,
    exit_code: i32,
    error_kind: Option<&ToolErrorKind>,
) -> String {
    let Some(error_kind) = error_kind.filter(|_| exit_code != 0) else {
        return output.to_string();
    };
    let guidance = tool_error_guidance(error_kind);
    if output.trim().is_empty() {
        return guidance;
    }
    if output.contains(&guidance) {
        return output.to_string();
    }
    format!("{}\n{}", output.trim_end(), guidance)
}

fn tool_error_guidance(error_kind: &ToolErrorKind) -> String {
    match error_kind {
        ToolErrorKind::VersionConflict { path, .. } => format!(
            "Typed failure: version conflict · reread {} and retry against its latest revision.",
            bounded_fragment(path, NOTICE_FRAGMENT_CHARS)
        ),
        ToolErrorKind::RemoteGitConflict { code, message } => {
            let message = bounded_fragment(message, NOTICE_FRAGMENT_CHARS);
            format!(
                "Typed failure: remote Git conflict ({}) · {} · refresh repository state before retrying.",
                bounded_fragment(code, STATUS_FRAGMENT_CHARS),
                if message.is_empty() {
                    "the remote rejected the current state"
                } else {
                    message.as_str()
                }
            )
        }
        ToolErrorKind::NotFound { path } => format!(
            "Typed failure: not found · refresh workspace state or correct {}.",
            bounded_fragment(path, NOTICE_FRAGMENT_CHARS)
        ),
        ToolErrorKind::InvalidArgument { message } => format!(
            "Typed failure: invalid arguments · {} · change the tool input before retrying.",
            bounded_fragment(message, NOTICE_FRAGMENT_CHARS)
        ),
        ToolErrorKind::Unsupported { message } => format!(
            "Typed failure: unsupported by this workspace backend · {} · choose a supported operation.",
            bounded_fragment(message, NOTICE_FRAGMENT_CHARS)
        ),
        ToolErrorKind::Timeout { op, duration_ms } => format!(
            "Typed failure: {} timed out after {} · retry only if the operation is still needed.",
            bounded_fragment(op, STATUS_FRAGMENT_CHARS),
            format_duration_ms(*duration_ms)
        ),
        ToolErrorKind::Transport { op } => format!(
            "Typed failure: {} transport failed · retry when the upstream service is reachable.",
            bounded_fragment(op, STATUS_FRAGMENT_CHARS)
        ),
        ToolErrorKind::Cancelled { op } => format!(
            "Typed failure: {} was cancelled by the owning session · no retry was started.",
            bounded_fragment(op, STATUS_FRAGMENT_CHARS)
        ),
        ToolErrorKind::PartialFailure { failed, total } => format!(
            "Typed failure: partial result · {failed} of {total} child operations failed; successful results were retained."
        ),
        ToolErrorKind::RateLimited { retry_after_ms } => retry_after_ms.map_or_else(
            || "Typed failure: rate limited · retry after the provider resets.".to_string(),
            |retry_after_ms| {
                format!(
                    "Typed failure: rate limited · retry after {}.",
                    format_duration_ms(retry_after_ms)
                )
            },
        ),
        _ => "Typed tool failure · inspect the result before retrying.".to_string(),
    }
}

fn notice_kind_for_level(level: &str) -> NoticeKind {
    match level.trim().to_ascii_lowercase().as_str() {
        "error" | "critical" | "fatal" => NoticeKind::Error,
        "warn" | "warning" => NoticeKind::Warning,
        _ => NoticeKind::Info,
    }
}

fn format_duration_ms(duration_ms: u64) -> String {
    if duration_ms >= 60_000 {
        return format!(
            "{}m {:02}s",
            duration_ms / 60_000,
            (duration_ms % 60_000) / 1_000
        );
    }
    if duration_ms >= 1_000 {
        return format!("{:.1}s", duration_ms as f64 / 1_000.0);
    }
    format!("{duration_ms}ms")
}

fn format_metric(value: f64) -> String {
    if !value.is_finite() {
        return "unknown".to_string();
    }
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

fn format_deadline_ms(deadline_ms: u64) -> String {
    i64::try_from(deadline_ms)
        .ok()
        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
        .map(|deadline| deadline.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| format!("unix-ms {deadline_ms}"))
}

fn bounded_fragment(source: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let sanitized =
        crate::system_agents::sanitize_display_text(source, max_chars.saturating_add(1));
    let mut characters = sanitized.chars();
    let mut output = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        output.push('…');
    }
    output
}

#[cfg(test)]
mod tests {
    use a3s_code_core::planning::ExecutionPlan;

    use super::*;

    #[test]
    fn live_status_prioritizes_blocking_phases_without_transcript_noise() {
        let mut status = CoreRunStatus::default();
        assert_eq!(status.activity_label(), "Working…");

        assert!(status
            .observe(&AgentEvent::AgentModeChanged {
                mode: "explore".to_string(),
                agent: "explore".to_string(),
                description: "Read-only exploration".to_string(),
            })
            .is_none());
        status.observe(&AgentEvent::Start {
            prompt: "inspect".to_string(),
        });
        assert_eq!(status.activity_label(), "Exploring…");

        assert!(status
            .observe(&AgentEvent::PlanningStart {
                prompt: "plan".to_string(),
            })
            .is_none());
        assert_eq!(status.activity_label(), "Planning…");

        assert!(status
            .observe(&AgentEvent::ContextResolving {
                providers: vec!["workspace".to_string(), "memory".to_string()],
            })
            .is_none());
        assert_eq!(
            status.activity_label(),
            "Resolving context · workspace, memory…"
        );

        assert!(status
            .observe(&AgentEvent::MemoryRecalled {
                memory_id: "memory-1".to_string(),
                content: "do not render this content".to_string(),
                relevance: 0.9,
            })
            .is_none());
        assert_eq!(status.activity_label(), "Recalling memory · 1 match…");
        assert!(status
            .observe(&AgentEvent::MemoriesSearched {
                query: Some("private query".to_string()),
                tags: vec!["private-tag".to_string()],
                result_count: 2,
            })
            .is_none());
        assert_eq!(status.activity_label(), "Recalling memory · 2 matches…");

        assert!(status
            .observe(&AgentEvent::ExternalTaskPending {
                task_id: "task-1".to_string(),
                session_id: "session".to_string(),
                lane: a3s_code_core::queue::SessionLane::Query,
                command_type: "remote-index".to_string(),
                payload: serde_json::Value::Null,
                timeout_ms: 1_000,
            })
            .is_none());
        assert_eq!(
            status.activity_label(),
            "Waiting for external task · remote-index…"
        );

        assert!(status
            .observe(&AgentEvent::ExternalTaskCompleted {
                task_id: "task-1".to_string(),
                session_id: "session".to_string(),
                success: true,
            })
            .is_none());
        assert_eq!(status.activity_label(), "Recalling memory · 2 matches…");

        status.observe(&AgentEvent::ContextResolved {
            total_items: 2,
            total_tokens: 64,
        });
        status.observe(&AgentEvent::PlanningEnd {
            plan: ExecutionPlan::new("goal", a3s_code_core::planning::Complexity::Simple),
            estimated_steps: 0,
        });
        assert_eq!(status.activity_label(), "Exploring…");
    }

    #[test]
    fn memory_storage_is_visible_without_leaking_ids_or_tags() {
        let mut status = CoreRunStatus::default();
        let notice = status
            .observe(&AgentEvent::MemoryStored {
                memory_id: "secret-id".to_string(),
                memory_type: "episodic\u{1b}]0;hidden\u{7}".to_string(),
                importance: 0.75,
                tags: vec!["private-tag".to_string()],
            })
            .expect("memory write notice");

        assert_eq!(notice.kind, NoticeKind::Info);
        assert_eq!(notice.message, "Stored episodic memory · importance 0.75");
        assert!(!notice.message.contains("secret-id"));
        assert!(!notice.message.contains("private-tag"));
        assert!(!notice.message.contains("hidden"));
    }

    #[test]
    fn verification_notices_surface_only_meaningful_terminal_contracts() {
        let empty = VerificationSummary::from_reports(&[]);
        assert!(verification_notice(&empty).is_none());

        let failed = VerificationSummary {
            status: VerificationStatus::Failed,
            report_count: 1,
            required_check_count: 2,
            pending_required_check_count: 0,
            failed_check_count: 1,
            residual_risk_count: 1,
            pending_subjects: Vec::new(),
            failed_subjects: vec!["tests\u{1b}]0;hidden\u{7}\u{202e}".to_string()],
        };
        let notice = verification_notice(&failed).expect("failed verification notice");
        assert_eq!(notice.kind, NoticeKind::Error);
        assert!(notice.message.contains("Verification failed"));
        assert!(notice.message.contains("tests"));
        assert!(!notice.message.contains("hidden"));
        assert!(!notice.message.contains('\u{202e}'));
    }

    #[test]
    fn operational_failures_are_typed_bounded_notices() {
        let mut status = CoreRunStatus::default();
        let retry = status
            .observe(&AgentEvent::CommandRetry {
                command_id: "cmd-1".to_string(),
                command_type: "tool".to_string(),
                lane: "normal".to_string(),
                attempt: 2,
                delay_ms: 250,
            })
            .expect("retry notice");
        assert_eq!(retry.kind, NoticeKind::Info);
        assert!(retry.message.contains("on normal"));
        assert!(retry.message.contains("attempt 2 after 250ms"));

        let persistence = status
            .observe(&AgentEvent::PersistenceFailed {
                session_id: "session".to_string(),
                operation: "save\u{1b}[2J".to_string(),
                error: "disk unavailable".to_string(),
            })
            .expect("persistence notice");
        assert_eq!(persistence.kind, NoticeKind::Error);
        assert!(!persistence.message.contains('\u{1b}'));
        assert!(persistence.message.contains("disk unavailable"));

        let hard_budget = status
            .observe(&AgentEvent::BudgetThresholdHit {
                resource: "tool_calls".to_string(),
                kind: "hard".to_string(),
                consumed: 12.0,
                limit: 10.0,
                message: Some("stop new work".to_string()),
            })
            .expect("budget notice");
        assert_eq!(hard_budget.kind, NoticeKind::Error);
        assert!(hard_budget.message.contains("12 / 10"));

        let dead_letter = status
            .observe(&AgentEvent::CommandDeadLettered {
                command_id: "cmd-2".to_string(),
                command_type: "index".to_string(),
                lane: "query".to_string(),
                error: "retries exhausted".to_string(),
                attempts: 3,
            })
            .expect("dead-letter notice");
        assert_eq!(dead_letter.kind, NoticeKind::Error);
        assert!(dead_letter.message.contains("after 3 attempts"));

        let queue_alert = status
            .observe(&AgentEvent::QueueAlert {
                level: "warning".to_string(),
                alert_type: "depth".to_string(),
                message: "queue is filling".to_string(),
            })
            .expect("queue alert");
        assert_eq!(queue_alert.kind, NoticeKind::Warning);
        assert!(queue_alert.message.contains("Queue depth"));

        status.observe(&AgentEvent::ExternalTaskPending {
            task_id: "task-failed".to_string(),
            session_id: "session".to_string(),
            lane: a3s_code_core::queue::SessionLane::Execute,
            command_type: "remote-build".to_string(),
            payload: serde_json::Value::Null,
            timeout_ms: 1_000,
        });
        let external = status
            .observe(&AgentEvent::ExternalTaskCompleted {
                task_id: "task-failed".to_string(),
                session_id: "session".to_string(),
                success: false,
            })
            .expect("failed external-task notice");
        assert_eq!(external.kind, NoticeKind::Warning);
        assert!(external.message.contains("remote-build"));
        assert_eq!(status.activity_label(), "Working…");

        let passivation = status
            .observe(&AgentEvent::PassivationRequested {
                reason: "node drain".to_string(),
                deadline_ms: Some(42),
            })
            .expect("passivation notice");
        assert_eq!(passivation.kind, NoticeKind::Warning);
        assert!(passivation.message.contains("1970-01-01T00:00:00.042Z"));

        let memory = status
            .observe(&AgentEvent::MemoryCleared {
                tier: "long_term".to_string(),
                count: 3,
            })
            .expect("memory-clear notice");
        assert_eq!(memory.kind, NoticeKind::Warning);
        assert!(memory.message.contains("Cleared 3 long_term memories"));

        let peer = status
            .observe(&AgentEvent::PeerInvocation {
                from_session_id: "peer-1".to_string(),
                from_tenant_id: None,
                correlation_id: None,
            })
            .expect("peer notice");
        assert_eq!(peer.kind, NoticeKind::Info);
        assert!(peer.message.contains("peer-1"));
    }

    #[test]
    fn typed_tool_failures_add_actionable_guidance_only_on_failure() {
        let conflict = ToolErrorKind::VersionConflict {
            path: "src/lib.rs".to_string(),
            expected: "one".to_string(),
            actual: Some("two".to_string()),
        };
        let output = enrich_tool_failure_output("write rejected", 1, Some(&conflict));
        assert!(output.contains("write rejected"));
        assert!(output.contains("reread src/lib.rs"));

        assert_eq!(
            enrich_tool_failure_output("success", 0, Some(&conflict)),
            "success"
        );
        assert_eq!(
            enrich_tool_failure_output("plain error", 1, None),
            "plain error"
        );
    }

    #[test]
    fn typed_tool_guidance_preserves_safe_operation_details() {
        let invalid = ToolErrorKind::InvalidArgument {
            message: "bad\u{1b}[2J pattern".to_string(),
        };
        let output = enrich_tool_failure_output("failed", 1, Some(&invalid));
        assert!(output.contains("bad pattern"), "{output}");
        assert!(!output.contains('\u{1b}'));

        let timeout = ToolErrorKind::Timeout {
            op: "web_fetch".to_string(),
            duration_ms: 1_500,
        };
        let output = enrich_tool_failure_output("failed", 1, Some(&timeout));
        assert!(
            output.contains("web_fetch timed out after 1.5s"),
            "{output}"
        );
    }

    #[test]
    fn every_current_tool_error_kind_has_specific_retry_semantics() {
        let cases = [
            ToolErrorKind::VersionConflict {
                path: "a".to_string(),
                expected: "one".to_string(),
                actual: None,
            },
            ToolErrorKind::RemoteGitConflict {
                code: "BRANCH_EXISTS".to_string(),
                message: "exists".to_string(),
            },
            ToolErrorKind::NotFound {
                path: "missing".to_string(),
            },
            ToolErrorKind::InvalidArgument {
                message: "bad".to_string(),
            },
            ToolErrorKind::Unsupported {
                message: "unsupported".to_string(),
            },
            ToolErrorKind::Timeout {
                op: "read".to_string(),
                duration_ms: 1_500,
            },
            ToolErrorKind::Transport {
                op: "fetch".to_string(),
            },
            ToolErrorKind::Cancelled {
                op: "search".to_string(),
            },
            ToolErrorKind::PartialFailure {
                failed: 2,
                total: 5,
            },
            ToolErrorKind::RateLimited {
                retry_after_ms: Some(2_000),
            },
        ];

        for kind in cases {
            let output = enrich_tool_failure_output("failed", 1, Some(&kind));
            assert!(output.contains("Typed failure:"), "{kind:?}: {output}");
            assert!(!output.contains("inspect the result"), "{kind:?}: {output}");
        }
    }

    #[test]
    fn fragments_are_terminal_safe_and_bounded() {
        let fragment = bounded_fragment(
            "\u{1b}[2Jalpha\n\tbeta\0gamma\u{1b}]0;hidden title\u{7}\u{202e} and a deliberately long suffix",
            16,
        );
        assert!(!fragment.contains('\u{1b}'));
        assert!(!fragment.contains('\n'));
        assert!(!fragment.contains("hidden title"));
        assert!(!fragment.contains('\u{202e}'));
        assert!(fragment.ends_with('…'));
        assert!(fragment.chars().count() <= 17);
    }
}
