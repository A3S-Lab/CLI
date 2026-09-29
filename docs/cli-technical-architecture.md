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
under Code; and proxy arguments are forced through UTF-8 `String`. The
migration characterizes public behavior with
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
the root. Code Exec and Top JSON/JSONL snapshot execution consume that token. A
cancelled machine stream writes its terminal error event before the renderer
returns exit `130`. Proxy signal forwarding remains separate acceptance work
and must not be inferred from this checkpoint.

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

```text
src/
├── main.rs
├── cli/                       args, context, and output
├── commands/                  one module per root command
├── components/                catalog and lifecycle
└── code_pager.rs              external a3s-code-tui launch
```

Parser types contain no business logic. Command modules orchestrate existing
domain modules, which return types and errors rather than formatted strings.
Files split by concern before reaching repository size limits.

Interactive `a3s code` and `a3s code resume` launch the external pager.
`a3s code exec`, sandbox, hooks, and session stay in this process. There is
no research runtime in the CLI.

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
`BashSandbox` execution contract, policy compiler, native provider, timeout,
process-tree cleanup, bounded output, and catastrophic-command floor. CLI owns
the capability probe and attachment across TUI and headless execution. There
is no downloaded support tree, npm bootstrap, sandbox sidecar, or user-wide
sandbox receipt.

Each invocation canonicalizes its workspace, validates protected entries and
hard-link aliases, resolves only trusted system executables, compiles a fresh
workspace policy, and runs a bounded probe through the selected OS boundary.
Release jobs execute that same integration probe on Linux, macOS, and Windows.
Installers and self-update move the CLI binary, the target-specific Moli
headless runtime, and the optional native window helper as one recoverable
payload. The Code runtime first discovers the adjacent Moli sidecar; source
and Cargo installs use the same digest-verified per-user cache guarded by a
cross-process lock, so concurrent Code processes converge on one copy.

Default and Auto admit ordinary Bash only when a verified sandbox handle is
attached. Plan denies Bash. Explicit `require_escalated` requests cross to the
host only after exact Default-mode approval; Auto denies them. When no sandbox
is available, every Bash request fails closed. Catastrophic commands and credential/control paths are denied
before either boundary. Permission, confirmation, sandbox, timeout, process
group, output, and streaming state are snapshotted per admitted run and
inherited by delegated and Skill work.

The native provider denies network egress and local listeners, bounds writes to
the workspace plus private scratch, protects control metadata, masks credential
stores and hard-link aliases, and scrubs ambient environment variables. Linux
uses bubblewrap/user namespaces, macOS uses a private Seatbelt policy file, and
Windows uses an AppContainer process inside a kill-on-close Job Object. Failure of
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

## 13. Code Command Integration

Interactive `a3s code` and `a3s code resume` hand the terminal to
`a3s-code-tui` and `a3s-code-acp`. `a3s code exec` runs one non-interactive
Core session in this process, including workspace memory and skill discovery.
Hooks, sandbox status, and session files stay in this process.

## 14. Help and Completion

Clap generates parsing, root/nested help, and completion from the same types.
Help shows canonical forms, precedence, network/mutation/privilege behavior,
examples, and related commands. `a3s help <path...>` and `<path...> --help` are
equivalent. Docs tables are generated or checked against parser metadata, and
deterministic snapshots verify wrapping and errors. Suggestions never execute.

## 15. Migration and Verification

Compatibility normalization, release milestones, parser/output/security/proxy
test matrices, and acceptance gates are defined in the
[Migration and Verification Plan](cli-migration-plan.md). Built-binary tests
must use isolated roots and disable automatic installation by default.

## 16. Architectural Invariants

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
