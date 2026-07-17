  const visibleEvidenceText = (value) => String(value || "")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/https?:\/\/[^\s<>\"']+/gi, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/[*_`#]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  const isPageChromeLine = (line) => {
    const text = String(line || "");
    const visible = visibleEvidenceText(text);
    if (
      /search code,\s*repositories,\s*users,\s*issues,\s*pull requests/i.test(text) ||
      /(?:data-color-mode|:focus-visible|--color-|headermenu|github copilot|write better code with ai|__next_f\.push|self\.\\?_\\?_next\\?_f\.push|__next_data__|webpack|datalayer|adsbygoogle|google_ad_client|globalnav|createaccountbutton|signinlabel)/i.test(text) ||
      /node_id.{0,400}(?:followers_url|following_url|stargazers_url)/i.test(text) ||
      /^(?:provide feedback|sign in|sign up|navigation menu|appearance settings|search or jump to)$/i.test(visible)
    ) {
      return true;
    }
    const markdownLinks = text.match(/\[[^\]]+\]\([^)]+\)/g) || [];
    return (markdownLinks.length >= 2 && visible.length < 80) ||
      (markdownLinks.length >= 3 && visible.length < 240 && !/[.!?。！？]/.test(visible));
  };
  const evidenceFocusTerms = (value) => {
    const stopwords = new Set([
      "and", "comparison", "design", "for", "from", "official", "results", "source",
      "the", "to", "versus", "vs", "with"
    ]);
    return unicodeSearchTerms(value, 32)
      .filter((term) => !stopwords.has(term))
      .slice(0, 16);
  };
  const researchContextText = () => {
    const planValues = (value) => {
      if (typeof value === "string") {
        return [value];
      }
      if (Array.isArray(value)) {
        return value.flatMap(planValues);
      }
      if (!value || typeof value !== "object") {
        return [];
      }
      return [
        value.title,
        value.focus,
        value.name,
        value.success_criterion
      ].filter((item) => typeof item === "string");
    };
    return [
      String(query || ""),
      ...planValues(researchPlan && researchPlan.tracks),
      ...planValues(researchPlan && researchPlan.search_queries),
      ...planValues(researchPlan && researchPlan.phases)
    ].join(" ");
  };
  const sourceIdentityTerms = (value) => {
    let decoded = String(value || "").toLowerCase();
    try {
      decoded = decodeURIComponent(decoded);
    } catch (_err) {
      // Keep the original URL when malformed percent escapes prevent decoding.
    }
    const infrastructureTerms = new Set([
      "api", "blob", "com", "dev", "docs", "documentation", "example", "git",
      "github", "gitlab", "guide", "head", "html", "http", "https", "index",
      "main", "markdown", "master", "pages", "raw", "readme", "repository",
      "source", "tree", "wiki", "www"
    ]);
    const terms = [];
    for (const token of decoded.match(/[a-z0-9][a-z0-9+_.-]{2,}/g) || []) {
      terms.push(token);
      terms.push(...token.split(/[-_.]+/));
    }
    return uniqueStrings(terms)
      .filter((term) =>
        term.length >= 3 &&
        !/^\d+$/.test(term) &&
        !infrastructureTerms.has(term)
      )
      .slice(0, 16);
  };
  const pageIdentityTerms = (item, text) => uniqueStrings([
    ...sourceIdentityTerms(item && item.url),
    ...pageHeadingTitles(text).flatMap((title) => evidenceFocusTerms(title))
  ]).slice(0, 24);
  const contextAnchoredIdentityTerms = (item, text) => {
    const context = researchContextText();
    return pageIdentityTerms(item, text)
      .filter((term) => queryTermMatches(context, term))
      .slice(0, 12);
  };
  const pageEvidenceFocusTerms = (item, text) => {
    const explicit = evidenceFocusTerms(
      Array.isArray(item && item.evidence_queries)
        ? item.evidence_queries.join(" ")
        : ""
    );
    return uniqueStrings([
      ...explicit,
      ...contextAnchoredIdentityTerms(item, text),
      ...queryTerms()
    ]).slice(0, 24);
  };
  const evidenceFocusRelevanceScore = (item) => {
    const focus = Array.isArray(item && item.evidence_queries)
      ? item.evidence_queries.join(" ")
      : "";
    const terms = evidenceFocusTerms(focus);
    if (terms.length === 0) {
      return 0;
    }
    const title = String(item.title || "");
    const url = String(item.url || "");
    const content = String(item.content || "");
    return terms.reduce((score, term) => score +
      (queryTermMatches(title, term) ? 5 : 0) +
      (queryTermMatches(url, term) ? 4 : 0) +
      (queryTermMatches(content, term) ? 1 : 0), 0);
  };
  const evidenceSnippet = (text, fallback, limit, focus) => {
    const compactFallback = compactText(fallback || "", limit);
    if (!isNonEmptyString(text)) {
      return compactFallback;
    }
    const focusedTerms = Array.isArray(focus)
      ? uniqueStrings(focus).slice(0, 24)
      : evidenceFocusTerms(focus);
    const terms = focusedTerms.length > 0 ? focusedTerms : queryTerms();
    const lines = text
      .split(/\n+/)
      .map((line) => line.replace(/\s+/g, " ").trim())
      .filter((line) =>
        line.length >= 30 &&
        !/"@context"\s*:\s*"https?:\\?\/\\?\/schema\.org|"@type"\s*:\s*"(?:BlogPosting|Article|WebPage)"/i.test(line) &&
        !/function\s*\(|var\s+className=|document\.cookie|mw\.config|client-js|<script/i.test(line) &&
        !isPageChromeLine(line)
      )
      .map((line, index) => ({ line, index, visible: visibleEvidenceText(line) }));
    const ranked = lines
      .map((item) => ({
        ...item,
        score: terms.reduce(
          (sum, term) => sum + (queryTermMatches(item.visible, term) ? 1 : 0),
          0
        )
      }))
      .filter((item) => item.score > 0)
      .sort((a, b) => b.score - a.score || a.index - b.index);
    const selectedIndexes = new Set();
    for (const item of ranked.slice(0, 3)) {
      selectedIndexes.add(item.index);
      if (lines[item.index + 1]) {
        selectedIndexes.add(item.index + 1);
      }
    }
    const selected = (selectedIndexes.size > 0
      ? Array.from(selectedIndexes)
          .sort((a, b) => a - b)
          .map((index) => lines[index].line)
      : (terms.length === 0 ? lines.slice(0, 3).map((item) => item.line) : []))
      .join(" ");
    return compactText(selected || compactFallback, limit);
  };
  const cleanFallbackSearchUrl = (value) => {
    let url = String(value || "").trim();
    while (/[.,;:!?\]}]$/.test(url)) {
      url = url.slice(0, -1);
    }
    while (url.endsWith(")")) {
      const openings = (url.match(/\(/g) || []).length;
      const closings = (url.match(/\)/g) || []).length;
      if (closings <= openings) {
        break;
      }
      url = url.slice(0, -1);
    }
    return url;
  };
  const evidenceFetchUrl = (value) => {
    const url = String(value || "").trim();
    let match = url.match(/^https:\/\/github\.com\/([^/?#]+)\/([^/?#]+)\/blob\/([^/?#]+)\/(.+?)(?:[?#].*)?$/i);
    if (match) {
      return `https://raw.githubusercontent.com/${match[1]}/${match[2]}/${match[3]}/${match[4]}`;
    }
    match = url.match(/^https:\/\/github\.com\/([^/?#]+)\/([^/?#]+)\/wiki\/(.+?)(?:[?#].*)?$/i);
    if (match) {
      const page = match[3].replace(/\.md$/i, "");
      return `https://raw.githubusercontent.com/wiki/${match[1]}/${match[2]}/${page}.md`;
    }
    match = url.match(/^https:\/\/github\.com\/([^/?#]+)\/([^/?#]+)\/releases\/?(?:[?#].*)?$/i);
    if (match) {
      return `https://api.github.com/repos/${match[1]}/${match[2]}/releases?per_page=10`;
    }
    match = url.match(/^https:\/\/github\.com\/([^/?#]+)\/([^/?#]+)\/?(?:[?#].*)?$/i);
    if (match) {
      return `https://raw.githubusercontent.com/${match[1]}/${match[2]}/HEAD/README.md`;
    }
    match = url.match(/^https:\/\/(?:www\.)?crates\.io\/crates\/([^/?#]+)\/?(?:[?#].*)?$/i);
    return match ? `https://crates.io/api/v1/crates/${match[1]}` : url;
  };
  const parseSearchResults = (output) => {
    const text = typeof output === "string" ? output : "";
    if (!text.trim()) {
      return [];
    }
    try {
      const parsed = JSON.parse(text);
      const items = Array.isArray(parsed)
        ? parsed
        : (parsed && Array.isArray(parsed.results) ? parsed.results : []);
      return items
        .filter((item) => item && typeof item === "object")
        .map((item) => ({
          title: isNonEmptyString(item.title) ? item.title.trim() : "",
          url: isNonEmptyString(item.url) ? item.url.trim() : (isNonEmptyString(item.url_or_path) ? item.url_or_path.trim() : ""),
          content: isNonEmptyString(item.content) ? item.content.trim() : (isNonEmptyString(item.snippet) ? item.snippet.trim() : ""),
          date: isNonEmptyString(item.published_date)
            ? item.published_date.trim()
            : (isNonEmptyString(item.publication_date)
              ? item.publication_date.trim()
              : (isNonEmptyString(item.date) ? item.date.trim() : "")),
          engines: Array.isArray(item.engines)
            ? item.engines.filter((engine) => typeof engine === "string")
            : []
        }))
        .filter((item) => /^https?:\/\//i.test(item.url));
    } catch (_err) {
      if (/No results found|Engine errors:|search\.brave\.com\/search/i.test(text)) {
        return [];
      }
      const urls = (text.match(/https?:\/\/[^\s<>"']+/g) || [])
        .map(cleanFallbackSearchUrl)
        .filter(Boolean);
      return urls.map((url, index) => ({
        title: `Search result ${index + 1}`,
        url,
        content: "",
        engines: []
      }));
    }
  };
  const batchOutputSections = (output, expectedCount) => {
    const text = String(output || "");
    const sections = Array.from({ length: expectedCount }, () => "");
    const markers = [];
    const markerPattern = /^--- \[(\d+):[^\n]*\] ---\r?\n/gm;
    let match = null;
    while ((match = markerPattern.exec(text)) !== null) {
      markers.push({
        index: Number(match[1]) - 1,
        headerStart: match.index,
        bodyStart: markerPattern.lastIndex
      });
    }
    for (let position = 0; position < markers.length; position += 1) {
      const marker = markers[position];
      if (!Number.isInteger(marker.index) || marker.index < 0 || marker.index >= expectedCount) {
        continue;
      }
      const end = position + 1 < markers.length
        ? markers[position + 1].headerStart
        : text.length;
      sections[marker.index] = text
        .slice(marker.bodyStart, end)
        .replace(/\nBatch completed with[\s\S]*$/, "")
        .replace(/^ERROR:\s*/, "")
        .trim();
    }
    return sections;
  };
  const batchChildResult = (batchResult, sections, index) => {
    const results = batchResult && batchResult.metadata && Array.isArray(batchResult.metadata.results)
      ? batchResult.metadata.results
      : [];
    const metadata = results.find((item) => Number(item && item.index) === index) || null;
    return {
      success: metadata ? metadata.success === true : Boolean(
        batchResult && Number(batchResult.exitCode) === 0 && isNonEmptyString(sections[index])
      ),
      output: sections[index] || "",
      metadata: metadata && metadata.metadata && typeof metadata.metadata === "object"
        ? metadata.metadata
        : {},
      error_kind: metadata ? metadata.error_kind : null
    };
  };
  const directWebQueries = () => {
    if (requestedSearchQueries !== null || plannedSearchQueries.length > 0) {
      return uniqueStrings(plannedSearchQueries).slice(0, directWebSearchLimit);
    }
    const base = searchableQueryText();
    if (!base) {
      return [];
    }
    const lower = base.toLowerCase();
    const years = uniqueStrings(base.match(/\b(?:19|20)\d{2}\b/g) || []);
    const latin = uniqueStrings((base.match(/[a-z][a-z0-9_.-]{2,}/gi) || [])
      .filter((term) => !/^(and|based|brief|citations?|compare|comparison|documentation|evidence|for|from|official|report|source|the|to|using|versus|with)$/i.test(term)))
      .slice(0, 8);
    const crossLanguage = [...latin, ...years].filter(Boolean).join(" ").trim();
    const candidates = crossLanguage && crossLanguage.toLowerCase() !== base.toLowerCase()
      ? [crossLanguage, base]
      : [base];
    if (!/(official|primary source|官网|官方|原始来源|权威来源)/i.test(lower)) {
      candidates.push(`${base} official source`);
    }
    if (/\b(version|release|stable)\b/i.test(lower)) {
      const releaseQuery = base
        .replace(/\b(official|source|sources|primary)\b/gi, " ")
        .replace(/\s+/g, " ")
        .trim();
      if (releaseQuery) {
        candidates.push(`${releaseQuery} release`);
      }
    }
    return uniqueStrings(candidates).slice(0, directWebSearchLimit);
  };
  const uniqueSearchResults = (items) => {
    const seen = new Map();
    const out = [];
    const ranked = items
      .map((item) => {
        const globalScore = sourceRelevanceScore(item);
        const focusScore = evidenceFocusRelevanceScore(item);
        return Object.assign({}, item, {
          relevance_score: (Number.isFinite(globalScore) ? globalScore : 0) + focusScore,
          evidence_focus_score: focusScore
        });
      })
      // Retrieval queries are authored semantically by the planner or inquiry
      // model and may be in a different language from the user's request.
      // Candidate admission therefore follows the query that produced the
      // result; the later fetch/checker gates still decide evidentiary value.
      // A model-authored retrieval query may expose one high-value signal in a
      // partial search result. Admit that lead for fetching; substantive page
      // text and the independent checker remain authoritative for evidence.
      .filter((item) => item.planned_seed === true || item.evidence_focus_score > 0)
      .sort((a, b) => b.relevance_score - a.relevance_score);
    for (const item of ranked) {
      const url = isNonEmptyString(item.url) ? item.url.trim() : "";
      const canonicalUrl = normalizeObservedSource(url);
      const dedupeKey = canonicalObservedSourceKey(canonicalUrl);
      if (!canonicalUrl || !/^https?:\/\//i.test(canonicalUrl)) {
        continue;
      }
      if (excludedSourceKeys.has(dedupeKey)) {
        continue;
      }
      if (seen.has(dedupeKey)) {
        const existing = out[seen.get(dedupeKey)];
        if (!isNonEmptyString(existing.date) && isNonEmptyString(item.date)) {
          existing.date = item.date;
        }
        existing.engines = uniqueStrings([
          ...(Array.isArray(existing.engines) ? existing.engines : []),
          ...(Array.isArray(item.engines) ? item.engines : [])
        ]);
        existing.evidence_queries = uniqueStrings([
          ...(Array.isArray(existing.evidence_queries) ? existing.evidence_queries : []),
          ...(Array.isArray(item.evidence_queries) ? item.evidence_queries : [])
        ]);
        continue;
      }
      seen.set(dedupeKey, out.length);
      out.push(item);
    }
    return out;
  };
  const diverseFetchCandidates = (items, limit) => {
    if (limit <= 0) {
      return [];
    }
    const selected = [];
    const selectedKeys = new Set();
    const seenHosts = new Set();
    for (const item of items) {
      const host = observedSourceHost(item.url);
      if (!host || seenHosts.has(host)) {
        continue;
      }
      selected.push(item);
      selectedKeys.add(canonicalObservedSourceKey(item.url));
      seenHosts.add(host);
      if (selected.length >= limit) {
        return selected;
      }
    }
    for (const item of items) {
      const key = canonicalObservedSourceKey(item.url);
      if (!key || selectedKeys.has(key)) {
        continue;
      }
      selected.push(item);
      selectedKeys.add(key);
      if (selected.length >= limit) {
        break;
      }
    }
    return selected;
  };
  const queryAwareFetchCandidates = (searches, limit) => {
    if (limit <= 0) {
      return [];
    }
    const selected = [];
    const selectedKeys = new Set();
    const searchGroups = searches.filter((item) => item.planned_seed_group !== true);
    const seedGroups = searches.filter((item) => item.planned_seed_group === true);
    const orderedGroups = [...searchGroups, ...seedGroups];
    for (const search of orderedGroups) {
      const taggedResults = (Array.isArray(search.results) ? search.results : []).map((item) =>
        Object.assign({}, item, {
          evidence_queries: search.planned_seed_group === true
            ? []
            : uniqueStrings([
                ...(Array.isArray(item && item.evidence_queries) ? item.evidence_queries : []),
                search.query
              ])
        })
      );
      const candidate = uniqueSearchResults(taggedResults)
        .find((item) => {
          const key = canonicalObservedSourceKey(item.url);
          return key && !selectedKeys.has(key);
        });
      if (!candidate) {
        continue;
      }
      selected.push(candidate);
      selectedKeys.add(canonicalObservedSourceKey(candidate.url));
      if (selected.length >= limit) {
        return selected;
      }
    }
    const remaining = uniqueSearchResults(searches.flatMap((search) =>
      (Array.isArray(search.results) ? search.results : []).map((item) =>
        Object.assign({}, item, {
          evidence_queries: search.planned_seed_group === true
            ? []
            : uniqueStrings([
                ...(Array.isArray(item && item.evidence_queries) ? item.evidence_queries : []),
                search.query
              ])
        })
      )
    ))
      .filter((item) => !selectedKeys.has(canonicalObservedSourceKey(item.url)));
    const fill = diverseFetchCandidates(remaining, limit - selected.length);
    return selected.concat(fill);
  };
  const fetchedTextForUrl = (fetches, url) => {
    const item = fetches.find((entry) => entry.url === url && entry.ok && isNonEmptyString(entry.output));
    return item ? item.output : "";
  };
  const fetchedTextForQueryMatching = (text) =>
    String(text || "").replace(/https?:\/\/[^\s<>"']+/gi, " ");
  const textMatchesQuery = (text) => {
    const terms = queryTerms();
    const matchable = fetchedTextForQueryMatching(text);
    return terms.length === 0 || terms.some((term) => queryTermMatches(matchable, term));
  };
  const textMatchesEvidenceFocus = (text, item) => {
    const matchable = fetchedTextForQueryMatching(text);
    const explicitTerms = evidenceFocusTerms(
      Array.isArray(item && item.evidence_queries)
        ? item.evidence_queries.join(" ")
        : ""
    );
    if (explicitTerms.length > 0) {
      const matched = explicitTerms.filter((term) => queryTermMatches(matchable, term)).length;
      return matched >= Math.min(2, explicitTerms.length);
    }
    const anchoredTerms = contextAnchoredIdentityTerms(item, text);
    if (anchoredTerms.length > 0) {
      const matched = anchoredTerms.filter((term) => queryTermMatches(matchable, term)).length;
      return matched >= Math.min(2, anchoredTerms.length);
    }
    return textMatchesQuery(text);
  };
  const normalizedSourceDate = (value) => {
    if (!isNonEmptyString(value)) {
      return "";
    }
    const date = compactText(sanitizeEvidenceText(value), 80).trim();
    return /^(unknown|n\/?a|none|null|undated|not\s+available|unavailable|未知|无日期|暂无)$/i.test(date)
      ? ""
      : date;
  };
  const pageHeadingTitles = (text) => {
    const titles = [];
    for (const line of String(text || "").split(/\r?\n/)) {
      const match = line.trim().match(/^#\s+(.+)$/);
      if (!match) {
        continue;
      }
      const title = sanitizeEvidenceText(match[1])
        .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
        .replace(/[*_`]/g, "")
        .trim();
      if (title.length >= 3 && title.length <= 180 && !isPageChromeLine(title)) {
        titles.push(title);
      }
    }
    return uniqueStrings(titles);
  };
  const fetchedPageTitle = (text, url) => {
    const context = researchContextText();
    const identityTerms = sourceIdentityTerms(url);
    return pageHeadingTitles(text)
      .map((title, index) => {
        const titleTerms = evidenceFocusTerms(title);
        const identityMatches = identityTerms.filter((term) => queryTermMatches(title, term)).length;
        const contextMatches = titleTerms.filter((term) => queryTermMatches(context, term)).length;
        return { title, index, score: identityMatches * 4 + contextMatches * 2 };
      })
      .sort((left, right) => right.score - left.score || left.index - right.index)
      .map((candidate) => candidate.title)[0] || "";
  };
  const sourceTitleFromUrl = (url) => {
    const match = String(url || "").match(/^https?:\/\/([^/?#]+)(\/[^?#]*)?/i);
    if (!match) {
      return "Source";
    }
    const host = match[1].replace(/^www\./i, "");
    const segments = String(match[2] || "")
      .split("/")
      .filter(Boolean)
      .slice(0, 4);
    if (host === "github.com" && segments.length >= 2) {
      const suffix = segments.length > 2 ? ` · ${segments.slice(2).join(" / ")}` : "";
      return `${segments[0]}/${segments[1]}${suffix} · GitHub`;
    }
    if (host === "docs.rs" && segments.length > 0) {
      return `${segments[0]} documentation · docs.rs`;
    }
    return host;
  };
  const resolveFetchedPageLink = (target, baseUrl) => {
    const value = String(target || "")
      .trim()
      .replace(/^<|>$/g, "")
      .replace(/&amp;/gi, "&");
    if (!value || /^(?:#|mailto:|javascript:|data:|tel:)/i.test(value)) {
      return "";
    }
    if (/^https?:\/\//i.test(value)) {
      return normalizeObservedSource(value);
    }
    const base = String(baseUrl || "").match(/^(https?):\/\/([^/?#]+)(\/[^?#]*)?/i);
    if (!base) {
      return "";
    }
    if (/^\/\//.test(value)) {
      return normalizeObservedSource(`${base[1]}:${value}`);
    }
    const targetPath = value.split(/[?#]/, 1)[0];
    const basePath = base[3] || "/";
    const joinedPath = targetPath.startsWith("/")
      ? targetPath
      : `${basePath.endsWith("/") ? basePath : basePath.replace(/[^/]*$/, "")}${targetPath}`;
    const segments = [];
    for (const segment of joinedPath.split("/")) {
      if (!segment || segment === ".") {
        continue;
      }
      if (segment === "..") {
        segments.pop();
        continue;
      }
      segments.push(segment);
    }
    return normalizeObservedSource(`${base[1]}://${base[2]}/${segments.join("/")}`);
  };
  const fetchedPageCandidateLeads = (results, fetches) => {
    const context = researchContextText();
    const resultByKey = new Map(
      results.map((item) => [canonicalObservedSourceKey(item.url), item])
    );
    const candidates = [];
    for (const fetch of fetches) {
      if (!fetch.ok || !isNonEmptyString(fetch.output)) {
        continue;
      }
      const sourceItem = resultByKey.get(canonicalObservedSourceKey(fetch.url)) || {};
      const explicitTerms = evidenceFocusTerms(
        Array.isArray(sourceItem.evidence_queries)
          ? sourceItem.evidence_queries.join(" ")
          : ""
      );
      const observed = [];
      const markdownLinks = /\[([^\]]*)\]\(([^\s)]+)(?:\s+"[^"]*")?\)/g;
      for (const match of String(fetch.output).matchAll(markdownLinks)) {
        observed.push({ title: visibleEvidenceText(match[1]), target: match[2] });
      }
      const htmlLinks = /\bhref\s*=\s*["']([^"']+)["']/gi;
      for (const match of String(fetch.output).matchAll(htmlLinks)) {
        observed.push({ title: "", target: match[1] });
      }
      const sourceCandidates = [];
      for (const link of observed) {
        const url = resolveFetchedPageLink(link.target, fetch.fetch_url || fetch.url);
        const key = canonicalObservedSourceKey(url);
        if (!url || !key || key === canonicalObservedSourceKey(fetch.fetch_url || fetch.url)) {
          continue;
        }
        // Rank the link by its own anchor and target, not by repeated identity
        // tokens inherited from the fetched parent page or repository host.
        const linkSignalText = `${link.title || ""} ${link.target || ""}`;
        const identityTerms = sourceIdentityTerms(linkSignalText);
        const anchored = identityTerms.filter((term) => queryTermMatches(context, term)).length;
        const explicit = explicitTerms
          .filter((term) => queryTermMatches(linkSignalText, term)).length;
        if (anchored === 0 && explicit === 0) {
          continue;
        }
        sourceCandidates.push({
          title: link.title || sourceTitleFromUrl(url),
          url,
          queries: uniqueStrings(
            Array.isArray(sourceItem.evidence_queries) ? sourceItem.evidence_queries : []
          ).slice(0, 2),
          observed_from: normalizeObservedSource(fetch.url),
          source_observed: true,
          relevance_score: anchored * 5 + explicit * 2 +
            (observedSourceHost(url) === observedSourceHost(fetch.url) ? 2 : 0)
        });
      }
      candidates.push(...sourceCandidates
        .sort((left, right) => right.relevance_score - left.relevance_score)
        .slice(0, 3));
    }
    const seen = new Set();
    return candidates
      .sort((left, right) => right.relevance_score - left.relevance_score)
      .filter((item) => {
        const key = canonicalObservedSourceKey(item.url);
        if (!key || seen.has(key) || excludedSourceKeys.has(key)) {
          return false;
        }
        seen.add(key);
        return true;
      })
      .slice(0, directWebFetchLimit * 2)
      .map(({ relevance_score: _score, ...item }) => item);
  };
  const sourceFromSearchResult = (item, fetches) => {
    const fetched = fetchedTextForUrl(fetches, item.url);
    if (!fetched) {
      return null;
    }
    const quote = sanitizeEvidenceText(
      evidenceSnippet(
        fetched,
        "",
        1000,
        pageEvidenceFocusTerms(item, fetched)
      )
    );
    const safeUrl = normalizeObservedSource(item.url);
    const title = sanitizeEvidenceText(compactText(
      fetchedPageTitle(fetched, safeUrl) ||
        (item.planned_seed === true
          ? sourceTitleFromUrl(safeUrl)
          : (item.title || sourceTitleFromUrl(safeUrl))),
      220
    ));
    if (!quote) {
      return null;
    }
    const provenance = item.engines.length > 0 ? ` via ${item.engines.join(", ")}` : "";
    const authority = /^https?:\/\/(?:www\.)?github\.com\//i.test(safeUrl)
      ? "Repository source; project authority must be established from its ownership and content"
      : (isPrimarySourceUrl(safeUrl)
        ? "Primary or authoritative source"
        : "Web source; publisher relationship and claims require independent corroboration");
    return {
      title: title || item.url,
      url_or_path: safeUrl,
      quote_or_fact: quote,
      date: normalizedSourceDate(item.date) || undefined,
      reliability: `${authority}; page text fetched${provenance}.`
    };
  };
  const directWebResearchFromSources = (searches, fetches, collectionErrors) => {
    const results = queryAwareFetchCandidates(searches, directWebMaxResults);
    const substantiveQueryTerms = queryTerms();
    const matchedQueryTerms = substantiveQueryTerms.filter((term) =>
      results.some((item) => {
        const fetched = fetchedTextForUrl(fetches, item.url);
        return queryTermMatches(
          `${item.title || ""} ${item.url || ""} ${item.content || ""} ${fetched}`,
          term
        );
      })
    );
    const matchedFetchedQueryTerms = substantiveQueryTerms.filter((term) =>
      fetches.some((item) =>
        item.ok &&
        isNonEmptyString(item.output) &&
        queryTermMatches(fetchedTextForQueryMatching(item.output), term)
      )
    );
    const safeCollectionErrors = uniqueStrings(
      collectionErrors.map(sanitizeEvidenceText).filter(Boolean)
    );
    const resultCandidateLeads = results.slice(0, directWebMaxResults).map((item) => ({
      title: sanitizeEvidenceText(compactText(item.title || sourceTitleFromUrl(item.url), 180)),
      url: normalizeObservedSource(item.url),
      queries: uniqueStrings(Array.isArray(item.evidence_queries) ? item.evidence_queries : [])
        .slice(0, 2)
        .map((value) => compactText(value, 180))
    })).filter((item) => item.url);
    const candidateLeads = [];
    const candidateKeys = new Set();
    for (const lead of [
      ...resultCandidateLeads,
      ...fetchedPageCandidateLeads(results, fetches)
    ]) {
      const key = canonicalObservedSourceKey(lead.url);
      if (!key || candidateKeys.has(key)) {
        continue;
      }
      candidateKeys.add(key);
      candidateLeads.push(lead);
      if (candidateLeads.length >= directWebMaxResults + directWebFetchLimit) {
        break;
      }
    }
    const sources = results
      .filter((item) => isNonEmptyString(fetchedTextForUrl(fetches, item.url)))
      .map((item) => sourceFromSearchResult(item, fetches))
      .filter((source) => isEvidenceSource(source));
    const fetchedCount = sources.length;
    const fetchedHostCount = new Set(
      fetches
        .filter((item) => item.ok)
        .map((item) => observedSourceHost(item.url))
        .filter(Boolean)
    ).size;
    const freshnessRequired = Boolean(researchPlan && researchPlan.freshness_required === true);
    const datedSourceCount = sources.filter((source) => isNonEmptyString(source.date)).length;
    const hostCount = new Set(
      sources.map((source) => observedSourceHost(source.url_or_path)).filter(Boolean)
    ).size;
    const metadata = {
      engineered_loop: engineeredLoopEnabled,
      search_count: searches.filter((item) => item.planned_seed_group !== true).length,
      result_count: results.length,
      source_count: sources.length,
      host_count: hostCount,
      freshness_required: freshnessRequired,
      dated_source_count: datedSourceCount,
      query_term_count: substantiveQueryTerms.length,
      matched_query_term_count: matchedQueryTerms.length,
      query_term_coverage: substantiveQueryTerms.length > 0
        ? matchedQueryTerms.length / substantiveQueryTerms.length
        : 0,
      fetched_query_term_count: matchedFetchedQueryTerms.length,
      fetched_query_term_coverage: substantiveQueryTerms.length > 0
        ? matchedFetchedQueryTerms.length / substantiveQueryTerms.length
        : 0,
      query_terms_truncated: queryTermAnalysis().truncated,
      fetch_count: fetches.length,
      fetched_count: fetchedCount,
      fetched_host_count: fetchedHostCount,
      task_count: 1,
      success_count: sources.length > 0 ? 1 : 0,
      failed_count: sources.length > 0 ? 0 : 1,
      all_success: sources.length > 0 && safeCollectionErrors.length === 0,
      partial_failure: sources.length > 0 && safeCollectionErrors.length > 0,
      candidate_urls: candidateLeads.map((item) => item.url),
      candidate_leads: candidateLeads,
      results: []
    };
    if (sources.length === 0) {
      return {
        tool: "web_search/web_fetch",
        algorithm: "direct_web_search_fetch",
        status: "failed",
        metadata,
        results: [],
        warnings: {
          collection_errors: safeCollectionErrors.slice(0, 10),
          note: "Direct collection found no traceable sources."
        }
      };
    }
    const structured = {
      summary: `Direct collection retained ${sources.length} source(s) with relevant fetched page text.`,
      sources,
      key_evidence: sources.slice(0, 8).map((source) => `${source.title}: ${source.quote_or_fact}`),
      contradictions: [],
      confidence: fetchedCount > 0
        ? "medium-high: source pages were fetched."
        : "medium: search snippets only.",
      gaps: uniqueStrings([
        fetchedCount === 0 ? "No relevant substantive page text was retained." : "",
        candidateLeads.length > sources.length
          ? `${candidateLeads.length - sources.length} search result(s) remain discovery leads, not evidence.`
          : "",
        safeCollectionErrors.length > 0 ? `Collection errors: ${safeCollectionErrors.slice(0, 3).join("; ")}` : ""
      ]).filter(Boolean)
    };
    const result = {
      task_id: "direct_web_research",
      agent: "workflow",
      success: true,
