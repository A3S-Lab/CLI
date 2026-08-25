# A3S Use and Component Platform

Status: Active implementation, pre-1.0

Updated: 2026-08-25

Owners: A3S CLI, A3S Use, A3S Runtime, A3S Flow, A3S Knowledge, A3S Gateway,
and A3S Updater

## 1. Decision

A3S Use is the AI Native Package Manager for the A3S capability and security
model. It owns one verifiable lifecycle for:

1. platform-native A3S components and provider assets; and
2. versioned cognitive packages that can contribute Tool, MCP, OKF, A3S Flow,
   Skill, and UI surfaces.

The primary user entry point is `a3s use`. The standalone `a3s-use` binary is
the delegated package engine and remains available for automation, embedding,
and diagnostics.

The umbrella host owns public commands, Registry configuration, trust roots,
policy, confirmation, secrets, and provider composition. It uses one shared
Plugin Manager for CLI, TUI, and management MCP entry points. No adapter may
create a second package lifecycle.

## 2. Product hierarchy

```text
A3S product components
├── code                         bundled with a3s
├── box                          separately distributed
├── bench                        separately distributed
├── search                       separately distributed
└── use                          separately distributed as a3s-use
    ├── use/browser              built-in domain
    ├── use/office               built-in native provider
    ├── use/ocr                  built-in local provider
    └── use/<publisher>/<name>   versioned cognitive package
```

`a3s install` manages registered A3S components. It is not a universal frontend
for Homebrew, APT, DNF, Pacman, WinGet, npm, pip, Cargo, or arbitrary package
names. Operating-system package managers retain ownership of arbitrary system
software.

## 3. Current implementation status

| Area | Current state |
| --- | --- |
| Native component foundation | Available: typed catalog, verified release/Homebrew sources, receipts, review/apply digests, durable batch checkpoints, ownership-safe uninstall, and first-use policy. |
| Built-in Use domains | Browser, Office, and OCR are projected through the Use parent with typed provider readiness. |
| Schema-v3 package format | Implemented in A3S Use: Tool, MCP, OKF, A3S Flow, Skill, UI, required README, SemVer dependencies, and a typed readiness graph. |
| Signed dependency graph | Implemented for remote cognitive packages: deterministic resolution, exact package lock, dependency-forward install, shared retention, one publication, reverse uninstall, and replay. |
| Host Plugin Manager | One Use-owned `PluginManagerService` serves CLI, TUI, the exact ten-tool manager-v4 MCP, and the canonical fenced remote `PluginHostManager`. Code injects Registry access, ACL policy, lifecycle providers, and trusted confirmation. The current MCP confirmation provider fails closed for apply. The service admits only provider-neutral drafts, binds actor and exact User/Workspace scope, resolves signed planning bundles through host-owned Runtime assignments, performs two-pass Grant/provider binding around full-plan policy evaluation, and persists confirmation, intent, lifecycle cutover, and replay evidence. |
| Reviewed Use authorization bridge | Complete schema-v3 install plans execute through an in-process reviewed provider. The exact host envelope, scope, signed planning bundles, durable Grant snapshot/revision, reviewed provider evidence, and confirmation reach Use without argv/environment authority. Apply reconstructs the same Grants, generations, assignments, and selection before download or mutation; Registry identity and provider evidence drift fail closed. |
| Code runtime composition | Code delegates package lifecycle to the shared Use managed factory and consumes a resident typed capability Registry snapshot/cursor. Verified managed MCP servers, Skills, eligible provider-qualified Runtime Tool Tasks, digest-bound non-queryable Knowledge Surface readiness, dependency-closed local Flows, and bounded path-free UI bindings publish through one Core atomic Session catalog cut, and every admitted Run or projected host handle acquires its own exact, non-clone Use snapshot lease. UI and Flow use canonical Use surface IDs and fail the candidate batch when a declared dependency is absent; reviewed Tool, MCP, and OKF readiness dependencies resolve within the same exact package generation, while transport, rendering, Flow execution, and Knowledge queries remain host-owned. Each MCP projection preserves canonical surface identity, activation, exact lifecycle/file evidence, and either a package-confined stdio launcher or opaque Runtime/Gateway readiness evidence; Code resolves no package-authored URL or credentials. The capability watcher exposes a reviewed Task only when the shared `PluginManager` contains its named provider. A trusted Linux ACL can explicitly compose the shared Box provider for release-backed Tool Tasks. Adding its private Gateway block assigns that same provider to Tool Services and Streamable HTTP MCP, starts a durable exact-generation loopback Gateway, and performs standard MCP initialize through the returned route. The default stays empty and workspace ACL cannot select either provider or Gateway authority. |
| Code scoped runtime composition | The one-shot Code Exec/Desktop host publishes an atomic managed-MCP/Skill/Runtime-Task/UI cut backed by one process-owned Plugin Manager. That single immutable host supplies both provider-qualified exact-generation Task dispatch and typed resolution of opaque HTTP MCP provider/reference/path evidence to a credential-free numeric loopback route. It remains alive while the frozen Session owns projected Tools and MCP clients; Session close precedes bounded Runtime/Gateway shutdown on success, cancellation, and failure. |
| Code Flow catalog | Every exact Flow remains available through the watched catalog. Dependency-free, Tool-dependent, MCP-dependent, and OKF-dependent Native TypeScript Flows additionally reverify and digest-stage their source, complete workspace-local preflight, and enter the resident atomic batch as exact Core `FlowBinding` values. OKF edges target a same-package `KnowledgeSurfaceBinding`, never the dynamic query tool. Missing readiness, preflight failure, MCP preparation failure, and cancellation during lock contention leave the current generation unchanged. |
| Code `flow.json` identity | Implemented for TUI and non-resident CLI: typed designs resolve an exact package/Flow/version/lifecycle-generation/source-digest tuple before runtime mutation. |
| Code OKF composition | Available through the real Use Knowledge lifecycle host: bounded OKF inspection, stage/promotion/removal, receipt-accounted scope quota, bounded generations/tombstones, SQLite/WAL compaction, durable exact-generation bindings, restart recovery, integrity audit, derived-index repair, versioned backup/offline verification, watched TUI projection, cited scope-bound retrieval, and exact published-generation query leases held through backend search and Registry revision verification. |
| Code UI lifecycle | Signed package UI assets receive integrity-bound static lifecycle evidence and scope/package/surface state cleanup. The CLI requires the versioned dependency-completeness marker, revalidates bounded UTF-8 assets, projects path-free `UiBinding` values plus canonical dependencies, and proves N/N+1 document and lease retention. A reviewed Tool-, MCP-, or Flow-dependent UI publishes with its exact dependency in one batch; provider absence or a missing Tool/MCP/Flow edge leaves the current catalog unchanged. Code does not expose a browser renderer or browser-readiness rendezvous; native rendering remains a host concern. |
| Code managed Runtime surfaces | Typed Runtime selection, Task dispatch, and endpoint/retirement contracts are composed. A host-injected deterministic provider proves signed OCI Tool Task install, retained planning-bundle recovery, offline restart-safe disable/re-enable, exact apply-time build reconstruction, drift rejection, Grant persistence, stopped-binding reauthorization, and replay. Capability Registry schema v2 carries the exact Task identity into a conservative `use_tool_*` Tool value in the resident Core batch; dispatch uses the durable binding rather than current assignments, and no compatibility Tool registration is performed. The Linux host now composes its explicit Box provider with a durable private Gateway for Tool Service health/routing and standard HTTP MCP initialize, including idempotent bind, restart recovery, exact receipt-owned drain, and removal tests. The Linux/macOS/Windows monorepo gate observes lifecycle planning through an independently built exact-revision `a3s-use` process. Tool Services remain Gateway-owned; real Box Service process-kill qualification, non-Linux providers, and the cross-platform uninstall/upgrade matrix remain open. |
| Hot-plug integration | A real extension N/N+1 regression proves an admitted Run keeps its N Skill registry and Use snapshot lease while N+1 publishes, blocks old-generation drain until completion, and gives new Runs only N+1. UI regressions prove an admitted N handle keeps N's exact document and Use lease after N+1, a reviewed Tool and Tool-dependent UI share one exact generation, and a missing dependency advances neither the Code catalog nor applied receipt. MCP regressions prove exact stdio file evidence, trusted loopback HTTP resolution, global host-name uniqueness, and same-package MCP closure for Flow/UI. Flow regressions prove same-package Tool/MCP/OKF dependency closure, canonical multi-scope Knowledge Surface aggregation, preflight-before-publication rollback, missing-OKF atomic rollback, and cancellation-bounded workspace-lock contention. Replacement TUI sessions publish the current atomic MCP/Skill/Runtime Tool/Knowledge Surface/Flow/UI generation asynchronously without copying extension values into compatibility registries. Runtime Tool N/N+1 and disable regressions prove one catalog advance per cutover and exact-generation dispatch; provider absence publishes nothing and emits a warning. Compatibility tests remain for built-in MCP and dynamic OKF Knowledge search. Query-carrier regressions prove Knowledge lease lifetime, package-generation deduplication, and fail-closed missing or conflicting lease evidence. |
| Remaining release gates | Managed Knowledge rollback, coordinated restore and authority recovery, backup rotation, real Box MCP/Service process-kill and retained-generation upgrade/uninstall validation, non-Linux Runtime providers, distributed Flow and Knowledge placement, and complete real-process cross-platform E2E remain open. |

