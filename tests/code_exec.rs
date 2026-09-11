mod support;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use support::{a3s_bin, TempWorkspace};

const TEST_API_KEY: &str = "code-exec-api-key-must-not-leak";

struct FakeOpenAi {
    base_url: String,
    main_calls: Arc<AtomicUsize>,
    extract_calls: Arc<AtomicUsize>,
    saw_image: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeOpenAi {
    fn start() -> Self {
        Self::start_with_behavior(FakeBehavior::WorkspaceWrite)
    }

    fn start_with_memory_extract(token: &'static str) -> Self {
        Self::start_with_behavior(FakeBehavior::MemoryExtract { token })
    }

    fn start_with_behavior(behavior: FakeBehavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let main_calls = Arc::new(AtomicUsize::new(0));
        let thread_calls = Arc::clone(&main_calls);
        let extract_calls = Arc::new(AtomicUsize::new(0));
        let thread_extract_calls = Arc::clone(&extract_calls);
        let saw_image = Arc::new(AtomicBool::new(false));
        let thread_saw_image = Arc::clone(&saw_image);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let request = read_request(&mut stream);
                        let Some(body) = request_body(&request) else {
                            continue;
                        };
                        if body.to_string().contains("\"image_url\"") {
                            thread_saw_image.store(true, Ordering::SeqCst);
                        }
                        let streaming =
                            body.get("stream").and_then(serde_json::Value::as_bool) == Some(true);
                        let pre_analysis =
                            request_contains_message(&body, "You are a pre-analysis assistant");
                        let memory_extraction = memory_extraction_request(&body);
                        if memory_extraction {
                            thread_extract_calls.fetch_add(1, Ordering::SeqCst);
                        }
                        let message = if pre_analysis {
                            match behavior {
                                FakeBehavior::WorkspaceWrite => pre_analysis_message(),
                                FakeBehavior::MemoryExtract { token } => {
                                    memory_extract_pre_analysis_message(token)
                                }
                            }
                        } else if memory_extraction {
                            match behavior {
                                FakeBehavior::WorkspaceWrite => serde_json::json!({
                                    "role": "assistant",
                                    "content": "{\"items\":[]}"
                                }),
                                FakeBehavior::MemoryExtract { token } => serde_json::json!({
                                    "role": "assistant",
                                    "content": format!(
                                        "{{\"items\":[{{\"memory_type\":\"semantic\",\"content\":\"Always use verification codename {token} for the memory-extract live gate.\",\"importance\":0.9,\"confidence\":0.95,\"tags\":[\"preference\",\"memory-extract\"],\"source\":\"preference\",\"scope\":\"workspace\",\"reason\":\"Reusable durable preference for future sessions.\"}}]}}"
                                    )
                                }),
                            }
                        } else if matches!(behavior, FakeBehavior::MemoryExtract { .. }) {
                            thread_calls.fetch_add(1, Ordering::SeqCst);
                            let FakeBehavior::MemoryExtract { token } = behavior else {
                                unreachable!()
                            };
                            serde_json::json!({
                                "role": "assistant",
                                "content": format!(
                                    "Understood. I will use the verification codename {token} when referring to the memory-extract live gate."
                                )
                            })
                        } else if thread_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            serde_json::json!({
                                "role": "assistant",
                                "content": null,
                                "tool_calls": [{
                                    "id": "call-write-answer",
                                    "type": "function",
                                    "function": {
                                        "name": "write",
                                        "arguments": "{\"file_path\":\"answer.txt\",\"content\":\"42\\n\"}"
                                    }
                                }]
                            })
                        } else {
                            serde_json::json!({
                                "role": "assistant",
                                "content": "Completed and verified."
                            })
                        };
                        let finish_reason = if pre_analysis || memory_extraction {
                            "stop"
                        } else if matches!(behavior, FakeBehavior::MemoryExtract { .. }) {
                            "stop"
                        } else if thread_calls.load(Ordering::SeqCst) == 1 {
                            "tool_calls"
                        } else {
                            "stop"
                        };
                        if streaming {
                            write_sse_response(
                                &mut stream,
                                streaming_completion_sse(&message, finish_reason).as_bytes(),
                            );
                            continue;
                        }
                        let response = serde_json::to_vec(&serde_json::json!({
                            "id": "chatcmpl-code-exec-test",
                            "object": "chat.completion",
                            "created": 0,
                            "model": "fake",
                            "choices": [{
                                "index": 0,
                                "message": message,
                                "finish_reason": finish_reason
                            }],
                            "usage": {
                                "prompt_tokens": 1,
                                "completion_tokens": 1,
                                "total_tokens": 2
                            }
                        }))
                        .unwrap();
                        write_response(&mut stream, "200 OK", &response);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fake OpenAI listener failed: {error}"),
                }
            }
        });
        Self {
            base_url,
            main_calls,
            extract_calls,
            saw_image,
            stop,
            thread: Some(thread),
        }
    }

    fn main_calls(&self) -> usize {
        self.main_calls.load(Ordering::SeqCst)
    }

    fn extract_calls(&self) -> usize {
        self.extract_calls.load(Ordering::SeqCst)
    }

    fn saw_image(&self) -> bool {
        self.saw_image.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Copy)]
