use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub(crate) const SKILL_OPTIMIZATION_SCHEMA: &str = "a3s.code.skill-optimization.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SkillOptimizationStatus {
    Queued,
    Running,
    Staged,
    Rejected,
    Adopted,
    Dismissed,
    Failed,
}

impl SkillOptimizationStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Staged => "staged",
            Self::Rejected => "rejected",
            Self::Adopted => "adopted",
            Self::Dismissed => "dismissed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SkillOptimizationSplit {
    Train,
    Validation,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SkillOptimizationRequest {
    pub(crate) tasks: Vec<SkillOptimizationTaskInput>,
    pub(crate) edit_budget: usize,
    pub(crate) task_count: usize,
}

impl Default for SkillOptimizationRequest {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            edit_budget: 3,
            task_count: 4,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SkillOptimizationTaskInput {
    pub(crate) id: Option<String>,
    pub(crate) prompt: String,
    pub(crate) rubric: String,
    pub(crate) split: Option<SkillOptimizationSplit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillOptimizationTask {
    pub(crate) id: String,
    pub(crate) prompt: String,
    pub(crate) rubric: String,
    pub(crate) split: SkillOptimizationSplit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillOptimizationSnapshot {
    pub(crate) summary: String,
    pub(crate) instructions: Vec<String>,
    pub(crate) digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SkillOptimizationEditOperation {
    Append,
    Replace,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillOptimizationEdit {
    pub(crate) operation: SkillOptimizationEditOperation,
    pub(crate) target: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillOptimizationScore {
    pub(crate) task_id: String,
    pub(crate) baseline: f32,
    pub(crate) candidate: f32,
    pub(crate) delta: f32,
    pub(crate) rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillOptimizationGate {
    pub(crate) baseline_score: f32,
    pub(crate) candidate_score: f32,
    pub(crate) improvement: f32,
    pub(crate) worst_task_regression: f32,
    pub(crate) strict_improvement: bool,
    pub(crate) regression_guard_passed: bool,
    pub(crate) accepted: bool,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillOptimizationAuditEvent {
    pub(crate) status: SkillOptimizationStatus,
    pub(crate) at: DateTime<Utc>,
    pub(crate) note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillOptimizationRunner {
    pub(crate) pid: u32,
    pub(crate) process_started_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillOptimizationRun {
    pub(crate) schema: String,
    pub(crate) id: String,
    pub(crate) candidate_id: String,
    pub(crate) candidate_title: String,
    pub(crate) status: SkillOptimizationStatus,
    pub(crate) edit_budget: usize,
    pub(crate) requested_task_count: usize,
    pub(crate) baseline: SkillOptimizationSnapshot,
    pub(crate) proposal: Option<SkillOptimizationSnapshot>,
    pub(crate) tasks: Vec<SkillOptimizationTask>,
    pub(crate) edits: Vec<SkillOptimizationEdit>,
    pub(crate) scores: Vec<SkillOptimizationScore>,
    pub(crate) gate: Option<SkillOptimizationGate>,
    pub(crate) model_calls: usize,
    #[serde(default)]
    pub(crate) runner: Option<SkillOptimizationRunner>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) completed_at: Option<DateTime<Utc>>,
    pub(crate) adopted_version: Option<u32>,
    pub(crate) error: Option<String>,
    pub(crate) audit: Vec<SkillOptimizationAuditEvent>,
}

impl SkillOptimizationRun {
    pub(crate) fn summary(&self) -> SkillOptimizationRunSummary {
        SkillOptimizationRunSummary {
            id: self.id.clone(),
            candidate_id: self.candidate_id.clone(),
            candidate_title: self.candidate_title.clone(),
            status: self.status,
            edit_count: self.edits.len(),
            task_count: self.tasks.len(),
            baseline_score: self.gate.as_ref().map(|gate| gate.baseline_score),
            candidate_score: self.gate.as_ref().map(|gate| gate.candidate_score),
            improvement: self.gate.as_ref().map(|gate| gate.improvement),
            created_at: self.created_at,
            updated_at: self.updated_at,
            adopted_version: self.adopted_version,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillOptimizationRunSummary {
    pub(crate) id: String,
    pub(crate) candidate_id: String,
    pub(crate) candidate_title: String,
    pub(crate) status: SkillOptimizationStatus,
    pub(crate) edit_count: usize,
    pub(crate) task_count: usize,
    pub(crate) baseline_score: Option<f32>,
    pub(crate) candidate_score: Option<f32>,
    pub(crate) improvement: Option<f32>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) adopted_version: Option<u32>,
}
