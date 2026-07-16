//! Closed-evidence structured generation for the final DeepResearch report.

use serde::{Deserialize, Serialize};

const REPORT_MIN_CHARS: usize = 120;
const REPORT_MAX_CHARS: usize = 30_000;

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
    pub(crate) track: String,
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

pub(super) fn deep_research_report_generation_args(
    prompt: &str,
    timeout_ms: u64,
) -> serde_json::Value {
    serde_json::json!({
        "schema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "markdown": {
                    "type": "string",
                    "minLength": REPORT_MIN_CHARS,
                    "maxLength": REPORT_MAX_CHARS,
                    "description": "The complete source-backed human-facing report in Markdown. Aim for 1,500-3,000 words or the equivalent in the query language, normally using 4-8 level-two sections; finish the bounded report instead of expanding toward the maximum."
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
                            "description": "One entry for every planned research track. Reuse the full track name when practical; otherwise use a concise, unambiguous semantic label. This is a private quality map, not report prose.",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "track": { "type": "string", "minLength": 2, "maxLength": 180 },
                                    "status": { "type": "string", "enum": ["answered", "bounded"] },
                                    "finding": { "type": "string", "minLength": 8, "maxLength": 600 },
                                    "interpretation": { "type": "string", "minLength": 8, "maxLength": 600 },
                                    "implication": { "type": "string", "maxLength": 600 },
                                    "uncertainty": { "type": "string", "maxLength": 600 }
                                },
                                "required": ["track", "status", "finding", "interpretation", "implication", "uncertainty"]
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
                                        "enum": ["anchor", "dense", "breathing"]
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
            "required": ["markdown", "editorial", "presentation"]
        },
        "schema_name": "deep_research_report",
        "schema_description": "A complete evidence-grounded DeepResearch report plus a semantic coverage map and content-driven report-master presentation lock",
        "prompt": prompt,
        "system": "You are a closed-evidence research writer. Return only the requested object and prioritize the report content. Finish a useful bounded Markdown report in this call; keep each private coverage field to one or two precise sentences. Audit every planned track through finding, interpretation, implication, and uncertainty. Only supported checker track and stop-condition assessments are checked findings; bounded or uncovered assessments remain gaps. Omit unsupported domain knowledge entirely, even when it could be labeled as common knowledge, inference, likely, or unverified. When the checked evidence cannot support the requested conclusion, say so in the thesis and provide decision criteria and evidence gaps instead of a ranking. After the Markdown outline is final, choose the small global presentation lock and one compact section-plan entry per exact H2. Select rhythm and composition from each section's information relationship and reader use; the host safely renders those choices. Do not generate HTML or CSS. Do not invoke or discuss tools, delegation, files, workflows, or the writing process.",
        "mode": "tool",
        "max_repair_attempts": 0,
        "timeout_ms": timeout_ms.clamp(1_000, 600_000)
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
                    "track": "Current status",
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
    fn report_generation_forces_one_closed_structured_output() {
        let args = deep_research_report_generation_args("Write the report", 160_000);

        assert_eq!(args["mode"], "tool");
        assert_eq!(args["timeout_ms"], 160_000);
        assert_eq!(args["max_repair_attempts"], 0);
        assert_eq!(args["schema"]["additionalProperties"], false);
        assert_eq!(
            args["schema"]["properties"]["markdown"]["maxLength"],
            REPORT_MAX_CHARS
        );
        assert_eq!(
            args["schema"]["properties"]["presentation"]["properties"]["archetype"]["enum"],
            serde_json::json!([
                "editorial",
                "analytical",
                "chronicle",
                "executive",
                "field-notes"
            ])
        );
        assert_eq!(
            args["schema"]["properties"]["presentation"]["properties"]["section_plan"]["items"]
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
        assert!(args["schema"]["properties"]["presentation"]["required"]
            .as_array()
            .is_some_and(|required| required.contains(&serde_json::json!("section_plan"))));
        assert!(args["system"].as_str().is_some_and(|system| system
            .to_ascii_lowercase()
            .contains("prioritize the report content")));
        assert!(args["system"].as_str().is_some_and(|system| {
            system.contains("one compact section-plan entry per exact H2")
        }));
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
