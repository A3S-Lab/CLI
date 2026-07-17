//! Closed-evidence resolution and deterministic section audit.

use std::collections::{BTreeMap, BTreeSet};

use a3s::research::OutlineSection;

use super::super::deep_research_evidence_ledger::AcceptedSource;
use super::super::AcceptedEvidence;
use super::SectionGeneration;

#[derive(Default)]
pub(super) struct UsedEvidenceCatalog {
    pub(super) claim_ids: BTreeSet<String>,
    pub(super) source_ids: BTreeSet<String>,
}

impl UsedEvidenceCatalog {
    pub(super) fn record(&mut self, evidence: &ResolvedEvidence) {
        self.claim_ids.extend(evidence.claim_ids.iter().cloned());
        self.source_ids.extend(evidence.source_ids.iter().cloned());
    }
}

#[derive(Debug)]
pub(super) struct ResolvedEvidence {
    pub(super) claim_ids: BTreeSet<String>,
    pub(super) source_ids: BTreeSet<String>,
    pub(super) claim_texts: Vec<String>,
    pub(super) source_anchors: Vec<String>,
}

pub(super) fn validate_section_obligation_coverage(
    section: &SectionGeneration,
    planned: &OutlineSection,
) -> Result<(), String> {
    let expected_claim_ids = planned.claim_ids.iter().cloned().collect::<BTreeSet<_>>();
    let expected_source_ids = planned.source_ids.iter().cloned().collect::<BTreeSet<_>>();
    let declared_claim_ids = section.claim_ids.iter().cloned().collect::<BTreeSet<_>>();
    let declared_source_ids = section.source_ids.iter().cloned().collect::<BTreeSet<_>>();
    if declared_claim_ids != expected_claim_ids {
        let missing = expected_claim_ids
            .difference(&declared_claim_ids)
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = declared_claim_ids
            .difference(&expected_claim_ids)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "section `{}` did not satisfy its committed claim obligations (missing: {}; unexpected: {})",
            section.section_id,
            missing.join(", "),
            unexpected.join(", ")
        ));
    }
    if declared_source_ids != expected_source_ids {
        let missing = expected_source_ids
            .difference(&declared_source_ids)
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = declared_source_ids
            .difference(&expected_source_ids)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "section `{}` did not satisfy its committed source obligations (missing: {}; unexpected: {})",
            section.section_id,
            missing.join(", "),
            unexpected.join(", ")
        ));
    }
    Ok(())
}

pub(super) fn audit_section_generation(
    section: &SectionGeneration,
    evidence: &[AcceptedEvidence],
) -> Result<ResolvedEvidence, String> {
    let claim_ids = section.claim_ids.iter().cloned().collect::<BTreeSet<_>>();
    let source_ids = section.source_ids.iter().cloned().collect::<BTreeSet<_>>();
    if claim_ids.len() != section.claim_ids.len() {
        return Err(format!(
            "section `{}` declared duplicate claim IDs",
            section.section_id
        ));
    }
    if source_ids.len() != section.source_ids.len() {
        return Err(format!(
            "section `{}` declared duplicate source IDs",
            section.section_id
        ));
    }
    let resolved = resolve_evidence_ids(&claim_ids, &source_ids, evidence)?;
    let audit = super::super::deep_research_report_audit::audit_report(
        &section.markdown,
        "",
        &resolved.claim_texts,
        &resolved.source_anchors,
    );
    if !audit.passed {
        return Err(format!(
            "section `{}` failed evidence audit: {}",
            section.section_id, audit.reason
        ));
    }
    validate_declared_evidence_usage(section, &claim_ids, &source_ids, evidence)?;
    Ok(resolved)
}

