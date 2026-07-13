# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Made the footer the single owner of live context usage; the composer status
  chip now keeps effort/mode information without duplicating context fill in
  the input border.
- Enabled Core's model-aware rolling compaction for TUI and Code Web sessions.
  Each selected model supplies its actual context window, requests compact
  before overflow, and can compact repeatedly throughout a long-running task.
  Core summaries are written back to each host's durable timeline so later
  turns continue from the latest generation instead of compacting it again.

## [0.7.8] - 2026-07-09

### Fixed

- Hardened DeepResearch against child-task evidence packaging failures: when a
  delegated research track returns source-backed notes but misses the expected
  metadata shape, the workflow now preserves the cited evidence, normalizes it
  into the recursive summary, and continues to synthesis instead of discarding
  useful sources.
- Prevented failed DeepResearch collection from producing false-success reports:
  if no source-backed evidence was collected, the CLI now materializes a
  transparent fallback draft without asking the model to recover current facts
  from memory.
- Cleaned DeepResearch partial-success reporting so final Markdown/HTML reports
  cite original sources and do not expose internal workflow labels, tool logs,
  metadata-normalization details, or stale fallback evidence.

## [0.7.7] - 2026-07-09

### Changed

- Reworked DeepResearch evidence gathering into a bounded, complexity-driven
  recursive parallel retrieval-summary workflow. Local DynamicWorkflowRuntime
  rounds now derive follow-up searches from prior gaps and contradictions,
  stop early when no useful follow-up remains, and keep OS Runtime tool-call
  fan-out disabled until Function-as-a-Service support is available.
- Scaled DeepResearch workflow budgets by query complexity so narrow questions
  get bounded child steps, tool-call/output limits, and host timeouts, while
  broad research still keeps the long-running recursive budget.

### Fixed

- Sanitized DeepResearch partial `parallel_task` failures before synthesis so
  successful structured evidence is preserved while failed child tasks are
  surfaced as compact caveats instead of raw failure blocks.
- Hardened DeepResearch report boundaries so workflow JSON, tool-card
  transcripts, `.a3s-flow` paths, truncated tool-output artifact notices, and
  raw failure diagnostics are withheld from prompts, fallback drafts, final
  report artifacts, and RemoteUI auto-open validation.
- Removed internal runtime/workflow labels from DeepResearch synthesis and
  repair prompts so reports cite original evidence sources instead of host
  implementation details.
- Added a DeepResearch report-phase tool gate: once evidence collection ends,
  synthesis and repair turns can only write or edit `.a3s/research/**` report
  artifacts, and verified reports can be host-cleaned into a final answer when
  the model's text contains artifact-operation narration.
- Added DeepResearch source-trace validation: when gathered workflow evidence
  contains source URLs or local paths, completed Markdown/HTML reports must cite
  at least one of those sources before RemoteUI can open them as final reports.
- Added host-side DeepResearch report completion: if synthesis produces a clean,
  source-traceable `report.md` but stalls before writing `index.html`, the host
  can materialize the HTML view and still validate it before RemoteUI opens.

## [0.7.6] - 2026-07-08

### Changed

- Made the A3S Code TUI default HITL policy risk-aware: read-only inspection,
  web research, safe git reads, and read-only batch calls can proceed without
  prompting, while writes, state-changing commands, delegated work, and unknown
  tools still require confirmation.

### Fixed

- Denied catastrophic shell patterns such as privilege escalation, destructive
  root/home removals, device writes, and `curl|sh` installer pipelines before
  they reach the approval prompt.
- Stabilized the DeepResearch CLI local workflow e2e test fixture so child
  evidence prompts no longer consume scripted report-synthesis responses.

## [0.7.5] - 2026-07-07

### Fixed

- Updated A3S Code Core to `4.3.1` so DynamicWorkflowRuntime scripts can use
  legacy `ctx.tools.<name>(args)` tool calls without bypassing allow-list,
  call-count, or output-size limits.
- Made `?` DeepResearch choose OS Runtime fan-out adaptively: broad,
  multi-source research can use the signed-in runtime, while concise or
  explicitly local tasks stay on the local dynamic workflow path and no longer
  require RemoteUI evidence.

## [0.7.4] - 2026-07-07

### Fixed

- Fixed A3S Code TUI transcript wrapping so the welcome banner, user bubbles,
  streaming Markdown, thinking text, pasted images, and tool cards render for
  the scrollbar-adjusted viewport width instead of wrapping a second time.
- Restored reliable transcript wheel scrolling and drag-to-copy behavior by
  keeping mouse capture enabled while copying the app-managed selection on mouse
  release with clamped viewport coordinates.
- Ignored terminal key-release events in `a3s-tui` so Windows terminals do not
  replay a single key press as duplicate input.

## [0.7.3] - 2026-07-07

### Fixed

- Published the matching `a3s-tui` crate API used by the CLI so crates.io
  verification can build the released package without relying on local
  workspace patches.
- Made the release workflow fail when crates.io or Homebrew publishing
  credentials are missing instead of reporting a misleading successful release.
- Added a crates.io User-Agent and retry policy to release dependency checks so
  GitHub Actions does not misclassify already published A3S crates as missing.

## [0.7.1] - 2026-07-07

### Fixed

- Changed Agent lifecycle commands to treat an Agent asset as a package
  directory, with `agent.md`/`agent.yaml`/`agent.yml` as the entrypoint only.
- Published Agent assets now upload the whole local package source plus
  generated manifest, config, and runtime-binding metadata instead of only the
  entry definition file.
- Updated non-interactive `a3s code agent ...` examples and resolution logic to
  prefer package paths while retaining entry-file compatibility.

## [0.7.0] - 2026-07-06

### Added

- Added DynamicWorkflowRuntime wiring for deep research and ultracode turns,
  including host-side workflow execution visibility in the TUI.
- Added richer A3S Code asset surfaces for agents, MCP servers, skills, OKF
  packages, workflows, asset resources, lifecycle stages, review, publishing,
  deployment, and runtime activity.
- Added expanded context, memory, and knowledge surfaces, including memory
  lifecycle rendering, knowledge-base panels, OKF management, sleep/context
  consolidation views, and durable loop engineering panels.
- Added mouse support across the slash menu, approvals, model picker, theme
  picker, file picker, plugin panel, effort slider, asset pickers, and top
  process table.
- Added shared TUI chrome for transcript gutters, input, footer status,
  reasoning, tool output, diffs, plans, task queues, subagents, help, panels,
  alerts, dividers, progress, banners, and RemoteUI links.

### Changed

- Reworked the Code TUI around shared `a3s-tui` components and AgentChrome so
  interaction, wrapping, status, and panel rendering are consistent across the
  app.
- Expanded the A3S Code TUI README into a capability guide covering everyday
  workflows, safety, OS/Runtime/RemoteUI behavior, asset development, memory,
  knowledge, and dynamic workflows.
- Hardened runtime tool projection and RemoteUI progressive response handling.
- Improved self-update behavior so Homebrew issues can fall back to direct
  release downloads.

### Fixed

- Scoped confirmation cleanup by tool id and kept confirmation streams draining,
  including resume behavior after pending confirmations.
- Confirmed knowledge-base deletion through the shared TUI confirmation flow.
- Refined footer context meter behavior and top process trend rendering.
