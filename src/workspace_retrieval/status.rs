use a3s_code_core::{WorkspaceRetrievalPhase, WorkspaceRetrievalStatus};

pub(crate) fn format_workspace_retrieval_status(status: &WorkspaceRetrievalStatus) -> String {
    if status.phase == WorkspaceRetrievalPhase::Disabled {
        return "disabled".to_string();
    }
    let phase = match status.phase {
        WorkspaceRetrievalPhase::Disabled => "disabled",
        WorkspaceRetrievalPhase::Building if status.indexed_chunks > 0 => "partial",
        WorkspaceRetrievalPhase::Building => "building",
        WorkspaceRetrievalPhase::Ready => "ready",
        WorkspaceRetrievalPhase::Degraded => "degraded",
        WorkspaceRetrievalPhase::Closed => "closed",
    };
    let model = status
        .model
        .as_ref()
        .map(|model| format!("{}/{}", model.provider, model.model))
        .unwrap_or_else(|| "unavailable".to_string());
    format!(
        "{phase} | {:.2}% | {}/{} files | {} chunks | queue {} | failures {} | model {model}",
        f64::from(status.coverage_bps) / 100.0,
        status.indexed_files,
        status.eligible_files,
        status.indexed_chunks,
        status.queue_depth,
        status.total_failures,
    )
}

#[cfg(test)]
mod tests {
    use a3s_code_core::embedding::EmbeddingProviderDescriptor;

    use super::*;

    #[test]
    fn disabled_status_is_compact() {
        assert_eq!(
            format_workspace_retrieval_status(&WorkspaceRetrievalStatus::disabled()),
            "disabled"
        );
    }

    #[test]
    fn building_with_coverage_is_reported_as_partial_without_endpoint_data() {
        let mut status = WorkspaceRetrievalStatus::disabled();
        status.phase = WorkspaceRetrievalPhase::Building;
        status.eligible_files = 10;
        status.indexed_files = 4;
        status.indexed_chunks = 12;
        status.coverage_bps = 4_000;
        status.queue_depth = 6;
        status.total_failures = 1;
        status.model = Some(EmbeddingProviderDescriptor::new("local", "embed-v1", 384));

        assert_eq!(
            format_workspace_retrieval_status(&status),
            "partial | 40.00% | 4/10 files | 12 chunks | queue 6 | failures 1 | model local/embed-v1"
        );
    }
}
