//! Closed-evidence structured generation for the final DeepResearch report.

use std::collections::{BTreeMap, BTreeSet};

use a3s::research::ResearchObligation;
use serde::{Deserialize, Serialize};

const REPORT_MIN_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportNarrativeMode {
    Pyramid,
    Narrative,
    Instructional,
    #[default]
    Briefing,
}

impl ReportNarrativeMode {
    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Pyramid => "mode-pyramid",
            Self::Narrative => "mode-narrative",
            Self::Instructional => "mode-instructional",
            Self::Briefing => "mode-briefing",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReportArchetype {
    #[default]
    Editorial,
    Analytical,
    Chronicle,
    Executive,
    FieldNotes,
}

impl ReportArchetype {
    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Editorial => "archetype-editorial",
            Self::Analytical => "archetype-analytical",
            Self::Chronicle => "archetype-chronicle",
            Self::Executive => "archetype-executive",
            Self::FieldNotes => "archetype-field-notes",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportPalette {
    #[default]
    Ocean,
    Graphite,
    Forest,
    Amber,
    Plum,
}

impl ReportPalette {
    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Ocean => "palette-ocean",
            Self::Graphite => "palette-graphite",
            Self::Forest => "palette-forest",
            Self::Amber => "palette-amber",
            Self::Plum => "palette-plum",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportDensity {
    Compact,
    #[default]
    Balanced,
    Spacious,
}

impl ReportDensity {
    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Compact => "density-compact",
            Self::Balanced => "density-balanced",
            Self::Spacious => "density-spacious",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportHero {
    Statement,
    #[default]
    Split,
    Metrics,
}

impl ReportHero {
    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Statement => "hero-statement",
            Self::Split => "hero-split",
            Self::Metrics => "hero-metrics",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportVisualStance {
    Safe,
    #[default]
    Shifted,
    Bold,
}

impl ReportVisualStance {
    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Safe => "stance-safe",
            Self::Shifted => "stance-shifted",
            Self::Bold => "stance-bold",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportSectionRhythm {
    Anchor,
    Dense,
    #[default]
    Breathing,
}

impl ReportSectionRhythm {
    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Anchor => "rhythm-anchor",
            Self::Dense => "rhythm-dense",
            Self::Breathing => "rhythm-breathing",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportSectionComposition {
    #[default]
    Prose,
    KeyPoints,
    Comparison,
    Timeline,
    Process,
    Evidence,
    SourceLedger,
}

impl ReportSectionComposition {
    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Prose => "composition-prose",
            Self::KeyPoints => "composition-key-points",
            Self::Comparison => "composition-comparison",
            Self::Timeline => "composition-timeline",
            Self::Process => "composition-process",
            Self::Evidence => "composition-evidence",
            Self::SourceLedger => "composition-source-ledger",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReportSectionTreatment {
    pub(crate) heading: String,
    pub(crate) rhythm: ReportSectionRhythm,
    pub(crate) composition: ReportSectionComposition,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReportPresentation {
    pub(crate) narrative_mode: ReportNarrativeMode,
    pub(crate) archetype: ReportArchetype,
    pub(crate) palette: ReportPalette,
    pub(crate) density: ReportDensity,
    pub(crate) hero: ReportHero,
    pub(crate) visual_stance: ReportVisualStance,
    pub(crate) rationale: String,
    pub(crate) section_plan: Vec<ReportSectionTreatment>,
}

impl ReportPresentation {
    pub(crate) fn body_classes(&self) -> String {
        [
            self.narrative_mode.class_name(),
            self.archetype.class_name(),
            self.palette.class_name(),
            self.density.class_name(),
            self.hero.class_name(),
            self.visual_stance.class_name(),
        ]
        .join(" ")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportTrackStatus {
    Answered,
    Bounded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReportTrackCoverage {
    pub(crate) obligation_id: String,
    pub(crate) status: ReportTrackStatus,
    pub(crate) finding: String,
    pub(crate) interpretation: String,
    pub(crate) implication: String,
    pub(crate) uncertainty: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReportEditorialPlan {
    pub(crate) thesis: String,
    pub(crate) track_coverage: Vec<ReportTrackCoverage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GeneratedDeepResearchReport {
    pub(crate) markdown: String,
    pub(crate) editorial: ReportEditorialPlan,
    pub(crate) presentation: ReportPresentation,
}

pub(crate) fn validate_report_obligation_coverage(
    editorial: &ReportEditorialPlan,
    obligations: Option<&[ResearchObligation]>,
) -> Result<(), String> {
    let planned_by_id = obligations.map(|obligations| {
        obligations
            .iter()
            .map(|obligation| (obligation.id.as_str(), obligation.title.as_str()))
            .collect::<BTreeMap<_, _>>()
    });
    if planned_by_id.as_ref().is_some_and(BTreeMap::is_empty) {
        return Err(
            "content rejected: Inquiry report context contains no research obligations".to_string(),
        );
    }

    let mut covered_ids = BTreeSet::new();
    for coverage in &editorial.track_coverage {
        let obligation_id = coverage.obligation_id.trim();
        if obligation_id != coverage.obligation_id
            || !stable_report_obligation_id(obligation_id)
            || !covered_ids.insert(obligation_id.to_string())
        {
            return Err(format!(
                "content rejected: the editorial quality map contains an invalid or duplicate obligation ID {:?}",
                coverage.obligation_id
            ));
        }
        if let Some(planned_by_id) = &planned_by_id {
            if !planned_by_id.contains_key(obligation_id) {
                return Err(format!(
                    "content rejected: the editorial quality map references unknown obligation ID `{obligation_id}`"
                ));
            }
        }
        if coverage.finding.trim().chars().count() < 8
            || coverage.interpretation.trim().chars().count() < 8
        {
            return Err(format!(
                "content rejected: research obligation `{obligation_id}` lacks a finding or interpretation"
            ));
        }
        if matches!(coverage.status, ReportTrackStatus::Bounded)
            && coverage.uncertainty.trim().is_empty()
        {
            return Err(format!(
                "content rejected: bounded research obligation `{obligation_id}` does not state its uncertainty"
            ));
        }
    }

    if let Some(planned_by_id) = planned_by_id {
        let missing = planned_by_id
            .into_iter()
            .filter(|(id, _)| !covered_ids.contains(*id))
            .map(|(id, title)| format!("{title} (`{id}`)"))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!(
                "content rejected: the report did not account for planner-authored research obligation(s): {}",
                missing.join("; ")
            ));
        }
    }
    Ok(())
}

fn stable_report_obligation_id(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && value.chars().count() <= 160
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
}

pub(super) fn deep_research_report_frame_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "report_title": {
                "type": "string",
                "minLength": 2,
                "maxLength": 120
            },
            "reader_labels": {
                "type": "object",
                "additionalProperties": false,
                "description": "Short reader-facing report labels in the query language. These are rendered by the Host; they are not private metadata.",
                "properties": {
                    "qualification_heading": { "type": "string", "minLength": 2, "maxLength": 120 },
                    "qualification_intro": { "type": "string", "minLength": 4, "maxLength": 300 },
                    "sources_heading": { "type": "string", "minLength": 2, "maxLength": 80 },
                    "decision_heading": { "type": "string", "minLength": 2, "maxLength": 100 },
                    "evidence_limitation": { "type": "string", "minLength": 2, "maxLength": 100 },
                    "primary_source_support": { "type": "string", "minLength": 2, "maxLength": 100 },
                    "independent_corroboration": { "type": "string", "minLength": 2, "maxLength": 100 },
                    "established_boundary": { "type": "string", "minLength": 4, "maxLength": 240 },
                    "qualified_boundary": { "type": "string", "minLength": 4, "maxLength": 240 },
                    "unresolved_boundary": { "type": "string", "minLength": 4, "maxLength": 240 }
                },
                "required": [
                    "qualification_heading",
                    "qualification_intro",
                    "sources_heading",
                    "decision_heading",
                    "evidence_limitation",
                    "primary_source_support",
                    "independent_corroboration",
                    "established_boundary",
                    "qualified_boundary",
                    "unresolved_boundary"
                ]
            },
            "decision_guidance": {
                "type": "array",
                "minItems": 0,
                "maxItems": 6,
                "description": "Reader-facing, evidence-bounded normative guidance. Cover each action or choice scenario explicitly requested by the query when supported premises exist; otherwise return an empty array.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "scenario": { "type": "string", "minLength": 2, "maxLength": 180 },
                        "recommendation": { "type": "string", "minLength": 8, "maxLength": 700 },
                        "basis_obligation_ids": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 6,
                            "uniqueItems": true,
                            "items": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 160,
                                "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,159}$"
                            }
                        },
                        "boundary": { "type": "string", "maxLength": 500 }
                    },
                    "required": ["scenario", "recommendation", "basis_obligation_ids", "boundary"]
                }
            },
            "editorial": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "thesis": {
                        "type": "string",
                        "minLength": 12,
                        "maxLength": 1200,
                        "description": "One reader-facing sentence that directly answers the query and can lead the report hero."
                    },
                    "track_coverage": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 6,
                        "description": "One entry for every planner-authored research obligation. Copy each stable obligation ID exactly from the closed frame packet; this is a private quality map, not report prose.",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "obligation_id": {
                                    "type": "string",
                                    "minLength": 1,
                                    "maxLength": 160,
                                    "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,159}$"
                                },
                                "status": { "type": "string", "enum": ["answered", "bounded"] },
                                "finding": { "type": "string", "minLength": 8, "maxLength": 600 },
                                "interpretation": { "type": "string", "minLength": 8, "maxLength": 600 },
                                "implication": { "type": "string", "maxLength": 600 },
                                "uncertainty": { "type": "string", "maxLength": 600 }
                            },
                            "required": ["obligation_id", "status", "finding", "interpretation", "implication", "uncertainty"]
                        }
                    }
                },
                "required": ["thesis", "track_coverage"]
            },
            "presentation": {
                "type": "object",
                "additionalProperties": false,
                "description": "A content-semantic art-direction lock. Choose deliberately from the report's argument, audience, evidence shape, and reading occasion; never from topic keywords or a task-specific template.",
                "properties": {
                    "narrative_mode": {
                        "type": "string",
                        "enum": ["pyramid", "narrative", "instructional", "briefing"],
                        "description": "How the report argues: conclusion-first, event arc, progressive explanation, or balanced scan."
                    },
                    "archetype": {
                        "type": "string",
                        "enum": ["editorial", "analytical", "chronicle", "executive", "field-notes"],
                        "description": "The visual composition family. Analytical emphasizes comparisons; chronicle emphasizes ordered change; executive is restrained and decision-first; field-notes suits observation and investigation; editorial suits long-form synthesis."
                    },
                    "palette": {
                        "type": "string",
                        "enum": ["ocean", "graphite", "forest", "amber", "plum"],
                        "description": "A curated accessible color system selected for tone and audience, independent from the subject's literal colors."
                    },
                    "density": {
                        "type": "string",
                        "enum": ["compact", "balanced", "spacious"],
                        "description": "Information density appropriate to evidence volume and reading occasion."
                    },
                    "hero": {
                        "type": "string",
                        "enum": ["statement", "split", "metrics"],
                        "description": "Cover composition: thesis-led statement, balanced split, or evidence-profile-led metrics."
                    },
                    "visual_stance": {
                        "type": "string",
                        "enum": ["safe", "shifted", "bold"],
                        "description": "Safe for formal/high-risk contexts, shifted for one controlled distinctive motif, bold only when audience and evidence support it."
                    },
                    "rationale": {
                        "type": "string",
                        "minLength": 12,
                        "maxLength": 240,
                        "description": "One concise private sentence naming the dominant information relationship, reader use, and resulting high-level structural choice. It is never rendered."
                    },
                    "section_plan": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 12,
                        "description": "One compact entry for every level-two Markdown heading, in report order. Copy each heading exactly and choose rhythm and composition from that section's information relationship rather than its topic words.",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "heading": {
                                    "type": "string",
                                    "minLength": 1,
                                    "maxLength": 180,
                                    "description": "The exact level-two Markdown heading without the ## marker."
                                },
                                "rhythm": {
                                    "type": "string",
                                    "enum": ["anchor", "dense", "breathing"],
                                    "description": "Section pacing only: use exactly anchor, dense, or breathing. Do not reuse the global density values compact, balanced, or spacious."
                                },
                                "composition": {
                                    "type": "string",
                                    "enum": ["prose", "key_points", "comparison", "timeline", "process", "evidence", "source_ledger"]
                                }
                            },
                            "required": ["heading", "rhythm", "composition"]
                        }
                    }
                },
                "required": ["narrative_mode", "archetype", "palette", "density", "hero", "visual_stance", "rationale", "section_plan"]
            }
        },
        "required": ["report_title", "reader_labels", "decision_guidance", "editorial", "presentation"]
    })
}

