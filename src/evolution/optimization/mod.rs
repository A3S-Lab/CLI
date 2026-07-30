mod engine;
mod input;
mod llm;
mod model;
mod store;

use std::sync::Arc;

use a3s_code_core::LlmClient;
use anyhow::anyhow;
use chrono::Utc;

pub(crate) use model::{
    SkillOptimizationRequest, SkillOptimizationRun, SkillOptimizationRunSummary,
    SkillOptimizationStatus,
};

use engine::{execute_run, queue_run};
use llm::LlmSkillOptimizationModel;
use model::{SkillOptimizationAuditEvent, SkillOptimizationStatus as Status};
use store::{list_run_summaries, list_runs, mutate_run, read_run};

use super::materialize::{materialize_optimized_candidate, OptimizedSkillMaterialization};
use super::model::EvolutionMutationResult;
use super::store::EvolutionPaths;

pub(super) fn queue_skill_optimization(
    paths: &EvolutionPaths,
    candidate_id: &str,
    request: SkillOptimizationRequest,
) -> anyhow::Result<SkillOptimizationRun> {
    queue_run(paths, candidate_id, request)
}

pub(super) async fn execute_skill_optimization(
    paths: &EvolutionPaths,
    run_id: &str,
    client: Arc<dyn LlmClient>,
) -> anyhow::Result<SkillOptimizationRun> {
    execute_run(paths, run_id, LlmSkillOptimizationModel::new(client)).await
}

pub(super) fn skill_optimization(
    paths: &EvolutionPaths,
    run_id: &str,
) -> anyhow::Result<SkillOptimizationRun> {
    read_run(paths, run_id)
}

pub(super) fn skill_optimizations(
    paths: &EvolutionPaths,
    candidate_id: Option<&str>,
) -> anyhow::Result<Vec<SkillOptimizationRun>> {
    list_runs(paths, candidate_id)
}

pub(super) fn skill_optimization_summaries(
    paths: &EvolutionPaths,
    candidate_id: Option<&str>,
) -> anyhow::Result<Vec<SkillOptimizationRunSummary>> {
    list_run_summaries(paths, candidate_id)
}

pub(super) fn adopt_skill_optimization(
    paths: &EvolutionPaths,
    run_id: &str,
) -> anyhow::Result<EvolutionMutationResult> {
    let run = read_run(paths, run_id)?;
    if run.status != Status::Staged {
        return Err(anyhow!(
            "skill optimization run `{run_id}` is {} and cannot be adopted",
            run.status.label()
        ));
    }
    if !run.gate.as_ref().is_some_and(|gate| gate.accepted) {
        return Err(anyhow!(
            "skill optimization run `{run_id}` did not pass its gate"
        ));
    }
    let proposal = run
        .proposal
        .as_ref()
        .ok_or_else(|| anyhow!("skill optimization run `{run_id}` has no proposal"))?;
    let result = materialize_optimized_candidate(
        paths,
        &run.candidate_id,
        OptimizedSkillMaterialization {
            run_id: run.id.clone(),
            baseline_digest: run.baseline.digest.clone(),
            proposal_digest: proposal.digest.clone(),
            summary: proposal.summary.clone(),
            instructions: proposal.instructions.clone(),
        },
    )?;
    let version = result.candidate.current_version;
    if let Err(error) = mutate_run(paths, run_id, |stored| {
        if stored.status != Status::Staged {
            return Err(anyhow!(
                "optimization status changed before adoption completed"
            ));
        }
        stored.status = Status::Adopted;
        stored.adopted_version = version;
        stored.audit.push(SkillOptimizationAuditEvent {
            status: Status::Adopted,
            at: Utc::now(),
            note: format!(
                "adopted as immutable Skill version v{}",
                version.unwrap_or_default()
            ),
        });
        Ok(())
    }) {
        tracing::warn!(%error, %run_id, "Skill version was adopted but optimization audit update was deferred");
    }
    Ok(result)
}

pub(super) fn dismiss_skill_optimization(
    paths: &EvolutionPaths,
    run_id: &str,
) -> anyhow::Result<SkillOptimizationRun> {
    mutate_run(paths, run_id, |run| {
        if !matches!(
            run.status,
            Status::Staged | Status::Rejected | Status::Failed
        ) {
            return Err(anyhow!(
                "skill optimization run `{run_id}` is {} and cannot be dismissed",
                run.status.label()
            ));
        }
        run.status = Status::Dismissed;
        run.audit.push(SkillOptimizationAuditEvent {
            status: Status::Dismissed,
            at: Utc::now(),
            note: "dismissed during local review".to_string(),
        });
        Ok(run.clone())
    })
}

pub(super) fn snapshot_digest(summary: &str, instructions: &[String]) -> String {
    input::snapshot_digest(summary, instructions)
}
