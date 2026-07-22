  const reducedSelectorPacket = (
    packet,
    shards,
    outputs,
    failures,
    shardStepPrefix
  ) => {
    if (!packet || shards.length === 0) {
      return {
        packet: null,
        candidate_count: 0,
        error: "Semantic selection produced no complete shard catalog.",
      };
    }
    const selectedIds = new Set();
    const sourceCoverage = [];
    const sourceRelevance = [];
    const shardErrors = [];
    let failedShardCount = 0;
    for (let index = 0; index < shards.length; index += 1) {
      const stepId = `${shardStepPrefix || STEP_SELECT_SHARD_PREFIX}${index + 1}`;
      if (failures[stepId]) {
        failedShardCount += 1;
        shardErrors.push(
          failures[stepId].error ||
            `semantic evidence shard ${index + 1} failed`
        );
        continue;
      }
      const selection = structuredOutput(outputs[stepId]);
      if (!selection || !Array.isArray(selection.chunk_ids)) {
        failedShardCount += 1;
        shardErrors.push(
          `Semantic evidence shard ${index + 1} returned no valid chunk ID list.`
        );
        continue;
      }
      if (selection.chunk_ids.length > MAX_SELECTOR_SHARD_CANDIDATES) {
        failedShardCount += 1;
        shardErrors.push(
          `Semantic evidence shard ${index + 1} exceeded its closed candidate limit.`
        );
        continue;
      }
      const allowedIds = new Set(
        shards[index].sources.flatMap((source) =>
          source.chunks.map((chunk) => chunk.chunk_id)
        )
      );
      const localIds = new Set();
      let invalidChunkCatalog = false;
      for (const chunkId of selection.chunk_ids) {
        if (
          typeof chunkId !== "string" ||
          !allowedIds.has(chunkId) ||
          localIds.has(chunkId)
        ) {
          invalidChunkCatalog = true;
          break;
        }
        localIds.add(chunkId);
      }
      if (invalidChunkCatalog) {
        failedShardCount += 1;
        shardErrors.push(
          `Semantic evidence shard ${index + 1} violated its closed chunk catalog.`
        );
        continue;
      }
      const shardCoverage = validatedSourceCoverage(
        shards[index],
        selection,
        localIds
      );
      if (shardCoverage.error) {
        failedShardCount += 1;
        shardErrors.push(
          `Semantic evidence shard ${index + 1} returned invalid typed coverage: ${shardCoverage.error}`
        );
        continue;
      }
      const shardRelevance = validatedSourceRelevance(
        shards[index],
        selection,
        localIds
      );
      if (shardRelevance.error) {
        failedShardCount += 1;
        shardErrors.push(
          `Semantic evidence shard ${index + 1} returned invalid typed relevance: ${shardRelevance.error}`
        );
        continue;
      }
      localIds.forEach((chunkId) => selectedIds.add(chunkId));
      sourceCoverage.push(...shardCoverage.bindings);
      sourceRelevance.push(...shardRelevance.bindings);
    }
    if (selectedIds.size === 0) {
      return {
        packet: null,
        candidate_count: 0,
        failed_shard_count: failedShardCount,
        error: uniqueStrings([
          ...shardErrors,
          "Semantic evidence shards retained no candidate chunk.",
        ]).join(" "),
      };
    }
    const sources = packet.sources.map((source) => {
      const chunks = source.chunks.filter((chunk) =>
        selectedIds.has(chunk.chunk_id)
      );
      return chunks.length > 0
        ? Object.assign({}, source, { chunks })
        : null;
    }).filter(Boolean);
    return {
      packet: {
        version: packet.version,
        focuses: packet.focuses,
        sources,
      },
      candidate_count: selectedIds.size,
      source_coverage: mergeSourceCoverage(sourceCoverage),
      source_relevance: mergeSourceRelevance(sourceRelevance),
      failed_shard_count: failedShardCount,
      error: uniqueStrings(shardErrors).join(" "),
    };
  };
