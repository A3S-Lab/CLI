# a3s

The umbrella CLI for the [A3S](https://github.com/A3S-Lab) platform.

`a3s <tool> [args...]` runs the matching A3S tool. `a3s box ...` proxies to
`a3s-box ...` and bootstraps the Box runtime automatically if it is missing:

```
a3s code            # launch the A3S Code TUI
a3s box ps          # → a3s-box ps (auto-installs a3s-box if needed)
a3s <tool> --help   # a tool's own help
a3s list            # list installed a3s-* tools
a3s --version
```

## Install

```sh
# from source
cargo install a3s

# or from this repo
cargo install --git https://github.com/A3S-Lab/Cli

# or Homebrew
brew install A3S-Lab/tap/a3s
```

Then run the tools you need. `a3s box ...` installs `a3s-box` on first use.
The Homebrew `a3s` formula installs the native RemoteUI helper
`a3s-webview` automatically on macOS; if a source/cargo install is missing it,
`a3s code` falls back immediately to printing the browser URL.

## A3S Code TUI

`a3s code` launches the interactive A3S Code terminal UI in the current
workspace. On first launch it creates `~/.a3s/config.acl`; use `/config` to edit
models, provider credentials, and optional paths such as `repo_dir`, `flow_dir`,
and `agent_dir`.

The TUI is a full coding workspace, not just a chat window:

| Area | Capability |
| --- | --- |
| Coding loop | Chat with the coding agent, approve tools, switch `/auto`, run shell turns with `!`, set `/goal`, tune `/effort`, and compact long sessions with `/compact`. |
| Workspace UI | `/ide` opens a superfile-style file tree and editor, `/git` provides status/diff/stage/commit flows, and `/output` shows the raw tool-call log. |
| Models | `/model` switches configured models and signed-in account tabs, including Claude Code account models when available. |
| Context | The status bar tracks context fill; auto-compaction keeps long sessions usable. `/ctx` searches past sessions and `/memory` browses durable memories. |
| Knowledge base | `/kb` opens a dashboard for `.a3s/kb`: add notes, import files or folders, search sources, open the vault, and compile concepts. |
| Review | `/review` runs a read-only repo review checklist. `& <git-url>` clones a repo into the configured repos folder and reviews it. |
| Process view | `/top` shows live local agent/container/process activity so long-running work stays inspectable. |
| Session utilities | `/help` shows the full command guide, `/theme` changes code highlighting, `/workflow` opens the latest dynamic workflow, `/sleep` consolidates the day into memory, and `/plugin` + `/reload` manage skills/plugins. |

### OS, Runtime, and RemoteUI

Add an OS endpoint to `config.acl`, then sign in:

```hcl
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
| `/list` | Browse and manage OS digital assets such as agents, workflows, and apps. |
| `/ps` | Browse deployed Runtime services/jobs, search them, and stop/cancel supported rows. |
| `/im` | Open OS chat for direct messages and groups as a standalone TUI surface. |
| `/view` | Reopen the latest OS RemoteUI view captured from a tool response. |
| `/run` | Pick a project from `repo_dir`, start it in dev mode on A3S Runtime, and auto-open the live RemoteUI run view. |
| `/deploy` | Pick a project from `repo_dir`, run Agentic CI/CD, deploy through the OS gateway, and auto-open the live CI/CD RemoteUI view. |
| `/flow` | Pick a local workflow DAG JSON and open it in the OS workflow designer; `/flow <description>` drafts a new DAG first. |

RemoteUI views are captured from OS progressive responses (`.view`/`viewUrl`).
The TUI remembers the latest view and, for `/run` and `/deploy`, opens the first
live Runtime/CI view automatically.

### Agents, Research, and Loops

| Command | What it does |
| --- | --- |
| `/agent` | Pick a local agent definition from `agent_dir` and enter local multi-turn agent-development mode. The TUI shows the active agent; press Esc or run `/agent off` to return to normal mode. While active, `/goal` becomes an agent-scoped development goal and `/loop` runs local agent-scoped loop engineering. No OS WebIDE or RemoteUI is opened for this local VibeCoding flow. |
| `/agent <description>` | Draft a Markdown agent definition with YAML frontmatter under `agent_dir`, then use `/agent` to iterate on it. |
| `? <question>` | Starts DeepResearch. When OS is signed in, it should split research across A3S Runtime workers, then create Markdown and HTML reports and surface a RemoteUI view. Without OS, it falls back to local research artifacts under `.a3s/research/`. |
| `/loop` | Opens the engineered-loop dashboard for persisted loops under `.a3s/loops/`. |
| `/loop init [name] [pattern]` | Creates a durable loop spec, `STATE.md`, `RUN_LOG.md`, budget file, skills, and reports folder. Built-in patterns include `daily-triage`, `ci-sweeper`, `pr-babysitter`, `dependency-sweeper`, `changelog-drafter`, and `agent-dev`. |
| `/loop run <name>` | Runs a loop with maker/checker separation. With OS signed in, normal workspace loops use A3S Runtime parallelism, Markdown/HTML reports, RemoteUI view data, and `/ps` visibility. Inside `/agent` mode, the same command stays local and targets the active agent definition. |
| `/loop audit <name>` / `/loop logs <name>` | Check loop readiness or open the append-only run log. |
| `/loop <task>` | Keeps the legacy quick-loop behavior: auto-continue a task until it reports completion or you stop it. |

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

## Testing

```sh
cargo test --all-targets
cargo test --test box_command_soak -- --ignored
cargo test --test ctx_compact_real_llm -- --ignored   # hits the configured LLM
```

The ignored soak test repeats `a3s box` after a fake first-use install and
verifies later runs reuse the installed `a3s-box`. The ignored
`ctx_compact_real_llm` test drives the configured model (`~/.a3s/config.acl`)
until the context crosses the auto-compact threshold and asserts streaming
usage is reported, compaction shrinks the history, and the next prompt drops —
the machinery behind the TUI's ctx%, fill warnings, and auto-compaction.

## Updating

In the TUI, **`/update`** upgrades to the latest release and restarts into your
session. Homebrew installs refresh the A3S tap, upgrade or reinstall
`a3s-lab/tap/a3s`, and verify both `PATH` and the Homebrew prefix binary.
Standalone installs download the matching GitHub release archive, find the
`a3s` binary inside it, swap the current binary, and verify the target version
before treating the update as successful. If restart fails after a successful
upgrade, the TUI prints the exact `a3s code resume <id>` command for the saved
session.

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