enum FakeBehavior {
    WorkspaceWrite,
    MemoryExtract { token: &'static str },
}

impl Drop for FakeOpenAi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if request.len() >= header_end + 4 + content_length {
            break;
        }
    }
    request
}

fn request_body(request: &[u8]) -> Option<serde_json::Value> {
    let body_start = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?
        + 4;
    serde_json::from_slice(&request[body_start..]).ok()
}

fn request_contains_message(body: &serde_json::Value, needle: &str) -> bool {
    body.get("messages")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                message
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|content| content.contains(needle))
            })
        })
}

fn memory_extraction_request(body: &serde_json::Value) -> bool {
    request_contains_message(
        body,
        "You extract durable, reusable memory for a coding agent",
    )
}

fn write_sse_response(stream: &mut TcpStream, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
}

fn streaming_completion_sse(message: &serde_json::Value, finish_reason: &str) -> String {
    let mut delta = serde_json::json!({ "role": "assistant" });
    if let Some(content) = message.get("content").filter(|value| !value.is_null()) {
        delta["content"] = content.clone();
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(|value| value.as_array()) {
        delta["tool_calls"] = serde_json::Value::Array(
            tool_calls
                .iter()
                .enumerate()
                .map(|(index, call)| {
                    let mut indexed = call.clone();
                    if let Some(object) = indexed.as_object_mut() {
                        object.insert("index".into(), serde_json::json!(index));
                    }
                    indexed
                })
                .collect(),
        );
    }
    let first = serde_json::json!({
        "id": "chatcmpl-code-exec-test",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "fake",
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": null
        }],
        "usage": null
    });
    let done = serde_json::json!({
        "id": "chatcmpl-code-exec-test",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "fake",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": finish_reason
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    });
    format!("data: {first}\n\ndata: {done}\n\ndata: [DONE]\n\n")
}

fn write_response(stream: &mut TcpStream, status: &str, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
}

fn pre_analysis_message() -> serde_json::Value {
    serde_json::json!({
        "role": "assistant",
        "content": serde_json::json!({
            "intent": "GeneralPurpose",
            "requires_planning": false,
            "goal": {
                "description": "Write 42 to answer.txt.",
                "success_criteria": ["answer.txt contains 42"]
            },
            "execution_plan": {
                "complexity": "Simple",
                "steps": [{
                    "id": "step-1",
                    "description": "Update answer.txt",
                    "tool": "write",
                    "dependencies": [],
                    "success_criteria": "answer.txt contains 42"
                }],
                "required_tools": ["write"]
            },
            "optimized_input": "Write 42 to answer.txt."
        })
        .to_string()
    })
}

fn memory_extract_pre_analysis_message(token: &str) -> serde_json::Value {
    serde_json::json!({
        "role": "assistant",
        "content": serde_json::json!({
            "intent": "GeneralPurpose",
            "requires_planning": false,
            "goal": {
                "description": format!("Acknowledge durable preference {token}."),
                "success_criteria": [format!("reply mentions {token}")]
            },
            "execution_plan": {
                "complexity": "Simple",
                "steps": [{
                    "id": "step-1",
                    "description": "Acknowledge the preference without tools",
                    "tool": null,
                    "dependencies": [],
                    "success_criteria": format!("reply mentions {token}")
                }],
                "required_tools": []
            },
            "optimized_input": format!(
                "Remember durable preference codename {token} and confirm understanding."
            )
        })
        .to_string()
    })
}

