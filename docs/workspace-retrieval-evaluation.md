# Workspace Retrieval ACL-host Evaluation

Status: Passed on 2026-08-15 in deterministic-oracle, loopback real-model,
and in-process local-CPU profiles with `deepseek/deepseek-v4-pro` and A3S Code
`5612bed`.

This evaluation exercises the real `a3s code exec` boundary, effective ACL
layering, the shared manifest backend, asynchronous in-memory indexing, hybrid
search, default RRF and optional deterministic reranking, JSONL projection,
and the real DeepSeek tool/completion loop. One real profile drives the
production OpenAI-compatible HTTP adapter; the other admits the same class of
revision-locked 384-dimensional multilingual model into the optional
FastEmbed/ONNX in-process CPU adapter with no source egress or vector database.
It complements the lower-level A3S Code quality and latency suites; it does not
replace them.

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
| Ranking correctness | Relevance is judged independently from DeepSeek | Each query has a lexical trap and a separately labeled semantic answer | Both the deterministic oracle and locked real model must put the expected path in Top 5, and DeepSeek must emit the exact answer identifier |
| Tool protocol | The model cannot hide extra exploration | The prompt requires exactly one bounded hybrid search | JSONL contains one successful `search` call with the exact query, path, include, mode, and limit, and no other tool call |
| Rerank selection | Requested policy and applied algorithm agree | The oracle profile enables deterministic reranking; the real profile omits it | Tool metadata reports requested/applied `deterministic` with `rrf_k60+deterministic_mmr_v1`, or `rrf_only` with `rrf_k60`, respectively |
| Cross-process encoding | CJK source reaches the real tokenizer unchanged | Rust writes UTF-8 JSON over a Windows pipe whose Python locale may be a legacy code page | The worker explicitly binds standard streams to strict UTF-8, the CJK task retrieves the labeled path, and no surrogate or source-bearing diagnostic is accepted |
| Resource bounds | Index state is measurable and bounded | Every task creates a fresh `code exec` session | Status reaches `ready`, coverage is 100%, chunks/vectors/bytes are exact, Core batching counters equal independent provider counters, and request amplification is at most 1.10x the three-limit lower bound |
| Cold readiness | An advertised semantic route must not silently become lexical-only in a one-shot task | Every local-CPU process starts with an unloaded 252 MB admitted artifact set | A trusted 30-second event-driven readiness bound produces semantic candidates, reports no `building` fallback, and remains cancellable; zero remains the compatibility default |
| Lifecycle | Headless execution closes its session | Three independent subprocess tasks rebuild and terminate | Every command returns successfully after `session.close()`; Core lifecycle tests remain the weak-reference and zero-retained-vector authority |

The first live run reached the production ownership guard and
found that CLI sessions carried chunk configuration while the CLI also supplied
host workspace services. The fix introduced a host wrapper that separates the
catalog strategy from session runtime options and configures the shared backend
before services attach. This was an architecture defect that configuration-only
tests could not expose.

The first in-process CPU runs exposed a second architecture defect rather than
a model-quality failure. Session creation correctly returned before indexing,
but the one allowed search raced cold model load and returned exact/BM25-only
evidence with `fallback = "building"` and zero semantic candidates. Code
`5612bed` added an opt-in, event-driven readiness barrier with a 30-second hard
ceiling and deterministic ready/degraded/timeout/cancellation behavior. The
CLI keeps the default at zero and sets 30 seconds only in this one-shot local
profile. The rerun produced 25 semantic candidates in every task and no
fallback.

## Fixture and model separation

The ignored integration test is
`tests/workspace_retrieval_real_deepseek.rs`. It creates its workspace below
the repository root so normal ancestor discovery selects the repository
`.a3s/config.acl`. The test never copies or prints that file. It creates a
temporary home containing one of two retrieval routes:

- an OpenAI-compatible loopback provider with explicit source-egress gates; or
- a typed local-CPU block referencing an external immutable artifact manifest,
  with no provider route and no source-egress grant;
- recursive 512-byte chunks with 64-byte overlap and explicit separators;
- the optional typed deterministic reranker.

