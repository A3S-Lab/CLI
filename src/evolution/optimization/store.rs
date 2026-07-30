use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, Context};
use chrono::Utc;
use fs2::FileExt;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use super::model::{
    SkillOptimizationAuditEvent, SkillOptimizationRun, SkillOptimizationRunSummary,
    SkillOptimizationRunner, SkillOptimizationStatus, SKILL_OPTIMIZATION_SCHEMA,
};
use crate::evolution::store::EvolutionPaths;

const MAX_LISTED_RUNS: usize = 100;

pub(super) fn create_run(paths: &EvolutionPaths, run: &SkillOptimizationRun) -> anyhow::Result<()> {
    validate_run(run)?;
    with_optimization_lock(paths, || {
        let path = run_path(paths, &run.id)?;
        if path.exists() {
            return Err(anyhow!(
                "skill optimization run `{}` already exists",
                run.id
            ));
        }
        write_run_unlocked(paths, run)
    })
}

pub(super) fn read_run(paths: &EvolutionPaths, id: &str) -> anyhow::Result<SkillOptimizationRun> {
    with_optimization_lock(paths, || {
        let mut run = read_run_unlocked(paths, id)?;
        if recover_interrupted_run(&mut run) {
            write_run_unlocked(paths, &run)?;
        }
        Ok(run)
    })
}

pub(super) fn mutate_run<R>(
    paths: &EvolutionPaths,
    id: &str,
    mutation: impl FnOnce(&mut SkillOptimizationRun) -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    with_optimization_lock(paths, || {
        let mut run = read_run_unlocked(paths, id)?;
        let result = mutation(&mut run)?;
        run.updated_at = Utc::now();
        validate_run(&run)?;
        write_run_unlocked(paths, &run)?;
        Ok(result)
    })
}

pub(super) fn list_runs(
    paths: &EvolutionPaths,
    candidate_id: Option<&str>,
) -> anyhow::Result<Vec<SkillOptimizationRun>> {
    with_optimization_lock(paths, || {
        if !paths.optimizations.is_dir() {
            return Ok(Vec::new());
        }
        let mut runs = Vec::new();
        for entry in fs::read_dir(&paths.optimizations)
            .with_context(|| format!("could not read {}", paths.optimizations.display()))?
        {
            let entry = entry
                .with_context(|| format!("could not inspect {}", paths.optimizations.display()))?;
            if !entry
                .file_type()
                .with_context(|| format!("could not inspect {}", entry.path().display()))?
                .is_file()
            {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes =
                fs::read(&path).with_context(|| format!("could not read {}", path.display()))?;
            let mut run: SkillOptimizationRun = serde_json::from_slice(&bytes)
                .with_context(|| format!("invalid skill optimization run {}", path.display()))?;
            validate_run(&run)?;
            if path.file_stem().and_then(|value| value.to_str()) != Some(run.id.as_str()) {
                return Err(anyhow!(
                    "skill optimization filename does not match run id `{}`",
                    run.id
                ));
            }
            if recover_interrupted_run(&mut run) {
                write_run_unlocked(paths, &run)?;
            }
            if candidate_id.is_none_or(|candidate_id| run.candidate_id == candidate_id) {
                runs.push(run);
            }
        }
        runs.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        runs.truncate(MAX_LISTED_RUNS);
        Ok(runs)
    })
}

pub(super) fn current_runner() -> anyhow::Result<SkillOptimizationRunner> {
    let pid = std::process::id();
    let system = process_snapshot(pid);
    let process = system
        .process(Pid::from_u32(pid))
        .ok_or_else(|| anyhow!("could not inspect the Skill optimizer host process"))?;
    Ok(SkillOptimizationRunner {
        pid,
        process_started_at: process.start_time(),
    })
}

fn recover_interrupted_run(run: &mut SkillOptimizationRun) -> bool {
    if run.status != SkillOptimizationStatus::Running {
        return false;
    }
    let Some(runner) = run.runner.as_ref() else {
        return false;
    };
    let system = process_snapshot(runner.pid);
    let alive = system
        .process(Pid::from_u32(runner.pid))
        .is_some_and(|process| process.start_time() == runner.process_started_at);
    if alive {
        return false;
    }

    let now = Utc::now();
    let note = "optimizer host exited before the run completed; start a new run to retry";
    run.status = SkillOptimizationStatus::Failed;
    run.updated_at = now;
    run.completed_at = Some(now);
    run.error = Some(note.to_string());
    run.audit.push(SkillOptimizationAuditEvent {
        status: SkillOptimizationStatus::Failed,
        at: now,
        note: note.to_string(),
    });
    true
}

fn process_snapshot(pid: u32) -> System {
    let pid = Pid::from_u32(pid);
    let pids = [pid];
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    system
}

pub(super) fn list_run_summaries(
    paths: &EvolutionPaths,
    candidate_id: Option<&str>,
) -> anyhow::Result<Vec<SkillOptimizationRunSummary>> {
    Ok(list_runs(paths, candidate_id)?
        .into_iter()
        .map(|run| run.summary())
        .collect())
}

fn with_optimization_lock<R>(
    paths: &EvolutionPaths,
    operation: impl FnOnce() -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    fs::create_dir_all(&paths.root)
        .with_context(|| format!("could not create {}", paths.root.display()))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&paths.optimization_lock)
        .with_context(|| format!("could not open {}", paths.optimization_lock.display()))?;
    FileExt::lock_exclusive(&lock)
        .with_context(|| format!("could not lock {}", paths.optimization_lock.display()))?;
    let result = operation();
    FileExt::unlock(&lock).ok();
    result
}

