# Plugin Authorization Policy

The umbrella A3S host owns plugin authorization. Package manifests, Skills,
Tool output, MCP descriptions, UI messages, and catalog text are untrusted
inputs and cannot modify this policy.

`PluginAuthorizationPolicy` parses the strict `a3s.plugin-policy.v1` block from
an A3S ACL document. Other top-level A3S configuration blocks are ignored by
this parser. If the document has no `plugins` block, agent install, upgrade,
and uninstall decisions default to `ask`.

## ACL schema

```acl
plugins {
  schema = "a3s.plugin-policy.v1"

  agent_install   = "allow"
  agent_upgrade   = "ask"
  agent_uninstall = "ask"

  trusted_registries = ["a3s"]
  trusted_publishers = ["a3s"]
  allowed_surfaces   = ["mcp", "skill", "tool", "ui"]

  max_download_bytes  = 52428800
  max_installed_bytes = 268435456
  max_packages        = 16
  max_surfaces        = 64

  allow_release_bundles = true
  allow_user_scope      = false
  workspace_ids         = ["workspace:research"]
  max_workspaces        = 1

  permissions {
    plugin_data = "read-write"
    temporary   = "read-write"

    native_execution = false
    child_process     = false
    private_service   = true
    secrets           = false

    ui_http              = true
    ui_methods           = ["get", "post"]
    max_ui_path_prefixes = 16

    max_cpu_millis              = 2000
    max_memory_bytes            = 1073741824
    max_pids                    = 256
    max_ephemeral_storage_bytes = 2147483648
    max_task_timeout_ms         = 300000
    max_stdout_bytes            = 8388608
    max_stderr_bytes            = 2097152

    network "api.example.com" {
      ports = [443]
    }

    workspace "inputs" {
      access = "read"
    }
  }
}
```

Every omitted ceiling is zero, empty, or false. Lists and rules are normalized
into canonical order before the policy digest is computed. Duplicate entries,
unknown fields, invalid identifiers, non-integral numbers, broad network
patterns, and `secrets = true` fail closed. Network hosts and ports are exact.
Workspace filesystem paths are portable, scope-relative prefixes; access can
be `read` or `read-write`.

## Evaluation

The evaluator consumes a structurally valid
`a3s.use.plugin-operation-plan.v1`, not catalog search output. It checks every
added or replaced root/dependency package against:

- registry, publisher, release-bundle, package-count, and byte ceilings;
- resulting surface kinds and count;
- exact user/workspace scope impact;
- filesystem, network, native execution, child process, private Service,
  resource, secret-name, and UI HTTP ceilings; and
- the selected Runtime enforcement profile.

An exact plan within all ceilings receives the configured decision. If an
`allow` plan exceeds any ceiling, it becomes `ask`; it is never silently
broadened. An agent plan that adds a secret is `deny`. A local reviewed package
is user-only, and `native-unconfined` always blocks unattended `allow`.
Interactive user plans remain `ask`.

The normalized policy digest and decision are copied into
`PluginOperationPlan.authority`. Apply calls `verify_plan_authority` against
the current policy; any digest or decision drift requires a new plan and
review.

## Trusted policy sources

The CLI and management MCP load authorization from an explicit
`--config`/`A3S_CONFIG_FILE` path when present, otherwise from the existing
user-level `~/.a3s/config.acl`. An automatically discovered workspace
`.a3s/config.acl` is not an authorization source: repository content cannot
pre-authorize its own mutation. Reads are bounded to 256 KiB, invalid UTF-8 or
ACL fails closed, and an absent or empty configuration produces the default
`ask` policy.

The shared Plugin Manager stores this policy immutably and exposes one
complete-plan evaluation and apply-time verification API to CLI, Web,
management MCP, and the canonical remote `PluginHostManager` adapter. Web
currently constructs the Manager with the default `ask` policy until a trusted
host source is explicitly carried into that adapter.

Each adapter supplies the plan actor and scope from its trusted boundary. CLI
and Web select `user` with the fixed `user/current` scope; management MCP
selects `agent` while retaining that local scope. The managed-host adapter
selects `agent` and derives an exact Workspace `PlanScope` from the host-owned
`PluginManagedScope` fence. Package metadata, Tool output, Skill instructions,
MCP arguments, and remote requests cannot downgrade the actor or provision a
new fence. Reviewed records persist the complete selected actor and scope.

