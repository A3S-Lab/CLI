fn catalog_source_claim_eligible(
    anchor: &str,
    raw_chunks: &[serde_json::Value],
    semantic_source_admission: bool,
) -> bool {
    let host = url::Url::parse(anchor)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase));
    if host.as_deref().is_some_and(|host| {
        low_confidence_claim_host(host) || protected_publisher_lookalike(host)
    }) {
        return false;
    }
    let text = raw_chunks
        .iter()
        .filter_map(|chunk| chunk.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let low_confidence_markers = [
        "skypiea.tv",
        "免费观看",
        "无需注册",
        "高清流畅",
        "watch free",
        "free stream",
        "free live stream",
        "no registration",
        "user-generated content",
    ];
    let self_publishing_disclaimer = (text.contains("观点仅代表作者本人")
        && (text.contains("信息发布平台") || text.contains("信息存储空间服务")))
        || (text.contains("views expressed")
            && (text.contains("solely those of the author")
                || text.contains("only those of the author")))
        || (text.contains("publisher only provides storage")
            || text.contains("platform only provides information storage"));
    let content_is_eligible = !self_publishing_disclaimer
        && !low_confidence_markers
            .iter()
            .any(|marker| text.contains(marker));
    content_is_eligible
        && (semantic_source_admission || accountable_fallback_publisher(anchor))
}

