use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;

const DIMENSION: usize = 8;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub(super) struct OracleTarget {
    pub(super) query: &'static str,
    pub(super) identifier: &'static str,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EmbeddingSnapshot {
    pub(super) requests: usize,
    pub(super) document_requests: usize,
    pub(super) query_requests: usize,
    pub(super) document_inputs: usize,
    pub(super) query_inputs: usize,
    pub(super) input_bytes: usize,
    pub(super) non_text_inputs: usize,
}

impl EmbeddingSnapshot {
    pub(super) fn difference(self, earlier: Self) -> Self {
        Self {
            requests: self.requests.saturating_sub(earlier.requests),
            document_requests: self
                .document_requests
                .saturating_sub(earlier.document_requests),
            query_requests: self.query_requests.saturating_sub(earlier.query_requests),
            document_inputs: self.document_inputs.saturating_sub(earlier.document_inputs),
            query_inputs: self.query_inputs.saturating_sub(earlier.query_inputs),
            input_bytes: self.input_bytes.saturating_sub(earlier.input_bytes),
            non_text_inputs: self.non_text_inputs.saturating_sub(earlier.non_text_inputs),
        }
    }
}

#[derive(Default)]
struct Counters {
    requests: AtomicUsize,
    document_requests: AtomicUsize,
    query_requests: AtomicUsize,
    document_inputs: AtomicUsize,
    query_inputs: AtomicUsize,
    input_bytes: AtomicUsize,
    non_text_inputs: AtomicUsize,
}

pub(super) struct EmbeddingServer {
    pub(super) base_url: String,
    counters: Arc<Counters>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl EmbeddingServer {
    pub(super) fn start(targets: Vec<OracleTarget>) -> Self {
        assert!(targets.len() <= DIMENSION);
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind evaluation embedding server");
        listener
            .set_nonblocking(true)
            .expect("make evaluation embedding server nonblocking");
        let base_url = format!(
            "http://{}/v1",
            listener.local_addr().expect("server address")
        );
        let counters = Arc::new(Counters::default());
        let thread_counters = Arc::clone(&counters);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("make accepted evaluation connection blocking");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .expect("set evaluation server read timeout");
                        handle_request(&mut stream, &targets, &thread_counters)
                            .expect("serve evaluation embedding request");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("evaluation embedding listener failed: {error}"),
                }
            }
        });
        Self {
            base_url,
            counters,
            stop,
            thread: Some(thread),
        }
    }

    pub(super) fn snapshot(&self) -> EmbeddingSnapshot {
        EmbeddingSnapshot {
            requests: self.counters.requests.load(Ordering::Acquire),
            document_requests: self.counters.document_requests.load(Ordering::Acquire),
            query_requests: self.counters.query_requests.load(Ordering::Acquire),
            document_inputs: self.counters.document_inputs.load(Ordering::Acquire),
            query_inputs: self.counters.query_inputs.load(Ordering::Acquire),
            input_bytes: self.counters.input_bytes.load(Ordering::Acquire),
            non_text_inputs: self.counters.non_text_inputs.load(Ordering::Acquire),
        }
    }
}

impl Drop for EmbeddingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            if !std::thread::panicking() {
                result.expect("evaluation embedding server thread");
            }
        }
    }
}

fn handle_request(
    stream: &mut TcpStream,
    targets: &[OracleTarget],
    counters: &Counters,
) -> std::io::Result<()> {
    let request = read_request(stream)?;
    let request_line = request
        .split(|byte| *byte == b'\n')
        .next()
        .and_then(|line| std::str::from_utf8(line).ok())
        .unwrap_or_default();
    if !request_line.starts_with("POST /v1/embeddings ") {
        return write_response(stream, "404 Not Found", b"{}");
    }
    let Some(body_start) = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
    else {
        return write_response(stream, "400 Bad Request", b"{}");
    };
    let body: serde_json::Value = match serde_json::from_slice(&request[body_start..]) {
        Ok(body) => body,
        Err(_) => return write_response(stream, "400 Bad Request", b"{}"),
    };
    let inputs = match body.get("input") {
        Some(serde_json::Value::Array(inputs)) => inputs
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>(),
        Some(serde_json::Value::String(input)) => vec![input.as_str()],
        _ => return write_response(stream, "400 Bad Request", b"{}"),
    };
    if inputs.is_empty() {
        return write_response(stream, "400 Bad Request", b"{}");
    }
    let query_request = inputs
        .iter()
        .all(|input| targets.iter().any(|target| input.trim() == target.query));
    counters.requests.fetch_add(1, Ordering::AcqRel);
    if query_request {
        counters.query_requests.fetch_add(1, Ordering::AcqRel);
    } else {
        counters.document_requests.fetch_add(1, Ordering::AcqRel);
    }

    let data = inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            counters
                .input_bytes
                .fetch_add(input.len(), Ordering::AcqRel);
            if query_request {
                counters.query_inputs.fetch_add(1, Ordering::AcqRel);
            } else {
                counters.document_inputs.fetch_add(1, Ordering::AcqRel);
            }
            if input.contains("NON_TEXT_ASSET_SENTINEL") {
                counters.non_text_inputs.fetch_add(1, Ordering::AcqRel);
            }
            serde_json::json!({
                "object": "embedding",
                "index": index,
                "embedding": vector(input, targets),
            })
        })
        .collect::<Vec<_>>();
    let response = serde_json::to_vec(&serde_json::json!({
        "object": "list",
        "model": "semantic-fixture-v1",
        "data": data,
        "usage": {"prompt_tokens": 0, "total_tokens": 0},
    }))
    .map_err(std::io::Error::other)?;
    write_response(stream, "200 OK", &response)
}

fn vector(text: &str, targets: &[OracleTarget]) -> Vec<f32> {
    let axis = targets
        .iter()
        .position(|target| text.trim() == target.query)
        .or_else(|| {
            targets
                .iter()
                .position(|target| text.contains(target.identifier))
        })
        .unwrap_or_else(|| targets.len() + stable_bucket(text, DIMENSION - targets.len()));
    let mut vector = vec![0.0; DIMENSION];
    vector[axis] = 1.0;
    vector
}

fn stable_bucket(text: &str, buckets: usize) -> usize {
    text.bytes().fold(0usize, |hash, byte| {
        hash.wrapping_mul(16_777_619).wrapping_add(byte as usize)
    }) % buckets
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > MAX_REQUEST_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "evaluation embedding request exceeded one MiB",
            ));
        }
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
        if content_length > MAX_REQUEST_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "evaluation embedding body exceeded one MiB",
            ));
        }
        if request.len() >= header_end + 4 + content_length {
            request.truncate(header_end + 4 + content_length);
            break;
        }
    }
    Ok(request)
}

fn write_response(stream: &mut TcpStream, status: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}