## 4. System architecture

```text
       explicitly trusted named TUF Registries
                         │
                         ▼
                 Host Plugin Manager
 source set · ACL policy · Grant snapshot · review · confirmation
                         │
                         ▼
              A3S Use package graph manager
       resolve · lock · stage · journal · reconcile
                         │
          ┌───────────┬──────────┬───────────┐
          ▼           ▼          ▼           ▼
   Native launcher  A3S Flow  Knowledge   Static host
    Task · stdio    native-ts  OKF/FTS5    Skill · UI
          │           │          │           │
          └───────────┴──────────┴───────────┘
                         ▼
        typed capability snapshot · cursor · watch
                         │
                         ▼
            atomic Code Session Skill catalog
                         │
              fresh Use lease per Code Run
                         │
                         ▼
                   A3S Code TUI
          /use + /flow + managed OKF
                         │
                         ▼
                  local a3s-flow
                staged source · events
```

OCI or long-lived Tool workloads and HTTP MCP join the same lifecycle only
when their Runtime and Gateway hosts return typed readiness evidence. Their
absence is an admission failure, not a fallback route. OKF uses the composed
local Knowledge host and remains bound to its explicit scope and generation.

### 4.1 Ownership

| Owner | Owns | Does not own |
| --- | --- | --- |
| `a3s` umbrella CLI | Public command grammar, component catalog, named Registries, trust roots, first-use policy, review/apply UX, and trusted Runtime/Gateway composition | Package archive internals, Browser actions, Knowledge indexing, or workflow engine semantics |
| Shared Plugin Manager | Marketplace projection, provider-neutral draft admission, immutable plan identity, actor/scope binding, two-pass Grant/provider binding, policy evaluation, durable planning evidence, exact apply reconstruction, confirmation, parent lifecycle intent/cutover, Use-owned schema-v3 enablement routing, and replay | Package validation, package-selected providers, or child provider execution |
| Fenced managed-host adapter | Canonical Use host capabilities plus exact managed graph plan/apply/observe and reviewed schema-v3 enable/disable over the shared Manager, with one durable host-owned Workspace fence | A second package manager, remote fence provisioning/rotation, or permission-bearing enablement without exact Grant cutover evidence |
| `a3s-updater` | Release resolution, download, verification, staging, receipts, atomic activation primitives, and owned-file removal | Product catalog or cognitive surface semantics |
| A3S Use | Package validation, dependency lock, immutable generations, provider/Grant planning semantics, package/child journals, receipts, Runtime/Flow/Knowledge bindings, and capability reconciliation | Host policy, user confirmation, host assignment choice, or product UI |
| Native launcher | Package-bound non-interactive Tool Tasks and stdio MCP process lifecycle | OCI workloads, long-lived Services, package dependency resolution, or capability publication |
| A3S Runtime / Gateway | OCI/Service execution and HTTP MCP serving/readiness | Native Task/stdio launch, package dependency resolution, or capability publication |
| A3S Flow | Native TypeScript preflight, compiled artifacts, durable execution, history, replay, scheduling, and observation | A second package lifecycle or visual asset ownership |
| A3S Knowledge | OKF staging, promotion, scoped projection, cited retrieval, and observation | Package installation or host policy |
| A3S Code | Agent sessions, TUI presentation, dedicated Use worker, resident typed snapshot/cursor consumption, atomic verified-Skill publication, per-Run Use lease acquisition, workspace-local composition of the sole A3S Flow engine, and read-only exact-generation Knowledge query projection | A second Registry, resolver, package journal, workflow engine, or Knowledge lifecycle |

