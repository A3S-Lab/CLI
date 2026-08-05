use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ── pure logic ──────────────────────────────────────────────────────────

#[test]
fn unwrap_envelope_returns_data_and_surfaces_errors() {
    let ok =
        RuntimeTool::unwrap_envelope(r#"{"code":200,"status":"OK","data":{"batchId":"b1"}}"#, 200)
            .unwrap();
    assert_eq!(ok.get("batchId").unwrap(), "b1");
    let err =
        RuntimeTool::unwrap_envelope(r#"{"code":403,"message":"Forbidden"}"#, 200).unwrap_err();
    assert!(err.to_string().contains("403") && err.to_string().contains("Forbidden"));
    assert!(RuntimeTool::unwrap_envelope(r#"{"message":"x"}"#, 500).is_err());
    assert!(RuntimeTool::unwrap_envelope("<html>502</html>", 502).is_err());
    let bare = RuntimeTool::unwrap_envelope(r#"{"batchId":"b2"}"#, 200).unwrap();
    assert_eq!(bare.get("batchId").unwrap(), "b2");
}

#[test]
fn idempotency_key_covers_worker_and_task_set() {
    let a = idempotency_key("w1", &[json!("q1"), json!("q2")]);
    assert_eq!(a, idempotency_key("w1", &[json!("q1"), json!("q2")]));
    // Same tasks on a DIFFERENT worker must be a different batch.
    assert_ne!(a, idempotency_key("w2", &[json!("q1"), json!("q2")]));
    assert_ne!(a, idempotency_key("w1", &[json!("q2"), json!("q1")]));
    assert!(a.starts_with("a3s-code-runtime-") && a.len() == "a3s-code-runtime-".len() + 24);
}

#[test]
fn uuid_detection_is_strict() {
    assert!(looks_like_uuid("57989959-0b1d-41da-974c-31ad8101df37"));
    assert!(!looks_like_uuid("risk-reporter"));
    assert!(!looks_like_uuid("57989959-0b1d-41da-974c-31ad8101df3")); // 35 chars
    assert!(!looks_like_uuid("g7989959-0b1d-41da-974c-31ad8101df37")); // non-hex
}

#[test]
fn remote_runtime_calls_always_escalate_authorization() {
    let tool = RuntimeTool {
        origin: "https://runtime.example.invalid".to_string(),
        token: "test-token".to_string(),
        poll_start: Duration::from_millis(1),
        poll_cap: Duration::from_millis(1),
    };

    assert!(tool.requires_confirmation(&json!({
        "worker": "worker",
        "tasks": ["task"]
    })));
}

// ── mock A3S Runtime speaking the exact OS contract ─────────────────────

/// Scripted mock state: each poll consumes the next `poll_plan` item and
/// repeats the final item after the plan is exhausted.
struct MockState {
    submit_path: Option<String>,
    submit_body: Option<String>,
    assets_response: Option<String>,
    submit_response: Option<String>,
    invocation_response: Option<String>,
    /// Each entry: Some(counts json) → 200 with those counts; None → HTTP 500.
    poll_plan: Vec<Option<String>>,
    poll_idx: usize,
}

async fn spawn_mock(state: Arc<Mutex<MockState>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let st = state.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16384];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let line = req.lines().next().unwrap_or("").to_string();
                let body = req.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                let (status, payload) = route(&st, &line, &body);
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            });
        }
    });
    origin
}

