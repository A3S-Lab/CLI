//! The `runtime` tool — A3S Runtime offload for the a3s-code TUI.
//!
//! Registered into the session **only after the user logs in to OS (OS)**
//! (see `Tui::sync_runtime_tool`), so the model never sees it while signed out.
//!
//! When the model calls it, it fans the given subtasks out to OS's
//! Function-as-a-Service batch API (`POST /api/v1/functions/<worker>/batch`) for
//! **parallel** execution on the OS pod substrate, polls the batch to completion,
//! collects each invocation's output, and returns the aggregated results — so a
//! decomposed job (e.g. deep-research sub-questions) runs concurrently on remote
//! compute instead of serially in-process. See `docs/function-compute.md` in the
//! OS repo for the wire contract.
//!
//! Mechanics (each live-validated against the deployed OS):
//! - `worker` accepts a tool-kind agent asset **UUID**, or an asset **name**
//!   which is resolved via the assets API (the OS itself only takes UUIDs —
//!   a raw name fails server-side with `RUNNER_ERROR: invalid input syntax
//!   for type uuid`). Unknown names fail fast listing the available workers.
//! - Polling streams live progress into the TUI (`ToolStreamEvent`), backs off
//!   exponentially (1.5s → 6s cap), and tolerates transient network errors.
//! - On completion, results are collected **concurrently**; on budget expiry
//!   the finished subset is still returned (`partial: true` + `batchId`).

mod http;

use a3s_code_core::tools::{Tool, ToolContext, ToolOutput, ToolStreamEvent};
use a3s_code_core::AgentEvent;
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use self::http::{endpoint_url, request_envelope, run_until_deadline, validate_runtime_id};
use crate::a3s_os::{os_origin, StoredOsSession};
use crate::system_agents::{sanitize_display_text, sanitize_multiline_text};

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Overall wait cap for a batch to finish (ceiling; callers can lower via `timeout_ms`).
const DEFAULT_BATCH_TIMEOUT_MS: u64 = 600_000;
/// Match the Core tool execution ceiling: a remote batch cannot hold a run for
/// more than 30 minutes even when the model supplies a larger value.
const MAX_BATCH_TIMEOUT_MS: u64 = 1_800_000;
const MAX_TASKS: usize = 64;
const MAX_SUBMIT_BODY_BYTES: usize = 1024 * 1024;
const MAX_CONTROL_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_INVOCATION_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_INVOCATION_RESULT_BYTES: usize = 16 * 1024;
const MAX_RESULT_STRING_CHARS: usize = 8 * 1024;
const MAX_RESULT_PREVIEW_CHARS: usize = 4 * 1024;
const MAX_WORKER_CHARS: usize = 128;
const MAX_RUNTIME_ID_CHARS: usize = 128;
const MAX_PROGRESS_CHARS: usize = 512;
const MAX_EVENT_OUTPUT_CHARS: usize = 4 * 1024;
const MAX_ERROR_CHARS: usize = 2 * 1024;
const RESULT_FETCH_CONCURRENCY: usize = 8;
/// Poll backoff: start fast (short batches return quickly), then ease off so a
/// 10-minute batch costs ~110 polls instead of 400.
const POLL_START: Duration = Duration::from_millis(1500);
const POLL_CAP: Duration = Duration::from_millis(6000);
/// Consecutive poll failures tolerated before giving up — one flaky HTTP tick
/// must not abandon an entire running batch.
const MAX_POLL_FAILURES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BatchProgress {
    done: u64,
    running: u64,
    queued: u64,
    pending: u64,
}

pub(crate) struct RuntimeTool {
    /// OS origin (`scheme://host[:port]`), derived from the login session address.
    origin: String,
    /// OS bearer token (the OAuth access token captured at login/refresh).
    token: String,
    /// Poll pacing — fields (not consts) so tests can run with tiny intervals.
    poll_start: Duration,
    poll_cap: Duration,
}

