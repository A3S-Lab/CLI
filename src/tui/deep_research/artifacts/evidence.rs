use std::collections::HashSet;

use super::{
    canonical_research_source_anchor, deep_research_output_has_internal_leak,
    deep_research_workflow_metadata_digest, deep_research_workflow_output_digest,
    markdown_plain_text,
};

#[derive(Clone, Debug)]
pub(super) struct StructuredEvidenceItem {
    pub(super) summary: String,
    pub(super) sources: Vec<StructuredEvidenceSource>,
    key_evidence: Vec<String>,
    contradictions: Vec<String>,
    gaps: Vec<String>,
    confidence: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct StructuredEvidenceSource {
    pub(super) title: Option<String>,
    pub(super) url_or_path: String,
    pub(super) date: Option<String>,
    pub(super) quote_or_fact: Option<String>,
    pub(super) reliability: Option<String>,
}

pub(super) fn deep_research_structured_evidence_from_workflow(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Vec<StructuredEvidenceItem> {
    let mut roots = Vec::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(workflow_output) {
        roots.push(deep_research_workflow_output_digest(&value));
    }
    if let Some(metadata) = workflow_metadata {
        roots.push(deep_research_workflow_metadata_digest(metadata));
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        collect_structured_evidence_items(&root, &mut out, &mut seen);
    }
    out
}

fn collect_structured_evidence_items(
    value: &serde_json::Value,
    out: &mut Vec<StructuredEvidenceItem>,
    seen: &mut HashSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(item) = structured_evidence_item_from_value(value) {
                let first_source = item
                    .sources
                    .first()
                    .map(|source| source.url_or_path.clone())
                    .unwrap_or_default();
                let key = format!("{}|{}", item.summary.to_ascii_lowercase(), first_source);
                if seen.insert(key) {
                    out.push(item);
                }
            }
            for (key, child) in map {
                if matches!(
                    key.as_str(),
                    "query"
                        | "input"
                        | "history"
                        | "output_summary"
                        | "error_summary"
                        | "collection_error"
                ) {
                    continue;
                }
                collect_structured_evidence_items(child, out, seen);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_structured_evidence_items(item, out, seen);
            }
        }
        _ => {}
    }
}

pub(crate) fn parse_embedded_structured_evidence_json(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.len() < 8 {
        return None;
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            return Some(value);
        }
    }
    if !(trimmed.contains("\"summary\"") && trimmed.contains("\"sources\"")) {
        return None;
    }
    for (start, ch) in trimmed.char_indices() {
        if !matches!(ch, '{' | '[') {
            continue;
        }
        let Some(end) = balanced_json_end(trimmed, start) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[start..end]) {
            return Some(value);
        }
    }
    None
}

fn balanced_json_end(text: &str, start: usize) -> Option<usize> {
    let opener = text.get(start..)?.chars().next()?;
    let expected = match opener {
        '{' => '}',
        '[' => ']',
        _ => return None,
    };
    let mut stack = vec![expected];
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in text[start..].char_indices().skip(1) {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' if stack.pop() == Some(ch) => {
                if stack.is_empty() {
                    return Some(start + offset + ch.len_utf8());
                }
            }
            '}' | ']' => return None,
            _ => {}
        }
    }
    None
}

fn structured_evidence_item_from_value(
    value: &serde_json::Value,
) -> Option<StructuredEvidenceItem> {
    let summary = clean_report_text(value.get("summary")?.as_str()?, 900)?;
    let sources = value
        .get("sources")?
        .as_array()?
        .iter()
        .filter_map(structured_evidence_source_from_value)
        .collect::<Vec<_>>();
    if sources.is_empty() {
        return None;
    }

    Some(StructuredEvidenceItem {
        summary,
        sources,
        key_evidence: clean_report_string_array(value.get("key_evidence"), 8, 500),
        contradictions: clean_report_string_array(value.get("contradictions"), 8, 500),
        gaps: clean_report_string_array(value.get("gaps"), 8, 500),
        confidence: value
            .get("confidence")
            .and_then(serde_json::Value::as_str)
            .and_then(|text| clean_report_text(text, 500)),
    })
}

