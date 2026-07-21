//! Typed, host-validated outline for composing a research report.
//!
//! The model may propose this shape through structured generation, but the
//! host remains authoritative: every reference must resolve to the accepted
//! inquiry/evidence catalogs and every material inquiry must be assigned to
//! exactly one section before the outline is committed.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::EvidenceRef;

const MAX_OUTLINE_SECTIONS: usize = 16;
const MAX_REFERENCES_PER_KIND: usize = 32;
const MAX_ID_CHARS: usize = 160;
const MAX_HEADING_CHARS: usize = 240;
const MAX_PURPOSE_CHARS: usize = 4_000;
const MAX_COMPOSITION_HINT_CHARS: usize = 4_000;
const STABLE_ID_PATTERN: &str = r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,159}$";

/// A validated, evidence-addressed report outline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchOutline {
    pub sections: Vec<OutlineSection>,
}

/// One independently addressable section of a research report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutlineSection {
    /// Stable section identifier used by later drafting and audit events.
    pub id: String,
    pub heading: String,
    pub purpose: String,
    /// Historical replay metadata. Active outline generation no longer emits
    /// or consumes perspective IDs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub perspective_ids: Vec<String>,
    pub question_ids: Vec<String>,
    pub claim_ids: Vec<String>,
    pub source_ids: Vec<String>,
    /// Content-specific guidance for composing this section.
    pub composition_hint: String,
}

/// Host-owned catalogs and material-coverage requirements for an outline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutlineValidationContext {
    /// Historical perspective-outline replay catalog. Active contexts leave it
    /// empty and the active generation schema omits perspective references.
    pub allowed_perspective_ids: BTreeSet<String>,
    pub allowed_question_ids: BTreeSet<String>,
    pub allowed_claim_ids: BTreeSet<String>,
    pub allowed_source_ids: BTreeSet<String>,
    /// Accepted evidence items preserve the claim-to-source relationship that
    /// flat allowed-ID catalogs cannot express.
    pub evidence_catalog: BTreeMap<String, EvidenceRef>,
    /// Evidence IDs declared by the accepted answer to each question.
    pub question_evidence_ids: BTreeMap<String, BTreeSet<String>>,
    /// Historical perspective-outline replay requirement. Active contexts
    /// leave it empty.
    pub material_perspective_ids: BTreeSet<String>,
    /// Answered material questions whose accepted evidence must be covered by
    /// the outline. Bounded material questions remain in
    /// `required_question_ids` so the report must disclose them without
    /// pretending they have answer evidence.
    pub material_question_ids: BTreeSet<String>,
    /// Every inquiry path the final report must address, including bounded
    /// supporting paths that make an otherwise complete report qualified.
    pub required_question_ids: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineIdKind {
    Section,
    /// Historical perspective-outline replay only.
    Perspective,
    Question,
    Evidence,
    Claim,
    Source,
}

impl std::fmt::Display for OutlineIdKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Section => "section",
            Self::Perspective => "perspective",
            Self::Question => "question",
            Self::Evidence => "evidence",
            Self::Claim => "claim",
            Self::Source => "source",
        };
        formatter.write_str(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineValidationError {
    message: String,
}

impl OutlineValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for OutlineValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OutlineValidationError {}

/// Strict JSON Schema for model-generated [`ResearchOutline`] values.
pub fn research_outline_json_schema() -> serde_json::Value {
    let id = || {
        serde_json::json!({
            "type": "string",
            "minLength": 1,
            "maxLength": MAX_ID_CHARS,
            "pattern": STABLE_ID_PATTERN
        })
    };
    let id_array = |minimum: usize| {
        serde_json::json!({
            "type": "array",
            "items": id(),
            "minItems": minimum,
            "maxItems": MAX_REFERENCES_PER_KIND,
            "uniqueItems": true
        })
    };

    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "sections": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_OUTLINE_SECTIONS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "id": id(),
                        "heading": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": MAX_HEADING_CHARS
                        },
                        "purpose": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": MAX_PURPOSE_CHARS
                        },
                        "question_ids": id_array(0),
                        "claim_ids": id_array(1),
                        "source_ids": id_array(1),
                        "composition_hint": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": MAX_COMPOSITION_HINT_CHARS
                        }
                    },
                    "required": [
                        "id",
                        "heading",
                        "purpose",
                        "question_ids",
                        "claim_ids",
                        "source_ids",
                        "composition_hint"
                    ]
                }
            }
        },
        "required": ["sections"]
    })
}

