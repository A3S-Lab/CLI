# A3S CLI Technical Architecture

- Status: Accepted; incremental migration in progress
- Date: 2026-07-15
- Parent: [A3S CLI Product Design](cli-product-design.md)
- Delivery: [Migration and Verification Plan](cli-migration-plan.md)
- Related: [Cross-Platform Install Architecture](cross-platform-install-architecture.md)

## 1. Architecture Decision

The umbrella CLI will use one typed parse-dispatch-render pipeline. Command
handlers return typed outcomes and never own process termination or output
format selection. Product proxies remain process boundaries and receive raw
arguments, streams, execution context, and status without a shell.

```text
argv / environment / terminal facts
                |
                v
        clap parser and aliases
                |
                v
       InvocationContext builder
  config | output | policy | paths | cancellation
                |
                v
       typed command dispatcher
     /          |           |          \
  Code      components     services    proxy
     \          |           |          /
                v
          CommandOutcome
                |
                v
      human / JSON / JSONL renderer
                |
                v
        stdout, stderr, exit code
```

This is an in-process application architecture. It does not introduce a
daemon, universal action API, or custom JSON-RPC transport.

## 2. Current Problems to Remove

The current root and Code routers manually match strings. Help is incomplete;
`update` is overloaded; unknown Code words and some typos fall through; output,
prompting, and exit behavior vary by handler; global administration is nested
under Code; Web lacks lifecycle commands; and proxy arguments are forced
through UTF-8 `String`. The migration characterizes public behavior with
integration tests but does not preserve accidental fuzzy dispatch or the
documented-but-unrouted `a3s code view` form.

## 3. Parser and Command Types

Use Clap 4 derive APIs as a direct CLI dependency. The parser is the single
source of truth for help, usage, aliases, conflicts, value enums, completion,
and spelling suggestions.

Conceptually:

```rust
#[derive(clap::Parser)]
struct Cli {
    #[command(flatten)]
    global: GlobalOptions,
    #[command(subcommand)]
    command: Option<RootCommand>,
}

#[derive(clap::Subcommand)]
enum RootCommand {
    Code(CodeArgs),
    Web(WebArgs),
    Top(TopArgs),
    Box(ProxyArgs),
    Compose(ProxyArgs),
    Up(ProxyArgs),
    Down(ProxyArgs),
    Ps(ProxyArgs),
    Logs(ProxyArgs),
    Bench(ProxyArgs),
    Search(ProxyArgs),
    Use(ProxyArgs),
    Auth(AuthArgs),
    Model(ModelArgs),
    Config(ConfigArgs),
    List(ComponentListArgs),
    Info(ComponentInfoArgs),
    Install(InstallArgs),
    Upgrade(UpgradeArgs),
    Uninstall(UninstallArgs),
    Doctor(DoctorArgs),
    Registry(RegistryArgs),
    Cache(CacheArgs),
    Self_(SelfArgs),
    Completion(CompletionArgs),
    Version(VersionArgs),
    Help(HelpArgs),
}
```

Command value types use validated newtypes such as `ComponentId`, `SourceId`,
`ModelRef`, `SessionId`, `OutputMode`, and `InstallScope`. Strings are converted
at the parser boundary, not repeatedly inside handlers.

Deprecated forms use hidden Clap aliases or one explicit compatibility
normalizer before canonical parsing. They do not create duplicate handler
paths. The normalizer returns a canonical invocation plus structured warnings.
Aliases that cannot be expressed without ambiguity, such as the old dual-use
`update`, have narrowly scoped pre-parser rewrites with dedicated tests.

No `allow_external_subcommands` behavior is enabled at the root. Registered
proxy variants use a trailing raw argument field. Unknown root commands remain
usage errors.

## 4. Process Entry and Exit

The binary entry point returns `ExitCode`, passes `std::env::args_os()` to the
application runner, and contains no business logic. The runner performs:

1. compatibility normalization;
2. canonical parsing;
3. invocation-context construction;
4. cancellation registration;
5. dispatch;
6. one render pass;
7. exit classification.

Handlers return `Result<CommandOutcome, CliError>`. They do not call
`process::exit`, select a renderer, or print ad hoc JSON. Deep library code does
not know CLI exit codes.

