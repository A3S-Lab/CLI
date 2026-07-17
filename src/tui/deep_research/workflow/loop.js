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
        // The checker must see the retained source fact, not a title-sized
        // teaser. Truncating schema-bounded evidence here previously forced a
        // redundant retrieval/checker pass even though the maker had already
        // collected the required official text.
        const fact = compactText(source.quote_or_fact, 700);
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
    const leads = (Array.isArray(metadata.candidate_leads) ? metadata.candidate_leads : [])
      .filter((lead) => lead && typeof lead === "object" && isNonEmptyString(lead.url))
      .map((lead) => {
        const title = compactText(lead.title, 140);
        const queries = Array.isArray(lead.queries)
          ? boundedDigestStrings(lead.queries, 240, 2).join("; ")
          : "";
        return `${compactText(lead.url, 300)}${title ? ` — ${title}` : ""}${queries ? ` [lead for: ${queries}]` : ""}`;
      });
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
      summaries: boundedDigestStrings(summaries, Math.floor(contentBudget * 0.08), 6),
      sources: boundedDigestStrings(sources, Math.floor(contentBudget * 0.55), 10),
      discovery_leads_not_evidence: boundedDigestStrings(
        leads,
        Math.floor(contentBudget * 0.06),
        8
      ),
      key_evidence: boundedDigestStrings(keyEvidence, Math.floor(contentBudget * 0.12), 8),
      gaps: boundedDigestStrings(gaps, Math.floor(contentBudget * 0.07), 6),
      contradictions: boundedDigestStrings(
        contradictions,
        Math.floor(contentBudget * 0.05),
        6
      ),
      confidence: boundedDigestStrings(confidence, Math.floor(contentBudget * 0.03), 4),
      failures: boundedDigestStrings(failures, Math.floor(contentBudget * 0.04), 4)
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
    // Evidence transfer is cheaper than a false-negative checker followed by
    // another web pass and another checker call. Keep independent hard bounds,
    // but size them for the schema-bounded source facts collected by a normal
    // multi-track run.
    const makerBudget = hasMaker && hasDirect ? 10000 : 14000;
    const directBudget = hasMaker && hasDirect ? 6000 : 14000;
    return JSON.stringify({
      digest_version: 2,
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
      ? researchPlan.tracks.slice(0, 4).map((track, planIndex) => ({
          plan_index: planIndex,
          objective: assessmentLabel(track, `Track ${planIndex + 1}`)
        }))
      : [],
    stop_conditions: researchPlan && Array.isArray(researchPlan.stop_conditions)
      ? researchPlan.stop_conditions.slice(0, 3).map((condition, planIndex) => ({
          plan_index: planIndex,
          condition: assessmentLabel(condition, `Stop condition ${planIndex + 1}`)
        }))
      : []
  });
  const assessmentLabel = (value, fallback) => {
    if (isNonEmptyString(value)) return compactText(value, 240);
    if (!value || typeof value !== "object") return fallback;
    return compactText(value.title || value.focus || value.name || value.success_criterion, 240) || fallback;
  };
  const plannedAssessmentLabels = (values, prefix, limit) =>
    (Array.isArray(values) ? values : []).slice(0, limit)
      .map((value, index) => assessmentLabel(value, `${prefix} ${index + 1}`));
  const acceptedEvidenceSources = (evidence) => {
    const combined = evidence && typeof evidence === "object" &&
      (Object.prototype.hasOwnProperty.call(evidence, "maker") ||
        Object.prototype.hasOwnProperty.call(evidence, "direct"));
    const accepted = new Map();
    for (const research of combined ? [evidence.direct, evidence.maker] : [evidence]) {
      for (const result of research && Array.isArray(research.results) ? research.results : []) {
        if (!result || result.success !== true || !result.structured) continue;
        for (const source of Array.isArray(result.structured.sources) ? result.structured.sources : []) {
          const normalized = normalizeObservedSource(source && source.url_or_path);
          const key = canonicalObservedSourceKey(normalized);
          if (key && !accepted.has(key)) accepted.set(key, normalized);
        }
      }
    }
    return accepted;
  };
  const validateAssessmentSet = (raw, labels, labelKey, accepted) => {
    const byIndex = new Map();
    const duplicates = new Set();
    for (const item of Array.isArray(raw) ? raw : []) {
      const index = Number(item && item.plan_index);
      if (!Number.isInteger(index) || index < 0 || index >= labels.length) continue;
      if (byIndex.has(index)) duplicates.add(index); else byIndex.set(index, item);
    }
    let invalidSources = 0;
    const assessments = labels.map((label, planIndex) => {
      const rawItem = byIndex.get(planIndex);
      const finding = compactText(rawItem && rawItem.finding, 1600) || `No supported finding for ${label}.`;
      const sourceUrls = [];
      let invalidForItem = 0;
      for (const url of uniqueStrings(rawItem && Array.isArray(rawItem.source_urls) ? rawItem.source_urls : [])) {
        const key = canonicalObservedSourceKey(url);
        if (key && accepted.has(key)) sourceUrls.push(accepted.get(key));
        else { invalidSources += 1; invalidForItem += 1; }
      }
      let status = rawItem && ["supported", "bounded", "uncovered"].includes(rawItem.status)
        ? rawItem.status : "bounded";
      if (duplicates.has(planIndex) ||
          (status === "supported" && (sourceUrls.length === 0 || invalidForItem > 0))) status = "bounded";
      const item = { plan_index: planIndex, status, finding, source_urls: uniqueStrings(sourceUrls) };
      item[labelKey] = label;
      return item;
    });
    return {
      assessments,
      invalidSources,
      ok: labels.length > 0 && assessments.every((item) => item.status === "supported"),
      gaps: assessments.filter((item) => item.status !== "supported")
        .map((item) => `${item[labelKey]}: ${item.finding}`)
    };
  };
  const validateCheckerDecision = (decision, evidence) => {
    if (!decision || typeof decision !== "object") return null;
    const trackLabels = plannedAssessmentLabels(researchPlan && researchPlan.tracks, "Track", 4);
    const stopLabels = plannedAssessmentLabels(researchPlan && researchPlan.stop_conditions, "Stop condition", 3);
    const accepted = acceptedEvidenceSources(evidence);
    const tracks = validateAssessmentSet(decision.track_assessments, trackLabels, "track", accepted);
    const stops = validateAssessmentSet(decision.stop_condition_assessments, stopLabels, "stop_condition", accepted);
    const rawGaps = uniqueStrings(Array.isArray(decision.unresolved_gaps) ? decision.unresolved_gaps : []);
    const invalidSources = tracks.invalidSources + stops.invalidSources;
    const contractSatisfied = accepted.size > 0 && tracks.ok && stops.ok && rawGaps.length === 0 && invalidSources === 0;
    const forcedDegrade = decision.decision === "finalize" && !contractSatisfied;
    const verifiedFindings = uniqueStrings([...tracks.assessments, ...stops.assessments]
      .filter((item) => item.status === "supported").map((item) => item.finding)).slice(0, 8);
    const sanitized = Object.assign({}, decision, {
      decision: forcedDegrade ? "degrade" : decision.decision,
      report_summary: compactText(verifiedFindings.join(" ") ||
        "The retained evidence does not support a complete answer to the requested question.", 4800),
      verified_findings: verifiedFindings,
      track_assessments: tracks.assessments,
      stop_condition_assessments: stops.assessments,
      unresolved_gaps: uniqueStrings([...rawGaps, ...tracks.gaps, ...stops.gaps]).slice(0, 12),
      limitations: uniqueStrings(Array.isArray(decision.limitations) ? decision.limitations : []).slice(0, 8),
      contract_validation: {
        version: 1,
        finalize_gate_passed: contractSatisfied,
        accepted_source_count: accepted.size,
        planned_track_count: trackLabels.length,
        supported_track_count: tracks.assessments.filter((item) => item.status === "supported").length,
        planned_stop_condition_count: stopLabels.length,
        supported_stop_condition_count: stops.assessments.filter((item) => item.status === "supported").length,
        invalid_source_reference_count: invalidSources
      }
    });
    if (forcedDegrade) {
      const issues = [!tracks.ok && "tracks", !stops.ok && "stop conditions",
        rawGaps.length > 0 && "material gaps", invalidSources > 0 && "unaccepted sources"]
        .filter(Boolean).join(", ");
      Object.assign(sanitized, {
        next_action: "none", search_queries: [], seed_urls: [], next_tracks: [],
        reason: `Host evidence validation rejected finalize: ${issues || "no accepted evidence"}.`
      });
    }
    return sanitized;
  };
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
    const remainingDirectPasses = Math.max(0, maxResearchRounds - roundNumber - 1);
    const repeatedDirectDirective = kind === "direct_follow_up" || kind === "round_follow_up"
      ? (remainingDirectPasses > 0
        ? ` A direct follow-up already ran; ${remainingDirectPasses} planned evidence pass(es) remain. Request another direct_retrieval only for a still-consequential gap with a new query or unfetched observed lead.`
        : (webOnlyReview
          ? " The planned direct evidence passes are exhausted. Do not use maker for more web reading; finalize only if the core answer is supported, otherwise degrade."
          : " The planned direct evidence passes are exhausted. Use maker only for a distinct non-web task; otherwise finalize only if the core answer is supported, or degrade."))
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
      prompt: `Check evidence against the semantic plan; do not call tools. Judge every planned track and stop condition by its plan_index. Return exactly one track_assessments entry for every planned track and one stop_condition_assessments entry for every stop condition. Mark an entry supported only when its finding is present in retained relevant page text and source_urls contains the exact supporting source URL(s) or path(s) from the retained source list; bounded means partially supported, and uncovered means no retained support. A URL, title, or search snippet alone is a discovery lead, never evidence; only retained relevant page text supports a claim. Consequential status, quantitative, governance, comparison, and recommendation claims need primary or authoritative evidence plus independent corroboration when available. If a material track has only discovery leads, generic/SEO comparisons, failed primary fetches, or unsupported inference and another planned pass fits, continue that exact gap with direct_retrieval; maker is only for distinct evidence production or non-web work. Never repeat a failed URL or unchanged query. Finalize only when every track and stop condition is supported and unresolved_gaps is empty. Put only material plan obligations that prevent the requested answer in unresolved_gaps; put peripheral caveats that do not change the core answer in limitations. An exhausted clock or pass budget never turns insufficient core evidence into finalize. Choose degrade when a core comparison, requested recommendation, or material stop condition remains unsupported. Finalize/degrade require next_action=none.${repeatedDirectDirective}${capabilityDirective}${checkerBudgetDirective()} Return concise schema fields in the query's language. report_summary directly answers the request and must not fill evidence gaps from prior knowledge. Each verified finding states a fact present in retained evidence, but the host will rebuild verified_findings from supported assessments. Findings state facts, never that sources, articles, comparisons, discussions, or collection exist. Put uncertainty only in gaps, limitations, or contradictions.\n\nPlan:\n${boundedPlan}\n\nEvidence (${kind}, iteration ${roundNumber}):\n${boundedEvidence}`,
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
    const candidateLeads = [];
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
      for (const lead of Array.isArray(metadata.candidate_leads) ? metadata.candidate_leads : []) {
        if (lead && typeof lead === "object" && isNonEmptyString(lead.url)) {
          candidateLeads.push(lead);
        }
      }
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
    const sourceKeys = new Set();
    const sourceHosts = new Set();
    let datedSourceCount = 0;
    for (const result of results) {
      const structured = result && result.structured && typeof result.structured === "object"
        ? result.structured
        : null;
      for (const source of structured && Array.isArray(structured.sources) ? structured.sources : []) {
        const url = source && source.url_or_path;
        const key = canonicalObservedSourceKey(url);
        if (!key || sourceKeys.has(key)) {
          continue;
        }
        sourceKeys.add(key);
        const host = observedSourceHost(url);
        if (host) {
          sourceHosts.add(host);
        }
        if (source && isNonEmptyString(source.date)) {
          datedSourceCount += 1;
        }
      }
    }
    const uniqueCandidateLeads = [];
    const candidateKeys = new Set();
    for (const lead of candidateLeads) {
      const key = canonicalObservedSourceKey(lead.url);
      if (!key || candidateKeys.has(key)) {
        continue;
      }
      candidateKeys.add(key);
      uniqueCandidateLeads.push(lead);
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
      source_count: sourceKeys.size,
      fetched_count: sourceKeys.size,
      host_count: sourceHosts.size,
      fetched_host_count: sourceHosts.size,
      dated_source_count: datedSourceCount,
      candidate_urls: uniqueCandidateLeads.map((lead) => lead.url),
      candidate_leads: uniqueCandidateLeads,
      all_success: !hasFailure && results.length > 0,
      partial_failure: hasFailure && results.length > 0,
      results
    };
    const aggregate = {
      tool: "web_search/web_fetch",
      algorithm: "llm_targeted_direct_retrieval",
      status: results.length === 0
        ? (uniqueCandidateLeads.length > 0 ? "leads_only" : "failed")
        : (hasFailure ? "partial_success" : "success"),
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
    const metadata = research.metadata && typeof research.metadata === "object"
      ? research.metadata
      : {};
    const candidateLeads = Array.isArray(metadata.candidate_leads)
      ? metadata.candidate_leads
      : [];
    const sourceObservedCandidates = candidateLeads
      .filter((lead) => lead && lead.source_observed === true && isNonEmptyString(lead.url))
      .map((lead) => lead.url);
    const discoveryCandidates = candidateLeads.length > 0
      ? candidateLeads
          .filter((lead) => lead && lead.source_observed !== true && isNonEmptyString(lead.url))
          .map((lead) => lead.url)
      : (Array.isArray(metadata.candidate_urls) ? metadata.candidate_urls : []);
    const linkedCandidates = [];
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
        linkedCandidates.push(...(String(text || "").match(/https?:\/\/[^\s<>"'\\]+/gi) || []));
      }
    }
    const seen = new Set();
    const normalized = (candidates) => candidates
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
        sourceRelevanceScore({ title: "", url: left, content: "" }));
    // A link parsed from a successfully fetched page has stronger provenance
    // than checker-generated or search-result URLs. Preserve that provenance
    // through cumulative rounds and only then consider lower-confidence leads.
    return normalized(sourceObservedCandidates)
      .concat(normalized(linkedCandidates))
      .concat(normalized(discoveryCandidates))
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
  const budgetClosedChecker = (decision, reason, limitation) => {
    const priorDecision = decision && decision.decision;
    const terminalDecision = priorDecision === "finalize" ? "finalize" : "degrade";
    return Object.assign({}, decision || {}, {
      decision: terminalDecision,
      next_action: "none",
      coverage_summary: compactText(
        `${decision && decision.coverage_summary ? decision.coverage_summary : "The checked evidence package is incomplete."} ${limitation || "A remaining checked gap cannot be resolved and independently checked inside the workflow budget."}`,
        800
      ),
      reason: reason || (terminalDecision === "finalize"
        ? "The previously finalized evidence remains reportable; no further checked pass fits the remaining workflow budget."
        : "The checker requested more evidence, but no further checked pass fits the remaining workflow budget; the run must remain degraded rather than claim completion.")
    });
  };
  const failedRecheckFinalizedChecker = (decision) => {
    const gap = "Follow-up evidence was not independently rechecked; conclusions relying on it remain provisional.";
    const checker = budgetClosedChecker(
      decision,
      "Previously checked findings are retained, but the follow-up recheck did not complete, so unresolved core gaps remain degraded.",
      gap
    );
    checker.unresolved_gaps = uniqueStrings([
      gap,
      ...((decision && decision.unresolved_gaps) || [])
    ]).slice(0, 4);
    return checker;
  };
  const noEvidenceAfterFollowUpChecker = (decision) => ({
    decision: "degrade",
    next_action: "none",
    coverage_summary: compactText(
      `${decision && decision.coverage_summary ? decision.coverage_summary : "The initial evidence pass was incomplete."} The targeted follow-up also retained no relevant page text; discovery leads and transport failures are not evidence.`,
      800
    ),
    report_summary: compactText(
      decision && decision.report_summary
        ? decision.report_summary
        : "The requested answer cannot be supported because no traceable source text was retained.",
      1200
    ),
    verified_findings: [],
    unresolved_gaps: uniqueStrings(
      decision && Array.isArray(decision.unresolved_gaps)
        ? decision.unresolved_gaps
        : ["No traceable source text was retained after the targeted follow-up."]
    ).slice(0, 4),
    contradictions: [],
    search_queries: [],
    seed_urls: [],
    next_tracks: [],
    reason: "A checked follow-up still produced zero accepted evidence, so another model checker cannot turn the same discovery leads into a supported answer."
  });
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
      (hasStructuredEvidence(directWebResearch) || hasCandidateLeads(directWebResearch)) &&
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
      const checkerDecision = validateCheckerDecision(
        structuredTaskOutput(stepOutputs[checkerId]),
        checkerEvidence
      );
      const priorDirectChecker = directIteration > 0
        ? validateCheckerDecision(
            structuredTaskOutput(stepOutputs[checkerStepId("direct", 0)]),
            directWebResearch
          )
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
        !hasStructuredEvidence(directEvidence)
      ) {
        return {
          type: "complete",
          output: {
            query,
            mode: "direct_web_degraded",
            plan: researchPlan,
            checker: noEvidenceAfterFollowUpChecker(priorDirectChecker),
            research: directEvidence,
            zero_evidence_after_follow_up: true
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
            mode: "direct_web_degraded",
            plan: researchPlan,
            checker: budgetClosedChecker(
              priorDirectChecker,
              "The targeted follow-up is retained, but the prior checker requested more evidence and another independent checker pass cannot finish; the run remains degraded."
            ),
            research: directEvidence,
            budget_limited: true
          }
        };
      }
      if (!checkerDecision && !checkerFailure) {
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
              mode: "direct_web_degraded",
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
        directIteration + 1 < maxResearchRounds &&
        !retrievalBudgetExhausted &&
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
        checkerDecision.next_action === "maker" &&
        !makerFitsWorkflowBudget()
      ) {
        return {
          type: "complete",
          output: {
            query,
            mode: "direct_web_degraded",
            plan: researchPlan,
            checker: budgetClosedChecker(checkerDecision),
            research: directEvidence,
            budget_limited: true
          }
        };
      }
      if (
        checkerDecision &&
        checkerDecision.decision === "continue" &&
        checkerDecision.next_action === "maker" &&
        !directRecoveryAfterMakerFailure &&
        nextTracks.length > 0
      ) {
        return scheduleMakerStep(