### 4.2 Why the lifecycle is a saga

Package storage, permission grants, Runtime, Gateway, A3S Flow, Knowledge, and
capability publication do not share one database transaction. A cognitive
package mutation is therefore a durable, idempotent saga:

```text
resolve signed dependency closure
    → freeze exact package lock
    → bind reviewed host plan and authority
    → revalidate every Registry before download
    → verify and stage dependencies before dependents
    → prepare grants and surface hosts
    → publish changed packages in one capability generation
    → hide and drain superseded surfaces
    → remove unused packages in reverse dependency order
```

Each child action has an idempotency key and durable evidence. Recovery
revalidates the current state before continuing. A child success without the
matching parent cutover is never reported as a completed package operation.

## 5. Cognitive package contract

A package is one versioned, content-addressed unit:

```text
<publisher>-<name>/
├── a3s-use-extension.acl
├── README.md
├── tools/
├── releases/
├── flows/
├── skills/
├── ui/
└── okf/
```

Only the ACL manifest and `README.md` filenames are fixed. Every contribution
path is manifest-owned, package-relative, and digest-bound. ACL is the A3S
Agent Configuration Language and must be parsed with `a3s-acl`; it is not HCL.

The package ID is lifecycle identity. A route is only a presentation/dispatch
alias. Individual surfaces can be selected and projected by name, but cannot
be independently installed, upgraded, enabled, disabled, or uninstalled
outside their package generation.

### 5.1 Surface readiness

| Surface | Required evidence |
| --- | --- |
| Tool Task | Signed planning target, deterministic native-launcher evidence, exact executable/package binding, and terminal observation. |
| Tool Service | Runtime Service endpoint and health for the exact generation. |
| stdio MCP | Signed planning target, deterministic native-launcher evidence, and standard MCP initialization/tool-list readiness. |
| HTTP MCP | Gateway/Runtime endpoint, route ownership, and serving readiness. |
| OKF | Knowledge stage, promotion, observation, and scoped projection evidence. |
| A3S Flow | Valid `a3s-flow`/`native-ts` source, compiler preflight, compiled artifact, dependency edges, and exact-generation binding. |
| Skill | Content-bound `SKILL.md`; every declared Tool/MCP/OKF/Flow dependency must already be ready. |
| UI | Integrity-bound entry/assets, sandbox compatibility, and every bound Skill/MCP/Flow dependency ready. |

## 6. Replaceable Registry model

Registry endpoints and trust roots are host input. They are never supplied by a
package dependency declaration or compiled into the resolver.

```bash
a3s registry add packages https://packages.example.org/a3s/ \
  --root-sha256 <root-sha256> \
  --trusted-root ./root.json \
  --yes
a3s registry refresh packages
a3s registry list # copy the current revision before each mutation
a3s registry disable packages --revision <current-revision> --yes
a3s registry replace packages https://mirror.example.org/a3s/ \
  --root-sha256 <mirror-root-sha256> \
  --trusted-root ./mirror-root.json \
  --revision <current-revision> \
  --yes
a3s registry enable packages --revision <current-revision> --yes
```

The canonical `state/use/registries.acl` source document is shared by CLI,
Marketplace, planning, and apply. Each mutation uses the exact revision shown
by `a3s registry list`; a reviewed install/upgrade plan also freezes that
revision before its first apply intent. The package engine accepts a root
Registry plus a bounded set of dependency
Registries. Resolution rejects missing versions, incompatible constraints,
cycles, search-bound exhaustion, and the same package appearing in more than
one enabled source.

The canonical lock freezes, per package:

- package identity, exact version, and satisfied dependency edges;
- Registry name/URL/channel/target and root identity;
- TUF root, timestamp, snapshot, and targets versions;
- archive length and digest;
- expanded package and manifest digests; and
- host target and A3S Use compatibility.

An installed receipt remains bound to the selected Registry identity. Removing,
disabling, or replacing that source blocks upgrade. The operator must restore
and enable the exact source or perform an explicit migration/reinstall; Use
never searches another Registry implicitly.

The umbrella CLI supports named add, remove, list, show, refresh, stable-name
replace, and enable/disable. Disabled sources remain visible to inspection and
Marketplace source status but are never browsed or admitted into resolution.
Replacement writes a content-addressed managed root before atomically switching
the Registry ACL, preserves the source enabled state, and never mutates package
receipt provenance.

## 7. One A3S Flow model

A cognitive package Flow, Code visual Flow, and A3S Flow runtime are layers of
one identity:

| Layer | Responsibility |
| --- | --- |
| `a3s-use-extension.acl` | Package/lifecycle identity, TypeScript source, export, digest, and Tool/MCP/OKF edges. |
| `flows/*.ts` | Code-authored handlers shipped in the package. |
| `native-ts` | Current source-to-runtime adapter. |
| `flow.json` | A3S Code visual design/deployment document for the same Flow identity. |
| `a3s-flow` | Sole preflight and execution engine. |

The current package schema accepts only `engine = "a3s-flow"` and
`runtime = "native-ts"`. Code accepts an absolute
`A3S_FLOW_NATIVE_TS_COMPILER` path or resolves `a3s-flow-native-compiler` from
`PATH` to one stable absolute path before lifecycle composition. Use retains package
lifecycle evidence under its state root. Code stores host-owned digest-addressed
source staging, native cache, path-free run bindings, and append-only event
history under `.a3s/flow-runtime/` in the active workspace.

The Use lifecycle binding records scope, package, Flow surface, lifecycle
generation, manifest/package/source digests, export, entrypoint, and compiled
artifact digest. Code's separate run binding contains only path-free installed
identity and catalog evidence. Projection checks the exact installed source;
every new Run checks it again immediately before staging and compilation.
Source presence alone never marks the Flow ready.

