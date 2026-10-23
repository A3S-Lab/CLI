# Completion verification for automation

`a3s code exec --completion-verification require-verification "..."` is the default.

An automation host that intentionally verifies artifacts outside the agent session can select:

```sh
a3s code exec --completion-verification waive-missing-verification "..."
```

This authorizes only missing verification evidence. It does not declare tests passed or grant tool permissions. The completion terminal is `Waived`, not `Verified`. A bound non-passing report, unresolved tool effect, unfinished child or incomplete observation remains blocking. The agent must still return its final response; a tool call alone does not end the run. Existing explicit digest-bound waivers retain their separate behavior.

JSON output includes `completionVerificationPolicy` and `completionTerminal`. No workspace configuration or process-wide environment variable enables this option.

This feature depends on the companion Core change. Historical HTML experiments passed both checks, but did not demonstrate a performance improvement; this PR makes host policy explicit.
