pub(super) async fn call_generation_with_progress(
    session: &AgentSession,
    generation_args: Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    _run_clock: &EvidenceFirstRunClock,
    stage_label: &str,
    execution_timeout_ms: u64,
    max_attempts: u8,
) -> Result<ToolCallResult, String> {
    if !(1..=2).contains(&max_attempts) {
        return Err(format!(
            "structured {stage_label} generation requires one or two attempts"
        ));
    }
    // Host-direct generate_object: avoid nested dynamic_workflow/PTC for a
    // single structured call (opaque `program script error:` collapses the
    // planner/report path). Retries stay Host-owned under the stage budget.
    let attempt_timeout_ms = generation_args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(execution_timeout_ms);
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(execution_timeout_ms.max(1));
    let mut last_error = None;
    for attempt in 1..=max_attempts {
        let remaining_ms = deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .as_millis();
        if remaining_ms == 0 {
            return Err(format!(
                "{stage_label} generation timed out after {execution_timeout_ms} ms"
            ));
        }
        let budget_ms = attempt_timeout_ms
            .max(1)
            .min(u64::try_from(remaining_ms).unwrap_or(u64::MAX));
        let mut arguments = generation_args.clone();
        if let Some(object) = arguments.as_object_mut() {
            object.insert("timeout_ms".to_string(), Value::from(budget_ms));
        }
        match tokio::time::timeout(
            std::time::Duration::from_millis(budget_ms),
            call_tool_with_progress(
                session,
                "generate_object",
                arguments,
                progress_tx,
                false,
            ),
        )
        .await
        {
            Ok(Ok(result)) if result.exit_code == 0 => return Ok(result),
            Ok(Ok(result)) => {
                last_error = Some(
                    result
                        .output
                        .lines()
                        .next()
                        .unwrap_or("structured generation failed")
                        .to_string(),
                );
            }
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => {
                last_error = Some(format!(
                    "{stage_label} generation attempt {attempt} timed out after {budget_ms} ms"
                ));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        format!("{stage_label} generation failed after {max_attempts} attempts")
    }))
}

pub(super) async fn run_dynamic_workflow(
    session: &AgentSession,
    args: Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
) -> Result<ToolCallResult, String> {
    let result =
        call_tool_with_progress(session, "dynamic_workflow", args, progress_tx, true).await?;
    if result.exit_code != 0 {
        return Err(result
            .output
            .lines()
            .next()
            .unwrap_or("dynamic_workflow failed without an error message")
            .to_string());
    }
    Ok(result)
}

pub(super) async fn run_bootstrap_acquisition_stage(
    session: &AgentSession,
    args: Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    timeout_ms: u64,
) -> Result<ToolCallResult, String> {
    let recovery_args = args.clone();
    let result = within_inquiry_stage_timeout_typed(
        run_dynamic_workflow(session, args, progress_tx),
        timeout_ms,
        "bootstrap acquisition",
    )
    .await;
    match result {
        Ok(result) => Ok(result),
        Err(error @ InquiryStageError::TimedOut { .. }) => {
            recover_bootstrap_acquisition_after_timeout(session, &recovery_args)
                .ok_or_else(|| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn recover_bootstrap_acquisition_after_timeout(
    session: &AgentSession,
    args: &Value,
) -> Option<ToolCallResult> {
    let recovered =
        recover_deep_research_bootstrap_acquisition_from_store(session.workspace(), args)?;
    let output = recovered.output?;
    let expected_query = args.pointer("/input/query").and_then(Value::as_str)?;
    bootstrap_acquisition_value(&output, expected_query)?;
    Some(ToolCallResult {
        name: "dynamic_workflow".to_string(),
        output,
        exit_code: 0,
        metadata: Some(recovered.metadata),
        error_kind: None,
    })
}

fn bootstrap_acquisition_value(output: &str, expected_query: &str) -> Option<Value> {
    let value = serde_json::from_str::<Value>(output).ok()?;
    if value.get("query").and_then(Value::as_str) != Some(expected_query)
        || value.get("mode").and_then(Value::as_str) != Some("bootstrap_acquisition")
        || value
            .pointer("/execution/terminal_authority")
            .and_then(Value::as_str)
            != Some("host_inquiry_reducer")
    {
        return None;
    }
    let acquisition = value.get("acquisition")?.clone();
    let sources = acquisition.pointer("/packet/sources")?.as_array()?;
    if sources.is_empty() || sources.len() > 16 {
        return None;
    }
    let valid = sources.iter().all(|source| {
        source
            .get("source_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty())
            && source
                .get("url_or_path")
                .and_then(Value::as_str)
                .is_some_and(|anchor| !anchor.trim().is_empty())
            && source
                .get("chunks")
                .and_then(Value::as_array)
                .is_some_and(|chunks| {
                    !chunks.is_empty()
                        && chunks.iter().all(|chunk| {
                            chunk
                                .get("chunk_id")
                                .and_then(Value::as_str)
                                .is_some_and(|id| !id.trim().is_empty())
                                && chunk
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .is_some_and(|text| !text.trim().is_empty())
                        })
                })
    });
    valid.then_some(acquisition)
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum InquiryStageError {
    Operation(String),
    TimedOut { stage: String, timeout_ms: u64 },
}

impl std::fmt::Display for InquiryStageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Operation(error) => formatter.write_str(error),
            Self::TimedOut { stage, timeout_ms } => write!(
                formatter,
                "DeepResearch {stage} stage timed out after {timeout_ms} ms"
            ),
        }
    }
}

pub(super) async fn within_inquiry_stage_timeout_typed<T, F>(
    future: F,
    timeout_ms: u64,
    stage: &str,
) -> Result<T, InquiryStageError>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), future).await {
        Ok(result) => result.map_err(InquiryStageError::Operation),
        Err(_) => Err(InquiryStageError::TimedOut {
            stage: stage.to_string(),
            timeout_ms,
        }),
    }
}

