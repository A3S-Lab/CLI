  const hasStructuredEvidence = (runtimeOutput) =>
    runtimeOutput &&
    Array.isArray(runtimeOutput.results) &&
    runtimeOutput.results.some((item) => item && item.success === true && item.structured);
  const plannerTaskInput = () => {
    const planner = loopContract.planner && typeof loopContract.planner === "object"
      ? loopContract.planner
      : null;
    if (!planner || !planner.output_schema || !isNonEmptyString(planner.prompt)) {
      return null;
    }
    return {
      schema: planner.output_schema,
      schema_name: "deep_research_plan",
      schema_description: "LLM-authored adaptive DeepResearch plan and budget",
      prompt: planner.prompt,
      mode: "auto",
      max_repair_attempts: 1,
      timeout_ms: Math.max(5000, Math.min(120000, Number(planner.timeout_ms) || 120000)),
    };
  };
  const structuredTaskOutput = (output) => {
    const results = output && output.metadata && Array.isArray(output.metadata.results)
      ? output.metadata.results
      : [];
    const delegated = results
      .filter((item) => item && item.success === true && item.structured && typeof item.structured === "object")
      .map((item) => item.structured)[0] || null;
    if (delegated) {
      return delegated;
    }
    const raw = output && isNonEmptyString(output.output) ? output.output : "";
    if (!raw) {
      return null;
    }
    try {
      const generated = JSON.parse(raw);
      return generated && generated.object && typeof generated.object === "object" && !Array.isArray(generated.object)
        ? generated.object
        : null;
    } catch (_) {
      return null;
    }
  };
  const checkerStepId = (kind, roundNumber) => {
    if (kind === "direct") {
      return "research_checker_direct";
    }
    if (kind === "direct_follow_up") {
      return `research_checker_direct_follow_up_${roundNumber}`;
    }
    if (kind === "round_follow_up") {
      return `research_checker_round_${roundNumber}_direct_follow_up`;
    }
    return `research_checker_round_${roundNumber}`;
  };
  const boundedDigestStrings = (values, charBudget, maxItems) => {
    const candidates = uniqueStrings(values);
    const count = Math.min(candidates.length, maxItems);
    if (count === 0 || charBudget <= 0) {
      return [];
    }
    // Reserve space for JSON framing and escaping.
    const perItemChars = Math.max(48, Math.floor((charBudget * 0.8) / count));
    return candidates.slice(0, count).map((item) => compactText(item, perItemChars));
  };
  const checkerEvidenceClassDigest = (research, charBudget) => {
    if (!research || typeof research !== "object" || Array.isArray(research)) {
      return { present: false };
    }
    const metadata = research.metadata && typeof research.metadata === "object"
      ? research.metadata
      : {};
    const results = Array.isArray(research.results) ? research.results : [];
    const structured = results
      .filter((item) => item && item.success === true && item.structured && typeof item.structured === "object")
      .map((item) => item.structured);
    const summaries = structured.map((item) => item.summary);
    const sources = [];
    const seenSources = new Set();
    for (const item of structured) {
      for (const source of Array.isArray(item.sources) ? item.sources : []) {
        if (!source || typeof source !== "object") {
          continue;
        }
        const url = compactText(source.url_or_path, 300);
        const key = canonicalObservedSourceKey(url) || url.toLowerCase();
        if (!url || seenSources.has(key)) {
          continue;
        }
        seenSources.add(key);
        const title = compactText(source.title, 140);
        const reliability = compactText(source.reliability, 120);
        const fact = compactText(source.quote_or_fact, 320);
        sources.push(`${url}${title ? ` — ${title}` : ""}${reliability ? ` [${reliability}]` : ""}${fact ? `: ${fact}` : ""}`);
      }
    }
    const keyEvidence = structured.flatMap((item) =>
      Array.isArray(item.key_evidence) ? item.key_evidence : []
    );
    const gaps = structured.flatMap((item) => Array.isArray(item.gaps) ? item.gaps : []);
    const contradictions = structured.flatMap((item) =>
      Array.isArray(item.contradictions) ? item.contradictions : []
    );
    const confidence = structured.map((item) => item.confidence);
    const failedTasks = research.warnings && Array.isArray(research.warnings.failed_tasks)
      ? research.warnings.failed_tasks
      : [];
    const failedRounds = research.warnings && Array.isArray(research.warnings.failed_rounds)
      ? research.warnings.failed_rounds
      : [];
    const failures = [
      ...failedTasks.map((item) => item && (item.error_summary || item.error)),
      ...failedRounds.map((item) => item && item.error)
    ];
    const contentBudget = Math.max(600, charBudget - 500);
    return {
      present: true,
      status: compactText(research.status, 80),
      algorithm: compactText(research.algorithm, 120),
      counts: {
        results: Number(metadata.result_count) || results.length,
        validated: structured.length,
        succeeded: Number(metadata.success_count) || 0,
        failed: Number(metadata.failed_count) || 0,
        sources: Number(metadata.source_count) || 0,
        fetched: Number(metadata.fetched_count) || 0,
        hosts: Number(metadata.host_count) || 0
      },
      summaries: boundedDigestStrings(summaries, Math.floor(contentBudget * 0.16), 6),
      sources: boundedDigestStrings(sources, Math.floor(contentBudget * 0.34), 8),
      key_evidence: boundedDigestStrings(keyEvidence, Math.floor(contentBudget * 0.22), 8),
      gaps: boundedDigestStrings(gaps, Math.floor(contentBudget * 0.09), 6),
      contradictions: boundedDigestStrings(
        contradictions,
        Math.floor(contentBudget * 0.09),
        6
      ),
      confidence: boundedDigestStrings(confidence, Math.floor(contentBudget * 0.05), 4),
      failures: boundedDigestStrings(failures, Math.floor(contentBudget * 0.05), 4)
    };
  };
  const checkerEvidenceDigest = (evidence) => {
    const combined = evidence && typeof evidence === "object" && !Array.isArray(evidence) &&
      (Object.prototype.hasOwnProperty.call(evidence, "maker") ||
        Object.prototype.hasOwnProperty.call(evidence, "direct"));
    const maker = combined ? evidence.maker : null;
    const direct = combined ? evidence.direct : evidence;
    const hasMaker = maker && typeof maker === "object";
    const hasDirect = direct && typeof direct === "object";
    const makerBudget = hasMaker && hasDirect ? 2600 : 3600;
    const directBudget = hasMaker && hasDirect ? 1800 : 3600;
    return JSON.stringify({
      digest_version: 1,
      maker: checkerEvidenceClassDigest(maker, makerBudget),
      direct: checkerEvidenceClassDigest(direct, directBudget)
    });
  };
  const makerEvidenceContext = (evidence) =>
    compactText(checkerEvidenceDigest(evidence), 4000);
  const checkerPlanDigest = () => JSON.stringify({
    answer_shape: researchPlan && researchPlan.answer_shape,
    report_title: researchPlan && researchPlan.report_title,
    execution_route: researchPlan && researchPlan.execution_route,
    freshness_required: Boolean(researchPlan && researchPlan.freshness_required === true),
    workspace_evidence_required: Boolean(
      researchPlan && researchPlan.workspace_evidence_required === true
    ),
    phases: researchPlan && Array.isArray(researchPlan.phases)
      ? researchPlan.phases.slice(0, 3)
      : [],
    tracks: researchPlan && Array.isArray(researchPlan.tracks)
      ? researchPlan.tracks.slice(0, 4)
      : [],
    stop_conditions: researchPlan && Array.isArray(researchPlan.stop_conditions)
      ? researchPlan.stop_conditions.slice(0, 3)
      : []
  });
  const workflowRemainingMs = () => {
    const startedAt = Number(input.run_started_at_ms);
    const timeoutMs = Number(input.workflow_timeout_ms);
    if (!Number.isFinite(startedAt) || startedAt <= 0 || !Number.isFinite(timeoutMs) || timeoutMs <= 0) {
      return null;
    }
    return Math.max(0, Math.floor(timeoutMs - Math.max(0, Date.now() - startedAt)));
  };
  const checkerBudgetDirective = () => {
    const remainingMs = workflowRemainingMs();
    const makerFloorMs = makerAndCheckerFloorMs;
    if (remainingMs === null) {
      return " Workflow budget: remaining wall time is unavailable; request only one consequential next action.";
    }
    const makerGuidance = remainingMs < makerFloorMs
      ? "Do not request maker: maker plus independent checking cannot finish in the remaining window. Finalize with explicit limitations unless the core answer would be misleading."
      : "A maker pass plus independent checking still fits, but request it only for a consequential multi-step or local-evidence gap.";
    return ` Workflow budget: approximately ${remainingMs} ms remains; one maker pass plus an independent checker needs at least ${makerFloorMs} ms. ${makerGuidance}`;
  };
  const checkerTaskInput = (kind, roundNumber, evidence) => {
    const checker = loopContract.checker && typeof loopContract.checker === "object"
      ? loopContract.checker
      : null;
    if (!checker || !checker.output_schema) {
      return null;
    }
    const boundedEvidence = checkerEvidenceDigest(evidence);
    const boundedPlan = compactText(checkerPlanDigest(), 2400);
    const webOnlyReview = directWebEnabled && !workspaceEvidenceRequired;
    const repeatedDirectDirective = kind === "direct_follow_up" || kind === "round_follow_up"
      ? (webOnlyReview
        ? " One direct follow-up already ran. Do not retrieve again or use maker for more web reading; finalize with limitations or degrade."
        : " One direct follow-up already ran. Do not repeat it; use maker only for a distinct non-web task, otherwise finalize or degrade.")
      : "";
    const capabilityDirective = webOnlyReview
      ? " Public web gaps use direct_retrieval. Maker is only for evidence production or required non-web/local work."
      : "";
    const remainingMs = workflowRemainingMs();
    const availableCheckerMs = remainingMs === null
      ? configuredCheckerTimeoutMs
      : Math.max(5000, remainingMs - workflowClosureReserveMs);
    return {
      schema: checker.output_schema,
      schema_name: "deep_research_check",
      schema_description: "Independent DeepResearch evidence coverage decision",
      prompt: `Check evidence against the semantic plan; do not call tools. Judge every planned track and stop condition. A URL or search snippet alone is not evidence; titles and snippets are leads unless retained page text supports the claim. Consequential status, quantitative, governance, and recommendation claims need primary or authoritative evidence plus independent corroboration when available. If a track has only snippets, generic/SEO comparisons, failed primary fetches, or unsupported inference and one follow-up fits, continue that gap with direct_retrieval; maker is only for distinct evidence production or non-web work. Never repeat failed work. Finalize only a useful source-backed answer; degrade only if the core answer would mislead. Finalize/degrade require next_action=none.${repeatedDirectDirective}${capabilityDirective}${checkerBudgetDirective()} Return concise schema fields in the query's language. report_summary directly answers the request; unsupported conditions stay in unresolved_gaps. Each verified finding states a fact present in retained evidence and includes its exact supporting source URL(s). Findings state facts, never that sources, articles, comparisons, discussions, or collection exist. Put uncertainty only in gaps or contradictions.\n\nPlan:\n${boundedPlan}\n\nEvidence (${kind}, iteration ${roundNumber}):\n${boundedEvidence}`,
      mode: "auto",
      max_repair_attempts: 1,
      timeout_ms: Math.min(configuredCheckerTimeoutMs, availableCheckerMs),
    };
  };
  const scheduleChecker = (kind, roundNumber, evidence) => {
    const input = checkerTaskInput(kind, roundNumber, evidence);
    return input ? {
      type: "schedule_step",
      step_id: checkerStepId(kind, roundNumber),
      step_name: "generate_object",
      input,
      retry: checkerWorkflowRetry,
    } : null;
  };
  const directFollowUpStepId = (roundNumber) => `direct_web_follow_up_${roundNumber}`;
  const roundDirectFollowUpStepId = (roundNumber) =>
    `direct_web_after_round_${roundNumber}`;
  const collectDirectFollowUps = (outputs) => {
    const followUps = [];
    for (let roundNumber = 1; roundNumber <= maxResearchRounds; roundNumber += 1) {
      const research = outputs[directFollowUpStepId(roundNumber)];
      if (!research) {
        break;
      }
      followUps.push({ round: roundNumber, research });
    }
    return followUps;
  };
  const collectRoundDirectFollowUps = (outputs) => {
    const followUps = [];
    for (let roundNumber = 1; roundNumber <= maxResearchRounds; roundNumber += 1) {
      const research = outputs[roundDirectFollowUpStepId(roundNumber)];
      if (research) {
        followUps.push({ round: maxResearchRounds + roundNumber, research });
      }
    }
    return followUps;
  };
  const cumulativeDirectResearch = (seed, followUps) => {
    const attempts = [{ round: 0, research: seed }, ...followUps];
    const results = [];
    const collectionErrors = [];
    let retrievalStartedAtMs = 0;
    let retrievalElapsedMs = 0;
    let successCount = 0;
    let failedCount = 0;
    let allEngineered = true;
    for (const attempt of attempts) {
      const research = attempt.research || {};
      const metadata = research.metadata || {};
      allEngineered = allEngineered && metadata.engineered_loop === true;
      if (!retrievalStartedAtMs && Number(metadata.retrieval_started_at_ms) > 0) {
        retrievalStartedAtMs = Number(metadata.retrieval_started_at_ms);
      }
      if (Number(metadata.retrieval_elapsed_ms) > 0) {
        retrievalElapsedMs += Number(metadata.retrieval_elapsed_ms);
      }
      successCount += Number(metadata.success_count) || 0;
      failedCount += Number(metadata.failed_count) || 0;
      const attemptResults = Array.isArray(research.results) ? research.results : [];
      for (let index = 0; index < attemptResults.length; index += 1) {
        const item = attemptResults[index];
        results.push(Object.assign({}, item, {
          iteration: attempt.round,
          task_id: `direct-${attempt.round}-${index + 1}`
        }));
      }
      const errors = research.warnings && Array.isArray(research.warnings.collection_errors)
        ? research.warnings.collection_errors
        : [];
      collectionErrors.push(...errors);
    }
    const hasFailure = failedCount > 0 || collectionErrors.length > 0;
    const metadata = {
      engineered_loop: allEngineered,
      retrieval_started_at_ms: retrievalStartedAtMs || undefined,
      retrieval_elapsed_ms: retrievalElapsedMs,
      task_count: attempts.length,
      result_count: results.length,
      success_count: successCount,
      failed_count: failedCount,
      all_success: !hasFailure && results.length > 0,
      partial_failure: hasFailure && results.length > 0,
      results
    };
    const aggregate = {
      tool: "web_search/web_fetch",
      algorithm: "llm_targeted_direct_retrieval",
      status: results.length === 0 ? "failed" : (hasFailure ? "partial_success" : "success"),
      completed_iterations: followUps.length,
      metadata,
      results
    };
    if (collectionErrors.length > 0) {
      aggregate.warnings = { collection_errors: uniqueStrings(collectionErrors).slice(0, 10) };
    }
    return aggregate;
  };
  const observedDirectUrls = (research) => {
    if (!research || typeof research !== "object") {
      return [];
    }
    const urls = [];
    for (const result of Array.isArray(research.results) ? research.results : []) {
      const structured = result && result.structured && typeof result.structured === "object"
        ? result.structured
        : null;
      for (const source of structured && Array.isArray(structured.sources) ? structured.sources : []) {
        if (source && isNonEmptyString(source.url_or_path)) {
          urls.push(source.url_or_path);
        }
      }
    }
    const errors = research.warnings && Array.isArray(research.warnings.collection_errors)
      ? research.warnings.collection_errors
      : [];
    for (const error of errors) {
      urls.push(...(String(error || "").match(/https?:\/\/[^\s<>"'\\]+/gi) || []));
    }
    const seen = new Set();
    return urls
      .map((url) => normalizeObservedSource(String(url).replace(/[.,;:!?)\]}]+$/, "")))
      .filter((url) => {
        const key = canonicalObservedSourceKey(url);
        if (!key || seen.has(key)) {
          return false;
        }
        seen.add(key);
        return true;
      });
  };
  const linkedDirectUrls = (research) => {
    if (!research || typeof research !== "object") {
      return [];
    }
    const alreadyObserved = new Set(observedDirectUrls(research).map(canonicalObservedSourceKey));
    const candidates = [];
    for (const result of Array.isArray(research.results) ? research.results : []) {
      const structured = result && result.structured && typeof result.structured === "object"
        ? result.structured
        : null;
      if (!structured) {
        continue;
      }
      const texts = [
        structured.summary,
        ...(Array.isArray(structured.key_evidence) ? structured.key_evidence : []),
        ...(Array.isArray(structured.sources)
          ? structured.sources.map((source) => source && source.quote_or_fact)
          : [])
      ];
      for (const text of texts) {
        candidates.push(...(String(text || "").match(/https?:\/\/[^\s<>"'\\]+/gi) || []));
      }
    }
    const seen = new Set();
    return candidates
      .map((url) => normalizeObservedSource(String(url).replace(/[.,;:!?)\]}]+$/, "")))
      .filter((url) => {
        const key = canonicalObservedSourceKey(url);
        if (!key || alreadyObserved.has(key) || seen.has(key)) {
          return false;
        }
        seen.add(key);
        return true;
      })
      .sort((left, right) => sourceRelevanceScore({ title: "", url: right, content: "" }) -
        sourceRelevanceScore({ title: "", url: left, content: "" }))
      .slice(0, directWebFetchLimit);
  };
  const makerFitsWorkflowBudget = () => {
    const remainingMs = workflowRemainingMs();
    return remainingMs === null || remainingMs >= makerAndCheckerFloorMs;
  };
  const checkerFitsWorkflowBudget = () => {
    const remainingMs = workflowRemainingMs();
    return remainingMs === null || remainingMs >= checkerReserveMs;
  };
  const budgetFinalizedChecker = (decision, reason, limitation) => Object.assign({}, decision || {}, {
    decision: "finalize",
    next_action: "none",
    coverage_summary: compactText(
      `${decision && decision.coverage_summary ? decision.coverage_summary : "Useful traceable evidence was collected."} ${limitation || "Remaining checked gaps must be stated as report limitations because another full checked iteration cannot finish inside the workflow budget."}`,
      800
    ),
    reason: reason || "The evidence package is reportable with explicit limitations; no further maker pass fits the remaining workflow budget."
  });
  const failedRecheckFinalizedChecker = (decision) => {
    const gap = "Follow-up evidence was not independently rechecked; conclusions relying on it remain provisional.";
    const checker = budgetFinalizedChecker(
      decision,
      "Previously checked findings remain reportable, but the follow-up recheck did not complete.",
      gap
    );
    checker.unresolved_gaps = uniqueStrings([
      gap,
      ...((decision && decision.unresolved_gaps) || [])
    ]).slice(0, 4);
    return checker;
  };
  const checkerContinuationTracks = (decision) => {
    if (!decision || typeof decision !== "object") {
      return [];
    }
    const requested = Array.isArray(decision.next_tracks)
      ? decision.next_tracks.filter((track) => track && typeof track === "object")
      : [];
    if (requested.length > 0) {
      return requested.slice(0, maxLocalParallelTasks);
    }
    return (Array.isArray(decision.unresolved_gaps) ? decision.unresolved_gaps : [])
      .filter(isNonEmptyString)
      .slice(0, maxLocalParallelTasks)
      .map((gap, index) => ({
        title: `Resolve checked gap ${index + 1}`,
        focus: gap,
        completion_criteria: ["Return traceable evidence or an explicit source-backed limitation"]
      }));
  };
  const directStepInput = (searchQueries, seedUrls, excludedUrls = []) => ({
    query,
    current_date: input.current_date,
    run_started_at_ms: input.run_started_at_ms,
    workflow_timeout_ms: input.workflow_timeout_ms,
    engineered_loop_enabled: engineeredLoopEnabled,
    engineered_loop_fixture: input.engineered_loop_fixture === true,
    research_plan_fixture: input.research_plan_fixture === true,
    research_plan: researchPlan,
    search_queries: Array.isArray(searchQueries) ? searchQueries : [],
    seed_urls: Array.isArray(seedUrls) ? seedUrls : [],
    excluded_urls: Array.isArray(excludedUrls) ? excludedUrls : [],
    direct_web_max_results: directWebMaxResults,
    direct_web_fetch_limit: directWebFetchLimit,
    direct_web_search_limit: directWebSearchLimit,
    direct_web_search_timeout_secs: directWebSearchTimeoutSecs,
    direct_web_fetch_timeout_secs: directWebFetchTimeoutSecs,
    direct_web_engines: directWebEngines
  });

  if (inputs.kind === "workflow") {
    const stepFailures = inputs.step_failures || {};
    const plannerFailure = stepFailures.research_planner;
    if (engineeredLoopEnabled && !researchPlan) {
      if (plannerFailure || plannerStep) {
        return {
          type: "complete",
          output: {
            query,
            mode: "planning_failed",
            research: {
              status: "failed",
              algorithm: "llm_planned_engineered_loop",
              error: plannerFailure && plannerFailure.error
                ? plannerFailure.error
                : "The LLM planner returned no schema-valid ResearchPlan."
            }
          }
        };
      }
      const plannerInput = plannerTaskInput();
      if (!plannerInput) {
        return {
          type: "complete",
          output: {
            query,
            mode: "planning_failed",
            research: {
              status: "failed",
              algorithm: "llm_planned_engineered_loop",
              error: "The host did not provide a valid Loop Engineering planner contract."
            }
          }
        };
      }
      return {
        type: "schedule_step",
        step_id: "research_planner",
        step_name: "generate_object",
        input: plannerInput,
        retry: plannerWorkflowRetry,
      };
    }
    const directWebResearch = inputs.step_outputs.direct_web_research;
    const directWebFailure = stepFailures.direct_web_research;
    const directFollowUps = collectDirectFollowUps(inputs.step_outputs);
    const roundDirectFollowUps = collectRoundDirectFollowUps(inputs.step_outputs);
    const directAttempts = [
      ...(directWebResearch ? [{ round: 0, research: directWebResearch }] : []),
      ...directFollowUps,
      ...roundDirectFollowUps
    ];
    const directEvidence = directAttempts.length === 0
      ? null
      : (directAttempts.length === 1
        ? directAttempts[0].research
        : cumulativeDirectResearch(
            directAttempts[0].research,
            directAttempts.slice(1)
          ));
    const inheritedDirectSourceUrls = uniqueStrings([
      ...observedDirectUrls(directEvidence),
      ...evidenceSeedUrls(JSON.stringify(directEvidence || {}))
    ]);
    const localRounds = collectRoundOutputs(
      inputs.step_outputs,
      "local_research",
      inheritedDirectSourceUrls
    );
    const localRoundFailures = collectRoundFailures(stepFailures, "local_research");
    const directRecoveryAfterMakerFailure =
      engineeredLoopEnabled &&
      input.engineered_loop_fixture !== true &&
      localRounds.length === 0 &&
      localRoundFailures.length > 0;

    if (
      retrievalBudgetExhausted &&
      localRounds.length === 0 &&
      !(directWebResearch && hasStructuredEvidence(directWebResearch))
    ) {
      return {
        type: "complete",
        output: {
          query,
          plan: researchPlan,
          mode: "retrieval_deadline_reached",
          research: {
            status: "failed",
            algorithm: "llm_planned_engineered_loop",
            error: "The LLM-planned retrieval deadline was reached before usable evidence was collected."
          }
        }
      };
    }

    if (
      directWebResearch &&
      hasStructuredEvidence(directWebResearch) &&
      localRounds.length === 0 &&
      (localRoundFailures.length === 0 || directRecoveryAfterMakerFailure)
    ) {
      if (!engineeredLoopEnabled || input.engineered_loop_fixture === true) {
        const directMetadata = directWebResearch.metadata || {};
        const fixtureReady =
          maxResearchRounds === 1 &&
          Number(directMetadata.source_count) >= 2 &&
          Number(directMetadata.host_count) >= 2 &&
          Number(directMetadata.fetched_count) >= 1 &&
          Number(directMetadata.fetched_host_count) >= 2 &&
          Number(directMetadata.query_term_count) > 0 &&
          Number(directMetadata.matched_query_term_count) >= Number(directMetadata.query_term_count) &&
          Number(directMetadata.fetched_query_term_count) >= Number(directMetadata.query_term_count) &&
          directMetadata.query_terms_truncated !== true &&
          (directMetadata.freshness_required !== true || Number(directMetadata.dated_source_count) >= 1) &&
          directMetadata.partial_failure !== true;
        if (fixtureReady || (directWebResearch.metadata && directWebResearch.metadata.terminal === true)) {
          return {
            type: "complete",
            output: { query, mode: "direct_web", plan: researchPlan, research: directWebResearch }
          };
        }
        return scheduleMakerStep(
          roundStepId("local_research", 1),
          1,
          tracks,
          makerEvidenceContext(directWebResearch)
        );
      }
      const directIteration = directFollowUps.length;
      if (
        directThenMaker &&
        directIteration === 0 &&
        !directRecoveryAfterMakerFailure &&
        tracks.length > 0 &&
        makerFitsWorkflowBudget()
      ) {
        return scheduleMakerStep(
          roundStepId("local_research", 1),
          1,
          tracks,
          makerEvidenceContext(directEvidence),
          true
        );
      }
      const checkerKind = directIteration === 0 ? "direct" : "direct_follow_up";
      const checkerId = checkerStepId(checkerKind, directIteration);
      const checkerFailure = stepFailures[checkerId];
      const checkerDecision = structuredTaskOutput(stepOutputs[checkerId]);
      const priorDirectChecker = directIteration > 0
        ? structuredTaskOutput(stepOutputs[checkerStepId("direct", 0)])
        : null;
      const failedFollowUp = stepFailures[directFollowUpStepId(directIteration + 1)];
      if (failedFollowUp) {
        return {
          type: "complete",
          output: {
            query,
            mode: "direct_web_degraded",
            plan: researchPlan,
            checker: checkerDecision,
            research: directEvidence,
            retrieval_error: failedFollowUp.error || "Targeted direct retrieval failed."
          }
        };
      }
      if (
        !checkerDecision &&
        !checkerFailure &&
        directIteration > 0 &&
        priorDirectChecker &&
        !checkerFitsWorkflowBudget()
      ) {
        return {
          type: "complete",
          output: {
            query,
            mode: "direct_web",
            plan: researchPlan,
            checker: budgetFinalizedChecker(
              priorDirectChecker,
              "The previously verified findings remain reportable with explicit limitations; the targeted follow-up is retained as evidence, but another independent checker pass cannot finish inside the workflow budget."
            ),
            research: directEvidence,
            budget_limited: true
          }
        };
      }
      if (!checkerDecision && !checkerFailure) {
        const checkerEvidence = directRecoveryAfterMakerFailure
          ? {
              direct: directEvidence,
              maker: aggregateResearchRounds(
                [],
                "maker_failed_before_direct_recovery",
                localRoundFailures
              )
            }
          : directEvidence;
        const scheduled = scheduleChecker(checkerKind, directIteration, checkerEvidence);
        if (scheduled) {
          return scheduled;
        }
      }
      if (checkerDecision && checkerDecision.decision === "finalize") {
        return {
          type: "complete",
          output: {
            query,
            mode: "direct_web",
            plan: researchPlan,
            checker: checkerDecision,
            research: directEvidence
          }
        };
      }
      if (checkerDecision && checkerDecision.decision === "degrade") {
        return {
          type: "complete",
          output: {
            query,
            mode: "direct_web_degraded",
            plan: researchPlan,
            checker: checkerDecision,
            research: directEvidence
          }
        };
      }
      if (checkerFailure) {
        if (directIteration > 0 && priorDirectChecker) {
          return {
            type: "complete",
            output: {
              query,
              mode: "direct_web",
              plan: researchPlan,
              checker: failedRecheckFinalizedChecker(priorDirectChecker),
              research: directEvidence,
              verification: {
                status: "degraded",
                checker_completed: false,
                prior_checker_retained: true,
                error: checkerFailure.error || "Evidence follow-up checker failed."
              }
            }
          };
        }
        return {
          type: "complete",
          output: {
            query,
            mode: "direct_web",
            plan: researchPlan,
            research: directEvidence,
            verification: {
              status: "degraded",
              checker_completed: false,
              error: checkerFailure.error || "Evidence checker failed."
            }
          }
        };
      }
      if (retrievalBudgetExhausted) {
        return {
          type: "complete",
          output: {
            query,
            mode: "direct_web_degraded",
            plan: researchPlan,
            checker: checkerDecision,
            research: directEvidence,
            checker_error: "The LLM-planned retrieval deadline was reached before the requested follow-up evidence could start."
          }
        };
      }
      const followUpQueries = checkerDecision && Array.isArray(checkerDecision.search_queries)
        ? checkerDecision.search_queries.filter(isNonEmptyString).slice(0, directWebSearchLimit)
        : [];
      const checkerFollowUpUrls = checkerDecision && Array.isArray(checkerDecision.seed_urls)
        ? checkerDecision.seed_urls
            .filter((item) => isNonEmptyString(item) && /^https?:\/\//i.test(item.trim()))
        : [];
      const observedFollowUpUrls = linkedDirectUrls(directEvidence);
      const followUpUrls = uniqueStrings(followUpQueries.length > 0
        ? [...checkerFollowUpUrls, ...observedFollowUpUrls]
        : [...observedFollowUpUrls, ...checkerFollowUpUrls]
      ).slice(0, directWebFetchLimit);
      if (
        checkerDecision &&
        checkerDecision.decision === "continue" &&
        checkerDecision.next_action === "direct_retrieval" &&
        directWebEnabled &&
        directIteration === 0 &&
        (followUpQueries.length > 0 || followUpUrls.length > 0)
      ) {
        const nextIteration = directIteration + 1;
        return {
          type: "schedule_step",
          step_id: directFollowUpStepId(nextIteration),
          step_name: "direct_web_research",
          input: directStepInput(
            followUpQueries,
            followUpUrls,
            observedDirectUrls(directEvidence)
          ),
          retry: continueWorkflowRetry,
        };
      }
      const nextTracks = checkerContinuationTracks(checkerDecision);
      if (
        checkerDecision &&
        checkerDecision.decision === "continue" &&
        (checkerDecision.next_action === "maker" ||
          (checkerDecision.next_action === "direct_retrieval" && directIteration > 0)) &&
        !makerFitsWorkflowBudget()
      ) {
        return {
          type: "complete",
          output: {
            query,
            mode: "direct_web",
            plan: researchPlan,
            checker: budgetFinalizedChecker(checkerDecision),
            research: directEvidence,
            budget_limited: true
          }
        };
      }
      if (
        checkerDecision &&
        checkerDecision.decision === "continue" &&
        (checkerDecision.next_action === "maker" ||
          (checkerDecision.next_action === "direct_retrieval" && directIteration > 0)) &&
        !directRecoveryAfterMakerFailure &&
        nextTracks.length > 0
      ) {
        return scheduleMakerStep(