Root-owned exit codes are deliberately small and stable:

| Code | Meaning |
| --- | --- |
| `0` | Requested operation completed successfully |
| `1` | Runtime, health, policy, authentication, or operation failure |
| `2` | Invalid command, argument, value, or usage |
| `3` | A multi-target operation completed with partial failure |
| `130` | Root-owned operation cancelled by Ctrl-C where the platform permits |

Machine-readable error codes carry detail such as `auth.required`,
`component.not_owned`, or `config.invalid`; adding such codes does not consume
new process exit codes. Proxy commands preserve the child exit code and signal
outcome as closely as the operating system permits.

## 5. Invocation Context

Each root-owned handler receives an immutable context:

```rust
struct InvocationContext {
    directory: CanonicalWorkspace,
    config: Arc<EffectiveConfig>,
    paths: Arc<A3sPaths>,
    output: OutputPolicy,
    interaction: InteractionPolicy,
    network: NetworkPolicy,
    terminal: TerminalCapabilities,
    cancellation: CancellationToken,
    diagnostics: Arc<dyn DiagnosticSink>,
}
```

The context is built once. Handlers do not independently rediscover config,
inspect TTY state, parse environment variables, or invent directory defaults.
Tests construct a context with isolated paths, deterministic terminal facts,
and in-memory output sinks.

The working directory is resolved before workspace configuration. Root-owned
commands pass paths explicitly rather than changing the global process current
directory. Proxies set the child current directory to the resolved directory.

The current migration checkpoint creates one token and one Ctrl-C listener at
the root. Code Exec, Top JSON/JSONL snapshot execution, and `web logs --follow`
consume that token. A cancelled machine stream writes its terminal error event
before the renderer returns exit `130`. Foreground Web shutdown and proxy
signal forwarding remain separate acceptance work and must not be inferred
from this checkpoint.

## 6. Configuration Architecture

### 6.1 One ACL Resolver

All human-authored product configuration and component or extension manifests
use A3S ACL and are parsed through `a3s-acl`. ACL is not HCL. The CLI must not
add an HCL parser, label ACL syntax as HCL, or use an HCL-specific intermediate
model.

The resolver implements this precedence:

```text
typed command flags
    > typed A3S environment overrides
    > explicit --config or A3S_CONFIG_FILE
    > workspace .a3s/config.acl
    > user config.acl
    > built-in defaults
```

An explicit config path selects a reproducible single file and disables the
normal workspace/user file merge. Otherwise, the workspace file is a typed
overlay on user configuration. Merge rules are defined per field; collections
are not accidentally concatenated or replaced through generic JSON merging.

The resolver returns provenance for each effective field so `config show`,
`config validate`, `model current`, and diagnostics can explain where a value
came from.

### 6.2 Writes

Configuration mutation goes through typed editors, never regex replacement.
Writes are validated before an atomic same-filesystem replacement and preserve
permissions. If the available ACL editor cannot preserve comments for a
section, the command must either use a section-aware core editor or refuse and
open `config edit`; it must not rewrite unrelated user content.

`config show` always redacts credentials and secret-derived values. Generated
installation receipts, journals, signed registry transport metadata, and CLI
JSON remain versioned machine JSON because they are not human product config.

## 7. Credentials and Sensitive Data

Credential ingestion is limited to:

- browser-based OAuth with a bounded callback;
- protected stdin selected by `--token-stdin`;
- an explicitly selected file whose permissions and type are validated;
- the platform credential store or a permission-restricted compatibility
  store.

Secrets are never accepted as positional values. The parser marks sensitive
options for redaction before diagnostics are created. Debug output records the
presence and source class of a credential, not its content, length, prefix, or
hash.

Credential material is not written to ACL, component receipts, transaction
journals, telemetry, shell history, URLs, or generated JSON. Child processes
receive the minimum scoped credential material needed for their operation.

## 8. Output and Error Model

### 8.1 Typed Outcomes

Handlers return semantic data:

```rust
enum CommandOutcome {
    Value(serde_json::Value),
    Table(TableModel),
    Text(TextArtifact),
    Stream(Pin<Box<dyn Stream<Item = Result<CliEvent, CliError>> + Send>>),
    Proxy(ProxyOutcome),
}

struct CliError {
    code: ErrorCode,
    message: String,
    suggestion: Option<String>,
    details: serde_json::Value,
    class: ExitClass,
    source: Option<anyhow::Error>,
}
```

