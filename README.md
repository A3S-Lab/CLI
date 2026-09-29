<p align="center">
  <img
    src="assets/readme/hero.svg"
    width="100%"
    alt="A3S CLI runs the coding agent from the terminal"
  />
</p>


<p align="center">
  <strong>Language / 语言:</strong>
  <a href="README.md">English</a> ·
  <a href="README.zh-CN.md">中文</a>
</p>

<p align="center">
  <strong>Build with agents in the terminal.</strong>
</p>

<p align="center">
  <a href="https://github.com/A3S-Lab/CLI/actions/workflows/ci.yml"><img src="https://github.com/A3S-Lab/CLI/actions/workflows/ci.yml/badge.svg" alt="CI status" /></a>
  <a href="https://crates.io/crates/a3s"><img src="https://img.shields.io/crates/v/a3s.svg" alt="Crates.io version" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-f0b65b.svg" alt="MIT license" /></a>
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#a3s-code">A3S Code</a> ·
  <a href="#component-lifecycle">Components</a> ·
  <a href="#development">Development</a>
</p>

> [!IMPORTANT]
> **A3S 0.21.0 — September 29, 2026.** This repository is the canonical CLI
> surface for A3S Code. Interactive `a3s code` launches the A3S Code TUI built
> from a3s-code 9.1.1 (`a3s-code-tui` and `a3s-code-acp`, rev
> `e2c42e92bf871ac501cf83416fd335a61f8c6416`). The published `a3s`
> crate pins `a3s-code-core` `=9.1.1` (git rev
> `e2c42e92bf871ac501cf83416fd335a61f8c6416`) for `a3s code exec`. That Core line uses
> pure-Rust a3s-vec lexical FTS, treats non-empty `web_search` rows as success,
> and loads project instructions from `AGENTS.md` rather than an AgentDir
> `serve` layout. Host isolation, sandbox diagnosis, and worktree path handling
> ship with that pin. Extensions run through `a3s use`. `a3s install` places
> registered components only. `a3s plugin` is not a command. Unavailable
> providers fail closed.

## One CLI, one Code host

`a3s` is the umbrella command for the A3S developer platform. A base install
contains A3S Code. Other native products and A3S Use capabilities keep their
own release and lifecycle boundaries.

```text
a3s
├── code        interactive pager, or non-interactive exec
├── use         Browser, Office, OCR, and installed Use capabilities
├── compose     multi-service applications delegated to A3S Box
└── components  install · upgrade · inspect · repair · uninstall
```

| Entry point | Current role |
| --- | --- |
| `a3s code` | External Code pager, `a3s code exec`, durable sessions, and Core memory. |
| `a3s use …` | Delegate Browser, Office, OCR, Box, and extension capabilities to A3S Use. |
| `a3s install …` | Manage registered A3S products and delegated Use packages; it is not a universal OS package manager. |

## Quick start

The product entry is the **`a3s` binary**. Interactive Code is `a3s code`, not
a separate `a3s-code` install. Pick one channel and keep updates on that
channel.

### Install

```bash
# macOS or glibc Linux (x86_64 / aarch64) — official installer
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/A3S-Lab/CLI/main/install.sh \
  | A3S_MODIFY_PATH=1 sh
```

```powershell
# Windows x64 — PowerShell 5.1 or newer
$env:A3S_MODIFY_PATH = '1'
irm https://raw.githubusercontent.com/A3S-Lab/CLI/main/install.ps1 | iex
```

```bash
# macOS or Linux with Homebrew (preferred package-manager path)
brew tap a3s-lab/tap https://github.com/A3S-Lab/homebrew-tap
brew install a3s
# Equivalent: brew install a3s-lab/tap/a3s

# Any host with a Rust toolchain (CLI crate only; companions may be incomplete)
cargo install a3s --locked
```

Do **not** run `brew install a3s-code` for the umbrella CLI. That formula is
the legacy standalone `a3s-code` binary and does not provide `a3s`.

The installers compare the two official release repositories during the
current migration, select the newer stable SemVer, verify the GitHub-published
SHA-256, reject unsafe archive members, validate `a3s --version`, and activate
the binary, bundled target-specific Moli runtime, and `a3s-webview`
companion as one recoverable operation. They never use `sudo` or UAC. Omit
`A3S_MODIFY_PATH=1` to leave shell profiles and the user PATH unchanged.
Homebrew installs the same companions from the CLI archive, so a separate
`a3s-webview` formula is not required.

Default locations: `~/.local/bin` (Unix installer), Homebrew `bin/` (brew),
`%LOCALAPPDATA%\Programs\a3s\bin` (Windows). Overrides: `A3S_VERSION`,
`A3S_INSTALL_DIR`, `A3S_GITHUB_TOKEN`.

Supported release targets today: macOS 12+ (`aarch64` / `x86_64`), glibc Linux
(`x86_64` / `aarch64`), Windows x64. Not shipped: musl/Alpine, Windows ARM,
Mingw, Cygwin.

