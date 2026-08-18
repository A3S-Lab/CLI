use a3s_code_core::{WorkspaceRetrievalPhase, WorkspaceRetrievalStatus};

const MAX_PROVIDER_CHARS: usize = 32;
const MAX_MODEL_CHARS: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceRetrievalStatusReport {
    pub(crate) retrieval: String,
    pub(crate) vectors: Option<String>,
    pub(crate) embedding: Option<String>,
}

pub(crate) fn workspace_retrieval_status_report(
    status: &WorkspaceRetrievalStatus,
) -> WorkspaceRetrievalStatusReport {
    if status.phase == WorkspaceRetrievalPhase::Disabled {
        return WorkspaceRetrievalStatusReport {
            retrieval: "disabled".to_string(),
            vectors: None,
            embedding: None,
        };
    }

    let phase = retrieval_phase_label(status);
    let coverage_bps = status.coverage_bps.min(10_000);
    let retrieval = format!(
        "{phase} · {:.2}% · indexed {}/{} files · {} chunks · queue {} · failures {} ({} files)",
        f64::from(coverage_bps) / 100.0,
        status.indexed_files,
        status.eligible_files,
        status.indexed_chunks,
        status.queue_depth,
        status.total_failures,
        status.failed_files,
    );
    let vectors = Some(format!(
        "{} records · {} · catalog {} files / {} chunks · revisions c{} / s{} / v{}",
        status.vector_records,
        format_bytes(status.vector_bytes),
        status.catalog_files,
        status.catalog_chunks,
        status.catalog_revision,
        status.source_revision,
        status.vector_revision,
    ));
    let model = status
        .model
        .as_ref()
        .map(|model| {
            let provider = safe_label(&model.provider, MAX_PROVIDER_CHARS, "provider");
            let model_name = safe_label(&model.model, MAX_MODEL_CHARS, "model");
            format!("{provider}/{model_name} ({}d)", model.dimension)
        })
        .unwrap_or_else(|| "unavailable".to_string());
    let batching = &status.batching;
    let amplification = if batching.batch_limit_lower_bound > 0 {
        format!(
            "{:.2}×",
            batching.document_provider_requests as f64 / batching.batch_limit_lower_bound as f64
        )
    } else {
        "n/a".to_string()
    };
    let first_ready = batching
        .time_to_first_ready_ms
        .map(|milliseconds| format!("{milliseconds} ms"))
        .unwrap_or_else(|| "pending".to_string());
    let embedding = Some(format!(
        "{model} · {} inputs ({}) / {} batches · requests {} / lower bound {} / amplification {} · first ready {} · non-text {}",
        batching.document_inputs,
        format_bytes(batching.document_text_bytes),
        batching.document_batches,
        batching.document_provider_requests,
        batching.batch_limit_lower_bound,
        amplification,
        first_ready,
        batching.non_text_inputs,
    ));

    WorkspaceRetrievalStatusReport {
        retrieval,
        vectors,
        embedding,
    }
}

pub(crate) fn retrieval_phase_label(status: &WorkspaceRetrievalStatus) -> &'static str {
    match status.phase {
        WorkspaceRetrievalPhase::Disabled => "disabled",
        WorkspaceRetrievalPhase::Building if status.indexed_chunks > 0 => "partial",
        WorkspaceRetrievalPhase::Building => "building",
        WorkspaceRetrievalPhase::Ready => "ready",
        WorkspaceRetrievalPhase::Degraded => "degraded",
        WorkspaceRetrievalPhase::Closed => "closed",
    }
}

fn safe_label(value: &str, max_chars: usize, fallback: &str) -> String {
    let value = crate::system_agents::sanitize_display_text(value, max_chars);
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn format_bytes(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * KIB;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use a3s_code_core::embedding::EmbeddingProviderDescriptor;

    use super::*;

    #[test]
    fn disabled_status_is_compact() {
        assert_eq!(
            workspace_retrieval_status_report(&WorkspaceRetrievalStatus::disabled()),
            WorkspaceRetrievalStatusReport {
                retrieval: "disabled".to_string(),
                vectors: None,
                embedding: None,
            }
        );
    }

    #[test]
    fn building_with_coverage_reports_bounded_diagnostics_without_endpoint_data() {
        let mut status = WorkspaceRetrievalStatus::disabled();
        status.phase = WorkspaceRetrievalPhase::Building;
        status.eligible_files = 10;
        status.catalog_files = 10;
        status.catalog_chunks = 30;
        status.indexed_files = 4;
        status.indexed_chunks = 12;
        status.coverage_bps = 4_000;
        status.queue_depth = 6;
        status.failed_files = 1;
        status.total_failures = 1;
        status.vector_records = 12;
        status.vector_bytes = 6 * 1024;
        status.catalog_revision = 7;
        status.source_revision = 8;
        status.vector_revision = 6;
        status.batching.document_inputs = 12;
        status.batching.document_text_bytes = 4 * 1024;
        status.batching.document_batches = 3;
        status.batching.document_provider_requests = 4;
        status.batching.batch_limit_lower_bound = 2;
        status.batching.time_to_first_ready_ms = Some(18);
        status.model = Some(EmbeddingProviderDescriptor::new(
            "local\u{001b}]8;;https://secret.invalid\u{0007}",
            "embed-v1\nignored",
            384,
        ));

        let report = workspace_retrieval_status_report(&status);
        assert_eq!(
            report.retrieval,
            "partial · 40.00% · indexed 4/10 files · 12 chunks · queue 6 · failures 1 (1 files)"
        );
        assert_eq!(
            report.vectors.as_deref(),
            Some("12 records · 6.0 KiB · catalog 10 files / 30 chunks · revisions c7 / s8 / v6")
        );
        let embedding = report.embedding.expect("embedding diagnostics");
        assert!(
            embedding.contains("local/embed-v1 ignored (384d)"),
            "{embedding}"
        );
        assert!(
            embedding.contains("requests 4 / lower bound 2 / amplification 2.00×"),
            "{embedding}"
        );
        assert!(embedding.contains("first ready 18 ms"), "{embedding}");
        assert!(!embedding.contains('\u{001b}'), "{embedding:?}");
        assert!(!embedding.contains("secret.invalid"), "{embedding:?}");
    }
}
