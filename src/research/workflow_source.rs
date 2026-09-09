//! Host-side DeepResearch workflow source compatibility.

use a3s_deep_research::engine::DeepResearchRequest;
use serde_json::Value;

/// PTC source used by Code DeepResearch. The workflow function is deterministic
/// and only schedules work; side effects live in Flow steps.
///
/// DeepResearch 0.1.4 still parses Core `batch` sections with the pre-staged
/// header `--- [N: label] ---`. Core now emits
/// `--- [N / step S: label] ---`, which made `batchSections` fail closed,
/// set `output_truncated`, and re-issue the same `web_search` (duplicate
/// queries under `direct_searches=1`). Patch only that header matcher until
/// DeepResearch ships a compatible release.
#[cfg(test)]
pub(crate) fn patched_retrieval_workflow_source() -> &'static str {
    use std::sync::OnceLock;
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE
        .get_or_init(|| {
            patch_deep_research_batch_section_headers(
                a3s_deep_research::workflow::retrieval_workflow_source(),
            )
        })
        .as_str()
}

/// Compile a DeepResearch request into workflow arguments with the host-side
/// Core batch-header compatibility patch applied to `source`.
pub(crate) fn code_deep_research_workflow_arguments(
    request: &DeepResearchRequest,
) -> Result<Value, String> {
    let mut arguments = request.to_workflow_arguments()?;
    apply_patched_retrieval_workflow_source(&mut arguments);
    Ok(arguments)
}

/// Patch `arguments.source` in place when it still uses the DeepResearch 0.1.4
/// legacy batch-header matcher. Never replace the whole source string: Host
/// fixtures and staged args may already rewrite tool names while keeping the
/// same PTC body.
pub(crate) fn apply_patched_retrieval_workflow_source(arguments: &mut Value) {
    let Some(source) = arguments.get("source").and_then(Value::as_str) else {
        return;
    };
    if !source.contains(LEGACY_BATCH_HEADER) {
        return;
    }
    arguments["source"] = Value::String(patch_deep_research_batch_section_headers(source));
}

const LEGACY_BATCH_HEADER: &str =
    "const header = `--- [${position + 1}: ${label}] ---\\n`;";
const STAGED_BATCH_HEADER: &str = "const step = Number(metadata.step);\n\
      const header = Number.isInteger(step) && step > 0\n\
        ? `--- [${position + 1} / step ${step}: ${label}] ---\\n`\n\
        : `--- [${position + 1}: ${label}] ---\\n`;";

fn patch_deep_research_batch_section_headers(source: &str) -> String {
    assert!(
        source.contains(LEGACY_BATCH_HEADER),
        "a3s-deep-research workflow no longer contains the legacy batch header matcher; drop the CLI patch"
    );
    source.replacen(LEGACY_BATCH_HEADER, STAGED_BATCH_HEADER, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_deep_research::engine::EvidenceScope;

    #[test]
    fn deep_research_batch_header_patch_is_surgical() {
        let patched = patch_deep_research_batch_section_headers(
            "prefix\nconst header = `--- [${position + 1}: ${label}] ---\\n`;\nsuffix",
        );
        assert!(patched.contains("/ step ${step}: ${label}"));
        assert!(patched.starts_with("prefix\n"));
        assert!(patched.ends_with("\nsuffix"));
        assert!(!patched.contains("const header = `--- [${position + 1}: ${label}] ---\\n`;"));
    }

    #[test]
    fn production_workflow_arguments_include_staged_batch_header_patch() {
        let request = DeepResearchRequest::new(
            "patch-contract",
            "Audit staged batch headers",
            EvidenceScope::WebAndWorkspace,
        );
        let arguments = code_deep_research_workflow_arguments(&request)
            .expect("workflow arguments should compile");
        let source = arguments["source"]
            .as_str()
            .expect("workflow source must be a string");
        let upstream = a3s_deep_research::workflow::retrieval_workflow_source();
        assert_ne!(source, upstream);
        assert!(source.contains("/ step ${step}: ${label}"));
        assert!(source.contains("--- [${position + 1}: ${label}]"));
        assert!(!source.contains(LEGACY_BATCH_HEADER));
    }

    #[test]
    fn apply_patch_preserves_fixture_tool_rewrites() {
        let mut arguments = serde_json::json!({
            "source": format!(
                "prefix\n{LEGACY_BATCH_HEADER}\nsuffix\nctx.tool(\"evidence_first_fixture_search\")"
            ),
        });
        apply_patched_retrieval_workflow_source(&mut arguments);
        let source = arguments["source"].as_str().expect("source");
        assert!(source.contains("/ step ${step}: ${label}"));
        assert!(source.contains("evidence_first_fixture_search"));
        assert!(!source.contains(LEGACY_BATCH_HEADER));

        // Already-patched sources must be left alone (including fixture rewrites).
        let before = source.to_owned();
        apply_patched_retrieval_workflow_source(&mut arguments);
        assert_eq!(arguments["source"].as_str(), Some(before.as_str()));
    }
}
