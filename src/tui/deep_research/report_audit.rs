//! Deterministic claim/source audit for a materialized research report.

use comrak::{nodes::NodeValue, parse_document, Arena, Options};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReportAudit {
    pub(crate) passed: bool,
    pub(crate) accepted_claims: usize,
    pub(crate) matched_claims: usize,
    pub(crate) claim_coverage_basis_points: u16,
    pub(crate) accepted_sources: usize,
    pub(crate) cited_sources: usize,
    pub(crate) reason: String,
}

pub(crate) fn audit_report(
    markdown: &str,
    html: &str,
    claims: &[String],
    source_anchors: &[String],
) -> ReportAudit {
    if claims.is_empty() && source_anchors.is_empty() {
        return ReportAudit {
            passed: true,
            accepted_claims: 0,
            matched_claims: 0,
            claim_coverage_basis_points: 10_000,
            accepted_sources: 0,
            cited_sources: 0,
            reason: "legacy report has no event-sourced evidence graph to audit".to_string(),
        };
    }
    let report = normalize(&format!("{markdown}\n{html}"));
    let matched_claims = claims
        .iter()
        .filter(|claim| claim_matches(&report, claim))
        .count();
    let claim_coverage_basis_points = if claims.is_empty() {
        10_000
    } else {
        ((matched_claims.saturating_mul(10_000) / claims.len()).min(10_000)) as u16
    };
    let citation_targets = extract_citation_targets(markdown, html);
    let cited_sources = source_anchors
        .iter()
        .filter(|anchor| {
            normalize_citation_target(anchor)
                .is_some_and(|anchor| citation_targets.contains(&anchor))
        })
        .count();
    let sources_pass = !source_anchors.is_empty() && cited_sources > 0;
    let claims_pass = claims.is_empty() || claim_coverage_basis_points >= 5_000;
    let passed = sources_pass && claims_pass;
    let reason = if !sources_pass {
        "report cites none of the accepted evidence sources"
    } else if !claims_pass {
        "report covers less than half of the accepted claims"
    } else {
        "report claims and citations trace to accepted evidence"
    };
    ReportAudit {
        passed,
        accepted_claims: claims.len(),
        matched_claims,
        claim_coverage_basis_points,
        accepted_sources: source_anchors.len(),
        cited_sources,
        reason: reason.to_string(),
    }
}

pub(crate) fn cites_source_anchor(markdown: &str, html: &str, anchor: &str) -> bool {
    let Some(anchor) = normalize_citation_target(anchor) else {
        return false;
    };
    extract_citation_targets(markdown, html).contains(&anchor)
}

fn extract_citation_targets(markdown: &str, html: &str) -> HashSet<String> {
    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.autolink = true;
    let root = parse_document(&arena, markdown, &options);
    let mut targets = HashSet::new();
    for node in root.descendants() {
        let data = node.data.borrow();
        match &data.value {
            NodeValue::Link(link) => {
                if let Some(target) = normalize_citation_target(&link.url) {
                    targets.insert(target);
                }
            }
            NodeValue::HtmlInline(fragment) => {
                extract_html_href_targets(fragment, &mut targets);
            }
            NodeValue::HtmlBlock(block) => {
                extract_html_href_targets(&block.literal, &mut targets);
            }
            _ => {}
        }
    }
    extract_html_href_targets(html, &mut targets);
    targets
}

fn extract_html_href_targets(document: &str, targets: &mut HashSet<String>) {
    for captures in html_href_regex().captures_iter(document) {
        let Some(target) = captures
            .name("double")
            .or_else(|| captures.name("single"))
            .or_else(|| captures.name("bare"))
            .map(|capture| decode_basic_html_entities(capture.as_str()))
            .and_then(|target| normalize_citation_target(&target))
        else {
            continue;
        };
        targets.insert(target);
    }
}

fn normalize_citation_target(target: &str) -> Option<String> {
    let target = target.trim();
    let target = target
        .strip_prefix('<')
        .and_then(|target| target.strip_suffix('>'))
        .unwrap_or(target)
        .trim();
    if target.is_empty() {
        return None;
    }
    if let Some(scheme_end) = target.find("://") {
        let scheme = target[..scheme_end].to_ascii_lowercase();
        let remainder = &target[scheme_end + 3..];
        let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
        let authority = remainder[..authority_end].to_ascii_lowercase();
        if authority.is_empty() {
            return None;
        }
        let suffix = &remainder[authority_end..];
        let suffix = if suffix.is_empty() { "/" } else { suffix };
        return Some(format!("{scheme}://{authority}{suffix}"));
    }
    Some(normalize_local_target(target))
}

