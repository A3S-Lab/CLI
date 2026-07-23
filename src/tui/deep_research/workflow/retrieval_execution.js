  if (inputs.kind === "step") {
    if (inputs.step_name === STEP_DISCOVER_WEB) {
      return await discoverWeb(object(inputs.input));
    }
    if (
      inputs.step_name === STEP_WEB ||
      inputs.step_name === STEP_SUPPLEMENTAL_WEB
    ) {
      return await collectWeb(object(inputs.input));
    }
    if (inputs.step_name === STEP_LOCAL) {
      return await collectLocal(object(inputs.input));
    }
    if (inputs.step_name === STEP_CHECKPOINT_BOOTSTRAP) return object(inputs.input);
    if (inputs.step_name === STEP_CHECKPOINT_INITIAL) return object(inputs.input);
    if (inputs.step_name === "generate_object") {
      const result = await ctx.tool("generate_object", object(inputs.input));
      const exitCode = toolExitCode(result);
      if (exitCode !== 0) {
        const diagnostic = bounded(result && result.output, 600) ||
          "generate_object returned no diagnostic";
        const stage = inputs.step_id === STEP_SELECT_WEB
          ? "Semantic web source selection"
          : "Semantic chunk selection";
        throw new Error(
          `${stage} failed with exit code ${exitCode}: ${diagnostic}`
        );
      }
      return result;
    }
    return { error: `unknown retrieval step: ${String(inputs.step_name || "")}` };
  }
  if (inputs.kind !== "workflow") {
    return { error: `unknown DeepResearch retrieval invocation: ${String(inputs.kind || "")}` };
  }

  const input = object(inputs.input);
  const plan = object(input.research_plan);
  const query = String(input.query || "");
  const executionMode = String(input.execution_mode || "collect_only");
  const scope = input.evidence_scope === "local_only"
    ? "local_only"
    : "web_and_workspace";
  const needsWeb = scope === "web_and_workspace";
  const needsLocal = scope === "local_only" ||
    plan.workspace_evidence_required === true;
  const outputs = object(inputs.step_outputs);
  const failures = object(inputs.step_failures);
  const retrievalRetry = {
    max_attempts: 1,
    delay_ms: 0,
    on_exhausted: "continue_workflow",
  };
  const semanticSelectionRetry = {
    max_attempts: 2,
    delay_ms: 100,
    on_exhausted: "continue_workflow",
  };
  const webSourceSelectionRetry = {
    max_attempts: 2,
    delay_ms: 100,
    on_exhausted: "continue_workflow",
  };
  const bootstrapWebSourceSelectionRetry = {
    max_attempts: 1,
    delay_ms: 0,
    on_exhausted: "continue_workflow",
  };
  const semanticShardSelectionRetry = {
    max_attempts: 1,
    delay_ms: 0,
    on_exhausted: "continue_workflow",
  };

  if (!nonEmpty(query) || Object.keys(plan).length === 0) {
    return {
      type: "fail",
      error: "host-managed DeepResearch retrieval requires a query and validated research plan",
    };
  }
  if (executionMode === "bootstrap_acquisition") {
    if (
      needsWeb &&
      !outputs[STEP_DISCOVER_WEB] &&
      !failures[STEP_DISCOVER_WEB]
    ) {
      return {
        type: "schedule_step",
        step_id: STEP_DISCOVER_WEB,
        step_name: STEP_DISCOVER_WEB,
        input: {
          query,
          plan,
          search_timeout_secs: 12,
        },
        retry: retrievalRetry,
      };
    }
    const bootstrapDiscovery = needsWeb
      ? (outputs[STEP_DISCOVER_WEB] || {
          status: "failed",
          candidates: [],
          errors: [
            failures[STEP_DISCOVER_WEB] && failures[STEP_DISCOVER_WEB].error ||
              "bootstrap web discovery failed",
          ],
          metadata: {},
        })
      : null;
    const bootstrapCandidates = Array.isArray(
      bootstrapDiscovery && bootstrapDiscovery.candidates
    ) ? bootstrapDiscovery.candidates : [];
    const bootstrapDeterministicSelection = needsWeb
      ? deterministicOutcomeWebCandidates(query, plan, bootstrapDiscovery)
      : null;
    const bootstrapNeedsSelection = needsWeb && bootstrapCandidates.length > 0 &&
      !bootstrapDeterministicSelection;
    if (
      bootstrapNeedsSelection &&
      !outputs[STEP_SELECT_WEB] &&
      !failures[STEP_SELECT_WEB]
    ) {
      return {
        type: "schedule_step",
        step_id: STEP_SELECT_WEB,
        step_name: "generate_object",
        input: webSourceSelectorInput(plan, bootstrapDiscovery),
        retry: bootstrapWebSourceSelectionRetry,
      };
    }
    const bootstrapSelectorFailure = failures[STEP_SELECT_WEB] &&
      (failures[STEP_SELECT_WEB].error ||
        "bootstrap semantic web source admission failed");
    const bootstrapSelection = needsWeb
      ? (bootstrapDeterministicSelection || selectedWebCandidates(
          plan,
          bootstrapDiscovery,
          structuredOutput(outputs[STEP_SELECT_WEB]),
          bootstrapSelectorFailure
        ))
      : { candidates: [], mode: "none", error: "" };
    if (
      needsWeb &&
      bootstrapSelection.candidates.length > 0 &&
      !outputs[STEP_WEB] &&
      !failures[STEP_WEB]
    ) {
      return {
        type: "schedule_step",
        step_id: STEP_WEB,
        step_name: STEP_WEB,
        input: {
          plan,
          candidates: bootstrapSelection.candidates,
          discovery_errors: uniqueStrings([
            ...(Array.isArray(bootstrapDiscovery.errors)
              ? bootstrapDiscovery.errors
              : []),
            bootstrapSelectorFailure || "",
            bootstrapSelection.error || "",
          ]),
          discovery_metadata: object(bootstrapDiscovery.metadata),
          source_selection_mode: bootstrapSelection.mode,
          source_id_prefix: "bootstrap-web-source",
          fetch_timeout_secs: 20,
        },
        retry: retrievalRetry,
      };
    }
    if (needsLocal && !outputs[STEP_LOCAL] && !failures[STEP_LOCAL]) {
      return {
        type: "schedule_step",
        step_id: STEP_LOCAL,
        step_name: STEP_LOCAL,
        input: {
          query,
          plan,
          max_steps: input.local_max_steps,
        },
        retry: retrievalRetry,
      };
    }
    const bootstrapWeb = needsWeb
      ? (outputs[STEP_WEB] || {
          status: "failed",
          packet: null,
          errors: uniqueStrings([
            ...(Array.isArray(bootstrapDiscovery.errors)
              ? bootstrapDiscovery.errors
              : []),
            bootstrapSelectorFailure || "",
            bootstrapSelection.error || "",
            failures[STEP_WEB] && failures[STEP_WEB].error ||
              "bootstrap web retrieval did not complete",
          ]),
          metadata: Object.assign({}, object(bootstrapDiscovery.metadata), {
            source_selection_mode: bootstrapSelection.mode,
            selected_candidate_count: 0,
            fetched_count: 0,
          }),
        })
      : null;
    const bootstrapLocal = needsLocal
      ? (outputs[STEP_LOCAL] || {
          status: "failed",
          packet: null,
          errors: [
            failures[STEP_LOCAL] && failures[STEP_LOCAL].error ||
              "bootstrap local retrieval did not complete",
          ],
          metadata: {},
        })
      : null;
    const bootstrapAdmission = combinedEvidencePacket(
      plan,
      [bootstrapWeb, bootstrapLocal]
    );
    const bootstrapErrors = uniqueStrings([
      ...(Array.isArray(bootstrapWeb && bootstrapWeb.errors)
        ? bootstrapWeb.errors
        : []),
      ...(Array.isArray(bootstrapLocal && bootstrapLocal.errors)
        ? bootstrapLocal.errors
        : []),
      bootstrapAdmission.error || "",
    ]);
    const bootstrapOutput = {
      query,
      mode: "bootstrap_acquisition",
      acquisition: {
        status: bootstrapAdmission.packet
          ? (bootstrapErrors.length > 0 ? "partial" : "success")
          : "failed",
        packet: bootstrapAdmission.packet,
        errors: bootstrapErrors,
        metadata: {
          source_selection_mode: bootstrapSelection.mode,
          source_count: bootstrapAdmission.source_count,
          chunk_count: bootstrapAdmission.chunk_count,
          web: bootstrapWeb ? object(bootstrapWeb.metadata) : undefined,
          local: bootstrapLocal ? object(bootstrapLocal.metadata) : undefined,
        },
      },
      execution: {
        mode: "acquire_only",
        terminal_authority: "host_inquiry_reducer",
        note: "Raw sources were durably acquired before semantic planning settled.",
      },
    };
    if (
      !outputs[STEP_CHECKPOINT_BOOTSTRAP] &&
      !failures[STEP_CHECKPOINT_BOOTSTRAP]
    ) {
      return {
        type: "schedule_step",
        step_id: STEP_CHECKPOINT_BOOTSTRAP,
        step_name: STEP_CHECKPOINT_BOOTSTRAP,
        input: bootstrapOutput,
        retry: retrievalRetry,
      };
    }
    return {
      type: "complete",
      output: outputs[STEP_CHECKPOINT_BOOTSTRAP] || bootstrapOutput,
    };
  }
  const bootstrapAcquisition = object(input.bootstrap_acquisition);
  const bootstrapPacket = object(bootstrapAcquisition.packet);
  const hasBootstrapWeb = needsWeb &&
    Array.isArray(bootstrapPacket.sources) &&
    bootstrapPacket.sources.length > 0;
  if (
    needsWeb &&
    !hasBootstrapWeb &&
    !outputs[STEP_DISCOVER_WEB] &&
    !failures[STEP_DISCOVER_WEB]
  ) {
    return {
      type: "schedule_step",
      step_id: STEP_DISCOVER_WEB,
      step_name: STEP_DISCOVER_WEB,
      input: {
        query,
        plan,
        search_timeout_secs: 12,
      },
      retry: retrievalRetry,
    };
  }
  const webDiscovery = needsWeb && !hasBootstrapWeb
    ? (outputs[STEP_DISCOVER_WEB] || {
        status: "failed",
        candidates: [],
        errors: [
          failures[STEP_DISCOVER_WEB] && failures[STEP_DISCOVER_WEB].error ||
            "web discovery failed",
        ],
        metadata: {},
      })
    : null;
  const discoveryCandidatesList = Array.isArray(
    webDiscovery && webDiscovery.candidates
  )
    ? webDiscovery.candidates
    : [];
  const fetchLimit = clamp(
    object(plan.budget).direct_fetches,
    0,
    MAX_SOURCES,
    4
  );
  const deterministicWebSelection = needsWeb && !hasBootstrapWeb
    ? deterministicOutcomeWebCandidates(query, plan, webDiscovery)
    : null;
  const needsWebSourceSelection =
    needsWeb && !hasBootstrapWeb && fetchLimit > 0 &&
    discoveryCandidatesList.length > 0 && !deterministicWebSelection;
  if (
    needsWebSourceSelection &&
    !outputs[STEP_SELECT_WEB] &&
    !failures[STEP_SELECT_WEB]
  ) {
    return {
      type: "schedule_step",
      step_id: STEP_SELECT_WEB,
      step_name: "generate_object",
      input: webSourceSelectorInput(plan, webDiscovery),
      retry: webSourceSelectionRetry,
    };
  }
  const webSourceSelectorFailure = failures[STEP_SELECT_WEB] &&
    (failures[STEP_SELECT_WEB].error ||
      "semantic web source selection failed");
  const webSourceSelection = needsWeb && !hasBootstrapWeb
    ? (deterministicWebSelection || selectedWebCandidates(
        plan,
        webDiscovery,
        structuredOutput(outputs[STEP_SELECT_WEB]),
        webSourceSelectorFailure
      ))
    : { candidates: [], mode: hasBootstrapWeb ? "bootstrap_packet" : "none", error: "" };
  if (
    needsWeb &&
    !hasBootstrapWeb &&
    webSourceSelection.candidates.length > 0 &&
    !outputs[STEP_WEB] &&
    !failures[STEP_WEB]
  ) {
    return {
      type: "schedule_step",
      step_id: STEP_WEB,
      step_name: STEP_WEB,
      input: {
        plan,
        candidates: webSourceSelection.candidates,
        discovery_errors: uniqueStrings([
          ...(Array.isArray(webDiscovery.errors) ? webDiscovery.errors : []),
          webSourceSelectorFailure || "",
          webSourceSelection.error || "",
        ]),
        discovery_metadata: object(webDiscovery.metadata),
        source_selection_mode: webSourceSelection.mode,
        fetch_timeout_secs: 20,
      },
      retry: retrievalRetry,
    };
  }
  if (needsLocal && !outputs[STEP_LOCAL] && !failures[STEP_LOCAL]) {
    return {
      type: "schedule_step",
      step_id: STEP_LOCAL,
      step_name: STEP_LOCAL,
      input: {
        query,
        plan,
        max_steps: input.local_max_steps,
      },
      retry: retrievalRetry,
    };
  }

  const webRetrieval = needsWeb
    ? (hasBootstrapWeb
      ? {
          status: String(bootstrapAcquisition.status || "partial"),
          packet: bootstrapPacket,
          errors: Array.isArray(bootstrapAcquisition.errors)
            ? bootstrapAcquisition.errors
            : [],
          metadata: Object.assign({}, object(bootstrapAcquisition.metadata), {
            source_selection_mode: "bootstrap_packet",
            bootstrap_source_count: bootstrapPacket.sources.length,
          }),
        }
      : (outputs[STEP_WEB] || {
        status: "failed",
        packet: null,
        errors: uniqueStrings([
          ...(Array.isArray(webDiscovery.errors) ? webDiscovery.errors : []),
          webSourceSelectorFailure || "",
          webSourceSelection.error || "",
          failures[STEP_WEB] && failures[STEP_WEB].error ||
            "web retrieval did not complete",
        ]),
        metadata: Object.assign({}, object(webDiscovery.metadata), {
          source_selection_mode: webSourceSelection.mode,
          selected_candidate_count: 0,
          fetched_count: 0,
        }),
      }))
    : null;
  const localRetrieval = needsLocal
    ? (outputs[STEP_LOCAL] || {
        status: "failed",
        packet: null,
        errors: [
          failures[STEP_LOCAL] && failures[STEP_LOCAL].error ||
            "local retrieval failed",
        ],
        metadata: {},
      })
    : null;
  const admission = combinedEvidencePacket(
    plan,
    [webRetrieval, localRetrieval]
  );
  const packet = admission.packet;
  const selectorShards = packet ? selectorShardPackets(packet) : [];
  const usesSelectorShards =
    packet && admission.chunk_count > MAX_DIRECT_SELECTOR_CHUNKS;
  if (usesSelectorShards) {
    const pendingShardSteps = selectorShards
      .map((shard, index) => ({
        step_id: `${STEP_SELECT_SHARD_PREFIX}${index + 1}`,
        step_name: "generate_object",
        input: selectorInput(shard, { shard: true }),
        retry: semanticShardSelectionRetry,
      }))
      .filter((step) =>
        !outputs[step.step_id] && !failures[step.step_id]
      );
    if (pendingShardSteps.length > 0) {
      return {
        type: "schedule_steps",
        steps: pendingShardSteps,
      };
    }
  }
  const shardReduction = usesSelectorShards
    ? reducedSelectorPacket(packet, selectorShards, outputs, failures)
    : {
        packet,
        candidate_count: admission.chunk_count,
        error: "",
      };
  const sourceReductionPackets = usesSelectorShards
    ? selectorSourceReductionPackets(shardReduction.packet)
    : [];
  if (usesSelectorShards && shardReduction.packet) {
    const pendingSourceSteps = sourceReductionPackets
      .map((reduction, index) => ({
        step_id: `${STEP_SELECT_SOURCE_PREFIX}${index + 1}`,
        step_name: "generate_object",
        input: selectorInput(reduction.packet, {
          source_reduction: true,
        }),
        retry: semanticSelectionRetry,
      }))
      .filter((step) =>
        !outputs[step.step_id] && !failures[step.step_id]
      );
    if (pendingSourceSteps.length > 0) {
      return {
        type: "schedule_steps",
        steps: pendingSourceSteps,
      };
    }
  }
  const sourceReduction = usesSelectorShards
    ? reducedSourcePacket(
        shardReduction.packet,
        sourceReductionPackets,
        outputs,
        failures,
        shardReduction.source_coverage,
        shardReduction.source_relevance,
        STEP_SELECT_SOURCE_PREFIX
      )
    : shardReduction;
  if (
    !usesSelectorShards &&
    sourceReduction.packet &&
    !outputs[STEP_SELECT] &&
    !failures[STEP_SELECT]
  ) {
    return {
      type: "schedule_step",
      step_id: STEP_SELECT,
      step_name: "generate_object",
      input: selectorInput(sourceReduction.packet, { shard: false }),
      retry: semanticSelectionRetry,
    };
  }
  const selectorFailure = !usesSelectorShards && failures[STEP_SELECT] &&
    (failures[STEP_SELECT].error || "semantic chunk selection failed");
  const retrievalErrors = uniqueStrings([
    ...(Array.isArray(webRetrieval && webRetrieval.errors)
      ? webRetrieval.errors
      : []),
    ...(Array.isArray(localRetrieval && localRetrieval.errors)
      ? localRetrieval.errors
      : []),
    admission.error || "",
    shardReduction.error || "",
    sourceReduction.error || "",
    selectorFailure || "",
  ]);
  const semanticSelection = usesSelectorShards && sourceReduction.packet
      ? {
        chunk_ids: sourceReduction.packet.sources.flatMap((source) =>
          source.chunks.map((chunk) => chunk.chunk_id)
        ),
        source_coverage: sourceReduction.source_coverage,
        source_relevance: sourceReduction.source_relevance,
      }
    : structuredOutput(outputs[STEP_SELECT]);
  const supplementalRound = supplementalCoverageRound({
    plan,
    needs_web: needsWeb,
    web_discovery: webDiscovery,
    initial_candidates: webSourceSelection.candidates,
    packet,
    semantic_selection: semanticSelection,
    outputs,
    failures,
    retrieval_retry: retrievalRetry,
    semantic_web_selection_retry: webSourceSelectionRetry,
    semantic_selection_retry: semanticSelectionRetry,
    semantic_shard_selection_retry: semanticShardSelectionRetry,
  });
  const primarySelection = materializeEvidence(
    packet,
    semanticSelection,
    retrievalErrors,
    {
      catalog_source_count: admission.source_count,
      catalog_chunk_count: admission.chunk_count,
      semantic_selection_shard_count: usesSelectorShards
        ? selectorShards.length
        : 1,
      semantic_selection_candidate_count: shardReduction.candidate_count,
      semantic_selection_failed_shard_count:
        shardReduction.failed_shard_count || 0,
      semantic_selection_source_reduction_count: sourceReductionPackets.length,
      semantic_selection_failed_source_reduction_count:
        sourceReduction.failed_source_reduction_count || 0,
      semantic_selection_materialized_count: sourceReduction.candidate_count,
      bootstrap_source_count: hasBootstrapWeb
        ? bootstrapPacket.sources.length
        : 0,
      web: webRetrieval ? object(webRetrieval.metadata) : undefined,
      local: localRetrieval ? object(localRetrieval.metadata) : undefined,
    }
  );
  primarySelection.metadata = Object.assign(
    {},
    object(primarySelection.metadata),
    {
      retrieval_pass_count: 1,
      typed_coverage_gap_count: supplementalRound.coverage_gaps.length,
    }
  );
  const initialCheckpointOutput = initialRetrievalCheckpointOutput(
    query,
    plan,
    primarySelection
  );
  if (
    !outputs[STEP_CHECKPOINT_INITIAL] &&
    !failures[STEP_CHECKPOINT_INITIAL]
  ) {
    return {
      type: "schedule_step",
      step_id: STEP_CHECKPOINT_INITIAL,
      step_name: STEP_CHECKPOINT_INITIAL,
      input: initialCheckpointOutput,
      retry: retrievalRetry,
    };
  }
  if (supplementalRound.schedule) {
    return supplementalRound.schedule;
  }
  const selection = combineMaterializedSelections(
    primarySelection,
    supplementalRound.selection
  );
  selection.metadata = Object.assign({}, object(selection.metadata), {
    typed_coverage_gap_count: supplementalRound.coverage_gaps.length,
    supplemental_retrieval_attempted: supplementalRound.attempted,
  });
  const research = researchResult(selection);
  return {
    type: "complete",
    output: {
      query,
      mode: "inquiry_collection",
      plan,
      research,
      execution: {
        mode: "collect_only",
        terminal_authority: "host_inquiry_reducer",
        note: supplementalRound.attempted
          ? "The host-planned retrieval pass and one typed-coverage supplemental pass completed. Closed-evidence review and convergence remain host-owned."
          : "The host-planned retrieval pass completed without a runnable typed-coverage supplement. Closed-evidence review and convergence remain host-owned.",
      },
    },
  };
}