fn structured_evidence_source_from_value(
    value: &serde_json::Value,
) -> Option<StructuredEvidenceSource> {
    let url_or_path = clean_report_text(
        value
            .get("url_or_path")
            .or_else(|| value.get("url"))
            .or_else(|| value.get("path"))?
            .as_str()?,
        700,
    )?;
    let url_or_path = canonical_research_source_anchor(&url_or_path)?;
    Some(StructuredEvidenceSource {
        title: value
            .get("title")
            .and_then(serde_json::Value::as_str)
            .and_then(|text| clean_report_text(text, 220)),
        url_or_path,
        date: value
            .get("date")
            .or_else(|| value.get("publication_date"))
            .and_then(serde_json::Value::as_str)
            .and_then(|text| clean_report_text(text, 120)),
        quote_or_fact: value
            .get("quote_or_fact")
            .or_else(|| value.get("evidence"))
            .or_else(|| value.get("quote"))
            .or_else(|| value.get("fact"))
            .and_then(serde_json::Value::as_str)
            .and_then(|text| clean_report_text(text, 700)),
        reliability: value
            .get("reliability")
            .or_else(|| value.get("publisher"))
            .and_then(serde_json::Value::as_str)
            .and_then(|text| clean_report_text(text, 220)),
    })
}

#[cfg(test)]
pub(super) fn completed_report_markdown_from_workflow_evidence(
    query: &str,
    evidence: &[StructuredEvidenceItem],
) -> Option<String> {
    completed_report_markdown_with_verified_summary(query, evidence, None)
}

#[cfg(test)]
pub(super) fn completed_report_markdown_with_verified_summary(
    query: &str,
    evidence: &[StructuredEvidenceItem],
    verified_summary: Option<&str>,
) -> Option<String> {
    completed_report_markdown_with_verified_context(
        query,
        evidence,
        None,
        verified_summary,
        &[],
        &[],
    )
}

