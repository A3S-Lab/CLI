# A3S CLI Product Design

- Status: Accepted; incremental migration in progress
- Date: 2026-07-15
- Scope: Umbrella `a3s` command surface
- Related: [Technical Architecture](cli-technical-architecture.md),
  [Migration Plan](cli-migration-plan.md),
  [Component Management](component-management-design.md)

## 1. Decision

The `a3s` executable will become one coherent product CLI rather than a manual
router around independently evolved commands. The redesign uses these rules:

- keep frequent component lifecycle verbs at the top level;
- group secondary administration by noun, then verb;
- reserve explicit top-level namespaces for bundled products and trusted
  component proxies;
- use one global interaction and output contract for root-owned commands;
- keep aliases only for compatibility, never as competing documented forms;
- use A3S ACL for human-authored configuration and manifests;
- use ordinary CLI process contracts, standard MCP, and Skills instead of a
  custom JSON-RPC protocol.

The principal naming correction is:

```text
a3s self update       # update the a3s executable
a3s upgrade <id>      # upgrade an installed component
```

`update` will no longer have two meanings.

## 2. Scope

This design covers every command parsed or owned by the umbrella CLI,
including non-interactive A3S Code commands. It defines the proxy boundary for
Box, Bench, Search, and Use, but their internal command trees remain owned by
their respective executables. Interactive `/commands` inside the Code TUI are
not shell commands and are outside this design.

This document is the accepted target product contract. The implementation
baseline and remaining gaps are tracked in the migration plan; a target
contract must not be presented as implemented until its acceptance gate passes.

## 3. Product Principles

1. **Predictable grammar.** Commands use `noun verb` for administration and a
   small set of conventional top-level verbs for frequent package operations.
2. **One canonical spelling.** Full words are documented; abbreviations are
   compatibility aliases only.
3. **Safe reads by default.** Listing, inspection, planning, and diagnosis do
   not mutate state. Network access is explicit or evident from the command.
4. **Explicit mutation.** Destructive or provenance-changing work exposes a
   plan, confirmation policy, and dry-run behavior.
5. **Automation is a first-class client.** Machine output, exit status,
   non-interactive behavior, and stream formats are stable contracts.
6. **Human output may improve.** Scripts must request JSON or JSONL instead of
   scraping tables, colors, progress bars, or prose.
7. **No surprise execution.** Unknown commands do not discover and execute an
   arbitrary `a3s-*` binary from `PATH`.
8. **Secrets are not arguments.** Credentials never use positional CLI
   arguments and are redacted from output and diagnostics.
9. **Ownership controls deletion.** A3S mutates only receipt-owned files or
   delegates to the package manager or trusted parent that owns them.
10. **Cross-platform means declared support.** Unsupported targets fail before
    mutation; A3S is not a universal frontend for arbitrary operating-system
    packages.

These principles follow current patterns in the uv, GitHub, Docker, kubectl,
and Winget CLIs and the cross-vendor guidance at clig.dev.

## 4. Target Command Tree

```text
a3s
├── code                         interactive coding agent
│   ├── exec                     non-interactive coding task
│   ├── resume                   resume the newest or selected pager session
│   ├── sandbox                  native sandbox status and probe
│   ├── hooks                    trusted lifecycle hooks
│   └── session                  list, show, export, or delete sessions
├── box                          transparent a3s-box proxy
├── compose                      transparent a3s-box Compose namespace
├── up / down / ps / logs        frequent Compose workflow shortcuts
├── bench                        transparent a3s-bench proxy
├── search                       transparent a3s-search proxy
├── use                          transparent a3s-use proxy
├── auth                         account login, logout, and status
├── model                        model discovery and selection
├── config                       A3S ACL configuration
├── list                         component inventory
├── info                         component details and sources
├── install                      component installation or repair
├── upgrade                      component upgrade planning and execution
├── uninstall                    ownership-safe component removal
├── doctor                       read-only diagnostics
├── registry                     trusted component registries
├── cache                        download and derived-data caches
├── self update                  update the a3s executable
├── completion                   generate shell completion
├── version                      print version information
└── help                         command help
```

