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
complete-plan evaluation and apply-time verification API to CLI, Web, and
management MCP adapters. Web currently constructs the Manager with the
default `ask` policy until a trusted host source is explicitly carried into
that adapter.

The adapter also supplies the plan actor from its trusted boundary. CLI and
Web select `user`; management MCP selects `agent`. Package metadata, Tool
output, Skill instructions, and MCP arguments cannot select or downgrade that
actor. Reviewed records persist the selected actor and the current fixed
`user/current` lifecycle scope.

## Reviewed-plan binding

When the delegated planner returns `pluginOperationPlan`, the Manager accepts
only the strict `a3s.use.plugin-operation-plan-draft.v1` contract. That
contract has no fields for operation identity, lifetime, actor, scope, policy,
confirmation requirements, or derived secret changes, and unknown fields fail
closed. The Manager binds host authority and scope, verifies requested release
constraints and capability generation, and validates the resulting final
plan. Policy evaluation then supplies final authority and
`PluginOperationPlanEnvelope` computes the canonical reviewed digest.

The durable record keeps two distinct identities:

- the complete Use plan digest exposed to users and accepted by Manager apply;
- the upstream component digest passed only to the existing mutation child.

For a new apply intent, current policy must reproduce the stored authority.
`ask` additionally requires an exact
`a3s.use.plugin-operation-confirmation.v1` created only by a trusted
user-facing adapter. `deny`, missing confirmation, plan drift, policy drift,
and capability drift fail before intent. After intent is durable, crash
recovery validates and reuses the recorded confirmation so partial mutation
can converge safely.

Legacy component-only records remain compatible. The policy source, parser,
evaluator, full-plan persistence boundary, and apply guard are implemented and
independently tested. The umbrella component planner still needs to emit the
full Use draft before this path becomes the default lifecycle.
