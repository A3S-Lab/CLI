          roundStepId("local_research", 1),
          1,
          nextTracks,
          makerEvidenceContext(directEvidence)
        );
      }
      return {
        type: "complete",
        output: {
          query,
          mode: "direct_web_degraded",
          plan: researchPlan,
          checker: checkerDecision,
          research: directEvidence,
          checker_error: "The checker requested continuation without an actionable in-budget retrieval step."
        }
      };
    }

    if (localRounds.length > 0) {
      if (collectOnly) {
        return completeCollectionWave(
          aggregateResearchRounds(
            localRounds,
            localRoundFailures.length > 0
              ? "collection_wave_partial_evidence"
              : "collection_wave_complete",
            localRoundFailures
          ),
          directEvidence,
          "Maker evidence collection completed; question resolution and convergence are host-managed."
        );
      }
      if (!engineeredLoopEnabled || input.engineered_loop_fixture === true) {
        if (shouldScheduleFollowUpRound(localRounds, localRoundFailures)) {
          const nextRound = localRounds.length + 1;
          return scheduleMakerStep(
            roundStepId("local_research", nextRound),
            nextRound,
            followUpTracks(localRounds),
            evidenceSummary(localRounds)
          );
        }
        const aggregate = aggregateResearchRounds(
          localRounds,
          localRoundFailures.length > 0 ? "round_failed_after_partial_evidence" : "bounded_rounds_complete",
          localRoundFailures
        );
        const fixtureOutput = {
          query,
          plan: researchPlan,
          mode: directEvidence ? "hybrid_direct_web_parallel" : "local_parallel_task",
          research: aggregate
        };
        if (directEvidence) {
          fixtureOutput.seed_research = directEvidence;
        }
        return { type: "complete", output: fixtureOutput };
      }
      const latestRound = localRounds.length;
      const roundDirectStepId = roundDirectFollowUpStepId(latestRound);
      const roundDirectResearch = stepOutputs[roundDirectStepId];
      const roundDirectFailure = stepFailures[roundDirectStepId];
      const checkerKind = roundDirectResearch ? "round_follow_up" : "round";
      const checkerId = checkerStepId(checkerKind, latestRound);
      const checkerFailure = stepFailures[checkerId];
      const makerEvidenceForChecker = aggregateResearchRounds(
        localRounds,
        "checking_evidence",
        []
      );
      const checkerEvidenceForDecision = {
        direct: directEvidence,
        maker: makerEvidenceForChecker
      };
      const checkerDecision = validateCheckerDecision(
        structuredTaskOutput(stepOutputs[checkerId]),
        checkerEvidenceForDecision
      );
      const priorRoundChecker = roundDirectResearch
        ? validateCheckerDecision(
            structuredTaskOutput(stepOutputs[checkerStepId("round", latestRound)]),
            checkerEvidenceForDecision
          )
        : null;
      if (
        !checkerDecision &&
        !checkerFailure &&
        roundDirectResearch &&
        priorRoundChecker &&
        !checkerFitsWorkflowBudget()
      ) {
        const aggregate = aggregateResearchRounds(
          localRounds,
          "workflow_budget_reached_after_targeted_retrieval",
          localRoundFailures
        );
        const completedOutput = {
          query,
          plan: researchPlan,
          checker: budgetClosedChecker(
            priorRoundChecker,
            "The targeted follow-up is retained, but the prior checker requested more evidence and another independent checker pass cannot finish; the run remains degraded."
          ),
          mode: directEvidence ? "hybrid_direct_web_parallel_degraded" : "local_parallel_task_degraded",
          research: aggregate,
          budget_limited: true
        };
        if (directEvidence) {
          completedOutput.seed_research = directEvidence;
        }
        return { type: "complete", output: completedOutput };
      }
      if (!checkerDecision && !checkerFailure) {
        const scheduled = scheduleChecker(
          checkerKind,
          latestRound,
          checkerEvidenceForDecision
        );
        if (scheduled) {
          return scheduled;
        }
      }
      const directQueries = checkerDecision && Array.isArray(checkerDecision.search_queries)
        ? checkerDecision.search_queries.filter(isNonEmptyString).slice(0, directWebSearchLimit)
        : [];
      const directUrls = checkerDecision && Array.isArray(checkerDecision.seed_urls)
        ? checkerDecision.seed_urls
            .filter((item) => isNonEmptyString(item) && /^https?:\/\//i.test(item.trim()))
            .slice(0, directWebFetchLimit)
        : [];
      if (
        checkerDecision &&
        checkerDecision.decision === "continue" &&
        checkerDecision.next_action === "direct_retrieval" &&
        directWebEnabled &&
        !retrievalBudgetExhausted &&
        !roundDirectResearch &&
        !roundDirectFailure &&
        localRounds.length < maxResearchRounds &&
        (directQueries.length > 0 || directUrls.length > 0)
      ) {
        return {
          type: "schedule_step",
          step_id: roundDirectStepId,
          step_name: "direct_web_research",
          input: directStepInput(
            directQueries,
            directUrls,
            observedDirectUrls(directEvidence)
          ),
          retry: continueWorkflowRetry,
        };
      }
      const nextTracks = checkerContinuationTracks(checkerDecision);
      const makerContinuationRequested = Boolean(
        checkerDecision &&
        checkerDecision.decision === "continue" &&
        (checkerDecision.next_action === "maker" ||
          (checkerDecision.next_action === "direct_retrieval" && Boolean(roundDirectResearch))) &&
        localRounds.length < maxResearchRounds &&
        nextTracks.length > 0
      );
      if (makerContinuationRequested && !makerFitsWorkflowBudget()) {
        const aggregate = aggregateResearchRounds(
          localRounds,
          "workflow_budget_reached_after_checked_evidence",
          localRoundFailures
        );
        const completedOutput = {
          query,
          plan: researchPlan,
          checker: budgetClosedChecker(checkerDecision),
          mode: directEvidence ? "hybrid_direct_web_parallel_degraded" : "local_parallel_task_degraded",
          research: aggregate,
          budget_limited: true
        };
        if (directEvidence) {
          completedOutput.seed_research = directEvidence;
        }
        return { type: "complete", output: completedOutput };
      }
      if (
        makerContinuationRequested
      ) {
        const nextRound = localRounds.length + 1;
        return scheduleMakerStep(
          roundStepId("local_research", nextRound),
          nextRound,
          nextTracks,
          compactText(JSON.stringify({
            direct: directEvidence,
            maker: evidenceSummary(localRounds)
          }), 4000)
        );
      }
      const aggregate = aggregateResearchRounds(
        localRounds,
        roundDirectFailure
          ? "direct_follow_up_failed_after_partial_evidence"
          : checkerFailure
          ? "checker_failed_after_partial_evidence"
          : (checkerDecision ? `checker_${checkerDecision.decision}` : "bounded_rounds_complete"),
        localRoundFailures
      );
      const completedOutput = {
        query,
        plan: researchPlan,
        checker: checkerDecision,
        mode: checkerDecision && checkerDecision.decision === "degrade"
          ? (directEvidence
              ? "hybrid_direct_web_parallel_degraded"
              : "local_parallel_task_degraded")
          : (directEvidence ? "hybrid_direct_web_parallel" : "local_parallel_task"),
        research: aggregate
      };
      if (directEvidence) {
        completedOutput.seed_research = directEvidence;
      }
      if (roundDirectFailure) {
        completedOutput.retrieval_error = roundDirectFailure.error ||
          "Targeted direct retrieval after a maker round failed.";
      }
      if (checkerFailure) {
        if (roundDirectResearch && priorRoundChecker) {
          completedOutput.checker = failedRecheckFinalizedChecker(priorRoundChecker);
          completedOutput.verification = {
            status: "degraded",
            checker_completed: false,
            prior_checker_retained: true,
            error: checkerFailure.error || "Evidence follow-up checker failed."
          };
        } else {
          delete completedOutput.checker;
          completedOutput.verification = {
            status: "degraded",
            checker_completed: false,
            error: checkerFailure.error || "Evidence checker failed."
          };
        }
      }
      return {
        type: "complete",
        output: completedOutput
      };
    }

    if (localRoundFailures.length > 0) {
      const priorDirectChecker = validateCheckerDecision(
        structuredTaskOutput(stepOutputs[checkerStepId("direct", 0)]),
        directEvidence
      );
      const retainedRounds = collectRetainedFailureRounds(stepFailures, "local_research");
      if (retainedRounds.length > 0) {
        const completedOutput = {
          query,
          plan: researchPlan,
          checker: priorDirectChecker,
          mode: directEvidence
            ? "hybrid_direct_web_parallel"
            : "local_parallel_task_partial_success",
          research: aggregateResearchRounds(
            retainedRounds,
            "source_notes_retained",
            localRoundFailures
          )
        };
        if (directEvidence) {
          completedOutput.seed_research = directEvidence;
        }
        return { type: "complete", output: completedOutput };
      }
      // Recover a failed maker once through a distinct direct-evidence path.
      if (
        directWebSeedEnabled &&
        !directWebResearch &&
        !directWebFailure &&
        !retrievalBudgetExhausted
      ) {
        return {
          type: "schedule_step",
          step_id: "direct_web_research",
          step_name: "direct_web_research",
          input: directStepInput(plannedSearchQueries, plannedSeedUrls),
          retry: continueWorkflowRetry,
        };
      }
      const completedOutput = {
        query,
        plan: researchPlan,
        checker: priorDirectChecker,
        mode: "local_parallel_task_failed",
        research: {
          status: "failed",
          algorithm: "bounded_recursive_parallel_retrieval_summary",
          max_rounds: maxResearchRounds,
          completed_rounds: 0,
          error: localRoundFailures[0].error || "local research step failed",
          note: "Local evidence fan-out failed before producing usable structured evidence; synthesis should create a transparent fallback report instead of retrying the workflow."
        }
      };
      if (directWebResearch) {
        completedOutput.seed_research = directEvidence;
      }
      return {
        type: "complete",
        output: completedOutput
      };
    }

    if ((directWebResearch && !hasStructuredEvidence(directWebResearch)) || directWebFailure) {
      if (collectOnly && executionRoute === "direct_then_review") {
        return completeCollectionWave(
          directWebResearch || {
            status: "failed",
            algorithm: "direct_web_search_fetch",
            error: directWebFailure && directWebFailure.error
              ? directWebFailure.error
              : "Direct retrieval ended before retaining evidence."
          },
          null,
          "Direct retrieval retained no evidence; the host inquiry reducer owns any subsequent wave."
        );
      }
      return scheduleMakerStep(
        roundStepId("local_research", 1),
        1,
        tracks,
        directWebResearch ? makerEvidenceContext(directWebResearch) : ""
      );
    }

    if (directWebSeedEnabled && directWebFirst) {
      return {
        type: "schedule_step",
        step_id: "direct_web_research",
        step_name: "direct_web_research",
        input: directStepInput(plannedSearchQueries, plannedSeedUrls),
        retry: continueWorkflowRetry,
      };
    }

    if (!makerFitsWorkflowBudget()) {
      return {
        type: "complete",
        output: {
          query,
          plan: researchPlan,
          mode: "workflow_budget_exhausted",
          research: {
            status: "failed",
            algorithm: "llm_planned_engineered_loop",
            error: "The planned maker-first route no longer fits inside the workflow wall-clock fuse."
          }
        }
      };
    }

    return scheduleMakerStep(
      roundStepId("local_research", 1),
      1,
      tracks,
      plannedSeedEvidenceContext
    );
  }

  if (
    inputs.kind === "step" &&
    inputs.step_name === "direct_web_research"
  ) {
    return await collectDirectWebResearch();
  }

  if (
    inputs.kind === "step" &&
    inputs.step_name === "generate_object"
  ) {
    const stepStartedAtMs = Date.now();
    const generatedInput = inputs.input || {};
    const generated = await ctx.tool("generate_object", generatedInput);
    if (!generated || Number(generated.exitCode) !== 0) {
      throw new Error(generated && generated.output
        ? generated.output
        : "generate_object returned no schema-valid object");
    }
    const generatedMetadata = generated.metadata && typeof generated.metadata === "object"
      ? generated.metadata
      : {};
    const generatedPrompt = isNonEmptyString(generatedInput.prompt)
      ? generatedInput.prompt
      : "";
    const sourceMarker = "Runtime-observed source anchors (reuse these exact URLs; they were observed before this step):";
    const sourceEndMarker = "End runtime-observed source anchors.";
    const markerAt = generatedPrompt.indexOf(sourceMarker);
    const markerEnd = markerAt >= 0
      ? generatedPrompt.indexOf(sourceEndMarker, markerAt + sourceMarker.length)
      : -1;
    const inheritedSourceUrls = generatedInput.schema_name === "deep_research_evidence" && markerAt >= 0
      ? evidenceSeedUrls(generatedPrompt.slice(
          markerAt + sourceMarker.length,
          markerEnd >= 0 ? markerEnd : generatedPrompt.length
        ))
      : [];
    return {
      tool: "generate_object",
      output: generated.output || "",
      exit_code: Number(generated.exitCode) || 0,
      metadata: Object.assign({}, generatedMetadata, {
        step_elapsed_ms: Math.max(0, Date.now() - stepStartedAtMs),
        inherited_source_urls: inheritedSourceUrls
      })
    };
  }

  return { error: `unknown dynamic workflow invocation: ${inputs.kind}/${inputs.step_name || ""}` };
  };

  const result = await executeWorkflow();
  const requestedMode = inputs && inputs.input && inputs.input.execution_mode;
  if (
    requestedMode !== "collect_only" ||
    !result ||
    result.type !== "complete"
  ) {
    return result;
  }

  const output = result.output && typeof result.output === "object" && !Array.isArray(result.output)
    ? result.output
    : { research: result.output === undefined ? null : result.output };
  const collectionOutcome = typeof output.mode === "string" && output.mode !== "inquiry_collection_wave"
    ? output.mode
    : null;
  const priorExecution = output.execution && typeof output.execution === "object" && !Array.isArray(output.execution)
    ? output.execution
    : {};
  const execution = Object.assign({}, priorExecution, {
    mode: "collect_only",
    terminal_authority: "host_inquiry_reducer"
  });
  if (collectionOutcome) {
    execution.collection_outcome = collectionOutcome;
  }
  if (typeof execution.note !== "string" || !execution.note.trim()) {
    execution.note = "The workflow completed one bounded evidence-collection wave; question resolution and convergence are host-managed.";
  }
  return Object.assign({}, result, {
    output: Object.assign({}, output, {
      mode: "inquiry_collection_wave",
      execution
    })
  });
}
