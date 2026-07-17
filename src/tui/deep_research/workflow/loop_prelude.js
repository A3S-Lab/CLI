  const hasStructuredEvidence = (runtimeOutput) =>
    runtimeOutput &&
    Array.isArray(runtimeOutput.results) &&
    runtimeOutput.results.some((item) => item && item.success === true && item.structured);
  const hasCandidateLeads = (runtimeOutput) =>
    runtimeOutput &&
    runtimeOutput.metadata &&
    Array.isArray(runtimeOutput.metadata.candidate_leads) &&
    runtimeOutput.metadata.candidate_leads.length > 0;
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