pub(super) fn deep_research_report_from_generation(
    output: &str,
    exit_code: i32,
) -> Result<GeneratedDeepResearchReport, String> {
    if exit_code != 0 {
        return Err(output
            .lines()
            .next()
            .unwrap_or("structured report generation failed")
            .to_string());
    }
    let envelope = serde_json::from_str::<serde_json::Value>(output)
        .map_err(|error| format!("structured report response was not valid JSON: {error}"))?;
    let object = envelope
        .get("object")
        .cloned()
        .ok_or_else(|| "structured report response did not contain an object".to_string())?;
    let mut report = serde_json::from_value::<GeneratedDeepResearchReport>(object)
        .map_err(|error| format!("structured report object violated its contract: {error}"))?;
    report.markdown = report.markdown.trim().to_string();
    report.editorial.thesis = report.editorial.thesis.trim().to_string();
    report.presentation.rationale = report.presentation.rationale.trim().to_string();
    for section in &mut report.presentation.section_plan {
        section.heading = section.heading.trim().to_string();
    }
    if report.markdown.chars().count() < REPORT_MIN_CHARS {
        return Err("structured report response did not contain substantive Markdown".to_string());
    }
    if report.editorial.thesis.chars().count() < 12 {
        return Err("structured report response did not contain a substantive thesis".to_string());
    }
    if report.editorial.track_coverage.is_empty() {
        return Err(
            "structured report response did not audit any planned research track".to_string(),
        );
    }
    reconcile_section_plan(&report.markdown, &mut report.presentation.section_plan)?;
    Ok(report)
}