## Reviewed-plan binding

The delegated planner must return `pluginOperationPlan`, and the Manager accepts
only the strict `a3s.use.plugin-operation-plan-draft.v1` contract. That
contract has no fields for operation identity, lifetime, actor, scope, policy,
confirmation requirements, or derived secret changes, and unknown fields fail
closed. The Manager binds host authority and scope, verifies requested release
constraints and capability generation, and validates the resulting final
plan. Policy evaluation then supplies final authority and
`PluginOperationPlanEnvelope` computes the canonical reviewed digest.

The delegated record must also carry a complete cognitive-package lock. The
Manager reads one `a3s.use.plugin-workspace-grant-snapshot.v1` from A3S Use's
durable Grant store using the exact plan scope and state revision. It evaluates
a provisional activation plan, binds the canonical Grant impact through
`bind_cognitive_package_grant_impacts` using the resulting host authority, and
evaluates the completed plan again. A missing snapshot, scope/revision drift,
prebound workspace impact, or changed authority fails closed. Package content
cannot supply its own Grant digest or policy identity.

The durable record keeps two distinct identities:

- the complete Use plan digest exposed to users and accepted by Manager apply;
- the upstream component digest retained only for component-plan/result
  binding.

For a new apply intent, current policy must reproduce the stored authority.
`ask` additionally requires an exact
`a3s.use.plugin-operation-confirmation.v1` created only by a trusted
user-facing adapter. `deny`, missing confirmation, plan drift, policy drift,
and capability drift fail before intent. After intent is durable, crash
recovery validates and reuses the recorded confirmation so partial mutation
can converge safely.

## Managed Workspace boundary

`ManagedPluginHostManager` is the sole remote package-host port. It advertises
one immutable protocol-v4 capability document and verifies its canonical
digest before every request. A trusted local enrollment boundary initializes
the durable `PluginManagedScope` fence; rotation uses compare-and-advance with
the same host and Workspace identity and a strictly newer generation. Remote
plan, apply, enablement, and observation requests can verify that exact fence
but can never create or advance it.

Managed plan records retain the complete `PluginHostPlanRequest`; apply intents
retain the complete `PluginHostApplyRequest` and exact confirmation. Reusing a
request ID with changed assignment, capabilities, scope, candidate, lock,
surface selection, operation, digest, or confirmation fails closed. Local
CLI/Web apply paths reject managed Workspace plans, and the remote path rejects
local plans. Completed results replay only for the byte-equivalent durable
request, including after host recreation.

Locked cognitive-package graph plans use the same exact Grant planner for local
and managed scopes. The managed fence chooses the Workspace scope; the Manager
then snapshots that scope at the durable planner revision and A3S Use derives
the exact prepare/cutover evidence from the host-owned authority. The remote
request cannot supply or replace the snapshot, impact digest, or authority.

Protocol v4 adds explicit reviewed enablement planning without creating a
second lifecycle. `PluginHostEnablementPlanRequest` is bound to its request ID,
assignment, capability digest, managed fence, package generation, and desired
state. The host derives a stable operation ID, asks A3S Use for an exact
plan-v4 envelope, and persists both the request result and operation index
before returning. `NoChange` is a terminal result with no synthetic mutation
plan. Repeating the same request replays the durable result; changed evidence
under the same identity fails closed.

The planned branch projects into the existing `PluginHostPlanResult`, so apply
continues to use only `PluginHostApplyRequest`. Before the first mutation, the
Manager re-evaluates current policy, verifies the operation ID, plan digest,
confirmation, lifetime, capability digest, and fence, then durably records the
complete apply intent. It composes
`ReviewedCognitivePackageAuthorizationProvider` and A3S Use must reproduce the
same plan before its existing enablement and Grant saga can start. The durable
result retains the Use completion time, result digest, and package state;
restart replay changes only `replayed`.