impl RuntimeTool {
    /// Build from the active OS session. Re-created on every login/refresh so the
    /// captured token stays current (the TUI rebuilds the session on auth change).
    pub(crate) fn new(session: &StoredOsSession) -> Self {
        Self {
            origin: os_origin(&session.address),
            token: session.access_token.clone(),
            poll_start: POLL_START,
            poll_cap: POLL_CAP,
        }
    }

    fn client(&self) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder().timeout(HTTP_TIMEOUT);
        if is_loopback_origin(&self.origin) {
            builder = builder.no_proxy();
        }
        Ok(builder.build()?)
    }

    /// Unwrap the shared OS response envelope `{code,status,message,data,...}` and
    /// return `data` (the real payload). Errors carry the wire `message` so the
    /// model sees why a call failed.
    fn unwrap_envelope(body: &str, status: u16) -> Result<Value> {
        let v: Value = serde_json::from_str(body).map_err(|e| {
            anyhow::anyhow!(
                "Non-JSON response (HTTP {status}): {e}: {}",
                truncate(body, 200)
            )
        })?;
        let code = v
            .get("code")
            .and_then(Value::as_u64)
            .unwrap_or(status as u64);
        if code >= 400 || status >= 400 {
            let msg = v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            anyhow::bail!("A3S Runtime returned {code}: {msg}");
        }
        Ok(v.get("data").cloned().unwrap_or(v))
    }
}

fn is_loopback_origin(origin: &str) -> bool {
    origin.starts_with("http://127.")
        || origin.starts_with("https://127.")
        || origin.starts_with("http://localhost")
        || origin.starts_with("https://localhost")
        || origin.starts_with("http://[::1]")
        || origin.starts_with("https://[::1]")
}

#[async_trait]
impl Tool for RuntimeTool {
    fn name(&self) -> &str {
        "runtime"
    }

    fn description(&self) -> &str {
        "Offload independent subtasks to OS A3S Runtime for parallel remote \
         execution, stream progress while they run, then return a combined result. \
         Use it for decomposable work such as multiple deep-research subquestions. \
         `worker` is a tool-kind agent asset UUID or name. Names auto-resolve; \
         invalid names list the available workers."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "Independent subtasks to run in parallel. Each item is passed to the worker as input, usually a subquestion string or an object matching the worker inputSchema.",
                    "items": { "type": ["string", "object"] },
                    "minItems": 1,
                    "maxItems": MAX_TASKS
                },
                "worker": {
                    "type": "string",
                    "description": "Worker that runs each subtask: a tool-kind agent asset UUID or name. Required.",
                    "minLength": 1,
                    "maxLength": MAX_WORKER_CHARS
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Maximum wait for the full batch in milliseconds. Defaults to 10 minutes; on timeout, returns completed results.",
                    "minimum": 1,
                    "maximum": MAX_BATCH_TIMEOUT_MS
                }
            },
            "required": ["tasks", "worker"]
        })
    }

    fn requires_confirmation(&self, _args: &Value) -> bool {
        // This tool submits work to a remote runtime and can incur external
        // side effects or cost. Default mode may authorize that exact call;
        // non-interactive Auto mode converts the escalation into a denial.
        true
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let tasks: Vec<Value> = match args.get("tasks").and_then(Value::as_array) {
            Some(a)
                if !a.is_empty()
                    && a.len() <= MAX_TASKS
                    && a.iter().all(|task| task.is_string() || task.is_object()) =>
            {
                a.clone()
            }
            Some(a) if a.len() > MAX_TASKS => {
                return Ok(ToolOutput::error(format!(
                    "`tasks` accepts at most {MAX_TASKS} items"
                )))
            }
            Some(a) if !a.is_empty() => {
                return Ok(ToolOutput::error(
                    "every `tasks` item must be a string or object",
                ))
            }
            _ => return Ok(ToolOutput::error("`tasks` must be a non-empty array")),
        };
        let encoded_tasks = match serde_json::to_vec(&tasks) {
            Ok(encoded) if encoded.len() <= MAX_SUBMIT_BODY_BYTES => encoded,
            Ok(encoded) => {
                return Ok(ToolOutput::error(format!(
                    "serialized `tasks` exceeds the {MAX_SUBMIT_BODY_BYTES}-byte limit ({} bytes)",
                    encoded.len()
                )))
            }
            Err(error) => {
                return Ok(ToolOutput::error(format!(
                    "could not serialize `tasks`: {error}"
                )))
            }
        };
        drop(encoded_tasks);
        let worker = match args.get("worker").and_then(Value::as_str) {
            Some(w) if !w.trim().is_empty() => {
                let worker = w.trim();
                let sanitized = sanitize_display_text(worker, MAX_WORKER_CHARS + 1);
                if worker.chars().count() > MAX_WORKER_CHARS || sanitized != worker {
                    return Ok(ToolOutput::error(format!(
                        "`worker` must be terminal-safe text no longer than {MAX_WORKER_CHARS} characters"
                    )));
                }
                worker.to_string()
            }
            _ => {
                return Ok(ToolOutput::error(
                    "`worker` is required: use a tool-kind agent asset UUID or name",
                ))
            }
        };
        let budget_ms = match args.get("timeout_ms") {
            None => DEFAULT_BATCH_TIMEOUT_MS,
            Some(value) => match value.as_u64() {
                Some(value) if (1..=MAX_BATCH_TIMEOUT_MS).contains(&value) => value,
                _ => {
                    return Ok(ToolOutput::error(format!(
                        "`timeout_ms` must be an integer from 1 through {MAX_BATCH_TIMEOUT_MS}"
                    )))
                }
            },
        };
        if ctx.is_cancelled() {
            return Ok(ToolOutput::error("A3S Runtime offload cancelled"));
        }

        match self.run_batch(&worker, tasks, budget_ms, ctx).await {
            Ok(out) => Ok(ToolOutput::success(out)),
            // A3S Runtime / network failure is a tool failure, not a crash — surface it.
            Err(e) => {
                let error = sanitize_multiline_text(&e.to_string(), MAX_ERROR_CHARS);
                Ok(ToolOutput::error(format!(
                    "A3S Runtime offload failed: {}",
                    if error.is_empty() {
                        "unknown error"
                    } else {
                        &error
                    }
                )))
            }
        }
    }
}