fn fixture(name: &str) -> (TempWorkspace, std::path::PathBuf, FakeOpenAi) {
    let root = TempWorkspace::new(name);
    let project = root.path("project");
    std::fs::create_dir_all(project.join(".a3s")).unwrap();
    std::fs::write(project.join("answer.txt"), "0\n").unwrap();
    let server = FakeOpenAi::start();
    std::fs::write(
        project.join(".a3s/config.acl"),
        format!(
            "default_model = \"openai/fake\"\nproviders \"openai\" {{\n  apiKey = \"{TEST_API_KEY}\"\n  baseUrl = \"{}\"\n  models \"fake\" {{ name = \"Fake\" attachment = true }}\n}}\n",
            server.base_url,
        ),
    )
    .unwrap();
    (root, project, server)
}

fn run(project: &std::path::Path, mode: &str, root: &TempWorkspace) -> std::process::Output {
    run_with_policy(project, mode, None, root)
}

fn run_with_policy(
    project: &std::path::Path,
    mode: &str,
    tool_policy: Option<&str>,
    root: &TempWorkspace,
) -> std::process::Output {
    run_with_policy_and_output(project, mode, tool_policy, root, "json")
}

fn run_with_policy_and_output(
    project: &std::path::Path,
    mode: &str,
    tool_policy: Option<&str>,
    root: &TempWorkspace,
    output: &str,
) -> std::process::Output {
    let mut command = Command::new(a3s_bin());
    command
        .args(["--output", output, "--non-interactive", "--directory"])
        .arg(project)
        .args(["code", "exec", "--mode", mode]);
    if let Some(tool_policy) = tool_policy {
        command.args(["--tool-policy", tool_policy]);
    }
    command
        .args([
            "--model",
            "openai/fake",
            "Write 42 to answer.txt, then verify it.",
        ])
        .env("HOME", root.path("home"))
        .env("A3S_DATA_HOME", root.path("data"))
        .env("A3S_STATE_HOME", root.path("state"))
        .env("A3S_CACHE_HOME", root.path("cache"))
        .output()
        .unwrap()
}

fn write_png(path: &std::path::Path, color: [u8; 3]) {
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 2, image::Rgb(color)))
        .save(path)
        .unwrap();
}

#[test]
fn auto_mode_executes_bounded_workspace_edits() {
    let (root, project, server) = fixture("code-exec-auto");
    let output = run(&project, "auto", &root);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(
        std::fs::read_to_string(project.join("answer.txt")).unwrap(),
        "42\n"
    );
    assert_eq!(server.main_calls(), 2);
}

#[test]
fn workspace_write_profile_executes_bounded_edits_and_echoes_the_boundary() {
    let (root, project, server) = fixture("code-exec-workspace-write");
    let output = run_with_policy(&project, "auto", Some("workspace-write"), &root);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["data"]["toolPolicy"], "workspace-write");
    assert_eq!(
        std::fs::read_to_string(project.join("answer.txt")).unwrap(),
        "42\n"
    );
    assert_eq!(server.main_calls(), 2);
}

#[test]
fn jsonl_exec_omits_provider_credentials_and_endpoint() {
    let (root, project, server) = fixture("code-exec-jsonl-redaction");
    let output =
        run_with_policy_and_output(&project, "auto", Some("workspace-write"), &root, "jsonl");

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for rendered in [&output.stdout, &output.stderr] {
        let rendered = String::from_utf8_lossy(rendered);
        assert!(!rendered.contains(TEST_API_KEY), "API key leaked");
        assert!(
            !rendered.contains(&server.base_url),
            "provider endpoint leaked"
        );
    }

    let documents = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let agent_end = documents
        .iter()
        .find(|document| {
            document
                .pointer("/event/type")
                .and_then(|value| value.as_str())
                == Some("agent_end")
        })
        .expect("JSONL must retain the terminal agent event");
    assert_eq!(agent_end.pointer("/event/meta/provider").unwrap(), "openai");
    assert!(agent_end.pointer("/event/meta/request_url").is_none());
}