pub(super) fn completed_report_markdown_with_verified_context(
    query: &str,
    evidence: &[StructuredEvidenceItem],
    report_title: Option<&str>,
    verified_summary: Option<&str>,
    verified_findings: &[String],
    verified_caveats: &[String],
) -> Option<String> {
    let items = evidence
        .iter()
        .filter(|item| {
            let lower = item.summary.to_ascii_lowercase();
            !lower.contains("internal workflow/tool log")
                && !lower.contains("internal tool logs withheld")
        })
        .take(10)
        .collect::<Vec<_>>();
    if items.is_empty() {
        return None;
    }
    let verified_summary = verified_summary.and_then(|summary| clean_report_text(summary, 1_200));
    let verified_findings = verified_findings
        .iter()
        .filter_map(|finding| clean_report_text(finding, 700))
        .take(5)
        .collect::<Vec<_>>();
    let verified_caveats = verified_caveats
        .iter()
        .filter_map(|caveat| clean_report_text(caveat, 700))
        .take(10)
        .collect::<Vec<_>>();
    let all_cited_sources = cited_structured_sources(&items, 20);
    let relevance_text = verified_summary
        .iter()
        .chain(verified_findings.iter())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    let cited_sources = if !relevance_text.is_empty() {
        let relevant = relevant_cited_sources(&relevance_text, &all_cited_sources);
        if !relevant.is_empty() {
            relevant
        } else {
            all_cited_sources
        }
    } else {
        all_cited_sources
    };
    if cited_sources.is_empty() {
        return None;
    }
    let is_cjk = query
        .chars()
        .any(|ch| ('\u{3400}'..='\u{9fff}').contains(&ch));
    let report_title = report_title
        .and_then(|title| clean_report_text(title, 160))
        .map(|title| markdown_plain_text(&title))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| evidence_report_title(query));
    let mut markdown = format!("# {report_title}\n\n");

    markdown.push_str(if is_cjk {
        "## 执行摘要\n\n"
    } else {
        "## Executive Summary\n\n"
    });
    if let Some(summary) = verified_summary.as_deref() {
        markdown.push_str(trim_terminal_punctuation(summary));
        markdown.push_str(if is_cjk { "。\n\n" } else { ".\n\n" });
    } else if !verified_findings.is_empty() {
        for finding in &verified_findings {
            markdown.push_str(&format!("- {}\n", trim_terminal_punctuation(finding)));
        }
        markdown.push('\n');
    } else {
        markdown.push_str(if is_cjk {
            "本报告仅使用可追溯的结构化证据，核心结论如下：\n\n"
        } else {
            "This report uses source-backed structured evidence. The strongest supported points are:\n\n"
        });
        for point in executive_summary_points(&items) {
            markdown.push_str(&format!("- {point}\n"));
        }
        markdown.push('\n');
    }

    markdown.push_str(if is_cjk {
        "## 核心发现\n\n"
    } else {
        "## Key Findings\n\n"
    });
    if !verified_findings.is_empty() {
        for (index, finding) in verified_findings.iter().enumerate() {
            let statement = verified_finding_statement(finding);
            let heading = finding_heading(&statement);
            let display_number = index + 1;
            if is_cjk {
                markdown.push_str(&format!("### {display_number}、{heading}\n\n"));
            } else {
                markdown.push_str(&format!("### {display_number}. {heading}\n\n"));
            }
            markdown.push_str(&concise_finding_summary(&statement, is_cjk));
            markdown.push_str("\n\n");
            let finding_sources = {
                let explicit = explicitly_cited_sources(finding, &cited_sources);
                if !explicit.is_empty() {
                    explicit
                } else {
                    let relevant = relevant_cited_sources(finding, &cited_sources);
                    if relevant.is_empty() {
                        cited_sources.iter().take(3).cloned().collect::<Vec<_>>()
                    } else {
                        relevant.into_iter().take(3).collect()
                    }
                }
            };
            let citations = finding_sources
                .iter()
                .map(|(_, source)| source_label(source, is_cjk))
                .collect::<Vec<_>>();
            if !citations.is_empty() {
                markdown.push_str(&format!(
                    "**{}:** {}\n\n",
                    if is_cjk { "关联来源" } else { "Sources" },
                    citations.join(if is_cjk { "；" } else { "; " })
                ));
            }
        }
    } else if let Some(summary) = verified_summary.as_deref() {
        markdown.push_str(if is_cjk {
            "### 已核验结论\n\n"
        } else {
            "### Verified finding\n\n"
        });
        markdown.push_str(trim_terminal_punctuation(summary));
        markdown.push_str(if is_cjk { "。\n\n" } else { ".\n\n" });
        let citations = cited_sources
            .iter()
            .take(4)
            .map(|(_, source)| source_label(source, is_cjk))
            .collect::<Vec<_>>();
        if !citations.is_empty() {
            markdown.push_str(&format!(
                "**{}:** {}\n\n",
                if is_cjk { "关联来源" } else { "Sources" },
                citations.join(if is_cjk { "；" } else { "; " })
            ));
        }
    }
    let supporting_items = items
        .iter()
        .filter(|item| {
            if verified_findings.is_empty() {
                verified_summary.is_none() || !item.key_evidence.is_empty()
            } else {
                !mechanical_collection_summary(&item.summary)
            }
        })
        .copied()
        .collect::<Vec<_>>();
    if !verified_findings.is_empty() && !supporting_items.is_empty() {
        markdown.push_str(if is_cjk {
            "## 证据详析\n\n"
        } else {
            "## Evidence Analysis\n\n"
        });
    }
    for (index, item) in supporting_items.into_iter().enumerate() {
        let supporting_number = index + 1;
        let heading = finding_heading_for_item(item);
        if is_cjk {
            markdown.push_str(&format!("### {supporting_number}、{heading}\n\n"));
        } else {
            markdown.push_str(&format!("### {supporting_number}. {heading}\n\n"));
        }
        if !mechanical_collection_summary(&item.summary) {
            markdown.push_str(&concise_finding_summary(&item.summary, is_cjk));
            markdown.push_str("\n\n");
        }
        let citations = inline_finding_citations(item, is_cjk);
        if !citations.is_empty() {
            markdown.push_str(&format!(
                "**{}:** {}\n\n",
                if is_cjk { "关联来源" } else { "Sources" },
                citations.join(if is_cjk { "；" } else { "; " })
            ));
        }
        let evidence_points = concise_evidence_points(item, is_cjk);
        if !evidence_points.is_empty() {
            markdown.push_str(if is_cjk {
                "证据要点：\n"
            } else {
                "Evidence points:\n"
            });
            for point in evidence_points {
                markdown.push_str(&format!("- {point}\n"));
            }
            markdown.push('\n');
        }
        if let Some(confidence) = &item.confidence {
            let confidence = trim_terminal_punctuation(confidence);
            if !confidence.is_empty() {
                markdown.push_str(&format!(
                    "{}{}{confidence}{}\n\n",
                    if is_cjk { "置信度" } else { "Confidence" },
                    if is_cjk { "：" } else { ": " },
                    if is_cjk { "。" } else { "." }
                ));
            }
        }
    }

    markdown.push_str(if is_cjk {
        "## 证据矩阵\n\n"
    } else {
        "## Evidence Matrix\n\n"
    });
    markdown.push_str(if is_cjk {
        "| 发现 | 来源 | 日期 | 证据 | 可靠性 |\n"
    } else {
        "| Finding | Source | Date | Evidence | Reliability |\n"
    });
    markdown.push_str("| --- | --- | --- | --- | --- |\n");
    for (finding_number, source) in &cited_sources {
        let item = items[*finding_number - 1];
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            finding_number,
            markdown_table_cell(&source_label(source, is_cjk)),
            markdown_table_cell(trim_terminal_punctuation(
                source
                    .date
                    .as_deref()
                    .unwrap_or(if is_cjk { "未注明" } else { "Not stated" },)
            )),
            markdown_table_cell(&bounded_report_text(
                source
                    .quote_or_fact
                    .as_deref()
                    .unwrap_or(item.summary.as_str()),
                is_cjk,
                240,
                360,
            )),
            markdown_table_cell(&bounded_report_text(
                source.reliability.as_deref().unwrap_or(if is_cjk {
                    "未注明"
                } else {
                    "Not stated"
                }),
                is_cjk,
                100,
                150,
            )),
        ));
    }
    markdown.push('\n');

    markdown.push_str(if is_cjk {
        "## 局限与注意事项\n\n"
    } else {
        "## Gaps And Caveats\n\n"
    });
    let mut caveats = Vec::new();
    // The independent checker has already compressed the consequential gaps.
    // Lead with those, then use bounded evidence-level caveats to add detail.
    // Transport/tool diagnostics are useful in the journal, not in a reader's
    // domain report.
    for caveat in &verified_caveats {
        push_report_caveat(&mut caveats, caveat, 8);
    }
    let evidence_caveat_limit = if caveats.is_empty() { 8 } else { 4 };
    for caveat in evidence_caveats(&items) {
        push_report_caveat(&mut caveats, &caveat, evidence_caveat_limit);
    }
    if caveats.is_empty() {
        markdown.push_str(if is_cjk {
            "- 结构化证据中未记录实质性矛盾；这不代表不存在未覆盖风险。\n\n"
        } else {
            "- No material contradictions or gaps were captured in the structured evidence.\n\n"
        });
    } else {
        for caveat in caveats {
            markdown.push_str(&format!("- {caveat}\n"));
        }
        markdown.push('\n');
    }

    markdown.push_str(if is_cjk {
        "## 来源质量与置信度\n\n"
    } else {
        "## Source Quality And Confidence\n\n"
    });
    if is_cjk {
        markdown.push_str(&format!(
            "本报告使用 {} 个可追溯来源，覆盖 {} 条独立证据轨道。",
            cited_sources.len(),
            items.len()
        ));
    } else {
        markdown.push_str(&format!(
            "The report uses {} unique traceable source{} across {} evidence track{}. ",
            cited_sources.len(),
            plural_suffix(cited_sources.len()),
            items.len(),
            plural_suffix(items.len())
        ));
    }
    let mut seen_confidence = HashSet::new();
    let confidence = items
        .iter()
        .filter_map(|item| item.confidence.as_deref())
        .map(|text| bounded_report_text(text, is_cjk, 160, 220))
        .filter(|text| seen_confidence.insert(report_point_key(text)))
        .take(3)
        .collect::<Vec<_>>()
        .join(if is_cjk { "；" } else { "; " });
    if confidence.is_empty() {
        markdown.push_str(if is_cjk {
            "置信度为中等：结论有可追溯来源支撑，但在消除所列证据缺口前不应视为穷尽性判断。\n\n"
        } else {
            "Confidence is medium because the report is grounded in traceable sources, but the listed gaps should be resolved before treating the conclusions as exhaustive.\n\n"
        });
    } else {
        if is_cjk {
            markdown.push_str(&format!(
                "置信度摘要：{confidence}。结论仍受所列缺口与来源覆盖范围限制。\n\n"
            ));
        } else {
            markdown.push_str(&format!(
                "Confidence summary: {confidence}. The report remains limited by the listed gaps and by the scope of the cited sources.\n\n"
            ));
        }
    }

    markdown.push_str(if is_cjk {
        "## 来源\n\n"
    } else {
        "## Sources\n\n"
    });
    for (_, source) in &cited_sources {
        markdown.push_str(&format!(
            "- {}{}\n",
            source_label(source, is_cjk),
            source_detail(source, is_cjk)
        ));
    }

    Some(markdown)
}