fn normalize_local_target(target: &str) -> String {
    let target = target.replace('\\', "/");
    let suffix_start = target.find(['?', '#']).unwrap_or(target.len());
    let (path, suffix) = target.split_at(suffix_start);
    let absolute = path.starts_with('/');
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." if components.last().is_some_and(|last| *last != "..") => {
                components.pop();
            }
            ".." if !absolute => components.push(component),
            ".." => {}
            _ => components.push(component),
        }
    }
    let mut normalized = if absolute {
        format!("/{}", components.join("/"))
    } else {
        components.join("/")
    };
    if normalized.is_empty() && !absolute {
        normalized.push('.');
    }
    normalized.push_str(suffix);
    normalized
}

fn decode_basic_html_entities(target: &str) -> String {
    target
        .replace("&amp;", "&")
        .replace("&#38;", "&")
        .replace("&#x26;", "&")
        .replace("&#X26;", "&")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#x22;", "\"")
        .replace("&#X22;", "\"")
}

/// Reject quantitative assertions that are absent from the closed evidence
/// package. This is intentionally a host-side publication gate: a model can
/// paraphrase prose, but it cannot introduce a new threshold, range, version,
/// date, multiplier, or approximate magnitude and still publish the report.
pub(crate) fn validate_quantitative_grounding(
    markdown: &str,
    grounding_texts: &[String],
) -> Result<(), String> {
    let unsupported = unsupported_quantitative_atoms(markdown, grounding_texts);
    if unsupported.is_empty() {
        return Ok(());
    }
    Err(format!(
        "content rejected: report introduced ungrounded quantitative claim(s): {}",
        unsupported
            .into_iter()
            .take(8)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Preserve a checker-qualified report without preserving claims the evidence
/// gate rejected. Structural Markdown rows are removed as a unit; prose is
/// reduced sentence by sentence. A deterministic boundary note makes the
/// omission visible without spending another model turn or publishing a
/// generic recovery report.
pub(crate) fn sanitize_ungrounded_quantitative_claims(
    markdown: &str,
    grounding_texts: &[String],
) -> Option<String> {
    let mut changed = false;
    let mut output = Vec::new();
    for line in markdown.lines() {
        if unsupported_quantitative_atoms(line, grounding_texts).is_empty() {
            output.push(line.to_string());
            continue;
        }
        changed = true;
        let trimmed = line.trim_start();
        if trimmed.starts_with('#')
            || trimmed.starts_with('|')
            || trimmed.starts_with('>')
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("+ ")
            || ordered_prefix_regex().is_match(line)
        {
            continue;
        }
        let retained = line
            .split_inclusive(['.', '!', '?', '。', '！', '？', ';', '；'])
            .filter(|sentence| unsupported_quantitative_atoms(sentence, grounding_texts).is_empty())
            .map(str::trim)
            .filter(|sentence| !sentence.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !retained.is_empty() {
            output.push(retained);
        }
    }
    if !changed {
        return None;
    }
    while output.last().is_some_and(|line| line.trim().is_empty()) {
        output.pop();
    }
    let cjk = markdown.chars().any(is_cjk);
    output.push(String::new());
    if cjk {
        output.push("## 证据边界".to_string());
        output.push(String::new());
        output.push(
            "宿主证据校验已移除缺少已接纳来源依据的定量阈值或建议；不得从保留内容反推出被移除的阈值。"
                .to_string(),
        );
    } else {
        output.push("## Evidence boundary".to_string());
        output.push(String::new());
        output.push(
            "Host evidence validation removed quantitative thresholds or recommendations that were absent from the accepted source facts; no omitted threshold should be inferred from the retained report."
                .to_string(),
        );
    }
    Some(output.join("\n"))
}

fn unsupported_quantitative_atoms(text: &str, grounding_texts: &[String]) -> Vec<String> {
    let accepted = grounding_texts
        .iter()
        .flat_map(|text| quantitative_atoms(text))
        .collect::<HashSet<_>>();
    let mut unsupported = quantitative_atoms(text)
        .into_iter()
        .filter(|atom| !quantity_is_grounded(atom, &accepted))
        .collect::<Vec<_>>();
    unsupported.sort();
    unsupported.dedup();
    unsupported
}

pub(crate) fn quantitative_claim_is_grounded(text: &str, grounding_texts: &[String]) -> bool {
    let accepted = grounding_texts
        .iter()
        .flat_map(|text| quantitative_atoms(text))
        .collect::<HashSet<_>>();
    quantitative_atoms(text)
        .iter()
        .all(|atom| quantity_is_grounded(atom, &accepted))
}

fn quantity_is_grounded(atom: &str, accepted: &HashSet<String>) -> bool {
    if accepted.contains(atom) {
        return true;
    }
    // A non-comparative point may be quoted from a grounded range. The reverse
    // is unsafe: seeing `1M-10M` never licenses a new `<1M` recommendation.
    if has_comparator(atom) {
        return false;
    }
    accepted
        .iter()
        .filter(|candidate| !has_comparator(candidate))
        .any(|candidate| range_components(candidate).any(|part| part == atom))
}

fn has_comparator(atom: &str) -> bool {
    atom.starts_with(['<', '>', '~'])
        || atom.contains("以下")
        || atom.contains("以上")
        || atom.contains("以内")
        || atom.contains("左右")
        || atom.starts_with("数")
        || atom.starts_with("几")
        || atom.starts_with("多")
}

fn range_components(atom: &str) -> impl Iterator<Item = &str> {
    atom.split(['-', '–', '—', '~', '至', '到'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn quantitative_atoms(text: &str) -> Vec<String> {
    let scrubbed = scrub_markdown_scaffolding(text);
    let mut atoms = quantity_regex()
        .find_iter(&scrubbed)
        .map(|found| normalize_quantity(found.as_str()))
        .filter(|atom| material_quantity(atom))
        .collect::<Vec<_>>();
    atoms.extend(
        cjk_quantity_regex()
            .find_iter(&scrubbed)
            .map(|found| normalize_quantity(found.as_str()))
            .filter(|atom| material_cjk_quantity(atom)),
    );
    atoms.sort();
    atoms.dedup();
    atoms
}

fn scrub_markdown_scaffolding(text: &str) -> String {
    let without_urls = url_regex().replace_all(text, " ");
    let without_citations = citation_regex().replace_all(&without_urls, " ");
    ordered_prefix_regex()
        .replace_all(&without_citations, "${prefix}")
        .into_owned()
}

fn normalize_quantity(value: &str) -> String {
    let mut value = value
        .to_lowercase()
        .replace([' ', '\t', '\n', ',', '_'], "")
        .replace(['–', '—', '−', '－'], "-")
        .replace('×', "x");
    for (from, to) in [
        ("atleast", ">="),
        ("atmost", "<="),
        ("lessthan", "<"),
        ("morethan", ">"),
        ("upto", "<="),
        ("under", "<"),
        ("below", "<"),
        ("over", ">"),
        ("above", ">"),
        ("大约", "~"),
        ("至少", ">="),
        ("至多", "<="),
        ("低于", "<"),
        ("少于", "<"),
        ("高于", ">"),
        ("超过", ">"),
        ("about", "~"),
        ("around", "~"),
        ("约", "~"),
        ("≈", "~"),
        ("≤", "<="),
        ("≥", ">="),
    ] {
        value = value.replace(from, to);
    }
    for (suffix, prefix) in [("以下", "<"), ("以内", "<="), ("以上", ">"), ("左右", "~")] {
        if let Some(stem) = value.strip_suffix(suffix) {
            value = format!("{prefix}{stem}");
        }
    }
    value
}

fn material_quantity(atom: &str) -> bool {
    if atom.is_empty() {
        return false;
    }
    let digits = atom.chars().filter(char::is_ascii_digit).count();
    let plain_integer = atom.chars().all(|ch| ch.is_ascii_digit());
    !plain_integer || digits >= 4 || atom.parse::<u64>().is_ok_and(|value| value >= 1_000)
}

fn material_cjk_quantity(atom: &str) -> bool {
    atom.contains(['万', '亿'])
        || atom.contains("以下")
        || atom.contains("以上")
        || atom.contains("以内")
        || atom.contains("左右")
        || atom.starts_with(['数', '几', '多', '<', '>', '~'])
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{3040}'..='\u{30ff}'
            | '\u{ac00}'..='\u{d7af}'
    )
}

fn quantity_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?ix)
            (?:(?:at\s+least|at\s+most|less\s+than|more\s+than|up\s+to|under|over|below|above|about|around)\s*|[<>≤≥≈~]\s*|(?:低于|高于|少于|超过|至少|至多|约|大约)\s*)?
            v?\d[\d,_]*(?:\.\d+)*
            (?:\s*(?:vcpu|seconds?|minutes?|hours?|days?|years?|vectors?|bytes?|qps|rps|kib|mib|gib|tib|kb|mb|gb|tb|ms|sec|dims?|%|x|×|万|亿|维|倍|个|项|条|人|次|度|年|月|日|小时|分钟|秒|k|m|b))?
            (?:\s*(?:[-–—~至到/:])\s*v?\d[\d,_]*(?:\.\d+)*(?:\s*(?:vcpu|seconds?|minutes?|hours?|days?|years?|vectors?|bytes?|qps|rps|kib|mib|gib|tib|kb|mb|gb|tb|ms|sec|dims?|%|x|×|万|亿|维|倍|个|项|条|人|次|度|年|月|日|小时|分钟|秒|k|m|b))?){0,2}
            \s*(?:\+|以下|以上|以内|左右)?",
        )
        .expect("quantitative grounding regex must compile")
    })
}

fn cjk_quantity_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?:约|大约|超过|不足|低于|高于|至少|至多)?(?:数|几|多|[一二三四五六七八九两])?(?:十|百|千|万|亿){1,3}(?:余|多|级)?(?:以下|以上|以内|左右)?",
        )
        .expect("CJK quantitative grounding regex must compile")
    })
}

