use std::collections::HashSet;

use anyhow::anyhow;
use sha2::{Digest, Sha256};

use super::model::{
    SkillOptimizationSnapshot, SkillOptimizationSplit, SkillOptimizationTask,
    SkillOptimizationTaskInput,
};
use crate::evolution::model::{EvolutionCandidate, EvolutionKind, EvolutionState};
use crate::evolution::store::{looks_sensitive, slugify, truncate_chars};

pub(super) const MIN_TASKS: usize = 4;
pub(super) const MAX_TASKS: usize = 8;

pub(super) fn normalize_tasks(
    inputs: Vec<SkillOptimizationTaskInput>,
) -> anyhow::Result<Vec<SkillOptimizationTask>> {
    if !(MIN_TASKS..=MAX_TASKS).contains(&inputs.len()) {
        return Err(anyhow!(
            "skill optimization requires {MIN_TASKS} to {MAX_TASKS} tasks"
        ));
    }
    let has_explicit_split = inputs.iter().any(|task| task.split.is_some());
    let train_tasks = inputs
        .iter()
        .filter(|task| task.split == Some(SkillOptimizationSplit::Train))
        .count();
    let validation_tasks = inputs
        .iter()
        .filter(|task| task.split == Some(SkillOptimizationSplit::Validation))
        .count();
    if has_explicit_split
        && (inputs.iter().any(|task| task.split.is_none())
            || train_tasks < 2
            || validation_tasks < 2)
    {
        return Err(anyhow!(
            "explicit optimization splits require every task to declare a split, with at least two train and two validation tasks"
        ));
    }
    let train_count = inputs.len() / 2;
    let mut seen = HashSet::new();
    inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| {
            let prompt = truncate_chars(input.prompt.trim(), 2_000);
            let rubric = truncate_chars(input.rubric.trim(), 1_200);
            if prompt.chars().count() < 8 || rubric.chars().count() < 8 {
                return Err(anyhow!(
                    "optimization tasks need a prompt and concrete rubric"
                ));
            }
            if looks_sensitive(&prompt) || looks_sensitive(&rubric) {
                return Err(anyhow!("optimization task contains secret-shaped content"));
            }
            let base_id = input
                .id
                .as_deref()
                .map(|id| slugify(id, "task"))
                .unwrap_or_else(|| format!("task-{}", index + 1));
            let id = if seen.insert(base_id.clone()) {
                base_id
            } else {
                format!("{base_id}-{}", index + 1)
            };
            Ok(SkillOptimizationTask {
                id,
                prompt,
                rubric,
                split: if has_explicit_split {
                    input.split.ok_or_else(|| {
                        anyhow!("explicit optimization task split disappeared during validation")
                    })?
                } else if index < train_count {
                    SkillOptimizationSplit::Train
                } else {
                    SkillOptimizationSplit::Validation
                },
            })
        })
        .collect()
}

pub(super) fn snapshot_for_candidate(candidate: &EvolutionCandidate) -> SkillOptimizationSnapshot {
    snapshot(candidate.summary.clone(), candidate.instructions.clone())
}

pub(super) fn snapshot(summary: String, instructions: Vec<String>) -> SkillOptimizationSnapshot {
    let digest = snapshot_digest(&summary, &instructions);
    SkillOptimizationSnapshot {
        summary,
        instructions,
        digest,
    }
}

pub(super) fn snapshot_digest(summary: &str, instructions: &[String]) -> String {
    let mut hasher = Sha256::new();
    hash_snapshot_field(&mut hasher, summary.as_bytes());
    hasher.update(
        u64::try_from(instructions.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for instruction in instructions {
        hash_snapshot_field(&mut hasher, instruction.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn hash_snapshot_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

pub(super) fn validate_candidate(candidate: &EvolutionCandidate) -> anyhow::Result<()> {
    if candidate.kind != EvolutionKind::Skill {
        return Err(anyhow!("only Skill evolution candidates can be optimized"));
    }
    if candidate.state == EvolutionState::Rejected {
        return Err(anyhow!(
            "reopen the rejected Skill candidate before optimizing it"
        ));
    }
    if candidate.has_conflicts {
        return Err(anyhow!(
            "resolve conflicting Skill evidence before optimization"
        ));
    }
    if candidate.instructions.is_empty() {
        return Err(anyhow!("Skill candidate has no trainable instructions"));
    }
    Ok(())
}
