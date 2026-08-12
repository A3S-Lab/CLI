# A3S Code GitHub Action

This repository-native JavaScript action runs `a3s code exec` without adding a
second agent implementation or a third-party runtime dependency. It can review
a checkout or make bounded workspace edits, then returns the final message,
session ID, usage, and complete JSON result as outputs. `final-message` is
bounded below GitHub's per-job output ceiling; `final-message-truncated` reports
whether the complete text must instead be read from `result-file`.

```yaml
permissions:
  contents: read

jobs:
  a3s-review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2
      - id: review
        # Pin this action to a reviewed full commit SHA in production.
        uses: A3S-Lab/CLI@main
        with:
          github-token: ${{ github.token }}
          prompt-file: .github/a3s/review.md
          config: .a3s/config.acl
          permissions: read-only
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
```

To publish a native, line-level pull-request review, grant only
`pull-requests: write` in addition to checkout access and opt in explicitly:

```yaml
on:
  pull_request:

permissions:
  contents: read
  pull-requests: write

jobs:
  a3s-pr-review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2
      - uses: A3S-Lab/CLI@main # Pin a reviewed full commit SHA in production.
        with:
          github-token: ${{ github.token }}
          prompt: Review this pull request for concrete regressions.
          permissions: read-only
          publish-review: true
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
```

On `issue_comment`, publication additionally requires an exact
`@a3s review` mention on a pull request. Trusted `workflow_dispatch` jobs can
instead pass `pull-request-number`. The action fetches bounded PR patches with
the GitHub API, marks them as untrusted data in the model prompt, accepts only
a strict fenced JSON protocol, validates every P0/P1 finding against a real
changed path and patch line, and then publishes one standard GitHub review.
The GitHub token remains in the action's Node host and is never passed to A3S
or the model process.

Exactly one of `prompt` or `prompt-file` is required. Prompt, config, and
working-directory paths resolve through real paths and must remain inside the
checkout. Inline prompts are sent over stdin, never interpolated into a shell
or command string.

The action verifies that the selected executable advertises the closed
`--tool-policy` contract before sending the prompt. When `main` is newer than
the latest published A3S release, pass `a3s-path` pointing to a compatible
preinstalled build.

The JavaScript entry point uses GitHub's Node 24 action runtime. Self-hosted
runners must support that runtime; GitHub-hosted current runners already do.

## Permission profiles

| Profile | Native tools exposed to the model |
| --- | --- |
| `read-only` | Bounded native workspace reads and path/text search. |
| `workspace-write` | The read-only set plus bounded native write, edit, and patch operations. |

Neither profile exposes shell, Git, task, runtime, plug-in, MCP, or network
tools. `workspace-write` changes the checkout but never stages, commits,
pushes, or runs tests. Review publication is a separate explicit
`publish-review` host capability and accepts only validated P0/P1 review data.
Read-only reviews use the ordinary one-shot execution route rather than the
multi-step planner, so the strict review protocol remains the terminal output.
Use ordinary later workflow steps to
inspect the diff and run deterministic validation. Those steps should be
least-privilege and hold no publishing credentials or unrelated secrets:
model-generated source remains untrusted until review and validation complete.

Before reading the prompt, the action requires the workflow actor to have
`write`, `maintain`, or `admin` repository permission through GitHub's
collaborator API. `allowed-actors` can name reviewed bot identities explicitly.
When review publication is enabled, the same masked token is also used by the
Node host to read PR patches and publish the validated review; it is never
placed in the model process environment.
The GitHub token is masked, used by that check and the bundled verified release
installer, then removed—along with all `INPUT_*`, `GITHUB_*`, and Actions
runtime/cache/OIDC token or URL capabilities—from the model-hosting A3S
process. Provider credentials remain available to the configured provider,
while the closed tool profile prevents the model from reading process
environment.

Do not combine this or any code-running action with an untrusted head checkout
under `pull_request_target`. Pin third-party actions to reviewed full commit
SHAs, grant the job only the GitHub permissions it needs, and avoid printing
`final-message` unless its untrusted model text is safe for your workflow.
