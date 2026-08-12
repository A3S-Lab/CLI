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
still permits local receipts, already installed verified package generations,
system probes, and already installed proxies. Cognitive-package installation
requires a fresh trusted Registry verification and therefore fails offline.
`A3S_NO_AUTO_INSTALL=1` maps to the stricter first-use policy.

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

Typed asset execution lives under `commands/code`. DeepResearch itself lives
in the independent `a3s-deep-research` crate. That crate owns the engine stage
machine, workflow assets, source admission, report admission, and
Markdown/HTML construction. Headless CLI, TUI, and Code Web call the shared
`src/research/CodeDeepResearchRunner` product adapter. It provides an isolated
read-only `AgentSession`, explicit evidence scope, workspace publication,
progress events, cancellation settlement, and a typed run journal. No surface
owns a planner or report implementation.

Every new CLI, TUI, or Web run compiles a transient evidence-first contract with
`quota.mode = bounded` and `execution.mode = progressively_publishable`. The
contract travels with durable runtime input but never creates a user-facing
`.a3s/loops/` asset. One Host-owned wall-clock origin bounds acquisition,
optional report proposal, and finalization. Stable Flow effects retain their
own replay semantics; the version-2 Code run journal is a surface-refresh
projection and does not claim to resume an interrupted root process.

`CodeDeepResearchRunner::start` is the new-run entry point. It validates a
`DeepResearchRequest`, creates an empty Skill registry, removes Web tools for
local-only evidence, and delegates one cancellable transaction to
`DeepResearchEngine::execute_request` through `CodeDeepResearchRuntime`. The
engine runs durable bootstrap acquisition, combines the exact query with only
the planner's closed supplemental-query contract, stages a source-backed or
no-evidence report, optionally upgrades it with an admitted proposal, and
returns one typed result. There is no string router or second report
implementation.

Every result is run-scoped under
`.a3s/research/artifacts/<run-id>/report.md` and `index.html`. Engine events are
persisted at `.a3s/research/runs/<run-id>/journal-v2.jsonl` with a strict
sequence and matching run identity. The durable projection retains lifecycle,
stage, publication, and quality while deliberately omitting absolute artifact
paths. Code Web uses it to restore progress after a browser refresh. Artifacts
are served only by validated run ID and `html` or `markdown` kind.

Web context files become bounded relative `WorkspaceSourceHint` values and are
preflighted as existing, non-empty, non-symlink files before the isolated
session starts. Non-empty Skill selections fail explicitly until the typed
runner defines a supported Skill contract. TUI escape, Agent Island stop, Web
cancel, and handle drop all cancel the root run; explicit cancellation waits
for settlement and closes the isolated session.

Web bootstrap always searches the exact query. One optional bounded semantic
outline decomposes at most 24 atomic request requirements, maps every one to
one or more of at most eight material tracks, and may supply zero to 15
supplemental plain-text queries. The Host validates their shape, mappings, and
exact identity; it does not derive query text
from topic words, dates, language, publishers, domains, or URL vocabulary.
If the outline is unavailable or invalid, the Host keeps only the exact query,
uses one generic track, and selects the conservative `comprehensive` plus
`freshness_required = true` contract. Planner failure therefore cannot
authorize an undated synthesized answer.
Up to two later gap-directed rounds may generate queries only from typed
missing criteria, missing source roles, or failed retrieval effects. The Host
expands incomplete tracks into atomic criterion targets and rotates them fairly.
They share Host-owned totals of at most 24 new queries and 16 supplemental
fetches, with the second round receiving only the budget not consumed by the
first.
Candidate and chunk selectors may return only IDs from the closed packets.
Transport fallback preserves bounded acquisition opportunities but cannot
promote bytes into evidence. Provider rank, snippet, date, engine, title,
hostname, and publisher remain discovery metadata.
Search output must decode as the declared JSON result shape. Web and workspace
text is restored only from typed range offsets, returned character or line
counts, exact source anchors, and artifact truncation metadata. Provider error,
continuation, or truncation prose is never matched inside tool output.

The Core 6.7 `web_search` cascade executes headless engines first and enters
HTTP/RSS or native API tiers only while its structural retrieval requirements
remain unmet. `ToolEnd.metadata` carries tier reports, retrieval health and requirements,
engine outcomes, circuit/failure evidence, fallback state, and bounded result
counts. The TUI parses only those typed fields into a bounded semantic search
projection; normal successful history does not render the provider body, while
the full transcript retains both the projection and bounded result.

