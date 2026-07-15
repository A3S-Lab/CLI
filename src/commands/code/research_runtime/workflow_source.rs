//! Built-in DeepResearch dynamic-workflow source.

pub(crate) fn deep_research_workflow_source() -> &'static str {
    r#"
async function run(ctx, inputs) {
  const input = inputs.input || {};
  const query = input.query || "";
  const osRuntimeEnabled = false;
  const fallbackTracks = [
    { title: "Facts and timeline", focus: "establish the current facts, dates, and key actors" },
    { title: "Primary sources", focus: "find official or primary-source evidence" },
    { title: "Independent analysis", focus: "compare reputable independent analysis and disagreements" },
    { title: "Quantitative evidence", focus: "find concrete numbers, benchmarks, market data, or measurable claims when applicable" },
    { title: "Contradictions", focus: "look for reputable disagreement, minority views, and claims that conflict with the emerging answer" },
    { title: "Risks and caveats", focus: "identify uncertainty, recency caveats, and weak claims" },
  ];
  const complexityMarkers = [
    ["comprehensive", "deep dive", "research", "全面", "深入", "调研", "研究"],
    ["compare", "comparison", "versus", "benchmark", "对比", "比较", "竞品"],
    ["latest", "recent", "timeline", "2026", "2025", "最新", "趋势", "时间线"],
    ["market", "regulation", "policy", "paper", "papers", "市场", "法规", "政策", "论文"],
    ["multi-source", "multi source", "many sources", "多来源", "大量", "并行"]
  ];
  const queryComplexity = () => {
    const q = String(query || "").toLowerCase();
    let score = 0;
    for (const group of complexityMarkers) {
      if (group.some((marker) => q.includes(marker))) {
        score += 1;
      }
    }
    const wordCount = q.split(/\s+/).filter(Boolean).length;
    const charCount = Array.from(q).length;
    if (wordCount >= 14 || charCount >= 80) {
      score += 1;
    }
    if (wordCount >= 28 || charCount >= 140) {
      score += 1;
    }
    const narrowOfficialLookup =
      (q.includes("latest") || q.includes("current") || q.includes("最新")) &&
      (q.includes("version") || q.includes("release") || q.includes("版本")) &&
      (q.includes("official") || q.includes("primary") || q.includes("官方")) &&
      !["compare", "comparison", "versus", "benchmark", "market", "regulation", "policy", "paper", "papers", "对比", "比较", "市场", "法规", "政策", "论文"].some((marker) => q.includes(marker));
    if (narrowOfficialLookup && score <= 2) {
      return 0;
    }
    return score;
  };
  const complexityScore = queryComplexity();
  const requestedLocalParallelTasks = Number(input.local_max_parallel_tasks);
  const maxLocalParallelTasks = Number.isFinite(requestedLocalParallelTasks) && requestedLocalParallelTasks > 0
    ? Math.max(1, Math.min(64, Math.floor(requestedLocalParallelTasks)))
    : fallbackTracks.length;
  const requestedResearchRounds = Number(input.local_research_rounds);
  const derivedResearchRounds = complexityScore <= 1
    ? 1
    : (complexityScore <= 3 ? 2 : (complexityScore <= 5 ? 3 : 4));
  const maxResearchRounds = Number.isFinite(requestedResearchRounds) && requestedResearchRounds > 0
    ? Math.max(1, Math.min(4, Math.floor(requestedResearchRounds)))
    : derivedResearchRounds;
  const minResearchRounds = maxResearchRounds > 1 ? 2 : 1;
  const initialFallbackTrackCount = Math.min(
    fallbackTracks.length,
    maxLocalParallelTasks,
    complexityScore <= 1 ? 1 : (complexityScore <= 3 ? 3 : (complexityScore <= 5 ? 4 : 6))
  );
  const requestedLocalMaxSteps = Number(input.local_max_steps);
  const localMaxSteps = Number.isFinite(requestedLocalMaxSteps) && requestedLocalMaxSteps > 0
    ? Math.floor(requestedLocalMaxSteps)
    : 200;
  const continueWorkflowRetry = {
    max_attempts: 1,
    delay_ms: 0,
    on_exhausted: "continue_workflow",
  };
  const providedTracks = Array.isArray(input.tracks)
    ? input.tracks.filter((track) => {
        if (track === null || track === undefined) {
          return false;
        }
        if (track && typeof track === "object" && !Array.isArray(track) && track.parallelizable === false) {
          return false;
        }
        return true;
      })
    : [];
  const tracks = (providedTracks.length > 0 ? providedTracks : fallbackTracks)
    .slice(0, providedTracks.length > 0 ? maxLocalParallelTasks : initialFallbackTrackCount);
  const evidenceSchema = {
    type: "object",
    additionalProperties: false,
    properties: {
      summary: { type: "string" },
      sources: {
        type: "array",
        items: {
          type: "object",
          additionalProperties: false,
          properties: {
            title: { type: "string" },
            url_or_path: { type: "string" },
            date: { type: "string" },
            quote_or_fact: { type: "string" },
            reliability: { type: "string" }
          },
          required: ["title", "url_or_path", "quote_or_fact"]
        }
      },
      key_evidence: { type: "array", items: { type: "string" } },
      contradictions: { type: "array", items: { type: "string" } },
      confidence: { type: "string" },
      gaps: { type: "array", items: { type: "string" } }
    },
    required: ["summary", "sources", "key_evidence", "contradictions", "confidence", "gaps"]
  };
  const localTasks = (roundNumber, roundTracks, previousEvidence) => roundTracks.map((track, index) => {
    const title = track.title || `Track ${index + 1}`;
    const focus = track.focus || String(track);
    const roundContext = roundNumber > 1
      ? `\n\nRecursive round: ${roundNumber}/${maxResearchRounds}. Use the prior evidence summary below to resolve only the remaining gaps or contradictions. Do not repeat the same query, source, path, or fetch from earlier rounds unless you are checking a changed/stronger source.\n\nPrior evidence summary:\n${previousEvidence || "No prior evidence summary available."}`
      : "";
    return {
      agent: "explore",
      description: `Research round ${roundNumber}.${index + 1}: ${title}`,
      max_steps: localMaxSteps,
      output_schema: evidenceSchema,
      prompt: `Deep-research evidence track for: ${query}\n\nFocus: ${focus}${roundContext}\n\nEvidence only: do not write files, create report artifacts, run tests, or modify the workspace. You are an evidence collector, not a verification runner. Use dedicated read-only tools: web_search/web_fetch for current external evidence, read/grep/glob/ls for local workspace evidence. Do not use bash for research collection. Do not inspect .a3s-flow/dynamic-workflows logs unless the focus explicitly asks about DeepResearch/runtime diagnostics; workflow logs are host diagnostics, not research evidence. If the query is about public/current facts, use web_search first and fetch the strongest sources; do not fall back to local repository grep just because a web query is empty. If web tools are unavailable or return no useful results after several distinct queries, report that gap with the attempted queries and stop instead of looping. If the query explicitly asks for local workspace evidence only, inspect local files and do not use web tools. Use as many distinct high-signal tool calls as the evidence actually requires, avoid repeating the same query/pattern/path, and synthesize when evidence is sufficient. Return concise evidence, URLs or local paths, publication dates when available, key evidence, contradictions, and confidence notes. Your final child response should contain enough information to satisfy the provided output_schema: summary, sources, key_evidence, contradictions, confidence, and gaps.`
    };
  });
  const isNonEmptyString = (value) => typeof value === "string" && value.trim().length > 0;
  const isStringArray = (value, allowEmpty) =>
    Array.isArray(value) &&
    (allowEmpty || value.length > 0) &&
    value.every((item) => typeof item === "string");
  const isEvidenceSource = (value) =>
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    isNonEmptyString(value.title) &&
    isNonEmptyString(value.url_or_path) &&
    isNonEmptyString(value.quote_or_fact) &&
    (value.date === undefined || typeof value.date === "string") &&
    (value.reliability === undefined || typeof value.reliability === "string");
  const isEvidenceObject = (value) =>
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    isNonEmptyString(value.summary) &&
    Array.isArray(value.sources) &&
    value.sources.length > 0 &&
    value.sources.every(isEvidenceSource) &&
    isStringArray(value.key_evidence, false) &&
    isStringArray(value.contradictions, true) &&
    isNonEmptyString(value.confidence) &&
    isStringArray(value.gaps, true);
  const normalizeRuntimeOutput = (runtimeOutput) => {
    if (!runtimeOutput || !Array.isArray(runtimeOutput.results)) {
      return runtimeOutput;
    }
    const normalized = Object.assign({}, runtimeOutput);
    normalized.results = runtimeOutput.results.map((item) => {
      if (!item || typeof item !== "object") {
        return item;
      }
      const output = item.output;
      let structured = item.structured || null;
      if (!structured && typeof output === "string") {
        try {
          structured = JSON.parse(output);
        } catch (_err) {
          structured = null;
        }
      } else if (!structured && output && typeof output === "object") {
        structured = output;
      }
      if (!structured) {
        return item;
      }
      if (!isEvidenceObject(structured)) {
        const next = Object.assign({}, item);
        delete next.structured;
        next.structured_error = "runtime output did not match DeepResearch evidence schema";
        return next;
      }
      return Object.assign({}, item, { structured });
    });
    return normalized;
  };
  const compactText = (value, limit) => {
    let text = "";
    if (typeof value === "string") {
      text = value;
    } else if (value !== undefined && value !== null) {
      try {
        text = JSON.stringify(value);
      } catch (_err) {
        text = String(value);
      }
    }
    const compact = text.replace(/\s+/g, " ").trim();
    if (compact.length <= limit) {
      return compact;
    }
    return `${compact.slice(0, limit)} ... [truncated]`;
  };
  const trimRecoveredEvidenceText = (value) => {
    let text = typeof value === "string" ? value : compactText(value, 4000);
    const marker = text.indexOf("[structured output failed:");
    if (marker >= 0) {
      text = text.slice(0, marker);
    }
    return text
      .replace(/\r/g, "")
      .replace(/\n?\[structured output failed:[\s\S]*$/g, "")
      .trim();
  };
  const extractUrls = (text) => {
    const seen = new Set();
    const urls = [];
    const re = /https?:\/\/[^\s`<>"')\]}]+/g;
    for (const match of text.matchAll(re)) {
      const url = match[0].replace(/[.,;:]+$/g, "");
      const key = url.toLowerCase();
      if (!seen.has(key)) {
        seen.add(key);
        urls.push(url);
      }
      if (urls.length >= 10) {
        break;
      }
    }
    return urls;
  };
  const sentenceLines = (text) =>
    text
      .split(/\n+/)
      .map((line) => line.replace(/^#+\s*/, "").replace(/^\s*(?:[-*]|\d+[.)]|\|)+\s*/, "").trim())
      .filter((line) => {
        const lower = line.toLowerCase();
        return line &&
        !/^[-:| ]+$/.test(line) &&
        !/^#+\s*$/.test(line) &&
        !["summary", "sources", "key evidence", "confidence", "gaps", "contradictions"].includes(lower) &&
        !lower.startsWith("sources:");
      });
  const recoveredSummary = (text) => {
    const lines = sentenceLines(text);
    const preferred = lines.find((line) =>
      /summary|latest|current|stable|version|released|finding|结论|最新|版本/.test(line.toLowerCase())
    ) || lines[0] || "Source-backed research notes are available.";
    return compactText(preferred.replace(/^#+\s*/, ""), 700);
  };
  const recoveredKeyEvidence = (text, summary) => {
    const lines = sentenceLines(text);
    const picked = lines.filter((line) =>
      /latest|current|stable|version|released|official|source|confirm|confidence|cve|date|最新|版本|官方|发布/.test(line.toLowerCase()) ||
      /\b\d+\.\d+(?:\.\d+)?\b/.test(line) ||
      /\b20\d{2}-\d{2}-\d{2}\b/.test(line)
    );
    const evidence = uniqueStrings(picked.map((line) => compactText(line, 350))).slice(0, 10);
    return evidence.length > 0 ? evidence : [summary];
  };
  const sourceTitleFromUrl = (url) => {
    try {
      const parsed = new URL(url);
      return parsed.hostname.replace(/^www\./, "");
    } catch (_err) {
      return "Recovered source";
    }
  };
  const recoveredSources = (text, summary) =>
    extractUrls(text).map((url) => {
      const line = sentenceLines(text).find((candidate) => candidate.includes(url)) || summary;
      return {
        title: sourceTitleFromUrl(url),
        url_or_path: url,
        quote_or_fact: compactText(line.replace(url, "").trim() || summary, 350),
        reliability: "source-backed evidence retained from cited research notes"
      };
    });
  const recoverEvidenceObject = (text) => {
    const clean = trimRecoveredEvidenceText(text);
    if (!clean || clean.length < 40) {
      return null;
    }
    const sources = recoveredSources(clean, recoveredSummary(clean));
    if (sources.length === 0) {
      return null;
    }
    const summary = recoveredSummary(clean);
    return {
      summary,
      sources,
      key_evidence: recoveredKeyEvidence(clean, summary),
      contradictions: [],
      confidence: "medium-high; the cited sources agree",
      gaps: []
    };
  };
  const recoverEvidenceFromParallelFailure = (failure, roundNumber) => {
    const text = String((failure && failure.error) || "");
    if (!text.includes("Output:")) {
      return [];
    }
    const recovered = [];
    const taskRe = /--- Task\s+(\d+)\s+\(([^)]*)\)\s+\[[^\]]+\]\s+---\n([\s\S]*?)(?=\n--- Task\s+\d+\s+\(|$)/g;
    for (const match of text.matchAll(taskRe)) {
      const body = match[3] || "";
      const marker = body.indexOf("Output:\n");
      if (marker < 0) {
        continue;
      }
      const evidence = recoverEvidenceObject(body.slice(marker + "Output:\n".length));
      if (!evidence) {
        continue;
      }
      recovered.push({
        task_id: `recovered-round-${roundNumber}-task-${match[1]}`,
        agent: match[2] || "explore",
        success: true,
        structured: evidence
      });
    }
    return recovered;
  };
  const recoveredRoundFromFailures = (failures, roundNumber) => {
    const recovered = failures.flatMap((failure) =>
      recoverEvidenceFromParallelFailure(failure, roundNumber)
    );
    if (recovered.length === 0) {
      return null;
    }
    return normalizeLocalResearch({
      tool: "parallel_task",
      exit_code: 0,
      metadata: {
        task_count: recovered.length,
        result_count: recovered.length,
        success_count: recovered.length,
        failed_count: 0,
        allow_partial_failure: true,
        results: recovered
      },
      results: recovered
    });
  };
  const failureSummary = (value) => {
    const compact = compactText(value, 600);
    const lower = compact.toLowerCase();
    if (lower.includes("permission denied: tool")) {
      return "Delegated task could not use a requested tool because the permission policy denied it.";
    }
    if (lower.includes("max tool rounds")) {
      return "Delegated task exhausted its tool-round budget before returning usable evidence.";
    }
    if (lower.includes("timed out") || lower.includes("[command timed out")) {
      return "Delegated task timed out before returning usable evidence.";
    }
    if (
      lower.includes("[tool output truncated") ||
      lower.includes("full output artifact:") ||
      lower.includes("a3s://tool-output")
    ) {
      return "Delegated task produced oversized tool output that was withheld from the report context.";
    }
    if (
      lower.includes(".a3s-flow/dynamic-workflows") ||
      lower.includes("● searched") ||
      lower.includes("● ran") ||
      lower.includes("● read") ||
      compact.includes("⎿")
    ) {
      return "Delegated task returned internal workflow/tool logs that were withheld from the report context.";
    }
    return "Delegated task failed before returning usable evidence.";
  };
  const copyIfPresent = (target, source, keys) => {
    for (const key of keys) {
      if (source[key] !== undefined) {
        target[key] = source[key];
      }
    }
  };
  const compactLocalResult = (item, success) => {
    const next = {};
    copyIfPresent(next, item, [
      "task_id",
      "session_id",
      "agent",
      "success",
      "artifact_id",
      "artifact_uri",
      "output_bytes",
      "truncated_for_context",
      "structured_error"
    ]);
    if (success) {
      if (item.structured) {
        next.structured = item.structured;
      } else if (item.output !== undefined) {
        next.output_summary = compactText(item.output, 1200);
      }
    } else {
      next.error_summary = failureSummary(item.output || item.error || "task failed");
    }
    return next;
  };
  const normalizeLocalResearch = (parallelOutput) => {
    if (!parallelOutput || typeof parallelOutput !== "object" || Array.isArray(parallelOutput)) {
      return parallelOutput;
    }
    const metadata = parallelOutput.metadata && typeof parallelOutput.metadata === "object"
      ? parallelOutput.metadata
      : null;
    const results = metadata && Array.isArray(metadata.results) ? metadata.results : null;
    if (!results) {
      return parallelOutput;
    }
    const successfulResults = results.filter((item) => item && item.success === true);
    const failedResults = results.filter((item) => item && item.success === false);
    const compactSuccesses = successfulResults.map((item) => compactLocalResult(item, true));
    const countFromMetadata = (key, fallback) => {
      const value = Number(metadata[key]);
      return Number.isFinite(value) ? value : fallback;
    };
    const successCount = countFromMetadata("success_count", successfulResults.length);
    const failedCount = countFromMetadata("failed_count", failedResults.length);
    const resultCount = countFromMetadata("result_count", results.length);
    const taskCount = countFromMetadata("task_count", resultCount);
    const partialFailure = failedCount > 0 && successCount > 0;
    const normalized = {
      tool: parallelOutput.tool || "parallel_task",
      exit_code: parallelOutput.exit_code ?? parallelOutput.exitCode ?? (successCount > 0 ? 0 : 1),
      status: failedCount > 0 ? (successCount > 0 ? "partial_success" : "failed") : "success",
      metadata: {
        task_count: taskCount,
        result_count: resultCount,
        success_count: successCount,
        failed_count: failedCount,
        all_success: failedCount === 0,
        partial_failure: partialFailure,
        allow_partial_failure: metadata.allow_partial_failure === true,
        results: compactSuccesses
      },
      results: compactSuccesses
    };
    copyIfPresent(normalized, parallelOutput, ["artifact_id", "artifact_uri"]);
    if (failedResults.length > 0) {
      normalized.warnings = {
        failed_tasks: failedResults.map((item) => compactLocalResult(item, false))
      };
    }
    return normalized;
  };
  const roundStepId = (prefix, roundNumber) =>
    roundNumber === 1 ? prefix : `${prefix}_round_${roundNumber}`;
  const collectRoundOutputs = (stepOutputs, prefix) => {
    const rounds = [];
    for (let roundNumber = 1; roundNumber <= maxResearchRounds; roundNumber += 1) {
      const output = stepOutputs[roundStepId(prefix, roundNumber)];
      if (!output) {
        break;
      }
      rounds.push({ round: roundNumber, research: normalizeLocalResearch(output) });
    }
    return rounds;
  };
  const collectRoundFailures = (stepFailures, prefix) => {
    const failures = [];
    for (let roundNumber = 1; roundNumber <= maxResearchRounds; roundNumber += 1) {
      const failure = stepFailures[roundStepId(prefix, roundNumber)];
      if (failure) {
        failures.push({
          round: roundNumber,
          error: failure.error || "research round failed",
          attempt: failure.attempt
        });
      }
    }
    return failures;
  };
  const uniqueStrings = (items) => {
    const seen = new Set();
    const unique = [];
    for (const item of items) {
      const text = typeof item === "string" ? item.trim() : "";
      if (!text) {
        continue;
      }
      const key = text.toLowerCase();
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      unique.push(text);
    }
    return unique;
  };
  const structuredEvidence = (rounds) => {
    const structured = [];
    for (const round of rounds) {
      const results = round.research && Array.isArray(round.research.results)
        ? round.research.results
        : [];
      for (const result of results) {
        if (result && result.structured) {
          structured.push({ round: round.round, value: result.structured });
        }
      }
    }
    return structured;
  };
  const evidenceSummary = (rounds) => {
    const structured = structuredEvidence(rounds);
    const summaries = structured.map((item) => `round ${item.round}: ${item.value.summary}`);
    const sources = structured.flatMap((item) =>
      Array.isArray(item.value.sources)
        ? item.value.sources.map((source) => `${source.title || "source"} — ${source.url_or_path || ""}`)
        : []
    );
    const gaps = structured.flatMap((item) => Array.isArray(item.value.gaps) ? item.value.gaps : []);
    const contradictions = structured.flatMap((item) =>
      Array.isArray(item.value.contradictions) ? item.value.contradictions : []
    );
    return compactText(JSON.stringify({
      summaries: summaries.slice(-8),
      sources: uniqueStrings(sources).slice(-16),
      gaps: uniqueStrings(gaps).slice(-12),
      contradictions: uniqueStrings(contradictions).slice(-12)
    }), 4000);
  };
  const followUpTracks = (rounds, nextRound) => {
    const structured = structuredEvidence(rounds);
    const gaps = uniqueStrings(structured.flatMap((item) =>
      Array.isArray(item.value.gaps) ? item.value.gaps : []
    ));
    const contradictions = uniqueStrings(structured.flatMap((item) =>
      Array.isArray(item.value.contradictions) ? item.value.contradictions : []
    ));
    const tracks = [];
    for (const gap of gaps) {
      tracks.push({
        title: `Resolve gap: ${compactText(gap, 80)}`,
        focus: `Resolve this remaining evidence gap without repeating prior searches: ${gap}`
      });
    }
    for (const contradiction of contradictions) {
      tracks.push({
        title: `Check contradiction: ${compactText(contradiction, 80)}`,
        focus: `Investigate this contradiction or disagreement and decide which claim is best supported: ${contradiction}`
      });
    }
    if (tracks.length === 0 && nextRound <= minResearchRounds) {
      tracks.push({
        title: "Independent corroboration",
        focus: "Find independent corroboration for the strongest claims from prior rounds; avoid repeating the same sources."
      });
      tracks.push({
        title: "Adversarial caveat check",
        focus: "Look for missing caveats, outdated claims, weak sources, or counterexamples in the prior evidence."
      });
    }
    return tracks.slice(0, maxLocalParallelTasks);
  };
  const aggregateResearchRounds = (rounds, stopReason, workflowFailures) => {
    const aggregate = {
      algorithm: "bounded_recursive_parallel_retrieval_summary",
      tool: "parallel_task",
      status: "failed",
      max_rounds: maxResearchRounds,
      completed_rounds: rounds.length,
      stop_reason: stopReason,
      complexity: {
        score: complexityScore,
        min_rounds: minResearchRounds,
        max_rounds: maxResearchRounds,
        initial_track_count: tracks.length,
        max_parallel_tasks: maxLocalParallelTasks
      },
      metadata: {
        task_count: 0,
        result_count: 0,
        success_count: 0,
        failed_count: 0,
        all_success: true,
        partial_failure: false,
        allow_partial_failure: true,
        results: []
      },
      results: [],
      rounds: []
    };
    const failedTasks = [];
    for (const round of rounds) {
      const research = round.research || {};
      const metadata = research.metadata || {};
      const results = Array.isArray(research.results) ? research.results : [];
      const roundResults = results.map((item) => Object.assign({ round: round.round }, item));
      aggregate.metadata.task_count += Number(metadata.task_count) || 0;
      aggregate.metadata.result_count += Number(metadata.result_count) || 0;
      aggregate.metadata.success_count += Number(metadata.success_count) || 0;
      aggregate.metadata.failed_count += Number(metadata.failed_count) || 0;
      aggregate.results.push(...roundResults);
      aggregate.metadata.results.push(...roundResults);
      if (research.warnings && Array.isArray(research.warnings.failed_tasks)) {
        failedTasks.push(...research.warnings.failed_tasks.map((item) =>
          Object.assign({ round: round.round }, item)
        ));
      }
      aggregate.rounds.push({
        round: round.round,
        status: research.status || "unknown",
        metadata: {
          task_count: metadata.task_count || 0,
          result_count: metadata.result_count || 0,
          success_count: metadata.success_count || 0,
          failed_count: metadata.failed_count || 0
        },
        results: roundResults,
        warnings: research.warnings
      });
    }
    aggregate.metadata.all_success = aggregate.metadata.failed_count === 0;
    aggregate.metadata.partial_failure =
      aggregate.metadata.failed_count > 0 && aggregate.metadata.success_count > 0;
    aggregate.status = aggregate.metadata.failed_count > 0
      ? (aggregate.metadata.success_count > 0 ? "partial_success" : "failed")
      : "success";
    if (workflowFailures && workflowFailures.length > 0 && aggregate.metadata.success_count > 0) {
      aggregate.status = "partial_success";
      aggregate.metadata.partial_failure = true;
    }
    if (failedTasks.length > 0 || (workflowFailures && workflowFailures.length > 0)) {
      aggregate.warnings = {};
      if (failedTasks.length > 0) {
        aggregate.warnings.failed_tasks = failedTasks;
      }
      if (workflowFailures && workflowFailures.length > 0) {
        aggregate.warnings.failed_rounds = workflowFailures.map((failure) => ({
          round: failure.round,
          attempt: failure.attempt,
          error_summary: failureSummary(failure.error || "research round failed")
        }));
      }
    }
    return aggregate;
  };
  const shouldContinueRounds = (rounds, failures) => {
    if (rounds.length === 0 || failures.length > 0 || rounds.length >= maxResearchRounds) {
      return false;
    }
    const aggregate = aggregateResearchRounds(rounds, "checking_next_round", []);
    if (aggregate.metadata.success_count === 0) {
      return false;
    }
    return followUpTracks(rounds, rounds.length + 1).length > 0;
  };
  const hasStructuredEvidence = (runtimeOutput) =>
    runtimeOutput &&
    Array.isArray(runtimeOutput.results) &&
    runtimeOutput.results.some((item) => item && item.structured);
  const runtimeStepError = (stepName, message) =>
    stepName === "runtime_preflight"
      ? `runtime preflight failed: ${message}`
      : message;

  if (inputs.kind === "workflow") {
    const runtimePreflight = inputs.step_outputs.runtime_preflight;
    const runtimeResearch = inputs.step_outputs.runtime_research;
    const stepFailures = inputs.step_failures || {};
    const localRounds = collectRoundOutputs(inputs.step_outputs, "local_research");
    const localRoundFailures = collectRoundFailures(stepFailures, "local_research");
    const localFallbackRounds = collectRoundOutputs(inputs.step_outputs, "local_fallback");
    const localFallbackFailures = collectRoundFailures(stepFailures, "local_fallback");

    if (localFallbackRounds.length > 0) {
      if (shouldContinueRounds(localFallbackRounds, localFallbackFailures)) {
        const nextRound = localFallbackRounds.length + 1;
        return {
          type: "schedule_step",
          step_id: roundStepId("local_fallback", nextRound),
          step_name: "parallel_task",
          input: {
            allow_partial_failure: true,
            tasks: localTasks(nextRound, followUpTracks(localFallbackRounds, nextRound), evidenceSummary(localFallbackRounds))
          },
          retry: continueWorkflowRetry,
        };
      }
      return {
        type: "complete",
        output: {
          query,
          mode: "local_fallback",
          runtime_error: (runtimeResearch && runtimeResearch.runtime_error) || (runtimePreflight && runtimePreflight.runtime_error),
          research: aggregateResearchRounds(
            localFallbackRounds,
            localFallbackFailures.length > 0 ? "round_failed_after_partial_evidence" : "bounded_rounds_complete",
            localFallbackFailures
          )
        }
      };
    }

    if (localRounds.length > 0) {
      if (shouldContinueRounds(localRounds, localRoundFailures)) {
        const nextRound = localRounds.length + 1;
        return {
          type: "schedule_step",
          step_id: roundStepId("local_research", nextRound),
          step_name: "parallel_task",
          input: {
            allow_partial_failure: true,
            tasks: localTasks(nextRound, followUpTracks(localRounds, nextRound), evidenceSummary(localRounds))
          },
          retry: continueWorkflowRetry,
        };
      }
      return {
        type: "complete",
        output: {
          query,
          mode: "local_parallel_task",
          research: aggregateResearchRounds(
            localRounds,
            localRoundFailures.length > 0 ? "round_failed_after_partial_evidence" : "bounded_rounds_complete",
            localRoundFailures
          )
        }
      };
    }

    if (localRoundFailures.length > 0) {
      const recoveredRound = recoveredRoundFromFailures(localRoundFailures, 1);
      if (recoveredRound) {
        return {
          type: "complete",
          output: {
            query,
            mode: "local_parallel_task_partial_success",
            research: aggregateResearchRounds(
              [{ round: 1, research: recoveredRound }],
              "source_notes_retained",
              localRoundFailures
            )
          }
        };
      }
      return {
        type: "complete",
        output: {
          query,
          mode: "local_parallel_task_failed",
          research: {
            status: "failed",
            algorithm: "bounded_recursive_parallel_retrieval_summary",
            max_rounds: maxResearchRounds,
            completed_rounds: 0,
            error_summary: failureSummary(localRoundFailures[0].error || "local research step failed"),
            note: "Local evidence fan-out failed before producing usable structured evidence; synthesis should create a transparent fallback report instead of retrying the workflow."
          }
        }
      };
    }

    if (localFallbackFailures.length > 0) {
      return {
        type: "complete",
        output: {
          query,
          mode: "local_fallback_failed",
          runtime_error: (runtimeResearch && runtimeResearch.runtime_error) || (runtimePreflight && runtimePreflight.runtime_error),
          research: {
            status: "failed",
            algorithm: "bounded_recursive_parallel_retrieval_summary",
            max_rounds: maxResearchRounds,
            completed_rounds: 0,
            error_summary: failureSummary(localFallbackFailures[0].error || "local fallback research step failed"),
            note: "Both OS-runtime research and local fallback fan-out failed; synthesis should report the failure and materialize a transparent fallback artifact."
          }
        }
      };
    }

    if (runtimeResearch && !runtimeResearch.runtime_error) {
      return { type: "complete", output: { query, mode: "os_runtime", research: runtimeResearch } };
    }

    if (runtimeResearch && runtimeResearch.runtime_error) {
      return {
        type: "schedule_step",
        step_id: roundStepId("local_fallback", 1),
        step_name: "parallel_task",
        input: { allow_partial_failure: true, tasks: localTasks(1, tracks, "") },
        retry: continueWorkflowRetry,
      };
    }

    if (runtimePreflight && runtimePreflight.runtime_error) {
      return {
        type: "schedule_step",
        step_id: roundStepId("local_fallback", 1),
        step_name: "parallel_task",
        input: { allow_partial_failure: true, tasks: localTasks(1, tracks, "") },
        retry: continueWorkflowRetry,
      };
    }

    if (runtimePreflight && !runtimePreflight.runtime_error) {
      return {
        type: "schedule_step",
        step_id: "runtime_research",
        step_name: "runtime_research",
        input: {
          query,
          worker: input.worker || "deep-research-worker",
          runtime_timeout_ms: input.runtime_timeout_ms,
          tracks,
        },
        retry: { max_attempts: 1, delay_ms: 0 },
      };
    }

    if (osRuntimeEnabled && input.os_runtime) {
      return {
        type: "schedule_step",
        step_id: "runtime_preflight",
        step_name: "runtime_preflight",
        input: {
          query,
          worker: input.worker || "deep-research-worker",
          runtime_preflight_timeout_ms: input.runtime_preflight_timeout_ms,
          tracks,
        },
        retry: { max_attempts: 1, delay_ms: 0 },
      };
    }

    return {
      type: "schedule_step",
      step_id: roundStepId("local_research", 1),
      step_name: "parallel_task",
      input: { allow_partial_failure: true, tasks: localTasks(1, tracks, "") },
      retry: continueWorkflowRetry,
    };
  }

  if (
    inputs.kind === "step" &&
    (inputs.step_name === "runtime_preflight" || inputs.step_name === "runtime_research")
  ) {
    const isPreflight = inputs.step_name === "runtime_preflight";
    const runtimeTracks = isPreflight
      ? [{
          title: "Runtime capability preflight",
          focus: `Verify the OS Runtime worker can use read-only research tools and return schema-shaped evidence for: ${inputs.input.query}`
        }]
      : inputs.input.tracks;
    const result = await ctx.tool("runtime", {
      worker: inputs.input.worker,
      timeout_ms: isPreflight
        ? (inputs.input.runtime_preflight_timeout_ms || 90000)
        : (inputs.input.runtime_timeout_ms || 240000),
      tasks: runtimeTracks.map((track, index) => ({
        query: inputs.input.query,
        title: track.title || `Track ${index + 1}`,
        focus: track.focus || String(track),
        capability_probe: isPreflight,
        required_tools: ["web_search", "web_fetch", "read", "grep", "glob", "ls"],
        output_schema: evidenceSchema,
        requirements: isPreflight
          ? "Capability preflight only. Use at least one harmless read-only research tool available to you: web_search/web_fetch for current external evidence, or read/grep/glob/ls for local workspace evidence. Return a JSON object matching output_schema with at least one traceable URL or local path. If the required tools are unavailable or permission-denied, do not fabricate evidence; surface the failure."
          : "Use web search and full-page reads. Return a JSON object matching output_schema with URLs, dates, key evidence, contradictions, confidence notes, and gaps. Do not write report artifacts in worker tasks."
      }))
    });
    if (!result || result.exitCode !== 0) {
      return {
        runtime_error: runtimeStepError(inputs.step_name, (result && result.output) || "runtime tool failed"),
        runtime_result: result || null
      };
    }
    let runtimeOutput = null;
    try {
      runtimeOutput = typeof result.output === "string" ? JSON.parse(result.output) : result.output;
    } catch (_err) {
      runtimeOutput = null;
    }
    runtimeOutput = normalizeRuntimeOutput(runtimeOutput);
    if (runtimeOutput && runtimeOutput.partial) {
      return {
        runtime_error: runtimeStepError(inputs.step_name, runtimeOutput.note || "runtime tool returned partial results before every subtask finished"),
        runtime_result: result,
        runtime_output: runtimeOutput,
      };
    }
    if (!hasStructuredEvidence(runtimeOutput)) {
      return {
        runtime_error: runtimeStepError(inputs.step_name, "runtime tool returned no valid structured DeepResearch evidence"),
        runtime_result: result,
        runtime_output: runtimeOutput,
      };
    }
    return {
      mode: isPreflight ? "runtime_preflight" : "os_runtime",
      runtime: result,
      runtime_output: runtimeOutput
    };
  }

  return { error: `unknown dynamic workflow invocation: ${inputs.kind}/${inputs.step_name || ""}` };
}
"#
}