The watched Flow catalog returns the Use snapshot generation/revision and typed, content-bound,
path-free Flow items. Code visual designs use the stable
`a3s.workflow.design.v1` envelope. A newly scaffolded design is an unbound visual
draft: Publish and Design may store/open it, but no runtime binding is created,
Run fails before local compiler/event mutation, and Deploy fails before OS
mutation.

A runnable design carries one strict, path-free reference:

```json
{
  "version": "a3s.workflow.design.v1",
  "name": "report-review",
  "installedFlow": {
    "schema": "a3s.use.installed-flow.v1",
    "packageId": "use/acme/report",
    "flowId": "review",
    "version": "1.0.0",
    "lifecycleGeneration": 9,
    "sourceSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  },
  "nodes": [],
  "edges": []
}
```

The reference accepts no unknown fields and contains no source path, route,
engine, runtime, export, or catalog revision that a design author could forge.
Code parses it under bounded size/item limits, requires canonical SemVer,
canonical package/surface IDs, a non-zero lifecycle generation, and a lowercase
SHA-256 digest, then resolves exactly one live catalog item.

Resolution requires the same package ID, Flow ID, version, lifecycle
generation, and digest. A Run then rejects symlinks, containment escape, digest
drift, non-regular files, and invalid UTF-8 before it writes the verified bytes
to host-owned staging or invokes the compiler. The resolved evidence adds
route, engine, runtime, export, catalog generation, and catalog revision without
exposing a managed path. It is persisted in the local run binding and, for
Deploy, copied into OS runtime-binding metadata and `.a3s/asset.acl`. Upgrade,
disable, uninstall, ambiguous projection, generation drift, and digest drift
all fail closed for new execution. Existing run histories stay readable after
the installed generation is withdrawn. Unrelated package changes do not
invalidate the design because catalog generation/revision remain observation
evidence rather than persisted reference fields.

The resident TUI uses the existing watched snapshot. A non-resident
`a3s code flow run` performs the same stable two-snapshot Use inspection and
source verification without starting a watcher. CLI and TUI status/logs read
the durable binding and event store without requiring Use or OS, so history
survives package upgrade, disable, or uninstall. CLI and TUI are the current
interactive local execution surfaces.

Invalid design syntax/schema returns `400`; a missing, stale, ambiguous, or
withdrawn installed identity returns `409`; unavailable Use projection returns
`503`. The current runtime is workspace-scoped and guarded by a cross-process
lock because `LocalFileEventStore` is single-process by itself. It provides
single-node crash/restart durability and observation; distributed workers,
automatic scheduling/resumption, and production retention remain product work.
The OS asset adapter remains `a3s-workflow-service` for publish/deploy/open and
does not create another package lifecycle or execution engine.

### 7.1 Managed OKF Knowledge

The Code host composes `OkfKnowledgeLifecycleHost` with the Use-owned
`SqliteOkfKnowledgeAdapter` and `OkfKnowledgeBindingStore`. Install inspects the
bounded OKF bundle, stages an immutable FTS5 index, promotes only matching
observation evidence, and persists the exact User or Workspace binding. Upgrade
selects the replacement generation; uninstall removes only receipt-owned
projection data. A newly constructed host can recover and query the promoted
binding after process restart.

The adapter enforces whole-scope projection and expanded-byte quotas in the same
transaction that stages a candidate. Its default policy retains at most 256
projections per scope, 32 generations per surface, 512 MiB of expanded OKF
content, and 256 scope-wide tombstones. Removal frees quota, globally prunes
old tombstones, vacuums SQLite, and truncates its WAL. The command
`a3s use knowledge usage --json` reports the exact non-secret allocation;
Workspace usage requires an explicit kind and ID.

The same proxy exposes `knowledge audit`, `knowledge backup <path>`,
`knowledge verify-backup <path>`, and confirmed
`knowledge repair-search-index`. Every operation binds the complete User or
Workspace scope. Audit checks SQLite, foreign keys, receipts, storage evidence,
scope identity, and FTS integrity. Backup is non-overwriting and can be
verified offline; repair rebuilds only derived FTS rows after authoritative
state passes validation. A backup digest detects corruption but is not a
Registry signature, and the artifact excludes Registry receipts, package
roots, bindings, journals, Grants, Flow history, and UI state. Coordinated
restore is not implemented, and directly copying a snapshot into live state is
unsupported.

The Use capability snapshot carries `OkfCapabilityProjection` evidence rather
than package paths or arbitrary backend names. Code validates every projection
and rejects two active generations of the same package surface in one scope.
The shared carrier groups by the complete scope identity (`kind` plus `id`),
requires an explicit selection when multiple scopes are active, and passes only
the current Registry revision's projections into Knowledge. If a Registry
cutover races a search, the carrier retries once against the replacement
revision and never returns the stale result as current context.

Before backend access, the carrier validates and deduplicates all selected OKF
surfaces into package-level lifecycle identities bound to the exact package
digest, manifest digest, and generation. It acquires a published Registry lease
for every identity and holds the complete set through SQLite search and the
final Registry revision check. A query accepted before cutover therefore joins
the same lifecycle drain as route dispatch and prevents prior-generation
retirement until its guard drops. Missing exact publication, conflicting
digests for one package generation, or a second racing revision fails closed.

When the projection set becomes non-empty, attached and newly created TUI
sessions receive the dynamic read-only `use_knowledge_search` tool. It is
withdrawn when no managed Knowledge remains. Results carry the exact package
surface, lifecycle generation, package/bundle/receipt/index digests, concept
identity, source path, and source digest. Package content is treated as
untrusted data, never as instructions.

## 8. A3S Code integration

### 8.1 Lifecycle factory

`CodeCognitivePackageLifecycleFactory` delegates one admitted package
generation to A3S Use's shared `ManagedCognitivePackageLifecycleFactory`:

- the built-in package-bound native lifecycle host for executable Tool Tasks
  and stdio MCP;
- `A3sFlowLifecycleHost` for packaged A3S Flow;
- `OkfKnowledgeLifecycleHost` backed by the scope-aware Use SQLite/FTS5
  adapter and durable binding store;
- `StaticPluginSurfaceLifecycleHost` for Skill and UI;
- the exact host-reviewed Runtime selection for release-backed Tasks and
  Services; and