Concrete Rust result types are preferred inside command modules; conversion to
`CommandOutcome` occurs at the presentation boundary. The value enum above is
conceptual and should not become a generic JSON business API.

### 8.2 JSON

One-shot machine output uses one envelope:

```json
{
  "schemaVersion": 1,
  "command": "component.list",
  "ok": true,
  "data": {},
  "warnings": []
}
```

Failures use the same envelope and a nonzero exit status:

```json
{
  "schemaVersion": 1,
  "command": "component.install",
  "ok": false,
  "error": {
    "code": "component.not_owned",
    "message": "Component 'search' is externally managed.",
    "suggestion": "Upgrade it with the package manager that installed it.",
    "details": {}
  },
  "warnings": []
}
```

Each command owns a versioned data schema. The common envelope does not imply
that unrelated commands share one untyped payload. Additive optional fields are
allowed within a schema version; removals, meaning changes, and type changes
require a new version.

Asset path fields remain JSON strings when the native path is valid UTF-8. A
native path that cannot be represented as a JSON string uses an object with
`display`, `encoding`, and lossless hexadecimal `value` fields. Current Unix
paths use `unix-bytes-hex`; Windows paths use `windows-wide-hex`. Path
resolution and process invocation always retain `PathBuf`/`OsString`; the
human display value is never reparsed as the path.

### 8.3 JSONL and Human Output

JSONL events include `schemaVersion`, `command`, `type`, a monotonic sequence,
and command-specific data. Final success or error is an explicit terminal
event. Lines are flushed individually. Truncated streams remain detectably
incomplete because they lack the terminal event.

Human renderers may use tables, Unicode, color, and TTY progress. They consume
the same typed result and never become the source for JSON. Data goes to stdout;
progress and diagnostics go to stderr. Broken pipes terminate quietly with the
conventional successful pipeline behavior where appropriate.

## 9. Interaction, Terminal, and Network Policy

`InteractionPolicy` is calculated centrally from output mode, explicit flags,
and terminal capabilities:

- JSON and JSONL are always non-interactive;
- `--non-interactive` disables every prompt;
- `--yes` answers only the plan confirmation associated with that command;
- missing trust, migration, elevation, or destructive-data consent still
  fails unless separately authorized;
- a prompt is allowed only when both its input and diagnostic output are safe
  TTYs;
- non-TTY progress is disabled rather than rendered as escape sequences.

`NetworkPolicy::Offline` prevents registry refresh, update checks, downloads,
OAuth browser login, and first-use installation before a request is sent. It
still permits local receipts, cached signed metadata, local packages, system
probes, and already installed proxies. `A3S_NO_AUTO_INSTALL=1` remains a
compatibility input and maps to the stricter first-use policy.

Color resolution is explicit flag, then `NO_COLOR`, then TTY capability. Child
processes receive compatible color and offline context without secrets.

## 10. Command Module Boundaries

A target layout keeps the boundary explicit without creating a monolithic
`cli.rs`:

```text
src/
├── main.rs
├── cli/                       args, compatibility, context, errors, renderers
├── commands/                  one orchestration module per root concern
├── components/                catalog and lifecycle application layer
├── proxy/                     resolution and child execution
├── api/                       Web application implementation
├── top/                       monitor model and views
└── tui/                       interactive Code implementation
```

Parser types contain no business logic. Command modules orchestrate existing
domain modules, which return types and errors rather than formatted strings.
Files split by concern before reaching repository size limits.

Typed asset execution and DeepResearch orchestration live under
`commands/code`. The Research application module owns workflow source, budgets,
prompts, evidence normalization, and report artifacts. Report models receive
only schema-constrained generation calls and no file, shell, retrieval,
delegation, Runtime, or MCP tools. The host validates the typed report result
and is the only authority that may atomically materialize the Markdown/HTML
pair. The TUI imports that module as a presentation adapter; the application
runtime never imports TUI internals and must not regain a string-based CLI
router.