fn verified_finding_statement(finding: &str) -> String {
    let mut end = finding.len();
    for marker in [
        "来源：",
        "来源:",
        "Sources:",
        "Source:",
        "sources:",
        "source:",
    ] {
        let Some(index) = finding.find(marker) else {
            continue;
        };
        let attribution = &finding[index + marker.len()..];
        if attribution.contains("http://") || attribution.contains("https://") {
            end = end.min(index);
        }
    }
    if let Some(index) = [finding.find("https://"), finding.find("http://")]
        .into_iter()
        .flatten()
        .min()
    {
        let prefix = finding[..index]
            .trim_end_matches(|ch: char| ch.is_whitespace() || matches!(ch, ':' | '：'));
        if !prefix.is_empty() {
            end = end.min(prefix.len());
        }
    }
    trim_terminal_punctuation(finding[..end].trim()).to_string()
}

fn explicitly_cited_sources(
    finding: &str,
    sources: &[(usize, StructuredEvidenceSource)],
) -> Vec<(usize, StructuredEvidenceSource)> {
    let mut anchors = HashSet::new();
    let mut remaining = finding;
    while let Some(start) = [remaining.find("https://"), remaining.find("http://")]
        .into_iter()
        .flatten()
        .min()
    {
        let url_and_rest = &remaining[start..];
        let end = url_and_rest
            .char_indices()
            .find_map(|(index, ch)| {
                (index > 0
                    && (ch.is_whitespace()
                        || matches!(
                            ch,
                            '。' | '！'
                                | '？'
                                | '；'
                                | '，'
                                | '（'
                                | '）'
                                | '('
                                | ')'
                                | '['
                                | ']'
                                | '<'
                                | '>'
                                | '"'
                                | '\''
                        )))
                .then_some(index)
            })
            .unwrap_or(url_and_rest.len());
        let candidate = url_and_rest[..end].trim_end_matches(['.', '!', '?', ';', ',', ':']);
        if let Some(anchor) = canonical_research_source_anchor(candidate) {
            anchors.insert(anchor);
        }
        if end == url_and_rest.len() {
            break;
        }
        remaining = &url_and_rest[end..];
    }
    if anchors.is_empty() {
        return Vec::new();
    }
    sources
        .iter()
        .filter(|(_, source)| {
            canonical_research_source_anchor(&source.url_or_path)
                .is_some_and(|anchor| anchors.contains(&anchor))
        })
        .cloned()
        .collect()
}

