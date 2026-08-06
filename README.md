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
| Linux, macOS, and Windows CI | Build and repository test suites on all three operating-system families. |
| Reviewed Use authorization bridge | A real signed schema-v3 Skill install keeps the umbrella operation ID, canonical plan, package lock, and persisted confirmation inside the in-process Use graph. The compatibility CLI/Web toggle advances the same Use-owned package-state generation for permission-free packages; Registry identity drift and operation substitution fail closed, and no child `a3s` mutation is launched. |
| Fenced managed Workspace host | Protocol v4 explicitly plans a signed package's enable/disable transition as plan-v4, binds user confirmation to its operation ID and digest, and applies through the existing host apply request. Planning evidence, apply intent, capability cutover, and result survive host recreation; stale generations, request or digest substitution, package-byte changes, and dependency-graph changes fail closed. A permission-bearing Tool regression proves that missing confirmation creates no apply intent or lifecycle mutation. |
| TUI first-use integration | A separately executed A3S Use process installs while Code remains responsive, then projects ready capabilities. |
| Web Marketplace lifecycle | Install, upgrade, and uninstall through the public Web API while verified Activity, Skill, and Flow entries appear and disappear without a Web restart. |
| Release-bundle recovery | A detached Web host recovers the package catalog and durable Flow history after restart. |

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
| **UI** | Sandboxed Web Activity projection with bounded host messages. | General-purpose native UI hosting. |
| **MCP** | Verified stdio MCP lifecycle. | HTTP MCP until the production Gateway adapter is injected. |
| **Tool** | Executable Task lifecycle through Runtime. | Long-lived Service execution until a production Runtime Service adapter is injected. |
| **A3S Flow** | Native TypeScript preflight, exact-generation binding, durable local runs, status, and history. | Web run/history controls, distributed placement, automatic resumption, and production retention. |
| **OKF** | Manifest, dependency, plan, validation, and Use lifecycle contracts. | Runtime knowledge projection until a production A3S Knowledge adapter is injected. |

Required surfaces fail closed when their adapter or evidence is unavailable;
they never silently downgrade to a different provider.

### Reviewed lifecycle

With an explicitly trusted Registry configured, metadata can be searched
without downloading package archives. Mutations create an immutable plan
before they change the active generation:

```bash
a3s plugin search science
a3s plugin inspect a3s/science

# Interactive review and apply
a3s plugin install a3s/science --channel stable
a3s plugin upgrade a3s/science
a3s plugin disable a3s/science
a3s plugin enable a3s/science
a3s plugin uninstall a3s/science

# Non-interactive two-step apply
a3s --output json plugin disable a3s/science --dry-run
a3s --output json plugin apply <operationId> \
  --plan-digest <canonicalPlanDigest> \
  --yes
```

For a complete schema-v3 plan, Plugin Manager persists the exact confirmation,
reconstructs only the Registry identities frozen in the reviewed package lock,
and invokes A3S Use in-process with
`ReviewedCognitivePackageAuthorizationProvider`. Use must reproduce the same
operation ID, plan digest, package transitions, impact, state revision, and
lock before it may mutate. Legacy component-only plans retain their bounded
subprocess compatibility path.

For a locked cognitive-package graph plan, Plugin Manager snapshots the A3S
Use Grant store at the plan's exact scope and durable state revision. It first
evaluates policy against the provisional activation, asks A3S Use to bind the
canonical Grant impact with the final host authority, and evaluates the final
plan again. Scope, revision, prebound-impact, or authority drift fails before
apply. A signed Tool Task regression proves that the reviewed permission graph
persists its exact Grant receipt without launching the legacy child mutation,
and that the completed operation replays idempotently.

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
enablement plans as well as install, upgrade, and uninstall plans. The old Web
`/packages/enabled` route is compatibility-only for schema-v1/v2 receipts; a
schema-v3 failure never falls back to it.

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

### Replaceable Registry sources

Registry URL and trust identity belong to host configuration, never to an
untrusted package. A mirror or private source can be added, disabled, or
replaced without changing resolver code:

```bash
a3s registry add https://packages.example.org/a3s/ \
  --trust-root ./root.json \
  --yes
a3s registry refresh packages
a3s registry disable packages --yes
a3s registry replace packages https://mirror.example.org/a3s/ \
  --trust-root ./mirror-root.json \
  --yes
a3s registry enable packages --yes
```

Installed receipts remain pinned to the Registry identity that supplied them.
Replacing a source does not rewrite those receipts; an upgrade fails closed
until provenance is restored or explicitly migrated. The official production
Registry root is not yet public, so these commands currently require a source
the operator deliberately trusts.

## Architecture

<p align="center">
  <img
    src="assets/readme/cognitive-hotplug-architecture.svg"
    width="100%"
    alt="Trusted package sources pass through one Plugin Manager and A3S Use graph before exact-generation capabilities reach Code TUI and Web"
  />
