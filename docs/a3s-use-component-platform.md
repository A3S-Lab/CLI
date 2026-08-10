# A3S Use and Component Platform

Status: Active implementation, pre-1.0

Updated: 2026-08-09

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
Plugin Manager for CLI, Web, and management MCP entry points. No adapter may
create a second package lifecycle.

## 2. Product hierarchy

```text
A3S product components
├── code                         bundled with a3s
├── web                          release/Cargo-managed Code frontend
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
| Host Plugin Manager | One manager serves CLI, TUI, Web, read-only management MCP planning, and the canonical fenced remote `PluginHostManager`. It admits only provider-neutral drafts, binds actor and exact User/Workspace scope, resolves signed planning bundles through host-owned Runtime assignments, performs two-pass Grant/provider binding around full-plan policy evaluation, and persists confirmation, intent, lifecycle cutover, and replay evidence. |
| Reviewed Use authorization bridge | Complete schema-v3 install plans execute through an in-process reviewed provider. The exact host envelope, scope, signed planning bundles, durable Grant snapshot/revision, reviewed provider evidence, and confirmation reach Use without argv/environment authority. Apply reconstructs the same Grants, generations, assignments, and selection before download or mutation; Registry identity and provider evidence drift fail closed. |
| Code runtime composition | Code delegates to the shared Use managed lifecycle factory for executable native Tool Tasks, stdio MCP, Skill, UI, workspace-local `a3s-flow` Native TypeScript execution, scope-aware SQLite/FTS5 OKF Knowledge, and typed Runtime/Gateway retirement. The default Code `PluginRuntimeHost` has no release-backed Runtime assignments or Gateway readiness adapter. |
| Code Flow catalog | Available through the exact-generation Use watcher and `GET /api/v1/plugins/flows`. |
| Code `flow.json` identity | Implemented for TUI, non-resident CLI, and Web: typed designs resolve an exact package/Flow/version/lifecycle-generation/source-digest tuple before runtime mutation. |
| Code OKF composition | Available through the real Use Knowledge lifecycle host: bounded OKF inspection, stage/promotion/removal, receipt-accounted scope quota, bounded generations/tombstones, SQLite/WAL compaction, durable exact-generation bindings, restart recovery, integrity audit, derived-index repair, versioned backup/offline verification, watched TUI/Web projection, cited scope-bound retrieval, and exact published-generation query leases held through backend search and Registry revision verification. |
| Code Web Activity composition | Enabled catalog items expose an exact generation/revision-bound document URL. The raw HTML response inlines only digest-verified package assets and enforces an opaque-origin CSP, no connections/frames/objects/forms/base URLs, same-origin embedding/resource policy, disabled browser permissions, no referrer, no cache, and MIME sniffing denial. Stale URLs return `410 Gone`; disabled or missing current-generation items return `404 Not Found`. The browser adopts only that URL, transfers a dedicated v3 `MessagePort`, ignores ambient messages, terminates self-navigation, identity-binds context review and bounded durable state, and drains/replaces the old frame on Registry changes. State operations hold exact published-generation leases and use durable scope/package/surface namespaces with explicit retained-surface cleanup. A hidden authority-free sandbox proves exact N+1 readiness before cutover; failure preserves N, rolls N+1 back without residue, and terminalizes the reviewed plan, while a fresh plan can retry the same lifecycle generation. Backend bindings, equivalent readiness outside Code Web, and native hosting remain open. |
| Code managed Runtime surfaces | Typed Runtime selection and endpoint/retirement contracts are composed. A host-injected deterministic provider proves signed OCI Tool Task install, retained planning-bundle recovery, offline restart-safe disable/re-enable, exact apply-time build reconstruction, drift rejection, Grant persistence, stopped-binding reauthorization, and replay. The Linux/macOS/Windows monorepo gate observes the same operation through an independently built exact-revision `a3s-use` process instead of granting authority to the standalone CLI. Disable and uninstall require no candidate provider selection; retirement resolves the exact provider from its binding receipt. Default-host OCI Tasks, Tool Services, HTTP MCP, and the real-provider cross-platform uninstall/upgrade matrix still require production composition. |
| Hot-plug integration | TUI and detached Web process tests cover disable and re-enable generation changes. TUI `/packages` adds an idle-only exact-plan review/confirmation surface; a signed schema-v3 Web regression covers reviewed disable, daemon restart, exact apply replay, `NoChange`, enable, and Activity withdrawal/restoration. Web also executes installed and upgraded Flow generations, retains their histories after uninstall, and recovers them after daemon restart. A real SQLite Knowledge test now proves tool/catalog withdrawal and restoration across TUI, replacement, and Web sessions, plus exact cited Web search. Query-carrier regressions prove lease lifetime, package-generation deduplication, and fail-closed missing or conflicting lease evidence. |
| Remaining release gates | Managed Knowledge rollback, coordinated restore and authority recovery, backup rotation, production Runtime/Gateway providers, real-provider retained-generation upgrade/uninstall validation, distributed Flow and Knowledge placement, and complete real-process cross-platform E2E. |

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
         exact-generation capability snapshot/watch
                         │
              ┌──────────┴──────────┐
              ▼                     ▼
        A3S Code TUI            A3S Code Web
   /use + /flow + OKF   Marketplace + Flow/OKF API
              │                     │
              └──── local a3s-flow ─┘
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
| A3S Code | Agent sessions, TUI/Web presentation, dedicated Use worker, live snapshot consumption, workspace-local composition of the sole A3S Flow engine, and read-only exact-generation Knowledge query projection | A second Registry, resolver, package journal, workflow engine, or Knowledge lifecycle |

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

The live Web catalog is:

```http
GET /api/v1/plugins/flows
```

It returns the Use snapshot generation/revision and typed, content-bound,
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

Resident TUI and Web paths use the existing watched snapshot. A non-resident
`a3s code flow run` performs the same stable two-snapshot Use inspection and
source verification without starting a watcher. CLI and TUI status/logs read
the durable binding and event store without requiring Use or OS, so history
survives package upgrade, disable, or uninstall. Web routes are:

```http
GET  /api/v1/plugins/flows
POST /api/v1/plugins/flows/resolve
POST /api/v1/plugins/flows/run
GET  /api/v1/plugins/flows/runs
GET  /api/v1/plugins/flows/runs/{runId}
GET  /api/v1/plugins/flows/runs/{runId}/events
```

The current Web integration is API-level. Marketplace continues to provide
package catalog/lifecycle and Activity UI, while visible Flow run/history
controls remain a product-adapter gate. CLI and TUI are the current interactive
local execution surfaces.

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

When the projection set becomes non-empty, attached and newly created TUI/Web
sessions receive the dynamic read-only `use_knowledge_search` tool. It is
withdrawn when no managed Knowledge remains. Results carry the exact package
surface, lifecycle generation, package/bundle/receipt/index digests, concept
identity, source path, and source digest. Package content is treated as
untrusted data, never as instructions.

Code Web exposes the same carrier without introducing a second lifecycle:

```http
GET  /api/v1/knowledge/packages
POST /api/v1/knowledge/packages/search
```

The catalog and search envelope include the live Registry generation and
revision. `scopeKind` and `scopeId` must be supplied together and are required
when more than one scope is active. This managed package path is distinct from
the personal `/kb` authoring and compilation vault.

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
set plus an explicitly unavailable Gateway port. It therefore rejects every
release-backed managed surface instead of silently selecting a provider.

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
semantics change, recovery, and replay. The default Code host still has no
production provider or Gateway adapter, and retained-package real-provider
upgrade/uninstall needs cross-platform qualification.

For every host-reviewed mutation, CLI, TUI, and Web call the package manager
through `ReviewedCognitivePackageAuthorizationProvider` in the same process. The
provider accepts only the stored envelope and confirmation, so Use must
reproduce the operation identity, package/impact/state evidence, authority,
and dependency locks exactly. Registry records are reconstructed by stable
name only when their current URL and trust root still equal the lock. The
current protocol requires catalog-v3 evidence and a complete package lock;
planning rejects older, incomplete, or unlocked input, and apply has no child
process mutation path.

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

Local CLI/TUI/Web enablement first inspects the installed receipt. A schema-v3
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

Every TUI/Web process keeps one Use snapshot watcher. It validates generation,
revision, package root, lifecycle generation, surface paths, SHA-256, media
types, dependency IDs, and source bytes before constructing projections.

The watcher currently exposes:

- standard MCP routes;
- verified package Skills;
- sandboxed Web Activity entries/assets; and
- a typed exact-generation A3S Flow catalog;
- exact promoted OKF projections and their scope-aware catalog; and
- the dynamic read-only `use_knowledge_search` tool while Knowledge is active.

Generation replacement withdraws old callable surfaces before draining their
connections. A running call settles under the boundary it was admitted with;
new calls see only the new generation. Knowledge queries additionally verify
that the Registry revision stayed current for the full cited search.

### 8.3 Dedicated Use worker

The stable worker identity is `use`:

- only `mcp__use_*` tools are visible to it;
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

`/packages` is idle-only and reads an authoritative installation snapshot. The
selected package first enters planning, never mutation. A planned schema-v3
result must preserve the requested component and state and exposes its complete
operation ID, canonical digest, expected package generation, and expiry for
review. Enter/y applies only that identity; Esc/n cancels it. `NoChange` has no
operation identity and refreshes directly, while an in-flight apply locks the
panel until its exact result arrives. The process-level Use watcher remains the
only owner of capability withdrawal, drain, and restoration after generation
change. The unrelated `/plugin` panel continues to toggle local Claude/Codex
Skills only. Managed OKF packages appear as a read-only session tool only while
their exact projections remain active.

### 8.5 Web adapter

Code Web stores the startup-created `Arc<PluginManager>` and every plugin route
clones it, preserving one policy and process-local lock inside that Web host.
TUI, Web, and management MCP processes also retain the same durable file-lock
boundary. Detached Web and the management MCP verify a digest-locked handoff of
the operator-selected policy source. Web instance reuse requires that exact
digest and offline mode; workspace ACL is never promoted to authorization. Code
Web also reads the same Use watcher. Relevant routes include:

```text
GET  /api/v1/plugins/marketplace
GET  /api/v1/plugins/activities
GET  /api/v1/plugins/activities/{key}/document?generation={generation}&revision={revision}
GET  /api/v1/plugins/flows
POST /api/v1/plugins/flows/resolve
POST /api/v1/plugins/flows/run
GET  /api/v1/plugins/flows/runs
GET  /api/v1/plugins/flows/runs/{runId}
GET  /api/v1/plugins/flows/runs/{runId}/events
GET  /api/v1/knowledge/packages
POST /api/v1/knowledge/packages/search
POST /api/v1/plugins/operations/plan
POST /api/v1/plugins/operations/apply
POST /api/v1/plugins/packages/enablement/plan
POST /api/v1/plugins/packages/enablement/apply
```

`POST /api/v1/plugins/packages/enablement/plan` accepts `componentId`,
`enabled`, and an optional optimistic `expectedPackageGeneration`.
`POST /api/v1/plugins/packages/enablement/apply` accepts only the returned
`operationId` and `planDigest`; `/operations/apply` accepts the same reviewed
enablement identity. The response preserves Use's state, generation, replay
flag, and operation-result digest. There is no direct package-toggle endpoint.

The Activity catalog adds `documentUrl` only to enabled items. That URL names
the exact Registry generation and revision; the server resolves the identity
and bytes from one immutable watcher snapshot, survives a process restart at
the same identity, and returns `410 Gone` once a newer generation is current.
The response inlines only verified HTML/CSS/JS and enforces the server-side
sandbox headers. `GET /activities/{key}` remains non-executable management
JSON. The product browser accepts only the exact catalog URL, renders it in an
opaque-origin script-only iframe, and transfers a fresh `a3s.activity.v3`
`MessagePort` after the first load. Ambient window messages are ignored; a
second load terminates the frame; unmount, retry, and Registry identity changes
close the old port. Every message and reviewed context proposal is bound to the
current key, generation, revision, and document URL. State get/set/delete/clear
requests are bounded, serialized, and forwarded only while an exact published
lifecycle-generation lease is held. Code stores them outside the iframe origin
under the canonical User scope, lifecycle package ID, and surface ID. This is
not yet a complete cross-host UI platform: reviewed Tool/MCP/Flow bindings,
equivalent candidate readiness in CLI/TUI/native hosts, and native UI
composition remain open. Code Web itself now loads exact path-free N+1 bytes in
a hidden script-only sandbox, transfers only readiness-mode identity over a
dedicated port, and accepts only `activity.ready`. Load, navigation, protocol,
or deadline failure keeps N callable and rolls N+1 back without receipt or
generation residue. The failed plan cannot republish; a fresh reviewed plan
may retry the same lifecycle generation before one successful cutover.

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

Code TUI may install verified Use and WebView product releases when networking
and first-use setup are allowed. Use preparation runs concurrently with the
remaining terminal startup and hot-plugs its registry when ready, while WebView
preparation retains its pre-terminal lifecycle. Managed component probes remain
bounded but allow for one-time executable scanning on macOS and Windows. The
cross-platform first-turn E2E waits for the attached registry revision after
the ordinary startup budget, without changing interactive TUI startup. Code
Web applies the same non-blocking first-use policy for Use: it serves the
browser immediately, then
hot-plugs the verified registry into existing and new Web sessions when ready.
`A3S_NO_AUTO_INSTALL=1` and offline mode disable first-use product installation
for CI and hermetic environments. Third-party capability runtimes require an
explicit install or interactive confirmation; non-interactive library calls do
not download them implicitly.

## 11. Verification evidence

Focused host gates:

```bash
cargo test --lib use_registry::tests:: --no-fail-fast
cargo test --bin a3s tui::panels::packages::tests --no-fail-fast
cargo test --test web_cli \
  plugin_api_exposes_catalog_and_fails_closed_without_trust_roots
