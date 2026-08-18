# A3S Code editor and CI integrations

A3S Code has two first-party automation adapters. Both are thin hosts around
the same installed `a3s` executable and durable session runtime used by the TUI:

| Host | Context and review surface | Native command |
| --- | --- | --- |
| VS Code, Cursor, or Windsurf | Active selection first, bounded open documents, streamed Output view, Source Control diff review, and immutable remote patch review/apply. | `a3s code exec` and `a3s code remote` |
| GitHub Actions | Prompt or workspace prompt file, actor authorization, structured result outputs, and an unchanged or edited checkout for later deterministic steps. | `a3s code exec` |

Neither adapter embeds another agent runtime, retains provider credentials, or
silently expands the authority of `code exec`.

## Closed automation profiles

Ordinary interactive execution keeps `--tool-policy standard`. Integrations
must select one of these closed profiles:

| Profile | Required mode | Model-visible authority |
| --- | --- | --- |
| `read-only` | `plan` or `auto` | Workspace-confined native read, path/text search, pure structured output, and installed-skill discovery. |
| `workspace-write` | `auto` | The read-only set plus workspace-confined native write, edit, and patch operations. |

Both profiles hide and deny shell, Git, language-server
diagnostics/navigation, batch/program/task delegation, runtime, dynamic
workflow, Skill execution, MCP, managed Task/Knowledge, download, and Web
tools. Unknown future tools are denied by default. File mutations also
reject path traversal, absolute targets, symlink escapes, and protected control
metadata including `.git`, `.a3s`, `.agents`, `.codex`, `.claude`, `.vscode`,
`.idea`, `.gitmodules`, and shell/MCP configuration files.

The runtime checker is authoritative for every invocation. A conservative
serializable policy is persisted with the session as a restart fallback, and a
successful JSON/JSONL result echoes `toolPolicy` so a host can reject policy
downgrade.

## VS Code-compatible extension

The source package lives in [`integrations/vscode`](../integrations/vscode).
It has no runtime npm dependency. Package and install it with:

```bash
node --test integrations/vscode/core.test.js
cd integrations/vscode
npx @vscode/vsce package
```

Use the command palette or editor context menu:

- **A3S Code: Ask with Editor Context** forces Plan plus `read-only`.
- **A3S Code: Edit with Editor Context** forces Auto plus
  `workspace-write`, then opens Source Control.
- **A3S Code: Review Remote Changes** opens the exact Cloud execution patch as
  a diff document.
- **A3S Code: Apply Remote Changes** asks for modal confirmation, delegates the
  digest and whole-patch preflight to the CLI, applies without staging or
  committing, then opens Source Control.

The active selection is admitted first. Active and other open file buffers are
then ordered deterministically and fitted into `a3sCode.maxContextBytes` by
their actual JSON-encoded UTF-8 size. Editor contents are carried as explicitly
untrusted quoted data, while the operator request is a separate field. The
operator request is never truncated. Commands require an editor-trusted
workspace, and real-path checks exclude open files reached through an escaping
symlink. The extension launches argument arrays with `shell: false`, streams
bounded and sequence-checked JSONL, requires one terminal result, verifies the
echoed tool policy, supports cancellation and deadlines, and strips terminal
control sequences before presentation.

This initial adapter does not yet provide a persistent native chat sidebar,
inline per-hunk accept/reject decorations, IDE account login, or creation and
monitoring of new Cloud tasks. Local edits use the editor's existing Source
Control review, while existing remote tasks can be reviewed and applied.

## GitHub Action

The reusable action is declared by [`action.yml`](../action.yml). A minimal
review job is available in
[`examples/github-actions/code-review.yml`](../examples/github-actions/code-review.yml):

```yaml
- id: a3s
  uses: A3S-Lab/CLI@main # Pin a reviewed full commit SHA in production.
  with:
    github-token: ${{ github.token }}
    prompt-file: .github/a3s/review.md
    config: .a3s/config.acl
    permissions: read-only
  env:
    OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
```

Before it reads the prompt, the action requires the workflow actor to have
GitHub `write`, `maintain`, or `admin` repository authority, or to appear in an
explicit exact `allowed-actors` list. It uses the bundled cross-platform
installer, which verifies release checksums, unless `a3s-path` identifies a
preinstalled executable. Before sending the prompt, it verifies that the
selected executable advertises the closed `--tool-policy` contract. Published
A3S 0.12.0 and newer releases expose this contract. Callers may still provide
a compatible `a3s-path` when they need to pin an independently reviewed binary.

The action declares GitHub's Node 24 runtime. Self-hosted runners must support
that runtime; current GitHub-hosted runners already do.

The GitHub token is masked and is used only for actor authorization and the
trusted installer. The action removes every `INPUT_*` and `GITHUB_*` value plus
Actions cache/runtime/OIDC token or URL capabilities before starting
A3S. Provider configuration remains available to the provider host, but the
model cannot invoke a process tool to read its environment. Prompts travel over
stdin and child processes always use an argument vector with no shell.

The action does not log model text, post a comment, stage, commit, push, or make
a pass/fail decision from prose. Consumers receive `final-message`,
`final-message-truncated`, `session-id`, `usage-json`, and a private
runner-temporary `result-file`; treat all model text as untrusted data. The
message output is UTF-8 bounded below GitHub's per-job output ceiling, while the
result file retains the complete response. A `workspace-write` job should review
the resulting diff and run tests only in explicit, least-privilege later steps
that hold no publishing credentials or unrelated secrets. Generated code
remains untrusted until review and validation complete.

Never check out an untrusted pull-request head in a privileged
`pull_request_target` job. Grant the workflow only the GitHub permissions it
needs, pin actions to reviewed full commit SHAs, and keep model credentials out
of fork-triggered jobs.
