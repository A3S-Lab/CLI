# Local CPU Workspace Embedding

Status: opt-in production-qualified. The source default remains model-free;
official release builds enable the adapter only on targets supported by the
pinned ONNX Runtime.

This adapter supplies semantic vectors to the existing asynchronous,
session-bound A3S Code retrieval runtime. It does not add a vector database,
persist vectors, change the workspace text classifier, or parse non-text
documents. PDF, Office, image, audio, archive, and other non-text assets remain
owned by the separate knowledge-compilation system.

## Baseline and enablement

Exact search, glob/path search, incremental BM25, reciprocal-rank fusion, and
the optional deterministic MMR reranker run directly on CPU without an
embedding or rerank model. Semantic search is additive and disabled by
default.

Local semantic search requires all of the following:

1. a binary compiled with `--features local-cpu-embedding`;
2. a trusted user ACL or explicitly selected `--config` file;
3. `workspace_retrieval.enabled = true` and one typed `local_cpu` block;
4. a separately installed, immutable artifact set described by `model.acl`.

An automatically discovered workspace ACL cannot enable or route retrieval.
It may only set `workspace_retrieval { enabled = false }`. A local route is
mutually exclusive with `allow_source_egress`, `model`, `endpoint`, `revision`,
`dimension`, and `normalization` remote-route fields.

## Immutable artifact manifest

The initial runtime contract is `fastembed-onnx-v1` version `5.17.3`. The
manifest must be an unlabeled ACL block with exactly five labeled files:

```acl
local_embedding_model {
  schema_version = 1
  model = "publisher/multilingual-embedding-model"
  revision = "immutable-upstream-revision"
  runtime = "fastembed-onnx-v1"
  runtime_version = "5.17.3"
  dimension = 384
  normalization = "unit"
  pooling = "mean"
  quantization = "dynamic"
  license = "Apache-2.0"
  max_length = 128

  file "model" {
    path = "model_optimized.onnx"
    sha256 = "<64 lowercase hexadecimal characters>"
  }
  file "tokenizer" {
    path = "tokenizer.json"
    sha256 = "<64 lowercase hexadecimal characters>"
  }
  file "config" {
    path = "config.json"
    sha256 = "<64 lowercase hexadecimal characters>"
  }
  file "special_tokens_map" {
    path = "special_tokens_map.json"
    sha256 = "<64 lowercase hexadecimal characters>"
  }
  file "tokenizer_config" {
    path = "tokenizer_config.json"
    sha256 = "<64 lowercase hexadecimal characters>"
  }
}
```

Generate each digest from the final installed bytes. For example, PowerShell
uses `Get-FileHash -Algorithm SHA256`; GNU systems use `sha256sum`. Model
installation is intentionally separate from session startup. A3S never
downloads or silently updates these files.

Artifact paths use portable relative forward-slash syntax and must remain
below the canonical manifest directory. The manifest and final artifact may
not be symbolic links. Admission rejects missing, empty, substituted,
oversized, duplicated, or unknown assets. The model file is limited to 256 MiB,
the tokenizer to 64 MiB, metadata files to 1 MiB each, and the complete set to
384 MiB. `max_length` is bounded to 8..8192 and the output dimension to
1..65536. The configured vector memory budget must hold at least one vector.

Run admission before starting an agent:

```bash
a3s config validate
a3s --output json config show
```

`config show` reports `backend = "local_cpu"`, `localCpuAvailable`,
`localCpuUnavailableReason`, and the effective readiness bound, but never the
artifact path, source text, vectors, credentials, or endpoint values. The
reason is null when usable; otherwise it is one of `feature_disabled`,
`unsupported_architecture`, or `missing_x86_64_v3`.

## Runtime and lifecycle

Session construction remains asynchronous. The model is loaded lazily on the
first embedding request using `spawn_blocking`; Tokio I/O workers are not
blocked. A process caches one content-compatible model, admits one native
inference job at a time, and feeds it at most two inputs per executor
microbatch. The model has an explicit `intra_threads` limit of 1..64.
Cancellation returns control to the caller promptly; an already-running native
inference retains the global permit until it finishes because ONNX Runtime does
not expose cooperative cancellation for that call. Cancelled waiters never
enter the native region, and the next live request proceeds when that permit is
released.

