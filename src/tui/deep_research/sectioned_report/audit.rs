//! Closed-evidence resolution and deterministic section audit.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use a3s::research::OutlineSection;
use comrak::{nodes::NodeValue, parse_document, Arena, Options};
use regex::Regex;

use super::super::deep_research_evidence_ledger::AcceptedSource;
use super::super::deep_research_report_audit::{
    audit_report, canonical_citation_target, report_citation_targets, CitationRequirement,
    ReportSourceReference,
};
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
    pub(super) claim_source_ids: BTreeMap<String, BTreeSet<String>>,
    pub(super) source_anchors: BTreeMap<String, String>,
}

impl ResolvedEvidence {
    pub(super) fn report_sources(&self) -> Vec<ReportSourceReference> {
        self.source_anchors
            .iter()
            .map(|(source_id, anchor)| ReportSourceReference {
                source_id: source_id.clone(),
                anchor: anchor.clone(),
            })
            .collect()
    }
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
    if declared_source_ids.is_empty() {
        return Err(format!(
            "section `{}` did not cite any source from its committed source catalog",
            section.section_id
        ));
    }
    if !declared_source_ids.is_subset(&expected_source_ids) {
        let unexpected = declared_source_ids
            .difference(&expected_source_ids)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "section `{}` cited sources outside its committed source catalog (unexpected: {})",
            section.section_id,
            unexpected.join(", ")
        ));
    }
    Ok(())
}