- a Gateway lifecycle port that consumes Runtime-published typed endpoints,
  drains before Runtime stop, and removes the route before Runtime removal.

`PluginRuntimeHost` owns the Runtime client registry, the explicit
package/surface-to-provider assignments, and the Gateway readiness adapter.
Package metadata supplies provider-neutral workload templates and cannot name
the provider. The default Code composition has an empty registry and assignment
set plus an explicitly unavailable Gateway port. A trusted user config or
explicit `--config` file may opt into one typed Linux Box block:

```acl
plugin_runtime {
  schema = "a3s.plugin-runtime-host.v1"

  box {
    isolation = "microvm"
    control_timeout_ms = 60000
    task_poll_interval_ms = 50
  }

  gateway {
    address = "127.0.0.1:43129"
  }
}
```

The Box block assigns release-backed Tool Tasks. When the sibling Gateway block
is present, Code starts one host-owned Gateway on that dedicated numeric
loopback socket and assigns the same provider to Tool Services and Streamable
HTTP MCP. The Gateway state file is absolute and durable under the component
state root. Its target UUID is reconstructed from scope, package/surface,
Runtime unit, and generation fields retained by the final receipt. Tool health
uses the reviewed Runtime plan; MCP readiness performs the standard
initialize/initialized exchange through the returned private route with no
ambient proxy or redirects. Retirement closes admission, drains accepted HTTP,
gRPC, WebSocket, and TCP work, removes only the exact receipt-owned binding,
and then permits Runtime removal. CLI and TUI exit paths explicitly shut down
the listener and release the state owner lock. Exactly one process owns that
state and listener at a time; a second configured host fails closed until the
owner exits instead of opening a divergent route store.

`microvm` never falls back to shared-kernel execution; `sandbox` must be
selected explicitly. Workspace ACL is not provider or Gateway authority,
omitting the Gateway keeps Services unassigned, and unsupported platforms fail
closed.

The reviewed activation path derives candidate generations from the exact package
lock, resolves every managed surface through the host assignments, probes the
current Runtime capabilities, and binds Grant plus provider evidence. Policy
evaluates the complete plan; A3S Use then rebuilds that evidence with the final
authority and rejects changes to provider identity/build, capabilities,
workload semantics, enforcement, authority, scope, or revision. The v3
operation record retains the canonical Registry source revision, signed
planning bundles, exact Grant snapshot, and reviewed provider evidence.

Apply reconstructs the Grants, lifecycle generations, assignments, and
provider selection from the durable record and current host registry. It must
exactly match the reviewed evidence before the lifecycle factory is created or
the package archive is downloaded. Reviewed enablement persists a separate
schema-v2 record containing the installed signed planning bundle, exact Grant
snapshot, and provider generations. Disable reconstructs no candidate
selection and retirement follows the durable Runtime binding receipt; re-enable
reconstructs activation from the retained bundle without Registry refetch. An
injected fake Runtime proves this across separate manager instances, including
provider-build drift rejection, stopped-binding replacement when authorization
semantics change, recovery, and replay. The opt-in Linux host now constructs
the shared Box driver over the durable Runtime state store and derives exact
Task and Service assignments when its private Gateway is configured. A Linux
qualification now proves retained N/N+1 Tool and MCP processes, private route
recovery across Gateway and lifecycle-host restart, independent Tool and MCP
provider-process loss, same-Runtime-generation endpoint rebinding, sibling
isolation, exact drain/removal, and zero residual process or Runtime state.
Non-Linux providers and cross-platform recovery still need qualification.

Runtime Task execution no longer depends on the retained operation-plan
record. A schema-v4 binding stores the argument-free reviewed Runtime unit
template together with its exact Grant, provider build, capability,
enforcement, semantics, scope, package digest, surface, and generation
evidence. `PluginManager::invoke_runtime_task` opens the canonical Code-owned
Use data/state roots, acquires the exact published-generation Registry lease,
loads that exact binding, and reconnects the recorded provider. The invocation
contributes only a unique invocation ID and bounded argv. The lease remains
held through output capture and Runtime removal; hide rejects later calls and
drain waits for already accepted work. A host regression proves this before
and after Manager reconstruction. Current assignments are deliberately not
consulted during dispatch, so an old UI or agent request cannot jump to a new
package generation.

Capability Registry schema v2 closes the session projection boundary. Each
published `toolTasks` entry carries a stable `use_tool_*` name, exact lifecycle
identity, complete `PlanScope`, surface, timeout, JSON-output contract, and the
reviewed provider ID. TUI watchers reuse the same process-level
`Arc<PluginManager>`, register the Task with conservative scheduling
capabilities, accept only a bounded argv array, and dispatch the snapshot's
exact identity through the receipt-owned Manager path. They never resolve the
current assignment at call time. Invalid JSON output fails the tool call.

Projection still fails closed at the production boundary. If the host lacks
the named reviewed provider, Code records an explicit warning and registers no
tool. Upgrade or disable withdraws the old dynamic tool before a replacement
can be registered, and every attached TUI session observes the same
transition. The default Code host has no release-backed production Runtime
provider, so this completed projection plumbing does not make OCI Tasks
production-available by itself.

For every host-reviewed mutation, CLI and TUI call the package manager
through `ReviewedCognitivePackageAuthorizationProvider` in the same process. The
provider accepts only the stored envelope and confirmation, so Use must
reproduce the operation identity, package/impact/state evidence, authority,
and dependency locks exactly. Registry records are reconstructed by stable
name only when their current URL and trust root still equal the lock. The
current protocol requires catalog-v3 evidence and a complete package lock;
planning rejects older, incomplete, or unlocked input, and apply has no child
process mutation path.

The interactive CLI and TUI consume one shared, deterministic read-only
projection derived directly from that envelope. It preserves the exact plan
identity and digest, candidate and prior lock nodes and dependency edges,
Registry/archive or retained installed source evidence, every package
transition and complete permission ceiling, secret/provider/Workspace impact,
operation impact and state, and the actor/policy confirmation boundary. It does
not replace or alter the standard manager JSON contracts.