Search, fetch, and structured-generation effects use stable A3S Flow identities.
Completed effects replay from their journals. A running effect whose completion
was not durably acknowledged is redelivered with the same attempt identity and
therefore has explicit at-least-once semantics. The local Flow JSONL store
preserves a complete final envelope missing only its delimiter and discards only
an unterminated torn tail before the next append; terminated or interior
corruption still fails closed.

The DeepResearch producer's `maxConcurrentGenerations` value passes through a
host contract gate that accepts only Core 6.7's 1-4 range. Core then assigns an
independent derived session id to each eligible `generate_object` step and
applies one shared generation admission bound. Providers that cannot fork
remain single-flight. Timed-out planned retrieval may recover only an exact
completed checkpoint whose durable run id, original query, and step id all
match; incomplete or cross-run output is rejected.

The Host builds a source-backed Markdown/HTML pair before report generation can
become a terminal risk. An empty catalog instead produces the no-evidence pair.
When claim-eligible sources exist, one typed claim-graph proposal may run with
at most one transient retry. The proposal receives bounded source and chunk
IDs, no tools, and no publication authority. Rust independently admits facts,
inferences, recommendations, relations, and gaps through the evidence
compiler. It validates exact dimension/source/chunk IDs, basis edges,
derivation inputs, contradiction endpoints, provenance, and graph bounds. It
then derives coverage, citations, the source ledger, and both renderings from
one `ReportDocument`. Invalid graph items cannot erase valid siblings.

An independently generated commercial editorial plan then reviews every mapped
requirement, dimension, and claim against the exact query, current date, and
closed cited excerpts. It classifies fact temporal status and returns one
evidence-preserving rewrite per admitted claim plus a bounded paragraph plan.
Rust rejects missing or duplicate reviews, inconsistent readiness, invalid
temporal classes, numeric drift, changed claim identity, dependency inversion,
or source-summary/shallow dimensions. A synthesized draft cannot remain
successful when this stage fails or rejects publication readiness.

Each attempt uses a schema compiled from the current packet: dimension, source,
and chunk enums contain only the validated run, and audit-only sources cannot
be referenced. Opaque IDs are control data only, not reader prose. Reader
language is request-owned: the Host pins one inferred or explicit BCP 47 tag
through planning, generation, admission, and publication, and rejects an
altered tag or an obvious aggregate prose-language mismatch. Semantic
entailment remains a model/evaluator concern; the Host does not match claim
prose to source prose. Larger fetched catalogs are divided only by source
identity and a 32 KiB UTF-8 packet budget; a closed exact-ID reduction retains
at most four excerpts per source. Complete material coverage publishes
`Synthesized`; useful admitted claims with a material typed gap publish
`Qualified` only as an incomplete preview after the resolved dimensions pass
the per-dimension depth gate. If every dimension remains bounded, the Host
admits at most one qualified partial preview only when it independently supplies the full two-source
comparison, explanation, implication, boundary, and substantive-length chain
plus a typed gap. A focused report may be structurally sufficient with one
cited direct-answer claim. Only `Synthesized` is successful; `Qualified`,
`SourceBacked`, and `NoEvidence` retain artifacts but return non-success
semantics from CLI, TUI, and Web.

Markdown and HTML publication pairs carry the same versioned artifact marker.
The Host never infers synthesized, source-backed, no-evidence, recovery, or
fallback status from titles or body vocabulary.

The fixed HTML renderer follows the A3S Web design tokens. Desktop output uses
a sticky left action menu, centered report surface, and sticky right table of
contents; narrow screens stack both navigation regions without horizontal
overflow. One Host-owned script supports edit mode, title and table-of-contents
synchronization, saving a self-contained HTML copy, and printing. Arbitrary
reader-authored scripts remain rejected.

The returned status distinguishes `synthesized`, `qualified`, `source_backed`,
and `no_evidence`; it never equates artifact availability with semantic truth
or command success.
Bootstrap metadata is deliberately withheld from the Host-owned terminal tool
result so workflow canonicalization cannot replace publication output with
acquisition output. Historical Inquiry events remain readable through the
generic event journal, but the former sectioned-report executor and report
resume transaction have been removed. They are not selectable runtime paths.

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

