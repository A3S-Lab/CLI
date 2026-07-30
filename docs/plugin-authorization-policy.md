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

The policy contract and evaluator are implemented and independently tested.
Lifecycle integration still requires the umbrella planner to persist the full
Use operation plan and to invoke evaluation before storing and again before
applying it.
