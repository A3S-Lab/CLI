use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use chrono::Utc;
use futures::{stream, StreamExt, TryStreamExt};
use sha2::{Digest, Sha256};

use super::input::{
    normalize_tasks, snapshot, snapshot_for_candidate, validate_candidate, MAX_TASKS, MIN_TASKS,
};
use super::model::{
    SkillOptimizationAuditEvent, SkillOptimizationEdit, SkillOptimizationEditOperation,
    SkillOptimizationGate, SkillOptimizationRequest, SkillOptimizationRun, SkillOptimizationScore,
    SkillOptimizationSnapshot, SkillOptimizationSplit, SkillOptimizationStatus,
    SkillOptimizationTask, SkillOptimizationTaskInput, SKILL_OPTIMIZATION_SCHEMA,
};
use super::store::{create_run, current_runner, mutate_run, read_run};
use crate::evolution::store::{
    looks_sensitive, read_catalog, short_hash, truncate_chars, EvolutionPaths,
};

const MAX_EDIT_BUDGET: usize = 4;
const MAX_INSTRUCTIONS: usize = 16;
const MAX_INSTRUCTION_CHARS: usize = 320;
const MAX_ROLLOUT_CONCURRENCY: usize = 2;
const MAX_VALIDATION_REGRESSION: f32 = 10.0;

#[derive(Debug, Clone)]
pub(super) struct GeneratedOptimizationTask {
    pub(super) prompt: String,
    pub(super) rubric: String,
}

#[derive(Debug, Clone)]
pub(super) struct TrainingObservation {
    pub(super) task: SkillOptimizationTask,
    pub(super) baseline_output: String,
}

#[derive(Debug, Clone)]
pub(super) struct ValidationPair {
    pub(super) task: SkillOptimizationTask,
    pub(super) output_a: String,
    pub(super) output_b: String,
    pub(super) baseline_is_a: bool,
}

#[derive(Debug, Clone)]
pub(super) struct ValidationJudgement {
    pub(super) task_id: String,
    pub(super) score_a: f32,
    pub(super) score_b: f32,
    pub(super) rationale: String,
}

#[async_trait]
pub(super) trait SkillOptimizationModel: Send + Sync {
    async fn generate_tasks(
        &self,
        baseline: &SkillOptimizationSnapshot,
        task_count: usize,
    ) -> anyhow::Result<Vec<GeneratedOptimizationTask>>;

    async fn rollout(
        &self,
        snapshot: &SkillOptimizationSnapshot,
        task: &SkillOptimizationTask,
    ) -> anyhow::Result<String>;

    async fn propose_edits(
        &self,
        baseline: &SkillOptimizationSnapshot,
        observations: &[TrainingObservation],
        edit_budget: usize,
    ) -> anyhow::Result<Vec<SkillOptimizationEdit>>;

    async fn judge_validation(
        &self,
        pairs: &[ValidationPair],
    ) -> anyhow::Result<Vec<ValidationJudgement>>;
}