pub(super) async fn call_tool_with_progress(
    session: &AgentSession,
    name: &str,
    args: Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    filter_dynamic_workflow_envelope: bool,
) -> Result<ToolCallResult, String> {
    let (progress_rx, join) = session.tool_with_events(name, args);
    forward_tool_call_with_progress(
        name,
        progress_rx,
        join,
        progress_tx,
        filter_dynamic_workflow_envelope,
    )
    .await
}

pub(super) async fn forward_tool_call_with_progress(
    name: &str,
    mut progress_rx: mpsc::Receiver<AgentEvent>,
    mut join: tokio::task::JoinHandle<a3s_code_core::Result<ToolCallResult>>,
    progress_tx: &mpsc::Sender<AgentEvent>,
    filter_dynamic_workflow_envelope: bool,
) -> Result<ToolCallResult, String> {
    let abort = join.abort_handle();
    let mut abort_on_drop = AbortInnerToolOnDrop(Some(abort.clone()));
    let mut progress_open = true;
    let result = loop {
        if !progress_open {
            let result = join
                .await
                .map_err(|error| format!("{name} task failed: {error}"))?
                .map_err(|error| format!("{name} failed: {error}"));
            abort_on_drop.disarm();
            break result;
        }
        tokio::select! {
            biased;
            event = progress_rx.recv() => {
                let Some(event) = event else {
                    progress_open = false;
                    continue;
                };
                if filter_dynamic_workflow_envelope && is_dynamic_workflow_envelope(&event) {
                    continue;
                }
                if progress_tx.send(event).await.is_err() {
                    abort.abort();
                    return Err("DeepResearch progress consumer closed".to_string());
                }
            }
            result = &mut join => {
                let result = result
                    .map_err(|error| format!("{name} task failed: {error}"))?
                    .map_err(|error| format!("{name} failed: {error}"));
                abort_on_drop.disarm();
                break result;
            }
        }
    };
    while let Ok(event) = progress_rx.try_recv() {
        if filter_dynamic_workflow_envelope && is_dynamic_workflow_envelope(&event) {
            continue;
        }
        if progress_tx.send(event).await.is_err() {
            break;
        }
    }
    result
}

fn is_dynamic_workflow_envelope(event: &AgentEvent) -> bool {
    match event {
        AgentEvent::ToolStart { name, .. }
        | AgentEvent::ToolExecutionStart { name, .. }
        | AgentEvent::ToolOutputDelta { name, .. }
        | AgentEvent::ToolEnd { name, .. } => name == "dynamic_workflow",
        _ => false,
    }
}

pub(super) fn generated_object<T: DeserializeOwned>(result: &ToolCallResult) -> Result<T, String> {
    if result.exit_code != 0 {
        return Err(result
            .output
            .lines()
            .next()
            .unwrap_or("structured generation failed")
            .to_string());
    }
    let envelope = serde_json::from_str::<Value>(&result.output)
        .map_err(|error| format!("structured generation returned invalid JSON: {error}"))?;
    let object = envelope
        .get("object")
        .cloned()
        .ok_or_else(|| "structured generation response omitted object".to_string())?;
    serde_json::from_value(object)
        .map_err(|error| format!("structured generation object violated its contract: {error}"))
}

#[cfg(test)]
mod timeout_tests {
    use super::{within_inquiry_stage_timeout_typed, InquiryStageError};

    #[tokio::test]
    async fn operation_text_cannot_impersonate_a_typed_timeout() {
        let result = within_inquiry_stage_timeout_typed(
            async {
                Err::<(), _>(
                    "DeepResearch bootstrap acquisition stage timed out after 1 ms".to_string(),
                )
            },
            1_000,
            "bootstrap acquisition",
        )
        .await;

        assert!(matches!(result, Err(InquiryStageError::Operation(_))));
    }
}
