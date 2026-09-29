# a3s

The umbrella CLI for the [A3S](https://github.com/A3S-Lab) platform.

`a3s <tool> [args...]` runs the matching A3S product. Code is included in the
main `a3s` installation. Box, Bench, Search, and Use are separately released
components. Box and Use may be installed visibly on first real use; every
optional component can also be prepared explicitly.

```
a3s code                       # launch the included A3S Code TUI
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
`a3s-search` library. In the **Code CLI product profile** (`scientific`, which
includes `headless-search`), Moli is the default `web_search` headless backend
and is discovered from the package or downloaded once into the shared Code
cache on first headless use (`auto_download_moli`). Core's crate `default`
feature (`local-code`) does **not** enable headless Moli — SDK embeds must opt
into `headless-search` / `scientific` for the same contract. Chrome and
Lightpanda remain explicit compatibility backends:

```bash
a3s search engines
a3s search doctor
a3s search browser install chrome
a3s search browser update chrome
a3s search browser repair lightpanda
```

The bundled Code Core 8.4.0 runtime uses a structurally gated cascade for
`web_search`: the Moli headless tier runs first, followed by HTTP/RSS and
native API fallbacks while retrieval requirements remain unmet. Configure the
`search.engine` entries in `config.acl` to replace the built-in selection,
including an explicit `anysearch { enabled = true }` entry when desired.
AnySearch can use `ANYSEARCH_API_KEY` when set. A failure or empty result from
any selected engine enters the same bounded fallback policy; quota exhaustion
and other provider failures remain visible in structured search metadata.

Moli is the default JavaScript-capable backend. Release archives bundle the
target-specific executable beside the CLI (`bin/moli/moli`). `a3s code` passes
that path to the Code agent as `A3S_CODE_MOLI_EXECUTABLE`. The agent also
finds the same sidecar by resolving the `a3s` executable on `PATH`, including
Homebrew symlinks into the Cellar, so it does not download a second copy.
Source and Cargo installs that omit the sidecar still resolve the pinned,
digest-verified runtime from the shared per-user cache on first use. The cache is process-safe and shared by local Code processes, so
multiple `a3s` installations do not download duplicate copies. Chrome and
Lightpanda remain explicit compatibility backends. `a3s search doctor` reads
the same project-local or user-global `config.acl` selected by `a3s code`,
reports the active backend and cache status, and returns an actionable install
or repair command when a configured backend is unavailable.

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
not download Box, Bench, Search, or Use. Published release archives also carry
the target-specific Moli sidecar and the native `a3s-webview` companion, so
the default Code search path and RemoteUI are immediately self-contained.
Source and Cargo installations retain lazy, digest-verified provisioning for
those companions, while every product still has one public entry point under
`a3s`.

Official installers and Homebrew install `a3s-webview` next to `a3s` from the
same release archive. It owns RemoteUI windows. If a source or Cargo
installation does not have that helper, RemoteUI falls back immediately to the
browser; this does not prevent the TUI from starting.

### Components and delayed installation

The built-in catalog registers `code`, `box`, `bench`, `search`, and `use`.
Browser, native Office, and OCR are capabilities owned by Use:

| Component | Installed with `a3s` | Public command | Installation behavior |
| --- | --- | --- | --- |
| Code | Yes | `a3s code ...` | Runs directly from the main `a3s` installation. |
| WebView | Yes | `a3s-webview` (companion) | Ships beside `a3s` in official installers and Homebrew; owns native RemoteUI windows. |
| Box | No | `a3s box ...` | Installs Box on first use, then forwards the arguments to it. |
| Bench | No | `a3s bench ...` | Requires an explicit compatible Bench installation. |
| Search | No | `a3s search ...` | Requires an explicit compatible Search installation. |
| Use | No | `a3s use ...` / `a3s code` | Installs Use on first real use, including TUI startup when policy allows, then forwards or projects native Browser, Office, OCR, Box, or extension surfaces. |
| Use/Browser | With Use | `a3s use browser ...` | Reports Browser provider readiness through Use; it is not a second product archive. |
| Use/Office | With Use | `a3s use office ...` | Projects the built-in native `a3s-office` CLI, Skill, and MCP (Word/Excel/PPT/Markdown/PDF); install with `a3s install use/office` when missing. |
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
Moli is a Code-owned runtime rather than a separately registered component:
release archives carry it beside the CLI, while source/Cargo installs use the
shared digest-verified cache and a cross-process lock. There is therefore no
`a3s install moli` command and no per-project browser copy.
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
```

`a3s install` and `a3s upgrade` do not place `use/<publisher>/<name>` cognitive
packages. Extensions run through `a3s use`. Registered components, including
delegated `use/browser`, `use/office`, and `use/ocr`, stay on `a3s install`.

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

`a3s registry` configures named sources. It does not install a package.
Registry URLs must use HTTPS, except loopback HTTP used by the hermetic test
suite. `a3s upgrade --all` upgrades managed catalog products. It does not
upgrade a signed Registry extension reported by `a3s use`.

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
and SHA-256 selected during resolution. Presentation text is deliberately
excluded.

A release dry-run reads release metadata and, when necessary, its checksum
file. It never downloads the component payload or changes component state. An
apply with `--plan-digest` resolves the plan again and fails with
`component.plan_mismatch` before payload download or mutation when anything
covered by the digest changed. Once accepted, the installer consumes the exact
resolved artifact from that plan instead of performing another `latest`
lookup.

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
```

Browser, native Office, and OCR are built-in Use domains. Independently
implemented domains can be explicitly installed from A3S ACL packages that
declare native CLI, standard MCP, and/or `SKILL.md` surfaces. A3S does not
define an extension JSON-RPC protocol; `--json` remains a one-command CLI
result.

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

### Platform support

macOS and Linux remain the broad runtime and managed-artifact targets for the
component platform. Windows x86_64 now supports the native WebView managed
release, bundled Moli runtime, and Code first-use installation. A real-process Windows E2E additionally
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

## A3S Code

`a3s code` launches the external Code pager (`a3s-code-tui` with `a3s-code-acp`) in the current workspace. `--worktree` creates an isolated Git worktree under `~/.a3s/worktrees` and starts the pager there. Machine output is rejected on this interactive path; use `a3s code exec`.

Start and resume the pager:

```sh
a3s code                         # launch the pager in the current workspace
a3s code --worktree              # isolated ~/.a3s/worktrees checkout, then the pager
a3s code --worktree feature-x    # same with an explicit branch/path identity
a3s code resume                  # resume the newest saved session here
a3s code resume 018f-session-id  # resume a specific saved session
a3s self update                  # update the a3s executable
```

Run one non-interactive coding task:

```sh
a3s code exec --mode auto "Update the focused test and verify it"
a3s code exec --mode plan --tool-policy read-only "Review this workspace"
a3s code exec --mode auto --tool-policy workspace-write "Apply the requested source edits"
a3s code exec --force "Ship the change without confirmation prompts"
a3s code exec --yolo --tool-policy workspace-write "Apply edits without HITL pauses"
a3s code exec --web-search enabled "Compare the current published guidance"
a3s code exec --mode auto --tool-policy local-workspace --model provider/model "Fix this offline task"
a3s code exec --image before.png,after.png "Compare these screenshots"
a3s --output json code exec --mode auto --prompt-file ./task.md
```

`--force` (alias `--yolo`) matches force mode: tool calls that would normally
`Ask` are allowed automatically. Critical rule denials, protected paths,
catastrophic shell, and leaving the verified sandbox stay denied. It is
incompatible with `--mode plan`. Write-capable closed tool policies accept
`--force`/`--yolo` as an alternative to `--mode auto`.

Ordinary `code exec` performs installed-only A3S Use discovery. It never
downloads Use or mutates component state. `--capability-runtime scoped-v1`
was removed, and results no longer include `capabilityRuntime`.

Now cargo fmt and cargo check. Check will take a while. I'll also look at whether exec JSON still has fields the reference mentions. The summary said JSON dropped workspaceRetrieval and capabilityRuntime. Good.

Also check if tests mention capabilityRuntime and will fail.

Auto mode runs bounded workspace reads and edits without hidden prompts while
retaining the shared safety floor. Operations that still require human
approval, such as unbounded shell commands, terminate immediately in this
non-interactive surface with a nonzero `approval.required` result. Default and
plan modes never silently approve workspace mutations.

`--tool-policy read-only` and `workspace-write` are closed automation profiles.
They do not expose shell, Git, delegated tasks, runtime or package execution,
MCP, download, or Knowledge tools; any unknown future tool is denied. Web
search/fetch stays hidden by default and is admitted only when the caller adds
`--web-search enabled`.
`workspace-write` is valid with `--mode auto` or `--force`/`--yolo` and adds bounded native file
write/edit/patch operations while rejecting repository and agent control
metadata. Successful JSON and JSONL results echo the effective `toolPolicy` and
`webSearch` preference so an automation host can verify both boundaries.

`local-workspace` is a separate deny-by-default profile for unattended local
coding. It requires `--mode auto` or `--force`/`--yolo` and retains bounded
workspace reads and edits, Code Intelligence, structured local Git, and
governed batch, program, task, parallel-task, dynamic-workflow, and Skill
execution. Web search/fetch remains denied unless `--web-search enabled`
explicitly admits those two governed network-read tools; download, Runtime,
Knowledge, managed Tool, MCP, and unknown dynamic tools remain hidden and denied. Bash is exposed only after
the A3S-owned native sandbox passes its capability probe, cannot request host
escalation, and runs with the sandbox's empty network allowlist. Nested task and
Skill runs inherit the same live checker and sandbox. The structured Git tool
does not implement fetch, push, pull, or clone.

`--web-search auto|enabled|disabled` is independent from planning mode and
workspace tool policy. `auto` preserves the selected policy's legacy behavior,
`enabled` exposes governed `web_search` and `web_fetch` reads, and `disabled`
hides and denies both for the entire run even when task wording requests the
Web. This flag does not enable download, arbitrary HTTP tools, Runtime, MCP, or
shell networking.

`code exec -i/--image` accepts repeated flags and comma-separated paths. PNG,
JPEG, GIF, and WebP inputs are detected from their bytes, decoded under bounded
resource limits, and retained in argument order. An image-only turn is valid.
Configured models must set `attachment = true` or include `"image"` in
`modalities.input`. Signed-in `a3s-os/<model>` routes accept images.
`a3s code exec` uses an ACL `provider/model`.
Inspect and create `config.acl`:

```sh
a3s config path                       # print the active config path
a3s config init                       # create the user config
a3s config init --scope workspace     # create .a3s/config.acl
a3s config show                       # print a redacted effective summary
a3s config validate                   # validate the effective A3S ACL
a3s config edit --scope workspace     # open VISUAL/EDITOR, or print the path
a3s config paths                      # print config, asset, and memory paths
```

### Workspace semantic retrieval

Semantic workspace retrieval was removed from `a3s code exec` and from
`a3s config show` / `validate`. An ACL `workspace_retrieval` block is ignored.
Exact, glob, and incremental BM25 search stay with the coding agent.
`a3s code exec` attaches manifest-backed workspace services and does not build
an embedding index.

Sign in to A3S OS and check account state:

```sh
a3s auth login os              # open the configured OS OAuth login flow
printf '%s' "$A3S_OS_TOKEN" | a3s auth login os --token-stdin
a3s auth status os             # show OS endpoint, account, and expiry
a3s auth logout os             # remove the stored OS session
```

List the managed A3S OS account:

```sh
a3s auth list
```

`a3s auth login` and `a3s auth logout` manage the configured A3S OS session.

Model routes are ACL `provider/model` values, plus signed-in `a3s-os/<model>`
gateway models:

```sh
a3s model list
a3s model current
a3s model use openai/my-model
a3s model use a3s-os/my-model
a3s model use openai/gpt-5 --scope workspace
a3s model reset --scope user
```

`a3s model use` validates the route and updates `default_model` in the selected
A3S ACL layer while preserving unrelated ACL text. `--config` always targets
that explicit file; otherwise `--scope user|workspace` selects the layer. No OS
token is copied. Prefixes `claude-code/`, `codex/`, `kimi/`, `workbuddy/`, and
`codebuddy/` are rejected. An ACL provider that uses one of those names is
selected as `config/<provider>/<model>`.

The model commands list `config.acl` models and signed-in OS gateway models.
These runtime routes are not digital asset repository entries whose category
happens to be `model`.

Asset authoring commands (`a3s code agent`, `mcp`, `skill`, `flow`, and `okf`) were removed. Publish those families from Desktop or the OS control plane. `a3s code exec` still discovers skills from `A3S_SKILL_DIR` or `~/.a3s/skills`, plus project skill roots.

`a3s code kb`, `a3s code context` (`ctx`), `a3s code memory` (`mem`), and
`a3s code research` were removed. `a3s code exec` still stores Core memory in
the workspace memory directory. Browse it from the Code pager or on disk.
There is no separate `a3s code view` command.

## Account Models

Borrowed Claude Code, Codex, Kimi, and WorkBuddy logins are not model routes.
`a3s model`, `a3s auth list`, and `a3s code exec` do not read those credential
files. Configure the provider in `config.acl` and select `provider/model`.
When the ACL provider name is `claude-code`, `codex`, `kimi`, `workbuddy`, or
`codebuddy`, select `config/<provider>/<model>`. Signed-in A3S OS gateway
models remain `a3s-os/<model>`.

An ACL provider named `codex` is selected with the `config/` prefix:

```acl
default_model = "config/codex/model-slug"

providers "codex" {
  models "model-slug" {
    name = "Codex model"
    toolCall = true
  }
}
```

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
A3S_REAL_LLM_GUARDRAIL_MODEL=openai/gpt-5 \
  cargo test --test host_guardrail_real_llm -- --ignored --nocapture
```

The ignored soak test repeats `a3s box` after a fake first-use install and
verifies later runs reuse the installed `a3s-box`. The ignored
`compose_acl_e2e` test crosses the real `a3s` and `a3s-box` process boundary,
checks canonical ACL discovery over conflicting YAML, environment resolution,
closed-schema rejection, convergent `up`, `ps`, `logs`, `exec`, `down`, and
post-shutdown storage cleanup against a real MicroVM runtime. The ignored
Use hot-plug E2E builds the independently released `a3s-use` binary in an
isolated target directory, then crosses its public process/JSON boundary to
verify installation, MCP invocation, version replacement, disable, and
re-enable convergence. It also packages the real binaries and Browser, Office,
and OCR Skills in release layout. The ignored
`ctx_compact_real_llm` test drives the configured model (`~/.a3s/config.acl`)
with matched compressed and uncompressed seeded histories. It asserts that
streaming usage is reported, compaction shrinks the history, the provider sees
a smaller prompt than the uncompressed baseline, and the reduction survives a
session restore.
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

`a3s self update` upgrades the main executable. The TUI's **`/update`** saves
the current session, upgrades that executable, and restarts into the session.
Neither form updates Box or Bench.

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
archives contain neither the retired sandbox payload nor its inert
compatibility marker. Clients that still require that retired archive layout
must upgrade once through the current standalone installer or Homebrew.

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
