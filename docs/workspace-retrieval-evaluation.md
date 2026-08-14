# Workspace Retrieval ACL-host Evaluation

Status: Passed on 2026-08-15 with `deepseek/deepseek-v4-pro` and A3S Code
`bdb86e17`.

This evaluation exercises the real `a3s code exec` boundary, effective ACL
layering, the shared manifest backend, asynchronous in-memory indexing, hybrid
search, deterministic reranking, JSONL projection, and the real DeepSeek
tool/completion loop. It complements the lower-level A3S Code quality and
latency suites; it does not replace them.

## Ownership under test

The CLI host owns the workspace manifest and chunk catalog. A Code session owns
the semantic runtime and vector index. Catalog configuration must therefore be
applied once to the host backend and must not also appear in per-session
retrieval options.

```text
trusted effective ACL
  |
  +-- chunk strategy ------> host ManifestWorkspaceBackend catalog (one-shot)
  |
  +-- provider/index/rerank -> Code session runtime -> in-memory vector index
```

`code exec`, the TUI, and Code Web use this same split. TUI model/effort
rebuilds and Web sessions reuse the host catalog, while every active Code
session retains an isolated semantic index and closes it through the normal
session lifecycle.

## First-principles adversarial plan

The test begins with assets and invariants, then chooses an independent
observable. Model prose alone is never treated as retrieval proof.

| Asset | Invariant | Adversarial condition | Observable and gate |
| --- | --- | --- | --- |
| ACL authority | Only a trusted layer can enable embedding egress | The discovered repository ACL supplies the chat route, while a temporary trusted user ACL supplies retrieval | Effective config has exactly two layers, DeepSeek remains the chat model, retrieval is enabled only by the trusted layer, and secrets/endpoints are redacted |
| Catalog ownership | One component configures chunking exactly once | Host-supplied workspace services are combined with explicit recursive chunking | The host catalog contains recursive 512/64; session options contain no catalog configuration; session construction succeeds instead of triggering Core's ownership guard |
| Source boundary | Non-text assets never enter text chunking or embedding | PDF, PPTX, and MP3 sentinels sit beside 30 admitted Rust files | Eligible/indexed files remain 30, non-text provider inputs remain zero, and failed files remain zero |
| Ranking correctness | Relevance is judged independently from DeepSeek | Each query has a lexical trap and a separately labeled semantic answer | A deterministic local embedding oracle records inputs; the expected path must be in Top 5 and the exact answer identifier must be emitted |
| Tool protocol | The model cannot hide extra exploration | The prompt requires exactly one bounded hybrid search | JSONL contains one successful `search` call with the exact query, path, include, mode, and limit, and no other tool call |
| Rerank selection | Requested policy and applied algorithm agree | Typed ACL enables the bounded deterministic reranker | Tool metadata reports requested/applied `deterministic` and `rrf_k60+deterministic_mmr_v1` |
| Resource bounds | Index state is measurable and bounded | Every task creates a fresh `code exec` session | Status reaches `ready`, coverage is 100%, chunks/vectors/bytes are exact, Core batching counters equal independent provider counters, and request amplification is at most 1.10x the three-limit lower bound |
| Lifecycle | Headless execution closes its session | Three independent subprocess tasks rebuild and terminate | Every command returns successfully after `session.close()`; Core lifecycle tests remain the weak-reference and zero-retained-vector authority |

The first live run reached the production ownership guard and
found that CLI sessions carried chunk configuration while the CLI also supplied
host workspace services. The fix introduced a host wrapper that separates the
catalog strategy from session runtime options and configures the shared backend
before services attach. This was an architecture defect that configuration-only
tests could not expose.

## Fixture and model separation

The ignored integration test is
`tests/workspace_retrieval_real_deepseek.rs`. It creates its workspace below
the repository root so normal ancestor discovery selects the repository
`.a3s/config.acl`. The test never copies or prints that file. It creates a
temporary home containing only:

- an OpenAI-compatible loopback embedding provider;
- explicit retrieval and source-egress gates;
- recursive 512-byte chunks with 64-byte overlap and explicit separators;
- the typed deterministic reranker.

The local embedding oracle makes ranking deterministic. DeepSeek remains the
real chat/tool model and must inspect the search schema, issue the exact tool
call, consume the returned evidence, and produce the labeled identifier. The
three tasks cover reconnect replay suppression, CJK session-projection cleanup,
and an answer beyond a recursive chunk boundary. The corpus contains 30 text
files, 39 expected chunks, and three non-text assets.

## Results

All three tasks passed the final serial run.

| Quality metric | Result |
| --- | ---: |
| Exact task completion | 3/3 (1.0000) |
| Exact tool protocol | 3/3 (1.0000) |
| Precision@5 | 0.2000 |
| Precision among returned results | 3/7 (0.4286) |
| Mean returned results | 2.3333 |
| Recall@5 | 1.0000 |
| Mean reciprocal rank | 0.5000 |
| nDCG@5 | 0.6309 |
| Mean relevant rank | 2.0000 |

Precision@5 uses the fixed five-position denominator. The runtime deliberately
returned only 2, 2, and 3 positive candidates rather than padding Top 5 with
zero-similarity results; the separately reported returned-result precision
therefore captures the density of the evidence actually exposed. Every labeled
answer ranked second behind its lexical trap.

| Operational metric | Result per session unless noted |
| --- | ---: |
| Retrieval phase / coverage | `ready` / 100% |
| Eligible / indexed / failed files | 30 / 30 / 0 |
| Indexed chunks / vector records | 39 / 39 |
| Accounted vector bytes | 9,595 |
| Embedding requests | 2 (1 document + 1 query) |
| Embedding inputs | 40 (39 document + 1 query) |
| Document batches / physical requests / lower bound | 1 / 1 / 1 |
| Document-request amplification | 1.0x |
| Time to first file-atomic publication, p50 / p95 | 9 / 10 ms |
| Non-text provider inputs | 0 |
| End-to-end task p50 / p95 | 11,220 / 31,116 ms |
| Total DeepSeek tokens, three tasks | 39,471 |

The post-`CODE-B2` run reduces the frozen 30.0x baseline to 1.0x. Each session's
39 document chunks fit one request under the configured input, text-byte, and
expected-vector-byte limits. Core status and the independent loopback provider
both observed exactly one document request, so model output cannot manufacture
the result. End-to-end task latency includes process/session setup, asynchronous
indexing, remote DeepSeek latency, tool execution, and completion. It is not a
retrieval-only latency claim. A3S Code's release benchmark remains the isolated
local retrieval latency gate.

## Reproduction

From the A3S CLI repository on PowerShell, with the A3S monorepo root containing
the real `.a3s/config.acl`:

```powershell
$env:A3S_REAL_EVAL_ROOT = (Resolve-Path 'C:\path\to\a3s').Path
cargo test --offline --locked `
  --test workspace_retrieval_real_deepseek `
  real_deepseek_acl_host_executes_recursive_reranked_workspace_tasks -- `
  --ignored --exact --nocapture --test-threads=1
```

The successful test prints one
`WSR_DEEPSEEK_ACL_HOST_EVAL=<json>` record and enforces every invariant above.
It is ignored by default because it requires repository DeepSeek credentials
and network access. The ACL-host `WSR-EVAL2` variant is qualified by this run;
cross-SDK real-model variants and any default-ranking change remain separate
gates.
