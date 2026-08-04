# A3S Use and Component Platform

Status: Active implementation, pre-1.0

Updated: 2026-08-04

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
| Host Plugin Manager | One manager serves CLI, Web, and read-only management MCP planning. It binds actor, scope, policy, confirmation, durable intent, lifecycle cutover, and replay. |
| Code runtime composition | Executable Tool Tasks, stdio MCP, Skill, UI, and real `a3s-flow` Native TypeScript preflight are composed. |
| Code Flow catalog | Available through the exact-generation Use watcher and `GET /api/v1/plugins/flows`. |
| Code OKF composition | Fail closed until a production A3S Knowledge lifecycle adapter is injected. |
| Code Tool Service / HTTP MCP | Fail closed until production Runtime Service and Gateway readiness adapters are injected. |
| Hot-plug integration | TUI and detached Web process tests cover generation changes. Web covers install, upgrade, and uninstall replacement without daemon restart. |
| Remaining release gates | Prior-generation retirement/GC, production Knowledge projection, service/HTTP adapters, `flow.json` installed-identity mapping, durable Flow execution routing, and complete real-process cross-platform E2E. |

## 4. System architecture

```text
local package · release bundle · named TUF Registries
                         │
                         ▼
                 Host Plugin Manager
       source set · ACL policy · review · confirmation
                         │
                         ▼
              A3S Use package graph manager
       resolve · lock · stage · journal · reconcile
                         │
          ┌──────────────┼──────────────┐
          ▼              ▼              ▼
       Runtime        A3S Flow       Static host
      Tool · MCP      native-ts      Skill · UI
          │              │              │
          └──────────────┼──────────────┘
                         ▼
         exact-generation capability snapshot/watch
                         │
              ┌──────────┴──────────┐
              ▼                     ▼
        A3S Code TUI            A3S Code Web
        /use + worker       Marketplace + Flow API
```

OKF, long-lived Tool Services, and HTTP MCP join the same lifecycle only when
their Knowledge, Runtime Service, and Gateway hosts return typed readiness
evidence. Their absence is an admission failure, not a fallback route.

### 4.1 Ownership

| Owner | Owns | Does not own |
| --- | --- | --- |
| `a3s` umbrella CLI | Public command grammar, component catalog, named Registries, trust roots, first-use policy, review/apply UX, and host provider composition | Package archive internals, Browser actions, Knowledge indexing, or workflow execution |
| Shared Plugin Manager | Marketplace projection, immutable plans, policy evaluation, actor/scope binding, confirmation, parent lifecycle intent/cutover, and replay | Package validation or child provider execution |
| `a3s-updater` | Release resolution, download, verification, staging, receipts, atomic activation primitives, and owned-file removal | Product catalog or cognitive surface semantics |
| A3S Use | Package validation, dependency lock, immutable generations, package/child journals, receipts, grants, Runtime/Flow/Knowledge bindings, and capability reconciliation | Host policy, user confirmation, provider selection, or product UI |
| A3S Runtime / Gateway | Task/Service execution and stdio/HTTP MCP serving/readiness | Package dependency resolution or capability publication |
| A3S Flow | Native TypeScript preflight, compiled artifacts, durable execution, history, replay, scheduling, and observation | A second package lifecycle or visual asset ownership |
| A3S Knowledge | OKF staging, promotion, scoped projection, cited retrieval, and observation | Package installation or host policy |
| A3S Code | Agent sessions, TUI/Web presentation, dedicated Use worker, and live snapshot consumption | A second Registry, resolver, or package journal |

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
| Tool Task | Selected Runtime provider plus exact executable/package evidence and terminal observation. |
| Tool Service | Runtime Service endpoint and health for the exact generation. |
| stdio MCP | Runtime binding and standard MCP initialization/tool-list readiness. |
| HTTP MCP | Gateway/Runtime endpoint, route ownership, and serving readiness. |
| OKF | Knowledge stage, promotion, observation, and scoped projection evidence. |
| A3S Flow | Valid `a3s-flow`/`native-ts` source, compiler preflight, compiled artifact, dependency edges, and exact-generation binding. |
| Skill | Content-bound `SKILL.md`; every declared Tool/MCP/OKF/Flow dependency must already be ready. |
| UI | Integrity-bound entry/assets, sandbox compatibility, and every bound Skill/MCP/Flow dependency ready. |

## 6. Replaceable Registry model

Registry endpoints and trust roots are host input. They are never supplied by a
package dependency declaration or compiled into the resolver.

```bash
a3s registry add https://packages.example.org/a3s/ \
  --trust-root ./root.json \
  --yes
a3s registry refresh packages
```

The package engine accepts a root Registry plus a bounded set of dependency
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

An installed receipt remains bound to the selected Registry identity. Removing
or changing that source blocks upgrade. The operator must restore the source or
perform an explicit migration/reinstall; Use never searches another Registry
implicitly.

The umbrella CLI currently supports named add, remove, list, and refresh.
Stable-name in-place replace and enable/disable require a separate command and
are not described as shipped behavior.

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
`runtime = "native-ts"`. Code selects the compiler through
`A3S_FLOW_NATIVE_TS_COMPILER` or `a3s-flow-native-compiler` and persists cache
under the Use state root.

