use std::collections::HashMap;

const SOURCE_CATALOG_MAX_SOURCES: usize = 16;
const SOURCE_CATALOG_MAX_CHUNKS: usize = 384;
const SOURCE_CATALOG_MAX_CHUNKS_PER_REPORT_SOURCE: usize = 2;
const SOURCE_CATALOG_MAX_CHUNKS_PER_INELIGIBLE_REPORT_SOURCE: usize = 1;
const SOURCE_CATALOG_MAX_CHUNK_CHARS: usize = 700;
const SOURCE_CATALOG_MAX_TITLE_CHARS: usize = 240;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeepResearchSourceCatalog {
    pub(crate) sources: Vec<DeepResearchCatalogSource>,
    pub(crate) omitted_source_count: usize,
    pub(crate) omitted_chunk_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeepResearchCatalogSource {
    pub(crate) alias: String,
    pub(crate) title: String,
    pub(crate) anchor: String,
    pub(crate) chunks: Vec<String>,
    pub(crate) claim_eligible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeepResearchEvidenceFirstPublication {
    Synthesized,
    SourceBacked,
    NoEvidence,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeepResearchPublicationQuality {
    pub(crate) direct_answer_count: usize,
    pub(crate) finding_count: usize,
    pub(crate) accepted_claim_count: usize,
    pub(crate) cited_source_count: usize,
    pub(crate) relevant_source_count: usize,
    pub(crate) source_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeepResearchPublishedReport {
    pub(crate) artifacts: ResearchReportArtifacts,
    pub(crate) publication: DeepResearchEvidenceFirstPublication,
    pub(crate) quality: DeepResearchPublicationQuality,
}

pub(crate) fn deep_research_source_catalog(
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Result<Option<DeepResearchSourceCatalog>, String> {
    let canonical = deep_research_canonical_workflow_output(workflow_output, workflow_metadata);
    if canonical.trim().is_empty() {
        return Ok(None);
    }
    let value = serde_json::from_str::<serde_json::Value>(&canonical)
        .map_err(|error| format!("decode DeepResearch source catalog: {error}"))?;
    let observed_query = value
        .get("query")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "DeepResearch source catalog omitted its query".to_string())?;
    if observed_query != query.trim() {
        return Err("DeepResearch source catalog belongs to a different query".to_string());
    }

    let Some(acquisition) = value.get("acquisition") else {
        return Ok(None);
    };
    let Some(packet) = acquisition
        .get("packet")
        .filter(|packet| packet.is_object())
    else {
        return Ok(None);
    };
    if packet.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err("DeepResearch source catalog has an unsupported packet version".to_string());
    }
    let Some(raw_sources) = packet.get("sources").and_then(serde_json::Value::as_array) else {
        return Ok(None);
    };
    if raw_sources.is_empty() {
        return Ok(None);
    }
    let semantic_source_admission = value
        .pointer("/acquisition/metadata/source_selection_mode")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|mode| mode == "semantic_candidate_ids");

    let mut catalog = DeepResearchSourceCatalog {
        sources: Vec::new(),
        omitted_source_count: raw_sources.len().saturating_sub(SOURCE_CATALOG_MAX_SOURCES),
        omitted_chunk_count: 0,
    };
    let mut source_ids = HashSet::new();
    let mut chunk_ids = HashSet::new();
    let mut source_by_anchor = HashMap::<String, usize>::new();
    let mut retained_chunk_count = 0usize;

    for raw_source in raw_sources.iter().take(SOURCE_CATALOG_MAX_SOURCES) {
        let Some(source_id) =
            bounded_catalog_text(raw_source.get("source_id"), 160, stable_catalog_identity)
        else {
            catalog.omitted_source_count += 1;
            continue;
        };
        if !source_ids.insert(source_id) {
            catalog.omitted_source_count += 1;
            continue;
        }
        let Some(anchor) = raw_source
            .get("url_or_path")
            .and_then(serde_json::Value::as_str)
            .and_then(canonical_research_source_anchor)
        else {
            catalog.omitted_source_count += 1;
            continue;
        };
        let title = bounded_catalog_text(
            raw_source.get("title"),
            SOURCE_CATALOG_MAX_TITLE_CHARS,
            |_| true,
        )
        .unwrap_or_else(|| anchor.clone());
        let Some(raw_chunks) = raw_source
            .get("chunks")
            .and_then(serde_json::Value::as_array)
        else {
            catalog.omitted_source_count += 1;
            continue;
        };
        let claim_eligible =
            catalog_source_claim_eligible(&anchor, raw_chunks, semantic_source_admission);

        let mut chunks = Vec::new();
        for raw_chunk in raw_chunks {
            if retained_chunk_count >= SOURCE_CATALOG_MAX_CHUNKS {
                catalog.omitted_chunk_count += 1;
                continue;
            }
            let Some(chunk_id) =
                bounded_catalog_text(raw_chunk.get("chunk_id"), 200, stable_catalog_identity)
            else {
                catalog.omitted_chunk_count += 1;
                continue;
            };
            let Some(text) = raw_chunk
                .get("text")
                .and_then(serde_json::Value::as_str)
                .and_then(sanitize_catalog_chunk)
            else {
                catalog.omitted_chunk_count += 1;
                continue;
            };
            if !chunk_ids.insert(chunk_id) {
                catalog.omitted_chunk_count += 1;
                continue;
            }
            if !chunks.iter().any(|existing| existing == &text) {
                chunks.push(text);
                retained_chunk_count += 1;
            }
        }
        if chunks.is_empty() {
            catalog.omitted_source_count += 1;
            continue;
        }
        if !semantic_source_admission && !catalog_source_matches_query(query, &title, &chunks) {
            catalog.omitted_source_count += 1;
            catalog.omitted_chunk_count += chunks.len();
            continue;
        }

        if let Some(index) = source_by_anchor.get(&anchor).copied() {
            let retained_source = &mut catalog.sources[index];
            retained_source.claim_eligible &= claim_eligible;
            let retained = &mut retained_source.chunks;
            for chunk in chunks {
                if !retained.contains(&chunk) {
                    retained.push(chunk);
                }
            }
            continue;
        }
        let alias = format!("source-{}", catalog.sources.len() + 1);
        source_by_anchor.insert(anchor.clone(), catalog.sources.len());
        catalog.sources.push(DeepResearchCatalogSource {
            alias,
            title,
            anchor,
            chunks,
            claim_eligible,
        });
    }

    if catalog.sources.is_empty() {
        Ok(None)
    } else {
        Ok(Some(catalog))
    }
}

pub(crate) fn materialize_deep_research_source_backed_report(
    workspace: &Path,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Result<Option<ResearchReportArtifacts>, String> {
    let Some(catalog) = deep_research_source_catalog(query, workflow_output, workflow_metadata)?
    else {
        return Ok(None);
    };
    let markdown = deep_research_source_backed_markdown(query, &catalog);
    let html = deep_research_completed_report_html(query, &markdown);
    let slug = deep_research_report_slug(query);
    let rel_html = format!(".a3s/research/{slug}/index.html");
    let (root, report_dir) = prepare_research_report_directory(workspace, &slug)?;
    write_research_report_pair(
        &report_dir.join("report.md"),
        markdown,
        &report_dir.join("index.html"),
        html,
    )?;
    let artifacts = trusted_research_report_artifact_paths(&rel_html, &root)
        .ok_or_else(|| "source-backed report artifacts failed path validation".to_string())?;
    source_backed_report_artifacts(&artifacts)
        .then_some(Some(artifacts))
        .ok_or_else(|| "source-backed report artifacts failed content validation".to_string())
}

pub(crate) fn materialize_deep_research_no_evidence_report(
    workspace: &Path,
    query: &str,
) -> Result<ResearchReportArtifacts, String> {
    let chinese = query.chars().any(source_backed_han_character);
    let title = markdown_plain_text(&query.chars().take(180).collect::<String>());
    let (status, status_text, limitations, limitation_text, sources, source_text) = if chinese {
        (
            "证据状态",
            "本次检索没有获得可安全发布的来源文字，因此不生成领域结论。",
            "限制",
            "此页面只说明证据边界；它不把检索失败解释为不存在相关事实，也不建议据此作出决定。",
            "来源",
            "没有可安全发布的来源。",
        )
    } else {
        (
            "Evidence Status",
            "This retrieval obtained no source text that can be published safely, so no domain conclusion is generated.",
            "Limitations",
            "This page states only the evidence boundary. It does not treat retrieval failure as proof that relevant facts do not exist and should not be used alone for a decision.",
            "Sources",
            "No safely publishable source was obtained.",
        )
    };
    let markdown = format!(
        "# {title}\n\n## {status}\n\n{status_text}\n\n## {limitations}\n\n{limitation_text}\n\n## {sources}\n\n{source_text}\n"
    );
    let html = deep_research_completed_report_html(query, &markdown);
    let slug = deep_research_report_slug(query);
    let rel_html = format!(".a3s/research/{slug}/index.html");
    let (root, report_dir) = prepare_research_report_directory(workspace, &slug)?;
    write_research_report_pair(
        &report_dir.join("report.md"),
        markdown,
        &report_dir.join("index.html"),
        html,
    )?;
    let artifacts = trusted_research_report_artifact_paths(&rel_html, &root)
        .ok_or_else(|| "no-evidence report artifacts failed path validation".to_string())?;
    no_evidence_report_artifacts(&artifacts)
        .then_some(artifacts)
        .ok_or_else(|| "no-evidence report artifacts failed content validation".to_string())
}

pub(crate) fn deep_research_evidence_first_published_report(
    workspace: &Path,
    query: &str,
    workflow_output: &str,
) -> Result<Option<DeepResearchPublishedReport>, String> {
    let value = match serde_json::from_str::<serde_json::Value>(workflow_output.trim()) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if value.get("mode").and_then(serde_json::Value::as_str) != Some("evidence_first_report") {
        return Ok(None);
    }
    if value.get("query").and_then(serde_json::Value::as_str) != Some(query) {
        return Err("evidence-first publication belongs to a different query".to_string());
    }
    let publication = match value
        .pointer("/publication/status")
        .and_then(serde_json::Value::as_str)
    {
        Some("synthesized") => DeepResearchEvidenceFirstPublication::Synthesized,
        Some("source_backed") => DeepResearchEvidenceFirstPublication::SourceBacked,
        Some("no_evidence") => DeepResearchEvidenceFirstPublication::NoEvidence,
        Some(_) => return Err("evidence-first publication has an unknown status".to_string()),
        None => return Err("evidence-first publication omitted its status".to_string()),
    };
    let quality = deep_research_publication_quality(&value)?;
    validate_deep_research_publication_quality(publication, quality)?;
    let slug = deep_research_report_slug(query);
    let expected = format!(".a3s/research/{slug}/index.html");
    let expected_markdown = format!(".a3s/research/{slug}/report.md");
    if value
        .pointer("/publication/markdown")
        .and_then(serde_json::Value::as_str)
        != Some(expected_markdown.as_str())
    {
        return Err("evidence-first publication points to an unexpected artifact".to_string());
    }
    if value
        .pointer("/publication/html")
        .and_then(serde_json::Value::as_str)
        != Some(expected.as_str())
    {
        return Err("evidence-first publication points to an unexpected artifact".to_string());
    }
    let artifacts = trusted_research_report_artifact_paths(&expected, workspace)
        .ok_or_else(|| "evidence-first publication artifacts failed path validation".to_string())?;
    let valid = match publication {
        DeepResearchEvidenceFirstPublication::Synthesized => {
            completed_research_report_artifacts(&artifacts)
        }
        DeepResearchEvidenceFirstPublication::SourceBacked => {
            source_backed_report_artifacts(&artifacts)
        }
        DeepResearchEvidenceFirstPublication::NoEvidence => {
            no_evidence_report_artifacts(&artifacts)
        }
    };
    if !valid {
        return Err("evidence-first publication artifacts failed content validation".to_string());
    }
    Ok(Some(DeepResearchPublishedReport {
        artifacts,
        publication,
        quality,
    }))
}

fn deep_research_publication_quality(
    value: &serde_json::Value,
) -> Result<DeepResearchPublicationQuality, String> {
    let quality = value
        .pointer("/publication/quality")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "evidence-first publication omitted its quality metrics".to_string())?;
    let metric = |name: &str| {
        quality
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| format!("evidence-first publication has an invalid `{name}` metric"))
    };
    Ok(DeepResearchPublicationQuality {
        direct_answer_count: metric("direct_answer_count")?,
        finding_count: metric("finding_count")?,
        accepted_claim_count: metric("accepted_claim_count")?,
        cited_source_count: metric("cited_source_count")?,
        relevant_source_count: metric("relevant_source_count")?,
        source_count: metric("source_count")?,
    })
}

fn validate_deep_research_publication_quality(
    publication: DeepResearchEvidenceFirstPublication,
    quality: DeepResearchPublicationQuality,
) -> Result<(), String> {
    let empty_claims = quality.direct_answer_count == 0
        && quality.finding_count == 0
        && quality.accepted_claim_count == 0
        && quality.cited_source_count == 0;
    match publication {
        DeepResearchEvidenceFirstPublication::Synthesized => {
            if quality.direct_answer_count == 0
                || quality.finding_count == 0
                || quality.accepted_claim_count < 2
                || quality.cited_source_count == 0
                || quality.cited_source_count > quality.relevant_source_count
                || quality.relevant_source_count == 0
                || quality.relevant_source_count > quality.source_count
            {
                return Err(
                    "synthesized publication failed the direct-answer, cited-claim, or source-relevance quality gate"
                        .to_string(),
                );
            }
        }
        DeepResearchEvidenceFirstPublication::SourceBacked => {
            if !empty_claims
                || quality.source_count == 0
                || quality.relevant_source_count == 0
                || quality.relevant_source_count > quality.source_count
            {
                return Err(
                    "source-backed publication reported synthesized claims or invalid source metrics"
                        .to_string(),
                );
            }
        }
        DeepResearchEvidenceFirstPublication::NoEvidence => {
            if !empty_claims || quality.source_count != 0 || quality.relevant_source_count != 0 {
                return Err("no-evidence publication reported evidence or claims".to_string());
            }
        }
    }
    Ok(())
}

fn deep_research_source_backed_markdown(
    query: &str,
    catalog: &DeepResearchSourceCatalog,
) -> String {
    let labels = source_backed_labels(query);
    let title = markdown_plain_text(&query.chars().take(180).collect::<String>());
    let mut markdown = format!(
        "# {title}\n\n> {}\n\n## {}\n\n{}\n",
        labels.status, labels.evidence_heading, labels.evidence_intro
    );
    for (index, source) in catalog.sources.iter().enumerate() {
        let number = index + 1;
        let title = markdown_plain_text(&source.title);
        markdown.push_str(&format!("\n### [{number}] {title}\n"));
        if !source.claim_eligible {
            markdown.push_str(&format!(
                "\n> **{}** {}\n",
                labels.ineligible_heading, labels.ineligible_explanation
            ));
        }
        for chunk in selected_source_chunks(query, source) {
            markdown.push('\n');
            markdown.push_str(&fenced_catalog_text(chunk));
            markdown.push('\n');
        }
        markdown.push_str(&format!(
            "\n{}\n",
            source_backed_source_link(source, number)
        ));
    }
    markdown.push_str(&format!(
        "\n## {}\n\n{}",
        labels.limitations_heading, labels.limitations
    ));
    if catalog.omitted_source_count > 0 || catalog.omitted_chunk_count > 0 {
        markdown.push_str(&format!(
            " {}",
            (labels.omissions)(catalog.omitted_source_count, catalog.omitted_chunk_count)
        ));
    }
    markdown.push_str(&format!("\n\n## {}\n", labels.sources_heading));
    for (index, source) in catalog.sources.iter().enumerate() {
        markdown.push_str(&format!(
            "\n{}. {}",
            index + 1,
            source_backed_source_title_link(source)
        ));
        if !source.claim_eligible {
            markdown.push_str(&format!(" — **{}**", labels.ineligible_short));
        }
    }
    markdown.push('\n');
    markdown
}

struct SourceBackedLabels {
    status: &'static str,
    evidence_heading: &'static str,
    evidence_intro: &'static str,
    limitations_heading: &'static str,
    limitations: &'static str,
    sources_heading: &'static str,
    ineligible_heading: &'static str,
    ineligible_explanation: &'static str,
    ineligible_short: &'static str,
    omissions: fn(usize, usize) -> String,
}

fn source_backed_labels(query: &str) -> SourceBackedLabels {
    if query.chars().any(source_backed_han_character) {
        SourceBackedLabels {
            status: "这是可核查的来源证据视图；它保留已获取的资料，但不把摘录冒充为完整综合结论。",
            evidence_heading: "已保留的来源证据",
            evidence_intro: "以下摘录按来源分组，来源文字仅作为不可信数据展示，可通过对应链接直接核查。",
            limitations_heading: "限制",
            limitations: "此结果保留相关来源摘录和链接，但不声称已完成全部分析，也不声称这些摘录覆盖了问题的所有方面。",
            sources_heading: "来源",
            ineligible_heading: "证据资格：不可用于结论",
            ineligible_explanation: "该来源属于低可信、自媒体或缺少可核查发布责任的材料，仅保留用于核查检索边界。",
            ineligible_short: "不可用于结论",
            omissions: |sources, chunks| {
                format!("安全边界另行省略了 {sources} 个来源和 {chunks} 个来源片段。")
            },
        }
    } else {
        SourceBackedLabels {
            status: "This is a verifiable source-evidence view. It preserves fetched material without presenting excerpts as a completed synthesis.",
            evidence_heading: "Preserved Source Evidence",
            evidence_intro: "The excerpts below are grouped by source and displayed only as untrusted data for direct verification through the corresponding links.",
            limitations_heading: "Limitations",
            limitations: "This result preserves relevant source excerpts and links, but it does not claim that analysis is complete or that the excerpts cover every aspect of the question.",
            sources_heading: "Sources",
            ineligible_heading: "Claim eligibility: not eligible for conclusions",
            ineligible_explanation: "This low-trust, self-published, or unaccountable source is retained only for auditing the retrieval boundary.",
            ineligible_short: "not eligible for conclusions",
            omissions: |sources, chunks| {
                format!("Safety bounds omitted {sources} source(s) and {chunks} source excerpt(s).")
            },
        }
    }
}

fn selected_source_chunks<'a>(query: &str, source: &'a DeepResearchCatalogSource) -> Vec<&'a str> {
    let features = source_backed_query_features(query);
    let mut ranked = source
        .chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            let lower = chunk.to_lowercase();
            let overlap = features
                .iter()
                .filter(|feature| lower.contains(feature.as_str()))
                .map(|feature| feature.chars().count())
                .sum::<usize>();
            let score = overlap as i64 * 24 + catalog_excerpt_readability_score(chunk);
            (index, score)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_index, left_score), (right_index, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_index.cmp(right_index))
    });
    ranked.truncate(if source.claim_eligible {
        SOURCE_CATALOG_MAX_CHUNKS_PER_REPORT_SOURCE
    } else {
        SOURCE_CATALOG_MAX_CHUNKS_PER_INELIGIBLE_REPORT_SOURCE
    });
    ranked.sort_by_key(|(index, _)| *index);
    ranked
        .into_iter()
        .map(|(index, _)| source.chunks[index].as_str())
        .collect()
}