/// Validate an outline against host-accepted inquiry and evidence IDs.
///
/// Section IDs are unique across the outline. Reference IDs are unique inside
/// each section list, while the same accepted evidence or inquiry may support
/// multiple sections. Material coverage therefore means "referenced at least
/// once", not "owned by exactly one section".
pub fn validate_research_outline(
    outline: &ResearchOutline,
    context: &OutlineValidationContext,
) -> Result<(), OutlineValidationError> {
    if outline.sections.is_empty() {
        return Err(OutlineValidationError::new(
            "research outline must contain at least one section",
        ));
    }
    if outline.sections.len() > MAX_OUTLINE_SECTIONS {
        return Err(OutlineValidationError::new(format!(
            "research outline has {} sections; maximum is {MAX_OUTLINE_SECTIONS}",
            outline.sections.len()
        )));
    }

    validate_material_catalog(
        OutlineIdKind::Perspective,
        &context.material_perspective_ids,
        &context.allowed_perspective_ids,
    )?;
    validate_material_catalog(
        OutlineIdKind::Question,
        &context.material_question_ids,
        &context.allowed_question_ids,
    )?;
    validate_material_catalog(
        OutlineIdKind::Question,
        &context.required_question_ids,
        &context.allowed_question_ids,
    )?;
    validate_evidence_graph(context)?;

    let mut section_ids = BTreeSet::new();
    let mut perspective_ids = BTreeSet::new();
    let mut question_ids = BTreeSet::new();
    let mut claim_ids = BTreeSet::new();
    let mut source_ids = BTreeSet::new();
    let mut section_evidence_coverage = BTreeMap::new();

    for section in &outline.sections {
        validate_stable_id(OutlineIdKind::Section, &section.id)?;
        if !section_ids.insert(section.id.clone()) {
            return Err(OutlineValidationError::new(format!(
                "duplicate section id `{}` in research outline",
                section.id
            )));
        }
        validate_section_text(&section.id, "heading", &section.heading, MAX_HEADING_CHARS)?;
        validate_section_text(&section.id, "purpose", &section.purpose, MAX_PURPOSE_CHARS)?;
        validate_section_text(
            &section.id,
            "composition hint",
            &section.composition_hint,
            MAX_COMPOSITION_HINT_CHARS,
        )?;
        if section.claim_ids.is_empty() {
            return Err(OutlineValidationError::new(format!(
                "section `{}` must reference at least one claim",
                section.id
            )));
        }
        if section.source_ids.is_empty() {
            return Err(OutlineValidationError::new(format!(
                "section `{}` must reference at least one source",
                section.id
            )));
        }

        validate_references(
            &section.id,
            OutlineIdKind::Perspective,
            &section.perspective_ids,
            &context.allowed_perspective_ids,
            &mut perspective_ids,
        )?;
        validate_references(
            &section.id,
            OutlineIdKind::Question,
            &section.question_ids,
            &context.allowed_question_ids,
            &mut question_ids,
        )?;
        validate_references(
            &section.id,
            OutlineIdKind::Claim,
            &section.claim_ids,
            &context.allowed_claim_ids,
            &mut claim_ids,
        )?;
        validate_references(
            &section.id,
            OutlineIdKind::Source,
            &section.source_ids,
            &context.allowed_source_ids,
            &mut source_ids,
        )?;
        section_evidence_coverage.insert(
            section.id.as_str(),
            validate_section_evidence_bindings(section, context)?,
        );
    }

    validate_material_coverage(
        OutlineIdKind::Perspective,
        &context.material_perspective_ids,
        &perspective_ids,
    )?;
    validate_material_coverage(
        OutlineIdKind::Question,
        &context.required_question_ids,
        &question_ids,
    )?;
    validate_material_question_evidence_coverage(outline, context, &section_evidence_coverage)?;
    Ok(())
}

