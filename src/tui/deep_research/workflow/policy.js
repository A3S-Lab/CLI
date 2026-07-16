      structured
    };
    metadata.results = [result];
    const research = {
      tool: "web_search/web_fetch",
      algorithm: "direct_web_search_fetch",
      status: safeCollectionErrors.length > 0 ? "partial_success" : "success",
      metadata,
      results: [result]
    };
    if (safeCollectionErrors.length > 0) {
      research.warnings = { collection_errors: safeCollectionErrors.slice(0, 10) };
    }
    return research;
  };
  const collectDirectWebResearch = async () => {
    const retrievalStartedAtMs = Date.now();
    const searches = plannedSeedUrls
      .filter((url) => !excludedSourceKeys.has(canonicalObservedSourceKey(url)))
      .slice(0, directWebFetchLimit)
      .map((url, index) => ({
      query: `planned seed ${index + 1}`,
      planned_seed_group: true,
      results: [{
        title: `Planned source ${index + 1}`,
        url,
        content: "",
        engines: [],
        planned_seed: true
      }]
    }));
    const collectionErrors = [];
    const searchQueries = directWebQueries();
    if (searchQueries.length > 0) {
      const invocations = searchQueries.map((searchQuery, index) => {
        const args = {
          query: searchQuery,
          format: "json",
          limit: directWebMaxResults,
          timeout: directWebSearchTimeoutSecs
        };
        // Omitted uses the healthy engines enabled by a3s-search.
        if (directWebEngines.length > 0) {
          args.engines = directWebEngines;
        }
        return { id: `search-${index + 1}`, tool: "web_search", args };
      });
      let searchBatch = null;
      try {
        searchBatch = await ctx.tool("batch", {
          invocations,
          max_concurrency: Math.min(8, invocations.length)
        });
      } catch (err) {
        collectionErrors.push(`web_search batch failed: ${String(err && err.message ? err.message : err)}`);
      }
      const sections = batchOutputSections(searchBatch && searchBatch.output, searchQueries.length);
      const fallbackSearches = [];
      for (let index = 0; index < searchQueries.length; index += 1) {
        const searchQuery = searchQueries[index];
        const child = batchChildResult(searchBatch, sections, index);
        if (!child.success) {
          fallbackSearches.push({
            index,
            query: searchQuery,
            primary_error: `web_search returned no usable output for "${searchQuery}": ${compactText(child.output, 300)}`
          });
          continue;
        }
        const searchStatus = String(child.metadata.status || "");
        if (searchStatus === "failed") {
          fallbackSearches.push({
            index,
            query: searchQuery,
            primary_error: `web_search failed for "${searchQuery}" after all configured engines failed`
          });
          continue;
        }
        if (searchStatus === "partial") {
          const failedEngines = Array.isArray(child.metadata.engine_errors)
            ? child.metadata.engine_errors
                .map((item) => item && item.engine)
                .filter(Boolean)
                .slice(0, 4)
            : [];
          collectionErrors.push(
            `web_search returned partial results for "${searchQuery}"${failedEngines.length > 0 ? `; failed engines: ${failedEngines.join(", ")}` : ""}`
          );
        }
        const results = parseSearchResults(child.output);
        if (results.length === 0) {
          fallbackSearches.push({
            index,
            query: searchQuery,
            primary_error: `web_search returned zero parseable results for "${searchQuery}"`
          });
          continue;
        }
        searches.push({ query: searchQuery, results });
      }
      if (fallbackSearches.length > 0 && directWebEngines.length === 0) {
        const fallbackInvocations = fallbackSearches.map((item, index) => ({
          id: `search-fallback-${index + 1}`,
          tool: "web_search",
          args: {
            query: item.query,
            format: "json",
            limit: directWebMaxResults,
            timeout: directWebSearchTimeoutSecs,
            engines: ["brave"]
          }
        }));
        let fallbackBatch = null;
        try {
          fallbackBatch = await ctx.tool("batch", {
            invocations: fallbackInvocations,
            max_concurrency: Math.min(8, fallbackInvocations.length)
          });
        } catch (err) {
          collectionErrors.push(`web_search fallback batch failed: ${String(err && err.message ? err.message : err)}`);
        }
        const fallbackSections = batchOutputSections(
          fallbackBatch && fallbackBatch.output,
          fallbackSearches.length
        );
        for (let index = 0; index < fallbackSearches.length; index += 1) {
          const pending = fallbackSearches[index];
          const child = batchChildResult(fallbackBatch, fallbackSections, index);
          const results = child.success && String(child.metadata.status || "") !== "failed"
            ? parseSearchResults(child.output)
            : [];
          if (results.length > 0) {
            searches.push({ query: pending.query, results });
            continue;
          }
          collectionErrors.push(pending.primary_error);
          collectionErrors.push(
            `web_search Brave fallback returned no usable results for "${pending.query}"`
          );
          searches.push({ query: pending.query, results: [] });
        }
      } else {
        for (const pending of fallbackSearches) {
          collectionErrors.push(pending.primary_error);
          searches.push({ query: pending.query, results: [] });
        }
      }
    }
    const candidates = queryAwareFetchCandidates(searches, directWebFetchLimit);
    const fetches = [];
    const safeCandidates = [];
    for (const item of candidates) {
      const safeUrl = normalizeObservedSource(item.url);
      if (safeUrl && /^https?:\/\//i.test(safeUrl)) {
        safeCandidates.push({ item, safeUrl, fetchUrl: evidenceFetchUrl(safeUrl) });
      } else {
        fetches.push({ url: item.url, ok: false, output: "" });
        collectionErrors.push("web_fetch skipped an unsafe search result URL");
      }
    }
    if (safeCandidates.length > 0) {
      const invocations = safeCandidates.map(({ fetchUrl }, index) => ({
        id: `fetch-${index + 1}`,
        tool: "web_fetch",
        args: {
          url: fetchUrl,
          format: "markdown",
          timeout: directWebFetchTimeoutSecs
        }
      }));
      let fetchBatch = null;
      try {
        fetchBatch = await ctx.tool("batch", {
          invocations,
          max_concurrency: Math.min(8, invocations.length)
        });
      } catch (err) {
        collectionErrors.push(`web_fetch batch failed: ${String(err && err.message ? err.message : err)}`);
      }
      const sections = batchOutputSections(fetchBatch && fetchBatch.output, safeCandidates.length);
      for (let index = 0; index < safeCandidates.length; index += 1) {
        const { item, safeUrl } = safeCandidates[index];
        const child = batchChildResult(fetchBatch, sections, index);
        const transportOk = child.success && isNonEmptyString(child.output);
        const relevant = transportOk && textMatchesQuery(child.output);
        const ok = Boolean(transportOk && relevant);
        fetches.push({ url: item.url, ok, output: ok ? child.output : "" });
        if (!ok) {
          collectionErrors.push(transportOk
            ? `web_fetch returned off-topic page text for ${safeUrl}`
            : `web_fetch returned no usable page text for ${safeUrl}: ${compactText(child.output, 300)}`);
        }
      }
    }
    const research = directWebResearchFromSources(searches, fetches, collectionErrors);
    research.metadata.retrieval_started_at_ms = retrievalStartedAtMs;
    research.metadata.retrieval_elapsed_ms = Math.max(0, Date.now() - retrievalStartedAtMs);
    return research;
  };
  const failureSummary = (value) => {
    const rawLower = String(value || "").toLowerCase();
    if (rawLower.includes("structured output failed")) {
      return "Delegated task failed schema validation.";
    }
    const compact = compactText(value, 600);
    const lower = compact.toLowerCase();
    if (lower.includes("permission denied: tool")) {
      return "A required research tool was denied.";
    }
    if (lower.includes("max tool rounds")) {
      return "The task exhausted its tool-round budget.";
    }
    if (lower.includes("timed out") || lower.includes("[command timed out")) {
      return "The task timed out.";
    }
    if (
      lower.includes("[tool output truncated") ||
      lower.includes("full output artifact:") ||
      lower.includes("a3s://tool-output")
    ) {
      return "Oversized tool output was withheld.";
    }
    if (
      lower.includes(".a3s/workflow/") ||
      lower.includes(".a3s\\workflow\\") ||
      lower.includes("● searched") ||
      lower.includes("● ran") ||
      lower.includes("● read") ||
      lower.includes("• searched") ||
      lower.includes("• ran") ||
      lower.includes("• read") ||
      compact.includes("⎿")
    ) {
      return "Internal tool logs were withheld.";
    }
    return "The task returned no usable evidence.";
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
    const sourceAnchors = safeSourceAnchors(item);
    if (sourceAnchors.length > 0) {
      next.source_anchors = sourceAnchors;
    }
    if (success) {
      if (item.structured) {
        const verified = verifiedEvidenceObject(item, item.structured);
        if (verified.structured) {
          next.structured = verified.structured;
        } else {
          next.structured_error = verified.error;
        }
      } else {
        next.structured_error =
          "delegated task returned no schema-validated DeepResearch evidence";
      }
    } else {
      next.error_summary = failureSummary(item.output || item.error || "task failed");
    }
    return next;
  };
  const generatedObjectAsParallelOutput = (output, inheritedSourceUrls) => {
    if (!output || output.tool !== "generate_object" || !isNonEmptyString(output.output)) {
      return output;
    }
    let generated = null;
    try {
      const parsed = JSON.parse(output.output);
      generated = parsed && parsed.object && typeof parsed.object === "object" &&
        !Array.isArray(parsed.object) ? parsed.object : null;
    } catch (_) {
      generated = null;
    }
    if (!generated) {
      return output;
    }
    const outputSourceUrls = output.metadata && Array.isArray(output.metadata.inherited_source_urls)
      ? output.metadata.inherited_source_urls
      : [];
    const inheritedSourceTool = "web_fetch";
    const sourceAnchors = uniqueStrings([
      ...outputSourceUrls,
      ...(Array.isArray(inheritedSourceUrls) ? inheritedSourceUrls : [])
    ]).map((url) => ({
      tool: inheritedSourceTool,
      url_or_path: url
    }));
    const result = {
      task_id: "structured-maker",
      agent: "deep-research",
      success: true,
      source_anchors: sourceAnchors,
      structured: generated
    };
    return Object.assign({}, output, {
      metadata: Object.assign({}, output.metadata || {}, {
        task_count: 1,
        result_count: 1,
        success_count: 1,
        failed_count: 0,
        all_success: true,
        partial_failure: false,
        allow_partial_failure: true,
        results: [result]
      })
    });
  };
  const normalizeLocalResearch = (parallelOutput, inheritedSourceUrls) => {
    parallelOutput = generatedObjectAsParallelOutput(parallelOutput, inheritedSourceUrls);
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
    const validatedSuccesses = compactSuccesses.filter((item) => item.structured);
    const rejectedSuccesses = compactSuccesses.filter((item) => !item.structured);
    const successCount = validatedSuccesses.length;
    const failedCount = failedResults.length + rejectedSuccesses.length;
    const resultCount = results.length;
    const metadataTaskCount = Number(metadata.task_count);
    const taskCount = Number.isFinite(metadataTaskCount) ? metadataTaskCount : resultCount;
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
    if (failedResults.length > 0 || rejectedSuccesses.length > 0) {
      normalized.warnings = {
        failed_tasks: [
          ...failedResults.map((item) => compactLocalResult(item, false)),
          ...rejectedSuccesses.map((item) => ({
            task_id: item.task_id,
            session_id: item.session_id,
            agent: item.agent,
            success: false,
            error_summary: item.structured_error || "delegated evidence failed verification"
          }))
        ]
      };
    }
    return normalized;
  };
  const roundStepId = (prefix, roundNumber) =>
    roundNumber === 1 ? prefix : `${prefix}_round_${roundNumber}`;
  const collectRoundOutputs = (stepOutputs, prefix, inheritedSourceUrls) => {
    const rounds = [];
    for (let roundNumber = 1; roundNumber <= maxResearchRounds; roundNumber += 1) {
      const output = stepOutputs[roundStepId(prefix, roundNumber)];
      if (!output) {
        break;
      }
      rounds.push({
        round: roundNumber,
        research: normalizeLocalResearch(output, inheritedSourceUrls)
      });
    }
    return rounds;
  };
  const collectRoundFailures = (stepFailures, prefix) => {
    const failures = [];
    for (let roundNumber = 1; roundNumber <= maxResearchRounds; roundNumber += 1) {
      const failure = stepFailures[roundStepId(prefix, roundNumber)];
      if (failure) {
        const errorSummary = failureSummary(failure.error || "research round failed");
        failures.push({
          round: roundNumber,
          error: errorSummary,
          error_summary: errorSummary,
          attempt: failure.attempt
        });
      }
    }
    return failures;
  };
  const retainedSourceNotesFromFailure = (failure, roundNumber) => {
    const raw = failure && isNonEmptyString(failure.error) ? failure.error : "";
    if (!/structured output failed/i.test(raw) || !/\nOutput:\s*\n/i.test(raw)) {
      return null;
    }
    const output = raw
      .split(/\nOutput:\s*\n/i)
      .slice(1)
      .join("\n")
      .replace(/\n\[structured output failed:[\s\S]*$/i, "")
      .trim();
    if (!output) {
      return null;
    }
    const sources = [];
    const seen = new Set();
    for (const line of output.split(/\r?\n/)) {
      const matches = line.match(/https?:\/\/[^\s<>"'`]+/gi) || [];
      for (const rawMatch of matches) {
        const candidate = rawMatch.replace(/[),.;:!?\]}]+$/, "");
        const url = normalizeObservedSource(candidate);
        const key = canonicalObservedSourceKey(url);
        if (!url || !key || seen.has(key)) {
          continue;
        }
        seen.add(key);
        const urlIndex = line.indexOf(rawMatch);
        const before = line
          .slice(0, Math.max(0, urlIndex))
          .replace(/^\s*(?:[-*+]\s+|\d+[.)]\s+)?/, "")
          .replace(/[\s:：—–-]+$/, "")
          .trim();
        const after = line
          .slice(urlIndex + rawMatch.length)
          .replace(/^\s*[:：—–-]?\s*/, "")
          .trim();
        sources.push({
          title: compactText(sanitizeEvidenceText(before || "Delegated source note"), 200),
          url_or_path: url,
          quote_or_fact: compactText(
            sanitizeEvidenceText(after || "The delegated output referenced this source."),
            700
          ),
          reliability: "Unvalidated delegated source note retained after schema failure; verify before relying on it."
        });
        if (sources.length >= 8) {
          break;
        }
      }
      if (sources.length >= 8) {
        break;
      }
    }
    if (sources.length === 0) {
      return null;
    }
    const summaryMatch = output.match(/(?:^|\n)#{1,6}\s+Summary\s*\n+([\s\S]*?)(?=\n#{1,6}\s|\n\s*[-*+]\s+https?:|$)/i);
    const summary = compactText(
      sanitizeEvidenceText(
        summaryMatch && summaryMatch[1]
          ? summaryMatch[1]
          : "A delegated task returned traceable source notes but failed the required schema."
      ),
      1000
    );
    const structured = {
      summary,
      sources,
      key_evidence: uniqueStrings([
        summary,
        ...sources.map((source) => source.quote_or_fact)
      ]).slice(0, 8),
      contradictions: [],
      confidence: "Low until the retained source notes are independently verified.",
      gaps: ["The delegated task failed schema validation; only its bounded source notes were retained."]
    };
    const result = {
      task_id: `retained-source-notes-${roundNumber}`,
      agent: "deep-research",
      success: true,
      retained_source_notes: true,
      structured
    };
    return {
      round: roundNumber,
      research: {
        tool: "parallel_task",
        exit_code: 0,
        status: "partial_success",
        metadata: {
          task_count: 1,
          result_count: 1,
          success_count: 1,
          failed_count: 0,
          all_success: false,
          partial_failure: true,
          allow_partial_failure: true,
          results: [result]
        },
        results: [result]
      }
    };
  };
  const collectRetainedFailureRounds = (stepFailures, prefix) => {
    const rounds = [];
    for (let roundNumber = 1; roundNumber <= maxResearchRounds; roundNumber += 1) {
      const retained = retainedSourceNotesFromFailure(
        stepFailures[roundStepId(prefix, roundNumber)],
        roundNumber
      );
      if (retained) {
        rounds.push(retained);
      }
    }
    return rounds;
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
  const followUpIssueKey = (kind, text) =>
    `${kind}:${String(text).trim().toLowerCase()}`;
  const roundFollowUpIssues = (round) => {
    const structured = structuredEvidence([round]);
    const gaps = uniqueStrings(structured.flatMap((item) =>
      Array.isArray(item.value.gaps) ? item.value.gaps : []
    ));
    const contradictions = uniqueStrings(structured.flatMap((item) =>
      Array.isArray(item.value.contradictions) ? item.value.contradictions : []
    ));
    return [
      ...gaps.map((text) => ({ key: followUpIssueKey("gap", text), kind: "gap", text })),
      ...contradictions.map((text) => ({
        key: followUpIssueKey("contradiction", text),
        kind: "contradiction",
        text
      }))
    ];
  };
  const unresolvedFollowUpIssues = (rounds) => {
    let backlog = [];
    for (let index = 0; index < rounds.length; index += 1) {
      if (index > 0) {
        backlog = backlog.slice(maxLocalParallelTasks);
      }
      const queuedKeys = new Set(backlog.map((issue) => issue.key));
      for (const issue of roundFollowUpIssues(rounds[index])) {
        if (!queuedKeys.has(issue.key)) {
          backlog.push(issue);
          queuedKeys.add(issue.key);
        }
      }
    }
    return backlog;
  };
  const followUpTracks = (rounds) => {
    const tracks = [];
    const cap = Math.min(maxLocalParallelTasks, 4);
    for (const issue of unresolvedFollowUpIssues(rounds).slice(0, cap)) {
      if (issue.kind === "gap") {
        tracks.push({
          title: `Resolve gap: ${compactText(issue.text, 80)}`,
          focus: `Resolve this remaining evidence gap without repeating prior searches: ${issue.text}`
        });
      } else {
        tracks.push({
          title: `Check contradiction: ${compactText(issue.text, 80)}`,
          focus: `Investigate this contradiction or disagreement and decide which claim is best supported: ${issue.text}`
        });
      }
    }
    return tracks.slice(0, cap);
  };
  const aggregateResearchRounds = (rounds, stopReason, workflowFailures) => {
    const aggregate = {
      algorithm: "bounded_recursive_parallel_retrieval_summary",
      tool: "parallel_task",
      status: "failed",
      max_rounds: maxResearchRounds,
      completed_rounds: rounds.length,
      stop_reason: stopReason,
      plan_limits: {
        min_rounds: 1,
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
    const failedRoundCount = workflowFailures && workflowFailures.length > 0
      ? workflowFailures.length
      : 0;
    const hasAnyFailure = aggregate.metadata.failed_count > 0 || failedRoundCount > 0;
    aggregate.metadata.all_success = !hasAnyFailure;
    aggregate.metadata.partial_failure =
      hasAnyFailure && aggregate.metadata.success_count > 0;
    aggregate.status = hasAnyFailure
      ? (aggregate.metadata.success_count > 0 ? "partial_success" : "failed")
      : "success";
    if (failedTasks.length > 0 || (workflowFailures && workflowFailures.length > 0)) {
      aggregate.warnings = {};
      if (failedTasks.length > 0) {
        aggregate.warnings.failed_tasks = failedTasks;
      }
      if (workflowFailures && workflowFailures.length > 0) {
        aggregate.warnings.failed_rounds = workflowFailures;
      }
    }
    return aggregate;
  };
  const shouldScheduleFollowUpRound = (rounds, failures) => {
    if (rounds.length === 0 || failures.length > 0 || rounds.length >= maxResearchRounds) {
      return false;
    }
    const aggregate = aggregateResearchRounds(rounds, "checking_next_round", []);
    if (aggregate.metadata.success_count === 0) {
      return false;
    }
    const sourceKeysForRounds = (items) => new Set(structuredEvidence(items).flatMap((item) =>
      item.value.sources.map((source) => canonicalObservedSourceKey(source.url_or_path)).filter(Boolean)));
    const latestKeys = sourceKeysForRounds(rounds.slice(-1));
    const previousKeys = sourceKeysForRounds(rounds.slice(0, -1));
    const newSourceCount = Array.from(latestKeys).filter((key) => !previousKeys.has(key)).length;
    const pendingIssues = unresolvedFollowUpIssues(rounds);
    const latestIssueKeys = new Set(roundFollowUpIssues(rounds[rounds.length - 1]).map((issue) => issue.key));
    const hasCarriedOverflow = pendingIssues.some((issue) => !latestIssueKeys.has(issue.key));
    if (rounds.length > 1 && newSourceCount === 0) {
      if (!hasCarriedOverflow) {
        return false;
      }
    }
    return followUpTracks(rounds).length > 0;
  };