fn catalog_excerpt_readability_score(value: &str) -> i64 {
    let character_count = value.chars().count().min(240) as i64;
    let sentence_count = value
        .chars()
        .filter(|character| matches!(character, '.' | '!' | '?' | '。' | '！' | '？'))
        .count() as i64;
    let markdown_links = value.matches("](").count() as i64;
    let markdown_images = value.matches("![").count() as i64;
    let template_markers = value.matches("{{").count() as i64 + value.matches("}}").count() as i64;
    character_count + sentence_count * 32
        - markdown_links * 90
        - markdown_images * 120
        - template_markers * 80
}

fn sanitize_catalog_chunk(value: &str) -> Option<String> {
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return None;
    }
    let mut text = value.replace("\r\n", "\n").replace('\r', "\n");
    for tag in ["script", "style", "noscript"] {
        text = strip_html_element_blocks(&text, tag);
    }
    text = strip_markdown_link_targets(&text);
    text = strip_catalog_html_tags(&text);
    let lines = text
        .lines()
        .map(strip_embedded_constructor_script_tail)
        .map(strip_embedded_serialized_configuration_tail)
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty() && !catalog_noise_line(line))
        .collect::<Vec<_>>();
    let text = lines.join(" ");
    let text = text.trim();
    (!text.is_empty()
        && text.chars().count() <= SOURCE_CATALOG_MAX_CHUNK_CHARS
        && !catalog_noise_payload(text))
    .then(|| text.to_string())
}