fn validate_evidence_graph(
    context: &OutlineValidationContext,
) -> Result<(), OutlineValidationError> {
    let mut bound_claim_ids = BTreeSet::new();
    let mut bound_source_ids = BTreeSet::new();
    for (catalog_id, evidence) in &context.evidence_catalog {
        validate_stable_id(OutlineIdKind::Evidence, catalog_id)?;
        if catalog_id != &evidence.evidence_id {
            return Err(OutlineValidationError::new(format!(
                "evidence catalog key `{catalog_id}` does not match evidence id `{}`",
                evidence.evidence_id
            )));
        }
        if evidence.claim_ids.is_empty() || evidence.source_ids.is_empty() {
            return Err(OutlineValidationError::new(format!(
                "evidence id `{catalog_id}` must bind at least one claim and one source"
            )));
        }
        validate_evidence_references(
            catalog_id,
            OutlineIdKind::Claim,
            &evidence.claim_ids,
            &context.allowed_claim_ids,
            &mut bound_claim_ids,
        )?;
        validate_evidence_references(
            catalog_id,
            OutlineIdKind::Source,
            &evidence.source_ids,
            &context.allowed_source_ids,
            &mut bound_source_ids,
        )?;
    }

    validate_bound_catalog(
        OutlineIdKind::Claim,
        &context.allowed_claim_ids,
        &bound_claim_ids,
    )?;
    validate_bound_catalog(
        OutlineIdKind::Source,
        &context.allowed_source_ids,
        &bound_source_ids,
    )?;

    for (question_id, evidence_ids) in &context.question_evidence_ids {
        if !context.allowed_question_ids.contains(question_id) {
            return Err(OutlineValidationError::new(format!(
                "evidence bindings reference unknown question id `{question_id}`"
            )));
        }
        for evidence_id in evidence_ids {
            if !context.evidence_catalog.contains_key(evidence_id) {
                return Err(OutlineValidationError::new(format!(
                    "question id `{question_id}` references unknown answer evidence id `{evidence_id}`"
                )));
            }
        }
    }
    for question_id in &context.material_question_ids {
        if !context
            .question_evidence_ids
            .get(question_id)
            .is_some_and(|evidence_ids| !evidence_ids.is_empty())
        {
            return Err(OutlineValidationError::new(format!(
                "material question id `{question_id}` has no accepted answer evidence binding"
            )));
        }
    }
    Ok(())
}

fn validate_evidence_references(
    evidence_id: &str,
    kind: OutlineIdKind,
    ids: &[String],
    allowed: &BTreeSet<String>,
    bound: &mut BTreeSet<String>,
) -> Result<(), OutlineValidationError> {
    let mut evidence_ids = BTreeSet::new();
    for id in ids {
        validate_stable_id(kind, id)?;
        if !allowed.contains(id) {
            return Err(OutlineValidationError::new(format!(
                "evidence id `{evidence_id}` references unknown {kind} id `{id}`"
            )));
        }
        if !evidence_ids.insert(id) {
            return Err(OutlineValidationError::new(format!(
                "evidence id `{evidence_id}` repeats {kind} id `{id}`"
            )));
        }
        bound.insert(id.clone());
    }
    Ok(())
}

fn validate_bound_catalog(
    kind: OutlineIdKind,
    allowed: &BTreeSet<String>,
    bound: &BTreeSet<String>,
) -> Result<(), OutlineValidationError> {
    if let Some(id) = allowed.iter().find(|id| !bound.contains(*id)) {
        return Err(OutlineValidationError::new(format!(
            "allowed {kind} id `{id}` is not bound to an accepted evidence item"
        )));
    }
    Ok(())
}