By default, the local embedding oracle makes ranking deterministic. When
`A3S_REAL_EMBEDDING_MODEL`, `A3S_REAL_EMBEDDING_REVISION`, and
`A3S_REAL_EMBEDDING_PYTHON` are set, the same loopback endpoint delegates to a
persistent Sentence Transformers JSON-lines worker. The worker locks the model
revision, returns unit-normalized vectors, reports its runtime versions and
device, and validates count, dimension, and finite values before the HTTP
response crosses into the production CLI provider. The trusted ACL then uses
the reported dimension and leaves the default RRF-only policy active. When
`A3S_LOCAL_CPU_MODEL_MANIFEST` is set, it instead selects the in-process
adapter, enables deterministic reranking, and binds a 30-second
semantic-readiness timeout. Runtime downloads remain unavailable.

DeepSeek remains the real chat/tool model in every profile and must inspect the
search schema, issue the exact tool call, consume the returned evidence, and
produce the labeled declaration name. The three tasks cover reconnect replay
suppression, CJK session-projection cleanup, and an answer beyond a recursive
chunk boundary. The corpus contains 30 text files, 39 expected chunks, and
three non-text assets.

## Results

All three tasks passed in all final serial profiles.

| Quality metric | Oracle + deterministic rerank | Loopback real model + RRF | In-process local CPU + deterministic rerank |
| --- | ---: | ---: | ---: |
| Exact task completion | 3/3 (1.0000) | 3/3 (1.0000) | 3/3 (1.0000) |
| Exact tool protocol | 3/3 (1.0000) | 3/3 (1.0000) | 3/3 (1.0000) |
| Precision@5 | 0.2000 | 0.2000 | 0.2000 |
| Precision among returned results | 3/7 (0.4286) | 3/15 (0.2000) | 3/15 (0.2000) |
| Mean returned results | 2.3333 | 5.0000 | 5.0000 |
| Recall@5 | 1.0000 | 1.0000 | 1.0000 |
| Mean reciprocal rank | 0.5000 | 0.5000 | 0.3444 |
| nDCG@5 | 0.6309 | 0.6309 | 0.5059 |
| Mean relevant rank | 2.0000 | 2.0000 | 3.3333 |

Precision@5 uses the fixed five-position denominator. The oracle returned only
2, 2, and 3 positive candidates rather than padding Top 5 with zero-similarity
results, while the real model produced five non-zero candidates for each task.
Returned-result precision therefore reports evidence density separately from
the fixed retrieval gate. Every labeled answer ranked second behind its lexical
trap in the two earlier profiles; local deterministic MMR placed the three
targets at ranks 5, 2, and 3.

| Operational metric | Oracle + deterministic rerank | Loopback real model + RRF | In-process local CPU + deterministic rerank |
| --- | ---: | ---: | ---: |
| Retrieval phase / coverage | `ready` / 100% | `ready` / 100% | `ready` / 100% |
| Eligible / indexed / failed files | 30 / 30 / 0 | 30 / 30 / 0 | 30 / 30 / 0 |
| Indexed chunks / vector records | 39 / 39 | 39 / 39 | 39 / 39 |
| Accounted vector bytes | 9,595 | 68,251 | 68,251 |
| Embedding requests | 2 (1 document + 1 query) | 2 (1 document + 1 query) | 2 (1 document + 1 query) |
| Embedding inputs | 40 (39 document + 1 query) | 40 (39 document + 1 query) | 40 (39 document + 1 query) |
| Document batches / physical requests / lower bound | 1 / 1 / 1 | 1 / 1 / 1 | 1 / 1 / 1 |
| Document-request amplification | 1.0x | 1.0x | 1.0x |
| Time to first file-atomic publication, p50 / p95 | 6 / 8 ms | 435 / 454 ms | 9,102 / 9,498 ms |
| Non-text provider inputs | 0 | 0 | 0 |
| End-to-end task p50 / p95 | 10,580 / 11,043 ms | 10,169 / 13,737 ms | 18,472 / 19,200 ms |
| Total DeepSeek tokens, three tasks | 39,432 | 40,096 | 40,382 |
| Embedding runtime | Rust oracle | Python 3.13.2; sentence-transformers 3.2.1; transformers 4.53.2; torch 2.7.1; CPU | FastEmbed 5.17.3; ONNX Runtime 2.0.0-rc.12; CPU |