/// Keep visible Markdown labels while removing transport URLs and image
/// syntax. The source anchor remains available in the Host-owned source
/// ledger, so inline targets add prompt weight without adding evidence.
fn strip_markdown_link_targets(value: &str) -> String {
    let without_images = strip_markdown_targets(value, true);
    let without_links = strip_markdown_targets(&without_images, false);
    strip_orphan_markdown_targets(&without_links)
}

fn strip_markdown_targets(value: &str, images_only: bool) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while cursor < characters.len() {
        let image = characters[cursor] == '!'
            && characters
                .get(cursor + 1)
                .is_some_and(|character| *character == '[');
        if images_only != image {
            output.push(characters[cursor]);
            cursor += 1;
            continue;
        }
        let label_start = if image {
            cursor + 2
        } else if !images_only && characters[cursor] == '[' {
            cursor + 1
        } else {
            output.push(characters[cursor]);
            cursor += 1;
            continue;
        };
        let Some(label_end) = characters[label_start..]
            .iter()
            .position(|character| *character == ']')
            .map(|offset| label_start + offset)
        else {
            output.push(characters[cursor]);
            cursor += 1;
            continue;
        };
        if characters.get(label_end + 1) != Some(&'(') {
            output.push(characters[cursor]);
            cursor += 1;
            continue;
        }
        let mut target_end = label_end + 2;
        let mut depth = 1usize;
        while target_end < characters.len() && depth > 0 {
            match characters[target_end] {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            target_end += 1;
        }
        if depth != 0 {
            output.push(characters[cursor]);
            cursor += 1;
            continue;
        }
        let label = characters[label_start..label_end]
            .iter()
            .collect::<String>();
        if !label.trim().is_empty() {
            output.push_str(label.trim());
        }
        output.push(' ');
        cursor = target_end;
    }
    output
}

