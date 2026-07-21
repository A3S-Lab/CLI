async function run(ctx, inputs) {
  const MAX_SOURCES = 8;
  const MAX_CATALOG_SOURCES = 16;
  const MAX_DISCOVERY_CANDIDATES = 35;
  // Eight ordinary fetched sources can each approach 40 semantic chunks.
  // Keep the complete catalog closed and fail rather than sample it, while
  // leaving bounded headroom for a full eight-source public portfolio.
  const MAX_CHUNKS = 384;
  const MAX_CHUNK_CHARS = 700;
  // Small catalogs use one direct selector. Larger catalogs are split only on
  // source identity: one model call sees the complete fetched text for one
  // source, so it can choose the best excerpts without a lossy map/reduce
  // cascade or arbitrating unrelated source contracts.
  const MAX_DIRECT_SELECTOR_CHUNKS = 10;
  const MAX_SELECTOR_SHARD_CANDIDATES = 4;
  const MAX_EXCERPTS_PER_SOURCE = 4;
  const MAX_EXCERPT_CHARS_PER_SOURCE = 2800;
  const MAX_DOCUMENT_RANGES = 3;
  const MAX_LOCAL_SOURCES = 8;
  const MAX_LOCAL_RANGES = 3;
  const MAX_LOCAL_RANGE_LINES = 240;
  // Candidate admission and small-catalog selection have repeatedly exhausted
  // 210 seconds on the same ordinary eight-slot web portfolio. Give those
  // cross-source decisions the same five-minute active window used by the
  // closed question review; model-admission queue wait remains separate, while
  // the 25-minute whole-stage deadline still bounds retries, queued units, and
  // replacement work across the full portfolio.
  const MODEL_GENERATION_ACTIVE_TIMEOUT_MS = 300_000;
  // A real primary-source selector exceeded 210 seconds. Keep exactly one
  // attempt so a slow source cannot starve later siblings, but allow the same
  // 270-second long-tail bound used by other closed-evidence generations.
  const MODEL_GENERATION_SHARD_ACTIVE_TIMEOUT_MS = 270_000;
  const STEP_DISCOVER_WEB = "discover_web_sources";
  const STEP_SELECT_WEB = "select_web_sources";
  const STEP_WEB = "retrieve_web";
  const STEP_LOCAL = "retrieve_local";
  const STEP_SELECT = "select_evidence_chunks";
  const STEP_SELECT_SHARD_PREFIX = "select_evidence_chunks_shard_";
  const STEP_SELECT_SOURCE_PREFIX = "select_evidence_chunks_source_";
  const STEP_CHECKPOINT_BOOTSTRAP = "checkpoint_bootstrap_acquisition";
  const STEP_CHECKPOINT_INITIAL = "checkpoint_initial_retrieval";
  const STEP_SELECT_SUPPLEMENTAL_WEB = "select_supplemental_web_sources";
  const STEP_SUPPLEMENTAL_WEB = "retrieve_supplemental_web";
  const STEP_SELECT_SUPPLEMENTAL = "select_supplemental_evidence_chunks";
  const STEP_SELECT_SUPPLEMENTAL_SHARD_PREFIX =
    "select_supplemental_evidence_chunks_shard_";
  const STEP_SELECT_SUPPLEMENTAL_SOURCE_PREFIX =
    "select_supplemental_evidence_chunks_source_";

  const object = (value) =>
    value && typeof value === "object" && !Array.isArray(value) ? value : {};
  const nonEmpty = (value) => typeof value === "string" && value.trim().length > 0;
  const clamp = (value, minimum, maximum, fallback) => {
    const number = Number(value);
    return Number.isFinite(number)
      ? Math.max(minimum, Math.min(maximum, Math.floor(number)))
      : fallback;
  };
  const bounded = (value, maximum) => {
    const compact = String(value || "").replace(/\s+/g, " ").trim();
    const characters = Array.from(compact);
    return characters.length <= maximum
      ? compact
      : `${characters.slice(0, Math.max(0, maximum - 1)).join("")}…`;
  };
  const uniqueStrings = (values) => {
    const seen = new Set();
    const result = [];
    for (const value of values || []) {
      const text = typeof value === "string" ? value.trim() : "";
      if (!text || seen.has(text)) {
        continue;
      }
      seen.add(text);
      result.push(text);
    }
    return result;
  };
  const errorText = (error) =>
    bounded(error && error.message ? error.message : error, 600) ||
    "the tool returned no diagnostic";
  const toolExitCode = (result) => Number(
    result && (result.exitCode ?? result.exit_code)
  ) || 0;

  const cleanUrl = (value) => {
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
    if (
      !/^https?:\/\/[^/\s]+(?:\/[^\s]*)?$/i.test(url) ||
      /[\u2026\uFFFD{}<>]/u.test(url)
    ) {
      return "";
    }
    return url;
  };
  const urlParts = (value) => {
    const url = cleanUrl(value);
    const match = url.match(/^(https?):\/\/([^/?#]+)([^#]*)/i);
    if (!match) {
      return null;
    }
    const scheme = match[1].toLowerCase();
    let authority = match[2].split("@").pop().toLowerCase();
    if (
      (scheme === "https" && authority.endsWith(":443")) ||
      (scheme === "http" && authority.endsWith(":80"))
    ) {
      authority = authority.replace(/:\d+$/, "");
    }
    return {
      url,
      scheme,
      authority,
      suffix: match[3] || "/",
    };
  };
  const canonicalUrl = (value) => {
    const parsed = urlParts(value);
    if (!parsed) {
      return "";
    }
    const suffix = parsed.suffix || "/";
    return `${parsed.scheme}://${parsed.authority}${suffix}`.replace(/\/+$/, "");
  };
  const urlHost = (value) => {
    const parsed = urlParts(value);
    if (!parsed) {
      return "";
    }
    if (parsed.authority.startsWith("[")) {
      const end = parsed.authority.indexOf("]");
      return end >= 0 ? parsed.authority.slice(0, end + 1) : "";
    }
    return parsed.authority.replace(/:\d+$/, "");
  };
  const lowValueUrl = (value) => {
    const url = String(value || "").toLowerCase();
    return /\.(?:7z|aac|apk|avi|avif|bin|bmp|bz2|deb|dmg|eot|exe|flac|gif|gz|ico|iso|jpe?g|m4a|mov|mp3|mp4|mpeg|msi|ogg|opus|otf|pkg|png|rar|rpm|svg|tar|tiff?|tgz|ttf|wasm|wav|webm|webp|woff2?|xz|zip)(?:[?#].*)?$/i.test(url) ||
      /^https?:\/\/avatars\.githubusercontent\.com(?:\/|$)/.test(url) ||
      /^https?:\/\/(?:[^/]+\.)?gravatar\.com(?:\/|$)/.test(url);
  };
  const fetchUrl = (value) => {
    const url = cleanUrl(value);
    let match = url.match(
      /^https:\/\/github\.com\/([^/?#]+)\/([^/?#]+)\/blob\/([^/?#]+)\/(.+?)(?:[?#].*)?$/i
    );
    if (match) {
      return `https://raw.githubusercontent.com/${match[1]}/${match[2]}/${match[3]}/${match[4]}`;
    }
    match = url.match(
      /^https:\/\/github\.com\/([^/?#]+)\/([^/?#]+)\/releases\/?(?:[?#].*)?$/i
    );
    if (match) {
      return `https://github.com/${match[1]}/${match[2]}/releases.atom`;
    }
    match = url.match(
      /^https:\/\/github\.com\/([^/?#]+)\/([^/?#]+)\/?(?:[?#].*)?$/i
    );
    if (match) {
      return `https://raw.githubusercontent.com/${match[1]}/${match[2]}/HEAD/README.md`;
    }
    match = url.match(
      /^https:\/\/(?:www\.|export\.)?arxiv\.org\/abs\/((?:[a-z-]+(?:\.[a-z]{2})?\/\d{7}|\d{4}\.\d{4,5})(?:v\d+)?)(?:[?#].*)?$/i
    );
    return match ? `https://arxiv.org/pdf/${match[1]}` : url;
  };

  const batchSections = (output, expectedCount) => {
    const text = String(output || "");
    const sections = Array.from({ length: expectedCount }, () => "");
    const markers = [];
    const pattern = /^--- \[(\d+):[^\n]*\] ---\r?\n/gm;
    let match = null;
    while ((match = pattern.exec(text)) !== null) {
      markers.push({
        index: Number(match[1]) - 1,
        header: match.index,
        body: pattern.lastIndex,
      });
    }
    for (let position = 0; position < markers.length; position += 1) {
      const marker = markers[position];
      if (
        !Number.isInteger(marker.index) ||
        marker.index < 0 ||
        marker.index >= expectedCount
      ) {
        continue;
      }
      const end = position + 1 < markers.length
        ? markers[position + 1].header
        : text.length;
      sections[marker.index] = text
        .slice(marker.body, end)
        .replace(/\nBatch completed with[\s\S]*$/, "")
        .replace(/^ERROR:\s*/, "")
        .trim();
    }
    return sections;
  };
  const batchChild = (batch, sections, index) => {
    const results = batch && batch.metadata && Array.isArray(batch.metadata.results)
      ? batch.metadata.results
      : [];
    const metadata = results.find((item) => Number(item && item.index) === index);
    const output = sections[index] || "";
    const outputBytes = Number(metadata && metadata.output_bytes);
    const outputTruncated = Boolean(
      metadata &&
      metadata.success === true &&
      Number.isSafeInteger(outputBytes) &&
      outputBytes > 0 &&
      (!nonEmpty(output) || /\[tool output truncated:/i.test(output))
    );
    return {
      success: metadata
        ? metadata.success === true
        : Boolean(batch && toolExitCode(batch) === 0 && nonEmpty(output)),
      output,
      metadata: object(metadata && metadata.metadata),
      error_kind: metadata && metadata.error_kind,
      output_truncated: outputTruncated,
    };
  };
  const invokeBatch = async (invocations, maximumConcurrency) => {
    if (invocations.length === 0) {
      return { batch: null, children: [] };
    }
    const batch = await ctx.tool("batch", {
      invocations,
      max_concurrency: Math.max(
        1,
        Math.min(maximumConcurrency, invocations.length)
      ),
    });
    const sections = batchSections(batch && batch.output, invocations.length);
    return {
      batch,
      children: invocations.map((_invocation, index) =>
        batchChild(batch, sections, index)
      ),
    };
  };
  const invokeBatchWithOutputRecovery = async (
    invocations,
    maximumConcurrency
  ) => {
    const initial = await invokeBatch(invocations, maximumConcurrency);
    const children = initial.children.slice();
    const recoveryErrors = [];
    let recoveryCount = 0;
    for (let index = 0; index < children.length; index += 1) {
      if (!children[index] || children[index].output_truncated !== true) {
        continue;
      }
      try {
        const recovered = await invokeBatch([invocations[index]], 1);
        children[index] = recovered.children[0] || children[index];
        recoveryCount += 1;
      } catch (error) {
        recoveryErrors.push(
          `Batch output recovery ${index + 1} failed: ${errorText(error)}`
        );
      }
    }
    return {
      batch: initial.batch,
      children,
      output_recovery_count: recoveryCount,
      output_recovery_errors: recoveryErrors,
    };
  };

  const parseSearchResults = (output) => {
    const text = String(output || "").trim();
    if (!text) {
      return [];
    }
    try {
      const parsed = JSON.parse(text);
      const values = Array.isArray(parsed)
        ? parsed
        : (parsed && Array.isArray(parsed.results) ? parsed.results : []);
      return values
        .filter((item) => item && typeof item === "object")
        .map((item) => ({
          title: bounded(item.title || "", 220),
          url: cleanUrl(item.url || item.url_or_path),
          date: bounded(
            item.published_date || item.publication_date || item.date || "",
            100
          ),
          content: bounded(item.content || item.snippet || "", 600),
          engines: uniqueStrings(Array.isArray(item.engines) ? item.engines : [])
            .slice(0, 4),
        }))
        .filter((item) => item.url && !lowValueUrl(item.url));
    } catch (_error) {
      return uniqueStrings(text.match(/https?:\/\/[^\s<>"']+/g) || [])
        .map(cleanUrl)
        .filter((url) => url && !lowValueUrl(url))
        .map((url) => ({ title: "", url, date: "", engines: [] }));
    }
  };
  const documentRange = (metadata) => {
    const range = object(metadata && metadata.range);
    const offset = Number(range.offset);
    const nextOffset = range.next_offset === null || range.next_offset === undefined
      ? null
      : Number(range.next_offset);
    return {
      offset: Number.isSafeInteger(offset) && offset >= 0 ? offset : null,
      next_offset: Number.isSafeInteger(nextOffset) && nextOffset >= 0
        ? nextOffset
        : null,
      eof: range.eof === true,
    };
  };
  const extractedDocument = (metadata) => {
    const kind = String(metadata && metadata.document_kind || "").toLowerCase();
    const contentType = String(metadata && metadata.content_type || "").toLowerCase();
    return kind === "pdf" || kind === "document" ||
      /^application\/pdf(?:;|$)/.test(contentType);
  };
  const cleanFetchedText = (value, document) => {
    let text = String(value || "").replace(/\r\n?/g, "\n");
    if (document) {
      text = text
        .replace(/\n*\.\.\. \(more fetched content available; continue with offset=\d+\)\s*/gi, "\n")
        .replace(/([A-Za-z])-\s*\n\s*(?=[a-z])/g, "$1");
    }
    return text.trim();
  };
  const substantive = (value) => {
    const visible = String(value || "")
      .replace(/<script[\s\S]*?<\/script>/gi, " ")
      .replace(/<style[\s\S]*?<\/style>/gi, " ")
      .replace(/<[^>]+>/g, " ")
      .replace(/\s+/g, " ")
      .trim();
    return Array.from(visible).length >= 30;
  };
  const transientFetchFailure = (child) => {
    const errorKind = child && child.error_kind;
    const kind = typeof errorKind === "string"
      ? errorKind
      : String(errorKind && errorKind.type || "");
    return Boolean(
      child &&
      ["network", "timeout", "transport"].includes(
        kind.toLowerCase()
      )
    );
  };

  const evidenceLines = (value) => String(value || "")
    .split(/\n+/)
    .map((line) => line.replace(/\s+/g, " ").trim())
    .filter((line) => {
      if (Array.from(line).length < 12) {
        return false;
      }
      return !/<script|<\/script|document\.cookie|__next_data__|webpack|data-color-mode|--color-[a-z-]+\s*:/i.test(line) &&
        !/"@context"\s*:\s*"https?:\\?\/\\?\/schema\.org/i.test(line);
    });
  const splitLongText = (value) => {
    const characters = Array.from(String(value || ""));
    if (characters.length <= MAX_CHUNK_CHARS) {
      return [characters.join("")];
    }
    const chunks = [];
    let offset = 0;
    while (offset < characters.length) {
      const maximumEnd = Math.min(characters.length, offset + MAX_CHUNK_CHARS);
      let end = maximumEnd;
      if (maximumEnd < characters.length) {
        const minimumEnd = offset + Math.floor(MAX_CHUNK_CHARS * 0.65);
        for (let cursor = maximumEnd - 1; cursor >= minimumEnd; cursor -= 1) {
          if (/[\s.,;:!?…。，；：！？]/u.test(characters[cursor])) {
            end = cursor + 1;
            break;
          }
        }
      }
      const chunk = characters.slice(offset, end).join("").trim();
      if (chunk) {
        chunks.push(chunk);
      }
      if (end >= characters.length) {
        break;
      }
      offset = Math.max(offset + 1, end - 50);
    }
    return chunks;
  };
  const sourceChunks = (values, sourceId) => {
    const segments = Array.isArray(values) ? values : [values];
    const chunks = [];
    let pending = "";
    const retain = () => {
      const text = bounded(pending, MAX_CHUNK_CHARS);
      if (text) {
        chunks.push({
          chunk_id: `${sourceId}:chunk:${chunks.length + 1}`,
          text,
        });
      }
      pending = "";
    };
    for (const segment of segments) {
      const units = evidenceLines(segment).flatMap(splitLongText);
      for (const unit of units) {
        const candidate = pending ? `${pending} ${unit}` : unit;
        if (Array.from(candidate).length > MAX_CHUNK_CHARS) {
          retain();
          pending = unit;
        } else {
          pending = candidate;
        }
      }
      // A provider range is a structural retrieval boundary. Keep it as a
      // distinct semantic-selection unit without turning it into another pass.
      retain();
    }
    return chunks;
  };
  const atomFeedSegments = (values, fetchedUrl) => {
    const segments = Array.isArray(values) ? values : [values];
    if (!/\/releases\.atom(?:[?#].*)?$/i.test(String(fetchedUrl || ""))) {
      return segments;
    }
    const text = segments.map((value) => String(value || "")).join("\n");
    const starts = [];
    const entryPattern = /<entry(?:\s|>)/gi;
    let match = null;
    while ((match = entryPattern.exec(text)) !== null) {
      starts.push(match.index);
    }
    if (starts.length === 0) {
      return segments;
    }
    const boundedSegments = [];
    const header = text.slice(0, starts[0]).trim();
    if (header) {
      boundedSegments.push(header);
    }
    for (let index = 0; index < starts.length; index += 1) {
      const end = index + 1 < starts.length ? starts[index + 1] : text.length;
      const entry = text.slice(starts[index], end).trim();
      if (entry) {
        boundedSegments.push(entry);
      }
    }
    return boundedSegments;
  };
  const planFocuses = (plan) => {
    const tracks = Array.isArray(plan.tracks) ? plan.tracks : [];
    return tracks.slice(0, 4).map((track, index) => {
      const item = object(track);
      const questions = Array.isArray(item.questions)
        ? item.questions.filter(nonEmpty)
        : [];
      const completionCriteria = Array.isArray(item.completion_criteria)
        ? item.completion_criteria.filter(nonEmpty).slice(0, 2)
        : [];
      const evidenceRequirements = object(item.evidence_requirements);
      return {
        focus_index: index,
        obligation_id: bounded(item.id, 64),
        material: item.material === true,
        completion_criteria: completionCriteria.map((criterion) =>
          bounded(criterion, 400)
        ),
        evidence_requirements: {
          primary_source_required:
            evidenceRequirements.primary_source_required === true,
          independent_corroboration_required:
            evidenceRequirements.independent_corroboration_required === true,
        },
        focus: bounded(
          uniqueStrings([item.title, item.focus, ...questions]).join(": "),
          900
        ),
      };
    }).filter((item) => item.obligation_id && item.focus);
  };
  const webEvidencePacket = (plan, fetched, sourcePrefix) => {
    const focuses = planFocuses(plan);
    const prefix = nonEmpty(sourcePrefix) ? sourcePrefix : "web-source";
    const candidates = fetched
      .filter((item) => item.ok && substantive(item.text))
      .slice(0, MAX_SOURCES)
      .map((item, index) => {
        const sourceId = `${prefix}-${index + 1}`;
        const chunks = sourceChunks(
          atomFeedSegments(item.segments || [item.text], item.fetch_url),
          sourceId
        );
        if (chunks.length === 0) {
          return null;
        }
        return {
          source_id: sourceId,
          title: item.title || urlHost(item.url) || "Fetched source",
          url_or_path: item.url,
          // Provider dates are discovery metadata and may describe an index,
          // crawl, or documentation build rather than publication. Only the
          // fetched text may establish a date in closed evidence.
          reliability: `Fetched source text${item.engines.length > 0
            ? ` discovered via ${item.engines.join(", ")}`
            : ""}; authority and claim fit require closed-evidence review.`,
          chunks,
        };
      })
      .filter(Boolean);
    const chunkCount = candidates.reduce(
      (total, source) => total + source.chunks.length,
      0
    );
    if (focuses.length === 0 || candidates.length === 0) {
      return {
        packet: null,
        chunk_count: chunkCount,
        error: "",
      };
    }
    if (chunkCount > MAX_CHUNKS) {
      return {
        packet: null,
        chunk_count: chunkCount,
        error: `Fetched evidence produced ${chunkCount} chunks, exceeding the closed catalog limit of ${MAX_CHUNKS}; no fetched text was promoted.`,
      };
    }
    return {
      packet: {
        version: 1,
        focuses,
        sources: candidates,
      },
      chunk_count: chunkCount,
      error: "",
    };
  };
  const combinedEvidencePacket = (plan, retrievals) => {
    const focuses = planFocuses(plan);
    const sources = retrievals
      .filter((retrieval) => retrieval && retrieval.packet)
      .flatMap((retrieval) => retrieval.packet.sources || []);
    const chunkCount = sources.reduce(
      (total, source) =>
        total + (Array.isArray(source.chunks) ? source.chunks.length : 0),
      0
    );
    if (focuses.length === 0 || sources.length === 0) {
      return {
        packet: null,
        source_count: sources.length,
        chunk_count: chunkCount,
        error: "",
      };
    }
    if (sources.length > MAX_CATALOG_SOURCES || chunkCount > MAX_CHUNKS) {
      return {
        packet: null,
        source_count: sources.length,
        chunk_count: chunkCount,
        error: `Retrieved evidence produced ${sources.length} sources and ${chunkCount} chunks, exceeding the complete closed catalog limit of ${MAX_CATALOG_SOURCES} sources and ${MAX_CHUNKS} chunks; no retrieved text was promoted.`,
      };
    }
    return {
      packet: {
        version: 1,
        focuses,
        sources,
      },
      source_count: sources.length,
      chunk_count: chunkCount,
      error: "",
    };
  };