No-argument `a3s` prints concise help. It does not implicitly launch Code or
perform an update.

Command groups print help when their verb is omitted. The documented
exceptions are action commands with an intentional no-argument behavior:
`code` launches the external pager, `list` lists components,
`install` lists available components, and `upgrade` lists available upgrades.
No other group guesses a default verb.

## 5. Global Contract

Root-owned commands share these options:

```text
-h, --help
-V, --version
-C, --directory <path>
    --config <path>
    --output human|json|jsonl
    --json
-q, --quiet
-v, --verbose
    --color auto|always|never
    --no-progress
    --offline
    --non-interactive
```

`--json` is a stable shorthand for `--output json`. JSONL is accepted only by
event-producing commands such as `a3s code exec`.
An unsupported output mode is a usage error rather than a silent fallback.

Mutation-specific options are not global:

```text
--dry-run        resolve and display a plan without applying it
--yes            accept the displayed plan, but not extra trust or privilege
--force          repair within current ownership and provenance
```

`--force` never implies `--yes`, unsigned trust, source migration, privilege
elevation, or deletion of user data.

Human results go to stdout. Progress, warnings, and diagnostics go to stderr.
JSON mode writes exactly one versioned document to stdout. JSONL writes one
versioned event per line. Prompts, spinners, colors, and decorations never
appear in machine output. `NO_COLOR` and non-TTY output are honored.

JSON and JSONL imply non-interactive behavior. Human mode prompts only when the
required terminal streams are TTYs; otherwise a missing decision fails with an
actionable error and the flag needed to continue.

## 6. A3S Code

### 6.1 Interactive, Execution, and Sessions

```text
a3s code
a3s code exec [<prompt>] [--prompt-file <path>] [-i|--image <path>]... [--mode plan|default|auto] [--tool-policy standard|read-only|workspace-write|local-workspace] [--web-search auto|enabled|disabled]
a3s code resume [session-id]
a3s code session list
a3s code session show <session-id>
a3s code session export <session-id> [--output-file <path>]
a3s code session delete <session-id> [--yes]
```

`code` with no subcommand launches the external Code pager in the effective directory.
`code exec` is the explicit automation surface; it accepts one prompt argument,
a prompt file, piped stdin, or an image-only turn. Repeated `-i/--image` flags
and comma-separated paths preserve input order and use the same bounded,
content-based image validation.
Arbitrary trailing text after `a3s code` is never guessed to be a prompt. It
emits a final result in JSON or an event stream in JSONL. Any approval that
cannot be resolved in non-interactive mode fails
instead of blocking on hidden input. `auto` uses the shared risk classifier to
approve bounded workspace operations; high-risk or unknown operations still
fail with `approval.required`. A successful result requires a terminal Code
completion event rather than merely a closed event stream.

`--tool-policy standard` preserves the ordinary execution surface. The closed
`read-only` profile exposes only bounded native workspace reads and search.
`workspace-write` requires `--mode auto` and adds only native
write, edit, and patch operations. Both closed profiles hide and deny process,
Git, task, runtime, plug-in, MCP, Knowledge, and download tools, deny
unknown future tools by default, and reject control metadata such as `.git`,
`.a3s`, `.vscode`, and `.gitmodules`. They are the required profiles for the
first-party editor and GitHub Action integrations.

`--web-search auto|enabled|disabled` controls the governed `web_search` and
`web_fetch` network-read tools independently from mode and workspace tool
policy. Closed profiles keep them hidden under `auto`; `enabled` admits only
those two reads, while `disabled` hides and denies both for the complete run.

`local-workspace` is a different closed profile for unattended repository work
that needs the product's local agentic coding surface. It requires `--mode auto`; retains workspace reads, Code Intelligence,
bounded edits, structured local Git, and governed batch/program/task/workflow/
Skill execution; denies Web by default; and always denies download, Runtime,
Knowledge, managed Tool, MCP, and unknown tools. Explicit `--web-search enabled`
admits only the governed search/fetch pair. Bash is visible only with a successfully probed
A3S native sandbox, cannot escalate, and runs under its empty network allowlist. Nested runs
inherit the same live checker and sandbox. This profile is not selected by the
first-party editor or GitHub Action integrations.

