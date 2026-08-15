use super::embedding_server::{EmbeddingRuntime, EmbeddingSnapshot};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HostRunMetric {
    pub(super) task: &'static str,
    pub(super) completion_correct: bool,
    pub(super) tool_protocol_ok: bool,
    pub(super) returned_results: usize,
    pub(super) expected_path_rank: Option<usize>,
    pub(super) algorithm: Option<String>,
    pub(super) rerank_requested_mode: Option<String>,
    pub(super) rerank_applied_mode: Option<String>,
    pub(super) elapsed_ms: u64,
    pub(super) prompt_tokens: usize,
    pub(super) completion_tokens: usize,
    pub(super) total_tokens: usize,
    pub(super) phase: String,
    pub(super) coverage_bps: usize,
    pub(super) eligible_files: usize,
    pub(super) indexed_files: usize,
    pub(super) indexed_chunks: usize,
    pub(super) failed_files: usize,
    pub(super) vector_records: usize,
    pub(super) vector_bytes: usize,
    pub(super) batching: HostBatchingMetric,
    pub(super) embedding: EmbeddingSnapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HostBatchingMetric {
    pub(super) document_inputs: usize,
    pub(super) document_batches: usize,
    pub(super) document_provider_requests: usize,
    pub(super) batch_limit_lower_bound: usize,
    pub(super) generation_complete_flushes: usize,
    pub(super) time_to_first_ready_ms: Option<u64>,
    pub(super) non_text_inputs: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HostEvaluationSummary {
    pub(super) task_accuracy: f64,
    pub(super) tool_protocol_rate: f64,
    pub(super) precision_at_5: f64,
    pub(super) observed_result_precision: f64,
    pub(super) mean_returned_results: f64,
    pub(super) recall_at_5: f64,
    pub(super) mean_reciprocal_rank: f64,
    pub(super) ndcg_at_5: f64,
    pub(super) mean_relevant_rank: f64,
    pub(super) elapsed_p50_ms: u64,
    pub(super) elapsed_p95_ms: u64,
    pub(super) total_model_tokens: usize,
    pub(super) document_batches: usize,
    pub(super) document_provider_requests: usize,
    pub(super) document_batch_limit_lower_bound: usize,
    pub(super) document_request_amplification: f64,
    pub(super) time_to_first_ready_p50_ms: u64,
    pub(super) time_to_first_ready_p95_ms: u64,
    pub(super) non_text_provider_inputs: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HostEvaluationReport {
    pub(super) schema_version: u32,
    pub(super) chat_model: String,
    pub(super) embedding_kind: String,
    pub(super) embedding_model: String,
    pub(super) embedding_revision: String,
    pub(super) embedding_dimension: usize,
    pub(super) embedding_runtime: Option<EmbeddingRuntime>,
    pub(super) config_layers: usize,
    pub(super) chunking_strategy: &'static str,
    pub(super) rerank_algorithm: String,
    pub(super) text_file_count: usize,
    pub(super) non_text_file_count: usize,
    pub(super) expected_chunk_count: usize,
    pub(super) summary: HostEvaluationSummary,
    pub(super) runs: Vec<HostRunMetric>,
}

pub(super) fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

pub(super) fn percentile(mut values: Vec<u64>, quantile: f64) -> u64 {
    values.sort_unstable();
    let index = ((values.len() - 1) as f64 * quantile).ceil() as usize;
    values[index]
}