impl RuntimeTool {
    async fn run_batch(
        &self,
        worker: &str,
        tasks: Vec<Value>,
        budget_ms: u64,
        ctx: &ToolContext,
    ) -> Result<String> {
        let client = self.client()?;
        let n = tasks.len();
        let cancellation = ctx.cancellation_token();
        let progress = |msg: String| {
            if let Some(tx) = &ctx.event_tx {
                // Best-effort: progress must never block or fail the batch.
                let msg = sanitize_multiline_text(&msg, MAX_PROGRESS_CHARS);
                if !msg.is_empty() {
                    let _ = tx.try_send(ToolStreamEvent::OutputDelta(msg));
                }
            }
        };

        // 0. Resolve a worker NAME to its asset UUID (the OS API only accepts
        //    UUIDs). UUIDs pass straight through.
        let worker_id = if looks_like_uuid(worker) {
            worker.to_ascii_lowercase()
        } else {
            let id = self
                .resolve_worker_name(&client, worker, &cancellation)
                .await?;
            progress(format!("worker {worker} -> {id}\n"));
            id
        };

        // 1. Fan out. idempotencyKey (hash of worker + task set) makes a retry
        //    re-attach to the same batch instead of double-spending.
        let idem = idempotency_key(&worker_id, &tasks);
        let submit_url = endpoint_url(
            &self.origin,
            &["api", "v1", "functions", &worker_id, "batch"],
        )?;
        let submit_payload =
            json!({ "inputs": tasks, "agentKind": "tool", "idempotencyKey": idem });
        let submit_bytes = serde_json::to_vec(&submit_payload)?;
        if submit_bytes.len() > MAX_SUBMIT_BODY_BYTES {
            anyhow::bail!(
                "runtime request exceeds the {MAX_SUBMIT_BODY_BYTES}-byte limit ({} bytes)",
                submit_bytes.len()
            );
        }
        let submit = request_envelope(
            client
                .post(submit_url)
                .bearer_auth(&self.token)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(submit_bytes),
            MAX_CONTROL_RESPONSE_BYTES,
            &cancellation,
        );
        let data = submit.await?;
        let batch_id = validate_runtime_id(
            "batchId",
            data.get("batchId")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("batch response did not include batchId"))?,
        )?;
        let raw_invocation_ids = data
            .get("invocationIds")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("batch response did not include invocationIds"))?;
        if raw_invocation_ids.len() != n {
            anyhow::bail!(
                "batch response returned {} invocation IDs for {n} submitted tasks",
                raw_invocation_ids.len()
            );
        }
        let mut invocation_ids = Vec::with_capacity(raw_invocation_ids.len());
        let mut unique_invocation_ids = HashSet::with_capacity(raw_invocation_ids.len());
        for value in raw_invocation_ids {
            let id = validate_runtime_id(
                "invocationId",
                value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("invocationId must be a string"))?,
            )?;
            if !unique_invocation_ids.insert(id.clone()) {
                anyhow::bail!("batch response contained duplicate invocationId `{id}`");
            }
            invocation_ids.push(id);
        }
        progress(format!(
            "{n} parallel subtasks submitted (batch {batch_id})\n"
        ));
        emit_runtime_subagent_starts(ctx, &batch_id, &invocation_ids, &tasks);

        // 2. Poll until every member is terminal or the budget expires — with
        //    exponential backoff, live progress, and transient-failure tolerance.
        let poll_url = endpoint_url(
            &self.origin,
            &["api", "v1", "functions", "batches", &batch_id],
        )?;
        let deadline = Instant::now() + Duration::from_millis(budget_ms);
        let mut interval = self.poll_start;
        let mut consecutive_failures = 0u32;
        let mut last_report = String::new();
        let mut timed_out_pending = 0u64;
        let mut last_pending = n as u64;
        loop {
            let poll = run_until_deadline(
                async {
                    Ok(request_envelope(
                        client.get(poll_url.clone()).bearer_auth(&self.token),
                        MAX_CONTROL_RESPONSE_BYTES,
                        &cancellation,
                    )
                    .await)
                },
                deadline,
                &cancellation,
            )
            .await?;
            let Some(poll) = poll else {
                timed_out_pending = last_pending;
                break;
            };
            match poll {
                Ok(bd) => {
                    consecutive_failures = 0;
                    let batch_progress = batch_progress(&bd, n);
                    let (done, running, queued, pending) = (
                        batch_progress.done,
                        batch_progress.running,
                        batch_progress.queued,
                        batch_progress.pending,
                    );
                    // Emit progress only when the picture changes (no spam).
                    let report =
                        format!("⏳ {done}/{n} done · {running} running · {queued} queued\n");
                    if report != last_report {
                        progress(report.clone());
                        last_report = report;
                    }
                    if pending == 0 {
                        break;
                    }
                    last_pending = pending;
                }
                Err(e) => {
                    // Tolerate flaky ticks: one failed GET must not abandon a
                    // whole running batch.
                    consecutive_failures += 1;
                    if consecutive_failures > MAX_POLL_FAILURES {
                        return Err(e.context(format!(
                            "polling batch {batch_id} failed {consecutive_failures} consecutive times"
                        )));
                    }
                    progress(format!(
                        "poll failed (attempt {consecutive_failures}; retrying)\n"
                    ));
                }
            }
            if run_until_deadline(
                async {
                    tokio::time::sleep(interval).await;
                    Ok(())
                },
                deadline,
                &cancellation,
            )
            .await?
            .is_none()
            {
                timed_out_pending = last_pending;
                break;
            }
            interval = (interval * 3 / 2).min(self.poll_cap);
        }

        // 3. Collect every member's result CONCURRENTLY (one RTT, not N).
        //    On timeout this still runs: finished members' outputs are returned
        //    (partial) instead of being thrown away.
        let fetches = futures::stream::iter(invocation_ids.iter().cloned().map(|id| {
            let url = endpoint_url(
                &self.origin,
                &["api", "v1", "functions", "invocations", &id],
            );
            let client = client.clone();
            let token = self.token.clone();
            let cancellation = cancellation.clone();
            async move {
                let inv = match url {
                    Ok(url) => {
                        request_envelope(
                            client.get(url).bearer_auth(&token),
                            MAX_INVOCATION_RESPONSE_BYTES,
                            &cancellation,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
                .unwrap_or_else(|e| json!({ "error": e.to_string() }));
                let result = inv.get("result").cloned().unwrap_or(inv);
                let state = result
                    .get("status")
                    .and_then(Value::as_str)
                    .map(|state| sanitize_display_text(state, 32))
                    .filter(|state| !state.is_empty())
                    .unwrap_or_else(|| "unknown".to_string());
                let output = result.get("output").cloned().unwrap_or(Value::Null);
                let error = result.get("error").cloned().unwrap_or(Value::Null);
                json!({
                    "invocationId": id,
                    "state": state,
                    "output": bounded_result_value(output, MAX_INVOCATION_RESULT_BYTES),
                    "error": bounded_result_value(error, MAX_INVOCATION_RESULT_BYTES),
                })
            }
        }))
        .buffered(RESULT_FETCH_CONCURRENCY);
        let results: Vec<Value> = fetches.collect().await;
        if cancellation.is_cancelled() {
            anyhow::bail!("runtime request cancelled");
        }
        emit_runtime_subagent_ends(ctx, &batch_id, &invocation_ids, &results);

        let mut summary = json!({
            "batchId": batch_id,
            "worker": worker_id,
            "count": n,
            "results": results,
        });
        if timed_out_pending > 0 {
            summary["partial"] = json!(true);
            summary["note"] = json!(format!(
                "Timed out after {budget_ms}ms with {timed_out_pending} subtasks still pending; \
                 completed results were returned. Query batchId={batch_id} later for unfinished items."
            ));
        }
        Ok(serde_json::to_string_pretty(&summary)?)
    }

    /// Resolve a worker asset NAME to its UUID via the assets API. Fails with
    /// the list of available tool-kind workers so the model can self-correct.
    async fn resolve_worker_name(
        &self,
        client: &reqwest::Client,
        name: &str,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        let mut url = endpoint_url(&self.origin, &["api", "v1", "assets"])?;
        url.query_pairs_mut()
            .append_pair("category", "agent")
            .append_pair("limit", "100");
        let data = request_envelope(
            client.get(url).bearer_auth(&self.token),
            MAX_CONTROL_RESPONSE_BYTES,
            cancellation,
        )
        .await?;
        let items = data
            .get("items")
            .or_else(|| data.get("list"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let tools: Vec<(&str, &str)> = items
            .iter()
            .take(100)
            .filter(|a| a.get("agentKind").and_then(Value::as_str) == Some("tool"))
            .filter_map(|a| {
                Some((
                    a.get("id").and_then(Value::as_str)?,
                    a.get("name").and_then(Value::as_str)?,
                ))
            })
            .collect();
        if let Some((id, _)) = tools
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(name.trim()))
        {
            if !looks_like_uuid(id) {
                anyhow::bail!(
                    "tool-kind worker `{}` resolved to a non-canonical UUID",
                    sanitize_display_text(name, MAX_WORKER_CHARS)
                );
            }
            return Ok(id.to_ascii_lowercase());
        }
        let available: Vec<String> = tools
            .iter()
            .take(10)
            .filter(|(id, _)| looks_like_uuid(id))
            .map(|(id, n)| {
                let name = sanitize_display_text(n, MAX_WORKER_CHARS);
                format!("{name} ({})", id.to_ascii_lowercase())
            })
            .collect();
        let name = sanitize_display_text(name, MAX_WORKER_CHARS);
        anyhow::bail!(
            "No tool-kind worker named \"{name}\". Available workers: {}",
            if available.is_empty() {
                "none; create a tool-kind agent asset in the OS Asset Center first".to_string()
            } else {
                available.join(", ")
            }
        )
    }
}

fn bounded_result_value(value: Value, max_bytes: usize) -> Value {
    let original_bytes = serde_json::to_vec(&value).map_or(0, |encoded| encoded.len());
    let value = sanitize_result_value(value);
    if serde_json::to_vec(&value).is_ok_and(|encoded| encoded.len() <= max_bytes) {
        return value;
    }
    let preview_chars = MAX_RESULT_PREVIEW_CHARS.min(max_bytes.saturating_sub(256) / 4);
    let preview = sanitize_multiline_text(&value_to_compact_string(&value), preview_chars);
    json!({
        "truncated": true,
        "originalBytes": original_bytes,
        "preview": preview,
    })
}

fn sanitize_result_value(value: Value) -> Value {
    match value {
        Value::String(text) => {
            Value::String(sanitize_multiline_text(&text, MAX_RESULT_STRING_CHARS))
        }
        Value::Array(values) => {
            Value::Array(values.into_iter().map(sanitize_result_value).collect())
        }
        Value::Object(values) => {
            let mut sanitized = serde_json::Map::with_capacity(values.len());
            for (key, value) in values {
                let key = sanitize_display_text(&key, MAX_RUNTIME_ID_CHARS);
                if !key.is_empty() {
                    sanitized.insert(key, sanitize_result_value(value));
                }
            }
            Value::Object(sanitized)
        }
        value => value,
    }
}

fn emit_runtime_subagent_starts(
    ctx: &ToolContext,
    batch_id: &str,
    invocation_ids: &[String],
    tasks: &[Value],
) {
    let Some(tx) = &ctx.agent_event_tx else {
        return;
    };
    let started_ms = epoch_ms();
    for (idx, invocation_id) in invocation_ids.iter().enumerate() {
        let _ = tx.send(AgentEvent::SubagentStart {
            task_id: runtime_subagent_task_id(invocation_id),
            session_id: format!("runtime-{batch_id}-{idx}"),
            parent_session_id: ctx.session_id.clone().unwrap_or_default(),
            agent: "runtime".to_string(),
            description: runtime_task_description(idx, tasks.get(idx)),
            started_ms,
        });
    }
}

fn emit_runtime_subagent_ends(
    ctx: &ToolContext,
    batch_id: &str,
    invocation_ids: &[String],
    results: &[Value],
) {
    let Some(tx) = &ctx.agent_event_tx else {
        return;
    };
    let finished_ms = epoch_ms();
    for (idx, invocation_id) in invocation_ids.iter().enumerate() {
        let result = results.get(idx).cloned().unwrap_or_else(|| json!({}));
        let state = result
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let success = matches!(state, "succeeded" | "completed" | "success");
        let output = result
            .get("output")
            .filter(|value| !value.is_null())
            .or_else(|| result.get("error").filter(|value| !value.is_null()))
            .cloned()
            .unwrap_or(result);
        let output =
            sanitize_multiline_text(&value_to_compact_string(&output), MAX_EVENT_OUTPUT_CHARS);
        let _ = tx.send(AgentEvent::SubagentEnd {
            task_id: runtime_subagent_task_id(invocation_id),
            session_id: format!("runtime-{batch_id}-{idx}"),
            agent: "runtime".to_string(),
            output,
            success,
            finished_ms,
        });
    }
}

fn batch_progress(batch: &Value, expected_count: usize) -> BatchProgress {
    let expected = expected_count as u64;
    if let Some(counts) = batch.get("counts").and_then(Value::as_object) {
        let count = |keys: &[&str]| {
            keys.iter()
                .filter_map(|key| counts.get(*key).and_then(Value::as_u64))
                .sum::<u64>()
        };
        let queued = count(&["queued", "pending", "created", "scheduled"]);
        let running = count(&["running", "in_progress", "processing", "active"]);
        let done = count(&[
            "succeeded",
            "success",
            "completed",
            "done",
            "failed",
            "errored",
            "error",
            "canceled",
            "cancelled",
            "unknown",
        ]);
        let observed = done + queued + running;
        let missing = expected.saturating_sub(observed);
        let queued = queued + missing;
        let pending = queued + running;
        return BatchProgress {
            done,
            running,
            queued,
            pending,
        };
    }

    if let Some(items) = batch_member_items(batch) {
        let mut done = 0u64;
        let mut running = 0u64;
        let mut queued = 0u64;
        let mut unknown = 0u64;
        for item in items {
            match batch_item_state(item).as_deref() {
                Some(state) if is_terminal_runtime_state(state) => done += 1,
                Some(state) if is_queued_runtime_state(state) => queued += 1,
                Some(state) if is_running_runtime_state(state) => running += 1,
                _ => unknown += 1,
            }
        }
        let observed = done + running + queued + unknown;
        let missing = expected.saturating_sub(observed);
        queued += unknown + missing;
        return BatchProgress {
            done,
            running,
            queued,
            pending: queued + running,
        };
    }

    if let Some(state) = batch_item_state(batch) {
        if is_terminal_runtime_state(&state) {
            return BatchProgress {
                done: expected,
                running: 0,
                queued: 0,
                pending: 0,
            };
        }
        if is_running_runtime_state(&state) {
            return BatchProgress {
                done: 0,
                running: expected,
                queued: 0,
                pending: expected,
            };
        }
        if is_queued_runtime_state(&state) {
            return BatchProgress {
                done: 0,
                running: 0,
                queued: expected,
                pending: expected,
            };
        }
    }

    BatchProgress {
        done: 0,
        running: 0,
        queued: expected,
        pending: expected,
    }
}

fn batch_member_items(batch: &Value) -> Option<&Vec<Value>> {
    for key in ["invocations", "items", "results", "tasks", "members"] {
        if let Some(items) = batch.get(key).and_then(Value::as_array) {
            return Some(items);
        }
    }
    None
}

fn batch_item_state(value: &Value) -> Option<String> {
    value
        .get("status")
        .or_else(|| value.get("state"))
        .or_else(|| value.pointer("/result/status"))
        .or_else(|| value.pointer("/result/state"))
        .or_else(|| value.pointer("/execution/status"))
        .or_else(|| value.pointer("/execution/state"))
        .and_then(Value::as_str)
        .map(|state| state.trim().to_ascii_lowercase())
}

fn is_terminal_runtime_state(state: &str) -> bool {
    matches!(
        state,
        "succeeded"
            | "success"
            | "completed"
            | "complete"
            | "done"
            | "failed"
            | "failure"
            | "errored"
            | "error"
            | "canceled"
            | "cancelled"
            | "unknown"
    )
}

fn is_running_runtime_state(state: &str) -> bool {
    matches!(
        state,
        "running" | "in_progress" | "processing" | "active" | "started" | "executing"
    )
}

fn is_queued_runtime_state(state: &str) -> bool {
    matches!(
        state,
        "queued" | "pending" | "created" | "scheduled" | "submitted" | "waiting"
    )
}

fn runtime_subagent_task_id(invocation_id: &str) -> String {
    format!("runtime-{invocation_id}")
}

fn runtime_task_description(idx: usize, task: Option<&Value>) -> String {
    let Some(task) = task else {
        return format!("Runtime task {}", idx + 1);
    };
    let description = task
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| task.get("focus").and_then(Value::as_str))
        .or_else(|| task.get("query").and_then(Value::as_str))
        .or_else(|| task.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| value_to_compact_string(task));
    let description = sanitize_display_text(&description, 80);
    if description.is_empty() {
        format!("Runtime task {}", idx + 1)
    } else {
        description
    }
}

fn value_to_compact_string(value: &Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()))
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// A canonical hyphenated UUID (the only `ref` form the OS batch API accepts).
fn looks_like_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Deterministic idempotency key from the worker + task set (sha256, truncated) —
/// a retry of the same fan-out re-attaches to the existing batch instead of
/// double-spending, while the same tasks on a DIFFERENT worker stay distinct.
fn idempotency_key(worker: &str, tasks: &[Value]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(worker.as_bytes());
    h.update([0u8]);
    h.update(serde_json::to_vec(tasks).unwrap_or_default());
    let hex: String = h.finalize().iter().map(|b| format!("{:02x}", b)).collect();
    format!("a3s-code-runtime-{}", &hex[..24])
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        // Char-based to avoid panicking on a multibyte boundary.
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests;
