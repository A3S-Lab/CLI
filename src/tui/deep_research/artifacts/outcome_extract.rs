const DETERMINISTIC_OUTCOME_MAX_FINDINGS: usize = 4;
const DETERMINISTIC_OUTCOME_MAX_SPAN_CHARS: usize = 420;

#[derive(Clone, Debug)]
struct DeterministicOutcomeCandidate {
    text: String,
    source_alias: String,
    source_index: usize,
    score: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeterministicOutcomeRole {
    DirectAnswer,
    Finding,
}

/// Build a narrow extractive report only when a trusted source contains an
/// assertive answer to an event-result query. Every published block is a
/// bounded source span and still passes the same Host admission gates as a
/// model proposal. Complex or ambiguous questions deliberately return `None`.
pub(crate) fn deterministic_deep_research_outcome_report_at(
    query: &str,
    current_date: &str,
    catalog: &DeepResearchSourceCatalog,
) -> Result<Option<AdmittedDeepResearchReport>, String> {
    if !report_query_asks_for_competition_outcome(query) {
        return Ok(None);
    }
    let current_date = chrono::NaiveDate::parse_from_str(current_date, "%Y-%m-%d")
        .map_err(|_| "deterministic report requires current_date in YYYY-MM-DD form".to_string())?;
    let mut direct_answers = Vec::new();
    let mut findings = Vec::new();

    for (source_index, source) in catalog.sources.iter().enumerate() {
        if !source.claim_eligible
            || !(catalog_source_is_institutional(&source.anchor)
                || accountable_fallback_publisher(&source.anchor))
            || !report_source_current_claim_eligible(query, current_date, catalog, source)
        {
            continue;
        }
        for chunk in &source.chunks {
            for text in deterministic_outcome_spans(chunk) {
                if deterministic_outcome_span_is_unsafe(&text) {
                    continue;
                }
                if deterministic_span_asserts_outcome(&text)
                    && report_summary_answers_query_intent(query, &text)
                {
                    direct_answers.push(DeterministicOutcomeCandidate {
                        score: deterministic_outcome_span_score(
                            query,
                            &text,
                            source,
                            source_index,
                            DeterministicOutcomeRole::DirectAnswer,
                        ),
                        text: text.clone(),
                        source_alias: source.alias.clone(),
                        source_index,
                    });
                }
                if deterministic_span_is_material_finding(&text) {
                    findings.push(DeterministicOutcomeCandidate {
                        score: deterministic_outcome_span_score(
                            query,
                            &text,
                            source,
                            source_index,
                            DeterministicOutcomeRole::Finding,
                        ),
                        text,
                        source_alias: source.alias.clone(),
                        source_index,
                    });
                }
            }
        }
    }

    deterministic_outcome_candidates_sort(&mut direct_answers);
    let Some(direct_answer) = direct_answers.first().cloned() else {
        return Ok(None);
    };
    // Cross-source findings are admitted only when an exact score and at least
    // two non-generic identity features match the selected outcome. This keeps
    // independent reporting without joining unrelated events that happen to
    // use the same result vocabulary.
    findings.retain(|candidate| {
        (candidate.source_index == direct_answer.source_index
            || deterministic_outcome_same_event(&candidate.text, &direct_answer.text))
            && !deterministic_outcome_spans_overlap(&candidate.text, &direct_answer.text)
            && deterministic_outcome_finding_adds_dimension(&candidate.text)
    });
    deterministic_outcome_candidates_sort(&mut findings);
    let mut distinct_findings = Vec::new();
    for candidate in findings {
        if distinct_findings.iter().any(
            |retained: &DeterministicOutcomeCandidate| {
                deterministic_outcome_spans_overlap(&candidate.text, &retained.text)
            },
        ) {
            continue;
        }
        distinct_findings.push(candidate);
    }
    let mut findings = distinct_findings;
    findings.truncate(DETERMINISTIC_OUTCOME_MAX_FINDINGS);
    if findings.is_empty() {
        return Ok(None);
    }

    let proposal = serde_json::json!({
        "summary": [{
            "text": direct_answer.text,
            "source_aliases": [direct_answer.source_alias]
        }],
        "findings": findings
            .into_iter()
            .map(|candidate| serde_json::json!({
                "text": candidate.text,
                "source_aliases": [candidate.source_alias]
            }))
            .collect::<Vec<_>>(),
        "recommendations": [],
        "limitations": []
    });
    admit_deep_research_report_proposal_at(
        query,
        current_date.to_string().as_str(),
        catalog,
        proposal,
    )
}

fn deterministic_outcome_spans(chunk: &str) -> Vec<String> {
    static STRUCTURAL_BOUNDARY: std::sync::OnceLock<regex::Regex> =
        std::sync::OnceLock::new();
    let structural_boundary = STRUCTURAL_BOUNDARY.get_or_init(|| {
        regex::Regex::new(r"(?:^|\s)(?:\*+|#{1,6}|•|\|)(?:\s|$)")
            .expect("static deterministic outcome structural-boundary regex")
    });
    let structured = structural_boundary.replace_all(chunk, "\n");
    let mut spans = Vec::new();
    for unit in structured.lines() {
        let normalized = unit.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut current = String::new();
        for character in normalized.chars() {
            current.push(character);
            if matches!(character, '.' | '!' | '?' | '。' | '！' | '？' | ';' | '；') {
                deterministic_outcome_push_span(&mut spans, &current);
                current.clear();
            }
        }
        deterministic_outcome_push_span(&mut spans, &current);
    }

    // Topic pages often place a linked headline and its prose on the same
    // physical line. Exact suffixes let the prose sentence outrank its
    // headline without asking a model to invent or paraphrase text.
    let complete_spans = spans.clone();
    for span in complete_spans {
        let words = span.split_whitespace().collect::<Vec<_>>();
        for offset in 1..words.len() {
            let suffix = words[offset..].join(" ");
            if deterministic_outcome_suffix_has_subject(&suffix) {
                deterministic_outcome_push_span(&mut spans, &suffix);
            }
        }
    }
    spans
}

fn deterministic_outcome_suffix_has_subject(value: &str) -> bool {
    let value = value.trim_start();
    value
        .chars()
        .next()
        .is_some_and(source_backed_han_character)
        && ![
            "并",
            "且",
            "以及",
            "凭借",
            "随后",
            "其中",
            "同时",
            "此外",
            "在",
            "于",
            "以",
            "战胜",
            "击败",
            "夺得",
            "获得",
            "成为",
            "排在",
            "位居",
        ]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

fn deterministic_outcome_push_span(spans: &mut Vec<String>, candidate: &str) {
    let candidate = candidate
        .trim_matches(|character: char| {
            character.is_whitespace() || matches!(character, '*' | '#' | '-' | '|' | '•')
        })
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let character_count = candidate.chars().count();
    if (4..=DETERMINISTIC_OUTCOME_MAX_SPAN_CHARS).contains(&character_count)
        && !spans.iter().any(|span| span == &candidate)
    {
        spans.push(candidate);
    }
}

fn deterministic_outcome_span_is_unsafe(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains('?')
        || text.contains('？')
        || [
            "谁将",
            "答案将在",
            "将于",
            "将在",
            "有望",
            "争夺",
            "敬请期待",
            "please click",
            "click here",
            "ignore previous",
            "system prompt",
            "will be",
            "will take place",
            "upcoming",
            "scheduled",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
        || [
            "查看详情",
            "通过本文查看",
            "返回首页",
            "返回搜狐",
            "欢迎评论",
            "点击浏览",
            "专题首页",
            "完整赛程",
            "每日赛果",
            "积分榜一览",
            "运动项目",
            "赔率",
            "竞彩",
            "投注",
            "博彩",
            "赢指",
            "输盘",
            "预测",
            "前瞻",
            "盘点近",
            "近3届",
            "近三届",
            "历届",
        ]
            .iter()
            .any(|marker| text.contains(marker))
        || [
            "olympic channel",
            "click to view",
            "read more",
            "betting odds",
            "odds to win",
            "prediction",
            "preview",
            "historical roundup",
        ]
            .iter()
            .any(|marker| lower.contains(marker))
        || text.contains('*')
        || text.contains('#')
        || text.contains('|')
}

fn deterministic_span_asserts_outcome(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "夺冠",
        "战胜",
        "击败",
        "获胜",
        "胜出",
        "赢得",
        "捧杯",
        "夺得世界杯冠军",
        "成为世界杯冠军",
        "获得世界杯冠军",
        "排名第一",
        "位居第一",
        "居首",
        "战平",
        "晋级",
        "出局",
        "落幕",
        "won",
        "is the winner",
        "became the winner",
        "crowned champion",
        "became champion",
        "is the champion",
        "beat",
        "defeated",
        "final result",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || (deterministic_outcome_score_literal_observed(&lower)
            && [
                "比分",
                "赛果",
                "战胜",
                "击败",
                "获胜",
                "result",
                "final",
                "beat",
                "defeated",
                "ended",
            ]
            .iter()
            .any(|marker| lower.contains(marker)))
}

fn deterministic_span_is_material_finding(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let material = [
        "冠军",
        "夺冠",
        "战胜",
        "击败",
        "胜利",
        "比分",
        "赛果",
        "结果",
        "排名",
        "金球",
        "金靴",
        "金手套",
        "零封",
        "决赛",
        "赛后",
        "冲突",
        "调查",
        "won",
        "winner",
        "champion",
        "beat",
        "defeated",
        "score",
        "ranking",
        "investigation",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || deterministic_outcome_score_literal_observed(&lower);
    material
        && (deterministic_span_has_assertive_finding_predicate(&lower)
            || deterministic_outcome_score_literal_observed(&lower))
}

fn deterministic_span_has_assertive_finding_predicate(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "战胜",
        "击败",
        "获胜",
        "胜出",
        "赢得",
        "夺得",
        "夺冠",
        "捧杯",
        "获得",
        "重回",
        "居第",
        "位居",
        "排在",
        "排名第一",
        "调查",
        "爆发",
        "发生",
        "表示",
        "配得上",
        "举行",
        "零封",
        "晋级",
        "出局",
        "结束",
        "落幕",
        "won",
        "winner is",
        "became the winner",
        "crowned champion",
        "became champion",
        "beat",
        "defeated",
        "ranked",
        "finished",
        "investigated",
        "investigating",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn deterministic_outcome_finding_adds_dimension(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "排名",
        "世界第一",
        "金球",
        "金靴",
        "金手套",
        "零封",
        "赛后",
        "冲突",
        "调查",
        "进球",
        "加时",
        "主帅",
        "表示",
        "配得上",
        "巡游",
        "庆祝",
        "纪录",
        "射手",
        "奖杯",
        "获奖",
        "ranking",
        "ranked",
        "world number one",
        "golden ball",
        "golden boot",
        "golden glove",
        "clean sheet",
        "after the match",
        "post-match",
        "investigat",
        "goal",
        "extra time",
        "coach",
        "said",
        "stated",
        "reaction",
        "parade",
        "celebrat",
        "record",
        "top scorer",
        "award",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn deterministic_outcome_span_score(
    query: &str,
    text: &str,
    source: &DeepResearchCatalogSource,
    source_index: usize,
    role: DeterministicOutcomeRole,
) -> i64 {
    let lower = text.to_lowercase();
    let overlap = source_backed_query_features(query)
        .iter()
        .filter(|feature| lower.contains(feature.as_str()))
        .map(|feature| feature.chars().count() as i64)
        .sum::<i64>();
    let outcome_signals = [
        "冠军", "夺冠", "战胜", "击败", "胜利", "比分", "赛果", "结果", "won", "winner",
        "champion", "beat", "defeated", "score",
    ]
    .iter()
    .filter(|marker| lower.contains(**marker))
    .count() as i64;
    let aftermath_signals = [
        "排名",
        "金球",
        "金靴",
        "金手套",
        "零封",
        "赛后",
        "冲突",
        "调查",
        "ranking",
        "investigation",
    ]
    .iter()
    .filter(|marker| lower.contains(**marker))
    .count() as i64;
    let score_signal = i64::from(deterministic_outcome_score_literal_observed(&lower));
    let trust = if catalog_source_is_institutional(&source.anchor) {
        80
    } else {
        60
    };
    let character_count = text.chars().count() as i64;
    let numeric_literal_count = report_numeric_literals(text).len() as i64;
    let whitespace_claim_count = text
        .split_whitespace()
        .filter(|segment| deterministic_span_has_assertive_finding_predicate(segment))
        .count() as i64;
    let role_score = match role {
        DeterministicOutcomeRole::DirectAnswer => {
            overlap * 120 + outcome_signals * 180 - aftermath_signals * 120 + score_signal * 600
        }
        DeterministicOutcomeRole::Finding => {
            overlap * 120
                + outcome_signals * 180
                + aftermath_signals * 180
                + numeric_literal_count * 600
                - whitespace_claim_count.saturating_sub(1) * 750
        }
    };
    role_score + trust + catalog_excerpt_readability_score(text) - character_count * 3
        - source_index as i64
}

fn deterministic_outcome_candidates_sort(candidates: &mut [DeterministicOutcomeCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.source_index.cmp(&right.source_index))
            .then_with(|| left.text.cmp(&right.text))
    });
}

fn deterministic_outcome_score_literal_observed(value: &str) -> bool {
    static SCORE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    SCORE
        .get_or_init(|| {
            regex::Regex::new(
                r"(?:^|[^0-9])(?:0|[1-9][0-9]?)\s*(?:-|:|：)\s*(?:0|[1-9][0-9]?)(?:$|[^0-9])",
            )
            .expect("static deterministic outcome score regex")
        })
        .is_match(value)
}

fn deterministic_outcome_same_event(left: &str, right: &str) -> bool {
    let left_scores = deterministic_outcome_score_pairs(left);
    let right_scores = deterministic_outcome_score_pairs(right);
    if left_scores.len() != 1 || left_scores != right_scores {
        return false;
    }
    let left_features = deterministic_outcome_identity_features(left);
    let right_features = deterministic_outcome_identity_features(right);
    left_features.intersection(&right_features).take(2).count() == 2
}

fn deterministic_outcome_score_pairs(value: &str) -> HashSet<String> {
    static SCORE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    SCORE
        .get_or_init(|| {
            regex::Regex::new(
                r"((?:0|[1-9][0-9]?))\s*(?:-|:|：)\s*((?:0|[1-9][0-9]?))",
            )
            .expect("static deterministic outcome score-pair regex")
        })
        .captures_iter(value)
        .filter_map(|captures| {
            let whole = captures.get(0)?;
            if value[..whole.start()]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_ascii_digit())
                || value[whole.end()..]
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit())
            {
                return None;
            }
            Some(format!(
                "{}:{}",
                captures.get(1)?.as_str(),
                captures.get(2)?.as_str()
            ))
        })
        .collect()
}

fn deterministic_outcome_identity_features(value: &str) -> HashSet<String> {
    const NOISE: &[&str] = &[
        "after",
        "and",
        "beat",
        "champion",
        "cup",
        "defeated",
        "final",
        "game",
        "goal",
        "match",
        "result",
        "score",
        "team",
        "the",
        "winner",
        "won",
        "世界杯",
        "世界",
        "界杯",
        "冠军",
        "决赛",
        "赛中",
        "比赛",
        "比分",
        "赛果",
        "结果",
        "球队",
        "国队",
        "加时",
        "时赛",
        "战胜",
        "击败",
        "获胜",
        "胜出",
        "分钟",
        "钟打",
        "打入",
        "制胜",
        "胜球",
        "进球",
        "夺冠",
    ];
    source_backed_query_features(value)
        .into_iter()
        .map(|feature| feature.to_lowercase())
        .filter(|feature| !NOISE.contains(&feature.as_str()))
        .collect()
}

fn deterministic_outcome_spans_overlap(left: &str, right: &str) -> bool {
    let left = left
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let right = right
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    left == right || left.contains(&right) || right.contains(&left)
}