fn strip_orphan_markdown_targets(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while cursor < characters.len() {
        if characters[cursor] != ']'
            || characters.get(cursor + 1).is_none_or(|character| *character != '(')
        {
            output.push(characters[cursor]);
            cursor += 1;
            continue;
        }
        let mut target_end = cursor + 2;
        let mut depth = 1usize;
        while target_end < characters.len() && depth > 0 {
            match characters[target_end] {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            target_end += 1;
        }
        if depth == 0 {
            output.push(' ');
            cursor = target_end;
        } else {
            output.push(characters[cursor]);
            cursor += 1;
        }
    }
    output
}

fn strip_catalog_html_tags(value: &str) -> String {
    static HTML_TAG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = HTML_TAG.get_or_init(|| {
        regex::Regex::new(r"(?is)\\?</?[a-z][^>]{0,1200}>")
            .expect("static HTML tag regex")
    });
    pattern.replace_all(value, " ").into_owned()
}

fn strip_embedded_serialized_configuration_tail(value: &str) -> &str {
    static SERIALIZED_CONFIGURATION: std::sync::OnceLock<regex::Regex> =
        std::sync::OnceLock::new();
    let pattern = SERIALIZED_CONFIGURATION.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)((?:^|[}\]]\s*,?\s*\{?\s*|[,;]\s*\{?\s*)(?:(?:"type"|\\\"type\\\")\s*:\s*(?:"keyvalue"|\\\"keyvalue\\\")|(?:"variables"|\\\"variables\\\")\s*:\s*\[))"#,
        )
        .expect("static serialized configuration regex")
    });
    pattern
        .captures(value)
        .and_then(|captures| captures.get(1))
        .map_or(value, |payload| value[..payload.start()].trim_end())
}