The binding records scope, package, Flow surface, lifecycle generation,
manifest/package/source digests, export, entrypoint, and compiled artifact
digest. Projection rechecks the source and artifact for the exact installed
generation. Source presence alone never marks the Flow ready.

The live Web catalog is:

```http
GET /api/v1/plugins/flows
```

It returns the Use snapshot generation/revision and typed, content-bound Flow
items. Visual `flow.json` import/deployment mapping and durable run routing are
remaining product integration work; they must reuse the same installed Flow
identity, not add a second engine.

## 8. A3S Code integration

### 8.1 Lifecycle factory

`CodeCognitivePackageLifecycleFactory` composes one
`PluginLifecycleCoordinator` for an admitted package generation:

- `RuntimePluginSurfaceLifecycleHost` for executable Tool Tasks and stdio MCP;
- `A3sFlowLifecycleHost` for packaged A3S Flow;
- `StaticPluginSurfaceLifecycleHost` for Skill and UI; and
- explicit unavailable hosts for OKF, Tool Service, and HTTP MCP until their
  production adapters are injected.

Host validation rejects a package before publication when a required surface
has no provider. The same factory is used for install, recovery, upgrade, and
uninstall coordinators.

### 8.2 Live projection

Every TUI/Web process keeps one Use snapshot watcher. It validates generation,
revision, package root, lifecycle generation, surface paths, SHA-256, media
types, dependency IDs, and source bytes before constructing projections.

The watcher currently exposes:

- standard MCP routes;
- verified package Skills;
- sandboxed Web Activity entries/assets; and
- a typed exact-generation A3S Flow catalog.

Generation replacement withdraws old callable surfaces before draining their
connections. A running call settles under the boundary it was admitted with;
new calls see only the new generation.

### 8.3 Dedicated Use worker

The stable worker identity is `use`:

- only `mcp__use_*` tools are visible to it;
- workspace, shell, unrelated MCP, and recursive task tools are denied;
- verified package Skills provide guidance but cannot expand permission;
- missing annotations, mutations, destructive operations, and open-world
  access return to the parent confirmation stream; and
- application failures do not fall back to another provider.

The parent model sees the worker and its current capability IDs through live
`task`/`parallel_task` definitions but does not receive the raw Use tool set.

### 8.4 Web adapter

Code Web reads the same Plugin Manager and Use watcher. Relevant routes include:

```text
GET  /api/v1/plugins/marketplace
GET  /api/v1/plugins/activities
GET  /api/v1/plugins/flows
POST /api/v1/plugins/operations/plan
POST /api/v1/plugins/operations/apply
```

Activity HTML is non-callable and runs in an opaque-origin iframe under a
restrictive CSP. Package context can be added only through a verified
same-package Skill after explicit host review.

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
a3s upgrade use/a3s/science
a3s uninstall use/a3s/science
a3s doctor use

a3s use browser render https://example.com
a3s use office validate report.docx
a3s use ocr extract scan.png --json

a3s plugin search research
a3s plugin inspect a3s/science
a3s plugin install a3s/science
a3s plugin upgrade a3s/science
a3s plugin uninstall a3s/science
```

`a3s code` may visibly install the verified Use product before terminal takeover
when network and first-use policy permit. `--offline` and
`A3S_NO_AUTO_INSTALL=1` prevent that mutation. Code Web consumes an
already-ready Use product and does not install it.

## 11. Verification evidence

Focused host gates:

```bash
cargo test --lib use_registry::tests:: --no-fail-fast
cargo test --test web_cli \
  plugin_api_exposes_catalog_and_fails_closed_without_trust_roots
cargo test --test web_plugin_marketplace \
  marketplace_install_upgrade_uninstall_hot_plugs_verified_activity_skill_and_flow_catalog
cargo test --lib \
  code_host_preflights_flow_and_persists_exact_generation_binding
```

These prove:

- strict source/path/digest/dependency validation;
- exact Flow compiler preflight and persisted generation binding;
- TUI `/use` readiness changes after watcher updates;
- a detached Web process observes install, version replacement, and uninstall
  without restart; and
- the Flow catalog fails closed when Use is unavailable.

The monorepo `just use-hotplug-e2e` gate crosses the independently released
real `a3s-use` binary boundary for the existing native/MCP/Skill lifecycle.
Complete schema-v3 graph E2E with production Knowledge, Gateway, and Runtime
Service providers remains a release gate.

## 12. Platform scope

| Platform | Status |
| --- | --- |
| macOS arm64 / x86_64 | Supported release and managed-component target. |
| Linux arm64 / x86_64 | Supported release and managed-component target. |
| WSL | Supported through the Linux contract. |
| Windows x86_64 | Preview: native Code/WebView, verified Use ZIP, Edge core Browser profile, Office MCP operations, and local OCR E2E; complete plugin lifecycle parity remains gated. |

## 13. Release gates

The cognitive package line is not complete until all of the following pass:

- production A3S Knowledge composition for OKF stage, promotion, observation,
  cited retrieval, and scoped session projection;
- Runtime Service and HTTP MCP/Gateway provider selection and readiness;
- installed Flow identity mapping for `flow.json` and durable execution;
- prior-generation drain, retirement, rollback, and garbage collection;
- real signed dependency-graph install/upgrade/uninstall across Code TUI and
  Web on supported platforms;
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
