async function run(ctx, inputs) {
  const input = inputs.input || {};
  const query = input.query || "";
  const loopContract = input.loop_contract && typeof input.loop_contract === "object"
    ? input.loop_contract
    : {};
  // Preserve the engineered-loop marker across scheduled-step inputs.
  const engineeredLoopEnabled = input.engineered_loop_enabled === true ||
    Boolean(loopContract.planner && loopContract.checker);
  const stepOutputs = inputs.step_outputs && typeof inputs.step_outputs === "object"
    ? inputs.step_outputs
    : {};
  const plannerStep = stepOutputs.research_planner;
  const plannerResults = plannerStep && plannerStep.metadata && Array.isArray(plannerStep.metadata.results)
    ? plannerStep.metadata.results
    : [];
  const delegatedPlan = plannerResults
    .filter((item) => item && item.success === true && item.structured && typeof item.structured === "object")
    .map((item) => item.structured)[0] || null;
  let generatedPlan = null;
  if (plannerStep && typeof plannerStep.output === "string" && plannerStep.output.trim()) {
    try {
      const generated = JSON.parse(plannerStep.output);
      generatedPlan = generated && generated.object && typeof generated.object === "object" && !Array.isArray(generated.object)
        ? generated.object
        : null;
    } catch (_) {
      generatedPlan = null;
    }
  }
  const normalizePlannerBudget = (plan) => {
    if (!plan || typeof plan !== "object" || Array.isArray(plan)) {
      return plan;
    }
    const budget = plan.budget && typeof plan.budget === "object" && !Array.isArray(plan.budget)
      ? plan.budget
      : {};
    const normalizedBudget = Object.assign({}, budget);
    for (const [secondsKey, millisecondsKey] of [
      ["retrieval_timeout_secs", "retrieval_timeout_ms"],
      ["synthesis_timeout_secs", "synthesis_timeout_ms"],
      ["per_task_timeout_secs", "per_task_timeout_ms"]
    ]) {
      const seconds = Number(normalizedBudget[secondsKey]);
      if (Number.isFinite(seconds) && seconds > 0) {
        normalizedBudget[millisecondsKey] = Math.floor(seconds * 1000);
      }
      delete normalizedBudget[secondsKey];
    }
    return Object.assign({}, plan, { budget: normalizedBudget });
  };
  const plannedResult = normalizePlannerBudget(delegatedPlan || generatedPlan);
  const researchPlan = input.research_plan && typeof input.research_plan === "object"
    ? input.research_plan
    : plannedResult;
  const plannedBudget = researchPlan && researchPlan.budget && typeof researchPlan.budget === "object"
    ? researchPlan.budget
    : {};
  const plannedRetrievalTimeoutMs = Number(plannedBudget.retrieval_timeout_ms);
  const plannerObservedLatencyMs = Number(
    plannerStep && plannerStep.metadata && plannerStep.metadata.step_elapsed_ms
  );
  const plannerStructuredMode = plannerStep && plannerStep.metadata &&
    typeof plannerStep.metadata.mode_used === "string"
    ? plannerStep.metadata.mode_used
    : "unknown";
  const plannedSynthesisTimeoutMs = Number(plannedBudget.synthesis_timeout_ms);
  const configuredCheckerTimeoutMs = Math.max(
    5000,
    Math.min(
      180000,
      Number(loopContract.checker && loopContract.checker.timeout_ms) || 180000
    )
  );
  const observedCheckerLatencyMs = Object.entries(stepOutputs).reduce(
    (maximum, [stepId, output]) => {
      if (!stepId.startsWith("research_checker_")) {
        return maximum;
      }
      const elapsedMs = Number(
        output && output.metadata && output.metadata.step_elapsed_ms
      );
      return Number.isFinite(elapsedMs) && elapsedMs > maximum ? elapsedMs : maximum;
    },
    0
  );
  const checkerLatencyBaseMs = Math.max(
    Number.isFinite(plannedSynthesisTimeoutMs) && plannedSynthesisTimeoutMs > 0
      ? plannedSynthesisTimeoutMs
      : 45000,
    Number.isFinite(plannerObservedLatencyMs) && plannerObservedLatencyMs > 0
      ? plannerObservedLatencyMs
      : 0,
    observedCheckerLatencyMs
  );
  const checkerReserveMs = Math.min(
    configuredCheckerTimeoutMs,
    Math.max(5000, checkerLatencyBaseMs)
  );
  const workflowClosureReserveMs = 5000;
  const retrievalStepElapsedMs = (stepId, output) => {
    const metadata = output && output.metadata && typeof output.metadata === "object"
      ? output.metadata
      : {};
    if (
      stepId === "direct_web_research" ||
      stepId.startsWith("direct_web_follow_up_") ||
      stepId.startsWith("direct_web_after_round_")
    ) {
      const elapsedMs = Number(metadata.retrieval_elapsed_ms);
      return Number.isFinite(elapsedMs) && elapsedMs > 0 ? elapsedMs : 0;
    }
    return 0;
  };
  const retrievalBudgetUsedMs = Object.entries(stepOutputs).reduce(
    (total, [stepId, output]) => total + retrievalStepElapsedMs(stepId, output),
    0
  );
  const retrievalBudgetExhausted = Boolean(
    researchPlan &&
    Number.isFinite(plannedRetrievalTimeoutMs) && plannedRetrievalTimeoutMs > 0 &&
    retrievalBudgetUsedMs >= plannedRetrievalTimeoutMs
  );
  const plannedOrInput = (planKey, inputKey) => input.research_plan_fixture === true
    ? input[inputKey]
    : (plannedBudget[planKey] ?? input[inputKey]);
  const fallbackTracks = [
    { title: "Facts and timeline", focus: "facts, dates, actors" },
    { title: "Primary sources", focus: "official evidence" },
    { title: "Independent analysis", focus: "analysis and disagreements" },
    { title: "Quantitative evidence", focus: "numbers and measurements" },
    { title: "Contradictions", focus: "conflicting claims" },
    { title: "Risks and caveats", focus: "uncertainty and recency" },
  ];
  const requestedLocalParallelTasks = Number(
    plannedOrInput("max_parallel_tasks", "local_max_parallel_tasks")
  );
  const maxLocalParallelTasks = Number.isFinite(requestedLocalParallelTasks) && requestedLocalParallelTasks > 0
    ? Math.max(1, Math.min(4, Math.floor(requestedLocalParallelTasks)))
    : 1;
  const requestedResearchRounds = Number(plannedOrInput("max_iterations", "local_research_rounds"));
  const maxResearchRounds = Number.isFinite(requestedResearchRounds) && requestedResearchRounds > 0
    ? Math.max(1, Math.min(4, Math.floor(requestedResearchRounds)))
    : 1;
  const initialFallbackTrackCount = Math.min(
    fallbackTracks.length,
    maxLocalParallelTasks,
    1
  );
  const requestedLocalMaxSteps = Number(plannedOrInput("max_steps_per_task", "local_max_steps"));
  const localMaxSteps = Number.isFinite(requestedLocalMaxSteps) && requestedLocalMaxSteps > 0
    ? Math.floor(requestedLocalMaxSteps)
    : 2;
  const localTaskMaxSteps = Math.max(1, Math.min(2, localMaxSteps));
  const localAgentTurnBudget = Math.max(2, localTaskMaxSteps + 1);
  const localEvidenceToolBudget = localTaskMaxSteps;
  const requestedLocalParallelTaskTimeoutMs = Number(
    plannedOrInput("per_task_timeout_ms", "local_parallel_task_timeout_ms")
  );
  const localParallelTaskTimeoutMs = Number.isFinite(requestedLocalParallelTaskTimeoutMs) && requestedLocalParallelTaskTimeoutMs > 0
    ? Math.max(1000, Math.floor(requestedLocalParallelTaskTimeoutMs))
    : 90 * 1000;
  const promptMakerReserveMs = Math.min(
    localParallelTaskTimeoutMs,
    checkerReserveMs
  );
  const makerReserveMs = plannerStructuredMode === "prompt"
    ? promptMakerReserveMs
    : localParallelTaskTimeoutMs;
  const makerAndCheckerFloorMs = makerReserveMs + checkerReserveMs;
  const evidenceScope = input.evidence_scope === "local_only" || input.evidence_scope === "web_and_workspace"
    ? input.evidence_scope
    : null;
  const directWebEnabled = evidenceScope
    ? evidenceScope === "web_and_workspace"
    : input.direct_web_enabled !== false;
  const workspaceEvidenceRequired = evidenceScope === "local_only" || (
    researchPlan
      ? researchPlan.workspace_evidence_required === true
      : evidenceScope === "web_and_workspace"
  );
  const directWebSeedEnabled = directWebEnabled;
  const evidenceScopeDirective = !directWebEnabled
    ? "Authoritative scope: local_only. Never use web tools; use workspace tools and report gaps."
    : (workspaceEvidenceRequired
      ? "Authoritative scope: web_and_workspace. Use web_search/web_fetch for public facts and workspace tools only for claims that genuinely depend on local artifacts."
      : "Authoritative scope: web_only. Use web_search/web_fetch and do not inspect the workspace; local or embedded product deployment is not workspace evidence.");
  const requestedDirectWebMaxResults = Number(input.direct_web_max_results);
  const directWebMaxResults = Number.isFinite(requestedDirectWebMaxResults) && requestedDirectWebMaxResults > 0
    ? Math.max(1, Math.min(12, Math.floor(requestedDirectWebMaxResults)))
    : 8;
  const requestedDirectWebFetchLimit = Number(
    plannedOrInput("direct_fetches", "direct_web_fetch_limit")
  );
  const directWebFetchLimit = Number.isFinite(requestedDirectWebFetchLimit) && requestedDirectWebFetchLimit >= 0
    ? Math.max(0, Math.min(8, Math.floor(requestedDirectWebFetchLimit)))
    : 2;
  const requestedDirectWebSearchLimit = Number(plannedOrInput("direct_searches", "direct_web_search_limit"));
  const directWebSearchLimit = Number.isFinite(requestedDirectWebSearchLimit) && requestedDirectWebSearchLimit > 0
    ? Math.max(1, Math.min(4, Math.floor(requestedDirectWebSearchLimit)))
    : 2;
  const executionRoute = researchPlan && ["direct_only", "direct_then_review", "direct_then_maker", "maker_first"]
    .includes(researchPlan.execution_route)
    ? researchPlan.execution_route
    : "direct_only";
  const directWebFirst = executionRoute !== "maker_first";
  const directThenMaker = Boolean(
    researchPlan && researchPlan.execution_route === "direct_then_maker"
  );
  const requestedDirectWebSearchTimeoutSecs = Number(
    plannedOrInput("direct_call_timeout_secs", "direct_web_search_timeout_secs")
  );
  const directWebSearchTimeoutSecs = Number.isFinite(requestedDirectWebSearchTimeoutSecs) && requestedDirectWebSearchTimeoutSecs > 0
    ? Math.max(1, Math.min(60, Math.floor(requestedDirectWebSearchTimeoutSecs)))
    : 8;
  const requestedDirectWebFetchTimeoutSecs = Number(
    plannedOrInput("direct_call_timeout_secs", "direct_web_fetch_timeout_secs")
  );
  const directWebFetchTimeoutSecs = Number.isFinite(requestedDirectWebFetchTimeoutSecs) && requestedDirectWebFetchTimeoutSecs > 0
    ? Math.max(1, Math.min(120, Math.floor(requestedDirectWebFetchTimeoutSecs)))
    : 10;
  const directWebEngines = Array.isArray(input.direct_web_engines)
    ? input.direct_web_engines.filter((engine) => typeof engine === "string" && engine.trim()).slice(0, 4)
    : [];
  const requestedLocalMinSuccessCount = Number(input.local_min_success_count);
  const localMinSuccessCount = (roundTracks) =>
    Number.isFinite(requestedLocalMinSuccessCount) && requestedLocalMinSuccessCount > 0
      ? Math.max(1, Math.min(roundTracks.length, Math.floor(requestedLocalMinSuccessCount)))
      : null;
  const continueWorkflowRetry = {
    max_attempts: 1,
    delay_ms: 0,
    on_exhausted: "continue_workflow",
  };
  const plannerWorkflowRetry = {
    max_attempts: 1,
    delay_ms: 0,
    on_exhausted: "continue_workflow",
  };
  const checkerWorkflowRetry = {
    max_attempts: 1,
    delay_ms: 0,
    on_exhausted: "continue_workflow",
  };
  const plannedTracks = researchPlan && Array.isArray(researchPlan.tracks)
    ? researchPlan.tracks
    : null;
  const requestedSearchQueries = Array.isArray(input.search_queries)
    ? input.search_queries.filter((item) => typeof item === "string" && item.trim())
    : [];
  const plannedSearchQueries = requestedSearchQueries.length > 0
    ? requestedSearchQueries
    : (researchPlan && Array.isArray(researchPlan.search_queries)
      ? researchPlan.search_queries.filter((item) => typeof item === "string" && item.trim())
      : []);
  const requestedSeedUrls = Array.isArray(input.seed_urls) ? input.seed_urls : [];
  const excludedSourceUrls = Array.isArray(input.excluded_urls)
    ? input.excluded_urls.filter((item) => typeof item === "string" && /^https?:\/\//i.test(item.trim()))
    : [];
  const plannedSeedUrls = (requestedSeedUrls.length > 0
    ? requestedSeedUrls
    : (researchPlan && Array.isArray(researchPlan.seed_urls) ? researchPlan.seed_urls : []))
        .filter((item) => typeof item === "string" && /^https?:\/\//i.test(item.trim()))
        .map((item) => item.trim());
  const plannedSeedEvidenceContext = plannedSeedUrls.length > 0
    ? JSON.stringify({ planned_seed_urls: plannedSeedUrls })
    : "";
  const providedTracks = Array.isArray(plannedTracks || input.tracks)
    ? (plannedTracks || input.tracks).filter((track) => {
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
  const packMakerTracks = (roundTracks) => {
    if (plannerStructuredMode !== "prompt" || !Array.isArray(roundTracks) || roundTracks.length <= 1) {
      return roundTracks;
    }
    const packedFocus = roundTracks.map((track, index) => {
      const title = typeof track === "string"
        ? track
        : (track && (track.title || track.focus)) || `Track ${index + 1}`;
      const focus = typeof track === "string"
        ? track
        : (track && (track.focus || track.title)) || title;
      return `${index + 1}. ${title}: ${focus}`;
    }).join("\n");
    return [{
      title: "Planned evidence synthesis",
      focus: `Cover every LLM-planned evidence track in one schema-valid evidence package:\n${packedFocus}`
    }];
  };
  const hasReusableEvidencePackage = (previousEvidence) =>
    plannerStructuredMode === "prompt" &&
    isNonEmptyString(previousEvidence) &&
    /"sources"\s*:/.test(previousEvidence) &&
    /"key_evidence"\s*:/.test(previousEvidence);
  const evidenceSchema = {
    type: "object",
    additionalProperties: false,
    properties: {
      summary: { type: "string", minLength: 1, maxLength: 600 },
      sources: {
        type: "array",
        minItems: 1,
        maxItems: 4,
        items: {
          type: "object",
          additionalProperties: false,
          properties: {
            title: { type: "string", maxLength: 200 },
            url_or_path: { type: "string", maxLength: 1000 },
            date: { type: "string", maxLength: 100 },
            quote_or_fact: { type: "string", maxLength: 600 },
            reliability: { type: "string", maxLength: 240 }
          },
          required: ["title", "url_or_path", "quote_or_fact"]
        }
      },
      key_evidence: {
        type: "array",
        minItems: 1,
        maxItems: 5,
        items: { type: "string", maxLength: 320 }
      },
      contradictions: {
        type: "array",
        maxItems: 3,
        items: { type: "string", maxLength: 240 }
      },
      confidence: { type: "string", minLength: 1, maxLength: 240 },
      gaps: {
        type: "array",
        maxItems: 4,
        items: { type: "string", maxLength: 240 }
      }
    },
    required: ["summary", "sources", "key_evidence", "contradictions", "confidence", "gaps"]
  };
  const evidenceSeedUrls = (previousEvidence) => {
    const context = String(previousEvidence || "");
    const failedSeedKeys = new Set();
    const failurePattern = /(?:no usable page text for|failed for|off-topic page text for)\s+(https?:\/\/[^\s<>"'\\]+)/gi;
    for (const match of context.matchAll(failurePattern)) {
      const failedUrl = normalizeObservedSource(String(match[1] || "").replace(/[.,;:!?)\]}]+$/, ""));
      const failedKey = canonicalObservedSourceKey(failedUrl);
      if (failedKey) {
        failedSeedKeys.add(failedKey);
      }
    }
    const matches = context.match(/https?:\/\/[^\s<>"'\\]+/gi) || [];
    const seen = new Set();
    const urls = [];
    for (const match of matches) {
      const url = normalizeObservedSource(match.replace(/[.,;:!?)\]}]+$/, ""));
      const key = canonicalObservedSourceKey(url);
      if (!url || !key || seen.has(key) || failedSeedKeys.has(key)) {
        continue;
      }
      seen.add(key);
      urls.push(url);
      if (urls.length >= 8) {
        break;
      }
    }
    return urls;
  };
  const localTasks = (roundNumber, roundTracks, previousEvidence) => roundTracks.map((track, index) => {
    const title = compactText(
      typeof track === "string" ? track : (track.title || `Track ${index + 1}`),
      48
    );
    const focus = typeof track === "string" ? track : (track.focus || title);
    const seedUrls = evidenceSeedUrls(previousEvidence);
    const roundContext = previousEvidence
      ? `\nExisting evidence and unresolved gaps; reuse these results instead of repeating broad collection:\n${previousEvidence}`
      : (roundNumber > 1
        ? `\nRecursive round: ${roundNumber}/${maxResearchRounds}. Remaining gaps: None recorded.`
        : "");
    const seedContext = seedUrls.length > 0
      ? `\nObserved candidate URLs (use only a URL that directly matches Focus):\n${seedUrls.map((url) => `- ${url}`).join("\n")}`
      : "";
    const collectionStrategy = hasReusableEvidencePackage(previousEvidence)
      ? "Existing evidence is already a schema-validated, traceable source package. Do not call any tool or refetch an observed URL. Analyze all evidence relevant to Focus, return the existing source-backed evidence without a tool call, and record any unsupported planned track as a precise gap for the checker to route."
      : seedUrls.length > 0
      ? `If Existing evidence already contains a traceable source and fact that directly supports Focus, return the existing source-backed evidence without a tool call. Otherwise, if one observed candidate URL directly matches Focus, use web_fetch on the best matching URL first. You may use the remaining part of the ${localEvidenceToolBudget}-round evidence budget for one focused search or one source-observed link only when the first page leaves a consequential gap. If no candidate matches Focus, make one focused web_search first and omit engines so the configured search service chooses healthy engines.`
      : `Use at most ${localEvidenceToolBudget} high-signal tool rounds and call at most one tool per round. For web_search, omit engines so the configured search service chooses healthy engines.`;
    return {
      agent: "deep-research",
      description: `${roundNumber}.${index + 1} · ${title}`,
      max_steps: localAgentTurnBudget,
      output_schema: evidenceSchema,
      prompt: `Deep-research evidence track for: ${query}\nFocus: ${focus}${seedContext}${roundContext}\nStay strictly within Focus; other tracks own the other source families and questions.\n${evidenceScopeDirective}\nEvidence focus: gather evidence first. You are an evidence collector. Do not use bash, python, curl, wget, node, or custom scripts. For local evidence, use read/grep/glob/ls only when the authoritative scope permits workspace evidence. Omit cursor on the first glob/ls call; only reuse an exact cursor returned by that tool. ${collectionStrategy} A web_search query must target one source family and one question; do not combine sites or questions with semicolons. Stop after at most ${localEvidenceToolBudget} evidence-tool rounds and immediately return the best output_schema object. Write summary and key_evidence as reader-facing conclusions, never as collection-status narration. If a tool fails, return a transparent gap instead of retrying or broadening. Do not inspect .a3s/workflow logs. Return output_schema fields: summary, sources, key_evidence, contradictions, confidence, and gaps.`
    };
  });
  const localParallelInput = (roundNumber, roundTracks, previousEvidence) => {
    const scheduledTracks = packMakerTracks(roundTracks);
    const input = {
      allow_partial_failure: true,
      timeout_ms: localParallelTaskTimeoutMs,
      tasks: localTasks(roundNumber, scheduledTracks, previousEvidence)
    };
    const minSuccess = localMinSuccessCount(scheduledTracks);
    if (minSuccess !== null) {
      input.min_success_count = minSuccess;
    }
    return input;
  };
  const structuredMakerInput = (roundNumber, roundTracks, previousEvidence) => {
    const tasks = localTasks(roundNumber, packMakerTracks(roundTracks), previousEvidence);
    if (tasks.length !== 1) {
      return null;
    }
    const inheritedSourceUrls = evidenceSeedUrls(previousEvidence);
    const inheritedSourceContext = inheritedSourceUrls.length > 0
      ? `\nRuntime-observed source anchors (reuse these exact URLs; they were observed before this step):\n${inheritedSourceUrls.map((url) => `- ${url}`).join("\n")}\nEnd runtime-observed source anchors.`
      : "";
    return {
      schema: evidenceSchema,
      schema_name: "deep_research_evidence",
      schema_description: "Source-grounded packed DeepResearch evidence synthesis",
      prompt: `${tasks[0].prompt}${inheritedSourceContext}`,
      mode: "auto",
      max_repair_attempts: 1,
      timeout_ms: localParallelTaskTimeoutMs
    };
  };
  const scheduleMakerStep = (
    stepId,
    roundNumber,
    roundTracks,
    previousEvidence,
    reuseExistingEvidenceOnly = false
  ) => {
    const structuredInput = engineeredLoopEnabled &&
      input.engineered_loop_fixture !== true &&
      reuseExistingEvidenceOnly &&
      hasReusableEvidencePackage(previousEvidence)
      ? structuredMakerInput(roundNumber, roundTracks, previousEvidence)
      : null;
    return {
      type: "schedule_step",
      step_id: stepId,
      step_name: structuredInput ? "generate_object" : "parallel_task",
      input: structuredInput || localParallelInput(roundNumber, roundTracks, previousEvidence),
      retry: continueWorkflowRetry
    };
  };
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
  const isLowValueSourceUrl = (url) => {
    const lower = String(url || "").toLowerCase();
    return /\.(?:7z|aac|apk|avi|avif|bin|bmp|bz2|deb|dmg|eot|exe|flac|gif|gz|ico|iso|jpe?g|m4a|mov|mp3|mp4|mpeg|msi|ogg|opus|otf|pkg|png|rar|rpm|svg|tar|tiff?|tgz|ttf|wasm|wav|webm|webp|woff2?|xz|zip)\/?$/.test(lower) ||
      /^https?:\/\/(?:[^/]+\.)?gravatar\.com(?:\/|$)/.test(lower) ||
      /^https?:\/\/avatars\.githubusercontent\.com(?:\/|$)/.test(lower) ||
      /^https?:\/\/api\.github\.com\/users\/[^/]+\/?$/.test(lower);
  };
  const normalizeObservedSource = (value) => {
    let text = String(value || "").trim();
    if (!text) {
      return "";
    }
    if (/^https?:\/\//i.test(text)) {
      text = text.split('#', 1)[0].split('?', 1)[0];
      const match = text.match(/^(https?):\/\/([^/]+)(.*)$/i);
      const scheme = match ? match[1].toLowerCase() : "";
      let authority = match ? match[2].split('@').pop().toLowerCase() : "";
      if ((scheme === "https" && authority.endsWith(":443")) ||
          (scheme === "http" && authority.endsWith(":80"))) {
        authority = authority.replace(/:\d+$/, "");
      }
      const normalized = match
        ? `${scheme}://${authority}${match[3] || "/"}`
        : "";
      return isLowValueSourceUrl(normalized) ? "" : normalized;
    }
    return text.replace(/\\/g, "/").replace(/^\.\//, "");
  };
  const reportedSourceCandidates = (value) => {
    const exact = normalizeObservedSource(value);
    if (!exact) {
      return [];
    }
    const candidates = [exact];
    if (!/^https?:\/\//i.test(exact)) {
      const withoutFragment = exact.split('#', 1)[0];
      if (withoutFragment && !candidates.includes(withoutFragment)) {
        candidates.push(withoutFragment);
      }
      const withoutLine = withoutFragment.replace(/:\d+$/, "");
      if (withoutLine && !candidates.includes(withoutLine)) {
        candidates.push(withoutLine);
      }
    }
    return candidates;
  };
  const canonicalObservedSourceKey = (value) => {
    const safe = normalizeObservedSource(value);
    if (!safe) {
      return "";
    }
    return /^https?:\/\//i.test(safe) ? safe.replace(/\/+$/, "") : safe;
  };
  const excludedSourceKeys = new Set(
    excludedSourceUrls.map(canonicalObservedSourceKey).filter(Boolean)
  );
  const observedSourceHost = (value) => {
    const safe = normalizeObservedSource(value);
    const match = safe.match(/^https?:\/\/([^/]+)/i);
    if (!match) {
      return "";
    }
    const authority = match[1].toLowerCase();
    if (authority.startsWith("[")) {
      const closing = authority.indexOf("]");
      return closing >= 0 ? authority.slice(0, closing + 1) : "";
    }
    return authority.replace(/:\d+$/, "");
  };
  const sourceAnchorTools = new Set(["read", "grep", "web_search", "web_fetch"]);
  const safeSourceAnchors = (item) => {
    const anchors = [];
    const seen = new Set();
    for (const anchor of (Array.isArray(item && item.source_anchors) ? item.source_anchors : [])) {
      const tool = anchor && typeof anchor === "object" && typeof anchor.tool === "string"
        ? anchor.tool
        : "";
      if (!sourceAnchorTools.has(tool)) {
        continue;
      }
      const safe = normalizeObservedSource(anchor.url_or_path);
      if (!safe || seen.has(safe)) {
        continue;
      }
      seen.add(safe);
      anchors.push({ tool, url_or_path: safe });
    }
    return anchors;
  };
  const sanitizeEvidenceText = (value) => String(value || "").replace(
    /https?:\/\/[^\s<>\"'`]+/gi,
    (match) => {
      let candidate = match;
      let suffix = "";
      while (/[),.;:!?\]}]$/.test(candidate)) {
        suffix = `${candidate.slice(-1)}${suffix}`;
        candidate = candidate.slice(0, -1);
      }
      const safe = normalizeObservedSource(candidate);
      return safe ? `${safe}${suffix}` : "";
    }
  );
  const verifiedEvidenceObject = (item, structured) => {
    if (!isEvidenceObject(structured)) {
      return {
        structured: null,
        error: "runtime output did not match DeepResearch evidence schema"
      };
    }
    const observed = new Map();
    for (const anchor of safeSourceAnchors(item)) {
      observed.set(anchor.url_or_path, anchor.url_or_path);
    }
    const sources = structured.sources
      .map((source) => {
        const safe = reportedSourceCandidates(source.url_or_path)
          .map((candidate) => observed.get(candidate))
          .find(Boolean);
        return safe
          ? {
              title: sanitizeEvidenceText(source.title),
              url_or_path: safe,
              quote_or_fact: sanitizeEvidenceText(source.quote_or_fact),
              date: source.date === undefined ? undefined : sanitizeEvidenceText(source.date),
              reliability: source.reliability === undefined
                ? undefined
                : sanitizeEvidenceText(source.reliability)
            }
          : null;
      })
      .filter(Boolean);
    const omitted = structured.sources.length - sources.length;
    if (sources.length === 0) {
      return {
        structured: null,
        error: "delegated evidence had no source observed by a successful research tool"
      };
    }
    const gaps = Array.isArray(structured.gaps) ? structured.gaps.slice() : [];
    if (omitted > 0) {
      gaps.push(`${omitted} self-reported source(s) omitted because no successful research tool observed them.`);
    }
    return {
      structured: {
        summary: sanitizeEvidenceText(structured.summary),
        sources,
        key_evidence: structured.key_evidence.map(sanitizeEvidenceText),
        contradictions: structured.contradictions.map(sanitizeEvidenceText),
        confidence: sanitizeEvidenceText(structured.confidence),
        gaps: Array.from(new Set(gaps.map(sanitizeEvidenceText).map((gap) => gap.trim()).filter(Boolean)))
      },
      error: null
    };
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
    return `${compact.slice(0, limit)}…`;
  };
  const analyzeQueryTerms = () => {
    const maxQueryTerms = 48;
    const fullQuery = String(query || "");
    const rawQuery = searchableQueryText();
    const inputTruncated = sanitizeEvidenceText(fullQuery).length > 140;
    const stopwords = new Set([
      "about", "after", "an", "and", "before", "brief", "citation", "citations", "cited", "cite",
      "compare", "comparison", "concise", "current", "evidence", "find", "from", "latest", "official",
      "in", "is", "of", "on", "or", "please", "release", "report", "research", "source",
      "sources", "stable", "summary", "the", "to", "using", "version", "versus", "vs",
      "what", "when", "where", "which", "who", "why", "with", "how"
    ]);
    const normalized = rawQuery
      .slice(0, 8192)
      .toLowerCase()
      .replace(/https?:\/\/\S+/g, " ");
    const asciiTerms = normalized.match(/[a-z0-9][a-z0-9+_.-]{1,}/g) || [];
    const cjkStopwords = new Set([
      "请", "请问", "全面", "调研", "研究", "报告", "来源", "证据", "最新", "当前",
      "什么", "如何", "比较", "对比", "以及"
    ]);
    const cjkTerms = [];
    for (const chunk of normalized.match(/[\u3400-\u9fff]{2,}/g) || []) {
      let content = chunk;
      for (const stopword of cjkStopwords) {
        content = content.split(stopword).join(" ");
      }
      for (const segment of content.match(/[\u3400-\u9fff]{2,}/g) || []) {
        if (segment.length === 2) {
          if (cjkTerms.length <= maxQueryTerms) {
            cjkTerms.push(segment);
          }
          continue;
        }
        for (let index = 0; index < segment.length - 1 && cjkTerms.length <= maxQueryTerms; index += 1) {
          cjkTerms.push(segment.slice(index, index + 2));
        }
      }
    }
    const candidates = uniqueStrings([...asciiTerms, ...cjkTerms])
      .filter((term) => !stopwords.has(term) && !cjkStopwords.has(term));
    return {
      terms: candidates.slice(0, maxQueryTerms),
      truncated: inputTruncated || candidates.length > maxQueryTerms
    };
  };
  let queryTermAnalysisCache = null;
  const queryTermAnalysis = () => {
    if (queryTermAnalysisCache === null) {
      queryTermAnalysisCache = analyzeQueryTerms();
    }
    return queryTermAnalysisCache;
  };
  const queryTerms = () => queryTermAnalysis().terms;
  const searchableQueryText = () => {
    const safeQuery = sanitizeEvidenceText(query);
    const cleaned = safeQuery
      .replace(/\b(concise|brief|cited|citation|citations|report|summary)\b/gi, " ")
      .replace(/\b(from|with|using|please)\b/gi, " ")
      .replace(/\s+/g, " ")
      .trim();
    const firstSentence = (cleaned || safeQuery.trim()).split(/[。！？\n]/)[0].trim();
    const topicMatch = firstSentence.match(/^.{0,60}?(?:关于|围绕)(.+)$/);
    const topic = (topicMatch ? topicMatch[1] : firstSentence).trim();
    return Array.from(topic).slice(0, 140).join("");
  };
  const queryTermMatches = (text, term) => {
    const haystack = String(text || "").toLowerCase();
    const needle = String(term || "").toLowerCase();
    if (!needle) {
      return false;
    }
    if (/[\u3400-\u9fff]/.test(needle)) {
      return haystack.includes(needle);
    }
    const compoundParts = needle.split(/[-_.]+/).filter(Boolean);
    const escaped = compoundParts
      .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
      .join(compoundParts.length > 1 ? "(?:[-_.]|\\s)+" : "");
    return new RegExp(`(^|[^a-z0-9])${escaped}($|[^a-z0-9])`, "i").test(haystack);
  };
  const isPrimarySourceUrl = (value) => {
    const url = String(value || "").toLowerCase();
    return /https?:\/\/(?:api\.)?github\.com\//.test(url) ||
      /https?:\/\/(?:www\.)?(?:crates\.io|docs\.rs|rustsec\.org)\//.test(url) ||
      /https?:\/\/[^/]+\.(?:gov|edu)(?:[/:]|$)/.test(url) ||
      /\/(?:releases?|changelog|advisories|security|documentation|docs)(?:[/?#]|$)/.test(url);
  };
  const sourceRelevanceScore = (item) => {
    const terms = queryTerms();
    if (terms.length === 0) {
      return 1;
    }
    const wantsPrimarySource = /(official|primary source|primary-source|官网|官方|原始来源|权威来源)/i.test(String(query || ""));
    const title = String(item.title || "").toLowerCase();
    const url = String(item.url || "").toLowerCase();
    const content = String(item.content || "").toLowerCase();
    let score = 0;
    let matchedTerms = 0;
    for (const term of terms) {
      let matched = false;
      if (queryTermMatches(title, term)) {
        score += 5;
        matched = true;
      }
      if (queryTermMatches(url, term)) {
        score += 4;
        matched = true;
      }
      if (queryTermMatches(content, term)) {
        score += 1;
        matched = true;
      }
      if (matched) {
        matchedTerms += 1;
      }
    }
    if (matchedTerms === 0) {
      return Number.NEGATIVE_INFINITY;
    }
    if (/(^|[./_-])(docs?|blog|download|developer|github|official)([./_-]|$)/.test(url)) {
      score += 3;
    }
    if (isPrimarySourceUrl(url)) {
      score += wantsPrimarySource ? 10 : 5;
    } else if (wantsPrimarySource && /\/(?:articles?|blog|comparisons?|tutorials?|guides?)(?:[/?#]|$)/.test(url)) {
      score -= 4;
    }
    if (/wikipedia\.org/.test(url) && !/wiki|wikipedia/.test(String(query || "").toLowerCase())) {
      score -= wantsPrimarySource ? 12 : 2;
    }
    return score;
  };