#[test]
fn workspace_write_profile_rejects_non_auto_mode_as_usage() {
    let output = Command::new(a3s_bin())
        .args([
            "--output",
            "json",
            "--non-interactive",
            "code",
            "exec",
            "--mode",
            "plan",
            "--tool-policy",
            "workspace-write",
            "Do not run.",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "usage.invalid");
}

#[test]
fn exec_transmits_repeated_and_comma_separated_images() {
    let (root, project, server) = fixture("code-exec-images");
    write_png(&project.join("before.png"), [10, 20, 30]);
    write_png(&project.join("after.png"), [30, 20, 10]);
    write_png(&project.join("reference.png"), [40, 50, 60]);

    let output = Command::new(a3s_bin())
        .args(["--output", "json", "--non-interactive", "--directory"])
        .arg(&project)
        .args([
            "code",
            "exec",
            "--mode",
            "auto",
            "--model",
            "openai/fake",
            "--image",
            "before.png,after.png",
            "-i",
            "reference.png",
            "Compare these screenshots, then write 42 to answer.txt and verify it.",
        ])
        .env("HOME", root.path("home"))
        .env("A3S_DATA_HOME", root.path("data"))
        .env("A3S_STATE_HOME", root.path("state"))
        .env("A3S_CACHE_HOME", root.path("cache"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["data"]["imageCount"], 3);
    assert!(
        server.saw_image(),
        "provider request must contain image_url"
    );
}

fn walkdir_listing(root: &std::path::Path) -> String {
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            entries.push(path.display().to_string());
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    entries.sort();
    entries.join("\n")
}

#[test]
fn unresolved_default_mode_approval_fails_without_model_retries() {
    let (root, project, server) = fixture("code-exec-approval");
    let output = run(&project, "default", &root);

    assert!(!output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(
        result["error"]["code"],
        "approval.required",
        "result={result} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(project.join("answer.txt")).unwrap(),
        "0\n"
    );
    assert_eq!(
        server.main_calls(),
        1,
        "the model must not retry an approval that non-interactive exec cannot resolve"
    );
}

#[test]
fn auto_mode_persists_llm_memory_extraction_into_workspace_store() {
    const TOKEN: &str = "MEM_EXTRACT_TOKEN_7711";
    let root = TempWorkspace::new("code-exec-memory-extract");
    let project = root.path("project");
    std::fs::create_dir_all(project.join(".a3s")).unwrap();
    let server = FakeOpenAi::start_with_memory_extract(TOKEN);
    std::fs::write(
        project.join(".a3s/config.acl"),
        format!(
            "default_model = \"openai/fake\"\nproviders \"openai\" {{\n  apiKey = \"{TEST_API_KEY}\"\n  baseUrl = \"{}\"\n  models \"fake\" {{ name = \"Fake\" }}\n}}\n",
            server.base_url,
        ),
    )
    .unwrap();

    let prompt = format!(
        "Please remember this durable workspace preference for future sessions: \
         always use the verification codename {TOKEN} when referring to the \
         memory-extract live gate. Confirm you understood the preference. Do not call tools."
    );
    let output = Command::new(a3s_bin())
        .args(["--output", "json", "--non-interactive", "--directory"])
        .arg(&project)
        .args([
            "code",
            "exec",
            "--mode",
            "auto",
            "--tool-policy",
            "read-only",
            "--model",
            "openai/fake",
            prompt.as_str(),
        ])
        .env("HOME", root.path("home"))
        .env("A3S_DATA_HOME", root.path("data"))
        .env("A3S_STATE_HOME", root.path("state"))
        .env("A3S_CACHE_HOME", root.path("cache"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        server.extract_calls() >= 1,
        "LLM memory extraction must be invoked; extract_calls={} main_calls={} stdout={}",
        server.extract_calls(),
        server.main_calls(),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        server.main_calls() >= 1,
        "main model turn must run before extraction"
    );

    let memory_dir = project.join(".a3s/memory");
    let memory_index = memory_dir.join("index.json");
    let listing = walkdir_listing(&project.join(".a3s"));
    assert!(
        memory_index.is_file(),
        "code exec must materialize durable memory under .a3s/memory; listing={listing}; extract_calls={}; stdout={}; stderr={}",
        server.extract_calls(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stored = std::fs::read_to_string(&memory_index).unwrap();
    assert!(
        stored.to_ascii_lowercase().contains(&TOKEN.to_ascii_lowercase()),
        "extracted durable memory must mention {TOKEN}: {stored}"
    );
}
