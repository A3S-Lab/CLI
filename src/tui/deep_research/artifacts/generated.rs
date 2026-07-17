// Materialization and semantic depth gates for structured model reports.

pub(crate) fn materialize_deep_research_completed_report_from_generation(
    workspace: &Path,
    query: &str,
    generated: &GeneratedDeepResearchReport,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Result<ResearchReportArtifacts, String> {
    validate_generated_report_depth(generated, workflow_output)?;
    let markdown = completed_report_markdown_from_answer_text(query, &generated.markdown)
        .ok_or_else(|| {
            "content rejected: structured generation did not contain a completed Markdown report"
                .to_string()
        })?;
    let mut markdown = sanitize_unobserved_markdown_http_citations(
        &markdown,
        query,
        workflow_output,
        workflow_metadata,
    );
    let accepted_evidence = super::deep_research_evidence_ledger::accepted_evidence_ledger(
        workflow_output,
        workflow_metadata,
    );
    let grounding_texts =
        super::deep_research_evidence_ledger::report_grounding_texts(query, &accepted_evidence);
    if let Err(error) = super::deep_research_report_audit::validate_quantitative_grounding(
        &markdown,
        &grounding_texts,
    ) {
        if !workflow_allows_qualified_sanitization(workflow_output) {
            return Err(error);
        }
        markdown = super::deep_research_report_audit::sanitize_ungrounded_quantitative_claims(
            &markdown,
            &grounding_texts,
        )
        .ok_or(error)?;
        super::deep_research_report_audit::validate_quantitative_grounding(
            &markdown,
            &grounding_texts,
        )?;
    }
    let html = deep_research_completed_report_html_with_presentation(
        query,
        &markdown,
        Some(&generated.presentation),
        Some(&generated.editorial.thesis),
    );
    validate_deep_research_completed_report_content(
        &markdown,
        &html,
        query,
        workflow_output,
        workflow_metadata,
    )?;

    let slug = deep_research_report_slug(query);
    let (root, report_dir) = prepare_research_report_directory(workspace, &slug)?;
    write_research_report_pair(
        &report_dir.join("report.md"),
        markdown,
        &report_dir.join("index.html"),
        html,
    )?;

    let rel_html = format!(".a3s/research/{slug}/index.html");
    let artifacts = trusted_research_report_artifact_paths(&rel_html, &root)
        .ok_or_else(|| "completed report artifacts failed path validation".to_string())?;
    completed_research_report_artifacts(&artifacts)
        .then_some(artifacts)
        .ok_or_else(|| "completed report artifacts failed content validation".to_string())
}

fn workflow_allows_qualified_sanitization(workflow_output: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(workflow_output.trim())
        .ok()
        .is_some_and(|workflow| {
            workflow
                .pointer("/checker/decision")
                .and_then(serde_json::Value::as_str)
                == Some("degrade")
                || workflow
                    .pointer("/verification/status")
                    .and_then(serde_json::Value::as_str)
                    == Some("degraded")
                || workflow
                    .get("mode")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|mode| mode.contains("degraded"))
        })
}

