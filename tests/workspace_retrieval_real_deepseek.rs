//! Real DeepSeek ACL-host evaluation for session-local workspace retrieval.
//!
//! The discovered workspace ACL supplies only the real chat route. A temporary
//! trusted user ACL enables retrieval and points embeddings at a process-local
//! server. The default is a deterministic oracle; setting the real-embedding
//! environment selects a revision-locked Sentence Transformers worker behind
//! the same OpenAI-compatible HTTP boundary. Repository credentials are
//! neither copied nor printed. Run serially with `A3S_REAL_EVAL_ROOT` set.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

use serde_json::Value;

#[path = "workspace_retrieval_real_deepseek/embedding_server.rs"]
mod embedding_server;
use embedding_server::{EmbeddingServer, EmbeddingSnapshot, OracleTarget};
#[path = "workspace_retrieval_real_deepseek/fixture.rs"]
mod fixture;
use fixture::{
    write_fixture, write_trusted_user_config_with, write_trusted_user_local_cpu_config,
    EvaluationTask, TrustedRetrievalConfig, EXPECTED_CHUNK_COUNT, NON_TEXT_FILE_COUNT, TASKS,
    TEST_API_KEY, TEXT_FILE_COUNT,
};
#[path = "workspace_retrieval_real_deepseek/report.rs"]
mod report;
use report::{
    percentile, ratio, HostBatchingMetric, HostEvaluationReport, HostEvaluationSummary,
    HostRunMetric,
};

fn a3s_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_a3s"))
}

fn evaluation_root() -> PathBuf {
    let root = std::env::var_os("A3S_REAL_EVAL_ROOT")
        .map(PathBuf::from)
        .expect("set A3S_REAL_EVAL_ROOT to the monorepo root containing .a3s/config.acl");
    let root = root
        .canonicalize()
        .expect("canonicalize A3S real evaluation root");
    assert!(
        root.join(".a3s/config.acl").is_file(),
        "A3S_REAL_EVAL_ROOT must contain .a3s/config.acl"
    );
    root
}

fn configured_command(workspace: &Path, home: &Path) -> Command {
    let mut command = Command::new(a3s_binary());
    command
        .env("HOME", home)
        .env("A3S_DATA_HOME", home.join("data"))
        .env("A3S_STATE_HOME", home.join("state"))
        .env("A3S_CACHE_HOME", home.join("cache"))
        .env("A3S_NO_AUTO_INSTALL", "true")
        .env_remove("A3S_CONFIG_FILE")
        .env_remove("A3S_OFFLINE")
        .arg("--directory")
        .arg(workspace)
        .arg("--no-progress");
    command
}

fn assert_success(output: &Output, operation: &str) {
    assert!(
        output.status.success(),
        "{operation} failed with status {:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_redacted(output: &Output, sensitive_values: &[String]) {
    for rendered in [&output.stdout, &output.stderr] {
        let rendered = String::from_utf8_lossy(rendered);
        assert!(!rendered.contains(TEST_API_KEY), "test API key leaked");
        for sensitive in sensitive_values {
            assert!(
                !rendered.contains(sensitive),
                "configured embedding detail leaked"
            );
        }
    }
}

fn config_show(workspace: &Path, home: &Path, sensitive_values: &[String]) -> (Value, String) {
    let output = configured_command(workspace, home)
        .args(["--output", "json", "config", "show"])
        .output()
        .expect("run layered host evaluation config show");
    assert_success(&output, "layered config show");
    assert_redacted(&output, sensitive_values);
    let rendered = String::from_utf8(output.stdout).expect("UTF-8 config show output");
    let value = serde_json::from_str(&rendered).expect("parse config show JSON");
    (value, rendered)
}

fn run_task(
    workspace: &Path,
    home: &Path,
    sensitive_values: &[String],
    task: EvaluationTask,
) -> (Vec<Value>, u64) {
    let prompt = format!(
        "Inspect the search tool schema. Make exactly one search call and no other tool call. Use query exactly: {query}. Set path to '.', include to '*.rs', limit to 5, and mode to 'hybrid'. After the result, return exactly the Rust function or constant declaration name that directly answers the query and is supported by the evidence, or NOT_FOUND when no relevant declaration is present. Never return a path, file stem, module name, prose, or Markdown.",
        query = task.query,
    );
    let started = Instant::now();
    let output = configured_command(workspace, home)
        .args([
            "--output",
            "jsonl",
            "--non-interactive",
            "code",
            "exec",
            "--mode",
            "default",
            "--tool-policy",
            "read-only",
            &prompt,
        ])
        .output()
        .expect("run real DeepSeek ACL-host task");
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    assert_success(&output, task.name);
    assert_redacted(&output, sensitive_values);
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 code exec JSONL");
    let documents = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("parse code exec JSONL document"))
        .collect::<Vec<_>>();
    (documents, elapsed_ms)
}