fn validate_section_evidence_bindings(
    section: &OutlineSection,
    context: &OutlineValidationContext,
) -> Result<BTreeSet<(String, String)>, OutlineValidationError> {
    let section_claim_ids = section.claim_ids.iter().collect::<BTreeSet<_>>();
    let section_source_ids = section.source_ids.iter().collect::<BTreeSet<_>>();
    let paired_claims = context
        .evidence_catalog
        .iter()
        .filter(|(_, evidence)| {
            evidence
                .source_ids
                .iter()
                .any(|id| section_source_ids.contains(id))
        })
        .flat_map(|(evidence_id, evidence)| {
            evidence
                .claim_ids
                .iter()
                .filter(|claim_id| section_claim_ids.contains(claim_id))
                .map(|claim_id| (evidence_id.clone(), claim_id.clone()))
        })
        .collect::<BTreeSet<_>>();

    for claim_id in &section.claim_ids {
        let has_declared_source = context.evidence_catalog.values().any(|evidence| {
            evidence.claim_ids.contains(claim_id)
                && evidence
                    .source_ids
                    .iter()
                    .any(|source_id| section_source_ids.contains(source_id))
        });
        if !has_declared_source {
            return Err(OutlineValidationError::new(format!(
                "section `{}` claim id `{claim_id}` has no declared source from the same accepted evidence item",
                section.id
            )));
        }
    }
    Ok(paired_claims)
}