</p>

| Owner | Responsibility |
| --- | --- |
| Umbrella CLI | Commands, Registry trust, ACL policy, confirmation, component orchestration, and product UX. |
| Plugin Manager | Reviewed plans, actor and scope binding, durable planning/apply evidence, exact confirmation replay, in-process Use authorization forwarding, Grant cutover evidence, and fenced managed Workspace recovery. |
| A3S Use | Manifest validation, dependency resolution, immutable generations, receipts, journals, bindings, and capability reconciliation. |
| Code lifecycle host | Composes the adapters actually available and rejects required surfaces that are not ready. |
| Code TUI and Web | Consume one live snapshot; neither implements a second package manager. |

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
| Coding | Streaming agent loop, workspace tools, file attachments, image paste, saved-file Code Intelligence, bounded diffs, and Live Preview. |
| Control | Default, read-only Plan, and non-interactive Auto modes with exact grants and cancellable work. |
| Continuity | Durable sessions, resume, queued follow-ups, context search, memory, compaction, forks, and conflict-checked rewind. |
| Research | Evidence-first DeepResearch with bounded acquisition, citations, quality gates, and Markdown/HTML reports. |
| Assets | Local Agent, MCP, Skill, Flow, and OKF authoring; installed Flows bind by immutable package identity. |
| Models | ACL-configured providers plus account-owned Claude Code, Codex, Kimi, WorkBuddy, and A3S OS routes. |

Everyday commands:

```bash
a3s code
a3s code resume
a3s code exec --mode auto "Fix the focused test and verify it"
a3s code research --web "compare Tokio and async-std"
a3s top --json
```

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
/ide                          open the workspace browser and editor
/preview site/index.html      open a persistent local preview
/permissions                  review or revoke exact grants
/use status                   inspect Use setup and live capabilities
/packages                     review enable/disable for installed cognitive packages
/flow run                     run an exact installed Flow locally
/goal <outcome>               start a durable goal
```

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
workspace and never terminates an ambiguous port owner.

Package HTML runs in an opaque-origin iframe with restrictive CSP and bounded
messages. Context must be reviewed before a same-package Skill can enter Code.

## Component lifecycle

The base installation contains the umbrella CLI and A3S Code. Optional products
remain separately released:

| Component | Included | Public route | Lifecycle |
| --- | --- | --- | --- |
| Code | Yes | `a3s code` | Runs from the umbrella executable. |
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
| Default | Bounded reads and writes follow workspace policy. | Proven read-only commands run quietly; other non-critical commands enter review. | Exact allow-once, session, or project grants. |
| Plan | Read-only discovery. | Bash is unavailable. | Approval starts a separate Default turn. |
| Auto | Governed operations run without prompts. | Proven read-only commands run; unproven or mutating host commands are denied. | Hard workspace and policy denials remain authoritative. |

The dedicated Use worker receives only verified package Skills and
`mcp__use_*` tools. It has no workspace shell, unrelated MCP access, or
recursive delegation. Package mutations and open-world operations return to
the parent confirmation stream.

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
| macOS arm64 / x86_64 | Primary Code, Web, component, Use, and native WebView release target. |
| Linux arm64 / x86_64 | Primary Code, Web, component, Use, and headless runtime release target. |
| WSL | Uses the Linux runtime and filesystem contract. |
| Windows x86_64 | Preview: native Code/WebView and verified Use ZIP paths exist; complete Browser and cognitive-package lifecycle parity remains a gate. |

## Release readiness

The public CLI is usable for A3S Code, but the cognitive-package architecture
on `main` is **not yet a production release**. Promotion requires all of the
following:

- publish compatible A3S Use and Runtime crates/releases, then replace the
  current Git revision dependencies with exact published versions;
- complete the repository-owned CLI release and move public GitHub artifacts
  away from the former monorepo release path;
- publish and operationally validate the official Registry trust root;
- complete host-reviewed dependency-graph upgrade/uninstall planning and crash
  injection on every supported platform;
- run the full real-process cross-platform reviewed-enablement and watcher
  convergence matrix for CLI, TUI, and Web;
- inject production OKF/Knowledge, HTTP MCP/Gateway, and long-lived Tool
  Service adapters;
- close native Windows package-lifecycle parity; and
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
  reviewed_enablement_hot_plugs_web_and_replays_after_restart
cargo test \
  generation_watch_hot_plugs_and_disables_skill_mcp_and_flow_catalog
cargo test --bin a3s \
  bound_flow_deploy_resolves_fake_use_catalog_before_os_mutation
cargo test --lib \
  code_host_preflights_flow_and_persists_exact_generation_binding
cargo test --lib \
  signed_workspace_install_is_exact_fenced_and_replayable_after_restart
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