fn metric_from_documents(
    task: EvaluationTask,
    documents: &[Value],
    elapsed_ms: u64,
    provider_embedding: Option<EmbeddingSnapshot>,
) -> HostRunMetric {
    let tool_events = documents
        .iter()
        .filter(|document| {
            document.get("type").and_then(Value::as_str) == Some("event")
                && document
                    .get("event")
                    .and_then(|event| event.get("type"))
                    .and_then(Value::as_str)
                    == Some("tool_end")
        })
        .collect::<Vec<_>>();
    let tool = tool_events
        .first()
        .and_then(|document| document.get("event"));
    let args = tool.and_then(|event| event.get("args"));
    let tool_protocol_ok = tool_events.len() == 1
        && tool
            .and_then(|event| event.get("name"))
            .and_then(Value::as_str)
            == Some("search")
        && tool
            .and_then(|event| event.get("exit_code"))
            .and_then(Value::as_i64)
            == Some(0)
        && args
            .and_then(|args| args.get("query"))
            .and_then(Value::as_str)
            == Some(task.query)
        && args
            .and_then(|args| args.get("mode"))
            .and_then(Value::as_str)
            == Some("hybrid")
        && args
            .and_then(|args| args.get("path"))
            .and_then(Value::as_str)
            == Some(".")
        && args
            .and_then(|args| args.get("include"))
            .and_then(Value::as_str)
            == Some("*.rs")
        && args
            .and_then(|args| args.get("limit"))
            .and_then(Value::as_u64)
            == Some(5);
    let metadata = tool.and_then(|event| event.get("metadata"));
    let results = metadata
        .and_then(|metadata| metadata.get("results"))
        .and_then(Value::as_array);
    let expected_path_rank = results.and_then(|results| {
        results.iter().position(|result| {
            result.get("path").and_then(Value::as_str) == Some(task.expected_path)
        })
    });
    let expected_path_rank = expected_path_rank.map(|rank| rank + 1);
    let rerank = metadata.and_then(|metadata| metadata.get("rerank"));
    let result = documents
        .iter()
        .find(|document| document.get("type").and_then(Value::as_str) == Some("result"))
        .expect("terminal code exec result");
    let data = result.get("data").expect("code exec result data");
    let text = data.get("text").and_then(Value::as_str).unwrap_or_default();
    let normalized = text.trim().trim_matches('`').trim();
    let usage = data.get("usage").unwrap_or(&Value::Null);
    let status = data
        .get("workspaceRetrieval")
        .expect("workspace retrieval status");
    let batching = status.get("batching").expect("embedding batching status");
    let batching_metric = HostBatchingMetric {
        document_inputs: json_usize(batching, "documentInputs"),
        document_batches: json_usize(batching, "documentBatches"),
        document_provider_requests: json_usize(batching, "documentProviderRequests"),
        batch_limit_lower_bound: json_usize(batching, "batchLimitLowerBound"),
        generation_complete_flushes: json_usize(batching, "generationCompleteFlushes"),
        time_to_first_ready_ms: batching.get("timeToFirstReadyMs").and_then(Value::as_u64),
        non_text_inputs: json_usize(batching, "nonTextInputs"),
    };
    let embedding = provider_embedding.unwrap_or_else(|| {
        let query_requests = usize::from(tool_protocol_ok);
        EmbeddingSnapshot {
            requests: batching_metric.document_provider_requests + query_requests,
            document_requests: batching_metric.document_provider_requests,
            query_requests,
            document_inputs: batching_metric.document_inputs,
            query_inputs: query_requests,
            input_bytes: 0,
            non_text_inputs: batching_metric.non_text_inputs,
        }
    });
    HostRunMetric {
        task: task.name,
        completion_correct: normalized == task.expected_identifier,
        tool_protocol_ok,
        returned_results: results.map_or(0, Vec::len),
        expected_path_rank,
        exact_candidates: channel_candidate_count(metadata, "exact"),
        lexical_candidates: channel_candidate_count(metadata, "lexical"),
        structural_candidates: channel_candidate_count(metadata, "structural"),
        semantic_candidates: channel_candidate_count(metadata, "semantic"),
        fallback: metadata
            .and_then(|metadata| metadata.get("fallback"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        algorithm: metadata
            .and_then(|metadata| metadata.get("algorithm"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        rerank_requested_mode: rerank
            .and_then(|rerank| rerank.get("requestedMode"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        rerank_applied_mode: rerank
            .and_then(|rerank| rerank.get("appliedMode"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        rerank_input_candidates: json_usize(rerank.unwrap_or(&Value::Null), "inputCandidates"),
        rerank_evaluated_candidates: json_usize(
            rerank.unwrap_or(&Value::Null),
            "evaluatedCandidates",
        ),
        rerank_selected_candidates: json_usize(
            rerank.unwrap_or(&Value::Null),
            "selectedCandidates",
        ),
        elapsed_ms,
        prompt_tokens: json_usize(usage, "prompt_tokens"),
        completion_tokens: json_usize(usage, "completion_tokens"),
        total_tokens: json_usize(usage, "total_tokens"),
        phase: status
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("<missing>")
            .to_owned(),
        coverage_bps: json_usize(status, "coverageBps"),
        eligible_files: json_usize(status, "eligibleFiles"),
        indexed_files: json_usize(status, "indexedFiles"),
        indexed_chunks: json_usize(status, "indexedChunks"),
        failed_files: json_usize(status, "failedFiles"),
        vector_records: json_usize(status, "vectorRecords"),
        vector_bytes: json_usize(status, "vectorBytes"),
        batching: batching_metric,
        embedding,
    }
}

fn json_usize(value: &Value, field: &str) -> usize {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn channel_candidate_count(metadata: Option<&Value>, expected: &str) -> usize {
    metadata
        .and_then(|metadata| metadata.get("channels"))
        .and_then(Value::as_array)
        .and_then(|channels| {
            channels
                .iter()
                .find(|channel| channel.get("channel").and_then(Value::as_str) == Some(expected))
        })
        .map(|channel| json_usize(channel, "candidateCount"))
        .unwrap_or(0)
}

struct EvaluationEmbedding {
    server: Option<EmbeddingServer>,
    kind: &'static str,
    model: String,
    revision: String,
    dimension: usize,
    runtime: Option<embedding_server::EmbeddingRuntime>,
    deterministic_reranker: bool,
    telemetry_source: &'static str,
    sensitive_values: Vec<String>,
}

fn local_manifest_metadata(path: &Path) -> (String, String, usize) {
    let source = std::fs::read_to_string(path).expect("read local CPU model manifest");
    let document = a3s_acl::parse_acl(&source).expect("parse local CPU model manifest");
    assert_eq!(document.blocks.len(), 1);
    let block = &document.blocks[0];
    assert_eq!(block.name, "local_embedding_model");
    let string = |field: &str| {
        block.attributes[field]
            .as_str()
            .unwrap_or_else(|| panic!("local CPU model manifest {field} must be a string"))
            .to_owned()
    };
    let dimension = block.attributes["dimension"]
        .as_number()
        .expect("local CPU model manifest dimension must be numeric") as usize;
    (string("model"), string("revision"), dimension)
}

#[test]
#[ignore = "requires A3S_REAL_EVAL_ROOT, repository DeepSeek credentials, and network access"]
fn real_deepseek_acl_host_executes_recursive_reranked_workspace_tasks() {
    let root = evaluation_root();
    let workspace = tempfile::Builder::new()
        .prefix(".a3s-wsr-host-eval-")
        .tempdir_in(&root)
        .expect("create host evaluation workspace below config root");
    let home = tempfile::tempdir().expect("create host evaluation home");
    write_fixture(workspace.path());
    let embedding = if let Some(path) = std::env::var_os("A3S_LOCAL_CPU_MODEL_MANIFEST") {
        let path = PathBuf::from(path)
            .canonicalize()
            .expect("canonicalize A3S_LOCAL_CPU_MODEL_MANIFEST");
        let (model, revision, dimension) = local_manifest_metadata(&path);
        write_trusted_user_local_cpu_config(home.path(), &path);
        let rendered = path.to_string_lossy().into_owned();
        let portable = rendered.replace('\\', "/");
        let mut sensitive_values = vec![rendered];
        if portable != sensitive_values[0] {
            sensitive_values.push(portable);
        }
        EvaluationEmbedding {
            server: None,
            kind: "local_cpu_fastembed_onnx",
            model,
            revision,
            dimension,
            runtime: None,
            deterministic_reranker: true,
            telemetry_source: "host_batching_status",
            sensitive_values,
        }
    } else {
        let targets = TASKS
            .iter()
            .map(|task| OracleTarget {
                query: task.query,
                identifier: task.expected_identifier,
            })
            .collect::<Vec<_>>();
        let real_model = std::env::var("A3S_REAL_EMBEDDING_MODEL").ok();
        let (server, kind, deterministic_reranker) = if let Some(model) = real_model {
            let revision = std::env::var("A3S_REAL_EMBEDDING_REVISION")
                .expect("set A3S_REAL_EMBEDDING_REVISION with the real model");
            (
                EmbeddingServer::start_sentence_transformer(
                    targets,
                    model,
                    revision,
                    std::env::var("A3S_REAL_EMBEDDING_LOCAL_ONLY").as_deref() == Ok("1"),
                ),
                "sentence_transformers",
                false,
            )
        } else {
            (
                EmbeddingServer::start(targets),
                "deterministic_oracle",
                true,
            )
        };
        let configured_embedding_model = format!(
            "host-eval/{}",
            server.model.rsplit('/').next().unwrap_or("embed-v1")
        );
        write_trusted_user_config_with(
            home.path(),
            &server.base_url,
            TrustedRetrievalConfig {
                model: &configured_embedding_model,
                dimension: server.dimension,
                revision: &server.revision,
                deterministic_reranker,
            },
        );
        EvaluationEmbedding {
            kind,
            model: server.model.clone(),
            revision: server.revision.clone(),
            dimension: server.dimension,
            runtime: server.runtime.clone(),
            deterministic_reranker,
            telemetry_source: "provider_http_boundary",
            sensitive_values: vec![server.base_url.clone()],
            server: Some(server),
        }
    };
    let expected_algorithm = if embedding.deterministic_reranker {
        "rrf_k60+deterministic_mmr_v1"
    } else {
        "rrf_k60"
    };
    let expected_rerank_mode = if embedding.deterministic_reranker {
        "deterministic"
    } else {
        "rrf_only"
    };

    let (shown, _) = config_show(workspace.path(), home.path(), &embedding.sensitive_values);
    let data = &shown["data"];
    let chat_model = data["defaultModel"]
        .as_str()
        .expect("layered default model")
        .to_owned();
    assert!(chat_model.starts_with("deepseek/"), "{chat_model}");
    assert_eq!(data["explicit"], false);
    assert_eq!(data["layers"].as_array().map(Vec::len), Some(2));
    let retrieval = &data["workspaceRetrieval"];
    assert_eq!(retrieval["enabled"], true);
    assert_eq!(retrieval["semanticReadinessTimeoutMs"], 30_000);
    let local_cpu = embedding.server.is_none();
    let max_embedding_batch_inputs = json_usize(retrieval, "maxEmbeddingBatchInputs");
    assert!(max_embedding_batch_inputs > 0);
    let expected_document_batches = EXPECTED_CHUNK_COUNT.div_ceil(max_embedding_batch_inputs);
    assert_eq!(retrieval["sourceEgressAuthorized"], !local_cpu);
    assert_eq!(
        retrieval["backend"],
        if local_cpu {
            "local_cpu"
        } else {
            "openai_compatible"
        }
    );
    if local_cpu {
        assert_eq!(retrieval["localCpuAvailable"], true);
        assert_eq!(
            retrieval["localCpuUnavailableReason"],
            serde_json::Value::Null
        );
        assert_eq!(max_embedding_batch_inputs, 2);
    }
    assert_eq!(retrieval["chunking"]["strategy"], "recursive");
    assert_eq!(retrieval["chunking"]["targetBytes"], 512);
    assert_eq!(retrieval["chunking"]["overlapBytes"], 64);
    assert_eq!(
        retrieval["rerank"]["active"],
        embedding.deterministic_reranker
    );
    assert_eq!(retrieval["rerank"]["algorithm"], expected_algorithm);

    let mut runs = Vec::with_capacity(TASKS.len());
    for task in TASKS {
        let before = embedding.server.as_ref().map(EmbeddingServer::snapshot);
        let (documents, elapsed_ms) = run_task(
            workspace.path(),
            home.path(),
            &embedding.sensitive_values,
            task,
        );
        let provider_embedding = embedding
            .server
            .as_ref()
            .zip(before)
            .map(|(server, before)| server.snapshot().difference(before));
        runs.push(metric_from_documents(
            task,
            &documents,
            elapsed_ms,
            provider_embedding,
        ));
    }
    println!(
        "WSR_DEEPSEEK_ACL_HOST_RUNS={}",
        serde_json::to_string(&runs).expect("serialize ACL-host run metrics")
    );

    for run in &runs {
        assert!(run.completion_correct, "{run:#?}");
        assert!(run.tool_protocol_ok, "{run:#?}");
        assert!(
            run.expected_path_rank.is_some_and(|rank| rank <= 5),
            "{run:#?}"
        );
        assert_eq!(
            run.algorithm.as_deref(),
            Some(expected_algorithm),
            "{run:#?}"
        );
        assert_eq!(
            run.rerank_requested_mode.as_deref(),
            Some(expected_rerank_mode)
        );
        assert_eq!(
            run.rerank_applied_mode.as_deref(),
            Some(expected_rerank_mode)
        );
        assert_eq!(run.phase, "ready", "{run:#?}");
        assert_eq!(run.coverage_bps, 10_000, "{run:#?}");
        assert_eq!(run.eligible_files, TEXT_FILE_COUNT, "{run:#?}");
        assert_eq!(run.indexed_files, TEXT_FILE_COUNT, "{run:#?}");
        assert_eq!(run.indexed_chunks, EXPECTED_CHUNK_COUNT, "{run:#?}");
        assert_eq!(run.failed_files, 0, "{run:#?}");
        assert_eq!(run.vector_records, EXPECTED_CHUNK_COUNT, "{run:#?}");
        assert_eq!(
            run.embedding.requests,
            expected_document_batches + 1,
            "{run:#?}"
        );
        assert_eq!(
            run.embedding.document_requests, expected_document_batches,
            "{run:#?}"
        );
        assert_eq!(run.embedding.query_requests, 1, "{run:#?}");
        assert_eq!(
            run.embedding.document_inputs, EXPECTED_CHUNK_COUNT,
            "{run:#?}"
        );
        assert_eq!(run.embedding.query_inputs, 1, "{run:#?}");
        assert_eq!(run.embedding.non_text_inputs, 0, "{run:#?}");
        assert_eq!(
            run.batching.document_inputs, EXPECTED_CHUNK_COUNT,
            "{run:#?}"
        );
        assert_eq!(
            run.batching.document_batches, expected_document_batches,
            "{run:#?}"
        );
        assert_eq!(
            run.batching.document_provider_requests, expected_document_batches,
            "{run:#?}"
        );
        assert_eq!(
            run.batching.batch_limit_lower_bound, expected_document_batches,
            "{run:#?}"
        );
        assert_eq!(run.batching.generation_complete_flushes, 1, "{run:#?}");
        assert!(run.batching.time_to_first_ready_ms.is_some(), "{run:#?}");
        assert_eq!(run.batching.non_text_inputs, 0, "{run:#?}");
        assert_eq!(
            run.embedding.document_requests, run.batching.document_provider_requests,
            "{run:#?}"
        );
    }

    let ranks = runs
        .iter()
        .filter_map(|run| run.expected_path_rank)
        .collect::<Vec<_>>();
    let total_document_requests = runs
        .iter()
        .map(|run| run.embedding.document_requests)
        .sum::<usize>();
    let total_document_batches = runs
        .iter()
        .map(|run| run.batching.document_batches)
        .sum::<usize>();
    let total_batch_limit_lower_bound = runs
        .iter()
        .map(|run| run.batching.batch_limit_lower_bound)
        .sum::<usize>();
    assert!(ratio(total_document_requests, total_batch_limit_lower_bound) <= 1.10);
    let relevant_hits = ranks.iter().filter(|rank| **rank <= 5).count();
    let total_returned_results = runs.iter().map(|run| run.returned_results).sum::<usize>();
    let summary = HostEvaluationSummary {
        task_accuracy: ratio(
            runs.iter().filter(|run| run.completion_correct).count(),
            TASKS.len(),
        ),
        tool_protocol_rate: ratio(
            runs.iter().filter(|run| run.tool_protocol_ok).count(),
            TASKS.len(),
        ),
        precision_at_5: ratio(relevant_hits, TASKS.len() * 5),
        observed_result_precision: ratio(relevant_hits, total_returned_results),
        mean_returned_results: ratio(total_returned_results, TASKS.len()),
        recall_at_5: ratio(relevant_hits, TASKS.len()),
        mean_reciprocal_rank: ranks.iter().map(|rank| 1.0 / *rank as f64).sum::<f64>()
            / TASKS.len() as f64,
        ndcg_at_5: ranks
            .iter()
            .filter(|rank| **rank <= 5)
            .map(|rank| 1.0 / (*rank as f64 + 1.0).log2())
            .sum::<f64>()
            / TASKS.len() as f64,
        mean_relevant_rank: ranks.iter().sum::<usize>() as f64 / ranks.len() as f64,
        elapsed_p50_ms: percentile(runs.iter().map(|run| run.elapsed_ms).collect(), 0.50),
        elapsed_p95_ms: percentile(runs.iter().map(|run| run.elapsed_ms).collect(), 0.95),
        total_model_tokens: runs.iter().map(|run| run.total_tokens).sum(),
        document_batches: total_document_batches,
        document_provider_requests: total_document_requests,
        document_batch_limit_lower_bound: total_batch_limit_lower_bound,
        document_request_amplification: ratio(
            total_document_requests,
            total_batch_limit_lower_bound,
        ),
        time_to_first_ready_p50_ms: percentile(
            runs.iter()
                .filter_map(|run| run.batching.time_to_first_ready_ms)
                .collect(),
            0.50,
        ),
        time_to_first_ready_p95_ms: percentile(
            runs.iter()
                .filter_map(|run| run.batching.time_to_first_ready_ms)
                .collect(),
            0.95,
        ),
        non_text_provider_inputs: runs.iter().map(|run| run.embedding.non_text_inputs).sum(),
    };
    let report = HostEvaluationReport {
        schema_version: 4,
        chat_model,
        embedding_kind: embedding.kind.to_owned(),
        embedding_model: embedding.model,
        embedding_revision: embedding.revision,
        embedding_dimension: embedding.dimension,
        embedding_runtime: embedding.runtime,
        embedding_telemetry_source: embedding.telemetry_source,
        config_layers: 2,
        chunking_strategy: "recursive_512_64_explicit",
        rerank_algorithm: expected_algorithm.to_owned(),
        text_file_count: TEXT_FILE_COUNT,
        non_text_file_count: NON_TEXT_FILE_COUNT,
        expected_chunk_count: EXPECTED_CHUNK_COUNT,
        summary,
        runs,
    };
    println!(
        "WSR_DEEPSEEK_ACL_HOST_EVAL={}",
        serde_json::to_string(&report).expect("serialize ACL-host evaluation report")
    );
}