cargo test --test web_plugin_marketplace \
  marketplace_install_upgrade_uninstall_hot_plugs_verified_activity_skill_and_flow_catalog
cargo test --test web_plugin_marketplace \
  reviewed_enablement_hot_plugs_skill_ui_and_flow_and_replays_after_restart
cargo test --lib \
  generation_watch_hot_plugs_skill_mcp_flow_and_knowledge_across_tui_and_web
cargo test --bin a3s \
  complete_code_web_module_builds_with_nested_remote_kernel_imports
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
- local CLI/TUI/Web schema-v3 enablement uses an immutable User-scoped plan and
  the same Use-owned state/Grant saga, never launches a child mutation, renders
  the complete TUI review identity, and preserves exact Skill/UI/Flow replay
  across Web daemon restart;
- exact Flow compiler preflight and persisted generation binding;
- signed OKF install, restart recovery, exact upgrade cutover, stale-generation
  withdrawal, receipt-owned uninstall, cited retrieval, scope-isolated usage
  accounting, quota release, tombstone retention, and page reclamation;
- identical textual scope IDs remain isolated by User/Workspace kind, and
  ambiguous multi-scope or multi-generation projection fails closed;
- TUI, replacement, and Web sessions hot-plug and withdraw the same managed
  Knowledge tool, while the Code Web catalog/search routes return the exact
  Registry and lifecycle generations;
