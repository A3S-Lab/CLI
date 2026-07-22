use super::super::{ReportDocument, ReportSource, StructuralCoverage};
use std::collections::BTreeMap;

pub(super) struct RenderContext<'a> {
    pub(super) document: &'a ReportDocument,
    pub(super) labels: ReportLabels,
    claim_numbers: BTreeMap<&'a str, usize>,
}

impl<'a> RenderContext<'a> {
    pub(super) fn new(document: &'a ReportDocument) -> Self {
        let claim_numbers = document
            .direct_answer_claims
            .iter()
            .chain(
                document
                    .dimensions
                    .iter()
                    .flat_map(|dimension| dimension.claims.iter()),
            )
            .enumerate()
            .map(|(index, claim)| (claim.id.as_str(), index + 1))
            .collect();
        Self {
            document,
            labels: ReportLabels::for_language(&document.language),
            claim_numbers,
        }
    }

    pub(super) fn claim_number(&self, claim_id: &str) -> Option<usize> {
        self.claim_numbers.get(claim_id).copied()
    }

    pub(super) fn source(&self, source_id: &str) -> Option<&'a ReportSource> {
        self.document
            .source_ledger
            .iter()
            .find(|source| source.id == source_id)
    }

    pub(super) fn coverage_label(&self, coverage: StructuralCoverage) -> &'static str {
        match coverage {
            StructuralCoverage::ClaimsOnly => self.labels.coverage_claims,
            StructuralCoverage::ClaimsAndGap => self.labels.coverage_partial,
            StructuralCoverage::GapOnly => self.labels.coverage_bounded,
            StructuralCoverage::Missing => self.labels.coverage_missing,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ReportLabels {
    pub(super) lang: &'static str,
    pub(super) report_sections: &'static str,
    pub(super) direct_answer: &'static str,
    pub(super) research_dimensions: &'static str,
    pub(super) sources: &'static str,
    pub(super) status: &'static str,
    pub(super) findings: &'static str,
    pub(super) limitations: &'static str,
    pub(super) retained_excerpts: &'static str,
    pub(super) contradiction: &'static str,
    pub(super) inference: &'static str,
    pub(super) recommendation: &'static str,
    pub(super) basis: &'static str,
    pub(super) derivation: &'static str,
    pub(super) finding: &'static str,
    pub(super) captured: &'static str,
    pub(super) requested_as: &'static str,
    pub(super) source_backed: &'static str,
    pub(super) no_evidence: &'static str,
    pub(super) coverage_claims: &'static str,
    pub(super) coverage_partial: &'static str,
    pub(super) coverage_bounded: &'static str,
    pub(super) coverage_missing: &'static str,
}

impl ReportLabels {
    fn for_language(language: &str) -> Self {
        if language.eq_ignore_ascii_case("zh") || language.starts_with("zh-") {
            Self {
                lang: "zh",
                report_sections: "报告章节",
                direct_answer: "直接结论",
                research_dimensions: "研究维度",
                sources: "来源",
                status: "覆盖状态",
                findings: "研究发现",
                limitations: "证据边界",
                retained_excerpts: "保留的来源摘录",
                contradiction: "矛盾",
                inference: "推断",
                recommendation: "建议",
                basis: "依据",
                derivation: "推导",
                finding: "结论",
                captured: "获取时间",
                requested_as: "原始请求地址",
                source_backed: "来源保全报告",
                no_evidence: "未获取到可核查来源",
                coverage_claims: "已有引用结论",
                coverage_partial: "部分回答并保留证据边界",
                coverage_bounded: "仅保留证据边界",
                coverage_missing: "尚未回答",
            }
        } else {
            Self {
                lang: "en",
                report_sections: "Report sections",
                direct_answer: "Direct Answer",
                research_dimensions: "Research Dimensions",
                sources: "Sources",
                status: "Coverage",
                findings: "Findings",
                limitations: "Evidence Boundaries",
                retained_excerpts: "Retained Source Excerpts",
                contradiction: "Contradiction",
                inference: "Inference",
                recommendation: "Recommendation",
                basis: "Basis",
                derivation: "Derivation",
                finding: "finding",
                captured: "Captured",
                requested_as: "Requested as",
                source_backed: "Source-backed report",
                no_evidence: "No verifiable source was acquired",
                coverage_claims: "Addressed with cited findings",
                coverage_partial: "Partially addressed with an explicit boundary",
                coverage_bounded: "Bounded by the retained evidence",
                coverage_missing: "Not addressed",
            }
        }
    }
}