DeepResearch automatically materializes a transient Loop Engineering contract
for each TUI or headless run. The contract has `quota.mode = unlimited` and an
immutable `coverage_driven` stage graph. Its maximum logical cardinality is two
semantic iterations, retrieval passes, and semantic selections, followed by one
obligation-review stage, Host contract assessment, report transaction, and
targeted section revision allowance. It travels with durable workflow input but
never creates a user-facing `.a3s/loops/` asset. Rust Inquiry validates the
contract, owns legal state transitions and terminal authority, and delegates
only durable effects to Flow.

The first pass creates the complete provider candidate catalog and has eight
initial web-fetch slots. Source-local selectors emit obligation-relevance edges
for partial routing and separate criterion/role edges for full coverage. A
second pass is legal only when Host-validated coverage edges expose a typed gap
or the Host counts evidence lost to an initial fetch/source-selection failure;
it may fetch at most two additional previously unselected candidates and avoids
a fetch-failed transport surface when a distinct candidate remains. It cannot
call search again or create, rewrite, normalize, or translate a provider query. Per-call
deadlines, output bounds, concurrency ceilings, and catalog limits are safety
fuses rather than quota accounting; none can authorize a third pass or reopen a
terminal stage.

The initial materialized evidence result is an explicit durable Flow checkpoint
before the optional second pass. When the shared retrieval deadline interrupts
that optional work, Rust validates and restores the checkpoint by exact run ID,
query, evidence ledger, and Host terminal-authority marker, then continues into
closed review. An unfinished supplement cannot replace the initial result with
an empty collection.

Obligation review generation uses short Host-owned evidence references rather
than model-copied evidence hashes. Rust maps those references through the exact
per-question catalog before committing any answer event. The shared envelope is
validated once, then each question entry is decoded independently: a malformed
sibling fails closed on its own, while `answered` plus a non-empty limitation is
monotonically demoted to `partial`. Section generation
receives bounded claim excerpts for each committed evidence binding; the Host
normalizes ISO, English, and Chinese full dates and rejects a section whose date
does not resolve to one of its committed claims. Citation and date failures
share the single durable targeted-revision allowance.
Closed question review, section generation/revision, and frame generation also
carry one semantic-granularity contract. It forbids unstated interval math,
metadata reclassification, dependency-to-incompatibility inference,
discontinuation-to-no-future-fix inference, unsupported replacement properties,
and ecosystem-wide conclusions from a few examples. Source discovery/review
metadata remains internal when the final source ledger is assembled.
Reader-facing structured output uses the query language, and scoped evidence
absence cannot become a report-global assertion. Editorial-frame generation
uses two 270-second active attempts inside the one durable report budget.
Revision packets carry exact citation alternatives for the complete binding
set. Citation normalization accepts only a strict same-origin path descendant
of a non-root committed source and selects the longest matching parent; lexical
prefixes, broader roots, and code spans are not normalized.

## 11. Component Application Layer

The existing component catalog, discovery, lifecycle, and updater mechanics
remain separate concerns:

```text
CLI request
  -> catalog resolution
  -> target and installed-state probe
  -> source resolver
  -> immutable plan
  -> confirmation policy
  -> transaction executor
  -> verification
  -> receipt and typed result
```

`install`, `upgrade`, and `uninstall` share this pipeline. They do not each
reimplement source selection or output. Multi-component operations plan all
targets, serialize conflicting component work, execute independent components,
and return every result. One failure does not discard prior results or falsely
claim cross-manager rollback; mixed results exit with code 3.

No-argument `upgrade` resolves and reports candidates but does not apply them.
`--all` is represented in the request type and cannot be inferred after
planning. `--dry-run` ends after producing the same immutable plan that a real
operation would require. Plan digests protect approval from source, scope, or
privilege changes between review and execution.

The component manager supports registered A3S identities, not arbitrary native
package names. Backends construct typed argv for trusted source kinds. They do
not execute registry-provided command strings, remote shell scripts, or
installer command definitions embedded in ACL.

### 11.1 Code local sandbox supply

The local command sandbox is an internal Code support component rather than a
new public product proxy. Core owns the `BashSandbox` contract and permission
boundary. CLI owns user-wide preparation, receipt validation, compatible Node
selection, and the capability handshake. Runtime owns durable Task and Service
placement, while Box owns OCI and stronger-isolation workloads.