fn relevant_cited_sources(
    summary: &str,
    sources: &[(usize, StructuredEvidenceSource)],
) -> Vec<(usize, StructuredEvidenceSource)> {
    let terms = summary
        .split(|ch: char| !ch.is_alphanumeric() && ch != '.' && ch != '-')
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| {
            term.len() >= 3
                && !matches!(
                    term.as_str(),
                    "and" | "for" | "from" | "the" | "this" | "that" | "using" | "with"
                )
        })
        .collect::<HashSet<_>>();
    sources
        .iter()
        .filter(|(_, source)| {
            let haystack = format!(
                "{} {} {}",
                source.title.as_deref().unwrap_or_default(),
                source.quote_or_fact.as_deref().unwrap_or_default(),
                source.url_or_path
            )
            .to_ascii_lowercase();
            let haystack_terms = haystack
                .split(|ch: char| !ch.is_alphanumeric() && ch != '.' && ch != '-')
                .map(str::trim)
                .filter(|term| term.len() >= 3)
                .collect::<HashSet<_>>();
            terms
                .iter()
                .filter(|term| haystack_terms.contains(term.as_str()))
                .count()
                >= 2
        })
        .cloned()
        .collect()
}

fn evidence_report_title(query: &str) -> String {
    let plain = markdown_plain_text(query);
    let sanitized =
        strip_leading_scope_before_instruction(&strip_internal_validation_annotations(&plain));
    let is_comparison_query = [
        "请对比一下",
        "请比较一下",
        "请对比",
        "请比较",
        "Please compare ",
        "Compare ",
    ]
    .iter()
    .any(|prefix| sanitized.starts_with(prefix));
    let mut title = sanitized
        .split_once("关于")
        .map(|(_, topic)| topic)
        .unwrap_or(sanitized.as_str())
        .split(['。', '\n'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    for prefix in [
        "请用中文撰写一份",
        "请制作一份",
        "请撰写一份",
        "请对比一下",
        "请比较一下",
        "请对比",
        "请比较",
        "请研究",
        "请分析",
        "研究",
        "分析",
    ] {
        if let Some(stripped) = title.strip_prefix(prefix) {
            title = stripped.trim().to_string();
            break;
        }
    }
    for prefix in [
        "Please compare ",
        "Compare ",
        "compare ",
        "Please research ",
        "Research ",
        "research ",
        "Please analyze ",
        "Analyze ",
        "analyze ",
    ] {
        if let Some(stripped) = title.strip_prefix(prefix) {
            let stripped = stripped.trim();
            if !matches!(prefix, "Please analyze " | "Analyze ")
                || !(stripped.starts_with("https://") || stripped.starts_with("http://"))
            {
                title = stripped.to_string();
            }
            break;
        }
    }
    let lower_title = title.to_ascii_lowercase();
    if let Some(instruction_start) = [". cover ", ". include ", ". cite ", ". provide ", ". give "]
        .iter()
        .filter_map(|marker| lower_title.find(marker))
        .min()
    {
        title.truncate(instruction_start);
        title = title.trim().to_string();
    }
    let instruction_start = [
        "，并给出",
        ", 并给出",
        ",并给出",
        "；并给出",
        "; 并给出",
        ";并给出",
        "，给出",
        ", 给出",
        ",给出",
        "；给出",
        "; 给出",
        ";给出",
    ]
    .iter()
    .filter_map(|marker| title.find(marker))
    .min()
    .or_else(|| {
        (!is_comparison_query).then(|| {
            ["的生命史", "的时间线", "的路径", "的架构", "的维护状态"]
                .iter()
                .filter_map(|marker| title.find(marker))
                .min()
        })?
    });
    if let Some(instruction_start) = instruction_start {
        title.truncate(instruction_start);
        title = title.trim().to_string();
    }
    for suffix in ["的精美、全面、可核验研究报告", "的研究报告"] {
        if let Some(stripped) = title.strip_suffix(suffix) {
            title = stripped.trim().to_string();
        }
    }
    if title.is_empty() {
        return "DeepResearch Report".to_string();
    }
    let title_limit = if title
        .chars()
        .any(|ch| ('\u{3400}'..='\u{9fff}').contains(&ch))
    {
        46
    } else {
        84
    };
    if title.chars().count() > title_limit {
        title = title.chars().take(title_limit).collect::<String>();
        title = format!(
            "{}…",
            title.trim_end_matches([':', ';', ',', '，', '；', '：'])
        );
    }
    if title
        .chars()
        .any(|ch| ('\u{3400}'..='\u{9fff}').contains(&ch))
    {
        format!("{title}研究报告")
    } else {
        format!("{title} — Research Report")
    }
}

fn executive_summary_points(items: &[&StructuredEvidenceItem]) -> Vec<String> {
    let mut points = items
        .iter()
        .take(4)
        .map(|item| {
            let point = if mechanical_collection_summary(&item.summary) {
                item.key_evidence
                    .first()
                    .map(String::as_str)
                    .or_else(|| {
                        item.sources
                            .iter()
                            .find_map(|source| source.quote_or_fact.as_deref())
                    })
                    .unwrap_or(item.summary.as_str())
            } else {
                item.summary.as_str()
            };
            let limit = if point
                .chars()
                .any(|ch| ('\u{3400}'..='\u{9fff}').contains(&ch))
            {
                120
            } else {
                190
            };
            truncate_chars(point, limit)
        })
        .collect::<Vec<_>>();
    if points.is_empty() {
        points.push(
            "The workflow captured traceable evidence, but no individual summary was available."
                .to_string(),
        );
    }
    points
}

fn evidence_caveats(items: &[&StructuredEvidenceItem]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut caveats = Vec::new();
    for item in items {
        for point in item.contradictions.iter().chain(item.gaps.iter()) {
            let key = point.to_ascii_lowercase();
            if seen.insert(key) {
                caveats.push(point.clone());
            }
        }
    }
    caveats
}

fn push_report_caveat(caveats: &mut Vec<String>, candidate: &str, limit: usize) {
    if caveats.len() >= limit {
        return;
    }
    let lower = candidate.trim().to_ascii_lowercase();
    if lower.is_empty()
        || lower.starts_with("collection errors:")
        || lower.starts_with("web_fetch ")
        || lower.starts_with("web_search ")
        || lower.contains("internal tool log")
    {
        return;
    }
    let key = report_point_key(candidate);
    if key.is_empty()
        || caveats.iter().any(|existing| {
            let existing_key = report_point_key(existing);
            existing_key == key
                || (existing_key.chars().count().min(key.chars().count()) >= 24
                    && (existing_key.contains(&key) || key.contains(&existing_key)))
        })
    {
        return;
    }
    caveats.push(candidate.to_string());
}

fn report_point_key(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn concise_finding_summary(summary: &str, is_cjk: bool) -> String {
    let concise = bounded_report_text(summary, is_cjk, 360, 520);
    if concise.is_empty() {
        concise
    } else {
        format!("{concise}{}", if is_cjk { "。" } else { "." })
    }
}

fn concise_evidence_points(item: &StructuredEvidenceItem, is_cjk: bool) -> Vec<String> {
    let mut seen = HashSet::new();
    item.key_evidence
        .iter()
        .filter_map(|point| {
            let concise = bounded_report_text(point, is_cjk, 200, 300);
            let key = concise
                .chars()
                .filter(|ch| ch.is_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            (!key.is_empty() && seen.insert(key)).then_some(concise)
        })
        .take(6)
        .collect()
}

fn inline_finding_citations(item: &StructuredEvidenceItem, is_cjk: bool) -> Vec<String> {
    let mut seen = HashSet::new();
    item.sources
        .iter()
        .filter_map(|source| {
            let anchor = canonical_research_source_anchor(&source.url_or_path)?;
            seen.insert(anchor).then(|| source_label(source, is_cjk))
        })
        .take(3)
        .collect()
}

fn bounded_report_text(text: &str, is_cjk: bool, cjk_limit: usize, english_limit: usize) -> String {
    let clean = trim_terminal_punctuation(text);
    truncate_chars(clean, if is_cjk { cjk_limit } else { english_limit })
}

fn strip_internal_validation_annotations(value: &str) -> String {
    let mut clean = value.to_string();
    for (open, close) in [('（', '）'), ('(', ')'), ('[', ']')] {
        let mut search_from = 0usize;
        while let Some(relative_start) = clean[search_from..].find(open) {
            let start = search_from + relative_start;
            let content_start = start + open.len_utf8();
            let Some(relative_end) = clean[content_start..].find(close) else {
                break;
            };
            let end = content_start + relative_end;
            let annotation = clean[content_start..end].to_ascii_lowercase();
            if annotation.contains("run-id")
                || annotation.contains("cache-bust")
                || annotation.contains("cache bust")
                || annotation.contains("e2e-")
                || annotation.contains("validation")
            {
                clean.replace_range(start..end + close.len_utf8(), " ");
                search_from = start;
            } else {
                search_from = end + close.len_utf8();
            }
        }
    }
    clean.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_leading_scope_before_instruction(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let scoped = value.starts_with("截至") || lower.starts_with("as of ");
    if !scoped {
        return value.to_string();
    }
    let start = [
        "请对比",
        "请比较",
        "请研究",
        "请分析",
        "研究",
        "分析",
        "Compare ",
        "compare ",
        "Research ",
        "research ",
        "Analyze ",
        "analyze ",
    ]
    .iter()
    .filter_map(|marker| value.find(marker))
    .min();
    start
        .map(|index| value[index..].to_string())
        .unwrap_or_else(|| value.to_string())
}

fn finding_heading(summary: &str) -> String {
    let summary = summary.trim_start();
    let heading = summary
        .split(['.', '。', '!', '！', '?', '？'])
        .next()
        .unwrap_or(summary)
        .trim();
    if heading.is_empty() {
        "Source-backed finding".to_string()
    } else {
        let limit = if heading
            .chars()
            .any(|ch| ('\u{3400}'..='\u{9fff}').contains(&ch))
        {
            42
        } else {
            72
        };
        let clause = heading
            .char_indices()
            .filter_map(|(index, ch)| {
                matches!(ch, ',' | '，' | ';' | '；' | ':' | '：' | '—')
                    .then_some(heading[..index].trim())
            })
            .find(|clause| {
                let count = clause.chars().count();
                count >= 8 && count <= limit * 2
            });
        clause
            .map(str::to_string)
            .unwrap_or_else(|| truncate_chars(heading, limit))
    }
}

fn mechanical_collection_summary(summary: &str) -> bool {
    let lower = summary.trim().to_ascii_lowercase();
    (lower.starts_with("direct collection found")
        || lower.starts_with("collection found")
        || lower.starts_with("research collected"))
        && lower.contains("source")
}

fn finding_heading_for_item(item: &StructuredEvidenceItem) -> String {
    if !mechanical_collection_summary(&item.summary) {
        return finding_heading(&item.summary);
    }
    if let Some(title) = item
        .sources
        .iter()
        .filter_map(|source| source.title.as_deref())
        .find(|title| !title.to_ascii_lowercase().starts_with("planned source"))
    {
        return finding_heading(title);
    }
    item.key_evidence
        .first()
        .map(|point| finding_heading(point))
        .unwrap_or_else(|| finding_heading(&item.summary))
}

fn markdown_table_cell(text: &str) -> String {
    text.replace('|', "\\|")
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn cited_structured_sources(
    items: &[&StructuredEvidenceItem],
    max_sources: usize,
) -> Vec<(usize, StructuredEvidenceSource)> {
    if max_sources == 0 {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for (index, item) in items.iter().enumerate() {
        for source in &item.sources {
            let Some(anchor) = canonical_research_source_anchor(&source.url_or_path) else {
                continue;
            };
            if seen.insert(anchor) {
                sources.push((index + 1, source.clone()));
                if sources.len() == max_sources {
                    return sources;
                }
            }
        }
    }
    sources
}

fn source_label(source: &StructuredEvidenceSource, is_cjk: bool) -> String {
    let title = source
        .title
        .as_deref()
        .unwrap_or(if is_cjk { "来源" } else { "Source" });
    if source.url_or_path.starts_with("http://") || source.url_or_path.starts_with("https://") {
        format!("[{}]({})", title, source.url_or_path)
    } else {
        format!("{} (`{}`)", title, source.url_or_path)
    }
}

fn source_detail(source: &StructuredEvidenceSource, is_cjk: bool) -> String {
    let mut parts = Vec::new();
    if let Some(date) = &source.date {
        let date = trim_terminal_punctuation(date);
        if !date.is_empty() {
            parts.push(format!(
                "{}{}{}",
                if is_cjk { "日期" } else { "date" },
                if is_cjk { "：" } else { ": " },
                date
            ));
        }
    }
    if let Some(reliability) = &source.reliability {
        let reliability = trim_terminal_punctuation(reliability);
        if !reliability.is_empty() {
            parts.push(format!(
                "{}{}{}",
                if is_cjk { "可靠性" } else { "reliability" },
                if is_cjk { "：" } else { ": " },
                reliability
            ));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(
            " — {}{}",
            parts.join(if is_cjk { "；" } else { "; " }),
            if is_cjk { "。" } else { "." }
        )
    }
}

fn trim_terminal_punctuation(text: &str) -> &str {
    text.trim_end_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '.' | '。' | '!' | '！' | '?' | '？' | ';' | '；' | ',' | '，'
            )
    })
}

fn clean_report_string_array(
    value: Option<&serde_json::Value>,
    max_items: usize,
    max_chars: usize,
) -> Vec<String> {
    let mut seen = HashSet::new();
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter_map(|text| clean_report_text(text, max_chars))
                .filter(|text| seen.insert(text.to_ascii_lowercase()))
                .take(max_items)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn clean_report_text(text: &str, max_chars: usize) -> Option<String> {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() || deep_research_output_has_internal_leak(&compact) {
        None
    } else {
        Some(truncate_chars(&compact, max_chars))
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