- accepted Knowledge queries hold exact published package-generation leases
  through backend search and revision verification; missing leases prevent
  backend invocation, repeated surfaces deduplicate, and conflicting package
  evidence fails closed;
- exact `flow.json` resolution before OS mutation, path-free binding evidence,
  and stale-generation rejection;
- source drift and symlink substitution fail before compiler/event mutation;
- CLI/TUI/Web share local Run/Status/Logs behavior without OS authority;
- idempotent runs and event history survive runtime recreation, package
  upgrade, and uninstall without leaking managed paths;
- TUI `/use` readiness changes after watcher updates;
- a detached Web process observes install, version replacement, and uninstall
  without restart; and
- exact Activity document URLs survive restart, reject prior generations with
  `410 Gone`, carry the sandbox/security header set, and expose no managed or
  Workspace paths;
- the production Web build adopts the exact Activity URL, reaches ready only
  through a transferred v3 `MessagePort`, reviews a port-delivered bounded
  proposal, serializes host-owned state operations, preserves the opaque
  network/storage sandbox, and records empty console and page-error evidence;
  and
- the Flow catalog fails closed when Use is unavailable.

The monorepo `just use-hotplug-e2e` gate crosses an independently built real
`a3s-use` binary boundary on Linux, macOS, and Windows for Code watcher
convergence, platform-native TUI first-use installation, generic signed Web
install/restart/upgrade/uninstall without retained paths or package residue,
and host-injected managed Runtime/Grant planning, drift rejection, cutover,
durable Grant observation, enablement recovery, and terminal replay.
The local managed Knowledge path is covered in-process with real signed package
and SQLite evidence, including exact query leases, scope quota, and
tombstone/physical GC. Complete real-process cross-platform graph E2E, managed
rollback, coordinated restore, production Runtime/Gateway injection, the
real-provider retained-generation upgrade/uninstall matrix, and complete
Runtime Service/HTTP MCP coverage remain release gates.