fn validate_declared_evidence_usage(
    section: &SectionGeneration,
    claim_ids: &BTreeSet<String>,
    source_ids: &BTreeSet<String>,
    evidence: &[AcceptedEvidence],
) -> Result<(), String> {
    for claim_id in claim_ids {
        let claim_text = evidence
            .iter()
            .flat_map(|item| &item.claims)
            .find(|claim| claim.id == *claim_id)
            .map(|claim| claim.text.clone())
            .ok_or_else(|| format!("declared claim ID `{claim_id}` is not accepted evidence"))?;
        let linked_anchors = evidence
            .iter()
            .filter(|item| item.claims.iter().any(|claim| claim.id == *claim_id))
            .flat_map(|item| &item.sources)
            .filter(|source| source_ids.contains(&source.id))
            .map(|source| source.anchor.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let claim_audit = super::super::deep_research_report_audit::audit_report(
            &section.markdown,
            "",
            &[claim_text],
            &linked_anchors,
        );
        if !claim_audit.passed {
            return Err(format!(
                "section `{}` claim ID `{claim_id}` is not covered with an inline source from the same accepted evidence item: {}",
                section.section_id, claim_audit.reason
            ));
        }
    }

    for source_id in source_ids {
        let anchor = evidence
            .iter()
            .flat_map(|item| &item.sources)
            .find(|source| source.id == *source_id)
            .map(|source| source.anchor.as_str())
            .ok_or_else(|| format!("declared source ID `{source_id}` is not accepted evidence"))?;
        if !super::super::deep_research_report_audit::cites_source_anchor(
            &section.markdown,
            "",
            anchor,
        ) {
            return Err(format!(
                "section `{}` declared source ID `{source_id}` but did not cite its anchor inline",
                section.section_id
            ));
        }
    }
    Ok(())
}

pub(super) fn resolve_evidence_ids(
    claim_ids: &BTreeSet<String>,
    source_ids: &BTreeSet<String>,
    evidence: &[AcceptedEvidence],
) -> Result<ResolvedEvidence, String> {
    if claim_ids.is_empty() || source_ids.is_empty() {
        return Err("section evidence declarations require claim and source IDs".to_string());
    }

    let mut claims_by_id = BTreeMap::<&str, &str>::new();
    for claim in evidence.iter().flat_map(|item| &item.claims) {
        match claims_by_id.get(claim.id.as_str()) {
            Some(text) if *text != claim.text => {
                return Err(format!(
                    "accepted evidence claim ID `{}` resolves to conflicting texts",
                    claim.id
                ));
            }
            Some(_) => {}
            None => {
                claims_by_id.insert(&claim.id, &claim.text);
            }
        }
    }
    let mut sources_by_id = BTreeMap::<&str, &str>::new();
    for source in evidence.iter().flat_map(|item| &item.sources) {
        match sources_by_id.get(source.id.as_str()) {
            Some(anchor) if *anchor != source.anchor => {
                return Err(format!(
                    "accepted evidence source ID `{}` resolves to conflicting anchors",
                    source.id
                ));
            }
            Some(_) => {}
            None => {
                sources_by_id.insert(&source.id, &source.anchor);
            }
        }
    }

    let claim_texts = claim_ids
        .iter()
        .map(|id| {
            claims_by_id
                .get(id.as_str())
                .map(|text| (*text).to_string())
                .ok_or_else(|| format!("declared claim ID `{id}` is not accepted evidence"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let source_anchors = source_ids
        .iter()
        .map(|id| {
            sources_by_id
                .get(id.as_str())
                .map(|anchor| (*anchor).to_string())
                .ok_or_else(|| format!("declared source ID `{id}` is not accepted evidence"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for claim_id in claim_ids {
        let has_declared_source = evidence.iter().any(|item| {
            item.claims.iter().any(|claim| claim.id == *claim_id)
                && item
                    .sources
                    .iter()
                    .any(|source| source_ids.contains(&source.id))
        });
        if !has_declared_source {
            return Err(format!(
                "declared claim ID `{claim_id}` has no declared source from the same accepted evidence item"
            ));
        }
    }
    Ok(ResolvedEvidence {
        claim_ids: claim_ids.clone(),
        source_ids: source_ids.clone(),
        claim_texts,
        source_anchors,
    })
}

pub(super) fn unique_sources_for_ids<'a>(
    evidence: &'a [AcceptedEvidence],
    source_ids: &BTreeSet<String>,
) -> Result<Vec<&'a AcceptedSource>, String> {
    let mut by_anchor = BTreeMap::new();
    let mut found_ids = BTreeSet::new();
    for source in evidence.iter().flat_map(|item| &item.sources) {
        if source_ids.contains(&source.id) {
            found_ids.insert(source.id.clone());
            by_anchor.entry(source.anchor.as_str()).or_insert(source);
        }
    }
    if found_ids != *source_ids {
        let missing = source_ids
            .difference(&found_ids)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "used source IDs are absent from accepted evidence: {}",
            missing.join(", ")
        ));
    }
    Ok(by_anchor.into_values().collect())
}