`code resume` remains canonical because it is a frequent user action. The
`session` group owns less frequent inspection and data lifecycle operations.
Deleting a session never deletes workspace files or memory.


### 6.2 Research

`a3s code research`, including the `deepresearch` and `deep-research` aliases, was removed. Interactive work uses the Code pager. One-shot automation uses `a3s code exec`.

### 6.3 Asset Families

The Code TUI/CLI five-pack asset authoring surfaces
(`a3s code agent|mcp|skill|flow|okf` and matching `/…` slash commands) were
removed. Publish, deploy, and OS activity for those families belong on Desktop
/ the OS control plane. `$` skill discovery remains for `a3s code exec`.
The in-process `/kb`, `/evolution`, and Use MCP panels were removed with the
in-process TUI.

### 6.4 Knowledge, Context, and Memory

`a3s code kb`, `a3s code context` (`ctx`), and `a3s code memory` (`mem`) were removed. `a3s code exec` still writes Core memory under the workspace memory directory (`.a3s/memory` by default, or `A3S_MEMORY_DIR` / the ACL `memory_dir`). Browse that directory, or use the Code pager.

## 7. Monitor

`a3s top` was removed. Inspect local processes with the host shell, and inspect
containers with `a3s box`.

## 8. Authentication, Models, and Configuration

### 8.1 Authentication

```text
a3s auth list
a3s auth status [provider]
a3s auth login [provider] [--token-stdin|--token-file <path>]
a3s auth logout [provider]
```

The managed provider is `os`. Browser OAuth is the default OS login. Tokens are
accepted only from protected stdin, an explicitly selected credential file, or
the platform credential store. A token is never accepted as a positional
argument.

### 8.2 Models

```text
a3s model list
a3s model current
a3s model use <provider/model> [--scope workspace|user]
a3s model reset [--scope workspace|user]
```

Model discovery distinguishes runtime-callable models from digital assets
whose category is `model`. `model use` validates the target before atomically
updating the selected ACL configuration layer. TUI `/model` selection remains
session state and does not silently rewrite product configuration.

### 8.3 Configuration

```text
a3s config path
a3s config paths
a3s config show
a3s config init [--scope workspace|user] [--force]
a3s config edit [--scope workspace|user]
a3s config validate [path]
```

`config show` prints the effective, redacted configuration; it is not a raw
secret dump. `config validate` parses with `a3s-acl` and reports source
locations. `config paths` reports configuration, data, state, cache, asset,
memory, KB, and OKF roots.

Human-authored configuration is A3S Agent Configuration Language in `.acl`
files. ACL is not HCL, and no HCL parser or HCL terminology is used.

The effective precedence is:

1. command flags;
2. typed `A3S_*` environment overrides;
3. an explicit `--config` or `A3S_CONFIG_FILE`, when present;
4. workspace `.a3s/config.acl` over the user ACL configuration;
5. built-in defaults.

When an explicit config path is present, it replaces the normal workspace/user
file stack so execution is reproducible.

## 9. Component Lifecycle

The high-frequency lifecycle commands remain top-level:

```text
a3s list [--installed|--available|--updates] [--kind <kind>]
a3s info <component> [--versions] [--sources]

a3s install <component>...
    [--version <requirement>]
    [--channel stable|beta|nightly]
    [--source auto|<source-id>]
    [--scope user|system]
    [--dry-run] [--offline] [--migrate] [--force]
    [--yes]

a3s upgrade
a3s upgrade <component>... [--dry-run] [--offline] [--yes]
a3s upgrade --all [--dry-run] [--offline] [--yes]

a3s uninstall <component>...
    [--cascade] [--purge] [--dry-run] [--yes]

a3s doctor [component]
```

No-argument `install` lists available components without mutation.
No-argument `upgrade` lists available upgrades without mutation. `upgrade
--all` is required to mutate every eligible component. Missing components are
not silently installed by `upgrade`.

`doctor` is read-only and returns a failing exit status when required health
checks fail. It may suggest an exact install, repair, authentication, or config
command, but it does not accept a hidden `--fix` mutation mode.