The production supply path is release-owned:

1. reuse only a managed installation whose receipt, exact package identity and
   version, registry integrity, lock file, and complete tree digest match;
2. otherwise locate the support tree carried by the CLI archive or Homebrew
   formula, reject links and workspace-local copies, and compare its complete
   normalized tree against the digest compiled into the CLI;
3. require Node.js 20.11 or newer, pin that executable for the Code process, and
   run the Core handshake before attaching the sandbox;
4. permit the verified release payload in offline mode and with
   `A3S_NO_AUTO_INSTALL=1` because discovery performs no mutation;
5. only when no release payload exists and first-use mutation is allowed, use
   the fixed npm lock as a source/Cargo development bootstrap with lifecycle
   scripts disabled and official registry URLs pinned;
6. if every source fails, continue without a sandbox so Default can request one
   exact host command and Auto can deny Bash.

The TUI does not select an arbitrary `srt` from `PATH`. Release packaging,
Homebrew installation, and standalone self-update all preserve the same
verified support tree. Runtime and Box remain the owners of durable placement
and stronger isolation; this payload does not become a public component
catalog or stack-wide execution contract.

Executable discovery and version probes use bounded output files and an
explicit portable timeout. They must not install process-global signal
handlers: the root invocation owns signal registration, and component probes
may run while that listener is active.

## 12. Proxy Architecture

Proxy argument storage uses `Vec<OsString>`, not `Vec<String>`. The runner:

1. resolves a registered `ComponentId`;
2. applies offline and catalog-authorized first-use policy;
3. verifies health and compatibility;
4. selects the absolute executable from trusted state;
5. sets the resolved child working directory;
6. forwards raw argv and inherited stdin/stdout/stderr without a shell;
7. waits with signal forwarding;
8. preserves the child outcome.

Arguments after `box`, `bench`, `search`, or `use` belong to the child and are
not parsed by the root. Universal root options are parsed before the proxy
namespace. A versioned `A3S_CLI_*` child context conveys directory, output,
color, progress, offline, and non-interactive policy to compatible first-party
children. During migration, an incompatible child receives only safe process
context and the root reports which global behavior it cannot guarantee.

There is no generic fallback from an unknown root word to `a3s-<word>`. Dynamic
Use domains remain inside the trusted `use` namespace, where A3S Use validates
their ACL package and declared CLI, MCP, and Skill surfaces.

`a3s use box ...` is the one composed proxy route. The root resolves both
registered components and remains the sole Box lifecycle owner. It passes the
canonical Box executable to Use in the child environment; Use validates that
explicit path and delegates to it. No PATH rediscovery, wrapper package,
copied binary, or second receipt is allowed. Non-Box Use calls may receive an
already-ready Box path for diagnostics, but they never trigger Box
installation.

Proxying `a3s search` is an umbrella UX boundary only. Search continues to link
the typed `a3s-use-browser` renderer library directly; it does not call the Use
CLI or depend on a resident Use process.

Root-to-child component lifecycle uses argv, one versioned JSON document,
stderr diagnostics, and an exit status. Long-running domain tools use their
native CLI stream or standard MCP. None of these contracts is JSON-RPC.

## 13. Web Lifecycle Architecture

Detached Web instances use a cross-platform child-process supervisor contract,
not Unix-only daemonization. One managed instance is identified by each
canonical workspace. State records include:

- schema version and instance ID;
- canonical workspace and bound address;
- PID plus the recorded launch time;
- executable path and version;
- a random launch nonce known to the worker;
- log path, start time, and readiness state.

`web start --detach` launches a hidden internal worker mode and waits on a
bounded readiness handshake. It writes state atomically only after the server
binds. A workspace-keyed lifecycle lock makes concurrent starts converge on the
same worker. Failure returns the child diagnostic and cleans incomplete state.

`stop`, `status`, `logs`, and `open` resolve the same instance. Before requesting
shutdown, stop verifies the recorded PID and random launch nonce against the
private control route. A stale or ambiguous record is reported and quarantined;
it never causes a blind kill.
Graceful shutdown has a bounded timeout and does not fall back to force
termination.