The post-`CODE-B2` profiles reduce the frozen 30.0x baseline to 1.0x. Each
session's 39 document chunks fit one request under the configured input,
text-byte, and expected-vector-byte limits. Core status and the independent
loopback provider both observed exactly one document request; the local profile
reports the same host batching boundary because no HTTP adapter exists. Model
output therefore cannot manufacture the result. The loopback real model took
less than half a second to publish the first file-atomic ready partition. The
local profile includes model admission and cold ONNX load in each fresh
process, reaching full readiness in roughly 9 seconds. End-to-end task
latency includes process/session setup, asynchronous indexing, remote DeepSeek
latency, tool execution, and completion. It is not a retrieval-only latency
claim. A3S Code's release benchmark remains the isolated local retrieval
latency gate.

The isolated local provider microbenchmark excludes DeepSeek and workspace
startup. On the Windows reference host its schema-v2 report recorded a 7,568
ms cold call, a 20 ms warm query, 0 ms cancellation return, and a 971 MiB peak
RSS increase below the locked 1 GiB bound. It produced 384-dimensional
unit-normalized deterministic output, a relevant cosine score of `0.2941847`,
and a distractor score of `0.1241785`.

The same Windows debug build measured a 28,013,568-byte / 15.22% binary delta
between the model-free and local-CPU feature graphs. Optimized release sizes
remain target-specific CI evidence rather than an inference from this number.

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

The real-embedding profile adds an optional Python test dependency and a locked
model revision. The following example uses a pre-populated local Hugging Face
cache and therefore keeps model loading offline:

```powershell
$env:A3S_REAL_EMBEDDING_PYTHON = py -3.13 -c 'import sys; print(sys.executable)'
$env:A3S_REAL_EMBEDDING_MODEL = `
  'sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2'
$env:A3S_REAL_EMBEDDING_REVISION = `
  'e8f8c211226b894fcb81acc59f3b34ba3efd5f42'
$env:A3S_REAL_EMBEDDING_LOCAL_ONLY = '1'
cargo test --offline --locked `
  --test workspace_retrieval_real_deepseek `
  real_deepseek_acl_host_executes_recursive_reranked_workspace_tasks -- `
  --ignored --exact --nocapture --test-threads=1
```

The in-process profile uses an already installed artifact set admitted by
`model.acl`; neither command downloads a model at runtime:

```powershell
$env:A3S_LOCAL_CPU_MODEL_MANIFEST = `
  (Resolve-Path 'C:\path\to\model.acl').Path
cargo test --offline --locked --features local-cpu-embedding `
  --bin a3s `
  real_local_cpu_model_embeds_offline_and_preserves_multilingual_relevance -- `
  --ignored --nocapture
cargo test --offline --locked --features local-cpu-embedding `
  --test workspace_retrieval_real_deepseek `
  real_deepseek_acl_host_executes_recursive_reranked_workspace_tasks -- `
  --ignored --exact --nocapture --test-threads=1
```

The successful test prints one schema-v4
`WSR_DEEPSEEK_ACL_HOST_EVAL=<json>` record and enforces every invariant above.
It is ignored by default because it requires repository DeepSeek credentials
and network access; the loopback real profile additionally requires the Python
runtime and model weights, while the local profile requires a feature-built
binary and admitted artifacts. Code `cde887b` qualified the public Node.js, Python, and Go
real-model variants against one versioned fixture and normalized report. Each
SDK passed 3/3 exact tasks and one-Search protocols with Recall@5 1.0, MRR 0.5,
1.0x document-request amplification, zero non-text provider inputs, and
complete post-close vector release. CLI `5a27e81` closes the separate
production HTTP-provider gate using the same locked multilingual model; this
evaluation now adds the in-process CPU path and the Code `5612bed` readiness
contract. The
detailed cross-SDK metrics and reproduction commands are in the
[A3S Code cross-SDK evaluation](https://github.com/A3S-Lab/Code/blob/7e5c1850ff4ae62a16b4585ab9b8946aa63d75b5/sdk/evaluation/README.md).
The three-task matrix closes the portability gate but does not qualify a
default-ranking change.