fn reconcile_section_plan(
    markdown: &str,
    section_plan: &mut Vec<ReportSectionTreatment>,
) -> Result<(), String> {
    let markdown_headings = markdown_level_two_headings(markdown);
    if markdown_headings.is_empty() {
        return Err(
            "structured report response did not contain any level-two Markdown sections"
                .to_string(),
        );
    }
    if section_plan.is_empty() {
        return Ok(());
    }

    // Presentation metadata is advisory. A model can finish a strong report and
    // then rename, insert, or reorder one heading while constructing the private
    // section plan. Reconcile that local drift deterministically instead of
    // discarding the report and spending another model turn on unchanged prose.
    let mut remaining = std::mem::take(section_plan)
        .into_iter()
        .enumerate()
        .map(|(index, treatment)| Some((index, treatment)))
        .collect::<Vec<_>>();
    let mut reconciled = std::iter::repeat_with(|| None)
        .take(markdown_headings.len())
        .collect::<Vec<Option<ReportSectionTreatment>>>();

    // Preserve treatments by semantic heading first, regardless of plan order.
    for (markdown_index, heading) in markdown_headings.iter().enumerate() {
        let normalized = normalize_section_heading(heading);
        let Some(plan_index) = remaining.iter().position(|entry| {
            entry.as_ref().is_some_and(|(_, treatment)| {
                normalize_section_heading(&treatment.heading) == normalized
            })
        }) else {
            continue;
        };
        let Some((_, mut treatment)) = remaining[plan_index].take() else {
            continue;
        };
        treatment.heading = heading.clone();
        reconciled[markdown_index] = Some(treatment);
    }

    // For renamed headings, retain the nearest unclaimed positional treatment.
    // This keeps the model's content-driven rhythm/composition choice without
    // pretending that stale metadata is an independently valid section.
    for (markdown_index, heading) in markdown_headings.iter().enumerate() {
        if reconciled[markdown_index].is_some() {
            continue;
        }
        let nearest = remaining
            .iter()
            .enumerate()
            .filter_map(|(slot, entry)| {
                entry
                    .as_ref()
                    .map(|(original_index, _)| (slot, original_index.abs_diff(markdown_index)))
            })
            .min_by_key(|(_, distance)| *distance)
            .map(|(slot, _)| slot);
        let mut treatment = nearest
            .and_then(|slot| remaining[slot].take())
            .map(|(_, treatment)| treatment)
            .unwrap_or_else(|| ReportSectionTreatment {
                heading: heading.clone(),
                rhythm: ReportSectionRhythm::Breathing,
                composition: ReportSectionComposition::Prose,
            });
        treatment.heading = heading.clone();
        reconciled[markdown_index] = Some(treatment);
    }

    *section_plan = reconciled.into_iter().flatten().collect();
    Ok(())
}