fn route(st: &Arc<Mutex<MockState>>, line: &str, body: &str) -> (&'static str, String) {
    let env = |data: &str| format!(r#"{{"code":200,"status":"OK","data":{data}}}"#);
    // Specific paths BEFORE the generic "/batch" (substring overlap).
    if line.contains("/api/v1/assets") {
        if let Some(response) = st.lock().unwrap().assets_response.clone() {
            return ("200 OK", response);
        }
        return (
            "200 OK",
            env(r#"{"items":[
                {"id":"57989959-0b1d-41da-974c-31ad8101df37","name":"risk-reporter","agentKind":"tool"},
                {"id":"74af5078-7b53-4857-bf69-fc59c9fdce06","name":"shangfei-poc3","agentKind":"tool"},
                {"id":"02f3b08e-9358-43aa-97c3-05981b57a1a2","name":"some-app","agentKind":"application"}
            ]}"#),
        );
    }
    if line.contains("/batches/") {
        let mut s = st.lock().unwrap();
        let i = s.poll_idx.min(s.poll_plan.len().saturating_sub(1));
        s.poll_idx += 1;
        return match s.poll_plan.get(i).cloned().flatten() {
            Some(payload) => {
                let trimmed = payload.trim();
                let data = if poll_payload_is_counts(trimmed) {
                    format!(r#"{{"batchId":"batch-1","counts":{trimmed}}}"#)
                } else {
                    let inner = trimmed.trim_start_matches('{').trim_end_matches('}');
                    if inner.contains(r#""batchId""#) {
                        format!("{{{inner}}}")
                    } else {
                        format!(r#"{{"batchId":"batch-1",{inner}}}"#)
                    }
                };
                ("200 OK", env(&data))
            }
            None => ("500 Internal Server Error", "boom".to_string()),
        };
    }
    if line.contains("/invocations/inv-1") {
        if let Some(response) = st.lock().unwrap().invocation_response.clone() {
            return ("200 OK", response);
        }
        return (
            "200 OK",
            env(
                r#"{"status":"succeeded","result":{"status":"succeeded","output":{"answer":"alpha"},"error":null}}"#,
            ),
        );
    }
    if line.contains("/invocations/") {
        if let Some(response) = st.lock().unwrap().invocation_response.clone() {
            return ("200 OK", response);
        }
        return ("200 OK", env(r#"{"status":"running","result":null}"#));
    }
    if line.contains("/batch") {
        let mut s = st.lock().unwrap();
        s.submit_path = Some(line.split_whitespace().nth(1).unwrap_or("").to_string());
        s.submit_body = Some(body.to_string());
        if let Some(response) = s.submit_response.clone() {
            return ("200 OK", response);
        }
        return (
            "200 OK",
            env(r#"{"batchId":"batch-1","invocationIds":["inv-1","inv-2"]}"#),
        );
    }
    ("404 Not Found", "{}".to_string())
}

fn poll_payload_is_counts(payload: &str) -> bool {
    payload.contains(r#""queued":"#)
        || payload.contains(r#""running":"#)
        || payload.contains(r#""succeeded":"#)
        || payload.contains(r#""failed":"#)
        || payload.contains(r#""canceled":"#)
        || payload.contains(r#""cancelled":"#)
}

fn fast_tool(origin: String) -> RuntimeTool {
    RuntimeTool {
        origin,
        token: "test-token".into(),
        poll_start: Duration::from_millis(10),
        poll_cap: Duration::from_millis(20),
    }
}

fn state(poll_plan: Vec<Option<&str>>) -> Arc<Mutex<MockState>> {
    Arc::new(Mutex::new(MockState {
        submit_path: None,
        submit_body: None,
        assets_response: None,
        submit_response: None,
        invocation_response: None,
        poll_plan: poll_plan.into_iter().map(|p| p.map(String::from)).collect(),
        poll_idx: 0,
    }))
}

/// A ToolContext with a progress channel; returns (ctx, drained-events fn).
fn ctx_with_progress() -> (ToolContext, tokio::sync::mpsc::Receiver<ToolStreamEvent>) {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let ctx = ToolContext::new(std::env::temp_dir()).with_event_tx(tx);
    (ctx, rx)
}

fn ctx_with_progress_and_agent_events() -> (
    ToolContext,
    tokio::sync::mpsc::Receiver<ToolStreamEvent>,
    tokio::sync::broadcast::Receiver<AgentEvent>,
) {
    let (tool_tx, tool_rx) = tokio::sync::mpsc::channel(64);
    let (agent_tx, agent_rx) = tokio::sync::broadcast::channel(64);
    let ctx = ToolContext::new(std::env::temp_dir())
        .with_event_tx(tool_tx)
        .with_agent_event_tx(agent_tx);
    (ctx, tool_rx, agent_rx)
}

#[tokio::test]
async fn full_flow_streams_progress_and_aggregates() {
    // Two live ticks then terminal — exercises backoff + change-only progress.
    let st = state(vec![
        Some(r#"{"queued":1,"running":1,"succeeded":0,"failed":0}"#),
        Some(r#"{"queued":0,"running":1,"succeeded":1,"failed":0}"#),
        Some(r#"{"queued":0,"running":0,"succeeded":2,"failed":0}"#),
    ]);
    let origin = spawn_mock(st.clone()).await;
    let tool = fast_tool(origin);
    let (ctx, mut rx, mut agent_rx) = ctx_with_progress_and_agent_events();
    let out = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "57989959-0b1d-41da-974c-31ad8101df37" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(out.success, "{}", out.content);

    // Request contract: inputs + agentKind + worker-scoped idempotency key.
    let sent: Value =
        serde_json::from_str(st.lock().unwrap().submit_body.as_ref().unwrap()).unwrap();
    assert_eq!(sent["inputs"], json!(["a", "b"]));
    assert_eq!(sent["agentKind"], "tool");
    assert_eq!(
        sent["idempotencyKey"].as_str().unwrap(),
        idempotency_key(
            "57989959-0b1d-41da-974c-31ad8101df37",
            &[json!("a"), json!("b")]
        )
    );

    // Progress streamed: submit line + one line per distinct counts picture.
    let mut deltas = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        let ToolStreamEvent::OutputDelta(s) = ev;
        deltas.push(s);
    }
    let all = deltas.join("");
    assert!(all.contains("2 parallel subtasks submitted"), "{all}");
    assert!(
        all.contains("0/2 done") && all.contains("2/2 done"),
        "{all}"
    );

    // Aggregation: both results, in invocation order.
    let agg: Value = serde_json::from_str(&out.content).unwrap();
    assert_eq!(agg["count"], 2);
    assert_eq!(agg["results"][0]["output"]["answer"], "alpha");
    assert!(
        agg.get("partial").is_none(),
        "terminal batch is not partial"
    );

    let mut starts = Vec::new();
    let mut ends = Vec::new();
    while let Ok(event) = agent_rx.try_recv() {
        match event {
            AgentEvent::SubagentStart {
                task_id,
                session_id,
                agent,
                description,
                ..
            } => starts.push((task_id, session_id, agent, description)),
            AgentEvent::SubagentEnd {
                task_id,
                session_id,
                agent,
                ..
            } => ends.push((task_id, session_id, agent)),
            _ => {}
        }
    }
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert_eq!(ends.len(), 2, "{ends:?}");
    assert_eq!(starts[0].0, "runtime-inv-1");
    assert_eq!(starts[0].1, "runtime-batch-1-0");
    assert_eq!(starts[0].2, "runtime");
    assert_eq!(starts[0].3, "a");
    assert_eq!(starts[1].0, "runtime-inv-2");
    assert_eq!(starts[1].1, "runtime-batch-1-1");
    assert_eq!(ends[0].0, "runtime-inv-1");
    assert_eq!(ends[0].1, starts[0].1);
    assert_eq!(ends[1].0, "runtime-inv-2");
    assert_eq!(ends[1].1, starts[1].1);
}

#[tokio::test]
async fn transient_poll_failure_is_tolerated_but_persistent_is_not() {
    // One 500 tick between two good ones → still succeeds.
    let st = state(vec![
        Some(r#"{"queued":0,"running":1,"succeeded":1,"failed":0}"#),
        None, // 500
        Some(r#"{"queued":0,"running":0,"succeeded":2,"failed":0}"#),
    ]);
    let origin = spawn_mock(st).await;
    let tool = fast_tool(origin);
    let (ctx, _rx) = ctx_with_progress();
    let out = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "57989959-0b1d-41da-974c-31ad8101df37" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(out.success, "one flaky tick must not abandon the batch");

    // 4+ consecutive failures → gives up with the poll error surfaced.
    let st2 = state(vec![
        Some(r#"{"queued":0,"running":2,"succeeded":0,"failed":0}"#),
        None,
        None,
        None,
        None,
    ]);
    let origin2 = spawn_mock(st2).await;
    let tool2 = fast_tool(origin2);
    let (ctx2, _rx2) = ctx_with_progress();
    let out2 = tool2
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "57989959-0b1d-41da-974c-31ad8101df37" }),
            &ctx2,
        )
        .await
        .unwrap();
    assert!(!out2.success);
    assert!(
        out2.content.contains("consecutive times"),
        "{}",
        out2.content
    );
}

#[tokio::test]
async fn timeout_returns_partial_results_not_nothing() {
    // Batch never finishes; budget expires after the first tick. The
    // finished member (inv-1) must still come back, flagged partial.
    let st = state(vec![Some(
        r#"{"queued":0,"running":1,"succeeded":1,"failed":0}"#,
    )]);
    let origin = spawn_mock(st).await;
    let tool = fast_tool(origin);
    let (ctx, _rx) = ctx_with_progress();
    let out = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "57989959-0b1d-41da-974c-31ad8101df37", "timeout_ms": 1 }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(out.success, "{}", out.content);
    let agg: Value = serde_json::from_str(&out.content).unwrap();
    assert_eq!(agg["partial"], true);
    assert!(agg["note"].as_str().unwrap().contains("batch-1"));
    assert_eq!(agg["results"][0]["output"]["answer"], "alpha"); // finished one kept
    assert_eq!(agg["results"][1]["state"], "unknown"); // unfinished: no result yet
}

#[tokio::test]
async fn poll_without_counts_does_not_finish_until_status_is_terminal() {
    let st = state(vec![
        Some(r#"{"status":"running"}"#),
        Some(r#"{"status":"running"}"#),
        Some(r#"{"status":"completed"}"#),
    ]);
    let origin = spawn_mock(st.clone()).await;
    let tool = fast_tool(origin);
    let (ctx, _rx) = ctx_with_progress();
    let out = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "57989959-0b1d-41da-974c-31ad8101df37" }),
            &ctx,
        )
        .await
        .unwrap();

    assert!(out.success, "{}", out.content);
    assert_eq!(
        st.lock().unwrap().poll_idx,
        3,
        "missing counts must not make the first poll look terminal"
    );
    let agg: Value = serde_json::from_str(&out.content).unwrap();
    assert!(
        agg.get("partial").is_none(),
        "terminal status should not be marked partial"
    );
}

#[tokio::test]
async fn incomplete_counts_do_not_finish_until_expected_task_count_is_terminal() {
    let st = state(vec![
        Some(r#"{"queued":0,"running":0,"succeeded":1,"failed":0}"#),
        Some(r#"{"queued":0,"running":0,"succeeded":2,"failed":0}"#),
    ]);
    let origin = spawn_mock(st.clone()).await;
    let tool = fast_tool(origin);
    let (ctx, _rx) = ctx_with_progress();
    let out = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "57989959-0b1d-41da-974c-31ad8101df37" }),
            &ctx,
        )
        .await
        .unwrap();

    assert!(out.success, "{}", out.content);
    assert_eq!(
        st.lock().unwrap().poll_idx,
        2,
        "counts with fewer terminal tasks than submitted must keep polling"
    );
    let agg: Value = serde_json::from_str(&out.content).unwrap();
    assert!(
        agg.get("partial").is_none(),
        "eventual full counts should not be marked partial"
    );
}

#[tokio::test]
async fn worker_name_resolves_to_uuid_and_unknown_names_list_options() {
    let st = state(vec![Some(
        r#"{"queued":0,"running":0,"succeeded":2,"failed":0}"#,
    )]);
    let origin = spawn_mock(st.clone()).await;
    let tool = fast_tool(origin.clone());
    let (ctx, _rx) = ctx_with_progress();
    // Name → UUID: the submit URL must target the resolved asset id.
    let out = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "risk-reporter" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(out.success, "{}", out.content);
    let path = st.lock().unwrap().submit_path.clone().unwrap();
    assert!(
        path.contains("/functions/57989959-0b1d-41da-974c-31ad8101df37/batch"),
        "{path}"
    );

    // Unknown name → error listing available tool workers (not applications).
    let (ctx2, _rx2) = ctx_with_progress();
    let out2 = tool
        .execute(
            &json!({ "tasks": ["a"], "worker": "no-such-worker" }),
            &ctx2,
        )
        .await
        .unwrap();
    assert!(!out2.success);
    assert!(out2.content.contains("risk-reporter"), "{}", out2.content);
    assert!(
        !out2.content.contains("some-app"),
        "application-kind assets are not workers"
    );
}

#[tokio::test]
async fn runtime_contract_rejects_oversized_responses_and_unsafe_ids() {
    let oversized = state(vec![Some(
        r#"{"queued":0,"running":0,"succeeded":2,"failed":0}"#,
    )]);
    oversized.lock().unwrap().submit_response = Some("x".repeat(MAX_CONTROL_RESPONSE_BYTES + 1));
    let tool = fast_tool(spawn_mock(oversized).await);
    let ctx = ToolContext::new(std::env::temp_dir());
    let output = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "57989959-0b1d-41da-974c-31ad8101df37" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!output.success);
    assert!(
        output.content.contains("response exceeds"),
        "{}",
        output.content
    );

    let unsafe_id = state(vec![Some(
        r#"{"queued":0,"running":0,"succeeded":2,"failed":0}"#,
    )]);
    unsafe_id.lock().unwrap().submit_response = Some(
        r#"{"code":200,"data":{"batchId":"../escape","invocationIds":["inv-1","inv-2"]}}"#
            .to_string(),
    );
    let tool = fast_tool(spawn_mock(unsafe_id).await);
    let output = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "57989959-0b1d-41da-974c-31ad8101df37" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!output.success);
    assert!(
        output.content.contains("invalid batchId"),
        "{}",
        output.content
    );
    assert!(!output.content.contains("../escape"));
}

#[tokio::test]
async fn resolved_worker_must_be_a_canonical_uuid() {
    let st = state(vec![Some(
        r#"{"queued":0,"running":0,"succeeded":2,"failed":0}"#,
    )]);
    st.lock().unwrap().assets_response = Some(
        r#"{"code":200,"data":{"items":[{"id":"not-a-uuid","name":"bad-worker","agentKind":"tool"}]}}"#
            .to_string(),
    );
    let tool = fast_tool(spawn_mock(st).await);
    let ctx = ToolContext::new(std::env::temp_dir());
    let output = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "bad-worker" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!output.success);
    assert!(
        output.content.contains("non-canonical UUID"),
        "{}",
        output.content
    );
}

#[tokio::test]
async fn cancellation_interrupts_poll_backoff() {
    let st = state(vec![Some(
        r#"{"queued":0,"running":2,"succeeded":0,"failed":0}"#,
    )]);
    let origin = spawn_mock(st).await;
    let mut tool = fast_tool(origin);
    tool.poll_start = Duration::from_secs(5);
    tool.poll_cap = Duration::from_secs(5);
    let ctx = ToolContext::new(std::env::temp_dir());
    let cancellation = ctx.cancellation_token();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancellation.cancel();
    });
    let started = Instant::now();
    let output = tool
        .execute(
            &json!({ "tasks": ["a", "b"], "worker": "57989959-0b1d-41da-974c-31ad8101df37" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!output.success);
    assert!(output.content.contains("cancelled"), "{}", output.content);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "cancellation must not wait for the five-second poll interval"
    );
}

#[tokio::test]
async fn poll_timeout_uses_an_absolute_deadline() {
    let st = state(vec![Some(
        r#"{"queued":0,"running":1,"succeeded":1,"failed":0}"#,
    )]);
    let origin = spawn_mock(st).await;
    let mut tool = fast_tool(origin);
    tool.poll_start = Duration::from_secs(5);
    tool.poll_cap = Duration::from_secs(5);
    let ctx = ToolContext::new(std::env::temp_dir());
    let started = Instant::now();
    let output = tool
        .execute(
            &json!({
                "tasks": ["a", "b"],
                "worker": "57989959-0b1d-41da-974c-31ad8101df37",
                "timeout_ms": 20
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(output.success, "{}", output.content);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the configured deadline must interrupt the five-second backoff"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&output.content).unwrap()["partial"],
        true
    );
}

#[tokio::test]
async fn runtime_events_and_results_strip_terminal_control_payloads() {
    let st = state(vec![Some(
        r#"{"queued":0,"running":0,"succeeded":2,"failed":0}"#,
    )]);
    let poisoned = "visible\u{1b}]8;;https://evil.invalid\u{7}hidden\u{1b}]8;;\u{7}\u{202e}safe";
    let invocation = json!({
        "code": 200,
        "data": {
            "status": "succeeded",
            "result": {"status": "succeeded", "output": poisoned, "error": null}
        }
    });
    st.lock().unwrap().invocation_response = Some(invocation.to_string());
    let tool = fast_tool(spawn_mock(st).await);
    let (ctx, _progress, mut events) = ctx_with_progress_and_agent_events();
    let output = tool
        .execute(
            &json!({ "tasks": [poisoned, "plain"], "worker": "57989959-0b1d-41da-974c-31ad8101df37" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(output.success, "{}", output.content);
    assert!(!output.content.contains('\u{1b}'));
    assert!(!output.content.contains('\u{202e}'));

    let mut descriptions = Vec::new();
    let mut endings = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            AgentEvent::SubagentStart { description, .. } => descriptions.push(description),
            AgentEvent::SubagentEnd { output, .. } => endings.push(output),
            _ => {}
        }
    }
    assert_eq!(descriptions.len(), 2);
    assert_eq!(endings.len(), 2);
    for text in descriptions.iter().chain(endings.iter()) {
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert!(!text.contains('\u{202e}'), "{text:?}");
        assert!(!text.contains("evil.invalid"), "{text:?}");
    }
}

#[tokio::test]
async fn bad_args_are_tool_errors_not_requests() {
    let tool = fast_tool("http://127.0.0.1:1".into()); // never contacted
    let ctx = ToolContext::new(std::env::temp_dir());
    let e1 = tool
        .execute(&json!({ "tasks": [], "worker": "w" }), &ctx)
        .await
        .unwrap();
    assert!(!e1.success);
    let e2 = tool
        .execute(&json!({ "tasks": ["x"] }), &ctx)
        .await
        .unwrap();
    assert!(!e2.success && e2.content.contains("worker"));

    let invalid_task = tool
        .execute(&json!({ "tasks": [42], "worker": "w" }), &ctx)
        .await
        .unwrap();
    assert!(!invalid_task.success && invalid_task.content.contains("string or object"));

    let too_many = vec![json!("x"); MAX_TASKS + 1];
    let e3 = tool
        .execute(&json!({ "tasks": too_many, "worker": "w" }), &ctx)
        .await
        .unwrap();
    assert!(!e3.success && e3.content.contains("at most"));

    let e4 = tool
        .execute(
            &json!({ "tasks": ["x".repeat(MAX_SUBMIT_BODY_BYTES + 1)], "worker": "w" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!e4.success && e4.content.contains("serialized `tasks` exceeds"));

    for timeout in [json!(0), json!(MAX_BATCH_TIMEOUT_MS + 1), json!(1.5)] {
        let error = tool
            .execute(
                &json!({ "tasks": ["x"], "worker": "w", "timeout_ms": timeout }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!error.success && error.content.contains("timeout_ms"));
    }

    let unsafe_worker = tool
        .execute(
            &json!({ "tasks": ["x"], "worker": "safe\u{1b}]0;unsafe\u{7}" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!unsafe_worker.success && unsafe_worker.content.contains("terminal-safe"));

    let cancelled_ctx = ToolContext::new(std::env::temp_dir());
    cancelled_ctx.cancellation_token().cancel();
    let cancelled = tool
        .execute(
            &json!({ "tasks": ["x"], "worker": "worker" }),
            &cancelled_ctx,
        )
        .await
        .unwrap();
    assert!(!cancelled.success && cancelled.content.contains("cancelled"));
}
