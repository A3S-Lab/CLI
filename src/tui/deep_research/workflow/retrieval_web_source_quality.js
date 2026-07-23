  const hostMatchesDomain = (host, domain) =>
    host === domain || host.endsWith(`.${domain}`);
  const protectedPublisherLookalike = (host) => {
    const compact = String(host || "").toLowerCase().replace(/[^a-z0-9]/g, "");
    const protectedPublishers = [
      ["apnews", ["apnews.com"]],
      ["fifa", ["fifa.com"]],
      ["oecd", ["oecd.org"]],
      ["olympic", ["olympics.com", "olympic.org"]],
      ["reuters", ["reuters.com"]],
      ["sohu", ["sohu.com"]],
      ["worldbank", ["worldbank.org"]],
      ["xinhuanet", ["xinhuanet.com"]],
    ];
    return protectedPublishers.some(([marker, domains]) =>
      compact.includes(marker) &&
      !domains.some((domain) => hostMatchesDomain(host, domain))
    );
  };
  const fallbackCandidatePriority = (candidate) => {
    const host = urlHost(candidate && candidate.url);
    if (!host || protectedPublisherLookalike(host)) return 0;
    const lowConfidenceDomains = [
      "facebook.com",
      "instagram.com",
      "medium.com",
      "quora.com",
      "reddit.com",
      "substack.com",
      "tiktok.com",
      "twitter.com",
      "weibo.com",
      "x.com",
      "xiaohongshu.com",
      "youtube.com",
      "zhihu.com",
    ];
    if (lowConfidenceDomains.some((domain) => hostMatchesDomain(host, domain))) {
      return 0;
    }
    const institutionalDomains = [
      "crates.io",
      "docs.rs",
      "europa.eu",
      "fifa.com",
      "ietf.org",
      "oecd.org",
      "olympic.org",
      "olympics.com",
      "rfc-editor.org",
      "un.org",
      "w3.org",
      "who.int",
      "worldbank.org",
    ];
    if (
      institutionalDomains.some((domain) => hostMatchesDomain(host, domain)) ||
      /\.(?:gov|edu|int)$/i.test(host) ||
      /\.(?:ac|edu|gov)\.[a-z]{2}$/i.test(host)
    ) {
      return 3;
    }
    const editorialDomains = [
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
      "ifeng.com",
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
    return editorialDomains.some((domain) => hostMatchesDomain(host, domain))
      ? 2
      : 1;
  };
