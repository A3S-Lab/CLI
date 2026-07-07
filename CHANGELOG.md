# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