fn url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"https?://[^\s<>()]+").expect("URL regex must compile"))
}

fn citation_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\[(?:\d+(?:\s*[-,–]\s*\d+)*)\]").expect("citation regex must compile")
    })
}

fn ordered_prefix_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?m)^(?P<prefix>\s*(?:#{1,6}\s*)?)\d{1,3}[.)、]\s+")
            .expect("ordered-list prefix regex must compile")
    })
}

fn html_href_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?is)\bhref\s*=\s*(?:\"(?P<double>[^\"]*)\"|'(?P<single>[^']*)'|(?P<bare>[^\s\"'=<>`]+))"#,
        )
        .expect("HTML href regex must compile")
    })
}

fn claim_matches(report: &str, claim: &str) -> bool {
    let claim = normalize(claim);
    if claim.is_empty() || report.contains(&claim) {
        return !claim.is_empty();
    }
    let terms = claim
        .split_whitespace()
        .filter(|term| term.len() >= 3)
        .collect::<HashSet<_>>();
    if terms.is_empty() {
        return false;
    }
    let matched = terms
        .iter()
        .filter(|term| report.split_whitespace().any(|word| word == **term))
        .count();
    matched.saturating_mul(100) / terms.len() >= 60
}

fn normalize(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_report_with_source_and_claim_coverage() {
        let audit = audit_report(
            "The release date is July 12. Source: https://example.gov/release",
            "<p>The release was published on July 12.</p>",
            &["The release date is July 12.".to_string()],
            &["https://example.gov/release".to_string()],
        );
        assert!(audit.passed);
        assert_eq!(audit.claim_coverage_basis_points, 10_000);
    }

    #[test]
    fn rejects_polished_report_without_accepted_claims() {
        let audit = audit_report(
            "A polished but unrelated conclusion. https://example.gov/release",
            "<p>Unrelated analysis.</p>",
            &["The release date is July 12.".to_string()],
            &["https://example.gov/release".to_string()],
        );
        assert!(!audit.passed);
        assert!(audit.reason.contains("less than half"));
    }

    #[test]
    fn rejects_source_anchor_that_is_only_a_prefix_of_a_link_target() {
        let audit = audit_report(
            "The release date is July 12. [Source](https://example.gov/release-notes)",
            "",
            &["The release date is July 12.".to_string()],
            &["https://example.gov/release".to_string()],
        );
        assert!(!audit.passed);
        assert_eq!(audit.cited_sources, 0);
        assert!(audit.reason.contains("cites none"));
    }

    #[test]
    fn extracts_inline_autolink_and_reference_citation_targets() {
        for markdown in [
            "[Source](https://example.gov/release)",
            "<https://example.gov/release>",
            "https://example.gov/release",
            "[Source][release]\n\n[release]: https://example.gov/release",
        ] {
            let audit = audit_report(
                markdown,
                "",
                &[],
                &["https://example.gov/release".to_string()],
            );
            assert!(audit.passed, "{markdown}: {}", audit.reason);
            assert_eq!(audit.cited_sources, 1, "{markdown}");
        }
    }

    #[test]
    fn local_citation_targets_match_exact_normalized_paths() {
        let accepted = audit_report(
            "[Workspace source](docs/./research.md)",
            "",
            &[],
            &["./docs/research.md".to_string()],
        );
        assert!(accepted.passed, "{}", accepted.reason);

        let rejected = audit_report(
            "[Nested readme](docs/README.md)",
            "",
            &[],
            &["README.md".to_string()],
        );
        assert!(!rejected.passed);
        assert_eq!(rejected.cited_sources, 0);
    }

    #[test]
    fn link_like_text_inside_a_code_fence_is_not_a_citation() {
        let audit = audit_report(
            "```html\n<a href=\"https://example.gov/release\">not a citation</a>\n```",
            "",
            &[],
            &["https://example.gov/release".to_string()],
        );
        assert!(!audit.passed);
        assert_eq!(audit.cited_sources, 0);
    }

    #[test]
    fn rejects_new_threshold_even_when_the_magnitude_appears_in_a_range() {
        let error = validate_quantitative_grounding(
            "Use pgvector below 1M vectors; the benchmark covers 1M-10M vectors.",
            &["The benchmark covers 1M-10M vectors.".to_string()],
        )
        .unwrap_err();
        assert!(error.contains("<1m"), "{error}");
    }

    #[test]
    fn rejects_the_unverified_threshold_forms_seen_in_a_real_report() {
        let error = validate_quantitative_grounding(
            "向量规模 < 100 万；百万级以下可选 A；> 数百万考虑 B；规模 <1 GB。",
            &["公开基准覆盖 1M-10M 向量，最大样例约 8.6GB。".to_string()],
        )
        .unwrap_err();
        assert!(error.contains("<100万"), "{error}");
        assert!(error.contains("数百万"), "{error}");
        assert!(error.contains("<1gb"), "{error}");
    }

    #[test]
    fn accepts_grounded_ranges_versions_dates_and_multipliers() {
        validate_quantitative_grounding(
            "Version v0.8.5 was reviewed in 2026. The 1M-10M test reports 10x.",
            &["v0.8.5; 2026; 1M-10M vectors; 10x".to_string()],
        )
        .unwrap();
    }

    #[test]
    fn ignores_markdown_list_and_citation_numbers() {
        validate_quantitative_grounding(
            "1. First point [1]\n2. Second point [2]\n\n[1] https://example.com/2026/item",
            &[],
        )
        .unwrap();
    }

    #[test]
    fn qualified_sanitization_removes_only_ungrounded_sentences() {
        let sanitized = sanitize_ungrounded_quantitative_claims(
            "# Report\n\nUse A below 1M vectors. The benchmark covers 1M-10M vectors.\n\n| Choice | Boundary |\n| --- | --- |\n| A | below 1M |\n\n## Sources\n\n- https://example.com/benchmark",
            &["The benchmark covers 1M-10M vectors.".to_string()],
        )
        .expect("the unsupported threshold should be removed");
        assert!(!sanitized.contains("below 1M"), "{sanitized}");
        assert!(sanitized.contains("covers 1M-10M"), "{sanitized}");
        assert!(sanitized.contains("## Evidence boundary"), "{sanitized}");
        validate_quantitative_grounding(
            &sanitized,
            &["The benchmark covers 1M-10M vectors.".to_string()],
        )
        .unwrap();
    }
}
