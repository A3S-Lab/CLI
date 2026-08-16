# a3s

The umbrella CLI for the [A3S](https://github.com/A3S-Lab) platform.

`a3s <tool> [args...]` runs the matching A3S product. Code is included in the
main `a3s` installation. Box, Bench, Search, and Use are separately released
components. Box and Use may be installed visibly on first real use; every
optional component can also be prepared explicitly.

```
a3s code                       # launch the included A3S Code TUI
a3s web -d                     # start A3S Web in the background
a3s box ps                     # run Box, installing it on first real use
a3s up -d                      # converge the local Compose application with Box
a3s ps                         # list services in the local Compose application
a3s compose exec api -- sh     # use the complete Box Compose command namespace
a3s bench run ./tasks/smoke --agent codex
a3s search doctor              # run the registered Search product
a3s use browser open https://example.com
a3s use box compose up -d      # route through Use to the one managed Box
a3s list                       # list registered components and external tools
a3s install use                # install or repair a registered component
a3s upgrade use                # upgrade one managed component
a3s uninstall use              # remove component-owned files
a3s --version
```

Headless search browsers are execution dependencies of the embedded
`a3s-search` library and have their own lifecycle commands:

```bash
a3s search engines
a3s search browser install chrome
a3s search browser update chrome
a3s search browser repair lightpanda
```

The bundled Code runtime uses DuckDuckGo and Wikipedia as `web_search`'s
defaults when no request or ACL engine selection is provided. Configure the
`search.engine` entries in `config.acl` to replace that default, including an
explicit `anysearch { enabled = true }` entry when desired. AnySearch can use
`ANYSEARCH_API_KEY` when set. A failure or empty result from any selected
engine enters the same bounded fallback policy; quota exhaustion and other
provider failures remain visible in structured search metadata.

Managed downloads remain under `~/.a3s/chromium/` and
`~/.a3s/lightpanda/`. `a3s search doctor` reads the same project-local or
user-global `config.acl` selected by `a3s code`, reports enabled headless
engines, and returns an actionable install command when the configured backend
is unavailable.

## Install

```sh
# from crates.io
cargo install a3s

# or from source
cargo install --git https://github.com/A3S-Lab/CLI

# or Homebrew
brew install A3S-Lab/tap/a3s
```

The initial installation always contains the umbrella CLI and A3S Code. It does
not download Box, Bench, Search, Use, or WebView. This keeps a Code-only
installation small, while every product still has one public entry point under
`a3s`.
Homebrew and GitHub archives bundle the Web workspace. A Cargo installation
downloads the CLI's exact-version Web asset on the first `a3s web` start,
verifies its SHA-256, and caches it in the A3S data directory. `--offline` and
`A3S_NO_AUTO_INSTALL=1` never perform that download; use `--web-dir` for a
local build or `--api-only` when no frontend is required.

Release packages install the native `a3s-webview` companion on supported
desktop platforms. It owns both RemoteUI windows and the system-agent island.
If a source or Cargo installation does not have that helper, RemoteUI falls
back immediately to the browser and agent-island startup is skipped with a
diagnostic; neither condition prevents the TUI from starting.

### Weixin channel activation

A3S Web uses the native Rust `IlinkModule` provided by A3S Boot. The real
Tencent QR flow is available by default and does not require browser secrets,
environment variables, or an application identity in `config.acl`.

```acl
channels {
  weixin {
    enabled = true
  }
}
```

The block is optional; use `enabled = false` only when the local operator wants
to disable the channel explicitly. **Settings → Channels → Weixin** exposes QR
binding and starts the supervised read-only monitor after binding. Boot owns
the Tencent-compatible `iLink-App-Id: bot`, `bot_type=3`, protocol version,
headers, URL policy, polling, and message transport. The host still identifies
itself as `A3S/<version>` through `bot_agent`.

Unsafe local storage or invalid server routing keeps the channel unavailable or
degraded. Tokens, owner IDs, cursors, and authenticated iLink URLs never enter
the browser.

### Components and delayed installation

The built-in catalog registers `code`, `box`, `bench`, `search`, and `use`.
Browser, native Office, and OCR are capabilities owned by Use:

| Component | Installed with `a3s` | Public command | Installation behavior |
| --- | --- | --- | --- |
| Code | Yes | `a3s code ...` | Runs directly from the main `a3s` installation. |
| Box | No | `a3s box ...` | Installs Box on first use, then forwards the arguments to it. |
| Bench | No | `a3s bench ...` | Requires an explicit compatible Bench installation. |
| Search | No | `a3s search ...` | Requires an explicit compatible Search installation. |
| Use | No | `a3s use ...` / `a3s code` | Installs Use on first real use, including TUI startup when policy allows, then forwards or projects native Browser, Office, OCR, Box, or extension surfaces. |
| Use/Browser | With Use | `a3s use browser ...` | Reports Browser provider readiness through Use; it is not a second product archive. |
| Use/Office | With Use | `a3s use office ...` | Delegates OfficeCLI readiness and explicit installation through Use. |
| Use/OCR | With Use | `a3s use ocr ...` | Projects the built-in local PP-OCRv6 tools and Skill; install or repair its pinned models with `a3s install use/ocr`. |

First-use installation for opted-in components is persistent and user-wide. After a component has been
installed, subsequent commands reuse it; changing projects does not download it
again. Bench validates a downloaded bundle before switching its active-version
record, so a failed Bench download or validation is not reported as installed.
Run `a3s list` at any time to inspect local state without installing or updating
anything.

Help and version probes are read-only as well. If Box or Bench is missing,
`a3s box --help`, `a3s bench --help`, their nested `--help` forms, and
`--version` report wrapper/component status without triggering delayed
installation. The missing-component Bench help still shows its four normal
commands and explicit local-path rule. Once installed, those arguments are
forwarded to the component for command-specific help.

For example, a new user can start Code immediately and let the other products
arrive only when needed:

```sh
a3s code
a3s box ps
a3s install bench
a3s bench run ./tasks/smoke --agent codex
```

The first `a3s box ...` command resolves or installs Box. Bench and Search
require explicit installation so an evaluation or search command never starts
an unplanned download. Users still type `a3s bench`; its private executable is
not installed as another public command on `PATH`.

### Compose applications

`a3s compose` is the canonical multi-service application namespace. It resolves
the registered Box component and forwards the remaining arguments without a
shell or a second Compose parser:

```sh
a3s compose config                  # discovers compose.acl first
a3s compose up -d
a3s compose exec api -- sh
a3s compose down --volumes
a3s compose -f compose.yaml config # explicit YAML remains supported
```

The everyday project commands also have concise top-level forms:

```sh
a3s up -d
a3s ps
a3s logs --follow api
a3s down --volumes
```

These are exact routes to `a3s-box compose up|ps|logs|down`; project discovery,
service state, networks, volumes, health checks, and rollback remain owned by
Box. The global `-C/--directory` option selects the Compose project directory.
Because the route is transparent, newly shipped Box Compose subcommands become
available under `a3s compose` without a matching umbrella CLI release. The
canonical `compose.acl` schema, validation, environment resolution, and YAML
compatibility are all implemented once by Box.

The Bench control component compiles and locks tasks, plans trials, coordinates
evaluation, and produces scores and reports. It does not execute an Agent.
Candidate and Judge Agent Assets are both executed by A3S OS Runtime, which is
the sole Agent execution layer. This keeps sandboxing, credentials, model
access, resource limits, and execution evidence in the shared OS Runtime rather
than duplicating execution infrastructure in the control component.

### Explicit component installation

Use `a3s install` to prepare a component before its first use, for example on a
CI runner or before going offline:

```sh
a3s install code
a3s install box
a3s install bench
a3s install search
a3s install use
a3s install webview
a3s install use/browser
a3s install use/office
```

Installation is idempotent. If the requested component is already healthy,
`a3s` reports the available local version and location metadata instead of
downloading it again.

`a3s install code` reconciles the Code component already delivered by the
running `a3s` executable; it does not create a second `a3s-code` installation.
The very first installation of `a3s` itself must still be performed with Cargo,
Homebrew, or another supported system installer as shown above.

`a3s install box` performs the same installation that `a3s box ...` would
trigger automatically. `a3s install bench` downloads and validates the private
Bench control component without running a benchmark. This is the preferred
preparation step for machines whose benchmark run will not have network access.

The Bench repository currently publishes the canonical design and fixtures but
not a compatible control-component release. Until that release exists,
`a3s install bench` and direct `a3s bench ...` use fail with an explicit
diagnostic that the control component is not published and do not create an
installed-component record. Bench does not opt into first-use installation.

### Signed extension registries

External Use domains can be resolved from explicitly trusted TUF registries.
No Registry source is implicit. The CLI never invents or silently accepts a
replacement root. Add a named source with a pinned SHA-256 and optionally
import the exact root file:

```sh
a3s registry add packages https://packages.example.org/a3s/ \
  --root-sha256 <root-sha256> \
  --trusted-root ./root.json \
  --yes
a3s registry refresh packages

a3s --output json install use/acme/research --dry-run
a3s --output json install use/acme/research \
  --plan-digest <reviewed-plan-sha256>

a3s --output json upgrade use/acme/research --dry-run
a3s --output json upgrade use/acme/research \
  --plan-digest <reviewed-upgrade-sha256>
```

Root files are copied beneath the Use-owned
`state/use/registry-trust-roots/sha256/<digest>.json` path and checked against
the recorded digest whenever configuration is loaded. The canonical source
document is `state/use/registries.acl`; active CLI config selection never
creates another Registry state. A
digest-only registry bootstraps from `<registry>/metadata/root.json`; the root
is cached only after its bytes match the pin. `registry refresh` performs full
TUF root, timestamp, snapshot, and targets verification, including expiration
and rollback checks. It does not use a reachability-only `HEAD` request and does
not download package targets.

Install planning starts from the selected `--registry-name` or the configured
default, then uses the other enabled sources only for dependency resolution.
No match is an error, and the same package resolving from more than one trusted registry is
rejected as ambiguous. Cognitive-package installs accept only signed targets
from these explicitly trusted registries. There is no unsigned or local-package
installation bypass. Registry URLs must use HTTPS, except loopback HTTP used by
the hermetic test suite.

A cognitive-package upgrade reads the complete source provenance from the
installed Use receipt and queries only that registry and release channel. It
refuses a removed registry, a changed URL or trust root, and a semantic-version
downgrade. `a3s upgrade --all` includes eligible signed Registry packages.
Plain `a3s upgrade` includes newer signed targets in its update listing. When
verified metadata resolves to the already installed target, the operation
converges without downloading the archive or publishing a new activation.

### Reviewable component plans

Installation, upgrade, and uninstall support an immutable review/apply guard:

```sh
a3s install box --source release --dry-run --json
a3s install box --source release --plan-digest <reviewed-sha256> --json

a3s upgrade box --dry-run --json
a3s uninstall box --purge --dry-run --json
```

The dry-run result contains `planSchemaVersion`, `planCommand`, `planDigest`,
and the ordered `plans` array. The digest covers the requested operation and
flags, target platform, current component state and normalized receipt,
operation order and every exact GitHub release version, asset URL, asset name,
and SHA-256 selected during resolution. For cognitive packages it also covers
the registry name and URL, pinned root, all TUF metadata versions, package
version and channel, platform target, target path, archive length and SHA-256,
complete catalog-v3 record, exact dependency lock, and every signed executable
planning target. Presentation text is deliberately excluded.

A release dry-run reads release metadata and, when necessary, its checksum
file. It never downloads the component payload or changes component state. An
apply with `--plan-digest` resolves the plan again and fails with
`component.plan_mismatch` before payload download or mutation when anything
covered by the digest changed. Once accepted, the installer consumes the exact
resolved artifact from that plan instead of performing another `latest`
lookup.

Cognitive-package dry-runs verify metadata without downloading the archive. On
apply, the umbrella digest is checked first; `a3s` then passes the exact
resolved package, complete dependency lock, planning bundles, and inner
registry-plan digest to A3S Use in-process. Use repeats TUF verification before
target download, so a repository change between the outer check and payload
fetch also fails without activation.

`--plan-digest` and `--dry-run` are mutually exclusive. Homebrew plans bind the
formula, operation, flags, and current receipt, but Homebrew remains responsible
for selecting and installing its current bottle; use `--source release` when an
exact A3S release artifact must be bound by the digest.

### Durable component batches

Mutating `install`, `upgrade`, and `uninstall` batches are serialized by one
cross-process batch lock. After the operation plan has been resolved and any
expected digest has been verified, A3S writes an active journal before the
first component mutation. Every component success or failure is then
checkpointed with an atomic write and filesystem sync.

If the process or machine stops during a batch, rerunning the same action with
the same ordered component list resolves and verifies a fresh plan. A3S does
not replay a stale download plan. Instead, it validates every completed
checkpoint against current component presence, health, version, provenance,
and executable path. A still-applied checkpoint is skipped and appears in JSON
output with `"recovered": true`; a checkpoint whose state has drifted is
executed again through the normal idempotent lifecycle path. Duplicate
component IDs are rejected before locking or mutation.

Batch recovery preserves the existing partial-success contract: completed
components are not rolled back when another component fails. Normal failed
batches are finalized as failed; only an interrupted active batch is eligible
for checkpoint recovery. Journals live below the resolved A3S state root:

```text
component-operations/active.json
component-operations/last.json
component-operations/last-interrupted.json
```

The directory and files use private permissions on Unix. Invalid, oversized,
or symbolic-link active journals fail closed instead of being ignored.

### Listing installed components

```sh
a3s list
```

The list distinguishes bundled Code, optional product components, delegated
Use capabilities, and explicitly installed external Use domains. It reports
whether each entry is installed, missing, disabled, or broken. For
an installed component it includes version metadata when it can be read without
executing the component, plus its source and executable location. A Box found
on `PATH` can therefore show `-` for version: `a3s list` deliberately does not
run third-party commands just to probe them. Other executable `a3s-*` tools
found on `PATH` remain visible as additional tools; they are not treated as
managed A3S product components.

`a3s list` is a local inspection command. It does not contact a release server,
install a missing component, or update an installed one.

### Bench control-component files and project state

The Bench control component and benchmark project data have different lifetimes
and must not be mixed:

```text
~/.a3s/components/bench/   user-wide, versioned Bench control component
<project>/.a3s/bench/      locks, plans, attempts, evidence, and reports for one project
```

The global `~/.a3s/components/bench/` directory is owned by the component
manager and shared by every workspace. It contains validated versioned payloads
and the active-version record used by `a3s bench`. Set `A3S_COMPONENTS_DIR` when
the user-wide component root must live elsewhere; the `bench/` component remains
under that root.

The project-local `.a3s/bench/` directory is owned by the benchmark workflow.
It contains reproducibility locks and run state for that project, not the Bench
control component. Archiving or removing a project's `.a3s/bench/` state does
not uninstall Bench, and updating the global control component does not rewrite
a project's locked task, agent, plan, evidence, or report data. Benchmark
project state always uses `.a3s/bench/`; no separate top-level benchmark state
directory is created.

## Components And A3S Use

The umbrella CLI owns the catalog and delegates domain-specific runtimes to
their parent component:

```sh
a3s list --json
a3s install use
a3s install use/browser
a3s install use/office
a3s info use/ocr --sources
a3s doctor use/ocr
a3s uninstall use/office

a3s use capabilities --json
a3s use browser render https://example.com
a3s use browser open https://example.com --session research
a3s use browser snapshot --session research --json
a3s use browser close --session research
a3s use ocr doctor --json
a3s use box compose up --detach
a3s use knowledge usage --json
a3s use knowledge usage --scope-kind workspace --scope-id workspace/acme --json
a3s use knowledge audit --scope-kind workspace --scope-id workspace/acme --json
a3s use knowledge backup ./workspace.a3s-okf-backup --scope-kind workspace --scope-id workspace/acme --json
a3s use knowledge verify-backup ./workspace.a3s-okf-backup --scope-kind workspace --scope-id workspace/acme --json
a3s use knowledge repair-search-index --yes --scope-kind workspace --scope-id workspace/acme --json
a3s --output json plugin disable acme/slack --dry-run
a3s --output json plugin apply <operationId> --plan-digest <sha256> --yes
a3s use extension watch --after-generation 3 --timeout-ms 30000 --json
```

Browser, native Office, and OCR are built-in Use domains. Independently
implemented domains can be explicitly installed from A3S ACL packages that
declare native CLI, standard MCP, and/or `SKILL.md` surfaces. A3S does not
define an extension JSON-RPC protocol; `--json` remains a one-command CLI
result.

The Code TUI and Code Web treat Use as an asynchronous first-use component:
terminal and Web-server startup do not wait for Use discovery, download,
installation, or initial projection. Those steps continue in the background
when networking and automatic setup are allowed, and the live registry is
hot-plugged into current and future Code sessions when ready. Offline mode and
`A3S_NO_AUTO_INSTALL=1` remain strict no-mutation boundaries, and a failed or
slow setup does not prevent either surface from starting. Both surfaces keep
one registry watcher for the process. Browser, Office, OCR, enabled external
MCP/Skill surfaces, installed Flows, and exact promoted OKF projections are
projected into every active Code session. Code registers a dedicated `use`
worker that can
invoke only `mcp__use_*` tools; workspace, shell, unrelated MCP, and recursive
delegation tools are denied. The worker's current capability IDs and purpose are
published in the live `task` definition, so the parent
model can select it without a hard-coded prompt. Application failures do not
fall back to another execution surface, and an Office
`use.office.outcome_unknown` result is never retried automatically. A session
rebuild replays the current surfaces, and a Web process shares the watcher
across all concurrent sessions.

Managed OKF packages use a separate read-only session tool,
`use_knowledge_search`; they are not exposed as raw package text or delegated
to the Use worker. The tool appears only while at least one exact projection is
active and searches the current User/Workspace scope through the Use-owned
SQLite/FTS5 adapter. Results retain package, generation, projection, index,
concept-path, and source-digest citations. Before touching SQLite, the carrier
deduplicates projected surfaces by exact package generation and acquires a Use
Registry lease bound to the package digest, manifest digest, and lifecycle
generation. It holds every lease through search and final Registry revision
verification; missing or contradictory generation evidence fails closed, and
an accepted query delays prior-generation retirement until it completes. A
racing cutover is retried once without returning stale results. Code Web
exposes the same carrier at `GET /api/v1/knowledge/packages` and
`POST /api/v1/knowledge/packages/search`. These routes are separate from the
personal `/kb` vault.

`a3s use knowledge usage --json` reports non-secret storage evidence for the
default User scope. Workspace accounting requires `--scope-kind workspace` and
an exact `--scope-id`; the CLI does not guess Workspace identity. The default
Use policy bounds receipt-accounted expanded content, retained projections,
generations per surface, and removal tombstones, and receipt-owned removal
physically compacts SQLite and its WAL.

`a3s use knowledge audit` validates the exact scope's SQLite, foreign keys,
receipts, accounting, identity, and FTS index. `backup <path>` first audits and
then writes a new non-overwriting versioned snapshot; `verify-backup <path>`
checks its bounded manifest, scope, database digest, storage evidence, and FTS
integrity without changing live state. `repair-search-index --yes` can rebuild
only the derived FTS rows after authoritative state has passed validation.
These commands do not restore Registry receipts, package roots, bindings,
journals, Grants, Flow history, or UI state. Coordinated restore is not yet
implemented, and copying a verified snapshot into live state is unsupported.

Inside the TUI, `/use` and `/use status` report background setup progress, the
discovered binary, generation/revision convergence, provider readiness, MCP
connection/tool count, verified/loaded Skills, ready Flows, and managed OKF
projection counts. `/use repair` waits for an in-flight setup to settle before
printing explicit repair commands, but never executes them. The primary model
does not receive raw `mcp__use_*`
definitions; only the dedicated worker does. Closed-world read-only MCP tools,
including local PP-OCRv6 doctor and extraction, can proceed without another
prompt. Missing annotations, open-world access, mutations, destructive
operations, and submit risk escalate to the parent TUI. Parent denials remain
authoritative.

The TUI derives capability lifecycle labels only from standard MCP progress
emitted by the dedicated `use` worker: Browser progress appears as
`Using Browser` while live and `Used Browser` after completion. Multiple routes
remain ordered and deduplicated, restored task snapshots preserve the same
identity, and raw MCP tool names do not replace the user-facing worker label.

### Platform support

macOS and Linux remain the broad runtime and managed-artifact targets for the
component platform. Windows x86_64 now supports the native WebView managed
release and Code first-use installation. A real-process Windows E2E additionally
covers the verified Use ZIP layout, all 31 Browser core-profile tools against
Microsoft Edge, every native Office MCP operation and view, confirmed OfficeCLI
installation, and confirmed PP-OCRv6 model installation plus extraction.
Advanced Browser profiles, persistent-session promotion, and full file-lock
conformance remain roadmap work.

`a3s install` manages registered A3S components; it is not a universal frontend
for Homebrew, APT, DNF, Pacman, Winget, npm, pip, Cargo, or arbitrary package
names. Native package-manager adapters remain ownership-preserving and will be
added only for a registered component with release artifacts and conformance
tests.

## A3S Code TUI

`a3s code` launches the interactive A3S Code terminal UI in the current
workspace. On first launch it creates `~/.a3s/config.acl`; use `/config` to edit
models, provider credentials, and optional paths such as `flow_dir`,
`agent_dir`, `mcp_dir`, `skill_dir`, and memory/session storage.

A3S Code is a complete agentic workspace. It combines a coding-agent chat loop,
workspace editor, durable context, local asset development, OS asset publishing,
Runtime fan-out, RemoteUI views, and engineered automation loops in one terminal
surface.

Use this README as the TUI capability guide:

- [A3S Code CLI Command Examples](#a3s-code-cli-command-examples) shows
  copyable non-interactive command forms and how they map to TUI workflows.
- [Capability Overview](#capability-overview) maps the major product surfaces.
- [Everyday Capability Paths](#everyday-capability-paths) explains how those
  surfaces fit together during real work.
- [Inside The TUI](#inside-the-tui) explains the interactive transcript,
  input modes, panels, and keyboard model.
- [Code Intelligence](docs/code-intelligence.md) documents saved-file symbols,
  navigation, diagnostics, language prerequisites, and the shared TUI/Web
  behavior.
- [Startup, Sessions, And Safety](#startup-sessions-and-safety) covers launch,
  resume, confirmation, and smoke validation.
- [Effort Profiles](#effort-profiles) explains how `/effort` changes reasoning,
  tool rounds, continuations, and `ultracode`.
- [Dynamic Workflows](#dynamic-workflows) separates `DynamicWorkflowRuntime`
  from `/flow` OS Workflow as a Service.
- [OS, Runtime, and RemoteUI](#os-runtime-and-remoteui) shows what `/login`
  unlocks, including the login-gated `runtime` tool.
- [Core Command Reference](#core-command-reference) lists the everyday TUI
  commands that are not tied to an asset family.
- [Agents, Research, and Loops](#agents-research-and-loops) lists the detailed
  command forms for assets, DeepResearch, and engineered loops.

### A3S Code CLI Command Examples

`a3s code` is both the interactive TUI entry point and a small non-interactive
CLI for the same asset, model, knowledge, research, and OS surfaces. The CLI
forms are useful in scripts, release checks, terminals without a full-screen UI,
and docs that need reproducible examples. Commands that read or mutate OS
resources require `a3s auth login os`; local discovery, config, memory, KB, and
review prompts keep working without an OS session. Root-owned reads support
`--output json`; use JSON rather than scraping human tables.

Start, resume, and update the TUI:

```sh
a3s code                         # launch the TUI in the current workspace
a3s code resume                  # resume the newest saved TUI session here
a3s code resume 018f-session-id  # resume a specific saved session
a3s self update                  # update the a3s executable
```

Run one non-interactive coding task:

```sh
a3s code exec --mode auto "Update the focused test and verify it"
a3s code exec --mode plan --tool-policy read-only "Review this workspace"
a3s code exec --mode auto --tool-policy workspace-write "Apply the requested source edits"
a3s code exec --image before.png,after.png "Compare these screenshots"
a3s --output json code exec --mode auto --prompt-file ./task.md
```

Auto mode runs bounded workspace reads and edits without hidden prompts while
retaining the shared safety floor. Operations that still require human
approval, such as unbounded shell commands, terminate immediately in this
non-interactive surface with a nonzero `approval.required` result. Default and
plan modes never silently approve workspace mutations.

`--tool-policy read-only` and `workspace-write` are closed automation profiles.
They do not expose shell, Git, delegated tasks, runtime or package execution,
MCP, download, Knowledge, or Web tools; any unknown future tool is denied.
`workspace-write` is valid only with `--mode auto` and adds bounded native file
write/edit/patch operations while rejecting repository and agent control
metadata. Successful JSON and JSONL results echo the effective `toolPolicy` so
an automation host can verify that the boundary was retained.

Run audited L1 engineered loops on a local background cadence:

```sh
a3s code schedule list
a3s code schedule enable daily-triage --every 1d --model deepseek/deepseek-v4-flash
a3s code schedule run daily-triage
a3s code schedule status
a3s code schedule notifications
a3s code schedule disable daily-triage
a3s code schedule stop
```

`enable` validates the real loop contract and its denylist, persists
`schedule.json` beside the loop, and starts one detached worker for the
workspace. `run` queues one immediate execution through that same worker.
Claims are atomic and at most once: missed recurring intervals advance to the
next cadence rather than replaying a backlog, and a worker restart records an
uncertain active run as interrupted instead of repeating its possible effects.
The TUI automatically starts the worker when it opens a workspace with enabled
schedules; after an operating-system reboot, opening the TUI or running
`a3s code schedule start` resumes processing.

Scheduled runs use an internal, hidden `scheduled-report` profile. It allows
bounded native reads, `git status` and `git log`, and writes only the selected
loop's `STATE.md`, `RUN_LOG.md`, and descendants of `reports/`. It denies shell,
Web/network, Runtime, delegated work, package/Skill/MCP tools, unknown tools,
the active ACL configuration, credential paths, and the loop denylist. Each run
stores bounded stdout/stderr plus a structured result under the loop's `runs/`
directory. Completion notifications remain pending until the TUI displays them
or `schedule notifications` renders them successfully, then move to a durable
delivered log.

`code exec -i/--image` accepts repeated flags and comma-separated paths. PNG,
JPEG, GIF, and WebP inputs are detected from their bytes, decoded under bounded
resource limits, and retained in argument order. An image-only turn is valid.
Configured models must set `attachment = true` or include `"image"` in
`modalities.input`; unsupported account CLI transports fail before launching a
provider process instead of discarding the image.

Start the local Web API and bundled 书小安 frontend:

```sh
a3s web start
a3s web start --detach
a3s web start --detach --replace
a3s -C /path/to/project web start
a3s web start --host 127.0.0.1 --port 29653
a3s web start --api-only
```

The API is built with `a3s-boot` and reuses the same `config.acl` discovery as
the TUI. Installed packages discover Web assets beside the executable or under
the installation prefix's `share/a3s/web`; source builds fall back to the
Rsbuild output from `apps/web/dist/workspace`. Pass `--web-dir` to serve a
different frontend build. Background mode returns only after the service binds
successfully and prints its PID, URL, and workspace-keyed log path under the
platform A3S state directory.

Starting Web is idempotent. A healthy managed instance for the workspace is
reused, and a healthy foreground or older A3S instance on the requested address
is discovered without being killed only when it reports the same Plugin Manager
policy digest and offline mode. A changed or unreported policy requires
`--replace`. The policy originates from an explicit operator config or the
user-level config; an automatically discovered workspace ACL configures Code
but never authorizes plugin mutation. `status` and `open` can discover that
unmanaged default-address instance, while `stop` refuses to signal it. Use
`--replace` to restart an authenticated managed instance or a same-workspace
foreground instance whose health PID, executable, `web` command, and explicit
port all match the current invocation. A foreign or ambiguous port owner is
never terminated. Port binding and asset validation happen before
configuration-heavy session restoration, so conflicts and broken installations
fail quickly without unrelated restore warnings.

Open a workspace artifact or local development server from inside the TUI:

```text
/preview site/index.html
/preview docs/Product brief.pdf
/preview localhost:5173
/preview status
/preview stop
```

`/preview` reuses a compatible managed Web instance for the current workspace
or starts one on an ephemeral loopback port. It opens Work's persistent preview
panel in a native `a3s-webview` window when available and uses the system browser
as a fallback. Replacing a preview, `/preview stop`, and TUI exit close only the
exact native preview window; the shared Web service remains running. A browser
fallback is user-owned and cannot be closed safely by the TUI.

Inside `/ide`, press `p` on the selected workspace item. In an open editor, run
`:preview`; a dirty buffer is saved through the normal workspace path before
the preview opens. Static sites reload after debounced workspace changes, while
loopback URLs retain their development server's own navigation and HMR.

Web tasks, visible messages, titles, goals, effort, model selection, and
execution mode are saved under `~/.a3s/code-web` and restored before the API
starts accepting requests. Set `A3S_CODE_WEB_STATE_DIR` to isolate that store.
Default mode allows read-only tools and pauses mutating tools for Web HITL
approval; plan mode denies mutations, while auto mode runs without approval.

Code Web sessions auto-save Core snapshots under `~/.a3s/code-web/sessions`
and restore when `a3s web` starts again. Browser-only metadata such as
titles and a bounded recent UI transcript stays beside them under
`~/.a3s/code-web/metadata`. The projection preserves Web-only `/help`, shell,
fork, and structured-event records without adding them to model context;
neither directory is created in the selected workspace. Set
`A3S_CODE_WEB_DATA_DIR` to relocate this dedicated data root.

The browser uses the versioned Kernel session endpoints under
`/api/v1/kernel/sessions`. `POST .../{session_id}/messages/stream` returns the
core `AgentEvent` contract as Server-Sent Events; adjacent actions cancel an
active run or resolve a pending tool confirmation. A3S OS authorization remains
owned by the CLI through `/api/v1/os/login/browser`, so tokens never enter
browser storage. Code Web exposes one A3S Code agent and disables the Core
model-visible `task` delegation tool and its hidden compatibility alias. Its default permission mode allows
read-only tools and asks before writes or command execution; `auto` is the
explicit no-confirmation mode. The default listener and OAuth callback are
loopback-only.

Durable memory extraction may attach a validated, LLM-authored
`a3s.evolution.signal.v1` description for a reusable preference, Skill, or OKF
knowledge package. Code never promotes ordinary memory through keyword
matching. It aggregates matching evidence across sessions and can automatically
materialize a conflict-free local asset only after the stricter recurrence,
session, confidence, importance, and explicit-signal thresholds are all met.
Nothing is published automatically. The local `/api/v1/evolution` API lists
evidence-backed candidates, rescans the durable store, supports explicit save,
reject, and reconsider decisions, and rolls back immutable versions while
preserving a recovery copy. Rolling back to the baseline removes the active
asset without deleting its versions, so it can be restored later. Active
preferences are injected into bounded system-prompt context and active Skills
are loaded from the workspace Skill directory. TUI and Web keep an activation
barrier until every affected live session has refreshed successfully.

Code Web also exposes a VS Code-style package contribution boundary under
`/api/v1/plugins`. Activity catalogs and HTML content come only from the live,
digest-verified A3S Use registry. The Marketplace enumerates configured TUF
registries, generates install/upgrade/uninstall plans with `--dry-run`, and
applies only an explicitly reviewed `--plan-digest`. Package enablement uses
the existing Use lifecycle. Plugin HTML remains non-callable; the browser host
owns its opaque-origin iframe, restrictive CSP, bounded messages, and explicit
context review before a verified same-package Skill is added to Code.
Managed package Knowledge is exposed separately through the exact-generation
`/api/v1/knowledge/packages` catalog and
`/api/v1/knowledge/packages/search` query route; those responses never accept
a package path as authority.

The TUI `/ide` editor and the Web Monaco editor share native Code Intelligence
for saved-file symbols, definitions, declarations, references,
implementations, and diagnostics. Dirty editors remain local and explicitly
label semantic results as based on the saved version. The agent receives the
same read-only capability through `code_symbols`, `code_navigation`, and
`code_diagnostics`; existing `read`, unified `search` (grep mode), `edit`, and
`patch` tools remain the only source and mutation paths. See the
[Code Intelligence guide](docs/code-intelligence.md) for commands, keyboard
actions, language executables, and the typed local HTTP routes.

Inspect and create `config.acl`:

```sh
a3s config path                       # print the active config path
a3s config init                       # create the user config
a3s config init --scope workspace     # create .a3s/config.acl
a3s config show                       # print a redacted effective summary
a3s config validate                   # validate the effective A3S ACL
a3s config edit --scope workspace     # open VISUAL/EDITOR, or print the path
a3s config paths                      # print config, asset, memory, KB, and OKF paths
```

### Workspace semantic retrieval

Semantic and hybrid workspace search can use an asynchronous, session-bound
in-memory vector index. It does not start a vector database, write vectors to
disk, or serialize them into a session snapshot. A fresh or resumed session
rebuilds from the current admitted workspace; closing or replacing the session
cancels indexing and releases its vectors. Exact, glob, incremental BM25, and
Code Intelligence paths remain available while semantic coverage is building
or degraded.

The CLI host owns the manifest-backed chunk catalog and configures its selected
strategy exactly once before workspace services attach. TUI and Code Web may
reuse that catalog for the same workspace, but per-session options contain no
catalog override: each Code session still builds, owns, and closes its own
in-memory semantic projection.

Remote embedding sends admitted source chunks outside the machine. Enabling it
therefore requires two explicit gates in a trusted user ACL or a file selected
with `--config`:

```acl
workspace_retrieval {
  enabled = true
  allow_source_egress = true

  # This is an embedding route, independent from the default chat model.
  model = "openai/text-embedding-3-small"
  dimension = 1536
  normalization = "none"

  # Optional. Otherwise the effective provider/model baseUrl is extended with
  # /embeddings. Non-loopback endpoints must use HTTPS.
  # endpoint = "https://api.openai.com/v1/embeddings"

  provider_timeout_ms = 30000
  max_records = 100000
  max_bytes = 134217728
  shutdown_timeout_ms = 5000

  # Optional event-driven barrier for a query that races an in-flight semantic
  # generation. Zero preserves immediate lexical fallback; the hard maximum is
  # 30000. Session construction always remains asynchronous.
  semantic_readiness_timeout_ms = 0

  # Optional. Omit this typed block to preserve line chunking.
  chunking {
    recursive {
      target_bytes = 8192
      overlap_bytes = 512
      separators = ["\n\n", "\n", ". ", " "]
    }
  }

  # Optional. Omit this typed block to preserve RRF-only.
  deterministic_reranker {
    enabled = true
    max_candidates = 100
    max_feature_bytes_per_candidate = 4096
    max_fingerprints_per_candidate = 128
    max_scratch_bytes = 4194304
  }
}
```

The provider name must exist in `providers`; the embedding model ID does not
need to be listed as a chat model. Provider/model `apiKey`, `baseUrl`, and
static headers are resolved by the host. HTTP redirects are not followed,
successful response bodies are bounded, provider error bodies are discarded,
and credentials and full endpoints are excluded from debug/status output.
OpenAI-compatible `/embeddings` responses are supported by the initial host
adapter.

Local CPU embedding is a mutually exclusive trusted route. It requires an
`a3s` binary built with `local-cpu-embedding`, but it does not require
`allow_source_egress`, a remote provider, or a rerank model:

```acl
workspace_retrieval {
  enabled = true
  semantic_readiness_timeout_ms = 30000
  max_records = 100000
  max_bytes = 134217728
  shutdown_timeout_ms = 5000

  local_cpu {
    artifact_manifest = "models/multilingual-mini/model.acl"
    intra_threads = 2
  }
}
```

The manifest path is resolved relative to the trusted ACL. Model loading is
lazy and executes on a bounded blocking pool; one content-compatible model,
one native inference job, and two inputs per executor microbatch are admitted
per process. The
runtime never downloads model files. `a3s config validate` verifies the
manifest shape, containment, regular-file policy, size limits, and every
artifact SHA-256 before launch. The same checks run again on first load so a
post-validation substitution fails closed. See
[Local CPU workspace embedding](local-cpu-workspace-embedding.md) for the
versioned manifest contract, platform matrix, and operational limits.

`chunking` is a mutually exclusive typed block. It contains exactly one
`line {}`, `fixed_window { ... }`, or `recursive { ... }` child and does not
accept a primitive `strategy` field. Omission and an explicit `line {}` both
select compatibility line chunking. Fixed and recursive blocks require
`target_bytes`; `overlap_bytes` defaults to zero. Targets are limited to
4–65,536 UTF-8 bytes and overlap must be smaller than the target. A recursive
`separators` list is optional; when present it must contain 1–16 unique,
non-empty strings of at most 64 UTF-8 bytes without NUL. Omitting the list uses
the pinned Core defaults, while an empty list is invalid. Unsupported/custom
strategy blocks, mixed typed children, unknown fields, and workspace-layer
overrides fail before provider resolution or source egress. Arbitrary custom
splitters remain available only to trusted Rust hosts. Only admitted text files
are split and embedded; non-text files stay outside this retrieval pipeline and
belong to the separate knowledge-compilation path.

`deterministic_reranker` is a typed, default-off host option. It applies the
bounded `rrf_k60+deterministic_mmr_v1` second stage without a neural runtime or
additional remote call. Every limit is checked against the pinned A3S Code
contract before the provider route is resolved or the workspace catalog starts.
The block requires an explicit `enabled` boolean so a later trusted layer can
return to RRF-only with `enabled = false`. Raw `mode` and `algorithm` fields,
unknown fields, duplicate blocks, and out-of-range limits fail validation. An
automatically discovered workspace ACL cannot add or alter this block.

An automatically discovered workspace `.a3s/config.acl` is untrusted for
source egress. Its only accepted retrieval block is:

```acl
workspace_retrieval { enabled = false }
```

Provider credentials, base URLs, and headers used by retrieval are also
resolved only from the trusted user/explicit layer. A workspace provider block
may still configure the agent's ordinary model route, but it cannot indirectly
reroute an inherited embedding grant.

Use `a3s config validate` before launch. TUI `/status` distinguishes disabled,
building, partial, ready, degraded, and closed state and reports coverage,
indexed files/chunks, queue depth, failure count, and embedding model identity.
`a3s config show` additionally reports whether the local CPU route is
available, a null or stable non-sensitive unavailable reason, the effective
embedding batch-input limit, the selected backend, the semantic-readiness
timeout, the effective chunking strategy,
target/overlap, explicit separators or Core-default state, requested rerank
mode, versioned algorithm, active state, and non-sensitive resource limits.
Code Web session status returns the structured `workspaceRetrieval` snapshot;
JSON/JSONL `a3s code exec` results include the same field. These projections
never contain source text, vectors, credentials, or endpoints.

The embedding route is deliberately independent from `default_model`. For
example, `.a3s/config.acl` may use DeepSeek for the agent's chat/tool loop while
a local deterministic or separately admitted provider supplies embeddings.
The host never sends source code to a DeepSeek chat endpoint merely because it
is the active model. Architecture, ownership, delivery gates, and performance
budgets are maintained in the
[A3S Code Workspace Retrieval roadmap](https://github.com/A3S-Lab/Code/blob/main/ROADMAP.md#6-workspace-retrieval-program).
The separate
[ACL-host evaluation](workspace-retrieval-evaluation.md) documents the
first-principles adversarial plan, real DeepSeek reproduction command, quality
metrics, resource accounting, cross-file batching result, and subsequent
cross-SDK real-model qualification. It also records the production host run in
which a revision-locked multilingual Sentence Transformers model crosses the
same OpenAI-compatible HTTP adapter, reaches the in-memory index, and supports
the real DeepSeek completion loop.

Sign in to A3S OS and check account state:

```sh
a3s auth login os              # open the configured OS OAuth login flow
printf '%s' "$A3S_OS_TOKEN" | a3s auth login os --token-stdin
a3s auth status os             # show OS endpoint, account, and expiry
a3s auth logout os             # remove the stored OS session
```

Inspect managed and externally owned account sources without copying OAuth
credentials into `config.acl`:

```sh
a3s auth list
```

Claude Code, Codex, Kimi, and WorkBuddy continue to own their login flows and
local credential stores. The A3S CLI only reports whether those accounts are
available; `a3s auth login/logout` manages the configured A3S OS session.

Model routes use one catalog shared by the root CLI and the Code TUI. Custom
provider models come from `config.acl`; Claude Code, Codex, Kimi, WorkBuddy,
and A3S OS models remain bound to their product-owned credentials:

```sh
a3s model list
a3s model current
a3s model use openai/my-model
a3s model use claude-code/claude-opus-4-6
a3s model use codex/gpt-5.2-codex
a3s model use kimi/k3-agent
a3s model use workbuddy/glm-5.1
a3s model use a3s-os/my-model
a3s model use openai/gpt-5 --scope workspace
a3s model reset --scope user
```

`a3s model use` validates the route and updates `default_model` in the selected
A3S ACL layer while preserving unrelated ACL text. `--config` always targets
that explicit file; otherwise `--scope user|workspace` selects the layer. No
Claude, Codex, Kimi, WorkBuddy, or A3S OS token is copied. Selecting a route
probes only that credential source; `model list` refreshes independent account
catalogs concurrently.

List runtime-callable models:

```sh
a3s model list
a3s model current
```

The model commands list `config.acl` models, local Claude/Codex/Kimi/WorkBuddy
account models, and signed-in OS gateway models from the unified gateway.
Codex uses its current account catalog with its cache as an offline fallback;
Kimi discovers models from Kimi Desktop or Kimi Code account state, and
WorkBuddy discovery uses its installed CodeBuddy CLI. These runtime routes are
not digital asset repository entries whose category happens to be `model`.

Find local asset sources, clone repositories, and inspect OS assets:

```sh
a3s code agent list --location local
a3s code agent list --location local reviewer
a3s code agent clone https://github.com/acme/reviewer-agent.git
a3s code agent list --location os reviewer
a3s code agent activity failed

a3s code mcp list --location local weather
a3s code mcp clone https://github.com/acme/weather-mcp.git
a3s code mcp list --location os weather
a3s code mcp activity running

a3s code skill list --location local summarize
a3s code flow list --location local release
a3s code okf list --location local security

# One versioned JSON document; relative ACL asset roots resolve from -C.
a3s -C /path/to/project --output json code agent list --location local
```

`list --location local`, `clone`, and `review` are local developer operations.
`list --location os`, `activity`, and every publish/deploy/open/log/status
operation call OS APIs and therefore need a configured `os = "https://..."`
plus a valid login. `list --location all` returns local and OS results together
and fails explicitly when the OS side is unavailable.

Clone URLs must not embed credentials, query strings, or fragments. Use an SSH
remote or the platform Git credential manager for private repositories so
tokens never enter argv or machine output.

Run agent lifecycle commands:

```sh
a3s code agent review agents/reviewer
a3s code agent publish agents/reviewer --kind agentic
a3s code agent publish agents/portal --kind application
a3s code agent publish agents/sql-checker --kind tool
a3s code agent run agents/reviewer
a3s code agent deploy agents/portal
a3s code agent open agents/reviewer --kind agentic
a3s code agent logs agents/sql-checker --kind tool
a3s code agent status agents/portal --kind application
```

An Agent asset is a package directory. `agent.md`, `agent.yaml`, or
`agent.yml` is only the package entrypoint; passing the entry file still works
for compatibility, but publish/deploy uploads the whole package.

Run MCP lifecycle commands:

```sh
a3s code mcp review mcps/weather
a3s code mcp publish mcps/weather
a3s code mcp run mcps/weather
a3s code mcp test mcps/weather
a3s code mcp deploy mcps/weather
a3s code mcp open mcps/weather
a3s code mcp logs mcps/weather
a3s code mcp status mcps/weather
```

Run skill, workflow, and OKF lifecycle commands:

```sh
a3s code skill review skills/summarize/SKILL.md
a3s code skill publish skills/summarize/SKILL.md
a3s code skill deploy skills/summarize/SKILL.md
a3s code skill open skills/summarize/SKILL.md
a3s code skill status skills/summarize/SKILL.md

a3s code flow review flows/release-gate.json
a3s code flow publish flows/release-gate.json
a3s code flow run flows/release-gate.json
a3s code flow deploy flows/release-gate.json
a3s code flow open flows/release-gate.json
a3s code flow logs flows/release-gate.json
a3s code flow status flows/release-gate.json

a3s code okf review okf/security-playbook
a3s code okf publish okf/security-playbook
a3s code okf deploy okf/security-playbook
a3s code okf status okf/security-playbook
```

To run the gated real OS lifecycle smoke test, sign in first, then opt in
explicitly:

```sh
A3S_REAL_OS_LIFECYCLE=1 cargo test --test real_os_lifecycle -- --ignored --nocapture
```

The smoke test creates short-lived OS assets for agent, MCP, skill, workflow,
and OKF families, exercises their lifecycle commands, deletes the remote test
assets through OS, and verifies the timestamped test query returns `0 asset(s)`.

Manage local knowledge, context history, and memory:

```sh
a3s code kb stats
a3s code kb add "Release notes should mention the gateway model split."
a3s code kb import docs/
a3s code kb search "gateway model split"
a3s code kb vault

a3s code ctx search "RemoteUI view link"
a3s code ctx show 01HVEXAMPLEEVENT --window 8
a3s code ctx session 018f-session-id

a3s code memory list
a3s code memory list "database migration"
a3s code memory stats
a3s code memory dir
a3s code mem list "preference" # alias for memory
```

`/ctx <n>` attachment and `/ctx save <n>` memory promotion are interactive TUI
state, so the CLI exposes the durable `search`, `show`, and `session` forms
instead of pretending to attach context to a running transcript. Interactive
search never performs an implicit index refresh. The local `ctx` child runs
with null stdin, a dedicated process group, a 15-second deadline, and a 2 MiB
combined-output ceiling; parsed identifiers, titles, providers, and snippets
are sanitized and length-bounded before they reach the transcript. A selected
window is quote-prefixed as untrusted historical material and limited to 6,000
UTF-8 bytes before one-shot prompt attachment.

Inspect local process activity:

```sh
a3s code top
a3s code top --json
```

Run bounded DeepResearch without opening the TUI:

```sh
a3s code research --web "compare Tokio and async-std"
a3s code research --local-only "summarize the repository architecture"
```

`--web` and `--local-only` set the evidence scope explicitly and override
phrasing inside the query. The global `--offline` option also forces
local-only evidence; combining it with `--web` is rejected. DeepResearch has
one host-managed runtime and exposes no runtime-selection route.

Headless CLI, TUI, and Code Web all use `CodeDeepResearchRunner` with the typed
`a3s-deep-research` request, result, event, and cancellation contracts. The
runner creates an isolated read-only `AgentSession`; local-only mode does not
expose Web tools. Every exit path settles or aborts the root task and closes
the isolated session.

Each run writes authoritative artifacts to
`.a3s/research/artifacts/<run-id>/report.md` and `index.html`, and records its
bounded typed projection at
`.a3s/research/runs/<run-id>/journal-v2.jsonl`. Lifecycle is independent from
publication. A completed run reports exactly one of `synthesized`,
`qualified`, `source_backed`, or `no_evidence`.

Code Web converts context attachments into validated relative workspace source
hints; missing, empty, or symbolic-link inputs fail before launch. Selected
Skills are rejected explicitly because the isolated runner currently uses an
empty Skill registry. Web refresh reads lifecycle, stage, publication, and
quality from the v2 journal. Report links use only a validated `runId` and
`html` or `markdown` kind, and neither persisted events nor SSE metadata
contains an absolute artifact path.

RemoteUI and local research reports open from the inline `Open view` action in
the TUI. There is no separate `a3s code view` command.

Inside the TUI, the same surfaces are available through slash commands and
input prefixes:

```text
/help
/status
/model
/effort
/config
/terminal
/checkup
/queue
/history
/tasks
/permissions
/ide
/preview site/index.html
/login
/agent
/mcp
/skill
/flow
/okf
/relay
/loop init release-gate ci-sweeper
/loop run release-gate
? research how the OS gateway discovers runtime models
! cargo test --all-targets
@src/main.rs
```

### Capability Overview

| Area | What A3S Code TUI provides |
| --- | --- |
| Coding loop | Chat with the coding agent, stream semantic tool cards, choose Default, Plan, or Auto execution, inspect the current session and token budget with `/status`, control pending follow-ups with `/queue`, and inspect or safely cancel delegated work with `/tasks` or `Ctrl+B`. Run direct shell turns with `!`, run a durable Ultracode `/goal`, and fork, rewind, or clear sessions when needed. `/relay` pins the current session, searches a bounded 64-row catalog per source, preserves semantic selection across refreshes, shows saved state, model, age, unfinished runs, and live background-agent counts, or hands the latest task from a workspace-scoped external transcript to the active session. |
| Permission review | Gated calls enter a FIFO approval queue backed by the authoritative tool name and arguments. The overlay can allow once, grant that exact capability for the current session, atomically add the reviewed capability to `.a3s/permissions.acl`, or collect denial feedback for the agent. `/permissions` shows and cycles the next-turn Default/Plan/Auto mode with `M`, searches session and project grants, opens their canonical arguments, and revokes only after a second matching action. Changing the composer mode does not rewrite the active or already queued turn. Project rules are bounded, parsed and generated with `a3s-acl`, reject symbolic-link targets, and remain narrower than hard workspace guardrails. Revocation affects future checks, not tools already running. |
| Execution modes | Default runs bounded workspace file changes directly. A shared Rust guardrail silently admits a narrow, proven read-only host Bash subset; unproven commands, protected metadata, mutating Git operations, and annotated external side effects enter HITL, while critical commands fail closed. Plan exposes only read-only discovery tools and denies Bash. Auto never enters HITL: it admits only Rust-proven read-only Bash and denies other host commands, protected metadata, and mutating Git operations. A queued turn retains the mode captured when it was submitted. |
| Workspace UI | `/ide` opens a superfile-style tree and editor with terminal-stable file marks, `p` previews the selected item, and `:preview` saves and previews the open buffer. `/preview <path\|localhost-url>` opens the same persistent Work surface from the transcript. `/config` edits the active config in the shared editor, `Ctrl+T` opens the complete semantic transcript, and file edits render bounded diffs through the shared `DiffView` component. |
| Code Intelligence | One native, read-only runtime serves the agent, TUI `/ide`, and Web Monaco surfaces. Rust and TypeScript/JavaScript language servers provide saved-file outlines, workspace symbols, definitions, declarations, references, implementations, and diagnostics through `code_symbols`, `code_navigation`, and `code_diagnostics`. Queries are cancellable, time-bounded, workspace-confined, UTF-16-positioned, terminal-safe, and explicitly report stale saved-version evidence without replacing `read`, unified `search`, or mutation tools. |
| Models and effort | `/model` switches configured providers, OS gateway models, and signed-in account tabs. Codex account discovery delegates refresh and entitlement checks to the installed Codex CLI, so an expired identity token does not hide models while reusable account access remains. WorkBuddy `hy3` tagged calls are converted into native tool events without exposing protocol markup in streamed messages. `/effort` scales thinking budget, tool-round budget, auto-continuation, and model-agnostic rigor guidance from `low` through `max` and `ultracode`. A3S Code Core 6.7.0 structured calls use native JSON Schema or forced-tool output only when every active candidate advertises that capability; unknown custom OpenAI-compatible endpoints retain the bounded prompt fallback instead of receiving an assumed `tool_choice`. |
| Dynamic workflows | `ultracode` and `?` DeepResearch can use `DynamicWorkflowRuntime`, a local A3S Flow-backed workflow runner. It records workflow/step history while PTC scripts perform ordinary tool work, binds recovery to the exact run, query, and completed step, and permits 1-4 independently session-bound `generate_object` calls when the provider can fork sessions. DeepResearch 0.1.3's four-slot limit is validated and forwarded unchanged to Core 6.7, and the terminal card shows the active slot bound. This is separate from `/flow`, which is OS Workflow as a Service for persisted workflow assets. |
| Local and remote parallelism | Local subagent fan-out uses one `task` call with multiple independent `tasks[]` items. QuickJS/PTC may call one item directly but cannot fan out; dynamic workflows schedule a host Flow step named `task`. After `/login`, the approval-gated `runtime` tool can submit at most 64 independent tasks to an OS tool-worker UUID or resolved name, stream bounded progress, honor cancellation and a maximum 30-minute absolute poll deadline, and return completed members when the batch times out. Requests, responses, IDs, event text, and per-member results are bounded before entering the TUI or model context. |
| Deep research | Prefix a prompt with `?` to run the shared evidence-first Host path. Exact-query bootstrap and one bounded semantic outline run concurrently. The planner decomposes at most 24 atomic user requirements, maps all of them to at most eight material tracks, and may add at most 15 plain-text queries. Up to two later gap-directed rounds expand missing atomic criteria and share Host-owned totals of at most 24 new queries and 16 supplemental fetches. Core 6.7 searches headless engines first, continues through HTTP/RSS and native APIs only while structural retrieval requirements remain unmet, and retains typed engine/fallback evidence without an external semantic verifier. TUI search cards show the tier path, result count, retrieval decision, engine success ratio, and output limiting without treating provider metadata as evidence. The Host stages a source-backed artifact, admits one typed claim graph, and runs an independent commercial review over every mapped requirement and claim before `synthesized` can count as success. `qualified`, `source_backed`, and `no_evidence` remain accessible previews but return incomplete/failure semantics. Markdown and editable single-HTML output use the user's language and the fixed A3S Web design. |
| Context and memory | The bottom status bar is the single context-fill indicator. Auto-compaction uses the active model's real window, runs before an overflowing request, and re-arms after every cycle. `/history` or `Ctrl+R` searches prompts in the current session; local `/ctx` retrieval searches indexed A3S Code, Claude Code, Codex, and Cursor sessions, shows an exact hit window, stages one sanitized 6,000-byte quoted block for the next turn, or promotes a hit into durable memory with event/session provenance. CTX subprocesses have hard deadlines, isolated process groups, and combined-output limits. `/sleep` consolidates the day, and `/memory` browses the resulting event/entity graph. |
| Knowledge | `/kb` manages a local personal knowledge vault for notes, imports, search, browsing, and shared-confirm deletion. `/okf` manages shareable OKF knowledge-package assets under the visible `okf/` package root and publishes them to the OS Knowledge service when signed in. |
| Asset development | `/agent`, `/mcp`, `/skill`, and `/okf` enter local development modes with an active asset, review commands, clone/draft flows, and publish/deploy/status surfaces. `/flow` works differently: it selects or drafts workflow DAG assets and sends them to OS Workflow as a Service, without entering a persistent local dev mode. |
| Runtime activity | Asset-specific `activity` commands (`/agent activity`, `/mcp activity`, `/flow activity`, `/skill activity`, `/okf activity`) inspect OS Runtime jobs/runs for the selected asset. Use the standalone `a3s top` command for local process activity. |
| Engineered loops | `/loop init`, `/loop run`, `/loop audit`, and `/loop logs` manage durable loops under `.a3s/loops`. Loops use maker/checker separation, reports, budgets, state files, and OS Runtime/RemoteUI evidence when enabled; inside `/agent` mode they stay local and target the active agent package. |
| OS and RemoteUI | `/login` enables OS capabilities. A bounded progressive search → describe → execute path discovers supported OS operations without baking every service API into the client; it evaluates at most four candidates and limits request/response bytes, traversal depth, node count, identifiers, and schema fields. Shaped responses (`.view` or `viewUrl`) surface an inline `Open view` action, using the native `a3s-webview` helper when available and browser fallback otherwise. |
| Operations | `/help` shows the full command guide, `/terminal` reports negotiated terminal capabilities and fallbacks, and `/checkup` audits installation/configuration/context health in enforced read-only Plan mode before presenting proposed fixes at an Approve / Revise / Abandon boundary. The live activity row distinguishes context resolution, planning, exploration, and external-task waits; queue recovery, persistence failures, budget thresholds, and other operational events remain in the semantic transcript. Typed tool failures retain actionable version-conflict, argument, backend, timeout, transport, cancellation, partial-result, and rate-limit semantics. `/theme` cycles syntax themes, `/plugin` and `/reload` manage skills/plugins, `/update` upgrades and restarts, `/compact` summarizes context, `/fork worktree` creates an isolated workspace branch, and `/rewind` safely undoes the last completed turn. |

The reusable DeepResearch control flow lives in the independent
`a3s-deep-research` crate. The CLI implements its structured-generation,
workflow-execution, publication, and progress ports in
`CodeDeepResearchRuntime`; `CodeDeepResearchRunner` delegates one complete
typed run to `DeepResearchEngine::execute_request`. Search/fetch tools, Flow
durability, filesystem publication, CLI/TUI events, Web SSE, and cancellation
settlement remain product adapters.
Planning contracts, bounded fallback semantics, evidence admission, quality
gates, and report rendering do not have a second CLI implementation.

Every new DeepResearch run starts acquisition from the exact user query while
one bounded semantic planner runs concurrently. Bootstrap search never waits
for the planner. The planner returns only a structured research outline:
`focused` or `comprehensive` scope, freshness and workspace requirements, one
to 24 atomic request requirements, one to eight material evidence tracks with
observable completion criteria and exact requirement mappings, and zero to 15
supplemental search queries. It cannot return URLs, sources, facts, answers, or
transport budgets. Missing, duplicate, unknown, or unmapped requirements make
the outline invalid.

The Host keeps the exact query as the first query, rejects blank, duplicate, or
URL-shaped supplements, and caps the planner-owned query set at 16. For
web-capable scopes, it separately promotes at most three explicit HTTP(S)
references from the exact user query to direct fetch seeds, strips fragments,
rejects credential-bearing references, and deduplicates normalized URLs.
Local-only scope emits no URL seeds and remains network-free. Planned
retrieval reuses the durable bootstrap packet. When bootstrap already retained
web evidence, it does not search the exact query again; it searches only the
validated supplements and merges both packets before source and chunk
selection. If planning fails or returns an invalid contract, the fallback is
the exact query and one generic evidence track, with no inferred topic and no
query expansion. Unknown breadth and temporal intent fail toward the stronger
contract: the fallback is always `comprehensive` and
`freshness_required = true`, so an undated synthesized answer is never
authorized by planner failure. Up to two later gap-directed rounds may generate
queries only from typed missing criteria, missing source roles, or failed
retrieval effects. The Host expands each unresolved track into one ordered
target per missing atomic criterion and rotates targets across tracks. The
eight-track, three-criteria contract therefore derives a maximum of 24 new
queries, while both rounds share at most 16 supplemental fetches and subtract
prior actual consumption before scheduling more work. No product domain, named
entity, keyword family, publisher, domain, or language-specific topic template
changes this control flow.

DeepResearch model execution is also content-blind. The selected model remains
first, followed by at most three tool-capable, structured-output-compatible
candidates from configured and signed-in registries, ordered by model source
and endpoint failure-domain diversity. Failover reacts only to request failure,
cancellation, or the caller-derived generation deadline. A streaming candidate
is remembered only after its terminal `Done` event; partial events from a failed
candidate are buffered and discarded before the next candidate runs.

Discovery candidates first pass one bounded semantic selection over closed
candidate IDs. If that selection fails, the retrieval workflow may fetch a
bounded fallback set so transport failure does not erase potentially useful
material, but fallback web text remains audit-only. Every fetched web source,
including fallback material, must then pass the same closed semantic evidence
selector before it can support a report. The selector returns exact chunk IDs,
obligation-relevance edges, completion-criterion coverage, and typed
`supporting`, `primary`, and `independent` roles. Partial evidence can remain
visible and support an atomic claim through an exact relevance edge without
falsely closing a criterion. Only the separate completion-criterion edges and
their criterion-scoped roles can close a track. Small catalogs use one
selector; larger catalogs use complete source-local JSON windows capped at
32 KiB and a second exact-ID reduction capped at four excerpts per source.
Window boundaries are byte budgets and never inspect punctuation, language,
topic, or URL text.

Search rank, query-token overlap, language or script detection, publisher
names, hostnames, TLDs, URL path vocabulary, workspace path shape, and
maintained site lists never promote a source into claim evidence.
Search-provider metadata only identifies retrieval opportunities and remains
visible as audit metadata. The Host-projected inquiry collection is the sole
semantic admission authority for both web and workspace sources. The Host
validates exact source/chunk IDs, criterion indexes, role shapes, and
provenance before publication. The two bounded gap rounds may retrieve missing
evidence without opening an unbounded search loop or duplicating transport
budgets inside the workflow.

The report catalog preserves structurally valid source text without classifying
its vocabulary. Sanitization removes script/style/noscript element blocks,
HTML markup, control characters, and inline transport targets under fixed
bounds; visible text remains available to the closed semantic review even when
it resembles application state or JavaScript. A failed or malformed source
cannot erase valid siblings, and raw web or workspace acquisition that lacks
inquiry-projection provenance remains audit-only.

Once at least one source survives, the Host atomically stages `report.md` and
`index.html` before report synthesis becomes terminal. That source-backed
artifact is published independently as `source_backed`: it reports zero
synthesized claims and makes the evidence limitation explicit instead of
presenting source excerpts as a finished answer. With no semantically admitted
source, the same Host path publishes a versioned `no_evidence` boundary
artifact.

The optional report proposal is a closed typed claim graph over bounded
excerpts and the validated semantic dimensions. A transient generation failure
may retry once; there is no section fan-out, topic-specific compiler, or hidden
continuation. The Host builds the schema
for that exact attempt: dimension, source, and chunk enums contain only IDs
from the closed packet, while audit-only sources are absent. Claims are typed
as facts, inferences, or recommendations. Inferences and recommendations name
admitted basis claims; derived claims keep their method and exact input IDs;
contradictions name two admitted claims in one dimension. The compiler rejects
malformed items while keeping valid siblings and derives citations, coverage,
the source ledger, and both renderings from the admitted graph. It never
compares query, claim, or source prose and never promotes evidence by publisher,
domain, path, language, token, or error wording.

After graph admission, a separate commercial editorial generation receives the
exact query, current date, mapped requirements, admitted claims, and only the
closed excerpts cited by those claims. It must review every dimension and claim
for requirement coverage, support, analytical depth, boundary quality, scope,
attribution, modality, language, and temporal status. It also returns
evidence-preserving claim rewrites and paragraph order. The Host rejects an
omitted or duplicate review, inconsistent readiness, an invalid temporal class,
numeric drift, claim-identity changes, dependency inversions, or prose that
still reads as a source-summary inventory. Editorial failure can preserve a
qualified preview, but it can never let a synthesized draft remain successful.

Focused publication requires one cited direct-answer claim, at least one
admitted claim, and one cited eligible source. Comprehensive publication
additionally requires five findings, six admitted claims, two cited sources,
and 1,200 substantive characters. Every resolved material dimension also needs
two factual findings across two sources, one cross-source comparison, one
explanation, one supported implication, one challenge or boundary, one
cross-source synthesis, and 1,200 substantive characters. Complete material
coverage publishes `synthesized`; deeply analyzed resolved dimensions plus
explicit material gaps may publish `qualified` only as an incomplete preview.
If every dimension remains bounded, at most one qualified partial preview may
remain when it passes the same two-source analytical chain and keeps a typed
gap. Only `synthesized` is a successful completed research result; all other
publication outcomes return non-success semantics while preserving their
run-scoped artifacts. Audit-only sources do not poison an otherwise valid
proposal, but they cannot strengthen a conclusion. Reader-facing labels,
claims, editorial rewrites, and evidence-boundary prose are pinned to the
user's output language. Website metrics are derived from admitted typed claims
and citations, never from source headings or provider metadata. Markdown and
HTML use matching versioned artifact markers rather than title-word
classification. The admitted editorial plan groups exact claim IDs under
natural headings without changing evidence; the renderer produces continuous
prose and keeps basis edges in a collapsed traceability disclosure. The
standalone HTML uses the A3S Web design tokens, a sticky left action menu, and a
sticky right table of contents, with edit, save, print, and responsive mobile
controls built into the fixed Host script.

`a3s code research` (including its aliases) and the TUI `?` path call this same
evidence-first runtime; there is no second CLI implementation. Durable search,
fetch, generation, and publication identities are reused after restart, and
bootstrap workflow metadata cannot replace the Host-owned terminal result. A
report view opens only after the scoped run has settled and the Markdown/HTML
pair passes path, content, and quality validation.

### Everyday Capability Paths

A3S Code TUI is designed around work paths rather than isolated commands. Most
turns start as a normal chat prompt, then the TUI decides which context,
permissions, tools, panels, and follow-up evidence are needed.

| Work path | Typical flow | Useful surfaces |
| --- | --- | --- |
| Repository orientation | Start with `/init`, ask for a map of the codebase, attach files with `@`, and open `/ide` when you need to browse or edit directly. | `/init`, `/ide`, `@<path>`, `/ctx`, `/help` |
| Focused coding | Ask for a change, review streamed reads/searches/diffs, approve gated writes, and let the agent run focused checks before summarizing what changed. | Tool cards, approval overlay, `DiffView`, `Ctrl+T`, `! <command>` |
| Artifact inspection | Keep a static site, local development server, document, image, PDF, or source file visible in Work while coding continues; use the IDE shortcuts when already browsing or editing. | `/preview <target>`, `/preview status`, `/preview stop`, IDE `p`, editor `:preview` |
| Debugging and verification | Let the model inspect logs, search call sites, run shell or test commands, and keep the exact tool evidence visible in the semantic transcript. | `search`, `read`, `bash`, `git`, `Ctrl+T`, `a3s top` |
| Context carry-over | Search previous sessions, attach relevant transcript windows, save durable facts, and compact when the context meter gets high. | `/ctx <query>`, `/ctx <n>`, `/ctx save <n>`, `/memory`, `/sleep`, `/compact` |
| Deep work | Raise `/effort`, use `ultracode` for complex turns, and let the host decide whether planning, goal tracking, dynamic workflow execution, or parallel fan-out is justified. | `/effort`, `/goal`, `dynamic_workflow`, `task` |
| Research | Prefix with `?` so the Host acquires relevant sources first, stages a durable evidence view, and publishes a cited report only after deterministic quality admission. | `? <question>`, `web_search`, `web_fetch`, `batch`, `generate_object`, `DynamicWorkflowRuntime` |
| Local asset development | Enter an asset mode, iterate on the selected local definition, review it, then publish or deploy only when the OS side is available and appropriate. | `/agent`, `/mcp`, `/skill`, `/okf`, `/flow`, `/loop` |
| Operations and recovery | Resume saved sessions, inspect local or OS activity, hot-reload plugins, and update the CLI without losing the session. | `a3s code resume`, `Open view`, `a3s top`, asset `activity`, `/plugin`, `/reload`, `/update` |

The key boundary is that local automation stays useful without an OS account,
while OS-backed actions become available only after `/login`. Local commands can
draft assets, run tools, build memory, use MCP, delegate to child agents, and
execute dynamic workflows. Signed-in commands add OS assets, Runtime batches,
RemoteUI ViewLinks, service activity, and publishing or deployment.

The TUI keeps these paths observable. A long turn can show a plan row,
reasoning deltas, live tool status, approval prompts, subagent progress,
dynamic-workflow artifacts, memory events, RemoteUI actions, and final
verification evidence in the same transcript instead of scattering state across
separate logs. High-frequency Core phases stay on one transient activity row,
while queue recovery, persistence, budget, passivation, and external-task
failures become bounded semantic notices. Tool cards preserve Core's typed
failure discriminant and add retry guidance only when that structured evidence
exists; arbitrary error prose is never classified heuristically.

### Inside The TUI

The main screen is an event-driven transcript. User messages, model text,
reasoning deltas, tool starts, streamed tool output, approvals, subagent
progress, plans, memory events, and final summaries arrive as structured
`AgentEvent` values from `a3s-code-core` and are rendered incrementally through
`a3s-tui`.

The event reducer maintains a separate `CoreRunStatus` projection for selected
agent mode, context resolution, planning, and pending external tasks. These
states replace the generic working label without creating transcript noise.
Operational events that require attention are sanitized, length-bounded, and
stored as semantic notices, so resizing and transcript export preserve them.
Terminal tool events retain `ToolErrorKind`; the visible card distinguishes a
safe retry from a version refresh, argument correction, unsupported backend,
partial result, cancellation, or provider reset without parsing human-readable
output.

Every untrusted terminal-facing value crosses one shared layout-safe boundary
before component styling. It removes complete ESC/CSI/OSC/DCS/SOS/PM/APC and
C1 control strings, C0/C1 controls, and bidirectional formatting controls while
preserving ordinary spaces, line breaks, Unicode text, and four-column tab
stops. Notice, tool, and transcript render sources have explicit character
budgets, and semantic message sources are sanitized before they are retained.
The live assistant Markdown source stops at 4 MiB per segment, whole-turn
assistant capture stops at 4 MiB, and private reasoning stops at 1 MiB; each
uses a UTF-8-safe terminal marker and ignores later deltas. Before a terminal
tool result arrives, streamed arguments and output are also limited to 1 MiB
per transcript/runtime projection. Authoritative tool arguments and metadata
are structurally projected below a 1 MiB serialized-JSON ceiling before
retention, preserving common semantic fields and explicit truncation metadata;
HITL continues to authorize against the exact unprojected arguments. The pinned
plan retains at most 256 presentation-only task records, bounds each visible
step, validates the complete `update_plan` payload, and reports every omitted
row without changing semantic task IDs used for lifecycle matching. Queue and
delegated-task labels pass through the same boundary.

Tool calls occupy a stable transcript position from preparation through
approval, execution, and completion, so interleaved calls cannot swap order.
After a terminal model event, the TUI keeps new input queue-only until the
stream worker finishes persistence and releases the session's single-flight
lease; synthesis, loop, DeepResearch, and queued continuations cannot overlap
the previous operation.
When Core restarts an interrupted response stream, it reuses the same LLM turn
and message snapshot. A repeated `TurnStart` for that turn restores the TUI's
pre-attempt transcript, reasoning, Markdown, and tool projection before the
replacement stream arrives, so retries never append another user message or
leave duplicated partial output.
Pending host turns are scheduled by `a3s_lane::PriorityQueue`: explicit user
input has priority over host-generated continuations, while equal-priority
messages remain FIFO. A queue item is committed only after Core admits its
stream; `SessionBusy` and admission timeouts restore the same priority and FIFO
position. Enter appends a follow-up to that immutable FIFO queue. `Ctrl+O`
performs Send now: it cancels and settles the active turn, then promotes the
new prompt ahead of normal follow-ups without cancelling a durable goal. Each
queued row retains its submission-time execution mode even if the composer mode
changes later. The bottom queue strip contains pending turns only and removes a
message as soon as Lane claims it for execution. `/queue` opens the same
authoritative pending queue: move with Up/Down or the wheel, press Enter or `S`
to send the exact selected row now, press Delete or `D` to remove it, and press
`C` followed by an explicit confirmation to clear all pending rows. These
operations retain every untouched Lane priority/FIFO sequence; Send now also
retains the selected turn's attachments, Plan draft, and submission-time mode.
While a turn is actively running, Enter on an empty composer sends the current
queue head now.
The transcript uses Codex-style `•` headers with `└` detail and `│` command
continuations, groups adjacent reads/lists/searches into one Explore cell, and
reflows semantic arguments, output, diffs, and Markdown after a resize. User,
reasoning, and assistant message surfaces each own a blank row above and below
their content; streaming and finalized cells keep the same vertical rhythm
while adjacent tool activity remains compact. Streamed Markdown commits only
complete lines, paces stable rows with adaptive catch-up, keeps active tables in
a replaceable tail, and provisionally completes a candidate table before
painting it so raw pipe rows never flash or move the scrollbar. Tables use
compact rounded cards with a soft header surface and a stacked narrow-screen
fallback while preserving code, URLs, Unicode graphemes, headings, and every
cell value. Tail-only updates reuse the already-wrapped transcript prefix
instead of rebuilding the full viewport.

The transcript compositor owns vertical separation: every top-level semantic
cell boundary receives exactly one neutral blank row in both the main history
and the `Ctrl+T` view. Messages, notices, reasoning, tool calls, delegated-task
results, and the live assistant tail therefore follow the same rule without
cells adding or doubling their own outer padding.

| Surface | What you see and control |
| --- | --- |
| Transcript | Assistant text, reasoning, tool cards, diff summaries, task updates, memory recall/store notices, compaction notices, and RemoteUI action links stay in one scrollable history. Drag-select copies the complete semantic range on release; committed-history selection survives streaming refresh and terminal resize, and edge dragging auto-scrolls. |
| Input line | Type a normal prompt, use `Shift+Enter` for multiline input, prefix `!` for a direct shell turn, prefix `?` for DeepResearch, use `@<path>` to reference a workspace file (image files become validated visual attachments), mention an enabled Skill as `$<skill>`, or paste an image with `Ctrl+V`. |
| Command and Skill menus | Press `/` or type a slash command to open the wheel-browsable built-in command palette backed by the same registry and Workflow, Session, Context, Asset, and System groups used by `/help`. Command search ranks exact names, prefixes, descriptions, concepts, and bounded typos. Submitting an unknown slash command rejects it locally with suggestions; it never becomes an agent prompt. Type `$` at the start of an active prompt token to search and insert enabled Skills; non-built-in Skills no longer occupy slash-command names. |
| Pending queue | `/queue` shows each pending follow-up with its immutable execution mode. Keyboard or wheel selection can Send now, remove one row, or enter an explicit clear confirmation without changing the composer draft. |
| Prompt history | `/history` or `Ctrl+R` opens a fuzzy-searchable, newest-first catalog of the current session's prompts. Enter or Tab restores the selected prompt, while Esc closes without changing the current draft. |
| Delegated tasks | `/tasks` or `Ctrl+B` opens the authoritative task catalog while the parent turn keeps streaming. Search, inspect recent progress or full output, refresh, and cancel a running task with a second matching `X` or Delete press. |
| Permission grants | `/permissions` opens while a parent turn streams. `M` cycles the next-turn Default/Plan/Auto mode without changing the active or queued turns. The same panel separates session grants from project grants, filters exact tools/arguments, and requires a second matching `X` or Delete before revocation. Both scopes stop authorizing new calls immediately; project changes then atomically update `.a3s/permissions.acl` and restore the grant if persistence fails. Running tools are not cancelled. |
| Approvals | In Default mode, host Bash that the Rust guardrail cannot prove read-only and other calls that cross the established local boundary pause in a confirmation overlay with arguments and result context. Bounded workspace file tools and proven read-only commands remain quiet. Plan is a strict read-only boundary followed by Approve, Revise, or Abandon review. Auto never enters HITL; unproven host Bash and operations that cannot stay inside governed boundaries are denied. |
| Footer | The footer shows model/provider, effort, the active turn mode, a distinct `next:` composer mode when it differs, context fill, active asset, login/runtime state, and session hints. Context warnings re-arm after compaction, clear, or model switch. |
| System agent island | Enabled by default. `/island on`, `/island off`, and `/island status` persist or inspect the user preference, and the expanded island also offers `Turn off`. A fresh exact non-idle A3S lifecycle or recognized coding-agent process requests one native per-user window at the physical screen's top center; the shared lock prevents multiple `a3s code` TUIs from rendering duplicate islands. On notched Macs, native safe-area geometry makes the surface meet the physical top edge while its compact content occupies the two unobstructed side wings. A dedicated handle moves the window; periodic centering stops after a successful drag, and expand/collapse preserves the moved surface's top-center. Live `All`, `Needs you`, `Running`, and `Recent` filters preserve parent context and show direct-child progress. Every row shows state and elapsed time with an original vendor-colored robot; terminal durations freeze. Exact approval rows display a bounded reason, expose larger `Allow` / `Always` / `Deny` controls, and offer a direct reply composer; live parent rows can also accept replies or expose `Stop`, while running children may expose `Cancel`. Recognized Codex and other process-only rows are labeled `detected / process`, count as running evidence, trigger and keep the island visible, and never receive controls. Any exact planning/working row or recognized process enables the diffuse multicolor neon breathing border. The standalone Tao/Wry helper embeds offline HTML/CSS/JavaScript and does not use the GUI crate, React, or Next.js. Standard Wayland compositors may constrain exact global placement. |
| Tool calls | Live tool status appears inline while running. Inline `program` calls summarize structured intent, research scope, workflow phase, and completed nested-call results instead of repeating JavaScript wrapper source. |
| Semantic transcript | `Ctrl+T` opens the complete live session transcript in a dedicated full-width viewport, preserving user-surface, tool-state, and diff colors while showing reasoning, plans, every tool lifecycle and full output, subagent state, and the current live Markdown tail. |
| Workspace editor | `/ide` opens a full-screen file browser/editor. Press `p` on a workspace-tree item to open Live Preview, or use `:preview` in the editor to save a dirty buffer and preview it. `:status`, `:symbols`, `:definition`, `:declaration`, `:references`, `:implementations`, and `:diagnostics` query the shared saved-file Code Intelligence runtime asynchronously; a jump never discards a dirty buffer. `/config` reuses the editor for the active ACL config. |
| Cross-session context | `/ctx <query>` searches up to eight local indexed hits across supported coding-agent histories. `/ctx <n>` fetches the exact event window and stages it once as bounded, quoted, explicitly untrusted context; `/ctx save <n>` stores an episodic memory with `ctx_event_id` and `ctx_session_id` back-links. It does not upload transcript history to OS. |
| Memory and knowledge | `/memory` opens the durable memory graph, including promoted CTX provenance. `/kb` opens the local personal knowledge vault. `/okf` manages shareable knowledge packages. Memory search/recall/store and final verification are projected as bounded semantic activity without exposing recalled content or internal memory IDs in status notices. |
| Asset panels | `/agent`, `/mcp`, `/skill`, and `/okf` keep an active local asset visible while you iterate. `/flow` selects or drafts workflow DAG assets for OS Workflow as a Service rather than entering a persistent local dev mode. |
| Operations panels | `/status`, `/model`, `/effort`, `/history`, `/tasks`, `/permissions`, `/loop`, `/plugin`, `/theme`, `/help`, `/terminal`, and asset `activity` commands open focused panels or diagnostics without losing the current conversation. |

Key interactions:

| Key or input | Behavior |
| --- | --- |
| `Enter` | Send the prompt; when a turn is busy, queue the next message. On an empty composer during a live turn, send the current queue head now. |
| `Ctrl+O` | Send now: cancel the active turn and promote this prompt ahead of normal queued follow-ups. |
| `Shift+Enter` | Insert a newline in the input. |
| `Shift+Tab` | Cycle the composer mode: Default, strict read-only Plan, non-interactive Auto. Running and queued turns keep their submission-time mode. |
| `Up` / `Down` | Recall input history or move through menus/panels. |
| `PgUp` / `PgDn` | Scroll the transcript or the active full-screen panel. |
| `Shift+End` | Jump to the latest transcript output. |
| `Ctrl+T` | Open the complete live semantic session transcript, including full tool output and the current streaming tail. |
| `Ctrl+R` | Fuzzy-search current-session prompts; repeated Ctrl+R cycles matches. |
| `Ctrl+B` | Open or close delegated-task control without interrupting the parent turn. |
| `Esc` | Interrupt the running turn or close the active panel. |
| `Ctrl+C` twice | Quit the TUI after session persistence runs. |

The CLI refreshes the island snapshot every two seconds. Cooperating `a3s code`
TUI instances write versioned heartbeats under the platform's per-user A3S
state root; they expire after ten seconds, so a crash cannot leave a permanently
running agent. Parent and child terminal outcomes remain visible for eight
seconds. A fresh exact non-idle lifecycle or a recognized coding-agent process
triggers the native helper; an idle TUI heartbeat by itself does not. Every TUI
uses the same snapshot and `island.lock` path, so the per-user advisory lock
admits only one helper while another TUI can take over after its owner exits.
The helper closes once retained exact activity is idle and no fresh recognized
process evidence remains. Each collector atomically replaces
`system-snapshot.json` with a versioned, bounded projection for the native
helper. Only a sanitized workspace basename crosses this process boundary,
never the full path. Parent and child task descriptions are omitted on disk by
default, while the full text remains inside the owning TUI process.
`A3S_AGENT_STATUS_SHARE_TASKS=1` opts into sharing sanitized, single-line,
bounded task labels through heartbeats and the native snapshot. The collector
never exports command arguments from inferred third-party processes.

Inline controls are real TUI operations rather than display-only buttons.
Approval waits show a sanitized reason of at most 240 characters and expose
larger `Allow`, `Always`, and `Deny` buttons. A live parent stream can expose
`Stop`, a running child can expose `Cancel`, and exact parent rows can include a
reply composer. Replies accept at most 1,000 characters / 4 KiB; `Enter` sends
and `Shift+Enter` inserts a newline. A reply submitted while approval is pending
queues a normal follow-up message and does not implicitly approve or deny the
tool. The helper authorizes each action against its latest snapshot and appends
a versioned request to the private `control-requests` queue. The target TUI then
validates the short-lived, one-shot grant against its current instance,
activity, session, and tool or task context before reusing the normal approval,
submission, interruption, or subagent-cancellation path. The first valid action
consumes the shared activity token, so sibling controls and replay fail closed
until the next heartbeat publishes a fresh grant.

On Unix, the registry directory is restricted to `0700`; heartbeat and snapshot
files are `0600`; the control queue and request files use the same `0700` /
`0600` boundary. On Windows, the default registry is beneath the current user's
`%LOCALAPPDATA%` tree and relies on inherited ACLs; A3S does not install a
dedicated ACL. `A3S_AGENT_STATUS_DIR` can point diagnostics or hermetic tests at
an isolated registry and should always identify a user-private directory.

The persisted Agent Island preference defaults to on. Use `/island off` to
disable it, `/island on` to restore it, or `/island status` to inspect it. The
expanded island also exposes `Turn off`; reopening is intentionally done from
the TUI because no island control remains visible while it is disabled. This
preference controls only the floating surface; private exact-presence
heartbeats remain available to other system status consumers.

The `560 × 360` expanded attention workbench groups overlapping lifecycle
views: failures belong to both `Needs you` and `Recent`, while recognized
process-only evidence belongs to `Running` and `All`. Filtering a child retains
a labeled parent row, and parents report settled direct children against their
total. The ordinary compact and expanded surfaces are `392 × 60` and
`560 × 360`; their transparent native windows add 48 logical pixels of
horizontal and 32 pixels of vertical bleed on each side, producing `488 × 124`
and `656 × 424` windows. On a centered notched Mac display, the compact surface
widens when necessary to reserve the hardware gap plus 12 logical pixels of
clearance and a 160-pixel content wing on each side. Its top corners become
square and the surface starts at the physical top edge; the native glow may
extend above that edge. A small handle below the hardware gap starts native
window dragging. Once moved, the surface detaches from notch-fusion styling,
periodic recentering stops, and resizing preserves its top-center.

Set `A3S_AGENT_ISLAND=0` for an environment-level native launch override, or
`A3S_AGENT_ISLAND_BIN` to select a specific helper. SSH sessions and headless
Linux sessions skip automatic launch; `A3S_AGENT_ISLAND=1` explicitly enables
an attempt in those environments when the persisted user preference is on.
Launch and GUI failures are diagnostic only and never block `a3s code` startup.

### Startup, Sessions, And Safety

Launch the TUI from the repository or workspace the agent should inspect:

```sh
a3s code
a3s code resume <session-id>
a3s code resume
```

Config discovery checks `A3S_CONFIG_FILE`, then `.a3s/config.acl` while walking
upward from the current directory, then `~/.a3s/config.acl`. If none exists, the
first launch writes a starter `~/.a3s/config.acl` and opens it in the built-in
editor. Project-local config can set model/provider choices, OS endpoint,
`flow_dir`, `agent_dir`, `mcp_dir`, `skill_dir`, storage, memory, delegation,
and asset paths.

Core session snapshots auto-save under
`<workspace>/.a3s/tui/sessions/v1/sessions`; TUI-owned per-session state is
stored under `<workspace>/.a3s/tui/session-state/v1`. Exiting prints the exact
`a3s code resume <session-id>` command and highlights the command when color
output is enabled. `a3s code resume` without an id resumes the newest saved
session in that workspace. Resume restores the selected model and credential
source, effort profile, execution mode (`default`, `plan`, or `auto`), and syntax
theme instead of resetting them to launch defaults. `/fork` (or
`/fork session`) copies the current transcript into a new session id while
keeping the original. `/fork worktree` additionally creates
`a3s/fork-<id>` in a sibling `.a3s-worktrees` directory, transfers the current
tracked and untracked workspace content through a binary Git patch, copies the
complete session and TUI sidecar into the isolated workspace, and prints the
exact command that opens it. TUI persistence is transferred explicitly rather
than captured in the patch, and the user's real Git index is never modified.
A post-creation failure retains the worktree and reports its path.
Every new isolated session also records its source repository and immutable
base commit. From that session, `/worktree status` reports the branch, source,
changed-file count, and commits since the base. `/worktree handoff` snapshots
the complete workspace-scoped result through the same alternate-index path and
writes a Git binary patch plus a JSON manifest under the source repository's
`.a3s/tui/worktree-handoffs/v1` directory. The manifest binds the patch's exact
SHA-256 digest, byte length, session, branch, source, worktree, workspace scope,
and base commit. The command prints separate inspect and `git apply --3way`
commands but never applies, stages, commits, or merges automatically.
`/worktree cleanup` likewise prints ordinary `git worktree remove` and
`git branch -d` commands without executing them or suggesting a force flag;
Git therefore refuses cleanup while uncommitted or unintegrated work remains.

For ordinary user turns, the TUI records bounded pre/post Git tree checkpoints
with an alternate temporary index. `/rewind` forks the pre-turn conversation
under a new session id and reverses the last file patch only after `git apply
--check` succeeds. If a touched file changed after that turn, rewind refuses the
entire file operation without overwriting it. It never moves `HEAD` or changes
the real Git index, and the original conversation remains resumable. Outside a
Git worktree, the same command can still rewind the conversation and reports
that file rewind was unavailable. `/clear` starts a fresh conversation.

If exit interrupts a durable `/goal`, the goal is saved as paused. The resumed
TUI opens a startup picker with `Resume goal` and `Leave paused`. The first
choice continues the next goal iteration without changing the restored execution
mode; the second enters the session with the goal still paused, where
`/goal resume` can continue it later.

The TUI owns HITL confirmation for boundary crossings. In Default mode,
workspace-scoped file changes run without approval. Bash executes through the
active workspace's host command runner. One shared Rust guardrail silently
admits only a deliberately small command subset that it can prove read-only;
unproven shell work, protected metadata changes, mutating Git operations, and
MCP or application tools that advertise side effects enter the wheel-browsable,
clickable approval overlay. Critical shell operations fail closed before an
approval can be created. Each decision is scoped
explicitly: allow once; allow the exact operation and resource for this
session; add that exact capability to the project's `.a3s/permissions.acl`; or
deny it and send typed guidance back to the agent. Shell grants retain the exact
command and boundary request, file-mutation grants retain the exact operation
and path, and other tools retain their complete canonical arguments. Project
ACL writes use a bounded, validated, atomic replace and never derive a rule from
presentation text. Shift+Tab still cycles default, plan, and auto modes, and
`/auto` remains an explicit execution-mode choice rather than a side effect of
one approval.
Plan mode exposes only read-only discovery tools. When planning completes, the
TUI freezes queue draining at an explicit Approve, Revise, or Abandon boundary;
approval starts implementation as a separate Default turn, and remembered
grants cannot bypass the read-only plan boundary. Auto mode resolves every
decision without user interaction. Rust-proven read-only Bash is admitted;
unproven host Bash, protected metadata, and mutating Git operations fail closed,
and explicit policy denials and workspace guardrails remain authoritative.
Delegated tasks and Skill runs inherit the same permission checker and parent
confirmation boundary. Core freezes that governance when the run is admitted;
foreground, queued, parallel, Skill, and background descendants keep the same
snapshot even if the composer selects another mode for the next turn.
Tool timeouts and confirmation timeouts are tracked separately so a human
approval pause does not consume the command runtime budget.

All local filesystem work stays under the active workspace services and A3S Code
permission policy. OS operations require `/login`; before login the TUI can
still author local assets, run local subagents, use local memory, and execute
DynamicWorkflowRuntime, but the OS `runtime` tool, RemoteUI ViewLinks, asset
publishing, and OS service activity panels are unavailable.

For CI or release probes, set `A3S_CODE_TUI_SMOKE=1` to exercise the same
`AgentSession::stream()` integration without taking over the terminal.

### Tool Runtime And Safety

A3S Code TUI exposes tools through the session registry, not by letting the
model run arbitrary host APIs. Each tool call carries a name, JSON arguments,
streamed output, timeout policy, permission decision, and traceable event id.
The TUI then turns those events into live status lines, retained output logs,
approval prompts, and RemoteUI action links.

TUI, Code Web, and `code exec` attach the same verified managed SRT process
sandbox to Core. Default and Auto admit ordinary Bash only through that
boundary; Plan denies Bash. `sandbox_permissions = "require_escalated"` is an
explicit host-boundary request: Default asks for the exact command and Auto
denies it. If runtime preparation or the real OS capability probe fails,
Default asks for every otherwise valid host command and Auto denies Bash. The
shared Rust guardrail still rejects catastrophic commands and protected
credential/control paths before either sandbox or approval can run.

Official archives and Homebrew carry the exact locked support tree. Its package
identity, registry sources and integrity values, regular-file shape, bounds,
and complete normalized digest are verified before use; standalone self-update
replaces it in the same rollback transaction as the CLI and window helper.
Source builds can bootstrap the same exact tree with npm lifecycle scripts
disabled. A global `srt` is never selected. Node.js 20.11 or newer is required;
Linux additionally requires bubblewrap, socat, ripgrep, and usable unprivileged
user namespaces; macOS requires sandbox-exec and ripgrep; Windows requires one
explicit elevated machine setup through `a3s code sandbox setup`. Use
`a3s code sandbox status` for a read-only runtime and native-boundary probe.
Offline mode may reuse verified state or the release payload but never performs
registry bootstrap.

The sandbox denies network egress, local binding, and Unix sockets; limits
writes to the active workspace and a private scratch directory; protects Git,
A3S, editor, agent, and tool control metadata; blocks common credential stores
and nested `.env*` files; rejects sensitive hard-link aliases; and passes a
scrubbed environment plus explicit command values. Timeouts, process-group
cancellation, bounded output, streaming deltas, and delegated/Skill child runs
retain the same frozen boundary. There is no silent unsandboxed fallback.

The TUI also enables Core's local workspace credential policy for in-process
tools. `read`, range reads, unified `search` in grep mode, `write`, `edit`, and
`patch` therefore
cannot bypass the command boundary: explicit sensitive targets fail closed,
directory grep omits protected candidates, and source-tree hardlink aliases
are rejected before a write can truncate them. Legitimate package-store
hardlinks remain available unless they alias a discovered credential inode.
Read-only Git diff enumerates changed paths in a non-ambiguous format and
regenerates output only for allowed files. Option-like revision input cannot
become a Git flag, and remote display removes embedded HTTP credentials and
query tokens.

The local boundary targets routine repository work. Dependency-heavy,
untrusted, OCI, build, and test workloads that need stronger isolation still
belong on A3S Box or an A3S Runtime placement; they are never promoted silently
from the local sandbox.

| Tool family | TUI behavior |
| --- | --- |
| Workspace tools | `read`, `ls`, and unified `search` modes (`grep`, `glob`, `bm25`) coalesce into Explore cells; shell/git calls use Running/Ran command cells; writes and edits show Added/Edited/Deleted diffs only after successful execution. A3S Code v5.2.2 also supports resumable `write` calls with `mode = "append"` and a UTF-8 `expected_offset`, so long ordinary files can continue idempotently without resending prior content. All operations still run through workspace services, path boundaries, timeout handling, cancellation settlement, and confirmation policy. |
| Structured output | `generate_object` uses `Generating/Generated object` cards and keeps schema-shaped JSON in the same bounded tool event stream as normal tools. |
| Web retrieval | Successful `web_search` cards hide the raw provider body but project Core 6.7's structured result count, headless/HTTP/API tier path, retrieval-requirement decision, engine outcomes, fallback use, and output-limited state. Degraded success uses warning semantics; `Ctrl+T` expands the search evidence and result body. `web_fetch` keeps the same concise success and explicit-failure presentation. |
| MCP tools | Configured `mcp__<server>__<tool>` calls render as `Calling/Called server.tool({...})` while retaining the same approval, output, and error path. |
| PTC scripts | The `program` tool runs sandboxed JavaScript-compatible scripts with a host-provided `ctx` object and summarizes its structured nested-call metadata. Recursive `program`, `dynamic_workflow`, and the hidden `parallel_task` alias stay out of the default PTC allow-list; a one-item `task` call is allowed, but direct fan-out is blocked. |
| Delegation | `task` launches one focused child for a single `tasks[]` item or fans out multiple independent items on the native host runtime, preserves input order, emits subagent progress events, and respects `max_parallel_tasks`. |
| Dynamic workflow | `dynamic_workflow` is always registered because `ultracode` and `?` DeepResearch use it. Its cell shows the run id, active generation-slot limit, and structured step status instead of raw workflow metadata; durable history lives under `.a3s/workflow`. Exact completed-step recovery is bound to the original run and query. |
| OS runtime | The `runtime` tool is registered only after `/login`. Once present, normal model turns and dynamic workflow PTC steps can call it for OS Function as a Service batch execution. |
| Dynamic tools | Agent-directory and host-registered tools without a dedicated renderer fall back to bounded Codex-style `Calling/Called tool(args)` cards instead of exposing an unformatted tool name. |

### Effort Profiles

`/effort` is not just a UI label. It rebuilds the active session with a larger
reasoning budget, larger tool-round budget, longer auto-continuation allowance,
and stronger model-agnostic rigor guidance. These host-side budgets continue to
apply for every provider. Anthropic models also receive the thinking budget
directly; signed-in Codex models receive their catalog-supported native level as
`reasoning.effort`; other GPT, GLM, OS Gateway, and account-backed models use the
profile through prompt guidance and host limits.

These changes use an asynchronous atomic session replacement: the current
session remains live if the new configuration cannot be built, and is closed
only after the replacement is ready with the same persisted identity.

| Level | Thinking budget | Tool rounds | Continuations | Parallel tasks | Intended behavior |
| --- | ---: | ---: | ---: | ---: | --- |
| `low` | 2,048 | 240 | 4 | 4 | Fast, minimal changes with narrow verification. |
| `medium` | 8,192 | 800 | 8 | 8 | Balanced default behavior without extra depth steering. |
| `high` | 16,384 | 1,200 | 12 | 8 | More deliberate planning, relevant tests, and self-review. |
| `xhigh` | 32,768 | 1,800 | 16 | 8 | Compare alternatives, probe edge cases, and verify thoroughly. |
| `max` | 65,536 | 2,400 | 24 | 8 | Maximum rigor for correctness, adversarial checks, and completeness. |
| `ultracode` | 65,536 | 3,200 | 32 | 8 | Message-gated dynamic workflow mode: trivial turns stay direct; complex turns may use `dynamic_workflow`, A3S Flow replay, host-side `task` fan-out, and signed-in `runtime`. |

For signed-in Codex models, `low`, `medium`, `high`, `xhigh`, and `max` request
the same-named native reasoning effort. `ultracode` remains an A3S orchestration
profile and uses Codex's maximum wire effort: `max` for Sol, Terra, and Luna,
and `xhigh` for older GPT models. The account catalog's product-level `ultra`
label is never sent as `reasoning.effort`; like native Codex, A3S maps it to
`max` and supplies multi-agent orchestration separately. When a requested level
is unavailable, A3S clamps it downward and shows the effective level in the TUI.

All effort levels keep local `task` available with the profile-specific limits
shown above. Runtime-driven automatic delegation is
disabled for `low` through `max`; those levels continue to control native Codex
reasoning independently. `ultracode` enables automatic delegation alongside
`PlanningMode::Auto`, goal tracking, and dynamic-workflow guidance, while the
pre-analysis gate still decides whether a turn actually needs planning or
fan-out. The eight-task value is a shared provider-admission window, not a cap
on total goal work: larger investigations run in bounded waves without bursting
one signed-in account. Final-answer synthesis continuations never start another
delegation wave.

### Dynamic Workflows

There are two workflow concepts, intentionally kept separate:

| Concept | Surface | Purpose |
| --- | --- | --- |
| `DynamicWorkflowRuntime` | Model-visible `dynamic_workflow` tool, used by `ultracode` and `?` DeepResearch | Per-turn dynamic orchestration. A sandboxed JavaScript PTC function returns A3S Flow commands such as `complete`, `fail`, `schedule_step`, or `schedule_steps`; A3S Flow records replayable workflow and step history. |
| OS Workflow as a Service | `/flow`, `/flow publish`, `/flow run`, `/flow deploy`, `/flow open`, `/flow logs`, `/flow status` | Durable workflow asset lifecycle. Local DAG JSON files are published as OS `workflow` assets with runtime-binding metadata and opened in the OS workflow designer/run surfaces. |

Dynamic workflow PTC steps can call ordinary tools such as `ctx.read`,
`ctx.search`, or `ctx.tool("runtime", ...)` when `runtime` is registered after OS
login. They may call `task` with one item but cannot fan out directly. To fan
out local subagents, the workflow schedules a Flow step with
`step_name: "task"`; the TUI host then runs the native implementation outside
QuickJS. Legacy persisted `parallel_task` steps remain readable.

Minimal dynamic workflow scripts return Flow commands from a default exported
function. If you author the script in TypeScript locally, transpile it first:
the source passed to the TUI runtime must be JavaScript-compatible for the
QuickJS PTC sandbox.

```javascript
export default async function run(ctx, inputs) {
  if (inputs.kind === "workflow") {
    return {
      type: "schedule_steps",
      steps: [
        {
          step_id: "inspect",
          step_name: "inspect_workspace",
          input: { query: inputs.input.query }
        },
        {
          step_id: "fanout",
          step_name: "task",
          input: {
            tasks: [
              {
                agent: "explore",
                description: "Find test coverage",
                prompt: "Inspect relevant tests and coverage gaps."
              },
              {
                agent: "review",
                description: "Review risk",
                prompt: "Review the approach for regressions."
              }
            ]
          }
        }
      ]
    };
  }

  if (inputs.step_name === "inspect_workspace") {
    const hits = await ctx.search(inputs.input.query, {
      mode: "grep",
      include: "*.rs"
    });
    return { hits };
  }

  return { ok: true };
}
```

### Architecture

A3S Code is a TEA-style terminal application: terminal events and agent stream
events become `Msg` values, `Model.update` mutates one session model, and view
functions render the current state through `a3s-tui`. Runtime-heavy state is
kept as a small ECS-style projection: tool runs, subagent runs, Runtime activity
records, and RemoteUI links are updated by stable event ids and queried by
panels instead of coupling every panel to the streaming protocol.

The command palette, asset selectors, approval overlay, `/model` account picker,
`/plugin` skill toggles, detail panels, tool status lines, transcript gutters
and user bubbles, input prompt chrome, live reasoning, live and completed tool
output, pinned plan rows, task summaries, file-edit diffs, SPF/IDE file
metadata, `/loop` details, compaction progress, the live activity shimmer,
effort overlay, and footer status rows use
shared `a3s-tui` components such as
`MenuPanel`, `ChoicePrompt`, `TabbedMenuPanel`, `DetailPanel`, `Timeline`,
`ActivityBlock`,
`SectionHeader`, `ToolStatusLine`, `GutterBlock`, `InlineAction`, `Alert`,
`TextOverlay`, `Toast`,
`InputBorder`, `PromptLine`, `OutputBlock`, `Badge`, `Checklist`, `CursorLine`,
`DiffView`, `Divider`, `PanelFrame`, `Breadcrumb`, `Progress`, `Confirm`,
`Paragraph`, `PreviewPanel`, `TreePicker`, `ShimmerText`, `LevelSlider`,
`Scrollbar`, `Sparkline`, `KeyValue`, `DataTable`, `WrappedPrefixBlock`,
`SessionStatus`, `ModeLine`, and the `Meter` context fill rendered inside the
footer status row. Reusable menu scrolling, selection, slash command wheel
browsing and click-to-run, approval overlay wheel browsing and click-to-approve
or deny, `/model` account tab mouse switching, `/effort` wheel/click adjustment,
`/theme` wheel preview and click-to-apply, `@` file picker wheel browsing and
click-to-insert, `/agent` picker wheel browsing and click-to-develop,
`/mcp` picker wheel browsing and click-to-develop, `/skill` picker wheel
browsing and click-to-develop, `/okf` picker wheel browsing and click-to-develop,
`/flow` picker wheel browsing and click-to-open, `/plugin` wheel browsing and
click-to-toggle, approval choices, RemoteUI and jump-to-latest action links, tool status
truncation, shared alert rows for OS login/configuration warnings, overlay
composition for menus and prompts, IDE flash footer notifications, live tool
activity/output tails,
`/loop` key-value summaries, `/kb` delete confirmations, transcript gutters and
input bubbles, prompt continuation alignment, input border labels, shared
display-width wrapping for live reasoning and detail text, completed output tail
previews, pinned plan checklists, task status summaries, compaction progress
bars, pinned memory importance bars, transcript scrollbars, IDE cursor rows,
panel dividers, activity output tails, diff wrapping, framed panels, breadcrumbs,
detail-row layout, activity shimmer, `/model` tab hit-testing, `/effort` slider
hit-testing, slash command palette hit-testing, approval overlay hit-testing,
`/theme` preview hit-testing, `@` file picker hit-testing, `/agent` picker
hit-testing, `/mcp` picker hit-testing, `/skill` picker hit-testing, `/flow`
picker hit-testing, `/plugin` overlay hit-testing, and width-bounding fixes are
exercised directly by `a3s code`.

```mermaid
flowchart TD
    user["Terminal user"] --> cli["a3s CLI<br/>a3s code"]
    cli --> app["TEA TUI App<br/>App state + Msg"]

    app --> core["a3s-code-core<br/>AgentSession"]
    core --> events["AgentEvent stream<br/>text, tools, planning, subagents"]
    events --> pump["event pump<br/>AgentEvent -> Msg::Agent"]
    pump --> app

    app --> update["Model.update<br/>commands, approvals, panels"]
    update --> render["view + layout<br/>a3s-tui"]
    render --> frame["terminal frame"]
    frame --> user

    events --> projection["RuntimeProjection<br/>local ECS-style projection"]
    events --> corestatus["CoreRunStatus<br/>mode, context, planning, external tasks"]
    projection --> toolrun["ToolRun entities<br/>live input/output/status"]
    projection --> subrun["SubagentRun entities<br/>tokens, timing, result"]
    corestatus --> render

    app --> dynamic["DynamicWorkflowRuntime<br/>A3S Flow + PTC"]
    dynamic --> program["program tool<br/>QuickJS sandbox"]
    dynamic --> hostparallel["host task fan-out<br/>native local execution"]

    projection --> panels["TUI panels<br/>chat, plan, transcript"]
    app --> assets["Asset panels<br/>agent, MCP, flow, skill, OKF, KB"]
    panels --> render
    assets --> render

    core --> workspace["workspace services<br/>shell, files, MCP tools, permissions"]
    workspace --> project["current workspace<br/>source, config, .a3s assets"]
    assets --> project

    app --> os["A3S OS progressive APIs<br/>assets, runtime, functions, workflows, knowledge"]
    dynamic --> osruntime["login-gated runtime tool<br/>OS batch execution"]
    assets --> os
    core --> os
    os --> remote["RemoteUI ViewLink<br/>.view / viewUrl"]
    remote --> webview["a3s-webview<br/>browser fallback"]
    webview --> user
```

### OS, Runtime, and RemoteUI

Add an OS endpoint to `config.acl`, then sign in:

```acl
os = "https://os.example.com"
```

```sh
a3s code
# then inside the TUI:
/login
```

After login, A3S Code can use OS capabilities directly from the TUI:

| Command | What it does |
| --- | --- |
| `/flow` | Select a local workflow DAG JSON, publish it as an OS workflow asset, and open the OS workflow designer; `/flow <description>` drafts a new DAG first. `/flow` is OS Workflow as a Service, not the per-turn dynamic workflow runtime. |
| Asset `activity` subcommands | Browse asset-related Runtime activity through `/agent activity`, `/mcp activity`, `/flow activity`, `/skill activity`, or `/okf activity`; when a local asset is not active, A3S Code opens the matching selection panel first. |
| `/mcp publish/run/test` | Publish the active local MCP asset as an OS `mcp` asset, then run or batch-test it through OS Function as a Service. |
| `runtime` tool | Registered only after `/login`. It resolves a tool-kind worker asset by UUID or name, submits independent inputs to OS Function as a Service batch execution, streams progress, and returns aggregated results. |

Signed-out behavior is intentionally useful but local: chat, file editing,
tools, MCP, local asset drafting, memory, `/ctx`, `/kb`, `task`,
`dynamic_workflow`, full local DeepResearch, and local loops keep
working. Signed-in behavior adds OS assets, Function as a Service, Workflow as a
Service, Knowledge service deployment, RemoteUI ViewLinks, asset activity
panels, and the `runtime` tool.

| Capability | Signed out | Signed in after `/login` |
| --- | --- | --- |
| Coding chat and workspace tools | Available with local permission checks and HITL approval. | Available with the same local safety path. |
| Context, memory, and local knowledge | `/ctx`, `/memory`, `/sleep`, and `/kb` use local stores. | Local stores remain available; OS-backed reports can also return RemoteUI views. |
| Dynamic workflows | `DynamicWorkflowRuntime` can run local Flow-backed orchestration and host-side `task` fan-out. | Workflow PTC steps may also call the registered `runtime` tool for OS batch work. |
| Asset authoring | `/agent`, `/mcp`, `/skill`, `/flow <description>`, and `/okf` can draft and review local assets. | Publish, deploy, run, open, logs, status, list, and activity commands can use OS services. |
| RemoteUI | Validated local DeepResearch HTML opens through the loopback report viewer; OS `.view`/`viewUrl` responses are unavailable. | Local reports remain available, and OS `.view`/`viewUrl` responses also become inline `Open view` actions. |
| Runtime activity | Use the standalone `a3s top` command for local processes. | Asset `activity` commands inspect OS Runtime jobs, runs, invocations, indexing, and workflow activity. |
| Updates and recovery | `/update`, `/fork`, `/fork worktree`, `/rewind`, `/clear`, and `a3s code resume` remain local. | Same behavior; saved sessions keep OS login-derived capability state separate from secrets. |

### OS Service Mapping

| OS mechanism | A3S Code TUI path |
| --- | --- |
| Agent as a Service | `/agent publish agentic`, `/agent publish application`, `/agent run`, and `/agent deploy` use OS `agent` assets with `agentKind=agentic` or `agentKind=application`. Publish commits package source at the asset repository root and keeps the package visible; `.a3s/` is reserved for `asset.acl` only. OS agent-config and runtime-binding endpoints are synced from the ACL metadata when available. Run/deploy first discover the current OS operation through progressive capabilities with `shaped=true`, then fall back to REST probes and the OS asset view. `/agent open` and `/agent logs` inspect existing assets and prefer progressive ViewLinks before static OS views. |
| Function as a Service | Tool-kind agents and MCP tool calls stay Runtime workers. `/agent publish tool` uses OS `agent` assets with `agentKind=tool` and a Function as a Service runtime binding; `/mcp publish`, `/mcp run`, `/mcp deploy`, and `/mcp test` use OS `mcp` assets with serving metadata from `.a3s/asset.acl`; `/skill publish` and `/skill deploy` use OS `skill` assets with serving Function as a Service binding intent. These assets keep source at the repository root and do not generate family-specific JSON config files. MCP run/test require a real OS MCP runner/test capability discovered through progressive capabilities; if OS does not expose one yet, the command fails clearly instead of pretending an MCP asset is a Runtime Function. Skill deploy/open paths first try OS progressive capabilities with `shaped=true` so `.view`/`viewUrl` survives as a ViewLink, then fall back to safe asset views. The `runtime` tool sends parallel batches to OS Function as a Service only for real runtime function/tool workers; OS resolves the runnable kind server-side. |
| Workflow as a Service | `/flow`, `/flow publish`, `/flow run`, and `/flow deploy` create or update OS `workflow` assets, commit the visible DAG source as `flow.json`, write `.a3s/asset.acl`, then sync the runtime-binding endpoint when available. Run/deploy first try OS progressive capabilities with `shaped=true` for a workflow designer ViewLink and fall back to the standalone workflow designer for edit and run. `/flow open`, `/flow logs`, and `/flow status` inspect the asset, logs, and runtime binding without mutating it. |
| Knowledge service | `/okf` selects local OKF packages. `/okf publish` creates or updates an OS `knowledge` asset, uploads the visible package sources plus `.a3s/asset.acl`, and syncs the runtime-binding endpoint when available. `/okf deploy` publishes the package first, then tries OS progressive knowledge-service deployment with `shaped=true`; if no matching operation exists, the Knowledge service view opens. `/okf status` checks the OS asset and runtime binding without mutating it. `/kb vault` remains the local personal knowledge-base browser. Without OS, deploy stays local and reports the blocked knowledge-service inputs. |

### AI-Native Asset Lifecycle

A3S Code treats agents, MCP servers, skills, OKF knowledge packages, and
workflow flows as team digital assets and shared context. Each asset family
uses the same lifecycle vocabulary in the TUI and OS, while exposing only the
commands backed by real local or OS surfaces:

| Stage | TUI responsibility | OS responsibility |
| --- | --- | --- |
| Create | Draft a local asset package or definition from natural language. | Create a private/team asset workspace with typed metadata. |
| Develop | Agents, MCP servers, skills, and OKF packages enter local multi-turn asset-development mode with a visible active asset and an exit path. Workflow flows use local DAG editing plus the OS workflow designer instead of a persistent local mode. | Keep asset source, metadata, secrets, and collaboration history as shared context. |
| Run/test | Run local smoke checks first; expose direct run/test commands only when the asset family has a real service surface. | MCP servers use Function as a Service run/test calls. Agentic agents are exercised through `/agent run`, workflow flows through `/flow run`, application agents through `/agent deploy`, and tool agents, skills, and OKF packages do not expose direct TUI run commands. |
| Publish | Commit source, entrypoints, examples, tests, and `.a3s/asset.acl`. | Validate ACL-derived config/runtime binding intent, package the asset, record release gates, and expose team discovery. |
| Deploy | Trigger only the production deployment shape that matches the asset type. | Launch long-running applications only when needed; prefer serving Function as a Service for stateless tools and MCP calls. |
| Inspect | Open read-only asset views, status, logs, or runtime-binding checks only when that asset family exposes the surface. | Provide asset metadata, binding validation, service views, package state, and RemoteUI evidence without mutating assets. |
| Activity | Browse asset-scoped Runtime activity instead of using a top-level process or run manager. | Provide function invocations, batches, workflow runs, indexing/evaluation jobs, and agent runs filtered to the selected asset. |

OS RemoteUI views are captured from progressive responses (`.view`/`viewUrl`).
The TUI remembers the latest OS view and surfaces ViewLinks returned by
asset-scoped actions. DeepResearch uses a separate path: it validates a local,
source-traceable HTML report and serves it through the loopback viewer without
injecting OS credentials. OS-enabled loops may still require fan-out evidence
plus a shaped `.view`/`viewUrl`; when either part is missing, they spend the next
loop turn on a targeted Runtime-evidence retry before accepting a final answer.

### Core Command Reference

These commands are available outside the asset-specific flows:

| Command | Capability |
| --- | --- |
| `/help` | Open the full command guide with quick-start input, keys, commands grouped by Workflow, Session, Context, Asset, and System, domain-specific command forms, panels, and resume help. |
| `/status` | Print a read-only snapshot of session identity, workspace and branch, model and effort, active and next permission modes, workspace guardrails, context/output tokens, activity and pending queue, semantic retrieval phase/coverage/failures, OS account state, active asset/goal scopes, and the exact resume command. It performs no network request and is available while a turn runs. |
| `/model` | Switch among configured ACL models, OS gateway models, and signed-in account-backed model tabs when available. |
| `/effort` | Change the active effort profile from `low` to `ultracode`, with keyboard, wheel, and click adjustment before confirmation rebuilds the session with matching budgets and prompt guidance. |
| `/init` | Analyze the workspace and generate an `AGENTS.md` instruction file. |
| `/config` | Edit the active ACL config in the built-in editor. |
| `/terminal` | Inspect the detected emulator and multiplexer, terminal I/O, canvas size, render fallback, color depth, alternate-screen/mouse/paste support, enhanced keys, OSC 8 links, OSC 52 copy, and actionable passthrough warnings. |
| `/checkup` | Review local context hygiene from real `Skill` usage. A typed preflight scans at most 128 persisted sessions and reports invocation/session counts plus context bytes. Cleanup suggestions require at least three sessions, twelve completed turns, and a Skill older than 14 days. Recently changed, duplicate-name, disabled, managed, and unknown-age Skills are excluded. Instruction and MCP counts are footprint signals only and never treated as usage telemetry. A strict read-only Plan turn offers each eligible Skill as a separate reversible `/plugin` disable choice; it never deletes files or changes anything before Approve / Revise / Abandon and normal HITL. |
| `/queue` | Inspect pending follow-ups with their submission-time modes; Send now, remove one row, or explicitly confirm clearing all pending rows. |
| `/history` | Fuzzy-search up to 100 matching prompts from the current session and restore the selected text without disturbing the current draft on cancel. |
| `/copy` / `/copy transcript` | Copy the latest assistant source Markdown or the complete semantic session. Native clipboard delivery is reported only when verified; otherwise the TUI identifies the OSC 52 request and its 64,000-byte UTF-8 payload limit. |
| `/export [path]` | Atomically create a private Markdown session snapshot at a workspace-relative path. With no path, generate a unique session-and-time filename; never overwrite an existing target. |
| `/tasks` | Inspect the current session's running and recent delegated tasks, search status/progress/output, open full details, refresh, or safely cancel a running task. |
| `/review` / `/review working-tree` | Run a strictly read-only review of staged, unstaged, and relevant untracked changes, then open the existing severity-sorted issue checklist. |
| `/review commit <revision>` | Review exactly one commit patch plus the surrounding code needed to prove findings. |
| `/review branch <base>` | Review the merge-base-to-HEAD branch patch plus current staged and unstaged changes. |
| `/permissions` | Inspect or cycle the next-turn Default/Plan/Auto mode with `M`, search exact session and project grants, inspect canonical arguments, and revoke with a second matching confirmation. A mode change does not alter the active or already queued turn. Project revocation atomically updates `.a3s/permissions.acl`; all revocation applies to future checks only. |
| `/theme` | Cycle syntax highlighting themes. |
| `/login` / `/logout` | Sign in or out of the configured OS account; login registers OS capabilities and the `runtime` tool. |
| `/ide` | Open the workspace file browser and editor. |
| `/preview <path\|localhost-url>` | Open the target in Work's persistent Live Preview. `/preview status` reports the tracked window and `/preview stop` closes only the owned native window, leaving the shared Web service running. |
| `/memory` | Browse durable memory as an event/entity graph with tiers, aliases, relations, conflicts, and forget candidates. |
| `/evolution` | Review LLM-authored reusable preferences, Skills, and OKF candidates; inspect evidence, activation state, audit history, and immutable versions; save, reject, reconsider, restore a version, or return to the unmaterialized baseline. Mature conflict-free candidates may save locally automatically, but are never published. |
| `/ctx <query>` | Search up to eight matches in locally indexed A3S Code, Claude Code, Codex, and Cursor sessions without refreshing the index during the interactive request. |
| `/ctx <n>` | Fetch the selected event window and attach one sanitized, quote-prefixed, explicitly untrusted block of at most 6,000 bytes to the next message only. |
| `/ctx save <n>` | Promote the selected hit into durable episodic memory with its provider, event ID, session ID, and timestamp provenance. |
| `/sleep` | Consolidate the day's work into memory, including experience, preferences, and knowledge. |
| `/kb` / `/kb add` / `/kb import` / `/kb search` / `/kb vault` | Manage the local personal knowledge base. |
| `/goal <text>` | Start a durable goal run: switch to `ultracode`, create `.a3s/loops/goal-*` with state/log/budget and maker/verifier skills, force a dependency-ordered maker → verifier plan, and continue until Core emits a matching verified `GoalAchieved`. Independent read-only work may fan out inside a phase, while progress is persisted only at five-percent checkpoints. Esc or `/goal clear` cancels the run and invalidates pending retries. |
| `/goal resume` | Continue a durable goal that was left paused during session resume. |
| `/compact` | Summarize and shrink the active conversation context. |
| `/clear` | Start a fresh conversation in the current session surface. |
| `/fork` / `/fork session` | Branch the current transcript into a new session id. |
| `/fork worktree` | Create an isolated Git branch/worktree, transfer current workspace content without changing the real index, and copy the complete session into it. |
| `/worktree status` | Inspect the current A3S-managed isolated worktree, its immutable source/base identity, changed files, and commits since the base. |
| `/worktree handoff` | Create a bounded binary Git patch and SHA-256-bound JSON manifest containing committed, staged, unstaged, and untracked workspace content without mutating the source worktree. |
| `/worktree cleanup` | Print non-forcing worktree and branch removal commands. It does not remove anything while the TUI still owns the workspace. |
| `/rewind` | Fork the conversation before the last completed user turn and reverse its workspace patch only when conflict checks pass. |
| `/relay` | Open the multi-session/background-work dashboard. Press `/` to filter by task, status, model, session id, or source path; `Space` toggles a compact task peek; `R` refreshes immediately, and the panel also refreshes every 15 seconds while retaining the selected session when it is still present. Native resume restores model, effort, execution mode, theme, and paused goal. |
| `/auto` | Switch future submissions to non-interactive Auto mode. Operations that survive explicit policy and workspace hard denials execute without HITL. |
| `/plugin` / `/reload` | Manage and hot-reload skills/plugins, including wheel browsing and click-to-toggle skill state in the TUI. |
| `/update` | Upgrade the CLI and restart back into the saved session. |
| `/exit` | Quit `a3s code` after session persistence runs. |

Session copy/export uses the raw semantic transcript rather than terminal-wrapped
ANSI output. It preserves user and assistant Markdown, visible tool lifecycle
and output, and visible delegated-task results. Private reasoning, transient UI
notices, terminal-width-dependent rows, and hidden duplicate cells are excluded.
An explicit export path must stay inside the workspace, its parent directory
must already exist, and an existing file or escaping symlink is rejected.

A3S Code auto-discovers `SKILL.md` skills from project and user roots:
`.a3s/skills`, `.agents/skills`, `.codex/skills`, `.claude/skills`, plus
plugin-bundled `plugins/**/skills` directories under `.agents`, `.codex`, and
`.claude`. Discovered skills appear in `/plugin` and are selected on demand by
the skill matcher for the current request.

### Agents, Research, and Loops

| Command | What it does |
| --- | --- |
| `/agent` | Select a local agent package from `agent_dir` with keyboard, wheel, or click, then enter local multi-turn agent-development mode. The TUI shows the active agent; press Esc or run `/agent off` to return to normal mode. While active, `/goal` becomes an agent-scoped durable goal loop and `/loop` runs local agent-scoped loop engineering. No OS WebIDE or RemoteUI is opened for this local VibeCoding flow. |
| `/agent <description>` | Draft a package directory with a Markdown/YAML agent entrypoint under `agent_dir`, then use `/agent` to iterate on it. |
| `/agent clone <git-url>` | Clone an existing agent asset source into `agent_dir`, then use `/agent` to select it. |
| `/agent list [query]` | Browse OS agent assets through the asset-scoped list panel. |
| `/agent activity [query]` | Inspect Runtime activity, jobs, and runs for the selected local agent; when no agent is active, A3S Code opens the agent selection panel first. |
| `/agent review` | Review the active local agent. If no agent is active, A3S Code opens the agent selection panel first, enters agent-development mode, then reviews the selected agent. |
| `/agent publish agentic` | Publish the active local agent package as an OS `agent` asset with `agentKind=agentic`. The package source, entrypoint, manifest, runtime binding intent, and machine-readable agent config are saved with the asset source, then synced to OS agent-config and runtime-binding endpoints when available. |
| `/agent publish application` | Publish the active local agent package as an OS `agent` asset with `agentKind=application`, ready for OS-side application-agent deployment. The same manifest, runtime binding intent, agent-config sync, and runtime-binding sync are applied. |
| `/agent publish tool` | Publish the active local agent package as an OS `agent` asset with `agentKind=tool` and a Function as a Service runtime binding. The package source, entrypoint, config metadata, and runtime binding intent are committed; runtime-binding sync is attempted when available. |
| `/agent run` | Publish or update the active local agent as an agentic asset, then ask OS Agent as a Service to start a run through progressive capabilities. If the deployed OS does not expose a compatible operation yet, the TUI opens the OS asset view instead. |
| `/agent deploy` | Publish or update the active local agent as an application asset, sync agent config, read the latest asset source revision, trigger the OS application-agent build, and launch it into the selected/default Runtime namespace when package and namespace metadata are available. Otherwise the OS asset view opens for the missing input. |
| `/agent open [agentic\|application\|tool]` / `/agent logs [agentic\|application\|tool]` | Observe the existing OS asset or Runtime log view for the active local agent without creating or uploading it; progressive ViewLinks are preferred when available. |
| `/agent status [agentic\|application\|tool]` | Check whether the active local agent has a matching OS asset, valid config/runtime binding, and service-specific binding without creating, uploading, running, or deploying anything. |
| `/mcp` | Select a local MCP server asset from `mcp_dir` with keyboard, wheel, or click, then enter local MCP-development mode. The TUI shows the active MCP asset; press Esc or run `/mcp off` to return to normal mode. |
| `/mcp <description>` | Draft a local MCP server asset with metadata prepared for OS Function as a Service. |
| `/mcp clone <git-url>` | Clone an existing MCP asset source into `mcp_dir`, then use `/mcp` to select it. |
| `/mcp list [query]` | Browse OS MCP assets through the asset-scoped list panel. |
| `/mcp activity [query]` | Inspect Runtime activity, jobs, and tool invocations for the selected MCP asset; when no MCP is active, A3S Code opens the MCP selection panel first. |
| `/mcp review` | Review the active local MCP asset. If no MCP is active, A3S Code opens the MCP selector first, enters MCP-development mode, then reviews the selected MCP asset. |
| `/mcp publish` | Publish the active local MCP asset as an OS `mcp` asset, commit source at the asset root plus `.a3s/asset.acl`, then sync the OS runtime-binding endpoint when available. |
| `/mcp deploy` | Publish the active MCP asset and sync its serving Function as a Service runtime binding. |
| `/mcp run` | Publish the active MCP asset, then run it through a real OS MCP runner capability discovered with progressive capabilities and `shaped=true`. If OS has not exposed that MCP runner capability, the command fails with a clear capability-gap message. |
| `/mcp test` | Publish the active MCP asset, then batch-test MCP tools through a real OS MCP test capability discovered with progressive capabilities and `shaped=true`. If OS has not exposed that MCP test capability, the command fails with a clear capability-gap message. |
| `/mcp open` / `/mcp logs` / `/mcp status` | Inspect the OS MCP asset, logs, or runtime-binding status without mutating the asset; open/logs prefer progressive Function as a Service ViewLinks when available. |
| `/flow` | Select a local workflow DAG JSON from `flow_dir` with keyboard, wheel, or click, publish it as an OS `workflow` asset with a manifest and Workflow as a Service runtime binding, sync the runtime-binding endpoint when available, and open the workflow designer through a progressive ViewLink or standalone designer fallback. |
| `/flow <description>` | Draft a local workflow DAG JSON, then use `/flow` to publish and iterate through OS Workflow as a Service. This is an OS asset workflow, not `DynamicWorkflowRuntime`. |
| `/flow clone <git-url>` | Clone an existing workflow asset source into `flow_dir`; workflow DAG source should live at the visible asset root as `flow.json`, with `.a3s/` reserved for `asset.acl` metadata. |
| `/flow list [query]` | Browse OS workflow assets through the asset-scoped list panel. |
| `/flow activity [query]` | Inspect Runtime activity and workflow runs for a selected workflow asset. |
| `/flow review [file]` | Review a local workflow DAG without publishing it. |
| `/flow publish` / `/flow run` / `/flow deploy` | Open the workflow selection panel, publish the selected DAG as an OS workflow asset, sync Workflow as a Service runtime-binding intent, then open the asset view or Workflow as a Service designer/run surface. |
| `/flow open` / `/flow logs` / `/flow status` | Open the existing OS workflow designer, Workflow as a Service logs, or runtime-binding status without mutating the selected workflow asset. |
| `/skill` | Select a local skill asset from `skill_dir` with keyboard, wheel, or click, then enter local multi-turn skill-development mode. The TUI shows the active skill; press Esc or run `/skill off` to return to normal mode. |
| `/skill <description>` | Draft a local skill asset prototype with `SKILL.md`, examples, tests, and Function as a Service binding intent. |
| `/skill clone <git-url>` | Clone an existing skill asset source into `skill_dir`, then use `/skill` to select it. |
| `/skill list [query]` | Browse OS skill assets through the asset-scoped list panel. |
| `/skill activity [query]` | Inspect related Function as a Service activity for the selected skill asset. |
| `/skill review` | Review the selected local skill asset. If no skill is active, A3S Code opens the skill selection panel first and enters skill-development mode. |
| `/skill publish` | Publish the selected skill as an OS `skill` asset backed by Function as a Service, committing source plus `.a3s/asset.acl`. |
| `/skill deploy` | Publish the selected skill, sync its serving Function as a Service runtime binding, then prefer an OS progressive shaped deployment ViewLink before falling back to the asset view. |
| `/skill open` / `/skill status` | Inspect the OS skill asset or runtime-binding status without mutating the asset; open prefers progressive Function as a Service ViewLinks when available. |
| `/kb` | Open the local personal knowledge base for notes, imports, search, and vault browsing. |
| `/kb add/import/search/vault` | Capture a note, preview/import files or folders, search local knowledge sources, or browse the local `.a3s/kb` vault. |
| `/okf` | Select a local OKF knowledge package from the visible `okf/` package root with keyboard, wheel, or click, then enter local package-development mode. The TUI shows the active package; press Esc or run `/okf off` to return to normal mode. |
| `/okf <description>` | Draft a local OKF package prototype with sources, wiki concepts, eval notes, and OS knowledge asset metadata. |
| `/okf clone <git-url>` | Clone an existing OKF package source into `okf/`, then use `/okf` to select it. |
| `/okf list [query]` | Browse OS knowledge package assets through the asset-scoped list panel. |
| `/okf activity [query]` | Inspect related Runtime indexing/evaluation activity for the selected knowledge package; when no package is active, A3S Code opens the OKF selection panel first. |
| `/okf review` | Review the selected local OKF package. If no package is active, A3S Code opens the OKF selection panel first and enters OKF-development mode. |
| `/okf publish` / `/okf deploy` | Publish the selected OKF package as an OS `knowledge` asset, sync Knowledge service runtime-binding intent, then deploy through progressive knowledge-service capabilities or open the Knowledge service view. Without OS, A3S Code performs local validation and reports blocked deployment inputs. |
| `/okf status` | Check the existing OS knowledge asset and runtime-binding status without mutating the selected package. |
| `? <question>` | Starts the evidence-first DeepResearch path described above. Exact-query bootstrap and one bounded semantic outline run concurrently. The planner may map at most 24 atomic user requirements to at most eight material tracks and propose at most 15 supplemental plain-text queries. Up to two later typed-gap rounds expand unresolved atomic criteria and share Host-owned totals of at most 24 new searches and 16 supplemental fetches. The Host also promotes at most three explicit query URLs as direct seeds, searches only work not already covered by bootstrap, and merges evidence under fixed transport budgets. Invalid planning falls back to the unchanged exact query and one generic track. Web and workspace text becomes claim evidence only through closed source/chunk IDs and exact provenance edges; publisher names, hosts, paths, source language, and query-token rules cannot promote it. The Host stages a `source_backed` snapshot before attempting one typed claim graph, retries a transient proposal failure at most once, and submits admitted work to an independent requirement, evidence, temporal, depth, and prose review. Only `synthesized` passes the complete quality gate. `qualified`, `source_backed`, and `no_evidence` retain inspectable artifacts but settle with incomplete/failure semantics. Replay reuses completed durable effects and cannot promote a source snapshot without the matching closed publication receipt. |
| `/loop` | Opens the engineered-loop dashboard for persisted loops under `.a3s/loops/`. |
| `/loop init [name] [pattern]` | Creates a durable loop spec, `STATE.md`, `RUN_LOG.md`, budget file, skills, and reports folder. Built-in patterns include `daily-triage`, `ci-sweeper`, `pr-babysitter`, `dependency-sweeper`, `changelog-drafter`, and `agent-dev`. |
| `/loop run <name>` | Runs a loop with maker/checker separation. With OS signed in and `os_runtime = true`, normal workspace loops require Runtime/parallel fan-out, Markdown/HTML reports, RemoteUI report view data, and asset-scoped Runtime activity visibility. Inside `/agent` mode, the same command stays local and targets the active agent package. |
| `/loop schedule <name> [15m\|2h\|1d]` | Validate and enable a local unattended cadence for an audited L1 report-only loop, then start the workspace singleton worker. Completion is reported in the TUI and the durable CLI notification inbox. |
| `/loop unschedule <name>` / `/loop schedules` | Disable recurring execution without deleting history, or inspect all persisted schedules and worker state. |
| `/loop audit <name>` / `/loop logs <name>` | Check loop readiness or open the append-only run log. |
| `/loop <task>` | Runs an autonomous quick loop until the task reports completion or you stop it. |

## Account Models

In `a3s code`, `/model` lists configured `config.acl` models plus signed-in
account tabs. When Claude Code is logged in (`claude /login`), the Claude Code
tab can switch the current session to Claude models using the local Claude Code
OAuth credentials, including Claude Code's macOS Keychain entry.
`CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_AUTH_TOKEN` can also provide the account
token for non-standard environments. If Anthropic rejects the raw OAuth Messages
API bridge with a rate-limit or authentication error, a3s falls back to the
installed `claude` CLI in safe streaming mode; Claude Code's own tools stay
disabled while a3s host tools are requested through an adapter protocol and
still execute inside a3s-code. The adapter accepts Claude Code-style
`<function_calls>` output and tool names such as `Read` or `Bash`, normalizes
common argument aliases like `path` to a3s's `file_path`, and feeds tool results
back into the next Claude turn as structured history.

When Codex CLI is logged in (`codex login`), the Codex tab can switch the
current session to Codex account models using `$CODEX_HOME/auth.json` or
`~/.codex/auth.json`.

For local account paths, native Windows uses `%USERPROFILE%` (or
`%HOMEDRIVE%%HOMEPATH%`) when the `HOME` environment variable is absent. This
applies consistently to Claude Code, Codex, Kimi Code, and WorkBuddy state;
Kimi Desktop continues to use its `%APPDATA%` Daimon state.

Codex account requests start with the native Responses WebSocket transport.
HTTP 403/426 during the upgrade switches the same request immediately to HTTPS
SSE; other connection failures receive two bounded WebSocket retries before the
same fallback. HTTPS retries only transient statuses, and the selected fallback
remains sticky across turns in that session while a new or child session probes
WebSocket again. A stream interruption replays the turn over HTTPS without
committing its provisional deltas. Both transports honor `HTTP_PROXY`,
`HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`, macOS system/PAC proxy settings, platform
root certificates, and `CODEX_CA_CERTIFICATE` (with `SSL_CERT_FILE` as the
fallback). Only allowlisted Cloudflare infrastructure cookies are shared between
the transports. On HTTP 401, A3S first reloads a token rotated by Codex CLI and
then performs one OAuth refresh and one request retry if necessary.

When Kimi Desktop or Kimi Code is signed in, the Kimi tab exposes the models
enabled for that account as `kimi/<model>`. A3S prefers Kimi Desktop's local
Daimon model configuration and coding API key, then falls back to Kimi Code
OAuth state under `~/.kimi-code` or `~/.kimi`. Credentials are read only when
needed to authenticate requests; they are never copied into A3S configuration,
command output, or logs. Expired OAuth state is refreshed under a file lock and
atomically rotated in Kimi's own credential store. Responses use Kimi's coding
endpoint while tool calls continue through native A3S host tools.

When WorkBuddy is installed and signed in, the WorkBuddy tab locates its
bundled CodeBuddy CLI (or `codebuddy`/`cbc` on `PATH`), reuses the account state
under `~/.workbuddy`, and refreshes the models entitled to that account. A3S
does not read or copy WorkBuddy tokens. `A3S_CODEBUDDY_CLI` can select a
non-standard CLI installation. WorkBuddy's streamed tagged tool calls are
normalized into native A3S host-tool events so execution remains inside A3S.

The A3S Web task Composer consumes the same account-model discovery and client
routing as the TUI. Account-backed entries are source-qualified as
`claude-code/<model>`, `codex/<model>`, `kimi/<model>`, or
`workbuddy/<model>` and never expose local credentials to the browser.

Codex auth can also be used as a normal config provider:

```acl
default_model = "codex/model-slug"

providers "codex" {
  models "model-slug" {
    name = "Codex model"
    toolCall = true
  }
}
```
The Codex tab refreshes the model
catalog through `codex debug models` and exposes every picker-visible model
available to that ChatGPT account. This includes GPT-5.6 Sol, Terra, and Luna
when the account is entitled to them; internal hidden entries are not shown.
The catalog's context windows and Responses Lite transport metadata are applied
when switching models, and `$CODEX_HOME` is honored for Codex auth and cache
files. Its native reasoning-effort metadata also drives `/effort`: A3S sends the
resolved level as `reasoning.effort`, clamps unsupported requests downward, and
normalizes the product-only `ultra` label to the Responses wire value `max`.
The selected profile's host-side budgets and orchestration remain active. If
live refresh is unavailable, the last local catalog remains usable.

When the WorkBuddy desktop app is installed and signed in, the WorkBuddy tab
uses the app's bundled CodeBuddy CLI and `~/.workbuddy` account state. A3S does
not read, copy, persist, or log WorkBuddy's private tokens. Opening the tab
refreshes the model ids currently enabled for the account; `a3s model list` and
`a3s code models` use the same discovery path. The app bundle is detected
automatically on macOS. Windows discovery checks standard per-user and Program
Files locations plus registered uninstall metadata, so custom installation
directories remain discoverable. Installed `codebuddy` and `cbc` commands are
supported on `PATH`, and `A3S_CODEBUDDY_CLI` can select a non-standard
installation.

Claude Code and WorkBuddy share the account-CLI stream and A3S host-tool bridge.
Their own CLI tools are disabled, provider tool-call output is normalized into
native A3S tool-use events, and tool results return as structured conversation
history. Codex and Kimi keep direct account transports but use the same account
provider registry for availability, model selection, persistence, and restore.

## Testing

```sh
cargo test --all-targets
cargo test --test box_command_soak -- --ignored
cargo build --manifest-path ../box/src/Cargo.toml -p a3s-box-cli --bin a3s-box
A3S_BOX_E2E_BIN=../box/src/target/debug/a3s-box cargo test --test compose_acl_e2e -- --ignored --nocapture
# From the monorepo root; isolates Cargo output from concurrent component builds.
just use-hotplug-e2e
cargo test --test remote_registry_components
A3S_USE_E2E_BIN=../use/target/debug/a3s-use \
  cargo test --test remote_registry_components \
  full_stack_registry_install_and_upgrade_activate_only_reviewed_targets \
  -- --ignored --nocapture
cargo test --test ctx_compact_real_llm -- --ignored   # hits the configured LLM
A3S_REAL_LLM_GUARDRAIL_MODEL=codex/gpt-5.6-terra \
  cargo test --test host_guardrail_real_llm -- --ignored --nocapture
A3S_TEST_WORKBUDDY_REAL=1 cargo test real_workbuddy_account_completes_an_a3s_tool_round
A3S_TEST_KIMI_REAL=1 cargo test --bin a3s real_kimi_account_completes_an_a3s_tool_round
```

The ignored soak test repeats `a3s box` after a fake first-use install and
verifies later runs reuse the installed `a3s-box`. The ignored
`compose_acl_e2e` test crosses the real `a3s` and `a3s-box` process boundary,
checks canonical ACL discovery over conflicting YAML, environment resolution,
closed-schema rejection, convergent `up`, `ps`, `logs`, `exec`, `down`, and
post-shutdown storage cleanup against a real MicroVM runtime. The ignored
Use hot-plug E2E builds the independently released `a3s-use` binary in an
isolated target directory, then crosses its public process/JSON boundary to
verify installation, MCP invocation, version replacement, TUI session replay,
disable, and re-enable convergence. It also packages the real binaries and
Browser, Office, and OCR Skills in release layout, drives Code's first-use
download and verified install path, and proves those three routes are present
in the first model turn without exposing raw Use MCP tools. The ignored
`ctx_compact_real_llm` test drives the configured model (`~/.a3s/config.acl`)
with matched compressed and uncompressed seeded histories. It asserts that
streaming usage is reported, compaction shrinks the history, the provider sees
a smaller prompt than the uncompressed baseline, and the reduction survives a
session restore — the machinery behind the TUI's bottom status indicator, fill
warnings, and auto-compaction.
The ignored `host_guardrail_real_llm` test launches the current `a3s` binary
against a real configured model. It first probes the native sandbox, then
verifies that Default and Auto execute `pwd` only when that boundary is ready,
that their missing-sandbox behavior asks or denies respectively, that explicit
host escalation enters approval without creating its target, that `.env`
cannot be read, and that Plan does not expose Bash to the execution turn. Set
`A3S_REAL_LLM_GUARDRAIL_MODEL` to override the configured default model.

## Updating

Update one component at a time:

```sh
a3s update          # update Code; compatibility alias for `a3s update code`
a3s update code     # update the main a3s executable and included Code component
a3s update box      # update an installed Box component
a3s update bench    # update an installed Bench control component
```

The accepted form is `a3s update [code|box|bench]`. Omitting the component keeps
the established self-update behavior and selects `code`. There is no implicit
"update everything" mode: an explicit component update cannot unexpectedly
download or replace either of the other components.

`a3s update box` and `a3s update bench` require that component to be installed.
If it is missing, the command stops with the corresponding `a3s install box` or
`a3s install bench` instruction. Use `a3s list` before an update when a script
needs to distinguish "not installed" from "already up to date". Normal use of
`a3s box ...` or `a3s bench ...` remains the simplest way to install a missing
optional component lazily.

For Code, `a3s code update` and the TUI's **`/update`** remain aliases of the
Code update. The TUI saves the current session, upgrades the main executable,
and restarts into that session. Neither form updates Box or Bench.

Homebrew-managed Code installations refresh the A3S tap, upgrade or reinstall
`a3s-lab/tap/a3s`, and verify both `PATH` and the Homebrew prefix binary.
Standalone Code installations download the matching GitHub release archive,
find the `a3s` binary inside it, swap the current binary, and verify the target
version before treating the update as successful. If restart fails after a
successful upgrade, the TUI prints the exact `a3s code resume <id>` command for
the saved session.

Transition note: standalone installations from 0.9.9 through 0.10.10 already
read releases from `A3S-Lab/CLI`. Versions 0.11.0 and 0.11.1 briefly used the
monorepo release endpoint; `A3S-Lab/a3s` retains a verified compatibility relay
so those clients can perform one update to a CLI-owned release. Current
archives retain the inert, fail-closed marker required by older clients'
retired managed-SRT archive check and separately carry the current verified
sandbox support tree. Current discovery explicitly ignores the compatibility
marker. Homebrew upgrades are unaffected.

Box retains its complete runtime bundle during installation and update. Bench
downloads into a staging area, verifies the release checksum, component
manifest, target, required files, and CLI protocol, and only then activates the
new version under `~/.a3s/components/bench/`. A failed Bench update leaves the
previous active control component available.

If you're on an **older build (≤ 0.5.4)** whose `/update` was broken, it can't
upgrade itself, and `brew upgrade a3s` alone won't see the new version (Homebrew
doesn't re-sync a tap on `upgrade`). Bootstrap onto a current build once with:

```sh
brew update && brew upgrade a3s     # or: brew untap a3s-lab/tap && brew tap a3s-lab/tap && brew upgrade a3s
a3s --version
```

From 0.5.5 onward, `/update` handles the tap refresh itself, so this manual step
isn't needed again.

## License

MIT