Cold local models can take several seconds to load. Set
`semantic_readiness_timeout_ms` when a one-shot agent must wait for the first
complete semantic generation. The default `0` returns immediate exact/BM25
fallback for compatibility; the maximum `30000` waits on an event, not a
polling loop. Ready, degraded, timeout, caller cancellation, session close, and
the hard bound are enforced by A3S Code. Vectors stay in the session-owned A3S
Memory index and are released on close.

## Platform and packaging matrix

| Target | Official archive | Local CPU adapter |
| --- | --- | --- |
| Linux x86_64 | Yes | Enabled |
| Linux aarch64 | Yes | Enabled |
| Windows x86_64 | Yes | Enabled |
| macOS Apple Silicon | Yes | Enabled |
| macOS Intel x86_64 | Yes | Not compiled; model-free and remote retrieval remain available |

The Intel macOS exception follows the pinned `ort 2.0.0-rc.12` binary support:
upstream dropped `x86_64-apple-darwin`. Its supplied x86_64 binaries also use
the x86-64-v3 baseline, so local CPU embedding on Windows/Linux x64 requires a
Haswell/Broadwell, AMD Zen, or newer CPU. Model-free builds do not inherit that
runtime requirement. An operator may build against a
separately managed ONNX Runtime, but that is not an A3S release artifact or a
qualified configuration.

CI compiles, links, and performs real offline inference on native Linux x64,
Linux ARM64, Windows x64, and Apple Silicon runners. A checked-in ACL manifest
locks the Apache-2.0 smoke model revision and every SHA-256 digest; a separate
provisioning step downloads its roughly 23 MiB ONNX artifact, after which
admission, inference, cancellation, recovery, and RSS checks run offline. The
release matrix deliberately leaves the Intel macOS feature empty. The default
source feature set remains empty, which preserves a build with no FastEmbed or
ONNX Runtime dependency.

## Qualification evidence

The Windows reference run uses a revision-locked, statically quantized,
384-dimensional multilingual MiniLM model on CPU. The provider microbenchmark
reported a 7,045 ms cold call, a 19 ms warm call, 0 ms cancellation return,
267 ms cancellation-to-next-success recovery, and a 1,018,519,552-byte peak
RSS increase below the locked 1 GiB bound. The two-input microbatch replaced an
unbounded 64-input path that reached about 1.60 GiB under cancellation load.
Output remained a deterministic unit vector with higher cosine similarity for
the relevant multilingual code fact than for the distractor.

The smaller cross-platform smoke fixture reported a 648 ms cold call, a 3 ms
warm call, 0 ms cancellation return, 42 ms recovery, and an 84,578,304-byte
peak RSS increase on the Windows reference host. These values are a local
diagnostic; each native CI job emits its own schema-v4 record and enforces the
same lifecycle and 1 GiB hard bound.

On the same source and debug profile, the feature increased `a3s.exe` from
184,061,440 to 212,075,008 bytes: +28,013,568 bytes / 15.22%. This is a local
link diagnostic, not a compressed release-archive claim; release CI remains
the authority for each optimized target.

The real DeepSeek ACL-host matrix completed 3/3 tasks with one bounded hybrid
search per task, Recall@5 1.0, MRR 0.3444, nDCG@5 0.5059, and zero non-text
embedding inputs. Each fresh one-shot process indexed 30 files / 39 chunks in
20 memory-bounded document microbatches, exactly the configured lower bound,
used 68,251 accounted vector bytes, and reached full readiness in 12,163 ms
p50 / 12,342 ms p95. Full details and reproduction commands are in
[Workspace Retrieval ACL-host Evaluation](workspace-retrieval-evaluation.md).

The CI gate also rejects missing, empty, substituted, oversized, duplicated,
and unknown artifacts, simulates a missing x86-64-v3 feature before model
loading, and proves that a 32-waiter cancellation storm neither exceeds one
native job nor starves the recovery request. This closes the host-local CPU
runtime gate; ranking remains RRF-only by default because deterministic MMR did
not improve the locked multilingual corpus.