Before binding a locked graph plan, the host reads the A3S Use Grant snapshot
for the exact User/Workspace scope and durable planner revision. The delegated
draft must be unbound and contain no provider evidence. A3S Use composes the
Grant and provider preflight from the host assignments, the host evaluates the
complete plan, and Use repeats composition with the final authority. Scope,
revision, prebound impact/provider evidence, authority, provider build,
capability, semantic, or enforcement drift is rejected. At apply, A3S Use
re-derives the same Grant change set and owns prepare, receipt cutover, replay,
and package-graph coordination; the parent records only its exact lifecycle
binding and capability cutover.

Local CLI/TUI enablement first inspects the installed receipt. A schema-v3
package must be planned through the same in-process manager, User scope, host
policy, and Use-owned mutable package-state generation used by the managed
host. `NoChange` has no operation ID, digest, or mutation plan. A planned result
persists the complete envelope before it is displayed; apply accepts only its
operation ID and canonical digest after the trusted adapter collects exact
confirmation. Policy, expiry, confirmation, digest, and generation are checked
before durable intent. Once intent exists, recovery follows its recorded
evidence and completed results replay unchanged after process restart.
Permission-bearing packages use the same reviewed Grant saga rather than a
second toggle path. Only current schema-v3 receipts are accepted by this
surface.

### 8.2 Live projection

Every TUI process keeps one resident typed Use Registry watcher. It validates
the complete snapshot cursor, generation, capability revision, Registry
revision, exact package generations, package roots, lifecycle generations,
surface paths, SHA-256, media types, dependency IDs, and source bytes before
constructing projections. The command JSON envelope remains schema v1 and its
serialized capability Registry remains schema v2 for status, diagnostics, MCP
serving, and non-resident commands. Code validates those layers independently
and rejects Registry v1.

One stable resident snapshot builds one complete Core `SessionCapabilityBatch`
for the verified managed MCP, Skill, eligible provider-qualified Runtime Tool
Task, digest-bound Knowledge Surface, dependency-closed Flow, and UI set.
Preparation validates every
descriptor and runtime value;
publication commits the Code projection and its generation-specific Use lease
provider together. The provider holds the immutable snapshot and cursor but no
acquired lease. Each Run admission calls back into the resident Use Registry
for a new non-clone `CapabilitySnapshotLease`, verifies its generation,
capability revision, and Registry revision, and retains it until children,
tasks, effects, and Run teardown have settled. Consequently, an admitted N Run
keeps its N MCP/Skill/Tool registry and upstream lease after N+1 publication,
while a projected UI handle likewise retains N's path-free document. New Runs
and UI handles can admit only against N+1. A stale or hidden cursor fails
admission.

One-shot Code Exec uses the same Use authority with a shorter structured
lifetime. Its scoped host enables atomic managed-MCP/Skill/Runtime-Task/UI
projection, freezes a ready receipt, cancels and joins Registry discovery, and
verifies that the Session catalog and exact Task fingerprints still equal the
committed receipt before the first Run. The result binds
`a3s.code.scoped-capability-runtime.v1` to the Code catalog generation/digest,
the complete `a3s.use.capability-snapshot-cursor.v1` cursor, surface counts, and
a bounded Runtime Task count/digest. One process-owned Plugin Manager supplies
both the leased exact-generation Task dispatcher and the typed HTTP MCP
resolver. The resolver accepts only opaque provider, endpoint-reference, and
path evidence and returns only a credential-free numeric loopback route. The
Manager remains alive until Code closes the Session and its projected clients;
bounded Runtime/Gateway shutdown follows on normal completion, cancellation,
and failure. Built-in MCP, compatibility Knowledge, Flow, and Plugin Manager
presentation projection are not started by this host cut.

Ordinary CLI execution discovers only an already-ready Use component and does
not install it. If an optional installation is incompatible, fallback is legal
only after its watcher has fully stopped and both the Session capability and
dynamic-tool catalogs are unchanged.
A3S Desktop instead negotiates the exact reserved `scoped-v1` flag, permits the
normal policy-bounded first-use setup, and rejects a successful Code result
without well-formed frozen evidence. Required setup failure occurs before model
egress.

OCR is temporarily merged from the process snapshot as a non-leased host
built-in because its current ONNX Runtime ABI conflicts with the CLI's optional
local-embedding ABI. Its verified Skill still enters the same Core batch.
Atomic projection identity therefore includes both the typed Use cursor and
the verified managed-MCP, Skill, Runtime Tool, Knowledge Surface, Flow, and UI fingerprints, so a host overlay or
surface change cannot be skipped when the typed cursor is unchanged.

The watcher currently exposes:

- exact managed MCP projections plus built-in standard MCP routes;
- verified package Skills;
- bounded path-free UI bindings with canonical Skill/Tool/MCP/Flow dependency evidence;
- a typed exact-generation A3S Flow catalog;
- exact promoted OKF projections, their scope-aware query catalog, and
  canonical same-package `KnowledgeSurfaceBinding` readiness aggregated across scopes;
- the dynamic read-only `use_knowledge_search` tool while Knowledge is active;
  and
- provider-qualified Runtime Tasks as conservative `use_tool_*` tools carrying
  exact lifecycle identity, scope, and surface evidence.

Runtime Task projection is conditional rather than optimistic. The watcher
skips a Task when the process has no matching reviewed provider, reports that
condition, and does not fall back to another provider. The default host remains
empty; the explicit Linux Box block supplies only the named `a3s-box` provider.

Extension MCP values, Knowledge Surface readiness, and every dependency-closed Flow execution now join the resident atomic
batch. MCP preparation completes connection, initialize, and tool discovery
before publication, and cancellation or rollback closes only the staged
candidate. Built-in MCP wrappers and the dynamic Knowledge search tool remain
explicit compatibility projections with their existing typed readiness,
withdrawal, and exact-call lease owners. A Knowledge Surface is deliberately
non-queryable and cannot select a cognitive package. A missing
dependency fails the candidate batch rather than publishing an incomplete graph.

Generation replacement publishes the complete new managed-MCP/Skill/Runtime
Tool/Knowledge-Surface/Flow/UI catalog without rewriting an admitted Run. Compatibility
adapters withdraw or replace their own callable surfaces according to their
lifecycle. A running call settles under the boundary it was admitted with; new
calls see only the new generation.
Knowledge queries additionally verify that the Registry revision stayed current
for the full cited search.

