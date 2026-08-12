<p align="center">
  <img
    src="assets/readme/hero.svg"
    width="100%"
    alt="A3S CLI runs one coding workspace in the terminal and browser, with a reviewed cognitive-package path on main"
  />
</p>

<p align="center">
  <strong>Build with agents in the terminal or browser. Extend the same host through reviewed, versioned packages.</strong>
</p>

<p align="center">
  <a href="https://github.com/A3S-Lab/CLI/actions/workflows/ci.yml"><img src="https://github.com/A3S-Lab/CLI/actions/workflows/ci.yml/badge.svg" alt="CI status" /></a>
  <a href="https://crates.io/crates/a3s"><img src="https://img.shields.io/crates/v/a3s.svg" alt="Crates.io version" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-f0b65b.svg" alt="MIT license" /></a>
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#cognitive-packages-main-branch-preview">Cognitive packages</a> ·
  <a href="#a3s-code">A3S Code</a> ·
  <a href="#component-lifecycle">Components</a> ·
  <a href="#release-readiness">Readiness</a> ·
  <a href="#development">Development</a>
</p>

> [!IMPORTANT]
> **Release transition — August 5, 2026.** Public `0.11.1` installs are
> available from [crates.io](https://crates.io/crates/a3s), Homebrew, and the
> [A3S monorepo release](https://github.com/A3S-Lab/a3s/releases/tag/v0.11.1).
> This repository is now the canonical source for new CLI work, but its
> repository-owned `v0.11.1` release is still a draft. The cognitive-package
> integration described below is newer than the public `0.11.1` build and is a
> **main-branch preview**, not a production package release.

## One CLI, two Code surfaces

`a3s` is the umbrella command for the A3S developer platform. A base install
contains A3S Code. Other native products and A3S Use capabilities keep their
own release and lifecycle boundaries.

```text
a3s
├── code        interactive or non-interactive coding agent
├── web         the same Code host through a local Web application
├── plugin      reviewed cognitive-package lifecycle (main preview)
├── use         Browser, Office, OCR, and installed Use capabilities
├── compose     multi-service applications delegated to A3S Box
└── components  install · upgrade · inspect · repair · uninstall
```

| Entry point | Current role |
| --- | --- |
| `a3s code` | TUI, governed tools, durable sessions, memory, research, asset authoring, and local Flow execution. |
| `a3s web -d` | Loopback Web workspace and API using the same configuration, sessions, models, and package watcher. |
| `a3s plugin …` | Search, review, install, upgrade, enable, disable, and uninstall cognitive packages on `main`. |
| `a3s use …` | Delegate Browser, Office, OCR, Box, and extension capabilities to A3S Use. |
| `a3s install …` | Manage registered A3S products and delegated Use packages; it is not a universal OS package manager. |

### What is proven on `main`

The current source has regression coverage for the boundaries that matter to
the package host:

| Evidence | What it exercises |
| --- | --- |
| Linux, macOS, and Windows CI | Linux runs the full test, lint, installer, and release-build gate; macOS runs the native installer/TUI regression and release build; Windows runs its installer matrix and release build. |
| Reviewed Use authorization bridge | The delegated planner emits a provider-neutral, unbound draft. The host then binds the exact Grant and provider evidence from signed planning bundles, explicit Runtime assignments, and current provider capabilities before policy review, repeats that binding with the final authority, and rejects any provider, build, capability, semantic, enforcement, or authority drift. Real signed schema-v3 packages keep the umbrella operation ID, canonical plan, dependency locks, Grant snapshot, planning bundles, reviewed provider evidence, and confirmation inside the in-process Use graph; apply never launches a child `a3s` mutation. |
| Shared Code host policy | TUI and each Web host create one `PluginManager`; every Web plugin route clones the startup `Arc`, and all processes retain the same durable file-lock boundary. Detached Web and the read-only management MCP reparse only the operator-selected ACL source under a normalized digest lock; an automatically discovered workspace ACL never becomes plugin authorization. Web reuse also requires the exact policy digest and offline mode, otherwise `--replace` is required. |
| Fenced managed Workspace host | Protocol v4 explicitly plans a signed package's enable/disable transition as plan-v4, binds user confirmation to its operation ID and digest, and applies through the existing host apply request. Planning evidence, apply intent, capability cutover, and result survive host recreation; stale generations, request or digest substitution, package-byte changes, and dependency-graph changes fail closed. A permission-bearing Tool regression proves that missing confirmation creates no apply intent or lifecycle mutation. |
| TUI first-use integration | Linux, macOS, and Windows package an independently built A3S Use release as the platform-native archive, install it while Code remains responsive, tolerate bounded one-time executable scanning, and prove the attached registry revision is visible before the first model turn. |
| Web Marketplace lifecycle | On Linux, macOS, and Windows, a generic signed package crosses an independently built A3S Use process through the public Web plan/apply API, then proves install, Activity/Skill hot-plug, canonical `user/current` lifecycle diagnostics, process restart, exact upgrade, path-free content, and residue-free uninstall. Activity catalogs publish only enabled generation/revision-bound document URLs; the server returns the same sandboxed document after restart and `410 Gone` after upgrade, disable, or uninstall. Separate reviewed-enablement coverage exercises disable/re-enable and Flow generations. |
| Web Activity composition | The production A3S Web build adopts only the exact catalog `documentUrl` in an opaque-origin iframe, transfers a dedicated `a3s.activity.v3` `MessagePort`, ignores ambient messages, terminates self-navigation, replaces and drains the old port on Registry changes, and binds context review and bounded state operations to the exact document identity. Code-owned state is isolated by scope/lifecycle package/surface, survives restart and retained-surface transitions, and is cleared on true removal. A deterministic `a3s-test` suite proves ready-state handoff, a port-delivered proposal, blocked network/storage, empty console/page errors, accessibility output, and screenshot evidence. |
| Registry-backed restart recovery | A detached Web host reconstructs the exact signed Registry package generation and durable Flow history after restart. |
| Managed OKF Knowledge | A real signed package test covers install, durable SQLite/FTS5 projection, process restart, exact-generation upgrade, stale-generation withdrawal, cited search, uninstall, whole-scope usage accounting, quota release, tombstones, and physical page reclamation. Scope-local tests also cover integrity audit, non-overwriting backup, offline verification, and confirmed FTS repair. The watched Registry then hot-plugs the same read-only search tool into TUI and Web sessions; each accepted query holds exact package-generation Registry leases through backend search and revision verification, and Code Web exposes the exact catalog and scope-bound results. |
| Host-bound Runtime lifecycle | A real signed OCI Tool Task regression proves that a missing host assignment fails before archive download, an injected provider is selected only by the host, and build drift fails before install mutation. The Linux/macOS/Windows monorepo gate supplies the independently built, exact-revision `a3s-use` executable to that trusted host test, so plan and post-cutover capability evidence cross the real process boundary while Runtime and Grant authority stay injected in Plugin Manager. Schema-v4 Task bindings retain an argument-free reviewed Runtime template and exact provider/Grant evidence. The shared Manager dispatcher reconnects that provider after restart, derives only per-call identity and bounded argv, rejects hidden generations, and holds the Registry lease through capture and cleanup. Capability snapshot v2 now projects an exact Task as a conservative `use_tool_*` tool into both TUI and Web sessions only when the named reviewed provider exists; provider absence produces a warning and no tool. Upgrade and disable withdraw the old dynamic tool before replacement. Code still injects no production Runtime/Gateway provider by default. |
| Shared managed execution boundary | Code delegates package-host composition to the shared A3S Use managed factory. Runtime Services must publish an exact typed loopback endpoint; retirement is Gateway drain, Runtime stop, Gateway route removal, then Runtime removal, with the generation receipt retained until completion. The protocol is composed, but production Runtime assignments, readiness, and Gateway adapters are not. |

These tests support the preview claim. They do not replace the release gates in
[Release readiness](#release-readiness).

## Quick start

Install the newest public stable release with one command:

```bash
# macOS or glibc Linux (x86_64 / arm64)
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/A3S-Lab/CLI/main/install.sh \
  | A3S_MODIFY_PATH=1 sh
```

```powershell
# Windows x64 — PowerShell 5.1 or newer
$env:A3S_MODIFY_PATH = '1'
irm https://raw.githubusercontent.com/A3S-Lab/CLI/main/install.ps1 | iex
```

The installers compare the two official release repositories during the
current migration, select the newer stable SemVer, verify the GitHub-published
SHA-256, reject unsafe archive members, validate `a3s --version`, and activate
the binary, Web assets, and optional WebView companion as one recoverable
operation. They never use `sudo` or UAC. Omit `A3S_MODIFY_PATH=1` to leave
shell profiles and the user PATH unchanged.

Package-manager installation remains available:

```bash
# macOS or Linux with Homebrew
brew install A3S-Lab/tap/a3s

# Any platform supported by the Rust toolchain
cargo install a3s --locked
```

Start in the terminal or browser:

```bash
a3s code
a3s web -d
```

The first `a3s code` launch creates `~/.a3s/config.acl` when no configuration
exists. Inspect the selected ACL configuration with:

```bash
a3s config path
a3s config show
a3s config validate
```

To evaluate the newer cognitive-package integration, build the canonical
source instead of assuming it is present in public `0.11.1` binaries:

```bash
git clone https://github.com/A3S-Lab/CLI.git
cd CLI
cargo install --path . --locked
```

Prepare and inspect A3S Use explicitly when first-use installation is not
appropriate:

```bash
a3s install use --source release
a3s use doctor --json
a3s use capabilities --json
```

`--offline` and `A3S_NO_AUTO_INSTALL=1` are strict no-download boundaries.

## Cognitive packages: main-branch preview

A cognitive package is an npm-like, SemVer distribution unit owned by A3S
Use. It has a stable `<publisher>/<name>` identity, an ACL manifest, a required
README, optional package dependencies, and any combination of six surface
contracts:

```text
acme-research/
├── a3s-use-extension.acl   identity · version · dependencies · surfaces
├── README.md               required package documentation
├── tools/                  executable Tasks or long-lived Services
├── releases/               content-bound Tool and MCP descriptors
├── flows/                  A3S Flow TypeScript workflow sources
├── skills/                 SKILL.md files and supporting content
├── ui/                     integrity-bound static assets
└── okf/                    Open Knowledge Format bundles
```

The complete package generation—not an individual file—is the install,
upgrade, enable, disable, and uninstall unit. Dependencies install before
dependents, unused dependencies uninstall in reverse order, and one successful
cutover publishes one new capability generation.

### Host readiness by surface

The package format accepts all six surfaces. The current Code host does not
pretend that every execution adapter is ready:

| Surface | Composed on `main` | Still gated |
| --- | --- | --- |
| **Skill** | Content verification and live session projection. | — |
| **UI** | Integrity-checked Activity HTML/CSS/JS plus exact generation/revision-bound Code Web document URLs. The document response enforces an opaque-origin script sandbox, no connection/frame/object/form/base authority, same-origin embedding/resource policy, disabled browser permissions, no referrer, no cache, and MIME sniffing denial. A3S Web adopts only the exact URL, transfers a dedicated v3 `MessagePort`, ignores ambient messages, terminates self-navigation, binds reviewed proposals to the current document, serializes bounded state requests, and drains/replaces the old frame on Registry generation changes. Host-owned state uses exact published-generation leases and durable scope/package/surface isolation; disable, rollback, and retained-surface upgrade preserve it, while true removal clears it. Before cutover, Code Web loads exact path-free N+1 candidate bytes in a hidden script-only sandbox, gives it only readiness-mode `host.init` over a dedicated port, and accepts only `activity.ready`. Load, navigation, protocol, or timeout failure keeps N selected and callable, rolls the Use operation back without receipt/generation residue, and makes the failed plan non-replayable; a fresh plan can retry the same lifecycle generation and cut over once. | Reviewed Tool/MCP/Flow backend bindings, equivalent browser readiness outside Code Web, and general-purpose native UI hosting. CLI and TUI remain static-integrity-only until they inject an equivalent renderer. |
| **MCP** | Verified native stdio MCP lifecycle plus the shared typed Runtime/Gateway contract. | HTTP MCP activation and retirement until production Runtime assignments/readiness and a Gateway adapter are injected and carried through every mutation path. |
| **Tool** | Verified non-interactive native Task lifecycle, exact-generation Runtime dispatch from durable schema-v4 receipts, and watched TUI/Web projection as conservative `use_tool_*` tools. Projection carries the exact package/manifest digests, lifecycle generation, scope, surface, and provider ID; invocation accepts only bounded argv and never consults current assignments. Missing providers fail closed without registering a tool, while upgrade and disable withdraw the old generation. | A default production OCI Runtime provider, production long-lived Services/Gateway readiness, and the real-provider cross-platform uninstall/upgrade recovery matrix. |
| **A3S Flow** | Native TypeScript preflight, exact-generation binding, and durable local/Web runs, status, and history. | Distributed placement, automatic resumption, and production retention. |
| **OKF** | Scope-aware SQLite/FTS5 staging and promotion, receipt-accounted scope quotas, bounded generations and tombstones, physical cleanup, durable bindings, restart recovery, integrity audit, derived-index repair, versioned backup/offline verification, watched TUI/Web projection, cited retrieval through `use_knowledge_search` and Code Web, and exact published-generation query leases that participate in lifecycle drain. | Coordinated restore, authority recovery, backup rotation, managed rollback semantics, distributed Knowledge placement, and the complete cross-platform release matrix. |

Required surfaces fail closed when their adapter or evidence is unavailable;
they never silently downgrade to a different provider.

### Reviewed lifecycle

With an explicitly trusted Registry configured, metadata can be searched
without downloading package archives. Mutations create an immutable plan
before they change the active generation:

```bash
a3s plugin search research
a3s plugin inspect acme/research

# Interactive review and apply
a3s plugin install acme/research --channel stable
a3s plugin upgrade acme/research
a3s plugin disable acme/research
a3s plugin enable acme/research
a3s plugin uninstall acme/research

# Non-interactive two-step apply
a3s --output json plugin disable acme/research --dry-run
a3s --output json plugin apply <operationId> \
  --plan-digest <canonicalPlanDigest> \
  --yes
```

Every lifecycle mutation requires the current catalog-v3 evidence and a
complete cognitive-package lock. Package planning returns an unbound draft:
package content cannot select a Runtime provider or prebind host authority.
Plugin Manager resolves signed planning bundles through its host-owned
`PluginRuntimeHost`, whose explicit surface assignments, Runtime client
registry, and readiness adapter are injected at process composition.

For a locked graph, A3S Use derives the candidate lifecycle generations and
Grant/provider evidence, the host evaluates policy over that complete plan,
and Use regenerates the evidence with the final authority. Planning rejects
provider identity/build, capability, workload semantics, enforcement,
authority, scope, or revision drift. The v3 operation record persists the
canonical Registry source revision, planning bundles, exact Grant snapshot,
and complete reviewed provider evidence alongside the plan and confirmation.

Reviewed enablement uses its own schema-v2 durable plan record. It stores the
installed signed planning bundle, exact Grant snapshot, and package-to-provider
generations—not process-local Runtime clients. Apply reconnects the configured
providers and must reproduce the reviewed evidence. Disable carries an empty
candidate selection and lets A3S Use retire from the exact binding receipt;
re-enable reconstructs activation from the retained bundle. This remains valid
after manager restart and does not refetch Registry targets.

At apply, Plugin Manager reconstructs only the Registry identities frozen in
the lock, re-derives the Grants, lifecycle generations, host assignments, and
Runtime selection from current evidence, and requires an exact match with the
reviewed provider record before it creates the lifecycle factory or downloads
the package archive. It then invokes A3S Use in-process with
`ReviewedCognitivePackageAuthorizationProvider`. Use must reproduce the same
operation ID, plan digest, package transitions, impact, state revision, lock,
Grants, and provider evidence before it may mutate. Missing evidence, an older
schema, or an unlocked plan is rejected during planning; apply has no
subprocess mutation fallback.

The Grant snapshot is bound to the plan's exact canonical `user/current` scope
and durable state revision. Scope, revision, prebound impact/provider evidence,
or final authority drift fails before apply. A signed OCI Tool Task regression
proves that missing assignment and changed provider build fail before archive
or lifecycle mutation, while restoring the reviewed build persists the exact
Grant receipt without launching a child mutation and replays idempotently.
Upgrade binds both the exact installed and candidate locks; upgrade and
uninstall retain dependencies still owned by another installed root graph.

Managed Workspace enable and disable now use an explicit two-step protocol.
The host persists `PluginHostEnablementPlanRequest` and its exact plan-v4 or
terminal `NoChange` result, then accepts only the existing digest-bound
`PluginHostApplyRequest`. A confirmed apply reconstructs
`ReviewedCognitivePackageAuthorizationProvider`, and A3S Use reproduces the
same plan before its enablement and Grant saga may mutate. Permission-bearing
packages therefore use the same prepare, cutover, drain, retirement, and crash
recovery path as their reviewed graph lifecycle. Package bytes and the
dependency graph do not change.

Local CLI, TUI, and Web schema-v3 enable/disable now use the same reviewed
two-step contract. Planning persists the complete User-scoped Use envelope and
returns either `planned` with an operation ID and canonical digest or terminal
`no-change` without synthetic mutation identity. Apply revalidates policy,
lifetime, digest, and exact confirmation before durable intent, then resumes or
replays only the recorded saga after intent. `a3s plugin apply` accepts these
enablement plans as well as install, upgrade, and uninstall plans. The former
direct `/packages/enabled` mutation route has been removed; Web exposes only
the reviewed enablement plan/apply endpoints.

In Code TUI, `/packages` is available only while the agent is idle. It lists the
authoritative installed-package snapshot and keeps desired enablement separate
from current callability. Enter creates a plan without mutating; the review
shows the complete operation ID, canonical digest, expected package generation,
and expiry before Enter/y can apply that exact identity. Esc/n cancels, an
identity-free `NoChange` refreshes without apply, and the panel remains locked
while a confirmed apply is in flight. `/plugin` remains a separate local
Claude/Codex Skill switch and does not manage A3S Use packages.

Code TUI and Web observe one capability watcher per process:

```text
verified install   → generation N+1 → ready surfaces appear
reviewed disable   → generation N+2 → callable surfaces withdraw and drain
reviewed enable    → generation N+3 → exact installed surfaces return
verified upgrade   → generation N+4 → old evidence is replaced atomically
verified uninstall → generation N+5 → package surfaces withdraw and drain
```

### Managed OKF Knowledge

An installed OKF surface is indexed as immutable, non-executable content; Code
does not paste the package into the system prompt. A promoted projection binds
the exact User or Workspace scope, package and surface identity, lifecycle
generation, package and bundle digests, projection receipt, and index digest.
Only one generation of a surface may be active in a scope.

When at least one projection is active, current and newly attached TUI/Web
sessions receive the read-only `use_knowledge_search` tool. Disable or
uninstall removes the tool when no managed Knowledge remains. A query snapshots
the live Registry generation, deduplicates its projections into exact package,
manifest, and lifecycle-generation identities, and acquires every corresponding
published Registry lease before SQLite access. Those leases remain held through
backend search and final Registry revision verification, so a query accepted
before cutover participates in lifecycle drain and blocks prior-generation
retirement until it finishes. Missing leases and conflicting projection
digests fail closed. A racing cutover is retried once against the replacement
revision; stale results are never returned as current context. Every hit carries
its exact concept path, source digest, package generation, projection receipt,
and index digest.

The composed adapter inherits the Use default storage policy for every complete
User or Workspace scope: 512 MiB of receipt-accounted expanded content, 256
retained projections, 32 generations per surface, and 256 removal tombstones.
Staging checks the whole scope atomically; receipt-owned removal frees quota,
prunes tombstones, vacuums SQLite, and truncates its WAL. Operators can inspect
non-secret allocation evidence through `a3s use knowledge usage --json`; an
exact Workspace query also requires `--scope-kind workspace --scope-id <id>`.

Use also exposes scope-bound operator commands through the same transparent
`a3s use` proxy:

```bash
a3s use knowledge audit --json
a3s use knowledge backup ./workspace.a3s-okf-backup \
  --scope-kind workspace --scope-id workspace/acme --json
a3s use knowledge verify-backup ./workspace.a3s-okf-backup \
  --scope-kind workspace --scope-id workspace/acme --json
a3s use knowledge repair-search-index --yes \
  --scope-kind workspace --scope-id workspace/acme --json
```

Audit verifies SQLite, foreign keys, receipts, accounting, scope identity, and
FTS integrity. Repair only rebuilds the derived FTS rows after authoritative
state passes validation. Backup is non-overwriting and its bounded manifest
and database digest can be verified offline, but it is neither a Registry
signature nor a whole-product recovery artifact. Coordinated restore is not
implemented; copying a snapshot into live state is unsupported.

Code Web exposes the same carrier:

```http
GET  /api/v1/knowledge/packages
POST /api/v1/knowledge/packages/search
```

`scopeKind` (`user` or `workspace`) and `scopeId` are optional only when one
scope is active; when multiple scopes are projected they must be supplied
together. These endpoints are separate from the personal `/kb` authoring
vault and do not create another package or Knowledge lifecycle.

### Replaceable Registry sources

Registry URL and trust identity belong to host configuration, never to an
untrusted package. A mirror or private source can be added, disabled, or
replaced without changing resolver code:

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

`state/use/registries.acl` is the only Registry source document used by CLI,
TUI, Web, Marketplace, plan, and apply. Every mutation uses revision CAS.
Install may select an enabled source with `--registry-name`; upgrade and
uninstall remain pinned to installed provenance. Installed receipts remain
pinned to the Registry identity that supplied them.
Replacing a source does not rewrite those receipts; an upgrade fails closed
until provenance is restored or explicitly migrated. The official production
Registry root is not yet public, so these commands currently require a source
the operator deliberately trusts.

## Architecture

<p align="center">
  <img
    src="assets/readme/cognitive-hotplug-architecture.svg"
    width="100%"
    alt="Trusted package sources pass through one Plugin Manager and A3S Use graph before Skills, Flow, OKF, and provider-qualified Runtime Tasks reach Code TUI and Web"
  />
</p>

| Owner | Responsibility |
| --- | --- |
| Umbrella CLI | Commands, Registry trust, ACL policy, confirmation, component orchestration, product UX, and trusted Runtime/Gateway composition. |
| Plugin Manager | Provider-neutral draft admission, actor/scope binding, two-pass Grant/provider binding, policy evaluation, durable planning/apply evidence, exact provider reconstruction and confirmation replay, in-process Use authorization forwarding, capability cutover, and fenced managed Workspace recovery. |
| A3S Use | Manifest validation, dependency resolution, immutable generations, provider/Grant planning semantics, receipts, journals, bindings, and capability reconciliation. Host assignments and policy stay outside package control. |
| Code lifecycle host | Delegates to the shared Use managed factory for native Task/stdio MCP, A3S Flow, static Skill/UI, scope-aware local OKF Knowledge, typed Runtime/Gateway retirement, and exact-generation Runtime Task dispatch. It consumes only reviewed Runtime evidence; the default host has no production provider or managed assignments. |
| Code TUI and Web | Consume one live snapshot and one host-selected Plugin Manager policy; neither implements a second package manager. |

The Use CLI response envelope remains schema v1, while the capability Registry
inside it is schema v2. Code rejects an older inner Registry instead of
confusing transport compatibility with capability compatibility.

Code configuration and plugin authorization are intentionally separate. The
workspace ACL may configure the agent, while only an explicit operator config
or the user-level config may authorize plugin operations. TUI, Web, detached
Web children, and the management MCP retain that distinction end to end.

`flow.json` is a Code-owned visual design and deployment document that can bind
to one immutable installed Flow identity. It is not a second workflow engine:
`a3s-flow` remains responsible for preflight, durable execution, event history,
and replay. See [A3S Use Component Platform](docs/a3s-use-component-platform.md)
for the lifecycle and installed-Flow contracts.

## A3S Code

`a3s code` is an agentic developer workspace, not only a chat prompt. It keeps
conversation, tool execution, approvals, workspace changes, memory, and
verification evidence in one semantic transcript.

| Area | Product surface |
| --- | --- |
| Coding | Streaming agent loop, workspace tools, bounded image file/clipboard input, saved-file Code Intelligence, bounded diffs, and Live Preview. |
| Control | Default, read-only Plan, and non-interactive Auto modes with exact grants, cancellable work, and closed automation tool profiles. |
| Continuity | Durable sessions, resume, priority-queued follow-ups, context search, memory, compaction, isolated worktree forks with digest-bound patch handoff, conflict-checked rewind, and local scheduled report loops with durable completion notifications. |
| Research | Evidence-first DeepResearch with bounded acquisition, citations, quality gates, and Markdown/HTML reports. |
| Assets | Local Agent, MCP, Skill, Flow, and OKF authoring; installed Flows and managed Knowledge bind by immutable package identity. |
| Models | ACL-configured providers plus account-owned Claude Code, Codex, Kimi, WorkBuddy, and A3S OS routes. |
| Integrations | VS Code/Cursor/Windsurf commands for bounded editor context and diff review, plus a permissioned repository-native GitHub Action. |

Everyday commands:

```bash
a3s code
a3s code resume
a3s code exec --mode auto "Fix the focused test and verify it"
a3s code exec --mode plan --tool-policy read-only "Review this workspace"
a3s code exec --image before.png,after.png "Compare these screenshots"
a3s code research --web "compare Tokio and async-std"
a3s code sandbox status
a3s code sandbox setup  # Windows: explicit one-time UAC setup
a3s code schedule enable daily-triage --every 1d
a3s code schedule notifications
a3s code remote diff <execution-id> --organization <organization-id>
a3s top --json
```

See [Code editor and CI integrations](docs/code-integrations.md) for extension
installation, Action usage, exact permission profiles, and the deliberately
closed automation boundary.

### Headless Agent releases

`a3s code harness` runs the immutable Agent release process declared by an
admitted `.a3s/asset.acl` manifest. The manifest is authoritative for the HTTP
port, readiness and liveness paths, shutdown deadline, protocol version,
capability requirements, external secret slots, and artifact identity; the
only host override is the listen interface.

```bash
a3s code harness --manifest /app/.a3s/asset.acl
a3s code harness --manifest /app/.a3s/asset.acl --listen 127.0.0.1
```

The version-one service exposes the manifest-declared health paths plus:

| Method and path | Contract |
| --- | --- |
| `POST /v1/agent/commands` | Exact start, cancellation, and checkpoint-recovery commands with immutable run identity. |
| `POST /v1/agent/events:page` | Bounded pages of the existing lossless `EventEnvelopeV1` stream. |
| `POST /v1/agent/changes` | Immutable, digest-checked Git-compatible change set for one terminal execution. |

Admission, required external-secret checks, configuration loading, and Agent
initialization complete before the listener becomes ready. Health and
structured error responses contain no secret values or release identity.
`SIGINT` and `SIGTERM` make readiness false before draining requests and close
the Harness within `health.shutdown_grace_seconds`.

The closed manifest schema, storage boundaries, compatibility rules, and
breaking-change policy are documented in the
[A3S Code Agent release contract](https://github.com/A3S-Lab/Code/blob/main/manual/AGENT_RELEASE_CONTRACT.md).

Useful TUI inputs:

```text
@src/main.rs                  attach a workspace file
! cargo test -p my-crate      run a direct shell turn
/status                       inspect session, model, modes, and token usage
/ide                          open the workspace browser and editor
/preview site/index.html      open a persistent local preview
/fork worktree               create an isolated branch, workspace, and session
/worktree handoff            emit a SHA-256-bound binary Git patch + manifest
/permissions                  change next-turn mode or review exact grants
/use status                   inspect Use setup and live capabilities
/packages                     review enable/disable for installed cognitive packages
/flow run                     run an exact installed Flow locally
/goal <outcome>               start a durable goal
/loop schedule daily-triage 1d  run an audited L1 report loop in the background
```

Press `/` to browse the grouped command palette. Search matches command names,
descriptions, and common concepts such as `git` or `auth`; a misspelled or
unknown slash command is rejected locally with suggestions instead of being
sent to the model.

### Local command sandbox

TUI, Web, and `a3s code exec` share the same verified local process sandbox.
Default and Auto run ordinary Bash calls inside that boundary; Plan exposes no
Bash. An explicit `require_escalated` request never escapes silently: Default
asks for the exact host command and Auto denies it. If sandbox preparation or
its native capability probe fails, Default asks for every otherwise valid host
command and Auto fails Bash closed. Catastrophic commands and credential or
control paths remain hard denials in every case.

Official archives carry an integrity-checked `@anthropic-ai/sandbox-runtime`
support tree and standalone self-update replaces it transactionally with the
CLI. Source builds can bootstrap the exact locked tree from npm when online;
arbitrary global `srt` executables are not selected. Node.js 20.11 or newer is
required. Linux also requires `bubblewrap`, `socat`, `ripgrep`, and enabled
unprivileged user namespaces; macOS requires `sandbox-exec` and `ripgrep`;
Windows requires the runtime's one-time elevated machine setup. The CLI probes
the real OS boundary before attaching it and never falls back silently to an
unsandboxed process.

The sandbox denies network egress and local listeners, limits writes to the
workspace and a private scratch directory, protects repository/control
metadata, hides common credential stores and nested `.env*` files, scrubs the
ambient environment, and rejects credential hard-link aliases. Delegated and
Skill child runs inherit the same frozen sandbox and permission snapshot.

### Local scheduled loops

Audited L1 engineered loops can run through a workspace-local singleton worker:

```bash
a3s code schedule enable daily-triage --every 1d
a3s code schedule run daily-triage
a3s code schedule status
a3s code schedule notifications
a3s code schedule disable daily-triage
```

The worker atomically claims each due run, skips missed-interval replay storms,
records interrupted work without silently rerunning uncertain effects, and keeps
pending notifications until the TUI or CLI renders and acknowledges them. Its
internal execution profile exposes bounded workspace reads, `git status`/`log`,
and writes only the selected loop's `STATE.md`, `RUN_LOG.md`, and `reports/`
artifacts. Shell, Web/network, MCP, Runtime, delegation, package execution,
unknown tools, denylisted paths, and the active ACL configuration remain closed.

The worker stays detached after the launching terminal exits. Opening the TUI
restarts it when enabled schedules exist; after an operating-system reboot, use
the TUI or `a3s code schedule start` to resume local schedules.

### Code Web

```bash
a3s web start
a3s web start --detach
a3s web start --detach --replace
a3s web start --api-only
```

Web reuses Code configuration, model routing, sessions, permission modes,
research, Code Intelligence, and the Plugin Manager. The default listener and
OAuth callback are loopback-only. A managed instance is workspace-scoped;
`--replace` can replace only an authenticated A3S process for that same
workspace and never terminates an ambiguous port owner. Managed cognitive
Knowledge uses the same exact-generation carrier as TUI through
`/api/v1/knowledge/packages` and `/api/v1/knowledge/packages/search`.

For each enabled Activity, the catalog exposes a document URL bound to the
exact Registry generation and revision. Code Web serves only the verified
HTML/CSS/JS through a restrictive opaque-origin CSP response; an old URL
returns `410 Gone` after upgrade, disable, or uninstall. The JSON content route
remains management data, not an executable document. Browser-side iframe
adoption, bounded v3 messages, reliable self-navigation interception, active
iframe drain, and exact-generation durable state are implemented. State is
never stored in the iframe origin or `localStorage`; the host authorizes each
request against the exact document generation. A hidden pre-cutover iframe now
proves the exact N+1 candidate with a dedicated authority-free port before Use
can publish it. A failed load, navigation, protocol exchange, or deadline rolls
back while N remains callable; a rolled-back plan cannot publish again, and a
fresh reviewed plan can retry without inflating the lifecycle generation.
Reviewed backend bindings, equivalent readiness in CLI/TUI/native hosts, and
native composition remain release work. Context must be reviewed before a
same-package Skill can enter Code.

## Component lifecycle

The base installation contains the umbrella CLI and A3S Code. Optional products
remain separately released:

| Component | Included | Public route | Lifecycle |
| --- | --- | --- | --- |
| Code | Yes | `a3s code` | Runs from the umbrella executable; release archives also carry its verified local-sandbox support tree. |
| Web | Release-dependent assets | `a3s web` | Bundled by release archives/Homebrew; Cargo installs fetch the matching verified asset on first start unless offline. |
| Box | No | `a3s box`, `a3s compose` | Visible first-use install or explicit preparation. |
| Bench | No | `a3s bench` | Explicit install; a compatible public control-component release remains a gate. |
| Search | No | `a3s search` | Explicit component install; embedded Code search and browser engines retain separate lifecycles. |
| Use | No | `a3s use`, `a3s code` | Explicit install or asynchronous first-use preparation when policy allows. |
| WebView | Release-dependent | native RemoteUI windows | Managed native companion with browser fallback. |

```bash
a3s list
a3s info use --versions --sources
a3s install use --source release --dry-run --json
a3s install use --source release --plan-digest <reviewedSha256> --json
a3s upgrade use --yes
a3s doctor use
a3s uninstall use --yes
```

Downloaded releases are checked for target, manifest, digest, ownership, and
health before the active receipt changes. Mutating batches use a cross-process
lock and durable checkpoints; an interrupted or failed upgrade leaves the
previous healthy generation available.

## Safety and configuration

| Mode | Workspace | Host shell | Boundary crossings |
| --- | --- | --- | --- |
| Default | Bounded reads and writes follow workspace policy. | Ordinary commands use the verified sandbox; explicit host escalation or missing-sandbox execution enters review. | Exact allow-once, session, or project grants. |
| Plan | Read-only discovery. | Bash is unavailable. | Approval starts a separate Default turn. |
| Auto | Governed operations run without prompts. | Ordinary commands use the verified sandbox; host escalation or a missing sandbox is denied. | Hard workspace and policy denials remain authoritative. |

The primary Code session receives `use_knowledge_search` only while a managed
OKF projection is active; Default, Plan, Auto, and research evidence collection
treat it as bounded read-only retrieval. Exact managed Runtime Tasks appear as
conservative `use_tool_*` tools only while their snapshot generation and named
reviewed provider are available. The dedicated Use worker receives only
verified package Skills, `mcp__use_*`, and `use_tool_*` tools. It has no
workspace shell, unrelated MCP access, or recursive delegation. Package
mutations and open-world operations return to the parent confirmation stream.

Configuration uses A3S ACL—not TOML or HCL. Resolution checks an explicit
`A3S_CONFIG_FILE`, workspace `.a3s/config.acl`, then `~/.a3s/config.acl`.

```bash
a3s model list
a3s model current
a3s model use codex/gpt-5.6-sol
a3s model use openai/my-model --scope workspace
a3s auth list
a3s auth login os
```

Account-owned providers keep control of their login state; A3S does not copy
their account tokens into `config.acl`, command output, logs, or the browser.

## Platform support

| Platform | Current guarantee |
| --- | --- |
| macOS arm64 / x86_64 | Primary Code, Web, component, Use, and native WebView release target; local command isolation uses `sandbox-exec`. |
| Linux arm64 / x86_64 | Primary Code, Web, component, Use, and headless runtime release target; local command isolation uses bubblewrap and user namespaces. |
| WSL | Uses the Linux runtime and filesystem contract. |
| Windows x86_64 | Preview: native Code/WebView and local sandbox support exist; the sandbox requires one explicit elevated machine setup. Complete Browser, six-surface, and failure-injection parity remains a gate. |

## Release readiness

The public CLI is usable for A3S Code, but the cognitive-package architecture
on `main` is **not yet a production release**. Promotion requires all of the
following:

- publish compatible A3S Use and Runtime crates/releases, then replace the
  current Git revision dependencies with exact published versions;
- complete the repository-owned CLI release and move public GitHub artifacts
  away from the former monorepo release path;
- publish and operationally validate the official Registry trust root;
- complete cross-platform crash injection and real-registry multi-root shared
  dependency lifecycle coverage;
- run the full real-process cross-platform reviewed-enablement and watcher
  convergence matrix for CLI, TUI, and Web;
- finish managed OKF rollback, coordinated restore and authority recovery,
  backup rotation, and distributed placement; exact published-generation query
  leases, scope quota, bounded retention, tombstone GC, integrity audit,
  derived-index repair, and verifiable scope-local backup are implemented in
  the composed SQLite backend;
- inject production Runtime assignments, readiness, and Gateway bindings for
  release-backed OCI Tasks, long-lived Tool Services, and HTTP MCP. The
  injected OCI Task path now proves install plus offline, restart-safe reviewed
  disable/re-enable, provider-drift recovery, and exact receipt-backed Task
  invocation after Manager restart. Accepted calls now lease their published
  generation through output capture and Runtime cleanup, while hide rejects
  new calls. The snapshot-v2 watcher now projects exact Tasks into TUI/Web when
  their reviewed provider is present, but a default production provider is
  still absent; Use contract tests prove exact receipt-owned Service
  retirement for uninstall and prior-generation cleanup. The real-provider
  cross-platform uninstall/upgrade recovery matrix remains open;
- close the remaining native Windows six-surface and failure-injection
  package-lifecycle parity; and
- finish reviewed Activity backend bindings and equivalent generation-aware
  readiness/sandbox composition in CLI, TUI, and native hosts; Code Web's
  failed-N+1 pre-cutover proof, rollback, fresh-plan retry, and single cutover
  are implemented; and
- define production scheduling, recovery, and retention for Flow beyond the
  current single-node local runtime.

Until those gates close, unavailable capabilities remain visible as
unavailable and fail closed.

## Development

Work in this repository directly or through the A3S monorepo's pinned
`crates/cli` submodule. Do not create a Rust workspace at the monorepo root.

The lockfile pins the current graph, including exact Git revisions for A3S Use
and Runtime on `main`. This makes the evaluated source graph reproducible, but
it is also why this branch must not be described as crates.io-release-ready.

```bash
cargo fmt --all -- --check
cargo test --lib
cargo test --tests
cargo clippy --all-targets --all-features -- -D warnings
```

Focused package-host gates:

```bash
cargo test --lib use_registry::tests:: --no-fail-fast
cargo test --bin a3s tui::panels::packages::tests --no-fail-fast
cargo test --test web_plugin_marketplace \
  marketplace_install_upgrade_uninstall_hot_plugs_verified_activity_skill_and_flow_catalog
cargo test --test web_plugin_marketplace \
  reviewed_enablement_hot_plugs_skill_ui_and_flow_and_replays_after_restart
cargo test \
  generation_watch_hot_plugs_skill_mcp_runtime_task_flow_and_knowledge_across_tui_and_web
cargo test --lib \
  use_registry::runtime_tasks::tests --no-fail-fast
cargo test --lib \
  use_registry::knowledge::tests --no-fail-fast
cargo test --bin a3s \
  complete_code_web_module_builds_with_nested_remote_kernel_imports
cargo test --bin a3s \
  bound_flow_deploy_resolves_fake_use_catalog_before_os_mutation
cargo test --lib \
  code_host_preflights_flow_and_persists_exact_generation_binding
cargo test --lib \
  signed_okf_install_upgrade_restart_query_and_uninstall_use_code_host
cargo test --lib \
  reviewed_managed_runtime_graph_rejects_drift_and_persists_exact_grant
cargo test --lib \
  signed_workspace_install_is_exact_fenced_and_replayable_after_restart
cargo test --lib \
  reviewed_managed_runtime_graph_rejects_drift_and_persists_exact_grant
cargo test --lib plugin_manager::operation
cargo test --lib plugin_manager::managed_host
cargo test --lib components::cognitive_lifecycle
```

The real separate-process Use integration is orchestrated from the monorepo so
its Cargo outputs stay isolated:

```bash
just use-hotplug-e2e
```

## Documentation

- [CLI reference](docs/cli-reference.md)
- [CLI product design](docs/cli-product-design.md)
- [CLI technical architecture](docs/cli-technical-architecture.md)
- [A3S Use Component Platform](docs/a3s-use-component-platform.md)
- [Plugin authorization policy](docs/plugin-authorization-policy.md)
- [Code Intelligence](docs/code-intelligence.md)
- [DeepResearch evidence-first design](docs/deep-research-evidence-first-redesign.md)
- [Immutable Agent release contract](https://github.com/A3S-Lab/Code/blob/main/manual/AGENT_RELEASE_CONTRACT.md)
- [A3S Use website](https://a3s-lab.github.io/Use/)
- [A3S Use package contracts](https://github.com/A3S-Lab/Use/tree/main/docs)

## Updating

```bash
a3s self update --check
a3s self update
a3s upgrade use
```

`a3s update` and `a3s update <component>` remain compatibility aliases but are
deprecated. The TUI `/update` saves the current session, updates the CLI, and
resumes it. Component upgrades preserve their owning provenance.

## License

A3S CLI is licensed under the [MIT License](LICENSE). Release archives retain
the licenses and provenance notices of their bundled components.
