use std::path::Path;

use anyhow::Context;
use serde_json::json;

use crate::cli::args::{CodeScheduleArgs, CodeScheduleCommand, OutputMode};
use crate::cli::context::InvocationContext;
use crate::cli::output::render_value;
use crate::code_schedule::{
    acknowledge_schedule_notifications, disable_loop_schedule, enable_loop_schedule,
    list_loop_schedules, list_pending_schedule_notifications, queue_loop_schedule_run,
    read_schedule_worker_status, request_schedule_worker_stop, run_schedule_worker,
    start_schedule_worker, LoopScheduleState, WorkerStartOutcome,
};

pub(super) async fn run(args: CodeScheduleArgs, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    match args.command {
        CodeScheduleCommand::List => render_schedule_list(output, &context.directory),
        CodeScheduleCommand::Enable(args) => {
            let state = enable_loop_schedule(
                &context.directory,
                &args.loop_id,
                args.every.as_deref(),
                args.model,
            )?;
            let worker = start_worker(context).await?;
            render_schedule_change(output, "code.schedule.enable", state, worker, || {
                println!("enabled loop schedule `{}`", args.loop_id);
            })
        }
        CodeScheduleCommand::Disable(args) => {
            let state = disable_loop_schedule(&context.directory, &args.loop_id)?;
            render_value(
                output,
                "code.schedule.disable",
                json!({"schedule": state}),
                || println!("disabled loop schedule `{}`", args.loop_id),
            )
        }
        CodeScheduleCommand::Run(args) => {
            let state = queue_loop_schedule_run(&context.directory, &args.loop_id)?;
            let worker = start_worker(context).await?;
            render_schedule_change(output, "code.schedule.run", state, worker, || {
                println!("queued scheduled loop `{}`", args.loop_id);
            })
        }
        CodeScheduleCommand::Start => {
            let worker = start_worker(context).await?;
            render_value(
                output,
                "code.schedule.start",
                json!({"worker": worker_json(worker)}),
                || match worker {
                    WorkerStartOutcome::AlreadyRunning => {
                        println!("schedule worker is already running")
                    }
                    WorkerStartOutcome::Started { pid } => {
                        println!("schedule worker started (pid {pid})")
                    }
                },
            )
        }
        CodeScheduleCommand::Stop => {
            let requested = request_schedule_worker_stop(&context.directory)?;
            render_value(
                output,
                "code.schedule.stop",
                json!({"stopRequested": requested}),
                || {
                    if requested {
                        println!("schedule worker will stop after its current run")
                    } else {
                        println!("schedule worker is not running")
                    }
                },
            )
        }
        CodeScheduleCommand::Status => render_schedule_status(output, &context.directory),
        CodeScheduleCommand::Notifications => {
            let notifications = list_pending_schedule_notifications(&context.directory, 64)?;
            let human = notifications.clone();
            render_value(
                output,
                "code.schedule.notifications",
                json!({"notifications": notifications, "count": notifications.len()}),
                move || {
                    if human.is_empty() {
                        println!("no pending schedule notifications");
                    }
                    for notification in human {
                        println!(
                            "{}: {} ({})\n  {}\n  {}",
                            notification.loop_id,
                            notification.summary,
                            notification.outcome.label(),
                            notification.run_id,
                            notification.result_path.display()
                        );
                    }
                },
            )?;
            acknowledge_schedule_notifications(&context.directory, &notifications)
        }
        CodeScheduleCommand::Worker => {
            run_schedule_worker(&context.directory, context.explicit_config.as_deref()).await
        }
    }
}

async fn start_worker(context: &InvocationContext) -> anyhow::Result<WorkerStartOutcome> {
    let workspace = context.directory.clone();
    let config_path = crate::commands::config::active_config_path(context).ok();
    tokio::task::spawn_blocking(move || start_schedule_worker(&workspace, config_path.as_deref()))
        .await
        .context("schedule worker launcher failed")?
}

fn render_schedule_list(output: OutputMode, workspace: &Path) -> anyhow::Result<()> {
    let schedules = list_loop_schedules(workspace)?;
    let human = schedules.clone();
    render_value(
        output,
        "code.schedule.list",
        json!({"schedules": schedules, "count": schedules.len()}),
        move || render_human_schedules(&human),
    )
}

fn render_schedule_status(output: OutputMode, workspace: &Path) -> anyhow::Result<()> {
    let schedules = list_loop_schedules(workspace)?;
    let worker = read_schedule_worker_status(workspace)?;
    let enabled = schedules.iter().filter(|state| state.enabled).count();
    let human_worker = worker.clone();
    render_value(
        output,
        "code.schedule.status",
        json!({
            "worker": worker,
            "running": worker.is_some(),
            "scheduleCount": schedules.len(),
            "enabledCount": enabled,
        }),
        move || match human_worker {
            Some(worker) => println!(
                "schedule worker running (pid {}) · {enabled} enabled / {} total",
                worker.pid,
                schedules.len()
            ),
            None => println!(
                "schedule worker stopped · {enabled} enabled / {} total",
                schedules.len()
            ),
        },
    )
}

fn render_schedule_change(
    output: OutputMode,
    command: &'static str,
    state: LoopScheduleState,
    worker: WorkerStartOutcome,
    human: impl FnOnce(),
) -> anyhow::Result<()> {
    render_value(
        output,
        command,
        json!({"schedule": state, "worker": worker_json(worker)}),
        human,
    )
}

fn worker_json(worker: WorkerStartOutcome) -> serde_json::Value {
    match worker {
        WorkerStartOutcome::AlreadyRunning => json!({"running": true, "started": false}),
        WorkerStartOutcome::Started { pid } => {
            json!({"running": true, "started": true, "pid": pid})
        }
    }
}

fn render_human_schedules(schedules: &[LoopScheduleState]) {
    if schedules.is_empty() {
        println!("no loop schedules; create a loop, then run `a3s code schedule enable <id>`");
        return;
    }
    for schedule in schedules {
        let next = schedule
            .next_run_at_ms
            .map(format_epoch_ms)
            .unwrap_or_else(|| "disabled".to_string());
        let last = schedule
            .last_run
            .as_ref()
            .map(|run| {
                format!(
                    "{} ({})",
                    run.outcome.label(),
                    format_epoch_ms(run.finished_at_ms)
                )
            })
            .unwrap_or_else(|| "never".to_string());
        println!(
            "{} · {} · every {}s · next {} · last {}",
            schedule.loop_id,
            if schedule.enabled {
                "enabled"
            } else {
                "disabled"
            },
            schedule.cadence_seconds,
            next,
            last
        );
    }
}

fn format_epoch_ms(value: u64) -> String {
    let seconds = i64::try_from(value / 1_000).unwrap_or(i64::MAX);
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| value.to_string())
}