## 12. Platform scope

| Platform | Status |
| --- | --- |
| macOS arm64 / x86_64 | Supported release and managed-component target. |
| Linux arm64 / x86_64 | Supported release and managed-component target. |
| WSL | Supported through the Linux contract. |
| Windows x86_64 | Preview: native Code/WebView, verified Use ZIP first-use, generic signed Web package lifecycle, Edge core Browser profile, Office MCP operations, and local OCR E2E; complete six-surface and failure-injection parity remains gated. |

## 13. Release gates

The cognitive package line is not complete until all of the following pass:

- managed Knowledge rollback, coordinated restore/authority recovery, backup
  rotation, and distributed placement where required; exact query leases,
  scope quota, bounded retention, tombstone GC, integrity audit, verified
  backup, and derived-index repair are implemented;
- production Runtime assignments, readiness, and Gateway injection for OCI
  Tasks, Runtime Services, and HTTP MCP. The injected OCI Task path covers
  install plus offline restart-safe disable/re-enable and drift recovery; exact
  receipt-owned retirement is implemented. Real-provider retained-generation
  upgrade/uninstall and crash recovery remain gates;
- distributed Flow worker placement, automatic scheduling/resumption of waits
  and retries, and production retention/GC for resolved installed identities;
- a polished user-facing Web Flow run/status/logs/history interface over the
  completed API endpoints;
- reviewed Activity backend bindings and generation-aware readiness/sandbox
  composition in CLI, TUI, and native hosts; Code Web's failed-N+1 candidate
  proof and rollback are implemented;
- prior-generation drain, retirement, rollback, and garbage collection;
- real signed reviewed enablement and dependency-graph
  install/upgrade/uninstall across Code TUI and Web on supported platforms;
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