### 8.3 Dedicated Use worker

The stable worker identity is `use`:

- only `mcp__use_*` and `use_tool_*` tools are visible to it;
- workspace, shell, unrelated MCP, and recursive task tools are denied;
- verified package Skills provide guidance but cannot expand permission;
- missing annotations, mutations, destructive operations, and open-world
  access return to the parent confirmation stream; and
- application failures do not fall back to another provider.

The parent model sees the worker and its current capability IDs through the
live `task` definition but does not receive the raw Use tool set.

### 8.4 TUI adapter

Code constructs the shared Plugin Manager from the effective Code ACL path,
workspace, immutable offline policy, and separately host-selected authorization
policy. Automatically discovered workspace ACL may configure Code but cannot
authorize plugin mutation. Initialization failure is non-fatal to the editor
but fail-closed for package mutation and is reported when `/packages` opens.

`/packages` is idle-only and pages the standard manager installed-package
contract. Its rows keep Use-owned desired state separate from observed state.
The selected package first enters planning, never mutation. A planned
`PluginHostEnablementPlanResult` must preserve the requested package and state
and exposes its complete operation ID, plan digest, expected package generation,
expiry, package graph, source, transition, complete permission ceiling,
provider, impact, state, and confirmation boundary for review. Up and Down
scroll every wrapped evidence line, including on narrow terminals. Enter/y
supplies exact user confirmation and applies only that identity; Esc/n cancels
it. `NoChange` has no operation identity and refreshes directly, while an
in-flight apply locks the panel until its exact typed result arrives. The
process-level Use watcher remains the only owner of capability withdrawal,
drain, and restoration after generation change. The unrelated `/plugin` panel
continues to toggle local Claude/Codex Skills only. Managed OKF packages appear
as a read-only session tool only while their exact projections remain active.

## 9. Component and built-in domain boundaries

Browser, Office, and OCR remain typed domains rather than generic JSON actions:

- Browser owns providers, lifecycle, rendering, sessions, snapshots, and
  interactions. Search depends on the Browser Rust interface rather than
  spawning the Use CLI.
- Office preserves OfficeCLI native commands and standard MCP. Outcome-unknown
  mutations are never retried automatically.
- OCR exposes the local PP-OCRv6 provider, pinned model lifecycle, MCP tools,
  and a content-bound Skill.

Use preserves native `argv`, stdin, stdout, stderr, process status, HTTP, and
standard MCP. It does not invent a universal action envelope and does not load
untrusted package code through Rust's unstable dynamic-library ABI.

## 10. Public command surface

```bash
a3s list
a3s info use/ocr --sources
a3s install use --source release
a3s install use/ocr
a3s upgrade use/acme/research
a3s uninstall use/acme/research
a3s doctor use

a3s use browser render https://example.com
a3s use office validate report.docx
a3s use ocr extract scan.png --json

a3s plugin search research
a3s plugin inspect acme/research
a3s plugin install acme/research
a3s plugin upgrade acme/research
a3s plugin uninstall acme/research
```

Code TUI and a required Desktop Code Exec may install verified Use product
releases when networking and first-use setup are allowed. The TUI may also
install WebView. TUI Use preparation runs concurrently with the remaining
terminal startup and hot-plugs its registry when ready, while WebView
preparation retains its pre-terminal lifecycle. Managed component probes remain
bounded but allow for one-time executable scanning on macOS and Windows. The
cross-platform first-turn E2E waits for the attached registry revision after
the ordinary startup budget, without changing interactive TUI startup.
`A3S_NO_AUTO_INSTALL=1` and offline mode disable first-use product installation
for CI and hermetic environments. Third-party capability runtimes require an
explicit install or interactive confirmation; non-interactive library calls do
not download them implicitly.

## 11. Verification evidence

Focused host gates:

```bash
cargo test --lib use_registry::tests:: --no-fail-fast
cargo test --test code_use_first_use scoped_exec::
cargo test --test code_exec
cargo test --bin a3s tui::panels::packages::tests --no-fail-fast
cargo test --test plugin_commands
cargo test --test plugin_manager_mcp
cargo test --test remote_registry_components
cargo test --lib \
  generation_watch_hot_plugs_skill_mcp_runtime_task_flow_and_knowledge_across_tui_replacement
cargo test --lib \
  use_registry::runtime_tasks::tests --no-fail-fast
cargo test --bin a3s \
  bound_flow_deploy_resolves_fake_use_catalog_before_os_mutation
cargo test --bin a3s \
  bound_flow_run_status_and_logs_share_local_durable_runtime_without_os
cargo test --bin a3s \
  tui_routes_run_status_and_logs_locally_without_os_authority
cargo test --lib \
  code_host_preflights_flow_and_persists_exact_generation_binding
cargo test --lib \
  signed_okf_install_upgrade_restart_query_and_uninstall_use_code_host
cargo test --lib \
  reviewed_apply_uses_the_in_process_adapter_and_preserves_host_authority
cargo test --lib \
  signed_workspace_install_is_exact_fenced_and_replayable_after_restart
cargo test --lib plugin_manager::operation
cargo test --lib plugin_manager::managed_host
cargo test --lib components::cognitive_lifecycle
```

These prove:

- strict source/path/digest/dependency validation;
- exact umbrella-to-Use operation/confirmation forwarding without a child
  mutation process, plus Registry lock-drift rejection and durable replay;
- provider-neutral delegated drafts, explicit host Runtime assignment before
  archive download, two-pass Grant/provider binding, durable planning-bundle
  and Grant-snapshot evidence, exact apply-time provider reconstruction,
  provider-build drift rejection before lifecycle mutation, and successful OCI
  Task install/replay after restoring the reviewed build;
- exact remote Workspace capability, assignment, candidate, lock, surface,
  confirmation, and fence binding; local/managed plan separation; and durable
  result replay after host recreation;
- durable managed enablement intent before mutation, Use-owned package-state
  generation, non-destructive disable/enable, restart replay, operation-ID
  conflict rejection, and stale-generation rejection;
- local CLI/TUI schema-v3 enablement uses an immutable User-scoped plan and
  the same Use-owned state/Grant saga, never launches a child mutation, renders
  the complete TUI review identity, and preserves exact Skill/UI/Flow replay;