pub(super) fn queue_run(
    paths: &EvolutionPaths,
    candidate_id: &str,
    request: SkillOptimizationRequest,
) -> anyhow::Result<SkillOptimizationRun> {
    let catalog = read_catalog(paths)?;
    let candidate = catalog
        .candidates
        .iter()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| anyhow!("evolution candidate `{candidate_id}` was not found"))?;
    validate_candidate(candidate)?;

    let edit_budget = request.edit_budget.clamp(1, MAX_EDIT_BUDGET);
    let requested_task_count = request.task_count.clamp(MIN_TASKS, MAX_TASKS);
    let tasks = if request.tasks.is_empty() {
        Vec::new()
    } else {
        normalize_tasks(request.tasks)?
    };
    let now = Utc::now();
    let nonce = format!(
        "{}:{}:{}",
        candidate.id,
        now.timestamp_nanos_opt().unwrap_or_default(),
        std::process::id()
    );
    let id = format!("opt-{}-{}", now.timestamp_millis(), short_hash(&nonce));
    let run = SkillOptimizationRun {
        schema: SKILL_OPTIMIZATION_SCHEMA.to_string(),
        id,
        candidate_id: candidate.id.clone(),
        candidate_title: candidate.title.clone(),
        status: SkillOptimizationStatus::Queued,
        edit_budget,
        requested_task_count,
        baseline: snapshot_for_candidate(candidate),
        proposal: None,
        tasks,
        edits: Vec::new(),
        scores: Vec::new(),
        gate: None,
        model_calls: 0,
        runner: None,
        created_at: now,
        updated_at: now,
        completed_at: None,
        adopted_version: None,
        error: None,
        audit: vec![SkillOptimizationAuditEvent {
            status: SkillOptimizationStatus::Queued,
            at: now,
            note: "queued for isolated task replay and held-out evaluation".to_string(),
        }],
    };
    create_run(paths, &run)?;
    Ok(run)
}

pub(super) async fn execute_run(
    paths: &EvolutionPaths,
    run_id: &str,
    model: Arc<dyn SkillOptimizationModel>,
) -> anyhow::Result<SkillOptimizationRun> {
    let runner = current_runner()?;
    mutate_run_async(paths, run_id, move |run| {
        if run.status != SkillOptimizationStatus::Queued {
            return Err(anyhow!(
                "skill optimization run `{}` is {} and cannot start",
                run.id,
                run.status.label()
            ));
        }
        run.status = SkillOptimizationStatus::Running;
        run.runner = Some(runner);
        run.audit.push(SkillOptimizationAuditEvent {
            status: SkillOptimizationStatus::Running,
            at: Utc::now(),
            note: "started isolated baseline replay".to_string(),
        });
        Ok(())
    })
    .await?;

    let mut interruption_guard = OptimizationInterruptionGuard::new(paths, run_id);

    match execute_run_inner(paths, run_id, model).await {
        Ok(run) => {
            interruption_guard.disarm();
            Ok(run)
        }
        Err(error) => {
            let message = truncate_chars(&format!("{error:#}"), 800);
            if mutate_run_async(paths, run_id, move |run| {
                if run.status != SkillOptimizationStatus::Running {
                    return Ok(());
                }
                let now = Utc::now();
                run.status = SkillOptimizationStatus::Failed;
                run.error = Some(message.clone());
                run.completed_at = Some(now);
                run.audit.push(SkillOptimizationAuditEvent {
                    status: SkillOptimizationStatus::Failed,
                    at: now,
                    note: message.clone(),
                });
                Ok(())
            })
            .await
            .is_ok()
            {
                interruption_guard.disarm();
            }
            Err(error)
        }
    }
}

struct OptimizationInterruptionGuard {
    paths: EvolutionPaths,
    run_id: String,
    armed: bool,
}