fn read_run_unlocked(paths: &EvolutionPaths, id: &str) -> anyhow::Result<SkillOptimizationRun> {
    let path = run_path(paths, id)?;
    let bytes = fs::read(&path)
        .with_context(|| format!("could not read skill optimization run {}", path.display()))?;
    let run: SkillOptimizationRun = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid skill optimization run {}", path.display()))?;
    validate_run(&run)?;
    Ok(run)
}

fn write_run_unlocked(paths: &EvolutionPaths, run: &SkillOptimizationRun) -> anyhow::Result<()> {
    fs::create_dir_all(&paths.optimizations)
        .with_context(|| format!("could not create {}", paths.optimizations.display()))?;
    let path = run_path(paths, &run.id)?;
    let bytes =
        serde_json::to_vec_pretty(run).context("could not serialize skill optimization run")?;
    let tmp = temporary_path(paths, &run.id);
    {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .with_context(|| format!("could not create {}", tmp.display()))?;
        file.write_all(&bytes)
            .with_context(|| format!("could not write {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("could not sync {}", tmp.display()))?;
    }
    if let Err(error) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(error).with_context(|| format!("could not replace {}", path.display()));
    }
    Ok(())
}

fn run_path(paths: &EvolutionPaths, id: &str) -> anyhow::Result<PathBuf> {
    validate_id(id)?;
    Ok(paths.optimizations.join(format!("{id}.json")))
}

fn temporary_path(paths: &EvolutionPaths, id: &str) -> PathBuf {
    paths.optimizations.join(format!(
        ".{id}.{}.{}.tmp",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ))
}

fn validate_run(run: &SkillOptimizationRun) -> anyhow::Result<()> {
    if run.schema != SKILL_OPTIMIZATION_SCHEMA {
        return Err(anyhow!(
            "unsupported skill optimization schema `{}`",
            run.schema
        ));
    }
    validate_id(&run.id)
}

fn validate_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(anyhow!("invalid skill optimization run id"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::optimization::model::{
        SkillOptimizationAuditEvent, SkillOptimizationSnapshot, SkillOptimizationStatus,
    };

    fn run(id: &str) -> SkillOptimizationRun {
        let now = Utc::now();
        SkillOptimizationRun {
            schema: SKILL_OPTIMIZATION_SCHEMA.to_string(),
            id: id.to_string(),
            candidate_id: "skill-1".to_string(),
            candidate_title: "Focused checks".to_string(),
            status: SkillOptimizationStatus::Queued,
            edit_budget: 2,
            requested_task_count: 4,
            baseline: SkillOptimizationSnapshot {
                summary: "Run focused checks.".to_string(),
                instructions: vec!["Run the smallest test first.".to_string()],
                digest: "digest".to_string(),
            },
            proposal: None,
            tasks: Vec::new(),
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
                note: "queued".to_string(),
            }],
        }
    }

    #[test]
    fn run_store_is_atomic_filterable_and_path_safe() {
        let temp = tempfile::tempdir().unwrap();
        let paths = EvolutionPaths::new(temp.path());
        create_run(&paths, &run("opt-1")).unwrap();
        mutate_run(&paths, "opt-1", |run| {
            run.status = SkillOptimizationStatus::Running;
            Ok(())
        })
        .unwrap();

        assert_eq!(
            read_run(&paths, "opt-1").unwrap().status,
            SkillOptimizationStatus::Running
        );
        assert_eq!(list_runs(&paths, Some("skill-1")).unwrap().len(), 1);
        assert!(list_runs(&paths, Some("other")).unwrap().is_empty());
        assert!(read_run(&paths, "../state").is_err());
    }

    #[test]
    fn run_listing_rejects_a_filename_identity_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let paths = EvolutionPaths::new(temp.path());
        create_run(&paths, &run("opt-1")).unwrap();
        std::fs::copy(
            paths.optimizations.join("opt-1.json"),
            paths.optimizations.join("alias.json"),
        )
        .unwrap();

        let error = list_runs(&paths, None).unwrap_err().to_string();

        assert!(error.contains("filename does not match run id"));
    }

    #[test]
    fn read_recovers_a_run_owned_by_an_exited_process() {
        let temp = tempfile::tempdir().unwrap();
        let paths = EvolutionPaths::new(temp.path());
        let mut interrupted = run("opt-interrupted");
        interrupted.status = SkillOptimizationStatus::Running;
        interrupted.runner = Some(SkillOptimizationRunner {
            pid: u32::MAX,
            process_started_at: u64::MAX,
        });
        create_run(&paths, &interrupted).unwrap();

        let recovered = read_run(&paths, "opt-interrupted").unwrap();

        assert_eq!(recovered.status, SkillOptimizationStatus::Failed);
        assert!(recovered.completed_at.is_some());
        assert!(recovered
            .error
            .as_deref()
            .is_some_and(|error| error.contains("host exited")));
    }
}