Before configuration or session restoration, foreground startup reserves the
requested listener and detached startup probes any occupied address. A
versioned A3S health response identifies healthy foreground and legacy
instances for reuse and diagnostics, but it does not expose the control nonce
or confer general stop authority. `--replace` uses the authenticated control
route for a managed record. For an observed foreground instance, replacement
additionally requires the same canonical workspace, a health-reported PID, the
current A3S executable, a `web` command with the requested explicit port, and a
second health probe immediately before signaling. The CLI sends an interrupt
and waits for the listener to be released. A foreign or ambiguous listener is
never signaled.

Foreground and detached modes use the same server configuration and startup
path. Logs rotate under the shared state/log path policy and never contain
credentials or authorization headers.

## 14. Code Command Integration

The Code TUI, `code exec`, Web sessions, and research should reuse one session
application layer rather than parse or construct model/config state separately.
That layer owns:

- effective workspace and ACL configuration;
- session creation, resume, list, export, and deletion;
- model resolution and credential handles;
- permission and confirmation policy;
- event streaming and final results;
- cancellation and artifact reporting.

The TUI renders session events interactively. `code exec` renders the same
events as human output or JSONL. Web adapts them to its HTTP/SSE contract.
Research composes a bounded workflow and report artifact policy on top. The
shared layer does not force these surfaces through JSON-RPC.

Code Intelligence is a separate read-only workspace capability inside that
shared application layer. A local host builds one `ManifestWorkspaceBackend`,
then attaches the native provider to the resulting `WorkspaceServices`. The
provider subscribes to the existing manifest change stream and uses the same
workspace filesystem and path resolver; it must not start a second watcher,
file index, text-search service, mutation path, or memory store.

The Rust runtime owns framed stdio language protocol requests and child-process
lifecycle directly. TUI and Web never spawn language processes themselves.
They call the typed `WorkspaceCodeIntelligence` service asynchronously and
reuse their existing file-selection flows for returned locations. Web caches
the service bundle by canonical workspace and resolves an optional session ID
only to an already loaded workspace. Cache and process shutdown are explicit.

Semantic positions are zero-based UTF-16 throughout Core and HTTP contracts.
All queries use saved files and include bounded result metadata; dirty editors
must display saved-version behavior. Absolute paths, traversal, symlink
escapes, unknown sessions, malformed protocol locations, and unsupported
capabilities fail through typed errors before a file is exposed.

Interactive launch resolves an immutable `CodeRuntimeConfiguration` before
building a session. It contains the effective A3S ACL, primary ACL path, Code
asset roots, and memory root, all resolved from the invocation directory. TUI
panels receive these paths explicitly and never change the process current
directory to emulate `-C`.

Asset-family commands use a common typed discovery request with an explicit
`AssetLocation`. Each family registers only supported lifecycle operations.
Agent kind is a value enum option, removing positional inference.

## 15. Help and Completion

Clap generates parsing, root/nested help, and completion from the same types.
Help shows canonical forms, precedence, network/mutation/privilege behavior,
examples, and related commands. `a3s help <path...>` and `<path...> --help` are
equivalent. Docs tables are generated or checked against parser metadata, and
deterministic snapshots verify wrapping and errors. Suggestions never execute.

## 16. Migration and Verification

Compatibility normalization, release milestones, parser/output/security/proxy
test matrices, and acceptance gates are defined in the
[Migration and Verification Plan](cli-migration-plan.md). Built-binary tests
must use isolated roots and disable automatic installation by default.

## 17. Architectural Invariants

- There is one canonical parser and one render boundary.
- Unknown root commands never execute discovered binaries.
- Handlers do not call `process::exit` or print ad hoc machine JSON.
- Human-authored product data uses A3S ACL through `a3s-acl`, never HCL.
- CLI JSON is a versioned process result, not JSON-RPC.
- Standard MCP stays MCP; Skills stay `SKILL.md`; native CLI stays argv and
  streams.
- Secrets never enter argv or generated machine state.
- Offline is enforced before network I/O.
- Dry-run and apply share the same resolved plan.
- Files are deleted only with proven ownership.
- Proxies use absolute executables, no shell, raw native arguments, and child
  status preservation.
- Web lifecycle validates process identity before signaling.
- Public behavior is covered by built-binary integration tests on every
  supported operating-system family.