This path supports both permission-free and permission-bearing schema-v3
packages. Disable atomically publishes the hidden state before drain, Grant
retirement, and surface stop; enable prepares providers and Grants before
publishing. Neither transition changes package bytes or the dependency graph.
Missing confirmation, policy drift, stale generation, request or digest
substitution, and a missing durable plan fail before a new apply intent. Local
CLI, TUI, Web, and the managed host expose explicit scoped planning and
digest-bound apply through the same shared authorization implementation; no
direct enablement mutation request exists. The TUI `/packages` surface shows
the exact operation ID, canonical digest, expected generation, and expiry
before explicit confirmation; `NoChange` carries no mutation identity, and the
live watcher consumes only the resulting generation.

When the record carries a complete schema-v3 package lock, apply does not
serialize authority through argv, environment variables, or a temporary file.
The Manager reconstructs only the enabled Registry names whose URL and TUF
trust root still match the reviewed lock, creates
`ReviewedCognitivePackageAuthorizationProvider` from the stored envelope and
persisted confirmation, and invokes `CognitivePackageManager` in-process. A3S
Use must reproduce the exact action, package transitions, provider evidence,
impact, state revision, scope, authority, candidate/prior locks, operation ID,
and canonical digest. Registry replacement, disablement, removal, or trust
drift fails before download or package mutation.

Older catalogs, component-only records, and unlocked drafts are rejected during
planning. The policy source, parser, evaluator, full-plan persistence boundary,
and apply guard are implemented and independently tested. The Manager also
strictly retains plan-ready installed package evidence from the A3S Use
capability snapshot, including receipt, catalog-record, manifest,
expanded-package, desired-state, and exact reconciliation-surface bindings.

The in-process live joins cover dependency-locked schema-v3 install with
permission-free Skill/UI/Flow surfaces and a permission-bearing executable Tool
Task. Both use a real signed TUF repository, begin at capability generation
zero, prove that no child `a3s` mutation is launched, observe the next
generation, persist the parent cutover, and replay the exact confirmation. The
Tool regression additionally binds filesystem, network, secret, and native
provider evidence into an exact durable Grant receipt. The managed regression
adds exact capability/candidate/lock/surface/assignment/Workspace-fence
binding, rejects request and local-path substitution, verifies the returned
receipt and canonical result digest, and replays after recreating the host. It
then plans a plan-v4 disable, rejects request and digest substitution, applies
the exact confirmation, recreates the host, proves result replay, rejects a
stale generation, plans and applies re-enable, and verifies package and graph
content did not change. A separate permission-bearing Tool regression proves
that the plan carries exact Grant retirement evidence and that an unconfirmed
apply creates neither a durable intent nor a lifecycle mutation.

Catalog-v3 install and upgrade plans carry the complete verified candidate
catalog, exact TUF target, and resolved dependency lock. Upgrade and uninstall
also resolve the strict package-specific
`a3s.use.installed-plugin-plan-evidence.v1` record and match its receipt,
catalog, capability generation/revision, desired state, selected surfaces,
component identity, and version to the compact snapshot and umbrella current
state before deriving graph-wide add/replace/remove/retain transitions. Upgrade
binds exact prior and candidate locks; upgrade and uninstall preserve shared
dependencies still owned by another installed root graph. Missing or drifted
evidence fails closed.

Every complete draft carries aggregate impact, capability generation, and the
durable planner-state revision before host authorization. The private
planner-state record advances atomically and idempotently after successful
mutation. Apply first persists the exact A3S Use parent lifecycle binding, then
forwards the same reviewed envelope and confirmation to Use's package,
provider, and Grant saga. It requires the verified next capability generation
before planner-state advance and persists the parent capability cutover.
Result replay validates the binding, cutover, capability snapshot, and state
revision together; missing or drifted post-mutation evidence cannot become a
completed result.

For a locked cognitive-package graph, A3S Use owns the exact child lifecycle;
the parent does not duplicate Tool/provider/secret/Grant mutation. Code's
lifecycle factory still rejects unsupported required surfaces before
publication: OKF, long-lived Tool Services, and HTTP MCP remain unavailable
until their production Knowledge, Runtime Service, and Gateway adapters are
injected. Plans without a cognitive-package lock are rejected before durable
review. Cross-platform production E2E, real-registry multi-root dependency
graphs, and prior-generation retirement remain gated.