fn validate_material_question_evidence_coverage(
    outline: &ResearchOutline,
    context: &OutlineValidationContext,
    section_evidence_coverage: &BTreeMap<&str, BTreeSet<(String, String)>>,
) -> Result<(), OutlineValidationError> {
    for question_id in &context.material_question_ids {
        let Some(evidence_ids) = context.question_evidence_ids.get(question_id) else {
            return Err(OutlineValidationError::new(format!(
                "material question id `{question_id}` has no accepted answer evidence binding"
            )));
        };
        for evidence_id in evidence_ids {
            let evidence = context.evidence_catalog.get(evidence_id).ok_or_else(|| {
                OutlineValidationError::new(format!(
                    "material question id `{question_id}` references unknown answer evidence id `{evidence_id}`"
                ))
            })?;
            for claim_id in &evidence.claim_ids {
                let covered = outline.sections.iter().any(|section| {
                    section.question_ids.contains(question_id)
                        && section_evidence_coverage
                            .get(section.id.as_str())
                            .is_some_and(|paired| {
                                paired.contains(&(evidence_id.clone(), claim_id.clone()))
                            })
                });
                if !covered {
                    return Err(OutlineValidationError::new(format!(
                        "material question id `{question_id}` does not cover claim id `{claim_id}` from answer evidence id `{evidence_id}` with a bound source in any outline section"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn validate_material_catalog(
    kind: OutlineIdKind,
    material: &BTreeSet<String>,
    allowed: &BTreeSet<String>,
) -> Result<(), OutlineValidationError> {
    if let Some(id) = material.iter().find(|id| !allowed.contains(*id)) {
        return Err(OutlineValidationError::new(format!(
            "material {kind} id `{id}` is absent from the corresponding allowed-id catalog"
        )));
    }
    Ok(())
}

fn validate_material_coverage(
    kind: OutlineIdKind,
    material: &BTreeSet<String>,
    covered: &BTreeSet<String>,
) -> Result<(), OutlineValidationError> {
    if let Some(id) = material.iter().find(|id| !covered.contains(*id)) {
        return Err(OutlineValidationError::new(format!(
            "material {kind} id `{id}` is not covered by the research outline"
        )));
    }
    Ok(())
}

fn validate_references(
    section_id: &str,
    kind: OutlineIdKind,
    ids: &[String],
    allowed: &BTreeSet<String>,
    covered: &mut BTreeSet<String>,
) -> Result<(), OutlineValidationError> {
    if ids.len() > MAX_REFERENCES_PER_KIND {
        return Err(OutlineValidationError::new(format!(
            "section `{section_id}` has {} {kind} ids; maximum is {MAX_REFERENCES_PER_KIND}",
            ids.len()
        )));
    }
    let mut section_ids = BTreeSet::new();
    for id in ids {
        validate_stable_id(kind, id)?;
        if !allowed.contains(id) {
            return Err(OutlineValidationError::new(format!(
                "section `{section_id}` references unknown {kind} id `{id}`"
            )));
        }
        if !section_ids.insert(id.clone()) {
            return Err(OutlineValidationError::new(format!(
                "section `{section_id}` repeats {kind} id `{id}`"
            )));
        }
        covered.insert(id.clone());
    }
    Ok(())
}

fn validate_stable_id(kind: OutlineIdKind, id: &str) -> Result<(), OutlineValidationError> {
    let mut chars = id.chars();
    let valid = id.chars().count() <= MAX_ID_CHARS
        && chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'));
    if valid {
        Ok(())
    } else {
        Err(OutlineValidationError::new(format!(
            "{kind} id `{id}` is not a valid stable id"
        )))
    }
}

fn validate_section_text(
    section_id: &str,
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), OutlineValidationError> {
    if value.trim().is_empty() {
        return Err(OutlineValidationError::new(format!(
            "section `{section_id}` has an empty {field}"
        )));
    }
    let actual = value.chars().count();
    if actual > maximum {
        return Err(OutlineValidationError::new(format!(
            "section `{section_id}` {field} has {actual} characters; maximum is {maximum}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn section(id: &str, suffix: &str) -> OutlineSection {
        OutlineSection {
            id: id.to_string(),
            heading: format!("{suffix} findings"),
            purpose: "Explain the accepted evidence.".to_string(),
            perspective_ids: vec![format!("perspective:{suffix}")],
            question_ids: vec![format!("question:{suffix}")],
            claim_ids: vec![format!("claim:{suffix}")],
            source_ids: vec![format!("source:{suffix}")],
            composition_hint: "Lead with the finding, then cite the evidence.".to_string(),
        }
    }

    fn fixture() -> (ResearchOutline, OutlineValidationContext) {
        let suffixes = ["official", "independent"];
        let outline = ResearchOutline {
            sections: vec![
                section("section:official", suffixes[0]),
                section("section:independent", suffixes[1]),
            ],
        };
        let context = OutlineValidationContext {
            allowed_perspective_ids: set(&["perspective:official", "perspective:independent"]),
            allowed_question_ids: set(&["question:official", "question:independent"]),
            allowed_claim_ids: set(&["claim:official", "claim:independent"]),
            allowed_source_ids: set(&["source:official", "source:independent"]),
            evidence_catalog: [
                (
                    "evidence:official".to_string(),
                    EvidenceRef::new(
                        "evidence:official",
                        vec!["claim:official".to_string()],
                        vec!["source:official".to_string()],
                    ),
                ),
                (
                    "evidence:independent".to_string(),
                    EvidenceRef::new(
                        "evidence:independent",
                        vec!["claim:independent".to_string()],
                        vec!["source:independent".to_string()],
                    ),
                ),
            ]
            .into_iter()
            .collect(),
            question_evidence_ids: [
                ("question:official".to_string(), set(&["evidence:official"])),
                (
                    "question:independent".to_string(),
                    set(&["evidence:independent"]),
                ),
            ]
            .into_iter()
            .collect(),
            material_perspective_ids: set(&["perspective:official", "perspective:independent"]),
            material_question_ids: set(&["question:official", "question:independent"]),
            required_question_ids: set(&["question:official", "question:independent"]),
        };
        (outline, context)
    }

    fn assert_error(outline: &ResearchOutline, context: &OutlineValidationContext, text: &str) {
        let error = validate_research_outline(outline, context).unwrap_err();
        assert!(error.message().contains(text), "{error}");
    }

    #[test]
    fn schema_is_strict_and_bounded() {
        let schema = research_outline_json_schema();
        let section = &schema["properties"]["sections"]["items"];
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["sections"]["maxItems"], 16);
        assert_eq!(section["additionalProperties"], false);
        assert_eq!(section["required"].as_array().map(Vec::len), Some(7));
        assert!(section["properties"].get("perspective_ids").is_none());
        assert_eq!(section["properties"]["claim_ids"]["minItems"], 1);
        assert_eq!(section["properties"]["source_ids"]["minItems"], 1);
        for field in ["question_ids", "claim_ids", "source_ids"] {
            assert_eq!(section["properties"][field]["uniqueItems"], true);
            assert_eq!(section["properties"][field]["maxItems"], 32);
        }
    }

    #[test]
    fn active_outline_omits_legacy_perspective_ids() {
        let outline: ResearchOutline = serde_json::from_value(serde_json::json!({
            "sections": [{
                "id": "section:answer",
                "heading": "Answer",
                "purpose": "Answer the research question",
                "question_ids": ["question:answer"],
                "claim_ids": ["claim:answer"],
                "source_ids": ["source:answer"],
                "composition_hint": "Lead with the evidence-backed answer"
            }]
        }))
        .expect("active outline without perspective IDs");

        assert!(outline.sections[0].perspective_ids.is_empty());
        assert!(serde_json::to_value(outline).unwrap()["sections"][0]
            .get("perspective_ids")
            .is_none());
    }

    #[test]
    fn accepts_material_coverage_and_cross_section_reference_reuse() {
        let (mut outline, context) = fixture();
        assert_eq!(validate_research_outline(&outline, &context), Ok(()));
        let mut reused = outline.sections[0].clone();
        reused.id = "section:reused-evidence".to_string();
        outline.sections.push(reused);
        assert_eq!(validate_research_outline(&outline, &context), Ok(()));
    }

    #[test]
    fn requires_claim_source_and_allowed_ids() {
        let (mut outline, context) = fixture();
        outline.sections[0].claim_ids.clear();
        assert_error(&outline, &context, "at least one claim");
        let (mut outline, context) = fixture();
        outline.sections[0].source_ids.clear();
        assert_error(&outline, &context, "at least one source");
        let (mut outline, context) = fixture();
        outline.sections[0].question_ids[0] = "question:unknown".to_string();
        assert_error(&outline, &context, "unknown question id");
    }

    #[test]
    fn rejects_cross_evidence_claim_source_pairing() {
        let (mut outline, context) = fixture();
        outline.sections[0].source_ids = vec!["source:independent".to_string()];
        assert_error(&outline, &context, "same accepted evidence item");
    }

    #[test]
    fn material_question_must_cover_its_answer_evidence_pair() {
        let (mut outline, context) = fixture();
        outline.sections[0].claim_ids = vec!["claim:independent".to_string()];
        outline.sections[0].source_ids = vec!["source:independent".to_string()];
        assert_error(&outline, &context, "answer evidence id `evidence:official`");
    }

    #[test]
    fn material_question_covers_every_claim_in_its_answer_evidence() {
        let (mut outline, mut context) = fixture();
        context
            .allowed_claim_ids
            .insert("claim:official-detail".to_string());
        context.evidence_catalog.insert(
            "evidence:official".to_string(),
            EvidenceRef::new(
                "evidence:official",
                vec![
                    "claim:official".to_string(),
                    "claim:official-detail".to_string(),
                ],
                vec!["source:official".to_string()],
            ),
        );

        assert_error(
            &outline,
            &context,
            "does not cover claim id `claim:official-detail`",
        );

        outline.sections[0]
            .claim_ids
            .push("claim:official-detail".to_string());
        assert_eq!(validate_research_outline(&outline, &context), Ok(()));
    }

    #[test]
    fn rejects_duplicate_section_and_intra_section_reference_ids() {
        let (mut outline, context) = fixture();
        outline.sections[1].id = outline.sections[0].id.clone();
        assert_error(&outline, &context, "duplicate section id");
        let (mut outline, context) = fixture();
        outline.sections[0]
            .source_ids
            .push("source:official".to_string());
        assert_error(&outline, &context, "repeats source id");
    }

    #[test]
    fn requires_material_coverage_and_stable_section_ids() {
        let (mut outline, context) = fixture();
        outline.sections[1].perspective_ids.clear();
        assert_error(&outline, &context, "material perspective id");
        let (mut outline, context) = fixture();
        outline.sections[1].question_ids.clear();
        assert_error(&outline, &context, "material question id");
        let (mut outline, context) = fixture();
        outline.sections[0].id = "section with spaces".to_string();
        assert_error(&outline, &context, "valid stable id");
    }
}