### 11.1 Code local sandbox supply and execution

The local command sandbox is an internal Code support component. Core owns the
`BashSandbox` execution contract and catastrophic-command floor. CLI owns
user-wide preparation, receipt validation, compatible Node selection, native
capability probing, and attachment across TUI, Web, and headless execution.

The supply order is deterministic:

1. reuse only managed state whose receipt, package identity/version, registry
   integrity, lock graph, and complete installation-tree digest verify;
2. otherwise discover the regular-file support tree carried beside the release
   binary or under the Homebrew share prefix, reject workspace-local and linked
   copies, and compare it to the digest compiled into the CLI;
3. pin a trusted Node.js 20.11-or-newer executable and complete the Core package
   handshake plus a bounded command through the native OS boundary;
4. allow verified state and release payloads offline and when automatic install
   is disabled because those discovery paths do not mutate component state;
5. only when no payload is ready and online first-use mutation is permitted,
   install the exact npm lock with lifecycle scripts disabled, apply the tested
   compatibility patch in staging, verify the complete tree, then activate it
   atomically.

Release jobs build that same support tree once, verify its normalized digest,
exercise its complete Linux/macOS policy, and inject it into every archive.
Installers validate and atomically replace the support directory. Standalone
self-update installs it in the same transaction as the CLI and native window
helper and restores the previous tree if either companion or binary activation
fails. The inert `release-compat` tree remains separately packaged for old
updaters and is deliberately excluded from current payload discovery.

Default and Auto admit ordinary Bash only when a verified sandbox handle is
attached. Plan denies Bash. Explicit `require_escalated` requests cross to the
host only after exact Default-mode approval; Auto denies them. When no sandbox
is available, Default asks for every non-denied host command while Plan and Auto
fail closed. Catastrophic commands and credential/control paths are denied
before either boundary. Permission, confirmation, sandbox, timeout, process
group, output, and streaming state are snapshotted per admitted run and
inherited by delegated and Skill work.

The native provider denies network egress and local listeners, bounds writes to
the workspace plus private scratch, protects control metadata, masks credential
stores and hard-link aliases, and scrubs ambient environment variables. Linux
uses bubblewrap/user namespaces, macOS uses a private Seatbelt policy file, and
Windows uses the provisioned dedicated sandbox user plus WFP fence. Failure of
any prerequisite produces an actionable warning and never an unsandboxed
fallback. Runtime still owns durable Task/Service placement and Box owns OCI or
stronger-isolation workloads.

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

The TUI splits Core event presentation into three typed projections. Text,
reasoning, and tools enter the semantic transcript; tool and child lifecycles
also update the ID-keyed runtime projection; selected agent mode, context
resolution, planning, and external-task waits update a transient
`CoreRunStatus`. Queue retries and dead letters, queue alerts, persistence
failures, budget thresholds, passivation requests, peer invocation, and failed
external tasks become bounded transcript notices. A wildcard remains only for
forward-compatible informational events and must not swallow an operational
failure already defined by the supported Core version. Memory search and recall
share the transient activity row and clear authoritatively at context
resolution; memory storage produces a content-free notice that never exposes
internal IDs, tags, queries, or recalled text. A non-empty terminal verification
summary is emitted after the assistant message with passed, review, or failed
severity, while an empty skipped summary stays silent.

Terminal-facing projections share `sanitize_terminal_layout` before any
component adds ANSI styling. Its state machine consumes complete ESC, CSI, OSC,
DCS, SOS, PM, APC, and 8-bit C1 control strings across the source value, removes
remaining C0/C1 and bidirectional formatting controls, preserves ordinary
Unicode and line structure, expands tabs to four-column stops, and enforces an
exact character ceiling. Notice, tool, and transcript render boundaries use a
1,000,000-character defensive ceiling on top of Core's bounded tool transport;
the constructors sanitize semantic user, notice, assistant, and reasoning
sources before storage, while internal preformatted rows remain byte-exact.
`StreamingMarkdown` and whole-turn assistant capture retain at most 4 MiB, and
the private reasoning buffer retains at most 1 MiB. A UTF-8-safe marker becomes
the terminal suffix and rejects later deltas after either limit is crossed.
Before `ToolEnd`, both the semantic transcript and runtime projection retain at
most 1 MiB of streamed arguments and 1 MiB of live output per tool. At
`ToolExecutionStart` and `ToolEnd`, authoritative arguments and metadata pass
through a depth-, node-, collection-, key-, and string-budgeted structural JSON
projection whose serialized result is at most 1 MiB. It preserves common
semantic fields plus original-byte and truncation markers. The separate
`PendingToolApproval` continues to own exact arguments, so presentation bounds
cannot broaden or alter authorization. Plan, queue, and subagent labels use
smaller field-specific ceilings.

