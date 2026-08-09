# A3S Cognitive Package Integration

Status: development preview

Last updated: 2026-08-09

Parent: [A3S Use and Component Platform](a3s-use-component-platform.md)

Canonical package contracts live in the
[A3S Use repository](https://github.com/A3S-Lab/Use/tree/main/docs). This
document defines how the umbrella CLI and A3S Code consume those contracts.

## 1. Decision

A3S Use owns the only cognitive-package format and package-graph lifecycle.
The umbrella Plugin Manager supplies trusted Registry configuration, policy,
review, and confirmation. Code TUI and Web consume the same immutable
capability snapshots; they do not implement another package manager.

Only the current contracts are accepted:

- ACL manifest schema version 3;
- complete catalog-v3 Registry records;
- exact SemVer dependency locks;
- signed package targets and executable planning targets; and
- reviewed plan/apply with an exact operation ID and canonical digest.

There is no production path for an unsigned archive, local package source,
older manifest schema, partial catalog record, or direct surface mutation.

## 2. Package Aggregate

One cognitive package is an npm-like immutable distribution unit:

```text
acme-research/
├── a3s-use-extension.acl   identity · version · dependencies · surfaces
├── README.md               required package documentation
├── tools/                  native Tasks or provider-backed Services
├── releases/               content-bound Tool and MCP descriptors
├── flows/                  A3S Flow Native TypeScript sources
├── skills/                 SKILL.md files and supporting content
├── ui/                     integrity-bound static Activities
└── okf/                    Open Knowledge Format bundles
```

Only the manifest and `README.md` names are fixed. Every contribution path is
package-relative, manifest-owned, and digest-bound. ACL means A3S Agent
Configuration Language and is parsed with `a3s-acl`; it is not HCL.

The package ID is `<publisher>/<name>`. The package version is canonical
SemVer. Dependencies name package IDs and SemVer requirements only; they never
carry source URLs or trust roots.

The complete package generation is the lifecycle unit. Individual Tool, MCP,
OKF, Flow, Skill, and UI surfaces may be selected for projection, but they
cannot be independently installed, upgraded, enabled, disabled, or
uninstalled.

## 3. Six Native Surfaces

| Surface | Native contract | Code host readiness |
| --- | --- | --- |
| Tool Task | Package-bound non-interactive executable and argv contract | Package-local native Tasks use a signed planning target and deterministic native provider evidence. |
| Tool Service | Long-lived native or OCI workload | The shared typed endpoint and retirement lifecycle is composed; an exact production Runtime selection and Gateway binding are still required. |
| stdio MCP | Standard MCP process lifecycle | Package-local stdio servers use a signed planning target and deterministic native provider evidence. |
| HTTP MCP | Standard MCP over an HTTP endpoint | The shared Runtime/Gateway bind, drain, and removal contract is composed; production providers are still required. |
| OKF | Open Knowledge Format 0.2 bundle | Scope-aware SQLite/FTS5 stage, promotion, durable exact-generation binding, watched TUI/Web projection, and cited search are implemented. |
| A3S Flow | `a3s-flow` Native TypeScript source and export | Local preflight, exact-generation binding, execution, observation, and durable history are implemented. |
| Skill | Content-bound `SKILL.md` | Projected after all declared dependencies are ready. |
| UI | Integrity-bound static Activity | Code Web publishes and adopts an exact generation/revision-bound sandbox document after declared package dependencies are ready. The browser uses a dedicated v3 `MessagePort`, terminates self-navigation, binds context review and bounded durable state to the document identity, and drains/replaces old frames on Registry changes. State uses exact published-generation leases and scope/package/surface isolation, survives retained-surface transitions, and clears on true removal. Backend bindings, failed-N+1 readiness/cutover/rollback, and native hosting remain open. |

Tool and MCP retain their native protocols. A Tool is not an MCP
`tools/list` item, a Skill is not executable code, an OKF bundle is not a
workflow, and `flow.json` is not another workflow engine. A3S Use coordinates
their package lifecycle without wrapping them in a universal extension RPC.

Required surfaces fail closed when their exact provider or evidence is
unavailable. They never downgrade to a different execution mechanism.

## 4. Replaceable Registry Sources

Registry name, URL, trust root, enabled state, and cache location are host
configuration. Package metadata cannot add, replace, or prioritize a source.

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

The canonical `state/use/registries.acl` document is shared by Marketplace,
planning, and apply, and every mutation is revision-bound. Install chooses the
default or an explicit `registryName`; upgrade cannot switch away from the
installed Registry provenance. Resolution queries enabled dependency
Registries in stable name order. No match is an
error. The same package resolving from more than one trusted Registry is
ambiguous and fails closed.

An installed receipt retains its exact Registry identity and TUF provenance.
Replacing configuration does not rewrite the receipt. Upgrade fails until the
recorded provenance is restored or a separately reviewed migration exists.

## 5. Public Management Experience

Metadata search and inspection do not download package archives:

```bash
a3s plugin search research
a3s plugin search research --surface flow
a3s plugin search research --surface okf
a3s plugin inspect acme/research --channel stable
a3s plugin list
```

Mutations use one reviewed lifecycle:

```bash
a3s plugin install acme/research --channel stable
a3s plugin upgrade acme/research
a3s plugin disable acme/research
a3s plugin enable acme/research
a3s plugin uninstall acme/research
```

Automation first persists a read-only plan, then applies that exact identity:

```bash
a3s --output json plugin install acme/research --dry-run
a3s --output json plugin apply <operationId> \
  --plan-digest <canonicalPlanDigest> \
  --yes
```

`a3s plugin enable` and `a3s plugin disable` are reviewed package-state
operations. They replace the former direct A3S Use surface mutations. The
`a3s use` namespace remains the proxy for built-in Browser, Office, OCR, and
read-only Use diagnostics.

## 6. Resolution and Review Binding

For install and upgrade, Registry resolution produces:

- the selected complete catalog-v3 record;
- exact TUF root, timestamp, snapshot, and targets provenance;
- the package target path, length, and SHA-256;
- one exact dependency lock for the complete graph; and
- a separately signed planning target for every executable package that needs
  native Tool or stdio MCP evidence.

The Plugin Manager verifies every planning target before payload download and
binds it to the matching locked catalog record. A3S Use rebinds the downloaded
planning descriptor to the digest-bound schema-v3 manifest and derives
deterministic native provider evidence. Missing, extra, duplicated, stale, or
mismatched planning bundles fail before mutation.

The reviewed plan also binds actor, User or Workspace scope, policy digest,
current state revision, prior and candidate locks, package transitions,
selected surfaces, Grant impact, expiry, confirmation requirement, operation
ID, and canonical digest.

Apply accepts only the stored envelope and exact confirmation. It revalidates
Registry identity, policy, lock, package bytes, planning evidence, Grant
revision, and current package generation before durable intent. There is no
child `a3s` mutation or subprocess fallback.

## 7. Package Lifecycle and Hot Plug

Dependencies install and prepare before dependents. Disable and uninstall hide
capabilities first, drain consumers, retire providers in reverse dependency
order, and retain a dependency still owned by another installed root graph.
One successful cutover publishes one capability generation.

```text
verified install   -> generation N+1 -> ready surfaces appear
reviewed disable   -> generation N+2 -> callable surfaces withdraw and drain
reviewed enable    -> generation N+3 -> exact installed surfaces return
verified upgrade   -> generation N+4 -> old evidence is replaced atomically
verified uninstall -> generation N+5 -> package surfaces withdraw and drain
```

Code keeps one watcher per process. Existing and future TUI/Web sessions read
the same exact-generation snapshot. Restart reconstructs the signed installed
catalog, desired state, lifecycle bindings, Grants, Flow bindings, and durable
Flow history before capabilities are republished.

Managed OKF queries also join this drain boundary. Code derives one lifecycle
identity per selected package generation, including its exact package and
manifest digests, acquires every currently published Registry lease before
Knowledge access, and holds the leases through backend search and final watcher
revision verification. Multiple OKF surfaces from the same package generation
share one lease. Missing publication or conflicting digest evidence fails
closed before the backend is invoked; a query accepted before cutover keeps the
prior generation alive until the query finishes.

## 8. Ownership Boundaries

| Owner | Responsibility |
| --- | --- |
| Umbrella CLI | Registry configuration, trusted roots, command UX, policy, review, and confirmation. |
| Plugin Manager | Catalog query, immutable plans, Grant binding, durable apply intent, exact replay, and managed Workspace fencing. |
| A3S Use | Schema validation, dependency resolution, package storage, receipts, journals, immutable generations, and capability reconciliation. |
| Native launcher | Package-bound native Tool Task and stdio MCP process lifecycle. |
| Runtime / Gateway | OCI and long-lived Service execution plus HTTP MCP exposure. |
| A3S Flow | Workflow preflight, execution, events, observation, and replay. |
| Knowledge host | OKF staging, promotion, indexing, retrieval, and retirement. |
| Code TUI / Web | Presentation and consumption of one live capability snapshot. |

Browser, Office, and OCR remain typed built-in Use domains with their own
provider lifecycles. Their delegated component process contract does not grant
authority to mutate an external cognitive package.

## 9. Security Invariants

- Registry URLs use HTTPS except loopback HTTP in hermetic tests.
- TUF expiry, rollback, root rotation, target length, and target digest are
  verified before archive download.
- Archives reject traversal, links, duplicate normalized paths, special files,
  and bounded-size violations.
- Package paths remain relative and contained after extraction.
- Package metadata cannot grant filesystem, network, process, secret, UI HTTP,
  or resource authority.
- Tool and MCP planning descriptors are signed separately and rebound to the
  exact downloaded manifest.
- Workspace configuration cannot pre-authorize its own installation.
- Cancellation or process failure resumes only from durable, idempotent saga
  evidence.

## 10. Current Readiness

The source implementation and hermetic tests prove the current schema-v3
Registry path, exact dependency locking, native Tool Task and stdio MCP
planning, reviewed enablement, three-platform Code watcher convergence,
platform-native TUI first-use, generic signed Web Marketplace lifecycle, local
Flow persistence, and managed OKF install/restart/upgrade/
uninstall with scope-bound cited retrieval, receipt-accounted scope quota,
bounded tombstones, physical SQLite cleanup, scope-local integrity audit,
derived-index repair, verifiable backup, and exact published-generation query
leases through Code TUI/Web. This is still a development preview.

Code Web now provides a composed Activity boundary: enabled catalog entries
carry an exact generation/revision URL, verified HTML/CSS/JS is inlined under
an opaque-origin CSP and restrictive security headers, and stale URLs return
`410 Gone`. The browser adopts only that URL, transfers a dedicated
`a3s.activity.v3` `MessagePort`, ignores ambient messages, terminates a frame
that loads twice, replaces and drains the old port on Registry changes, and
binds reviewed context and serialized state requests to the exact document
identity. The JSON content endpoint remains non-executable management data.
Code owns bounded durable state per scope/lifecycle package/surface; exact
published-generation leases prevent stale iframes from writing, and lifecycle
intent retains or clears each surface explicitly. Browser evidence covers the
real production Web build; reviewed backend bindings, failed-N+1
readiness/cutover/rollback, and native UI hosting remain incomplete.

Production promotion additionally requires:

- an operational public Registry trust root and release process;
- published compatible Use and host dependencies instead of Git revisions;
- complete real-process Linux, macOS, and Windows lifecycle coverage;
- crash injection at every package/Grant/provider/capability saga boundary;
- managed Knowledge rollback, coordinated restore and authority recovery,
  backup rotation, and distributed placement; exact published-generation query
  leases, scope quota, bounded retention, tombstone GC, integrity audit,
  derived-index repair, and scope-local backup verification are implemented;
- production Runtime Service selections and HTTP MCP/Gateway adapter
  injection; the shared endpoint and retirement contract is implemented;
- reviewed Activity backend bindings, failed-N+1 readiness/cutover/rollback,
  and equivalent generation-aware composition in native hosts;
- remaining provider-specific prior-generation drain, retirement, rollback,
  and garbage collection; and
- production Flow scheduling, resumption, retention, and garbage collection.

## 11. Related Documents

- [A3S Use Component Platform](a3s-use-component-platform.md)
- [Plugin Authorization Policy](plugin-authorization-policy.md)
- [A3S Use package architecture](https://github.com/A3S-Lab/Use/blob/main/docs/plugin-platform-architecture.md)
- [A3S Use package contracts](https://github.com/A3S-Lab/Use/blob/main/docs/plugin-contracts.md)
- [Cognitive package lifecycle saga](https://github.com/A3S-Lab/Use/blob/main/docs/adr-002-cognitive-package-lifecycle-saga.md)
- [Runtime broker boundary](https://github.com/A3S-Lab/Use/blob/main/docs/adr-001-plugin-runtime-broker-boundary.md)