- exact Flow compiler preflight and persisted generation binding;
- signed OKF install, restart recovery, exact upgrade cutover, stale-generation
  withdrawal, receipt-owned uninstall, cited retrieval, scope-isolated usage
  accounting, quota release, tombstone retention, and page reclamation;
- identical textual scope IDs remain isolated by User/Workspace kind, and
  ambiguous multi-scope or multi-generation projection fails closed;
- TUI and replacement sessions hot-plug and withdraw the same managed
  Knowledge tool with exact Registry and lifecycle generations;
- real native Use snapshot leases are independently acquired per Run, stale
  providers reject new admission, and lifecycle drain waits for the final
  retained lease;
- an active Code Run keeps its N Skill search registry and real Use lease
  across N+1 publication, while the next Run resolves only N+1;
- scoped Code Exec freezes and joins its managed-MCP/Skill/Runtime-Task/UI
  watcher before provider egress, reports exact Code, Use generation, surface,
  and Task catalog evidence, never starts built-in MCP/Knowledge/Flow, and
  performs no implicit install in ordinary installed-only mode;
- required scoped execution fails before provider egress when offline or
  no-auto-install policy leaves Use unavailable, while an incompatible
  optional Use can be skipped only with unchanged capability/dynamic-tool
  catalogs and completed-watcher evidence;
- same-cursor host Skill changes republish through host fingerprints, and
  projected Skills never enter the mutable compatibility registry;
- the same watcher projects a provider-qualified Runtime Task into TUI,
  replacement, and frozen scoped agent sessions, withdraws it on disable,
  restores it on re-enable, and never registers it when the reviewed provider
  is absent;
- Runtime Task calls preserve exact package/manifest digests, lifecycle
  generation, scope, surface, and argv, use conservative scheduling
  capabilities, reject invalid declared JSON output, and replace a same-name
  upgraded tool with the new exact generation;
- accepted Knowledge queries hold exact published package-generation leases
  through backend search and revision verification; missing leases prevent
  backend invocation, repeated surfaces deduplicate, and conflicting package
  evidence fails closed;
- exact `flow.json` resolution before OS mutation, path-free binding evidence,
  and stale-generation rejection;
- source drift and symlink substitution fail before compiler/event mutation;
- CLI/TUI share local Run/Status/Logs behavior without OS authority;
- idempotent runs and event history survive runtime recreation, package
  upgrade, and uninstall without leaking managed paths;
- TUI `/use` readiness changes after watcher updates; and
- the Flow catalog fails closed when Use is unavailable.

The monorepo `just use-hotplug-e2e` gate crosses an independently built real
`a3s-use` binary boundary on Linux, macOS, and Windows for Code watcher
convergence, platform-native TUI first-use installation, signed package
install/restart/upgrade/uninstall without retained paths or package residue,
and host-injected managed Runtime/Grant planning, drift rejection, cutover,
durable Grant observation, enablement recovery, and terminal replay.
The local managed Knowledge path is covered in-process with real signed package
and SQLite evidence, including exact query leases, scope quota, and
tombstone/physical GC. Runtime Task projection is covered with a schema-v2
watch fixture and recording reviewed provider across TUI sessions;
production-provider execution is not inferred from that fixture. The Linux
real-process Runtime Service gate covers retained-generation upgrade,
independent Tool and MCP provider-process loss, same-generation endpoint
rebinding, and uninstall without inferring privileged OCI or MicroVM
conformance. Complete cross-platform graph E2E, managed rollback, coordinated
restore, and non-Linux Runtime provider composition remain release gates.

## 12. Platform scope

| Platform | Status |
| --- | --- |
| macOS arm64 / x86_64 | Supported release and managed-component target. |
| Linux arm64 / x86_64 | Supported release and managed-component target. |
| WSL | Supported through the Linux contract. |
| Windows x86_64 | Preview: native A3S Code with WebView, verified Use ZIP first-use, signed package lifecycle, Edge core Browser profile, Office MCP operations, and local OCR E2E; complete failure-injection parity remains gated. |

## 13. Release gates

The cognitive package line is not complete until all of the following pass:

- managed Knowledge rollback, coordinated restore/authority recovery, backup
  rotation, and distributed placement where required; exact query leases,
  scope quota, bounded retention, tombstone GC, integrity audit, verified
  backup, and derived-index repair are implemented;
- complete production Runtime assignments, readiness, and Gateway injection
  for OCI Tasks, Runtime Services, and HTTP MCP. The explicit Linux Box Task
  composition is implemented. The injected OCI Task path covers
  install plus offline restart-safe disable/re-enable and drift recovery; exact
  receipt-owned retirement is implemented. Linux real-process retained N/N+1
  Service upgrade/uninstall, independent Tool and MCP provider-process loss,
  same-generation endpoint rebinding, sibling isolation, and zero-residue
  cleanup are qualified; privileged provider conformance, non-Linux providers,
  and cross-platform recovery remain gates;
- distributed Flow worker placement, automatic scheduling/resumption of waits
  and retries, and production retention/GC for resolved installed identities;
- reviewed Activity backend bindings and generation-aware readiness/sandbox
  composition in CLI, TUI, and native hosts;
- prior-generation drain, retirement, rollback, and garbage collection;
- real signed reviewed enablement and dependency-graph
  install/upgrade/uninstall across Code TUI on supported platforms;
- real-registry multi-root shared-dependency upgrade/uninstall across supported
  platforms;
- crash injection at every parent/child saga boundary; and
- no leaked process, socket, lock, temporary file, grant, binding, route, or
  package generation after failure and recovery.

## 14. Related documents

- [A3S CLI Product Design](cli-product-design.md)
- [A3S CLI Technical Architecture](cli-technical-architecture.md)
- [Plugin Authorization Policy](plugin-authorization-policy.md)
- [Component Management Design](component-management-design.md)
- [Cross-Platform Install Product Design](cross-platform-install-product.md)
- [Cross-Platform Install Technical Architecture](cross-platform-install-architecture.md)
- [A3S Use package architecture](https://github.com/A3S-Lab/Use/blob/main/docs/plugin-platform-architecture.md)
- [Cognitive package lifecycle saga](https://github.com/A3S-Lab/Use/blob/main/docs/adr-002-cognitive-package-lifecycle-saga.md)