/// Apply syntax-level report invariants and derive the section's actual source
/// set from exact inline citation anchors. The outline remains the closed
/// allowlist; it is not itself a requirement to cite every alternative source.
pub(super) fn materialize_section_candidate(
    section: &mut SectionGeneration,
    planned: &OutlineSection,
    evidence: &[AcceptedEvidence],
) -> Result<(), String> {
    let planned_claim_ids = planned.claim_ids.iter().cloned().collect::<BTreeSet<_>>();
    let planned_source_ids = planned.source_ids.iter().cloned().collect::<BTreeSet<_>>();
    let resolved = resolve_evidence_ids(&planned_claim_ids, &planned_source_ids, evidence)?;
    section.markdown = normalize_section_markdown(&section.markdown);
    section.markdown =
        normalize_exact_bracketed_citations(&section.markdown, &resolved.source_anchors);
    if contains_forbidden_section_heading(&section.markdown) {
        return Err(format!(
            "section `{}` contains a level-one or level-two Markdown heading after deterministic normalization",
            section.section_id
        ));
    }
    validate_committed_date_literals(
        &section.section_id,
        &section.markdown,
        &planned_claim_ids,
        evidence,
    )?;

    let citation_targets = report_citation_targets(&section.markdown, "");
    section.source_ids = planned
        .source_ids
        .iter()
        .filter_map(|source_id| {
            let anchor = resolved.source_anchors.get(source_id)?;
            canonical_citation_target(anchor)
                .filter(|target| citation_targets.contains(target))
                .map(|_| source_id.clone())
        })
        .collect();
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct NormalizedDate {
    year: u32,
    month: u8,
    day: u8,
}

fn validate_committed_date_literals(
    section_id: &str,
    markdown: &str,
    claim_ids: &BTreeSet<String>,
    evidence: &[AcceptedEvidence],
) -> Result<(), String> {
    let accepted_dates = evidence
        .iter()
        .flat_map(|item| &item.claims)
        .filter(|claim| claim_ids.contains(&claim.id))
        .flat_map(|claim| normalized_dates(&claim.text))
        .map(|(date, _)| date)
        .collect::<BTreeSet<_>>();
    let visible_text = markdown_visible_text(markdown);
    for (date, literal) in normalized_dates(&visible_text) {
        if !accepted_dates.contains(&date) {
            return Err(format!(
                "section `{section_id}` contains date literal `{literal}` that is not present in its committed accepted claims"
            ));
        }
    }
    Ok(())
}

fn markdown_visible_text(markdown: &str) -> String {
    let arena = Arena::new();
    let root = parse_document(&arena, markdown, &Options::default());
    root.descendants()
        .filter_map(|node| match &node.data.borrow().value {
            NodeValue::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalized_dates(value: &str) -> Vec<(NormalizedDate, String)> {
    static ISO_DATE: OnceLock<Regex> = OnceLock::new();
    static CHINESE_DATE: OnceLock<Regex> = OnceLock::new();
    static MONTH_FIRST_DATE: OnceLock<Regex> = OnceLock::new();
    static DAY_FIRST_DATE: OnceLock<Regex> = OnceLock::new();
    let iso_date = ISO_DATE.get_or_init(|| {
        Regex::new(
            r"(?:^|[^0-9])(?P<literal>(?P<year>[0-9]{1,6})-(?P<month>[0-9]{1,2})-(?P<day>[0-9]{1,2}))(?:$|[^0-9])",
        )
        .expect("valid ISO date regex")
    });
    let chinese_date = CHINESE_DATE.get_or_init(|| {
        Regex::new(
            r"(?P<literal>(?P<year>[0-9]{1,6})\s*年\s*(?P<month>[0-9]{1,2})\s*月\s*(?P<day>[0-9]{1,2})\s*日)",
        )
        .expect("valid Chinese date regex")
    });
    let month_first_date = MONTH_FIRST_DATE.get_or_init(|| {
        Regex::new(
            r"(?i)(?P<literal>\b(?P<month_name>January|February|March|April|May|June|July|August|September|October|November|December)\s+(?P<day>[0-9]{1,2})(?:st|nd|rd|th)?[,]?\s+(?P<year>[0-9]{1,6})\b)",
        )
        .expect("valid month-first date regex")
    });
    let day_first_date = DAY_FIRST_DATE.get_or_init(|| {
        Regex::new(
            r"(?i)(?P<literal>\b(?P<day>[0-9]{1,2})(?:st|nd|rd|th)?\s+(?P<month_name>January|February|March|April|May|June|July|August|September|October|November|December)[,]?\s+(?P<year>[0-9]{1,6})\b)",
        )
        .expect("valid day-first date regex")
    });
    let mut dates = Vec::new();
    for captures in iso_date.captures_iter(value) {
        push_numeric_date(&mut dates, &captures);
    }
    for captures in chinese_date.captures_iter(value) {
        push_numeric_date(&mut dates, &captures);
    }
    for captures in month_first_date.captures_iter(value) {
        push_named_month_date(&mut dates, &captures);
    }
    for captures in day_first_date.captures_iter(value) {
        push_named_month_date(&mut dates, &captures);
    }
    dates
}

fn push_numeric_date(dates: &mut Vec<(NormalizedDate, String)>, captures: &regex::Captures<'_>) {
    let Some(year) = capture_u32(captures, "year") else {
        return;
    };
    let Some(month) = capture_u8(captures, "month") else {
        return;
    };
    let Some(day) = capture_u8(captures, "day") else {
        return;
    };
    push_date(dates, captures, year, month, day);
}

fn push_named_month_date(
    dates: &mut Vec<(NormalizedDate, String)>,
    captures: &regex::Captures<'_>,
) {
    let Some(year) = capture_u32(captures, "year") else {
        return;
    };
    let Some(day) = capture_u8(captures, "day") else {
        return;
    };
    let Some(month) = captures
        .name("month_name")
        .and_then(|value| english_month(value.as_str()))
    else {
        return;
    };
    push_date(dates, captures, year, month, day);
}

fn push_date(
    dates: &mut Vec<(NormalizedDate, String)>,
    captures: &regex::Captures<'_>,
    year: u32,
    month: u8,
    day: u8,
) {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return;
    }
    if let Some(literal) = captures.name("literal") {
        dates.push((
            NormalizedDate { year, month, day },
            literal.as_str().to_string(),
        ));
    }
}

fn capture_u32(captures: &regex::Captures<'_>, name: &str) -> Option<u32> {
    captures.name(name)?.as_str().parse().ok()
}

fn capture_u8(captures: &regex::Captures<'_>, name: &str) -> Option<u8> {
    captures.name(name)?.as_str().parse().ok()
}

fn english_month(value: &str) -> Option<u8> {
    match value.to_ascii_lowercase().as_str() {
        "january" => Some(1),
        "february" => Some(2),
        "march" => Some(3),
        "april" => Some(4),
        "may" => Some(5),
        "june" => Some(6),
        "july" => Some(7),
        "august" => Some(8),
        "september" => Some(9),
        "october" => Some(10),
        "november" => Some(11),
        "december" => Some(12),
        _ => None,
    }
}

/// Some providers render an exact source URL as `[https://…]`, which is
/// citation-shaped text but not a CommonMark link. Canonicalize only an exact
/// accepted anchor outside fenced and inline code. This preserves the closed
/// source catalog without inferring citation intent from prose or language.
pub(super) fn normalize_exact_bracketed_citations(
    markdown: &str,
    source_anchors: &BTreeMap<String, String>,
) -> String {
    let mut anchors = source_anchors
        .values()
        .filter(|anchor| !anchor.is_empty())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    anchors.sort_by_key(|anchor| std::cmp::Reverse(anchor.len()));
    let descendant_rewrites = descendant_citation_rewrites(markdown, &anchors);

    let mut active_fence = None::<(char, usize)>;
    markdown
        .split('\n')
        .map(|line| {
            if let Some((marker, minimum)) = active_fence {
                if closing_fence(line, marker, minimum) {
                    active_fence = None;
                }
                return line.to_string();
            }
            if let Some(fence) = opening_fence(line) {
                active_fence = Some(fence);
                return line.to_string();
            }
            normalize_bracketed_citations_in_line(line, &anchors, &descendant_rewrites)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A writer occasionally expands a committed collection URL into a child URL
/// copied from claim text. Resolve only a strict same-origin path descendant
/// back to the longest committed parent. Lexical siblings such as `release`
/// versus `release-notes`, root-domain parents, and code spans remain exact and
/// are never promoted.
fn descendant_citation_rewrites(markdown: &str, anchors: &[String]) -> Vec<(String, String)> {
    let exact = anchors
        .iter()
        .filter_map(|anchor| canonical_citation_target(anchor))
        .collect::<BTreeSet<_>>();
    let mut rewrites = report_citation_targets(markdown, "")
        .into_iter()
        .filter(|target| !exact.contains(target))
        .filter_map(|target| {
            let replacement = anchors
                .iter()
                .filter(|anchor| strict_http_source_descendant(&target, anchor))
                .max_by_key(|anchor| {
                    reqwest::Url::parse(anchor)
                        .map(|url| url.path().len())
                        .unwrap_or_default()
                })?
                .clone();
            Some((target, replacement))
        })
        .collect::<Vec<_>>();
    rewrites.sort_by_key(|(target, _)| std::cmp::Reverse(target.len()));
    rewrites
}

fn strict_http_source_descendant(target: &str, anchor: &str) -> bool {
    let Ok(target) = reqwest::Url::parse(target) else {
        return false;
    };
    let Ok(anchor) = reqwest::Url::parse(anchor) else {
        return false;
    };
    if !matches!(anchor.scheme(), "http" | "https")
        || target.scheme() != anchor.scheme()
        || target.username() != anchor.username()
        || target.password() != anchor.password()
        || target.host_str() != anchor.host_str()
        || target.port_or_known_default() != anchor.port_or_known_default()
        || anchor.query().is_some()
        || anchor.fragment().is_some()
    {
        return false;
    }
    let anchor_path = anchor.path().trim_end_matches('/');
    if anchor_path.is_empty() {
        return false;
    }
    target
        .path()
        .strip_prefix(anchor_path)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

fn normalize_bracketed_citations_in_line(
    line: &str,
    anchors: &[String],
    descendant_rewrites: &[(String, String)],
) -> String {
    let mut normalized = String::with_capacity(line.len());
    let mut cursor = 0;
    let mut inline_code_delimiter = None::<usize>;
    while cursor < line.len() {
        if line.as_bytes()[cursor] == b'`' {
            let count = line.as_bytes()[cursor..]
                .iter()
                .take_while(|byte| **byte == b'`')
                .count();
            match inline_code_delimiter {
                Some(active) if active == count => inline_code_delimiter = None,
                None => inline_code_delimiter = Some(count),
                Some(_) => {}
            }
            normalized.push_str(&line[cursor..cursor + count]);
            cursor += count;
            continue;
        }
        if inline_code_delimiter.is_none() {
            if let Some((target, replacement)) = descendant_rewrites
                .iter()
                .find(|(target, _)| line[cursor..].starts_with(target))
            {
                normalized.push_str(replacement);
                cursor += target.len();
                continue;
            }
        }
        if inline_code_delimiter.is_none() && line.as_bytes()[cursor] == b'[' {
            let escaped_or_image =
                cursor > 0 && matches!(line.as_bytes()[cursor - 1], b'\\' | b'!');
            if !escaped_or_image {
                let matched = anchors.iter().find(|anchor| {
                    let suffix = &line[cursor + 1..];
                    if !suffix.starts_with(anchor.as_str())
                        || suffix.as_bytes().get(anchor.len()) != Some(&b']')
                    {
                        return false;
                    }
                    let following = &suffix[anchor.len() + 1..];
                    !following.starts_with('(')
                        && (!following.starts_with('[')
                            || starts_with_exact_bracketed_anchor(following, anchors))
                });
                if let Some(anchor) = matched {
                    normalized.push('<');
                    normalized.push_str(anchor);
                    normalized.push('>');
                    cursor += anchor.len() + 2;
                    continue;
                }
            }
        }
        let character = line[cursor..]
            .chars()
            .next()
            .expect("cursor remains on a character boundary");
        normalized.push(character);
        cursor += character.len_utf8();
    }
    normalized
}

fn starts_with_exact_bracketed_anchor(value: &str, anchors: &[String]) -> bool {
    let Some(suffix) = value.strip_prefix('[') else {
        return false;
    };
    anchors.iter().any(|anchor| {
        suffix.starts_with(anchor.as_str())
            && suffix
                .as_bytes()
                .get(anchor.len())
                .is_some_and(|byte| *byte == b']')
    })
}

/// Section bodies are nested below a Host-owned H2. Demote model-authored H1
/// and H2 syntax without changing prose, links, or fenced code. A structural
/// AST check after this pass catches uncommon container forms and routes them
/// through the bounded section revision path.
pub(super) fn normalize_section_markdown(markdown: &str) -> String {
    let lines = markdown.split('\n').collect::<Vec<_>>();
    let mut normalized = Vec::with_capacity(lines.len());
    let mut active_fence = None::<(char, usize)>;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if let Some((marker, minimum)) = active_fence {
            normalized.push(line.to_string());
            if closing_fence(line, marker, minimum) {
                active_fence = None;
            }
            index += 1;
            continue;
        }
        if let Some(fence) = opening_fence(line) {
            active_fence = Some(fence);
            normalized.push(line.to_string());
            index += 1;
            continue;
        }
        if index + 1 < lines.len()
            && !line.trim().is_empty()
            && is_setext_heading_underline(lines[index + 1])
        {
            normalized.push(format!("### {}", line.trim()));
            index += 2;
            continue;
        }
        normalized.push(demote_atx_heading(line));
        index += 1;
    }
    normalized.join("\n")
}

fn demote_atx_heading(line: &str) -> String {
    let indentation = line.bytes().take_while(|byte| *byte == b' ').count();
    if indentation > 3 {
        return line.to_string();
    }
    let rest = &line[indentation..];
    let marker_count = rest.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=2).contains(&marker_count) {
        return line.to_string();
    }
    let suffix = &rest[marker_count..];
    if !suffix.is_empty() && !suffix.starts_with(char::is_whitespace) {
        return line.to_string();
    }
    format!("{}###{}", &line[..indentation], suffix)
}

fn opening_fence(line: &str) -> Option<(char, usize)> {
    let rest = markdown_block_prefix(line)?;
    let marker = rest.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let count = rest.chars().take_while(|value| *value == marker).count();
    (count >= 3).then_some((marker, count))
}

fn closing_fence(line: &str, marker: char, minimum: usize) -> bool {
    let Some(rest) = markdown_block_prefix(line) else {
        return false;
    };
    let count = rest.chars().take_while(|value| *value == marker).count();
    count >= minimum && rest[count..].trim().is_empty()
}

fn is_setext_heading_underline(line: &str) -> bool {
    let Some(rest) = markdown_block_prefix(line) else {
        return false;
    };
    let marker = match rest.chars().next() {
        Some(marker @ ('=' | '-')) => marker,
        _ => return false,
    };
    let count = rest.chars().take_while(|value| *value == marker).count();
    count > 0 && rest[count..].trim().is_empty()
}

fn markdown_block_prefix(line: &str) -> Option<&str> {
    let indentation = line.bytes().take_while(|byte| *byte == b' ').count();
    (indentation <= 3).then_some(&line[indentation..])
}

fn contains_forbidden_section_heading(markdown: &str) -> bool {
    let arena = Arena::new();
    let root = parse_document(&arena, markdown, &Options::default());
    root.descendants().any(|node| {
        matches!(
            &node.data.borrow().value,
            NodeValue::Heading(heading) if heading.level <= 2
        )
    })
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
    let bound_claim_ids = resolved
        .claim_source_ids
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if bound_claim_ids != claim_ids || resolved.claim_source_ids.values().any(BTreeSet::is_empty) {
        return Err(format!(
            "section `{}` does not have an exact accepted source binding for every declared claim ID",
            section.section_id
        ));
    }
    let audit = audit_report(
        &section.markdown,
        "",
        &resolved.report_sources(),
        CitationRequirement::EveryDeclared,
    );
    if !audit.passed {
        return Err(format!(
            "section `{}` failed evidence audit: {}",
            section.section_id, audit.reason
        ));
    }
    Ok(resolved)
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

    for id in claim_ids {
        if !claims_by_id.contains_key(id.as_str()) {
            return Err(format!("declared claim ID `{id}` is not accepted evidence"));
        }
    }
    let source_anchors = source_ids
        .iter()
        .map(|id| {
            sources_by_id
                .get(id.as_str())
                .map(|anchor| (id.clone(), (*anchor).to_string()))
                .ok_or_else(|| format!("declared source ID `{id}` is not accepted evidence"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut claim_source_ids = BTreeMap::new();
    for claim_id in claim_ids {
        let bound_sources = evidence
            .iter()
            .filter(|item| item.claims.iter().any(|claim| claim.id == *claim_id))
            .flat_map(|item| &item.sources)
            .filter(|source| source_ids.contains(&source.id))
            .map(|source| source.id.clone())
            .collect::<BTreeSet<_>>();
        if bound_sources.is_empty() {
            let accepted_anchors = evidence
                .iter()
                .filter(|item| item.claims.iter().any(|claim| claim.id == *claim_id))
                .flat_map(|item| &item.sources)
                .map(|source| source.anchor.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "declared claim ID `{claim_id}` has no declared source from the same accepted evidence item; cite one accepted same-item source: {accepted_anchors}"
            ));
        }
        claim_source_ids.insert(claim_id.clone(), bound_sources);
    }
    Ok(ResolvedEvidence {
        claim_ids: claim_ids.clone(),
        source_ids: source_ids.clone(),
        claim_source_ids,
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
