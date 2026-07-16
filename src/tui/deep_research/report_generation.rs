//! Closed-evidence structured generation for the final DeepResearch report.

use serde::{Deserialize, Serialize};

const REPORT_MIN_CHARS: usize = 120;
const REPORT_MAX_CHARS: usize = 60_000;

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
                    "description": "The complete source-backed human-facing report in Markdown."
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
                            "maxLength": 500,
                            "description": "A concise private explanation tying the choices to content structure and audience. It is never rendered."
                        }
                    },
                    "required": ["narrative_mode", "archetype", "palette", "density", "hero", "visual_stance", "rationale"]
                }
            },
            "required": ["markdown", "editorial", "presentation"]
        },
        "schema_name": "deep_research_report",
        "schema_description": "A complete evidence-grounded DeepResearch report plus a semantic coverage map and content-driven report-master presentation lock",
        "prompt": prompt,
        "system": "You are a closed-evidence research writer and report art director. Return only the requested object. Spend the completion budget on a genuinely useful Markdown report; keep each private coverage field to one or two precise sentences. Audit every planned track through finding, interpretation, implication, and uncertainty. Choose presentation from the argument, audience, evidence shape, and reading occasion; do not reuse a default style or infer design from topic keywords. Do not invoke or discuss tools, delegation, files, workflows, or the writing process.",
        "mode": "tool",
        "max_repair_attempts": 1,
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
    Ok(report)
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
                "rationale": "A decision-first analytical treatment fits the compact evidence set."
            }
        })
    }

    #[test]
    fn report_generation_forces_one_closed_structured_output() {
        let args = deep_research_report_generation_args("Write the report", 160_000);

        assert_eq!(args["mode"], "tool");
        assert_eq!(args["timeout_ms"], 160_000);
        assert_eq!(args["max_repair_attempts"], 1);
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
        assert!(args["system"]
            .as_str()
            .is_some_and(|system| system.contains("do not reuse a default style")));
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
}