fn strip_embedded_constructor_script_tail(value: &str) -> &str {
    static EMBEDDED_CONSTRUCTOR_ASSIGNMENT: std::sync::OnceLock<regex::Regex> =
        std::sync::OnceLock::new();
    let pattern = EMBEDDED_CONSTRUCTOR_ASSIGNMENT.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(?:^|[\s;])((?:(?:var|let|const)\s+)?[a-z_$][\w$\\]*\s*=\s*new\s+[a-z_$][\w$.]*\s*\()",
        )
        .expect("static embedded constructor assignment regex")
    });
    pattern
        .captures(value)
        .and_then(|captures| captures.get(1))
        .map_or(value, |assignment| value[..assignment.start()].trim_end())
}

fn strip_html_element_blocks(value: &str, tag: &str) -> String {
    let mut output = value.to_string();
    let opening = format!("<{tag}");
    let closing = format!("</{tag}>");
    loop {
        let lower = output.to_ascii_lowercase();
        let Some(start) = lower.find(&opening) else {
            break;
        };
        let end = lower[start..]
            .find(&closing)
            .map(|offset| start + offset + closing.len())
            .or_else(|| lower[start..].find('>').map(|offset| start + offset + 1))
            .unwrap_or(output.len());
        output.replace_range(start..end, " ");
    }
    output
}

