  const materializeEvidence = (packet, selector, errors, metadata) => {
    const boundedErrors = uniqueStrings(errors).slice(0, 16);
    if (!packet || !selector || !Array.isArray(selector.chunk_ids)) {
      return {
        status: "failed",
        results: [],
        errors: uniqueStrings([
          ...boundedErrors,
          "Retrieved text was not promoted because semantic chunk selection did not complete.",
        ]),
        metadata: Object.assign({}, metadata, {
          evidence_selection_mode: "semantic_chunk_ids",
          source_count: 0,
          selection_count: 0,
        }),
      };
    }
    const chunkById = new Map();
    for (const source of packet.sources) {
      for (const chunk of source.chunks) {
        chunkById.set(chunk.chunk_id, { source, chunk });
      }
    }
    const selectionFocus = bounded(
      packet.focuses
        .map((focus) => focus && focus.focus)
        .filter(nonEmpty)
        .join(" | "),
      300
    ) || "Research plan evidence";
    const selectedBySource = new Map();
    const seenChunkIds = new Set();
    let invalidSelection = "";
    for (const chunkId of selector.chunk_ids) {
      const selected = chunkById.get(chunkId);
      if (!selected) {
        invalidSelection =
          "Semantic chunk selection returned an ID outside the closed catalog.";
        break;
      }
      const { source, chunk } = selected;
      const retained = selectedBySource.get(source.source_id) || [];
      const retainedChars = retained.reduce(
        (total, item) => total + Array.from(item.quote_or_fact).length,
        0
      );
      if (
        seenChunkIds.has(chunkId) ||
        retained.length >= MAX_EXCERPTS_PER_SOURCE ||
        retainedChars >= MAX_EXCERPT_CHARS_PER_SOURCE
      ) {
        invalidSelection =
          "Semantic chunk selection violated the closed selection limits.";
        break;
      }
      const quote = String(chunk.text || "");
      if (
        !quote ||
        Array.from(quote).length > MAX_CHUNK_CHARS ||
        retainedChars + Array.from(quote).length >
          MAX_EXCERPT_CHARS_PER_SOURCE
      ) {
        invalidSelection =
          "Semantic chunk selection referenced an invalid bounded chunk.";
        break;
      }
      seenChunkIds.add(chunkId);
      retained.push({
        focus: selectionFocus,
        quote_or_fact: quote,
      });
      selectedBySource.set(source.source_id, retained);
    }
    if (invalidSelection) {
      return {
        status: "failed",
        results: [],
        errors: uniqueStrings([...boundedErrors, invalidSelection]),
        metadata: Object.assign({}, metadata, {
          evidence_selection_mode: "semantic_chunk_ids",
          source_count: 0,
          selection_count: 0,
        }),
      };
    }
    const sourceCoverage = validatedSourceCoverage(
      packet,
      selector,
      seenChunkIds
    );
    if (sourceCoverage.error) {
      return {
        status: "failed",
        results: [],
        errors: uniqueStrings([...boundedErrors, sourceCoverage.error]),
        metadata: Object.assign({}, metadata, {
          evidence_selection_mode: "semantic_chunk_ids_with_typed_coverage",
          source_count: 0,
          selection_count: 0,
          source_coverage_count: 0,
        }),
      };
    }
    const sourceRelevance = validatedSourceRelevance(
      packet,
      selector,
      seenChunkIds
    );
    if (sourceRelevance.error) {
      return {
        status: "failed",
        results: [],
        errors: uniqueStrings([...boundedErrors, sourceRelevance.error]),
        metadata: Object.assign({}, metadata, {
          evidence_selection_mode: "semantic_chunk_ids_with_typed_relevance",
          source_count: 0,
          selection_count: 0,
          source_coverage_count: 0,
          source_relevance_count: 0,
        }),
      };
    }
    const durableSourceCoverage = sourceCoverage.bindings.map((binding) => {
      const roles = object(binding.roles);
      return Object.assign({}, binding, {
        roles: ["supporting", "primary", "independent"].filter((role) =>
          roles[role] === true
        ),
      });
    });
    const sources = packet.sources.map((source) => {
      const excerpts = selectedBySource.get(source.source_id) || [];
      if (excerpts.length === 0) {
        return null;
      }
      return {
        source_id: source.source_id,
        title: source.title,
        url_or_path: source.url_or_path,
        quote_or_fact: excerpts[0].quote_or_fact,
        evidence_excerpts: excerpts,
        date: source.date,
        reliability: source.reliability,
      };
    }).filter(Boolean);
    if (sources.length === 0) {
      return {
        status: "failed",
        results: [],
        errors: uniqueStrings([
          ...boundedErrors,
          "Semantic chunk selection retained no source text.",
        ]),
        metadata: Object.assign({}, metadata, {
          evidence_selection_mode: "semantic_chunk_ids",
          source_count: 0,
          selection_count: 0,
        }),
      };
    }
    const facts = sources.flatMap((source) =>
      source.evidence_excerpts.map((excerpt) => excerpt.quote_or_fact)
    );
    const results = sources.map((source) => {
      const sourceFacts = uniqueStrings(
        source.evidence_excerpts.map((excerpt) => excerpt.quote_or_fact)
      );
      return {
        task_id: `evidence_retrieval:${source.source_id}`,
        agent: "workflow",
        success: true,
        structured: {
          summary: `Semantic selection retained ${sourceFacts.length} fetched evidence chunk(s) from one source.`,
          sources: [source],
          source_coverage: durableSourceCoverage.filter((binding) =>
            binding.source_id === source.source_id
          ),
          relevant_obligation_ids: sourceRelevance.bindings
            .filter((binding) => binding.source_id === source.source_id)
            .map((binding) => binding.obligation_id),
          key_evidence: sourceFacts,
          contradictions: [],
          confidence: "Closed-evidence review required; source text was restored from the closed catalog by semantic chunk ID.",
          gaps: [],
        },
      };
    });
    return {
      status: boundedErrors.length > 0 ? "partial" : "success",
      results,
      errors: boundedErrors,
      metadata: Object.assign({}, metadata, {
        evidence_selection_mode: "semantic_chunk_ids_with_typed_coverage",
        source_count: sources.length,
        selection_count: facts.length,
        source_coverage_count: durableSourceCoverage.length,
        source_relevance_count: sourceRelevance.bindings.length,
      }),
    };
  };