Intel macOS 12 is supported by the standalone installer above. The native
macOS sandbox uses the operating system's built-in Seatbelt boundary and does
not require Node.js or an npm runtime.

### Update

```bash
# Standalone macOS / Linux installer channel
a3s self update

# Homebrew
brew update && brew upgrade a3s

# Cargo
cargo install a3s --locked
```

```powershell
# Windows — re-run the installer (in-place self-update is not supported)
irm https://raw.githubusercontent.com/A3S-Lab/CLI/main/install.ps1 | iex
```

### Uninstall

```bash
# Homebrew
brew uninstall a3s

# Standalone Unix installer (no uninstaller script)
rm -f ~/.local/bin/a3s ~/.local/bin/a3s-webview
rm -rf ~/.local/bin/moli
# Remove any PATH line added when A3S_MODIFY_PATH=1 was used.

# Cargo
cargo uninstall a3s
```

```powershell
# Windows installer channel
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\Programs\a3s"
# Remove that directory from the user PATH if it was added.
```

Uninstalling the binary does not delete `~/.a3s/` (or the Windows equivalent).

### First session

```bash
a3s --version
a3s code
```

The first `a3s code` launch creates `~/.a3s/config.acl` when no configuration
exists. Inspect the selected ACL configuration with:

```bash
a3s config path
a3s config show
a3s config validate
```

Prepare A3S Use explicitly when first-use installation is not appropriate:

```bash
a3s install use --source release
a3s use doctor --json
a3s use capabilities --json
```