fn catalog_noise_line(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let known_chrome = [
        "your current user-agent string appears to be from an automated process",
        "doesn't work properly without javascript enabled",
        "does not work properly without javascript enabled",
        "please enable javascript to continue",
        "toggle the table of contents",
        "open main menu",
        "__next_data__",
        "webpack",
        "document.cookie",
        "globalthis.",
        "process.env",
        "window.onscroll",
        "echo.init(",
        "$(function",
        "<%=",
        "<%",
        "%>",
        "javascript:",
        "onerror=",
    ];
    if known_chrome.iter().any(|marker| lower.contains(marker)) {
        return true;
    }
    let trimmed = lower.trim_start_matches(['*', '-', ' ', '\t']);
    let script_assignment = ["var ", "let ", "const ", "window.", "document."]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
        && trimmed.contains('=');
    let script_function = (trimmed.starts_with("function ")
        || trimmed.starts_with("$(\"")
        || trimmed.starts_with("$('"))
        && (trimmed.contains('{') || trimmed.contains(".click(") || trimmed.contains(".css("));
    let jquery_script = lower.contains("$(")
        && [".click(", ".on(", ".html(", ".attr(", ".siblings("]
            .iter()
            .any(|marker| lower.contains(marker));
    let markdown_links = value.matches("](").count();
    script_assignment
        || script_function
        || jquery_script
        || markdown_links >= 7
        || catalog_serialized_or_script_payload(value)
}