`a3s install` manages registered A3S components and delegated capabilities. It
does not accept arbitrary Homebrew, Winget, APT, DNF, Pacman, language-package,
operating-system package names, or `use/<publisher>/<name>` cognitive packages.
Extensions run through `a3s use`. Supported native managers are trusted
backends for declared component sources; they retain ownership of their files.

Source selection uses the trusted catalog. It retains existing provenance
first, then honors an explicit source, then applies target-compatible source
priority. Candidate sources may be a managed signed artifact, a declared
native package-manager identity, or a trusted parent such as Use for
`use/browser`, `use/office`, and `use/ocr`.

## 10. Registries, Cache, and Self Management

```text
a3s registry list
a3s registry show <name>
a3s registry add <name> <url> --root-sha256 <digest> [--trusted-root <file>] [--yes]
a3s registry replace <name> <url> --root-sha256 <digest> [--trusted-root <file>] --revision <digest> [--yes]
a3s registry default <name> --revision <digest> [--yes]
a3s registry enable <name> --revision <digest> [--yes]
a3s registry disable <name> --revision <digest> [--yes]
a3s registry remove <name> --revision <digest> [--yes]
a3s registry refresh [name]

a3s cache path
a3s cache status
a3s cache prune [--dry-run]
a3s cache clean [--dry-run] [--yes]

a3s self update [--check] [--dry-run] [--yes]
a3s version [--verbose]
a3s completion bash|zsh|fish|powershell|elvish
a3s help [command...]
```

No Registry trust root is implicit. Adding any Registry source is an explicit
trust operation; HTTPS alone is not a trust root. `state/use/registries.acl` is
the single source of truth across CLI, TUI, Marketplace, plan, and apply;
all source-state mutations use revision CAS. Registry
configuration is ACL. Signed transport metadata and machine receipts may use
versioned JSON because they are generated machine state, not product config.

`cache prune` removes only unreferenced or expired entries. `cache clean`
removes all recreatable cache content, never configuration, receipts, sessions,
documents, browser profiles, or report artifacts.

Self-update preserves installation provenance. A Homebrew-owned CLI delegates
to Homebrew; a managed release updates through verified artifacts and atomic
activation. It never silently migrates provenance.

## 11. Product Proxies

These are explicit registered namespaces:

```text
a3s box <args...>       -> a3s-box
a3s compose <args...>   -> a3s-box compose
a3s up <args...>        -> a3s-box compose up
a3s down <args...>      -> a3s-box compose down
a3s ps <args...>        -> a3s-box compose ps
a3s logs <args...>      -> a3s-box compose logs
a3s bench <args...>     -> a3s-bench
a3s search <args...>    -> a3s-search
a3s use <args...>       -> a3s-use
```

The root resolves an absolute executable, forwards arguments and streams
without a shell, and preserves child status. Box and Use may retain visible,
catalog-authorized first-use installation. Bench and Search require explicit
installation unless their catalog policy changes. `--offline` always disables
first-use network mutation.

Compose and its shortcuts resolve the same registered Box component; they do
not create another executable identity or orchestration implementation. Only
these registered routes may proxy. An unregistered `a3s-foo` on
`PATH` can appear in the external section of `a3s list`, but `a3s foo` never
executes it.

Use owns the Browser and Office domains and externally installed domain
routes. Their native CLI, standard MCP, and Skill surfaces stay native. The
umbrella CLI does not translate them through a universal JSON action API or a
custom JSON-RPC service.

Root `--output` is not silently translated into a child-specific flag. Until a
compatible first-party child explicitly negotiates the versioned CLI context,
non-human root output is rejected for proxies. A child-native output flag placed
after `box`, `bench`, `search`, or `use` is forwarded unchanged.

The Search proxy is only a user entry point. `a3s-search` embeds the typed
`a3s-use-browser` library for rendering; it does not shell out to `a3s use` or
require an A3S Use service.

## 12. Migration and Acceptance

The complete current-to-target command map, security exceptions, release
milestones, verification matrix, and final acceptance criteria live in the
[A3S CLI Migration and Verification Plan](cli-migration-plan.md). Canonical
forms must land before compatible aliases are removed, and ordinary aliases
remain for at least two minor releases.