impl OptimizationInterruptionGuard {
    fn new(paths: &EvolutionPaths, run_id: &str) -> Self {
        Self {
            paths: paths.clone(),
            run_id: run_id.to_string(),
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for OptimizationInterruptionGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let note = "optimization task was cancelled before completion; start a new run to retry";
        let _ = mutate_run(&self.paths, &self.run_id, |run| {
            if run.status != SkillOptimizationStatus::Running {
                return Ok(());
            }
            let now = Utc::now();
            run.status = SkillOptimizationStatus::Failed;
            run.completed_at = Some(now);
            run.error = Some(note.to_string());
            run.audit.push(SkillOptimizationAuditEvent {
                status: SkillOptimizationStatus::Failed,
                at: now,
                note: note.to_string(),
            });
            Ok(())
        });
    }
}

async fn execute_run_inner(
    paths: &EvolutionPaths,
    run_id: &str,
    model: Arc<dyn SkillOptimizationModel>,
) -> anyhow::Result<SkillOptimizationRun> {
    let mut run = read_run_async(paths, run_id).await?;
    let mut model_calls = 0usize;
    if run.tasks.is_empty() {
        let generated = model
            .generate_tasks(&run.baseline, run.requested_task_count)
            .await
            .context("could not generate optimization tasks")?;
        model_calls += 1;
        run.tasks = normalize_tasks(
            generated
                .into_iter()
                .map(|task| SkillOptimizationTaskInput {
                    prompt: task.prompt,
                    rubric: task.rubric,
                    ..SkillOptimizationTaskInput::default()
                })
                .collect(),
        )?;
    }

    let baseline_outputs = run_rollouts(Arc::clone(&model), &run.baseline, &run.tasks).await?;
    model_calls += run.tasks.len();
    let observations = run
        .tasks
        .iter()
        .filter(|task| task.split == SkillOptimizationSplit::Train)
        .map(|task| {
            Ok(TrainingObservation {
                task: task.clone(),
                baseline_output: baseline_outputs
                    .get(&task.id)
                    .cloned()
                    .ok_or_else(|| anyhow!("baseline rollout `{}` is missing", task.id))?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let proposed_edits = model
        .propose_edits(&run.baseline, &observations, run.edit_budget)
        .await
        .context("could not reflect on baseline failures")?;
    model_calls += 1;
    let (proposal, edits) = apply_bounded_edits(&run.baseline, proposed_edits, run.edit_budget)?;

    let validation_tasks = run
        .tasks
        .iter()
        .filter(|task| task.split == SkillOptimizationSplit::Validation)
        .cloned()
        .collect::<Vec<_>>();
    let candidate_outputs = run_rollouts(Arc::clone(&model), &proposal, &validation_tasks).await?;
    model_calls += validation_tasks.len();
    let pairs = validation_tasks
        .iter()
        .map(|task| {
            let baseline = baseline_outputs
                .get(&task.id)
                .cloned()
                .ok_or_else(|| anyhow!("validation baseline `{}` is missing", task.id))?;
            let candidate = candidate_outputs
                .get(&task.id)
                .cloned()
                .ok_or_else(|| anyhow!("validation candidate `{}` is missing", task.id))?;
            let baseline_is_a = blinded_baseline_is_a(&run.id, &task.id);
            let (output_a, output_b) = if baseline_is_a {
                (baseline, candidate)
            } else {
                (candidate, baseline)
            };
            Ok(ValidationPair {
                task: task.clone(),
                output_a,
                output_b,
                baseline_is_a,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let judgements = model
        .judge_validation(&pairs)
        .await
        .context("could not score held-out validation pairs")?;
    model_calls += 1;
    let scores = map_scores(&pairs, judgements)?;
    let gate = evaluate_gate(&scores)?;
    let final_status = if gate.accepted {
        SkillOptimizationStatus::Staged
    } else {
        SkillOptimizationStatus::Rejected
    };
    let completed_at = Utc::now();

    mutate_run_async(paths, run_id, move |stored| {
        if stored.status != SkillOptimizationStatus::Running {
            return Err(anyhow!(
                "skill optimization run `{}` changed while it was executing",
                stored.id
            ));
        }
        stored.status = final_status;
        stored.tasks = run.tasks.clone();
        stored.proposal = Some(proposal.clone());
        stored.edits = edits.clone();
        stored.scores = scores.clone();
        stored.gate = Some(gate.clone());
        stored.model_calls = model_calls;
        stored.completed_at = Some(completed_at);
        stored.error = None;
        stored.audit.push(SkillOptimizationAuditEvent {
            status: final_status,
            at: completed_at,
            note: gate.reason.clone(),
        });
        Ok(())
    })
    .await?;
    read_run_async(paths, run_id).await
}

async fn read_run_async(
    paths: &EvolutionPaths,
    run_id: &str,
) -> anyhow::Result<SkillOptimizationRun> {
    let paths = paths.clone();
    let run_id = run_id.to_string();
    tokio::task::spawn_blocking(move || read_run(&paths, &run_id))
        .await
        .context("skill optimization reader task did not complete")?
}

async fn mutate_run_async<R, F>(
    paths: &EvolutionPaths,
    run_id: &str,
    mutation: F,
) -> anyhow::Result<R>
where
    R: Send + 'static,
    F: FnOnce(&mut SkillOptimizationRun) -> anyhow::Result<R> + Send + 'static,
{
    let paths = paths.clone();
    let run_id = run_id.to_string();
    tokio::task::spawn_blocking(move || mutate_run(&paths, &run_id, mutation))
        .await
        .context("skill optimization writer task did not complete")?
}

async fn run_rollouts(
    model: Arc<dyn SkillOptimizationModel>,
    snapshot: &SkillOptimizationSnapshot,
    tasks: &[SkillOptimizationTask],
) -> anyhow::Result<HashMap<String, String>> {
    let snapshot = Arc::new(snapshot.clone());
    stream::iter(tasks.iter().cloned().map(|task| {
        let model = Arc::clone(&model);
        let snapshot = Arc::clone(&snapshot);
        async move {
            let output = model
                .rollout(snapshot.as_ref(), &task)
                .await
                .with_context(|| format!("rollout `{}` failed", task.id))?;
            if output.trim().is_empty() {
                return Err(anyhow!("rollout `{}` returned no output", task.id));
            }
            Ok((task.id, truncate_chars(output.trim(), 8_000)))
        }
    }))
    .buffer_unordered(MAX_ROLLOUT_CONCURRENCY)
    .try_collect()
    .await
}

fn apply_bounded_edits(
    baseline: &SkillOptimizationSnapshot,
    proposed: Vec<SkillOptimizationEdit>,
    edit_budget: usize,
) -> anyhow::Result<(SkillOptimizationSnapshot, Vec<SkillOptimizationEdit>)> {
    let mut instructions = baseline.instructions.clone();
    let mut applied = Vec::new();
    for edit in proposed
        .into_iter()
        .take(edit_budget.clamp(1, MAX_EDIT_BUDGET))
    {
        let target = edit
            .target
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let content = edit
            .content
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if target.as_deref().is_some_and(looks_sensitive)
            || content.as_deref().is_some_and(looks_sensitive)
        {
            continue;
        }
        let changed = match edit.operation {
            SkillOptimizationEditOperation::Append => {
                let Some(content) = content.as_deref() else {
                    continue;
                };
                if content.chars().count() < 8
                    || content.chars().count() > MAX_INSTRUCTION_CHARS
                    || instructions.len() >= MAX_INSTRUCTIONS
                    || instructions.iter().any(|value| same_text(value, content))
                {
                    false
                } else {
                    instructions.push(content.to_string());
                    true
                }
            }
            SkillOptimizationEditOperation::Replace => {
                let (Some(target), Some(content)) = (target.as_deref(), content.as_deref()) else {
                    continue;
                };
                if content.chars().count() < 8 || content.chars().count() > MAX_INSTRUCTION_CHARS {
                    false
                } else if let Some(index) = instructions.iter().position(|value| value == target) {
                    if instructions.iter().any(|value| same_text(value, content)) {
                        false
                    } else {
                        instructions[index] = content.to_string();
                        true
                    }
                } else {
                    false
                }
            }
            SkillOptimizationEditOperation::Delete => {
                let Some(target) = target.as_deref() else {
                    continue;
                };
                if instructions.len() <= 1 {
                    false
                } else if let Some(index) = instructions.iter().position(|value| value == target) {
                    instructions.remove(index);
                    true
                } else {
                    false
                }
            }
        };
        if changed {
            let rationale = edit.rationale.trim();
            applied.push(SkillOptimizationEdit {
                operation: edit.operation,
                target: match edit.operation {
                    SkillOptimizationEditOperation::Append => None,
                    SkillOptimizationEditOperation::Replace
                    | SkillOptimizationEditOperation::Delete => target.clone(),
                },
                content: match edit.operation {
                    SkillOptimizationEditOperation::Append
                    | SkillOptimizationEditOperation::Replace => content.clone(),
                    SkillOptimizationEditOperation::Delete => None,
                },
                rationale: if looks_sensitive(rationale) {
                    "redacted by the local safety filter".to_string()
                } else {
                    truncate_chars(rationale, 600)
                },
            });
        }
    }
    if applied.is_empty() {
        return Err(anyhow!("optimizer proposed no applicable bounded edits"));
    }
    let baseline_chars = baseline
        .instructions
        .iter()
        .map(|value| value.chars().count())
        .sum::<usize>();
    let candidate_chars = instructions
        .iter()
        .map(|value| value.chars().count())
        .sum::<usize>();
    let growth_limit = baseline_chars.saturating_mul(135) / 100 + 480;
    if candidate_chars > growth_limit {
        return Err(anyhow!("bounded edits exceeded the Skill growth limit"));
    }
    Ok((snapshot(baseline.summary.clone(), instructions), applied))
}

fn same_text(left: &str, right: &str) -> bool {
    left.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .eq_ignore_ascii_case(&right.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn blinded_baseline_is_a(run_id: &str, task_id: &str) -> bool {
    let digest = Sha256::digest(format!("{run_id}:{task_id}").as_bytes());
    digest[0] % 2 == 0
}

fn map_scores(
    pairs: &[ValidationPair],
    judgements: Vec<ValidationJudgement>,
) -> anyhow::Result<Vec<SkillOptimizationScore>> {
    if judgements.len() != pairs.len() {
        return Err(anyhow!("validation judge returned an incomplete score set"));
    }
    let mut by_id = HashMap::new();
    for score in judgements {
        if !score.score_a.is_finite() || !score.score_b.is_finite() {
            return Err(anyhow!("validation judge returned a non-finite score"));
        }
        if by_id.insert(score.task_id.clone(), score).is_some() {
            return Err(anyhow!("validation judge returned duplicate task scores"));
        }
    }
    pairs
        .iter()
        .map(|pair| {
            let score = by_id
                .get(&pair.task.id)
                .ok_or_else(|| anyhow!("validation score `{}` is missing", pair.task.id))?;
            let score_a = score.score_a.clamp(0.0, 100.0);
            let score_b = score.score_b.clamp(0.0, 100.0);
            let (baseline, candidate) = if pair.baseline_is_a {
                (score_a, score_b)
            } else {
                (score_b, score_a)
            };
            Ok(SkillOptimizationScore {
                task_id: pair.task.id.clone(),
                baseline,
                candidate,
                delta: candidate - baseline,
                rationale: if looks_sensitive(score.rationale.trim()) {
                    "redacted by the local safety filter".to_string()
                } else {
                    truncate_chars(score.rationale.trim(), 600)
                },
            })
        })
        .collect()
}

fn evaluate_gate(scores: &[SkillOptimizationScore]) -> anyhow::Result<SkillOptimizationGate> {
    if scores.len() < 2 {
        return Err(anyhow!("held-out validation requires at least two tasks"));
    }
    let count = scores.len() as f32;
    let baseline_score = scores.iter().map(|score| score.baseline).sum::<f32>() / count;
    let candidate_score = scores.iter().map(|score| score.candidate).sum::<f32>() / count;
    let improvement = candidate_score - baseline_score;
    let worst_task_regression = scores
        .iter()
        .map(|score| (score.baseline - score.candidate).max(0.0))
        .fold(0.0_f32, f32::max);
    let strict_improvement = candidate_score > baseline_score;
    let regression_guard_passed = worst_task_regression <= MAX_VALIDATION_REGRESSION;
    let accepted = strict_improvement && regression_guard_passed;
    let reason = if !strict_improvement {
        format!(
            "rejected: held-out score did not strictly improve ({baseline_score:.1} → {candidate_score:.1})"
        )
    } else if !regression_guard_passed {
        format!("rejected: one held-out task regressed by {worst_task_regression:.1} points")
    } else {
        format!(
            "staged: held-out score improved strictly ({baseline_score:.1} → {candidate_score:.1}) with no regression above {MAX_VALIDATION_REGRESSION:.0} points"
        )
    };
    Ok(SkillOptimizationGate {
        baseline_score,
        candidate_score,
        improvement,
        worst_task_regression,
        strict_improvement,
        regression_guard_passed,
        accepted,
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::model::{
        EvolutionAuditEvent, EvolutionCandidate, EvolutionKind, EvolutionState, EvolutionVersion,
    };
    use crate::evolution::optimization::input::{snapshot, snapshot_digest};
    use crate::evolution::optimization::{adopt_skill_optimization, skill_optimization};
    use crate::evolution::store::mutate_catalog;

    struct ImprovingModel;

    #[async_trait]
    impl SkillOptimizationModel for ImprovingModel {
        async fn generate_tasks(
            &self,
            _baseline: &SkillOptimizationSnapshot,
            task_count: usize,
        ) -> anyhow::Result<Vec<GeneratedOptimizationTask>> {
            Ok((0..task_count)
                .map(|index| GeneratedOptimizationTask {
                    prompt: format!("Verify the focused change for component {index}."),
                    rubric: "Names a focused target before any broad verification and preserves the first diagnostic."
                        .to_string(),
                })
                .collect())
        }

        async fn rollout(
            &self,
            snapshot: &SkillOptimizationSnapshot,
            task: &SkillOptimizationTask,
        ) -> anyhow::Result<String> {
            if snapshot
                .instructions
                .iter()
                .any(|instruction| instruction.contains("smallest relevant"))
            {
                Ok(format!(
                    "Optimized plan for {}: run the smallest relevant target and preserve its first diagnostic.",
                    task.id
                ))
            } else {
                Ok(format!("Baseline plan for {}: run tests.", task.id))
            }
        }

        async fn propose_edits(
            &self,
            _baseline: &SkillOptimizationSnapshot,
            _observations: &[TrainingObservation],
            _edit_budget: usize,
        ) -> anyhow::Result<Vec<SkillOptimizationEdit>> {
            Ok(vec![SkillOptimizationEdit {
                operation: SkillOptimizationEditOperation::Replace,
                target: Some("Run the relevant tests.".to_string()),
                content: Some(
                    "Run the smallest relevant test target and preserve its first diagnostic before broader checks."
                        .to_string(),
                ),
                rationale: "Make verification order and evidence retention explicit.".to_string(),
            }])
        }

        async fn judge_validation(
            &self,
            pairs: &[ValidationPair],
        ) -> anyhow::Result<Vec<ValidationJudgement>> {
            Ok(pairs
                .iter()
                .map(|pair| ValidationJudgement {
                    task_id: pair.task.id.clone(),
                    score_a: if pair.output_a.contains("Optimized plan") {
                        86.0
                    } else {
                        58.0
                    },
                    score_b: if pair.output_b.contains("Optimized plan") {
                        86.0
                    } else {
                        58.0
                    },
                    rationale:
                        "The higher score names the focused target and diagnostic retention."
                            .to_string(),
                })
                .collect())
        }
    }

    fn baseline() -> SkillOptimizationSnapshot {
        snapshot(
            "Verify changes efficiently.".to_string(),
            vec![
                "Run the relevant tests.".to_string(),
                "Report the result.".to_string(),
            ],
        )
    }

    #[test]
    fn bounded_edits_apply_exact_targets_and_clip_the_budget() {
        let edits = vec![
            SkillOptimizationEdit {
                operation: SkillOptimizationEditOperation::Replace,
                target: Some("Run the relevant tests.".to_string()),
                content: Some(
                    "Run the smallest relevant test target before broad checks.".to_string(),
                ),
                rationale: "Make the first check concrete.".to_string(),
            },
            SkillOptimizationEdit {
                operation: SkillOptimizationEditOperation::Append,
                target: None,
                content: Some("Preserve the first failing diagnostic before retrying.".to_string()),
                rationale: "Retain evidence.".to_string(),
            },
        ];

        let (candidate, applied) = apply_bounded_edits(&baseline(), edits, 1).unwrap();

        assert_eq!(applied.len(), 1);
        assert_eq!(candidate.instructions.len(), 2);
        assert!(candidate.instructions[0].contains("smallest relevant"));
        assert_ne!(candidate.digest, baseline().digest);
    }

    #[test]
    fn task_splits_are_all_explicit_or_host_assigned() {
        let mut tasks = (0..4)
            .map(|index| SkillOptimizationTaskInput {
                id: Some(format!("task-{index}")),
                prompt: format!("Verify the focused behavior for case {index}."),
                rubric: "Names the focused check and a concrete success condition.".to_string(),
                ..SkillOptimizationTaskInput::default()
            })
            .collect::<Vec<_>>();
        tasks[0].split = Some(SkillOptimizationSplit::Train);
        assert!(normalize_tasks(tasks.clone()).is_err());

        tasks[1].split = Some(SkillOptimizationSplit::Train);
        tasks[2].split = Some(SkillOptimizationSplit::Validation);
        tasks[3].split = Some(SkillOptimizationSplit::Validation);
        let normalized = normalize_tasks(tasks).unwrap();

        assert_eq!(normalized[0].split, SkillOptimizationSplit::Train);
        assert_eq!(normalized[3].split, SkillOptimizationSplit::Validation);
    }

    #[test]
    fn snapshot_digest_preserves_field_boundaries() {
        assert_ne!(
            snapshot_digest("a", &["b\0c".to_string()]),
            snapshot_digest("a\0b", &["c".to_string()])
        );
    }

    #[test]
    fn held_out_gate_requires_improvement_without_large_regressions() {
        let accepted = evaluate_gate(&[
            SkillOptimizationScore {
                task_id: "one".into(),
                baseline: 60.0,
                candidate: 70.0,
                delta: 10.0,
                rationale: String::new(),
            },
            SkillOptimizationScore {
                task_id: "two".into(),
                baseline: 70.0,
                candidate: 72.0,
                delta: 2.0,
                rationale: String::new(),
            },
        ])
        .unwrap();
        assert!(accepted.accepted);

        let rejected = evaluate_gate(&[
            SkillOptimizationScore {
                task_id: "one".into(),
                baseline: 60.0,
                candidate: 90.0,
                delta: 30.0,
                rationale: String::new(),
            },
            SkillOptimizationScore {
                task_id: "two".into(),
                baseline: 90.0,
                candidate: 70.0,
                delta: -20.0,
                rationale: String::new(),
            },
        ])
        .unwrap();
        assert!(!rejected.accepted);
        assert!(!rejected.regression_guard_passed);
    }

    #[test]
    fn validation_mapping_rejects_non_finite_or_duplicate_scores() {
        let task = SkillOptimizationTask {
            id: "held-out".to_string(),
            prompt: "Verify one held-out behavior.".to_string(),
            rubric: "Names a concrete focused check.".to_string(),
            split: SkillOptimizationSplit::Validation,
        };
        let pair = ValidationPair {
            task,
            output_a: "A".to_string(),
            output_b: "B".to_string(),
            baseline_is_a: true,
        };
        assert!(map_scores(
            std::slice::from_ref(&pair),
            vec![ValidationJudgement {
                task_id: "held-out".to_string(),
                score_a: f32::NAN,
                score_b: 50.0,
                rationale: String::new(),
            }]
        )
        .is_err());

        let duplicate = ValidationJudgement {
            task_id: "held-out".to_string(),
            score_a: 50.0,
            score_b: 60.0,
            rationale: String::new(),
        };
        assert!(map_scores(&[pair.clone(), pair], vec![duplicate.clone(), duplicate]).is_err());
    }

    #[test]
    fn dropping_an_active_execution_marks_the_run_failed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = EvolutionPaths::new(temp.path());
        mutate_catalog(&paths, |catalog| {
            catalog.candidates.push(skill_candidate());
            Ok(())
        })
        .unwrap();
        let queued =
            queue_run(&paths, "skill-focused", SkillOptimizationRequest::default()).unwrap();
        mutate_run(&paths, &queued.id, |run| {
            run.status = SkillOptimizationStatus::Running;
            run.runner = Some(current_runner()?);
            Ok(())
        })
        .unwrap();

        drop(OptimizationInterruptionGuard::new(&paths, &queued.id));

        let interrupted = read_run(&paths, &queued.id).unwrap();
        assert_eq!(interrupted.status, SkillOptimizationStatus::Failed);
        assert!(interrupted
            .error
            .as_deref()
            .is_some_and(|error| error.contains("task was cancelled")));
    }

    #[tokio::test]
    async fn optimization_stays_staged_until_adoption_creates_a_version() {
        let temp = tempfile::tempdir().unwrap();
        let paths = EvolutionPaths::new(temp.path());
        mutate_catalog(&paths, |catalog| {
            catalog.candidates.push(skill_candidate());
            Ok(())
        })
        .unwrap();
        let queued =
            queue_run(&paths, "skill-focused", SkillOptimizationRequest::default()).unwrap();

        let staged = execute_run(&paths, &queued.id, Arc::new(ImprovingModel))
            .await
            .unwrap();

        assert_eq!(staged.status, SkillOptimizationStatus::Staged);
        assert!(staged.gate.as_ref().unwrap().accepted);
        assert_eq!(staged.model_calls, 9);
        assert!(!paths.skill_root.join("learned-focused-checks").exists());

        let adopted = adopt_skill_optimization(&paths, &queued.id).unwrap();

        assert_eq!(adopted.candidate.current_version, Some(1));
        assert!(adopted.requires_session_reload);
        assert_eq!(
            skill_optimization(&paths, &queued.id).unwrap().status,
            SkillOptimizationStatus::Adopted
        );
        let skill = std::fs::read_to_string(
            paths
                .skill_root
                .join("learned-focused-checks")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(skill.contains("smallest relevant test target"));
    }

    fn skill_candidate() -> EvolutionCandidate {
        let now = Utc::now();
        EvolutionCandidate {
            id: "skill-focused".to_string(),
            kind: EvolutionKind::Skill,
            pattern_key: "skill.focused-checks".to_string(),
            pattern_aliases: Vec::new(),
            title: "Focused checks".to_string(),
            summary: "Verify changes efficiently.".to_string(),
            instructions: vec![
                "Run the relevant tests.".to_string(),
                "Report the result.".to_string(),
            ],
            state: EvolutionState::Ready,
            evidence: Vec::new(),
            occurrences: 3,
            distinct_sessions: 2,
            confidence: 0.95,
            importance: 0.9,
            maturity: 0.95,
            has_conflicts: false,
            update_available: false,
            activation_pending: false,
            created_at: now,
            updated_at: now,
            ready_at: Some(now),
            materialized_at: None,
            rejected_at: None,
            rolled_back_at: None,
            rejection_reason: None,
            asset_path: None,
            current_version: None,
            versions: Vec::<EvolutionVersion>::new(),
            audit: Vec::<EvolutionAuditEvent>::new(),
        }
    }
}