fn host_matches_domain(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// A failed semantic admission cannot turn arbitrary search rank into source
/// authority. The deterministic fallback admits only local evidence, strict
/// institutional hosts, and a bounded set of publishers with an identifiable
/// editorial responsibility. Unknown hosts remain visible for audit but may
/// not support report claims.
fn accountable_fallback_publisher(anchor: &str) -> bool {
    if !anchor.starts_with("http://") && !anchor.starts_with("https://") {
        return true;
    }
    if catalog_source_is_institutional(anchor) {
        return true;
    }
    let Some(host) = url::Url::parse(anchor)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
    else {
        return false;
    };
    const ACCOUNTABLE_EDITORIAL_DOMAINS: &[&str] = &[
        "163.com",
        "apnews.com",
        "bbc.co.uk",
        "bbc.com",
        "bloomberg.com",
        "caixin.com",
        "cctv.cn",
        "cctv.com",
        "chinanews.com.cn",
        "espn.com",
        "ft.com",
        "news.cn",
        "nytimes.com",
        "people.com.cn",
        "reuters.com",
        "sina.cn",
        "sina.com.cn",
        "sohu.com",
        "theguardian.com",
        "thepaper.cn",
        "washingtonpost.com",
        "wsj.com",
        "xinmin.cn",
        "xinhuanet.com",
        "yicai.com",
        "zaobao.com.sg",
    ];
    ACCOUNTABLE_EDITORIAL_DOMAINS
        .iter()
        .any(|domain| host_matches_domain(&host, domain))
}

/// Reject registrable domains that embed a protected publisher or institution
/// name while living outside that publisher's real domain. Titles and page
/// prose are untrusted, so this check deliberately uses only the canonical
/// host identity.
fn protected_publisher_lookalike(host: &str) -> bool {
    let publisher = catalog_source_publisher_key(&format!("https://{host}"));
    let compact = publisher
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    const PROTECTED_PUBLISHERS: &[(&str, &[&str])] = &[
        ("apnews", &["apnews.com"]),
        ("fifa", &["fifa.com"]),
        ("oecd", &["oecd.org"]),
        ("olympic", &["olympics.com", "olympic.org"]),
        ("reuters", &["reuters.com"]),
        ("sohu", &["sohu.com"]),
        ("worldbank", &["worldbank.org"]),
        ("xinhuanet", &["xinhuanet.com"]),
    ];
    PROTECTED_PUBLISHERS.iter().any(|(marker, domains)| {
        compact.contains(marker)
            && !domains
                .iter()
                .any(|domain| host_matches_domain(host, domain))
    })
}

fn low_confidence_claim_host(host: &str) -> bool {
    [
        "xiaohongshu.com",
        "reddit.com",
        "x.com",
        "twitter.com",
        "facebook.com",
        "instagram.com",
        "tiktok.com",
        "weibo.com",
        "youtube.com",
        "zhihu.com",
        "quora.com",
        "medium.com",
        "substack.com",
    ]
    .iter()
    .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn catalog_source_is_institutional(anchor: &str) -> bool {
    if !anchor.starts_with("http://") && !anchor.starts_with("https://") {
        return true;
    }
    let Ok(url) = url::Url::parse(anchor) else {
        return false;
    };
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let known_institutional_domains = [
        "fifa.com",
        "olympic.org",
        "olympics.com",
        "un.org",
        "who.int",
        "worldbank.org",
        "oecd.org",
        "europa.eu",
        "ietf.org",
        "w3.org",
        "rfc-editor.org",
        "docs.rs",
        "crates.io",
    ];
    if known_institutional_domains
        .iter()
        .any(|domain| host_matches_domain(&host, domain))
    {
        return true;
    }
    let labels = host.split('.').collect::<Vec<_>>();
    let public_institution = host.ends_with(".gov")
        || host.ends_with(".edu")
        || host.ends_with(".int")
        || labels.get(labels.len().saturating_sub(2)).is_some_and(|label| {
            matches!(*label, "ac" | "edu" | "gov")
                && labels.last().is_some_and(|suffix| suffix.len() == 2)
        });
    public_institution
}

fn catalog_source_publisher_key(anchor: &str) -> String {
    let Ok(url) = url::Url::parse(anchor) else {
        return anchor.to_ascii_lowercase();
    };
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return anchor.to_ascii_lowercase();
    };
    let labels = host
        .trim_start_matches("www.")
        .split('.')
        .collect::<Vec<_>>();
    if labels.len() <= 2 {
        return labels.join(".");
    }
    let second_level_suffix = matches!(
        labels[labels.len() - 2],
        "ac" | "co" | "com" | "edu" | "gov" | "net" | "org"
    ) && labels.last().is_some_and(|label| label.len() == 2);
    let keep = if second_level_suffix { 3 } else { 2 };
    labels[labels.len().saturating_sub(keep)..].join(".")
}

fn catalog_source_latest_observed_date(
    source: &DeepResearchCatalogSource,
) -> Option<chrono::NaiveDate> {
    source
        .chunks
        .iter()
        .flat_map(|chunk| catalog_observed_dates(chunk))
        .max()
}

fn catalog_source_is_temporal_snapshot(source: &DeepResearchCatalogSource) -> bool {
    let text = source.chunks.join(" ").to_ascii_lowercase();
    [
        "截至目前",
        "截至当时",
        "截至",
        "当时",
        "as of",
        "at the time of writing",
        "at that point",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn catalog_observed_dates(value: &str) -> Vec<chrono::NaiveDate> {
    static NUMERIC_DATE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static CHINESE_DATE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static CHINESE_RANGE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static COMPACT_PATH_DATE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static NAMED_MONTH_DATE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let numeric_date = NUMERIC_DATE.get_or_init(|| {
        regex::Regex::new(
            r"(?:^|[^0-9])(?P<year>20[0-9]{2})[-/](?P<month>[0-9]{1,2})[-/](?P<day>[0-9]{1,2})(?:$|[^0-9])",
        )
        .expect("valid numeric date regex")
    });
    let chinese_date = CHINESE_DATE.get_or_init(|| {
        regex::Regex::new(
            r"(?P<year>20[0-9]{2})\s*年\s*(?P<month>[0-9]{1,2})\s*月\s*(?P<day>[0-9]{1,2})\s*日",
        )
        .expect("valid Chinese date regex")
    });
    let chinese_range = CHINESE_RANGE.get_or_init(|| {
        regex::Regex::new(
            r"(?P<year>20[0-9]{2})\s*年\s*[0-9]{1,2}\s*月\s*[0-9]{1,2}\s*日\s*(?:至|到|—|–|-)\s*(?P<month>[0-9]{1,2})\s*月\s*(?P<day>[0-9]{1,2})\s*日",
        )
        .expect("valid Chinese date-range regex")
    });
    let compact_path_date = COMPACT_PATH_DATE.get_or_init(|| {
        regex::Regex::new(
            r"(?:^|/)(?P<year>20[0-9]{2})/(?P<month>[01][0-9])(?P<day>[0-3][0-9])(?:/|$)",
        )
        .expect("valid compact path date regex")
    });
    let named_month_date = NAMED_MONTH_DATE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)\b(?P<month>January|February|March|April|May|June|July|August|September|October|November|December)\s+(?P<day>[0-9]{1,2})(?:st|nd|rd|th)?[,]?\s+(?P<year>20[0-9]{2})\b",
        )
        .expect("valid named-month date regex")
    });
    let mut dates = Vec::new();
    for captures in numeric_date.captures_iter(value) {
        push_catalog_numeric_date(&mut dates, &captures);
    }
    for captures in chinese_date.captures_iter(value) {
        push_catalog_numeric_date(&mut dates, &captures);
    }
    for captures in chinese_range.captures_iter(value) {
        push_catalog_numeric_date(&mut dates, &captures);
    }
    for captures in compact_path_date.captures_iter(value) {
        push_catalog_numeric_date(&mut dates, &captures);
    }
    for captures in named_month_date.captures_iter(value) {
        let month = match captures
            .name("month")
            .map(|value| value.as_str().to_ascii_lowercase())
            .as_deref()
        {
            Some("january") => 1,
            Some("february") => 2,
            Some("march") => 3,
            Some("april") => 4,
            Some("may") => 5,
            Some("june") => 6,
            Some("july") => 7,
            Some("august") => 8,
            Some("september") => 9,
            Some("october") => 10,
            Some("november") => 11,
            Some("december") => 12,
            _ => continue,
        };
        push_catalog_date_parts(&mut dates, &captures, month);
    }
    dates.sort_unstable();
    dates.dedup();
    dates
}

fn push_catalog_numeric_date(dates: &mut Vec<chrono::NaiveDate>, captures: &regex::Captures<'_>) {
    let Some(month) = captures
        .name("month")
        .and_then(|value| value.as_str().parse::<u32>().ok())
    else {
        return;
    };
    push_catalog_date_parts(dates, captures, month);
}

fn push_catalog_date_parts(
    dates: &mut Vec<chrono::NaiveDate>,
    captures: &regex::Captures<'_>,
    month: u32,
) {
    let Some(year) = captures
        .name("year")
        .and_then(|value| value.as_str().parse::<i32>().ok())
    else {
        return;
    };
    let Some(day) = captures
        .name("day")
        .and_then(|value| value.as_str().parse::<u32>().ok())
    else {
        return;
    };
    if let Some(date) = chrono::NaiveDate::from_ymd_opt(year, month, day) {
        dates.push(date);
    }
}