fn catalog_noise_payload(value: &str) -> bool {
    value.matches("](").count() >= 7 || catalog_serialized_or_script_payload(value)
}

fn catalog_serialized_or_script_payload(value: &str) -> bool {
    let character_count = value.chars().count();
    if character_count < 80 {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    let framework_markers = [
        "self.__next_f.push",
        "__next_data__",
        "webpackchunk",
        "hydrateroot(",
        "application/ld+json",
        "window.__",
        "document.createelement(",
        "addeventlistener(",
        "\"type\":\"keyvalue\"",
        "\\\"type\\\":\\\"keyvalue\\\"",
        "\"variables\":[",
        "\\\"variables\\\":[",
    ];
    if framework_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return true;
    }

    let escaped_quotes = value.matches("\\\"").count();
    let escaped_controls = ["\\\\n", "\\\\r", "\\\\t", "\\\\u", "\\u"]
        .iter()
        .map(|marker| value.matches(marker).count())
        .sum::<usize>();
    let json_pairs = value.matches("\":").count() + value.matches("\\\":").count();
    let structural_characters = value
        .chars()
        .filter(|character| matches!(character, '{' | '}' | '[' | ']' | ':' | ',' | ';' | '='))
        .count();
    let longest_token = value
        .split_whitespace()
        .map(|token| token.chars().count())
        .max()
        .unwrap_or_default();
    let script_syntax_count = [
        "=>",
        "&&",
        "||",
        "function(",
        "function ",
        ".push(",
        ".map(",
    ]
    .iter()
    .map(|marker| lower.matches(marker).count())
    .sum::<usize>();
    let css_property_count = [
        "background:",
        "display:",
        "float:",
        "height:",
        "margin-",
        "padding-",
        "position:",
        "width:",
    ]
    .iter()
    .map(|marker| lower.matches(marker).count())
    .sum::<usize>();

    (escaped_quotes >= 4 && (escaped_controls >= 2 || json_pairs >= 2))
        || escaped_controls >= 5
        || json_pairs >= 4
        || (json_pairs >= 6 && structural_characters.saturating_mul(8) >= character_count)
        || (longest_token >= 180 && structural_characters >= 12)
        || (script_syntax_count >= 3 && structural_characters >= 8)
        || (css_property_count >= 3 && value.contains('{') && value.contains('}'))
}

fn catalog_source_matches_query(query: &str, title: &str, chunks: &[String]) -> bool {
    let features = source_backed_query_features(query);
    if features.is_empty() {
        return false;
    }
    let haystack = format!("{title} {}", chunks.join(" ")).to_lowercase();
    features
        .iter()
        .any(|feature| haystack.contains(feature.as_str()))
}

