# Code Intelligence

A3S Code exposes one native Code Intelligence runtime to the agent, the TUI
workspace editor, and A3S Web. It provides saved-file symbols, semantic
navigation, and diagnostics while leaving source reads, text search, and edits
to the existing workspace tools.

## Prerequisites

The first release supports Rust and TypeScript/JavaScript. Install the language
executables that match the workspace before starting `a3s code` or `a3s web`:

```sh
rustup component add rust-analyzer
npm install --global typescript typescript-language-server
```

After a query first tries to start them, missing executables appear as an
unavailable or degraded language in status. They do not disable the editor,
file tools, or a working language profile.

## Saved-File Model

All results use the contents currently saved on disk. Unsaved editor buffers
are never published to the shared runtime. Both the TUI and Web show
"saved version" when an open buffer is dirty, and saving causes the shared
workspace manifest to refresh the runtime.

Lines and characters in the HTTP and Core contracts are zero-based. Characters
count UTF-16 code units. User interfaces convert only their one-based display
coordinates; they do not reinterpret protocol columns as UTF-8 bytes or Unicode
scalar counts.

## Runtime Isolation And Bounds

Each canonical workspace and stable project-layout hash acquires one lazy native
runtime generation. The provider reuses the existing workspace manifest change
stream instead of starting a second file watcher or text index. Rust owns the
language-server child process, framed stdio, capability negotiation, shutdown,
and cancellation; neither the TUI nor Web starts language processes directly.

Queries have a 15-second deadline. Protocol headers are capped at 8 KiB and
JSON-RPC bodies at 16 MiB. One workspace generation tracks at most 256 saved
documents and 512 diagnostic entries. Document outlines are capped at 2,000
symbols; navigation and workspace symbol results at 1,000. Workspace diagnostics
inspect at most 128 saved documents with concurrency eight and return at most
2,000 entries. A failed language profile degrades only that language, retries
after a bounded delay, and does not disable the editor or working profiles.

The runtime snapshots saved content before each semantic query and checks it
again on completion. If the file changed in between, the result remains usable
as explicitly stale evidence instead of being presented as current. Canonical
workspace paths, symlink confinement, typed capability checks, and bounded
result metadata are enforced before a location reaches either UI.

The TUI additionally limits workspace-symbol queries to 256 characters, result
titles to 240 characters, visible rows to 1,200 characters, and result lists to
2,000 rows. Hierarchical outlines are projected iteratively with at most 32
visible depth levels. Every language-server label, status, error, path, symbol,
detail, container, diagnostic source/code, and diagnostic message passes through
the shared ANSI/OSC/C1 and bidirectional-control sanitizer before terminal
rendering. Sanitization changes only visible labels; the typed workspace path
and UTF-16 position used by Enter remain intact and are revalidated by the
workspace resolver.

A read-only query that returns a typed protocol failure receives one delayed,
cancellation-aware retry inside the original 15-second absolute deadline. The
adapter does not inspect error prose, retry other failure categories, or loop
indefinitely. This covers the short content-update race observed from a real
language server immediately after document open.

## TUI `/ide`

Launch `a3s code`, open `/ide`, select a source file, press `:`, and enter one
of these commands:

| Command | Result |
| --- | --- |
| `:status` | Runtime state, language profiles, and negotiated capabilities |
| `:symbols` | Hierarchical symbols in the open saved file |
| `:symbols <query>` | Bounded workspace symbol search |
| `:definition` | Definitions at the saved-file cursor position |
| `:declaration` | Declarations at the saved-file cursor position |
| `:references` | References at the saved-file cursor position |
| `:implementations` | Implementations at the saved-file cursor position |
| `:diagnostics` | Diagnostics for the open saved file |
| `:diagnostics workspace` | Bounded diagnostics across saved workspace files |

Results replace the editor side of `/ide` temporarily. Use Up/Down or `j`/`k`
to select, Page Up/Page Down to move by a page, Home/End or `g`/`G` to move to
an edge, Enter to open a location, and Escape to close the result list. A jump
never discards an unsaved buffer.

Queries run asynchronously and are cancelled when replaced or closed. TUI
update and render paths do not start language processes or read semantic files.

The test suite covers fake-server framing and lifecycle behavior, terminal
control injection, oversized and deeply nested result data, UTF-16 positions,
stale requests, dirty-buffer navigation, workspace containment, cancellation,
and the typed protocol retry. An opt-in real saved-workspace smoke exercises
status, document and workspace symbols, definition navigation, diagnostics, and
graceful shutdown through the TUI query adapter when `rust-analyzer` is
installed; it runs outside the parallel unit suite so unrelated process-fixture
tests cannot alter its child-process environment.

## A3S Web and Monaco

The Monaco editor uses the same workspace provider:

- document symbols are available through Monaco's Go to Symbol in Editor;
- F12 opens a definition, Shift+F12 opens references, and Cmd/Ctrl+F12 opens
  implementations;
- declaration and all navigation operations are also available from the editor
  context menu;
- saved-file diagnostics are rendered as Monaco markers;
- the status bar reports startup, degraded/unavailable state, stale evidence,
  diagnostic count, and dirty-buffer saved-version behavior.

Navigation reuses the existing workspace file-selection flow. Workspace text
search remains the existing search panel; Code Intelligence does not add a
second search or replace UI.

## Read-Only HTTP API

The local Web service exposes typed routes under
`/api/v1/workspace/code-intelligence`:

| Route | Purpose |
| --- | --- |
| `GET /status` | Aggregate and per-language status |
| `GET /outline?path=...` | Document symbol hierarchy |
| `GET /symbols?query=...&limit=...` | Bounded workspace symbols |
| `GET /navigation?path=...&line=...&character=...&kind=...` | Semantic locations |
| `GET /diagnostics?path=...` | Document or bounded workspace diagnostics |

An optional `sessionId` selects the workspace of an already loaded Web
session. Unknown sessions, absolute paths, traversal, and symlink escapes are
rejected. Omitting `sessionId` uses the canonical workspace served by the Web
instance.

These endpoints are local, read-only product APIs. They are not a second agent
tool protocol, and callers still use the existing workspace endpoints for file
contents and mutations.