`PlanProjection` materializes at most 256 task records and keeps only the ID,
bounded content, and status required for presentation. The renderer combines
that retained prefix with an explicit omitted count. Codex-compatible
`update_plan` arguments are validated across the complete array so an invalid
omitted tail cannot be accepted, while allocation remains limited to the first
256 rows. Semantic task IDs remain unchanged for lifecycle matching; only
terminal-facing labels are sanitized.

Terminal tool presentation consumes Core's structured `ToolErrorKind` rather
than classifying human-readable output. Typed version conflicts instruct the
user to refresh the file, invalid arguments require changed input, unsupported
operations identify the backend boundary, and typed timeout, transport, rate
limit, cancellation, and partial-result outcomes keep their distinct recovery
semantics. Untyped failures receive no inferred retry label.

A `!` turn does not spawn a second shell path in the TUI. It invokes the
registered Core `bash` tool through the active `Session`, so the same immutable
workspace, cancellation, permission policy, sandbox selection, output bounds,
and semantic tool lifecycle apply. This also means `-C` and the TUI's process
current directory cannot diverge for direct shell execution.

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

The TUI projection treats every language-server string as untrusted terminal
input. It applies per-field, title, row, count, query, and outline-depth limits,
uses iterative outline traversal, and strips complete ANSI/OSC/C1 and
bidirectional-control sequences again at the final render boundary. Display
sanitization never rewrites the typed path or UTF-16 position used for a jump.
A typed protocol failure is the only category eligible for one delayed,
cancellation-aware retry under the same absolute deadline; no retry decision is
inferred from human-readable error text.

Cross-session retrieval is a separate local recall tier. The TUI invokes the
installed `ctx` executable with an argv vector, null stdin, temporary-file
stdout/stderr capture, and no shell interpolation. A two-second startup probe
uses a 64 KiB combined-output ceiling; interactive search/show calls use a
15-second deadline and a 2 MiB ceiling. On Unix every child leads a dedicated
process group, and deadline or output overflow kills and reaps the entire group.
Search input is limited to 1,000 characters and `--` terminates options before
the query. Parsed IDs, provider, title, snippet, timestamp, stderr, ANSI/OSC/C1
sequences, and bidirectional controls are sanitized and bounded before display.

`/ctx <n>` retrieves the exact event window, quotes every line as explicitly
untrusted historical material, and applies a UTF-8-safe 6,000-byte one-shot
injection ceiling. `/ctx save <n>` writes an episodic item through the live
session memory handle where available and preserves `ctx_event_id` and
`ctx_session_id` provenance for a later jump back to the raw session. The
headless `code ctx search/show/session` commands expose durable retrieval but do
not pretend to attach data to a non-existent interactive turn.

The login-gated `runtime` tool is an approval-requiring remote boundary. Its
schema and runtime both enforce 1–64 string/object tasks, a 1 MiB serialized
request ceiling, a terminal-safe 128-character worker name, and a 1 ms to
30-minute poll budget. Worker names resolve only to tool-kind assets with a
canonical UUID. Batch and invocation IDs are length-bounded safe path segments;
invocation IDs must also be unique, and the response must return exactly one
invocation ID per task.
Polling uses one absolute deadline, cancellation interrupts requests and
backoff, and timeout still fetches the completed subset. Control responses are
limited to 1 MiB, per-invocation responses to 256 KiB, result fetch concurrency
to eight, and each output/error value to a bounded terminal-safe representation.

OS progressive capabilities use one bounded search → describe → execute client
instead of embedding service-specific routes in each panel. It attempts at most
four scored candidates. Requests and responses are each capped at 1 MiB;
capability traversal is iterative and limited to depth 32, 4,096 nodes, 256
candidate operations, 256 schema fields, and bounded terminal-safe identifiers.
A malformed, oversized, or error envelope fails that candidate closed while an
eligible `.view` or `viewUrl` still flows through the shared RemoteUI parser.

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