fn validate_generated_report_depth(
    generated: &GeneratedDeepResearchReport,
    workflow_output: &str,
) -> Result<(), String> {
    let thesis = generated.editorial.thesis.trim();
    if thesis.chars().count() < 12 {
        return Err(
            "content rejected: the report has no substantive answer-first thesis".to_string(),
        );
    }
    if generated.presentation.rationale.trim().chars().count() < 12 {
        return Err(
            "content rejected: report-master presentation lacks a content-specific rationale"
                .to_string(),
        );
    }

    let planned_tracks = planned_report_tracks(workflow_output);
    let mut covered_tracks = HashSet::new();
    let mut coverage_tracks = Vec::with_capacity(generated.editorial.track_coverage.len());
    for coverage in &generated.editorial.track_coverage {
        let track = normalize_report_track(&coverage.track);
        if track.is_empty() || !covered_tracks.insert(track.clone()) {
            return Err(
                "content rejected: the editorial quality map contains an empty or duplicate research track"
                    .to_string(),
            );
        }
        coverage_tracks.push((track, coverage.track.trim().to_string()));
        if coverage.finding.trim().chars().count() < 8
            || coverage.interpretation.trim().chars().count() < 8
        {
            return Err(format!(
                "content rejected: research track {:?} lacks a finding or interpretation",
                coverage.track.trim()
            ));
        }
        if matches!(coverage.status, ReportTrackStatus::Bounded)
            && coverage.uncertainty.trim().is_empty()
        {
            return Err(format!(
                "content rejected: bounded research track {:?} does not state its uncertainty",
                coverage.track.trim()
            ));
        }
    }

    if !planned_tracks.is_empty() {
        let matched = matched_planned_report_tracks(&planned_tracks, &coverage_tracks);
        let missing = planned_tracks
            .iter()
            .enumerate()
            .filter(|(index, _)| !matched.contains(index))
            .map(|(_, (_, display))| display.as_str())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!(
                "content rejected: the report did not account for planned research track(s): {}",
                missing.join("; ")
            ));
        }
    }

    let answer_shape = serde_json::from_str::<serde_json::Value>(workflow_output.trim())
        .ok()
        .and_then(|workflow| {
            workflow
                .pointer("/plan/answer_shape")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
    let has_implication = generated
        .editorial
        .track_coverage
        .iter()
        .any(|coverage| !coverage.implication.trim().is_empty());
    let has_uncertainty = generated
        .editorial
        .track_coverage
        .iter()
        .any(|coverage| !coverage.uncertainty.trim().is_empty());
    match answer_shape.as_deref() {
        Some("investigation") if !has_implication || !has_uncertainty => Err(
            "content rejected: an investigation must explain implications and a counterpoint or uncertainty boundary"
                .to_string(),
        ),
        Some("briefing") if !has_implication => Err(
            "content rejected: a briefing must explain what the findings mean for the reader"
                .to_string(),
        ),
        _ => Ok(()),
    }
}

fn matched_planned_report_tracks(
    planned: &[(String, String)],
    coverage: &[(String, String)],
) -> HashSet<usize> {
    let mut candidates = planned
        .iter()
        .enumerate()
        .flat_map(|(planned_index, (_, planned_display))| {
            coverage.iter().enumerate().filter_map(
                move |(coverage_index, (_, coverage_display))| {
                    report_track_match_score(planned_display, coverage_display)
                        .map(|score| (score, planned_index, coverage_index))
                },
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });

    let mut matched_planned = HashSet::new();
    let mut matched_coverage = HashSet::new();
    for (_, planned_index, coverage_index) in candidates {
        if matched_planned.contains(&planned_index) || matched_coverage.contains(&coverage_index) {
            continue;
        }
        matched_planned.insert(planned_index);
        matched_coverage.insert(coverage_index);
    }
    matched_planned
}

fn report_track_match_score(planned: &str, coverage: &str) -> Option<usize> {
    let planned_full = normalize_report_track(planned);
    let coverage_full = normalize_report_track(coverage);
    if planned_full.is_empty() || coverage_full.is_empty() {
        return None;
    }
    if planned_full == coverage_full {
        return Some(1_000);
    }

    let planned_labels = report_track_labels(planned);
    let coverage_labels = report_track_labels(coverage);
    if planned_labels
        .iter()
        .any(|planned| coverage_labels.iter().any(|coverage| planned == coverage))
    {
        return Some(950);
    }

    let containment_score = planned_labels
        .iter()
        .flat_map(|planned| {
            coverage_labels
                .iter()
                .filter_map(move |coverage| report_track_containment_score(planned, coverage))
        })
        .max();
    if containment_score.is_some() {
        return containment_score;
    }

    let token_score = meaningful_report_track_tokens(&planned_full).and_then(|planned_tokens| {
        meaningful_report_track_tokens(&coverage_full).and_then(|coverage_tokens| {
            let common = planned_tokens.intersection(&coverage_tokens).count();
            let smaller = planned_tokens.len().min(coverage_tokens.len());
            (common >= 2 && common * 5 >= smaller * 3).then_some(600 + (common * 100 / smaller))
        })
    });
    if token_score.is_some() {
        return token_score;
    }

    let planned_bigrams = cjk_report_track_bigrams(&planned_full);
    let coverage_bigrams = cjk_report_track_bigrams(&coverage_full);
    let common = planned_bigrams.intersection(&coverage_bigrams).count();
    let smaller = planned_bigrams.len().min(coverage_bigrams.len());
    if common >= 2 && smaller > 0 && common * 5 >= smaller * 2 {
        Some(500 + (common * 100 / smaller))
    } else {
        None
    }
}

fn report_track_labels(track: &str) -> Vec<String> {
    let full = normalize_report_track(track);
    let label_end = track
        .char_indices()
        .find(|(_, ch)| matches!(ch, ':' | '：' | '—' | '–'))
        .map(|(index, _)| index)
        .unwrap_or(track.len());
    let label = normalize_report_track(&track[..label_end]);
    let mut labels = vec![full];
    if !label.is_empty() && !labels.contains(&label) {
        labels.push(label);
    }
    labels
}

fn report_track_containment_score(left: &str, right: &str) -> Option<usize> {
    let left_compact = left.replace(' ', "");
    let right_compact = right.replace(' ', "");
    let (shorter, longer) = if left_compact.chars().count() <= right_compact.chars().count() {
        (&left_compact, &right_compact)
    } else {
        (&right_compact, &left_compact)
    };
    let shorter_chars = shorter.chars().count();
    let contains_cjk = shorter.chars().any(is_cjk_report_track_char);
    let specific_enough = if contains_cjk {
        shorter_chars >= 4
    } else {
        shorter_chars >= 7
    };
    (specific_enough && longer.contains(shorter)).then_some(800 + shorter_chars.min(100))
}

fn meaningful_report_track_tokens(track: &str) -> Option<HashSet<String>> {
    const STOP_WORDS: &[&str] = &[
        "and",
        "for",
        "from",
        "into",
        "the",
        "versus",
        "with",
        "analysis",
        "assessment",
        "comparison",
        "evaluation",
        "overview",
        "review",
        "track",
    ];
    let tokens = track
        .split_whitespace()
        .filter(|token| token.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .filter(|token| token.len() >= 3 && !STOP_WORDS.contains(token))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    (tokens.len() >= 2).then_some(tokens)
}

fn cjk_report_track_bigrams(track: &str) -> HashSet<String> {
    let chars = track
        .chars()
        .filter(|ch| is_cjk_report_track_char(*ch))
        .collect::<Vec<_>>();
    chars
        .windows(2)
        .map(|pair| pair.iter().collect::<String>())
        .collect()
}

fn is_cjk_report_track_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{3040}'..='\u{30ff}'
            | '\u{ac00}'..='\u{d7af}'
    )
}

fn planned_report_tracks(workflow_output: &str) -> Vec<(String, String)> {
    let Ok(workflow) = serde_json::from_str::<serde_json::Value>(workflow_output.trim()) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let stable_obligations = workflow
        .pointer("/inquiry/state/obligations")
        .and_then(serde_json::Value::as_array)
        .filter(|obligations| !obligations.is_empty());
    stable_obligations
        .or_else(|| {
            workflow
                .pointer("/plan/tracks")
                .and_then(serde_json::Value::as_array)
        })
        .into_iter()
        .flatten()
        .filter_map(|track| {
            track
                .as_str()
                .or_else(|| track.get("title").and_then(serde_json::Value::as_str))
        })
        .map(str::trim)
        .filter(|track| !track.is_empty())
        .filter_map(|track| {
            let normalized = normalize_report_track(track);
            seen.insert(normalized.clone())
                .then(|| (normalized, track.to_string()))
        })
        .collect()
}

fn normalize_report_track(track: &str) -> String {
    track
        .chars()
        .flat_map(char::to_lowercase)
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