fn source_backed_query_features(query: &str) -> Vec<String> {
    let mut features = HashSet::new();
    let mut token = String::new();
    let mut han = String::new();
    let flush_token = |token: &mut String, features: &mut HashSet<String>| {
        let normalized = token.to_lowercase();
        if normalized.chars().count() >= 3
            && !matches!(
                normalized.as_str(),
                "the"
                    | "and"
                    | "for"
                    | "with"
                    | "from"
                    | "into"
                    | "which"
                    | "what"
                    | "when"
                    | "where"
                    | "who"
                    | "why"
                    | "how"
                    | "are"
                    | "was"
                    | "were"
                    | "this"
                    | "that"
            )
        {
            features.insert(normalized);
        }
        token.clear();
    };
    let flush_han = |han: &mut String, features: &mut HashSet<String>| {
        let characters = han.chars().collect::<Vec<_>>();
        for pair in characters.windows(2) {
            features.insert(pair.iter().collect());
        }
        han.clear();
    };
    for character in query.chars() {
        if source_backed_han_character(character) {
            flush_token(&mut token, &mut features);
            han.push(character);
        } else if character.is_alphanumeric() {
            flush_han(&mut han, &mut features);
            token.push(character);
        } else {
            flush_token(&mut token, &mut features);
            flush_han(&mut han, &mut features);
        }
    }
    flush_token(&mut token, &mut features);
    flush_han(&mut han, &mut features);
    let mut features = features.into_iter().collect::<Vec<_>>();
    features.sort();
    features
}

fn source_backed_source_link(source: &DeepResearchCatalogSource, number: usize) -> String {
    format!("[{number}] {}", source_backed_source_title_link(source))
}

fn source_backed_source_title_link(source: &DeepResearchCatalogSource) -> String {
    let title = markdown_plain_text(&source.title);
    if source.anchor.starts_with("http://") || source.anchor.starts_with("https://") {
        format!("[{title}]({})", source.anchor)
    } else {
        format!("{title} — {}", markdown_plain_text(&source.anchor))
    }
}

fn source_backed_report_artifacts(artifacts: &ResearchReportArtifacts) -> bool {
    let markdown = read_small_utf8_file(&artifacts.markdown);
    let html = read_small_utf8_file(&artifacts.html);
    let (Some(markdown), Some(html)) = (markdown, html) else {
        return false;
    };
    looks_like_deep_research_source_backed_report(&markdown)
        && looks_like_deep_research_source_backed_report(&html)
        && !looks_like_deep_research_no_evidence_report(&markdown)
        && !looks_like_deep_research_no_evidence_report(&html)
        && !looks_like_deep_research_fallback_draft(&markdown)
        && !looks_like_deep_research_recovery_report(&markdown)
        && complete_html_document(&html)
        && has_research_report_substance(&markdown, &html)
}

fn looks_like_deep_research_source_backed_report(text: &str) -> bool {
    text.contains("这是可核查的来源证据视图")
        || text.contains("This is a verifiable source-evidence view")
}

fn no_evidence_report_artifacts(artifacts: &ResearchReportArtifacts) -> bool {
    let markdown = read_small_utf8_file(&artifacts.markdown);
    let html = read_small_utf8_file(&artifacts.html);
    let (Some(markdown), Some(html)) = (markdown, html) else {
        return false;
    };
    looks_like_deep_research_no_evidence_report(&markdown)
        && looks_like_deep_research_no_evidence_report(&html)
        && !looks_like_deep_research_fallback_draft(&markdown)
        && !looks_like_deep_research_fallback_draft(&html)
        && !looks_like_deep_research_recovery_report(&markdown)
        && !looks_like_deep_research_recovery_report(&html)
        && !deep_research_output_has_internal_leak(&markdown)
        && !deep_research_output_has_internal_leak(&html)
        && complete_html_document(&html)
}

fn looks_like_deep_research_no_evidence_report(text: &str) -> bool {
    let english = text.contains(
        "This retrieval obtained no source text that can be published safely, so no domain conclusion is generated.",
    ) && text.contains("No safely publishable source was obtained.");
    let chinese = text.contains("本次检索没有获得可安全发布的来源文字，因此不生成领域结论。")
        && text.contains("没有可安全发布的来源。");
    english || chinese
}

fn fenced_catalog_text(content: &str) -> String {
    let longest_run = content
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest_run.saturating_add(1).max(3));
    format!("{fence}\n{}\n{fence}", content.trim())
}

fn bounded_catalog_text(
    value: Option<&serde_json::Value>,
    maximum_chars: usize,
    predicate: impl Fn(&str) -> bool,
) -> Option<String> {
    let value = value?
        .as_str()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!value.is_empty() && value.chars().count() <= maximum_chars && predicate(&value))
        .then_some(value)
}

fn stable_catalog_identity(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
}

fn source_backed_han_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
    )
}