fn markdown_level_two_headings(markdown: &str) -> Vec<String> {
    let mut headings = Vec::new();
    let mut active_fence = None;

    for raw_line in markdown.lines() {
        let Some(line) = markdown_line_after_optional_indent(raw_line) else {
            continue;
        };
        if let Some((active_marker, active_length)) = active_fence {
            if markdown_fence(line).is_some_and(|(marker, length, suffix)| {
                marker == active_marker && length >= active_length && suffix.trim().is_empty()
            }) {
                active_fence = None;
            }
            continue;
        }
        if let Some((marker, length, _)) = markdown_fence(line) {
            active_fence = Some((marker, length));
            continue;
        }
        if let Some(heading) = line
            .strip_prefix("## ")
            .filter(|heading| !heading.starts_with('#'))
            .map(clean_section_heading)
            .filter(|heading| !heading.is_empty())
        {
            headings.push(heading);
        }
    }

    headings
}

fn markdown_line_after_optional_indent(line: &str) -> Option<&str> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    (indent <= 3).then(|| &line[indent..])
}

fn markdown_fence(line: &str) -> Option<(u8, usize, &str)> {
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = line.bytes().take_while(|byte| *byte == marker).count();
    (length >= 3).then(|| (marker, length, &line[length..]))
}

fn normalize_section_heading(heading: &str) -> String {
    clean_section_heading(heading).to_lowercase()
}