`--offline` and `A3S_NO_AUTO_INSTALL=1` are strict no-download boundaries.
Monorepo-level install docs live in the [a3s README Installation
section](https://github.com/A3S-Lab/a3s#installation).

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

`state/use/registries.acl` is the only Registry source document used by
`a3s registry`. Every mutation uses revision CAS. `a3s install` does not
select a Registry source and does not place cognitive packages. The official
production Registry root is not yet public, so these commands currently
require a source the operator deliberately trusts.

## Architecture

Interactive `a3s code` launches the external pager (`a3s-code-tui` and `a3s-code-acp`). `a3s code exec` runs in this process. `a3s box`, `a3s search`, `a3s bench`, and `a3s use` proxy to their registered products. Component install covers the registered catalog only: `code`, `box`, `bench`, `search`, `use`, `use/browser`, `use/office`, `use/ocr`, and `webview`.

## A3S Code

`a3s code` is an agentic developer workspace, not only a chat prompt. It keeps
conversation, tool execution, approvals, workspace changes, memory, and
verification evidence in one semantic transcript.

| Area | Product surface |
| --- | --- |
| Coding | Streaming agent loop, workspace tools, bounded image file/clipboard input, saved-file Code Intelligence, bounded diffs, and Live Preview. |
| Retrieval | Exact, glob, and incremental BM25 search stay with the coding agent. The CLI does not attach semantic workspace retrieval. |
| Control | Default, read-only Plan, and non-interactive Auto modes with exact grants, cancellable work, and closed automation tool profiles. |
| Continuity | Durable sessions, resume, priority-queued follow-ups, context search, memory, compaction, isolated worktree forks with digest-bound patch handoff, and conflict-checked rewind. |
| Assets | `$` Skills remain for exec. `a3s code kb`, context browse, and the `a3s plugin` command were removed. |
| Models | ACL-configured `provider/model` routes, plus signed-in A3S OS gateway routes. Borrowed Claude Code, Codex, Kimi, and WorkBuddy logins are not model routes. |
| Integrations | VS Code and compatible editors commands for bounded editor context and diff review, plus a permissioned repository-native GitHub Action. |

Everyday commands:

```bash
a3s code
a3s code resume
a3s code exec --mode auto "Fix the focused test and verify it"
a3s code exec --mode plan --tool-policy read-only "Review this workspace"
a3s code exec --web-search enabled "Compare the current published guidance"
a3s code exec --mode auto --tool-policy local-workspace --model provider/model "Fix this offline task"
a3s code exec --image before.png,after.png "Compare these screenshots"
a3s code sandbox status
a3s code sandbox setup  # Probes the native platform boundary
```

Each `a3s code exec` run auto-saves its transcript in the same workspace-scoped
session store used by `a3s code`, `a3s code resume`, and `a3s code session`.

See [Code editor and CI integrations](docs/code-integrations.md) for extension
installation, Action usage, exact permission profiles, and the deliberately
closed automation boundary.

Semantic workspace retrieval was removed from `a3s code exec` and from
`a3s config show`. An ACL `workspace_retrieval` block is ignored. Exact, glob,
and incremental BM25 search stay with the coding agent.

### Local command sandbox

`a3s code exec` and `a3s code sandbox` use the verified local process sandbox.
Default and Auto run ordinary Bash calls inside that boundary; Plan exposes no
Bash. An explicit `require_escalated` request never escapes silently: Default
asks for the exact host command and Auto denies it. If sandbox preparation or
its native capability probe fails, Bash is denied in every mode. Catastrophic
commands and credential or control paths remain hard denials in every case.

The sandbox is implemented in Rust and invokes only the native OS isolation
boundary: Seatbelt on macOS, user/PID/network namespaces plus seccomp on Linux,
and AppContainer plus a kill-on-close Job Object on Windows. There is no Node.js,
npm package, sidecar runtime, or one-time elevated setup. Linux requires
`bubblewrap` and usable unprivileged user namespaces; macOS and Windows need no
additional sandbox package. The CLI probes the real OS boundary before use.
`code exec` probes the boundary before the run and does not fall back to an unsandboxed process.

For unattended repository work that must retain full local coding capability
without public network access, `code exec --mode auto --tool-policy
local-workspace` exposes workspace reads, Code Intelligence, bounded edits,
structured local Git, and governed batch, program, task, workflow, and Skill
execution. Host Web/download/Runtime/Knowledge/managed-Tool/MCP entry points and
unknown dynamic tools stay hidden and denied. Bash appears only after the native sandbox
probe succeeds, cannot escalate to the host, and uses the sandbox's empty
network allowlist; delegated and Skill runs inherit the same boundary. The
structured Git tool has no fetch, push, pull, or clone operation.

The sandbox denies network egress and local listeners, limits writes to the
workspace and a private scratch directory, protects repository/control
metadata, hides common credential stores and nested `.env*` files, scrubs the
ambient environment, and rejects credential hard-link aliases. Delegated and
Skill child runs inherit the same frozen sandbox and permission snapshot.

## Component lifecycle

The base installation contains the umbrella CLI and A3S Code. Optional products
remain separately released:

| Component | Included | Public route | Lifecycle |
| --- | --- | --- | --- |
| Code | Yes | `a3s code` | Runs from the umbrella executable and uses the native sandbox compiled into A3S Code Core. |
| Box | No | `a3s box`, `a3s compose` | Visible first-use install or explicit preparation. |
| Bench | No | `a3s bench` | Explicit install; a compatible public control-component release remains a gate. |
| Search | No | `a3s search` | Explicit component install; embedded Code search and browser engines retain separate lifecycles. |
| Use | No | `a3s use`, `a3s code` | Explicit install; asynchronous TUI first-use preparation when policy allows; required Desktop one-shot setup; ordinary Code Exec performs installed-only discovery without mutation. |
| Moli | Release-dependent | default Code web-search headless backend | Bundled per-target runtime; source/Cargo installs use one digest-verified shared cache with cross-process locking. |
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
| Default | Bounded reads and writes follow workspace policy. | Ordinary commands use the verified sandbox; explicit host escalation enters review and missing-sandbox execution is denied. | Exact allow-once, session, or project grants. |
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
a3s model use openai/my-model
a3s model use openai/my-model --scope workspace
a3s auth list
a3s auth login os
```

`a3s auth` manages the A3S OS session. Model selection reads ACL providers and,
when signed in, OS gateway models. Prefixes `claude-code/`, `codex/`, `kimi/`,
`workbuddy/`, and `codebuddy/` are rejected. An ACL provider that uses one of
those names is selected as `config/<provider>/<model>`.

## Platform support

| Platform | Current guarantee |
| --- | --- |
| macOS arm64 / x86_64 | Primary Code, component, Use, Moli, and native WebView release target; local command isolation uses `sandbox-exec`. |
| Linux arm64 / x86_64 | Primary Code, component, Use, Moli, and headless runtime release target; local command isolation uses bubblewrap and user namespaces. |
| WSL | Uses the Linux runtime and filesystem contract. |
| Windows x86_64 | Preview: native A3S Code with bundled Moli, WebView, and AppContainer/Job Object local isolation exists without a separate setup step. Complete Browser, six-surface, and failure-injection parity remains a gate. |


## Development

Work in this repository directly or through the A3S monorepo's pinned
`crates/cli` submodule. Do not create a Rust workspace at the monorepo root.

The lockfile pins published crates to exact versions and the composable Code,
Flow, Memory, and Search companions to immutable git revisions. Release
preflight verifies every revision and every platform Moli archive before an
archive or crate is published.

```bash
cargo fmt --all -- --check
cargo test --lib
cargo test --tests
cargo clippy --all-targets --all-features -- -D warnings
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
- [A3S Use website](https://a3s-lab.github.io/Use/)
- [A3S Use package contracts](https://github.com/A3S-Lab/Use/tree/main/docs)

## Updating

```bash
a3s self update --check
a3s self update
a3s upgrade use
```

`a3s update` remains a hidden compatibility route to `a3s self update`.
Component upgrades preserve their owning provenance.

## License

A3S CLI is licensed under the [MIT License](LICENSE). Release archives retain
the licenses and provenance notices of their bundled components.