fn clean_section_heading(heading: &str) -> String {
    heading.trim().trim_end_matches('#').trim().to_string()
}

#[cfg(test)]
pub(super) fn deep_research_report_markdown_from_generation(
    output: &str,
    exit_code: i32,
) -> Result<String, String> {
    deep_research_report_from_generation(output, exit_code).map(|report| report.markdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_object(markdown: String) -> serde_json::Value {
        serde_json::json!({
            "markdown": markdown,
            "editorial": {
                "thesis": "The evidence supports a bounded, source-backed conclusion.",
                "track_coverage": [{
                    "obligation_id": "obligation:current-status",
                    "status": "answered",
                    "finding": "The current status is documented by the cited source.",
                    "interpretation": "The documented status materially answers the request.",
                    "implication": "Readers can act on the stated current status.",
                    "uncertainty": "The evidence remains bounded to the retrieval date."
                }]
            },
            "presentation": {
                "narrative_mode": "pyramid",
                "archetype": "analytical",
                "palette": "graphite",
                "density": "compact",
                "hero": "metrics",
                "visual_stance": "safe",
                "rationale": "A decision-first analytical treatment fits the compact evidence set.",
                "section_plan": [
                    {
                        "heading": "Findings",
                        "rhythm": "anchor",
                        "composition": "key_points"
                    },
                    {
                        "heading": "Sources",
                        "rhythm": "dense",
                        "composition": "source_ledger"
                    },
                    {
                        "heading": "Limitations",
                        "rhythm": "breathing",
                        "composition": "prose"
                    }
                ]
            }
        })
    }

    #[test]
    fn report_frame_schema_is_closed_and_has_no_monolithic_markdown_contract() {
        let schema = deep_research_report_frame_schema();

        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("report_title").is_some());
        assert!(schema["properties"].get("reader_labels").is_some());
        assert!(schema["properties"].get("decision_guidance").is_some());
        assert!(schema["required"]
            .as_array()
            .is_some_and(|required| required.contains(&serde_json::json!("reader_labels"))));
        assert!(schema["required"]
            .as_array()
            .is_some_and(|required| required.contains(&serde_json::json!("decision_guidance"))));
        assert!(schema["properties"].get("markdown").is_none());
        assert_eq!(
            schema["properties"]["presentation"]["properties"]["archetype"]["enum"],
            serde_json::json!([
                "editorial",
                "analytical",
                "chronicle",
                "executive",
                "field-notes"
            ])
        );
        assert_eq!(
            schema["properties"]["presentation"]["properties"]["section_plan"]["items"]
                ["properties"]["composition"]["enum"],
            serde_json::json!([
                "prose",
                "key_points",
                "comparison",
                "timeline",
                "process",
                "evidence",
                "source_ledger"
            ])
        );
        assert!(schema["properties"]["presentation"]["required"]
            .as_array()
            .is_some_and(|required| required.contains(&serde_json::json!("section_plan"))));
    }

    #[test]
    fn report_generation_extracts_validated_content_and_art_direction() {
        let markdown = format!(
            "# Report\n\n## Findings\n\n{}\n\n## Sources\n\n- https://example.com/source\n\n## Limitations\n\nBounded evidence.",
            "Substantive source-backed analysis. ".repeat(5)
        );
        let output = serde_json::json!({
            "object": valid_object(markdown.clone()),
            "mode_used": "tool"
        })
        .to_string();

        let report = deep_research_report_from_generation(&output, 0).unwrap();
        assert_eq!(report.markdown, markdown);
        assert_eq!(report.presentation.archetype, ReportArchetype::Analytical);
        assert_eq!(report.presentation.hero, ReportHero::Metrics);
        assert_eq!(
            report.presentation.section_plan[0].composition,
            ReportSectionComposition::KeyPoints
        );
        assert_eq!(
            deep_research_report_markdown_from_generation(&output, 0).unwrap(),
            markdown
        );
        assert!(deep_research_report_from_generation(&output, 1).is_err());
        assert!(deep_research_report_from_generation("{}", 0).is_err());
    }

    #[test]
    fn report_generation_rejects_unapproved_presentation_values() {
        let markdown = "Substantive source-backed analysis. ".repeat(8);
        let mut object = valid_object(markdown);
        object["presentation"]["archetype"] = serde_json::json!("world-cup-special");
        let output = serde_json::json!({ "object": object }).to_string();

        let error = deep_research_report_from_generation(&output, 0).unwrap_err();
        assert!(error.contains("violated its contract"), "{error}");
    }

    #[test]
    fn report_generation_reconciles_a_section_plan_missing_a_report_section() {
        let markdown = format!(
            "# Report\n\n## Findings\n\n{}\n\n## Sources\n\n- https://example.com/source\n\n## Limitations\n\nBounded evidence.",
            "Substantive source-backed analysis. ".repeat(5)
        );
        let mut object = valid_object(markdown);
        object["presentation"]["section_plan"]
            .as_array_mut()
            .unwrap()
            .remove(2);
        let output = serde_json::json!({ "object": object }).to_string();

        let report = deep_research_report_from_generation(&output, 0).unwrap();
        assert_eq!(report.presentation.section_plan.len(), 3);
        assert_eq!(report.presentation.section_plan[2].heading, "Limitations");
        assert_eq!(
            report.presentation.section_plan[2].composition,
            ReportSectionComposition::Prose
        );
        assert_eq!(
            report.presentation.section_plan[2].rhythm,
            ReportSectionRhythm::Breathing
        );
    }

    #[test]
    fn report_generation_rejects_an_omitted_section_plan() {
        let markdown = format!(
            "# Report\n\n## Findings\n\n{}\n\n## Sources\n\n- https://example.com/source",
            "Evidence-backed finding. ".repeat(8)
        );
        let mut object = valid_object(markdown);
        object["presentation"]
            .as_object_mut()
            .expect("presentation fixture")
            .remove("section_plan");
        let output = serde_json::json!({ "object": object }).to_string();

        let error = deep_research_report_from_generation(&output, 0).unwrap_err();
        assert!(error.contains("section_plan"), "{error}");
    }

    #[test]
    fn report_generation_reorders_a_section_plan_into_report_order() {
        let markdown = format!(
            "# Report\n\n## Findings\n\n{}\n\n## Sources\n\n- https://example.com/source\n\n## Limitations\n\nBounded evidence.",
            "Substantive source-backed analysis. ".repeat(5)
        );
        let mut object = valid_object(markdown);
        object["presentation"]["section_plan"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        let output = serde_json::json!({ "object": object }).to_string();

        let report = deep_research_report_from_generation(&output, 0).unwrap();
        let headings = report
            .presentation
            .section_plan
            .iter()
            .map(|section| section.heading.as_str())
            .collect::<Vec<_>>();
        assert_eq!(headings, vec!["Findings", "Sources", "Limitations"]);
        assert_eq!(
            report.presentation.section_plan[0].composition,
            ReportSectionComposition::KeyPoints
        );
        assert_eq!(
            report.presentation.section_plan[1].composition,
            ReportSectionComposition::SourceLedger
        );
    }

    #[test]
    fn report_generation_preserves_positional_treatments_for_renamed_headings() {
        let markdown = format!(
            "# Report\n\n## Decision\n\n{}\n\n## Evidence base\n\n- https://example.com/source\n\n## Boundaries\n\nBounded evidence.",
            "Substantive source-backed analysis. ".repeat(5)
        );
        let output = serde_json::json!({ "object": valid_object(markdown) }).to_string();

        let report = deep_research_report_from_generation(&output, 0).unwrap();
        let headings = report
            .presentation
            .section_plan
            .iter()
            .map(|section| section.heading.as_str())
            .collect::<Vec<_>>();
        assert_eq!(headings, vec!["Decision", "Evidence base", "Boundaries"]);
        assert_eq!(
            report.presentation.section_plan[0].composition,
            ReportSectionComposition::KeyPoints
        );
        assert_eq!(
            report.presentation.section_plan[1].composition,
            ReportSectionComposition::SourceLedger
        );
    }

    #[test]
    fn report_generation_rejects_unstructured_markdown_instead_of_inventing_sections() {
        let markdown = "Substantive source-backed analysis. ".repeat(8);
        let output = serde_json::json!({ "object": valid_object(markdown) }).to_string();

        let error = deep_research_report_from_generation(&output, 0).unwrap_err();
        assert!(error.contains("level-two Markdown sections"), "{error}");
    }

    #[test]
    fn report_generation_ignores_heading_like_code_when_validating_the_section_plan() {
        let markdown = format!(
            "# Report\n\n## Findings\n\n{}\n\n```markdown\n## This is example code\n```\n\n    ## This is indented code\n\n## Sources\n\n- https://example.com/source\n\n## Limitations\n\nBounded evidence.",
            "Substantive source-backed analysis. ".repeat(5)
        );
        let output = serde_json::json!({ "object": valid_object(markdown) }).to_string();

        assert!(deep_research_report_from_generation(&output, 0).is_ok());
    }
}
