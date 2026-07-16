# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.2] - 2026-07-16

### Changed

- Updated the A3S Code Core baseline to 5.3.4 so shared language runtimes keep
  one cancellation-safe startup generation, cannot publish stale readiness
  after workspace removal or shutdown, and prepare a saved source before the
  first workspace-symbol search.

### Fixed

- Restored the Code TUI `/relay` picker for native A3S Code sessions and
  Claude Code or Codex task handoff, added WorkBuddy project transcripts as a
  fourth source, and preserved the selected native session's model, effort,
  execution mode, theme, and paused-goal state during an in-app resume.
- Guaranteed that `a3s code` returns after printing the session-saved hint even
  when filesystem discovery, session cleanup, or a language adapter does not
  settle. Runtime shutdown and cancellation acknowledgement now have explicit
  deadlines, and the interactive updater no longer starts an unowned blocking
  installation repair task.
- Restored packaged Web asset discovery beside release archives and under an
  installation prefix, including Homebrew symlinks into the Cellar. Release
  tests now require `web/index.html`, and the Web integration suite verifies
  code-intelligence route availability and workspace path confinement.

## [0.9.1] - 2026-07-16

### Added

- Projected observed `mcp__use_<route>__*` progress into capability-oriented
  TUI lifecycle labels such as `Using Browser` and `Used Browser`. Routes stay
  ordered and deduplicated, restored tracker snapshots replay the same
  identity, and ordinary workers or unrelated MCP tools are never reclassified.
- Added typed `list`, `info`, `install`, `upgrade`, `uninstall`, and `doctor`
  component lifecycle commands for Code, Box, Bench, Search, Use, and delegated
  Use capabilities.
- Added native Code Intelligence shared by agent tools, the TUI `/ide` editor,
  and A3S Web. The first release provides saved-file symbols, semantic
  navigation, and diagnostics for Rust and TypeScript/JavaScript while reusing
  the existing workspace manifest, file tools, path policy, and editor file
  selection.
- Added the canonical `a3s compose ...` Box route and concise
  `a3s up`, `a3s down`, `a3s ps`, and `a3s logs` application shortcuts. All
  routes preserve raw child arguments, working-directory context, streams, and
  the Box process exit status while retaining verified first-use installation.
  Box now discovers canonical `compose.acl` project files through the same
  transparent route while explicit Compose YAML remains available.
- Added `a3s use box ...` as a component-backed route. The root remains the
  sole Box installer and receipt owner, injects one canonical executable into
  Use, preserves native argv and status, and never auto-installs Box for
  unrelated Use commands. External Use domains now expose generation-based
  enable, disable, snapshot, and watch operations with graceful route draining.
- Added unified A3S Use capability hot-plug for Code TUI and Web sessions,
  including one shared Web watcher, generation-driven MCP/Skill projection,
  session-rebuild replay, bounded startup discovery, background recovery, and
  a permission-isolated `use` worker. Capabilities converge across install,
  upgrade, disable, and re-enable without restarting A3S Code.

### Changed

- Reorganized the umbrella command surface around typed Clap trees, immutable
  invocation context, explicit application services, and registered component
  proxies. Unknown top-level commands no longer discover and execute arbitrary
  `a3s-*` binaries from `PATH`.
- Made macOS and Linux the current component/runtime support targets; Windows
  remains a compile/package preview until its managed lifecycle and persistent
  Browser conformance gates pass.
- Promoted the dedicated A3S Use worker into the live `task` and
  `parallel_task` catalog with current capability IDs, MCP routes, and Skill
  guidance. It returns observable application evidence and never falls back to
  shell, workspace, unrelated MCP, or recursive delegation.
- Updated the A3S Code Core baseline to 5.3.2 so packaged CLI builds include
  the live worker-definition contract used by the dedicated Use worker and
  position-aware HITL classification for shell commands.
- Pinned standalone releases to ACL 0.2.2, Boot 0.1.2, Lane 0.5.1, and Updater
  0.3.0 so packaged builds retain bounded nested-block parsing, typed HTTP
  failures, deterministic queue lifecycle, and checksum-verified ZIP updates
  without depending on monorepo paths.

### Fixed

- Kept `--version` inside native product proxies so `a3s use --version` and
  sibling proxy commands report the delegated component version instead of the
  umbrella CLI version.
- Completed the Parent → Use worker → live `mcp__use_*` path for hot-plugged
  capabilities. Office `use.office.outcome_unknown` mutations are surfaced as
  potentially applied and are never retried automatically.
- Capped provider-facing child concurrency at eight for every interactive
  effort profile while retaining larger reasoning, tool-round, and continuation
  budgets. Ultracode and `/goal` now schedule larger workloads in bounded waves
  instead of bursting one signed-in provider account.
- Made `/goal` execute maker and verifier as dependency-ordered phases, with
  parallelism limited to independent read-only work inside each phase. Goal
  state writes are now checkpointed at five-percent progress boundaries and
  unchanged runtime sections are not rewritten.
- Rendered per-branch `parallel_task` output excerpts, retry attempts, and
  recovered branch counts without repeating the complete batch output in every
  plan step or DeepResearch synthesis prompt.
- Replaced the TUI-local pending-turn heap with `a3s-lane`'s stable typed
  priority queue. Explicit user messages now outrank automatic continuations,
  equal-priority turns remain FIFO, failed stream admission restores the exact
  queue item without losing image attachments, and Esc settles the real Core
  worker before consuming one queued successor. The bottom queue strip now
  projects pending turns only, so a claimed message disappears as soon as its
  execution begins.
- Stopped workspace discovery before the Code TUI and Web host tear down their
  Tokio runtimes. In-progress directory scans and Git file enumeration now
  observe cancellation, so quitting from a large workspace or home directory
  no longer hangs after the session-saved message. TUI session close and
  language-service cleanup also have host-level deadlines, and both startup
  and interactive `/update` checks now terminate their child process when the
  TUI closes, so unresponsive background work cannot block exit indefinitely.

## [0.8.2] - 2026-07-15

### Fixed

- Kept locally signed-in Codex accounts and their picker-visible models
  available when the shorter-lived identity token has expired but reusable
  Codex access state remains. Account refresh and entitlement validation stay
  delegated to the installed Codex CLI instead of treating ID-token expiry as
  a logout.
- Parsed WorkBuddy `hy3` tagged tool-call envelopes and withheld protocol
  markup across split streaming deltas. Tool calls now become native A3S tool
  events instead of exposing `<tool_call:...>` or closing XML tags in TUI
  messages.

## [0.8.1] - 2026-07-14

### Fixed

- Matched Codex Markdown section spacing by keeping exactly one blank terminal
  row before and after headings. A new section no longer touches the final line
  of the preceding paragraph, while existing code/table/heading separators are
  not doubled.

## [0.8.0] - 2026-07-14

### Added

- Added signed-in WorkBuddy account models to `/model`, `a3s model`, and
  `a3s code models`. The integration reuses WorkBuddy's bundled CodeBuddy CLI
  and local account state without reading or copying private tokens, discovers
  the models enabled for the account, streams responses, and preserves native
  A3S tool execution.

### Changed

- Consolidated Claude Code, Codex, and WorkBuddy integrations under the
  `account_providers` boundary. Claude and WorkBuddy share one cancellable
  account-CLI stream/tool bridge, while all three share account detection,
  client construction, model switching, persistence, and session restore.
- Unified TUI `/compact` with the direct, tool-free compactor used by Code Web.
  Repeated manual compaction now includes the previous summary without creating
  a temporary tool-capable agent session, while Core's rolling auto-compaction
  remains re-armed for long-running conversations.
- Rebuilt `/goal` as a durable Ultracode goal loop. Setting a goal now creates
  a complete `.a3s/loops/goal-*` Loop Engineering workspace, forces planning
  and goal tracking, runs separate maker/verifier guidance, and continues
  across normal ends and retryable errors until a matching `GoalAchieved`
  arrives. Esc and `/goal clear` invalidate delayed retries immediately, and
  normal Ultracode message-gated planning is restored when the goal closes.
- Aligned fenced Markdown code with Codex-style terminal highlighting: known
  languages retain distinct token colors, unknown languages stay plain, CRLF
  is normalized, and 512 KiB / 10,000-line guardrails avoid render stalls.
- Replaced width-unstable colorful emoji in the transcript, task queue,
  thinking indicator, and `/ide` tree with monochrome terminal-safe marks and
  consistent hair-space padding.
- Made the footer the single owner of live context usage; the composer status
  chip keeps effort and mode information without duplicating context fill in
  the input border.
- Enabled Core's model-aware rolling compaction for TUI and Code Web sessions.
  Each selected model supplies its actual context window, requests compact
  before overflow, and can compact repeatedly throughout a long-running task.
  A3S Code Core 5.2.4 budgets the retained suffix by estimated message tokens,
  bounds oversized summaries, and refuses replacements that would not reduce
  context. Core summaries are written back to each host's durable timeline so
  later turns continue from the latest generation instead of compacting it
  again.
- Moved model, effort, Ultracode, goal, auth, reload, fork, and clear session
  changes onto an async atomic replacement path. The UI no longer blocks the
  Tokio runtime, failed reconfiguration keeps the old session usable, and
  `/goal` can reliably enter forced-planning Ultracode before its first turn.
- Routed A3S Code 5.2.2 native structured output through TUI launch, configured
  model selection, effort rebuilds, and headless DeepResearch. Codex Responses
  and Responses Lite now force the schema function through `tool_choice`, while
  providers with verified JSON Schema support retain that path and unknown
  custom OpenAI-compatible endpoints keep the safe prompt fallback.
- Clarified that web-and-workspace scope describes available evidence tools,
  not mandatory tracks. The semantic planner now reserves workspace collection
  for queries that explicitly depend on a repository or local artifacts.
- Replaced the normal public-investigation `direct_then_maker` route with
  `direct_then_review`. Multi-query retrieval now flows into one structured
  synthesis-and-coverage review, removing a redundant slow model turn while
  preserving `direct_then_maker` replay compatibility for older journals.
- Expanded the LLM-selectable public evidence envelope to four searches and
  eight parallel fetches. Query-specific candidates are fetched before seed
  URLs, page excerpts are ranked against their owning evidence question, and a
  zero-result unconfigured search receives one bounded Brave fallback.
- Gave the independent checker a 180-second clock within the unchanged
  300-second workflow fuse and carry observed checker latency into subsequent
  scheduling decisions. Public-source gaps route to direct retrieval; makers
  remain reserved for evidence production or required local/non-web work.

### Fixed

- Corrected the real-LLM compaction integration test to compare matched
  compressed and uncompressed histories. It now proves provider-reported
  prompt reduction on the compacted request and again after session restore,
  instead of comparing two already-compacted turns.
- Preserved traceable structured evidence when the independent DeepResearch
  checker times out. The workflow now completes with an explicit degraded
  verification state and publishes a provisional evidence-derived report;
  only runs without reportable evidence fall back to a Recovery artifact.
- Kept explicit checker URLs out of finding prose and bound each one to its
  matching source card, preventing Chinese terminal punctuation from becoming
  part of an auto-linked URL and avoiding unrelated citations on a finding.
- Made long report headings prefer a complete semantic clause, bounded the
  caveat section to eight reader-relevant items, and added a mobile horizontal
  scroll cue to wide evidence matrices.
- Prevented structured no-tool makers from being used for checker-requested
  evidence collection. Existing-evidence synthesis is now an explicit legacy
  optimization, while new gaps retain tool-capable collection semantics.
- Tightened checker output so findings state supported facts instead of merely
  announcing that sources or comparisons exist, and requested recommendations
  must give a conditional answer or remain an explicit evidence gap.

## [0.7.9] - 2026-07-14

### Added

- Added durable Code Web sessions backed by Core `FileSessionStore` snapshots
  under `~/.a3s/code-web`. `a3s code serve` now restores conversation and run
  events after restart, keeps stable titles/timestamps, retains bounded Web-only
  `/help`, shell, fork, and structured-event display history, and deletes both
  live and persisted state through the Kernel session API.
- Added a unified account and model-routing surface: `a3s account
  list|status|login|logout` and `a3s model list|current|use|reset`. Configured
  providers, Claude Code, Codex, and A3S OS Gateway models now have explicit
  route identities while product OAuth credentials remain in their owning
  stores. Model selection is persisted separately from `config.acl`.
- Added `a3s login [token]` and `a3s logout` as main CLI commands for A3S OS
  browser OAuth, bearer-token login, and local session removal. The existing
  `a3s code login` and `a3s code logout` forms remain compatibility aliases.
- Added `a3s search` management commands for engine discovery, configuration
  diagnostics, and explicit Chrome/Lightpanda list, install, update, and repair
  operations. `a3s list` now includes the managed search runtimes.
- Added one component lifecycle under the `a3s` command:
  `a3s install code|box|bench`, `a3s list`, and
  `a3s update [code|box|bench]`. Code remains bundled by default, while Box and
  the private Bench control component install only on explicit install or first
  real use.
- Added validated, version-isolated Bench control-component installation under
  `~/.a3s/components/bench/`, including release and payload digests, manifest
  and protocol checks, atomic activation, stale-lock recovery, and local
  health reporting without adding a second public executable to `PATH`.
  The component plans and evaluates benchmark runs but delegates Candidate and
  Judge Agent Asset execution exclusively to A3S OS Runtime.
- Refreshed the signed-in Codex account model catalog asynchronously for the
  TUI and `a3s code models`, exposing every picker-visible account model,
  including GPT-5.6 Sol, Terra, and Luna when entitled.
- Added native Codex reasoning-effort support to `/effort`. The `low` through
  `max` profiles request their same-named `reasoning.effort`, while `ultracode`
  keeps A3S orchestration and requests the maximum Responses wire effort (`max`
  for GPT-5.6 account models and `xhigh` for older GPT models). Unsupported
  requests clamp downward without disabling the selected profile's host budgets.

### Changed

- Refined the `/ide` and `/config` editor with a terminal-safe file icon
  system, semantic type colors, directory disclosure rows, icon-bearing panel
  titles and metadata, and a quieter ruled line-number gutter. The shared `@`
  file picker now uses the same icon source of truth.
- Decoupled native Codex reasoning effort from A3S automatic delegation. The
  `low` through `max` levels keep their native reasoning settings without
  runtime-driven fan-out; `ultracode` remains the automatic orchestration mode,
  and synthesis-only continuations cannot recursively delegate.

- DeepResearch report turns now return Markdown without model-side file tools.
  The host validates the content against accepted evidence, atomically publishes
  the Markdown/HTML pair, and appends the trusted view marker only after both
  artifacts pass validation. This removes long `write` calls from the report
  critical path while A3S Code 5.2.2 keeps offset-checked, idempotent append mode
  available for ordinary large workspace files.
- Made the LLM planner choose an explicit `direct_only`, `direct_then_maker`,
  or `maker_first` execution route together with the report title, phases,
  tracks, and budgets. A substantial public investigation can now run A3S Code
  5.2.2 capability-governed batch search/fetch, immediately fan those sources
  into planned maker tracks, and invoke its first checker only after cumulative
  direct and maker evidence exists. Narrow lookups still use direct retrieval
  plus a checker, while evidence-production and workspace work can begin with
  makers. No topic, query-length, answer-shape, or track-count rule overrides
  the LLM route. Each maker may use its second evidence round to close a
  consequential gap instead of being forced to stop after one seed fetch.
- Carried the checker's reader-facing summary, verified findings, unresolved
  gaps, contradictions, and the planned title into both normal synthesis and
  deterministic timeout materialization. Generated reports no longer turn
  collection-status prose into findings, duplicate punctuation, or claim that
  no gaps exist after the checker recorded explicit limitations.
- Reduced model-command latency by validating `a3s model use` against only the
  selected credential source, running Codex and A3S OS catalog discovery in
  parallel, and parsing `config.acl` only once during `a3s model list`.
- Fixed TUI model and effort switching for live persisted sessions. Session
  rebuilds now save and close the current session before resuming the same
  durable ID, and restore the previous configuration if the requested rebuild
  fails, avoiding the `session is already live or being built` error.
- Improved JSON in streamed and completed assistant messages. Complete bare
  JSON values and complete `json` fences are now pretty-printed through the
  shared Markdown renderer, while partial streaming fragments remain untouched
  until they form valid JSON.
- Removed duplicate `Updated Plan` transcript cells; planning events now update
  only the pinned checklist above the input. The pinned parallel-subagent
  tracker also disappears immediately once every child reaches a terminal
  state, and completed turns clear the pinned plan instead of retaining stale
  task tracking UI.
- Collapsed persistent TUI state into one Codex-style footer row. Mode and
  context now have one owner, transient work stays in the activity row, and
  typed parallel-child outcomes distinguish succeeded, failed, cancelled, and
  tracking-lost tasks without duplicating agent counts in the footer.
- Restored conventional diff colors in the TUI: additions and `+N` header
  counts use green, while deletions and `-N` counts use red, with matching dark
  backgrounds for changed lines.
- Made interactive TUI DeepResearch synthesis and repair deadlines
  activity-based and independent from report tool execution, so an in-flight
  report write is not cancelled merely because the model synthesis timer
  reached its deadline.
- Closed the DeepResearch evidence-to-report boundary. Synthesis, recovery,
  repair, and verification now use only the bounded evidence package; retrieval,
  batch, shell, Git, delegation, program, runtime, and workflow definitions are
  removed from report-model requests and remain denied by the execution gate.
  A failed/degraded collection immediately publishes an explicit recovery
  report instead of starting another retrieval or 180-second synthesis cycle.
- Reserved final-response capacity in every delegated evidence task: children
  receive at most two high-signal evidence-tool rounds plus one provider turn
  for A3S Code v5.2.2 structured-output validation and repair.
- Removed topic-specific DeepResearch shortcuts, classifiers, query templates,
  source allowlists, report headings, and visual themes. TUI and headless runs
  now share the same LLM-planned generic collection loop for every subject.
  The checker routes a narrow externally retrievable gap to one bounded direct
  follow-up and reserves maker delegation for multi-step analysis or local
  evidence work. Follow-ups use unique replay-safe step IDs and cumulative
  evidence; non-actionable continuations degrade instead of looping.
- Aligned streamed tool cards more closely with Codex-style semantic coloring:
  executable names and paths are cyan, flags and parameter names are yellow,
  and parameter values are green while preserving the original command text.
- Replaced repeated JavaScript-wrapper previews in `program` cards with a
  source-free semantic summary derived from structured inputs. DeepResearch
  calls now show the query, evidence scope, bounded parallel plan, and current
  workflow phase; completed nested calls are aggregated into one bounded result
  row. Workflow handoff stores the same intent summary instead of copying the
  full PTC source back into synthesis context.
- Restored DeepResearch workflow executability after its embedded script grew
  beyond the runtime's 64 KiB source limit, and added a source-size regression
  gate. Delegated tasks now count as successful evidence only after schema and
  source-anchor verification; rejected or fabricated evidence contributes to
  failed/partial status instead of sending an empty package down the successful
  report path.
- Expanded offline `a3s bench --help` to show the canonical
  `list`/`info`/`run`/`result` interface and local Task path rule before the
  private control component is installed.
- Made local DeepResearch a first-class TUI and headless CLI path with explicit
  `web + workspace` versus `offline/local-only` evidence labels, a visible
  evidence-to-report handoff, and distinct completed and low-confidence
  recovery outcomes.
- Made DeepResearch evidence scope an explicit typed setting. CLI callers can
  use `--local-only`/`--offline` or `--web`, while the TUI accepts the same
  scope after `?`; natural-language intent remains only a compatibility
  fallback, and the selected scope drives both workflow inputs and network
  permission enforcement. Research-mode input now displays the available scope
  switches, and delegated/runtime branches receive scope-specific tool and
  prompt contracts instead of reinterpreting query wording. New workflow runs
  emit only `evidence_scope`; the legacy `direct_web_enabled` field is accepted
  only when replaying older inputs.
- Expanded the sanitized DeepResearch TUI/tool-card summary with direct-web
  source, host, fetch, topic, fetched-text, date, and bounded-query coverage.
  Hybrid runs now surface their web seed alongside delegated task progress, so
  users can see why direct completion was accepted or declined without opening
  raw workflow diagnostics.
- Rebuilt TUI tool-call rendering around stable call IDs and source-backed
  transcript entries. Preparing, approval, running, denied, interrupted, and
  completed calls now update in place; parallel completion order cannot reorder
  calls. Exec, Explore, Web, MCP, file-change, task, program, and dynamic
  workflow cells use Codex-style `•`/`└`/`│` grammar and reflow after resize.
- Made streamed Markdown newline-gated and lossless, with stable and mutable
  table regions, adaptive stable-line commit/catch-up pacing, cached mutable-tail
  viewport replacement, source-backed resize, grapheme-safe wrapping,
  ANSI-contained rows, and a key/value fallback for narrow tables instead of
  truncated cells.

### Removed

- Removed the TUI-only `/btw` background side conversation, including its
  command, asynchronous session path, message/state handling, overlay, input
  tint, tests, and documentation. The primary agent conversation is unchanged.
- Removed the TUI `/output` and `/top` commands and their dedicated panels;
  `Ctrl+T` remains the complete semantic transcript, and local process
  inspection remains available through the standalone `a3s top` command.

### Fixed

- Waited for the previous model stream's lifecycle handle before constructing
  synthesis, loop, DeepResearch, or queued continuations. Terminal events can
  precede persistence cleanup and release of the session's single-flight lease;
  stale stream starts are now cancelled on their originating session and joined
  instead of detaching the public handle, preventing `already has an active
  operation` failures between turns.
- Settled every child task observed during a DeepResearch run before opening
  its terminal report view or restoring the previous mode. Live children are
  cancelled through the core tracker, missing cancellers become explicit
  tracking-lost outcomes, and late watcher messages or stale tracker snapshots
  cannot recreate the footer. Esc interruption uses the same scoped settlement
  path without opening an incomplete report.
- Kept direct retrieval and maker child timeouts independent. A slow or
  partially successful maker pass no longer consumes the direct-retrieval clock
  before the checker can request one bounded recovery. The next checker receives
  all initial and follow-up direct evidence together with the retained maker
  evidence, so successful recovery can finalize instead of degrading from a
  stale package.
- Forwarded planner-provided seed URLs into maker prompts, removed the implicit
  raw-success-count early cancellation that could discard later valid evidence,
  and routed a fully failed maker-first pass through one direct evidence recovery
  before the independent checker. Failed Flow step IDs are never rescheduled.
- Bounded each maker to a compact evidence card: at most four sources, five
  decision-relevant facts, and short summaries, contradictions, confidence, and
  gaps. Research depth now comes from independent planner tracks instead of
  making every child spend its deadline writing a miniature report.
- Excluded navigation chrome, CSS, link-target-only keyword matches, and common
  repository-page boilerplate from direct-web evidence snippets and titles.
  Relevant visible text now carries adjacent context so fetched README and
  documentation facts are retained instead of menu rows.
- Made `/exit` and confirmed Ctrl+C close the active session and settle or abort
  its stream before quitting, preventing detached work and stale single-flight
  leases from surviving process shutdown.
- Reserved the final ten seconds of Smoke DeepResearch's six-minute absolute
  deadline for cancellation and recovery artifact publication. Workflow,
  synthesis, and repair phases can no longer consume the time required to write
  an explicit degraded report.
- Excluded leading freshness phrases and date-only tokens from headless
  DeepResearch entity extraction, preventing queries beginning with
  `As of <date>` from being misread as a comparison of crates named `as` and
  `of`.
- Closed and reaped execution-scoped Chrome and Lightpanda processes on search
  success, error, timeout, and cancellation. Browser tabs now remain guarded
  until explicit close, pool shutdown owns child cleanup in a detached task,
  and code-core no longer keeps a session-wide headless pool that can survive a
  cancelled `web_search` invocation.

- Bound DeepResearch report-phase writes to the current query's exact
  `.a3s/research/<slug>/report.md` and `index.html` pair, rejected unchanged
  same-query artifacts from earlier runs, and preflighted/staged both files
  with rollback before publishing a replacement generation.
- Filtered delegated DeepResearch sources against successful child tool
  provenance before follow-up planning or synthesis. Fabricated URLs and paths
  are now omitted, while runtime-observed URLs are stripped of credentials,
  fragments, and query parameters before reaching reports.
- Hardened DeepResearch provenance across live and recovered evidence: failed
  runtime branches, nested evidence-shaped text or objects, and case-changed
  resource paths cannot become structured evidence. Current-run `Completed`
  validation now requires a recognized citation in the Markdown source/reference
  section and at least one recognized HTML citation, and rejects the report when
  any recognized HTTP(S), link-target, or path citation does not canonically
  match an observed anchor. Canonicalization is comparison-only and does not
  rewrite report files. The headless CLI reuses recent same-query durable
  evidence only when its scope and completed-evidence contract still match;
  recovery reports remain explicitly low-confidence and are rejected by
  `Completed` artifact validation.
- Treated user-supplied DeepResearch queries as escaped single-line Markdown in
  host-generated completed, evidence-rebuilt, and recovery artifacts, preventing
  query text from injecting report sections while keeping HTML titles readable.
  Exact query-title URLs no longer count as report citations, while the same URL
  remains provenance-checked everywhere outside that title. The exemption also
  requires every title link target to exactly match the safe projection of a
  complete HTTP(S) URL token in the query, and only exact `href`/`src` attributes
  are recognized as links. Matching canonicalizes Unicode/percent-encoded forms
  after stripping credentials, query parameters, and fragments, preventing
  renderer differences or query secrets from leaking into durable artifacts.
  HTML rebuilt from Markdown without an H1 now applies the same safe projection
  to its fallback `<title>`.
- Preserved balanced parentheses in bare HTTP(S) citations and Markdown link
  targets, while continuing to trim unmatched closing punctuation, so valid
  resources such as `spec_(v2)` remain exactly traceable. Host-generated query
  Markdown also keeps complete URL tokens intact instead of escaping URL
  punctuation into a different rendered `href`.
- Propagated recovered workflow metadata into low-confidence recovery reports,
  so successful source anchors survive when a timeout leaves no final workflow
  output; the evidence-status section now reports those anchors consistently for
  empty output, timeout/error text, and withheld internal logs, distinguishing
  each case from a run with no captured sources.
- Made the 12-source recovery view explicit when additional verified sources are
  bounded out. The sanitized evidence digest carries only an omitted count, so
  reports can disclose "at least N more" without rehydrating truncated raw
  source objects. Omitted entries are now summed across independently bounded
  evidence items and combined with retained-but-hidden anchors, while duplicate
  output/metadata projections are counted only once. The recovery report also
  discloses evidence items omitted by the separate 18-item digest boundary.
- Filtered and safely projected evidence sources before applying the 12-source
  digest boundary, so invalid schemes cannot crowd out later traceable sources
  and URL credentials, query parameters, or fragments cannot enter synthesis
  prompts. Evidence deduplication now uses the first traceable source rather
  than the first raw source entry, preserving distinct evidence whose leading
  entry is invalid.
- Tightened direct-web relevance gating so generic release/version language or
  an authoritative-looking documentation URL cannot make an off-topic result
  count as evidence. At least one substantive query term must now match, and
  non-JSON URL fallback parsing no longer copies shared output text onto every
  candidate URL. ASCII terms use token boundaries, preventing queries such as
  `rust` from matching unrelated `trust`, `trusted`, or `rustic` text. Meaningful
  two-character entities such as `Go` and `AI` are retained rather than
  degrading the query into an unscoped search.
- Required successful `web_fetch` page text to match a substantive query term
  before counting it as verified evidence or enabling the direct-web fast path.
  Off-topic redirects/pages now fall back to the search snippet with an
  explicit bounded warning and lower confidence.
- Required the narrow direct-web fast path to cover every substantive query
  term across its retained search and fetched evidence. Two independent hosts
  can no longer end a comparison early when both only cover the same entity;
  coverage counts are preserved in bounded workflow and hybrid seed digests.
- Distinguished search-result coverage from successfully fetched page coverage.
  The fast path now requires every substantive query term to appear across
  verified page text, so a comparison cannot complete when one entity exists
  only in an unverified search snippet; both coverage ratios remain diagnostic.
- Excluded HTTP(S) URL tokens from fetched-page relevance and entity coverage
  matching. A fetch tool or page that echoes the requested URL can no longer
  verify an entity found only in that URL path while its actual text is silent.
- Added direct-web end-to-end coverage for URLs embedded inside fetched evidence
  text: credentials, query parameters, and fragments are removed from source
  quotes, key evidence, workflow output, and the bounded synthesis digest.
- Preserved `published_date` (plus compatible `publication_date`/`date` aliases)
  from direct web search results as sanitized evidence `date` fields, allowing
  synthesis to compare recency for current-version and news research.
- Merged complementary metadata across canonical duplicate search results.
  A later query or engine can now supply a missing publication date and engine
  provenance without inflating source counts; direct-web dates are bounded to
  compact evidence fields.
- Added explicit freshness intent and dated-source coverage to direct-web
  metadata. Queries asking for the latest/current/release state now require at
  least one dated source before using the fast path; otherwise they continue to
  delegated research even when topic and fetched-page coverage are complete.
- Excluded placeholder publication dates such as `unknown`, `N/A`, `undated`,
  and localized unknown-date markers from evidence and freshness coverage, so
  nonempty engine placeholders cannot satisfy the dated-source fast-path gate.
- Normalized common ASCII compound-name separators during direct-web relevance
  matching, so `a3s-code`, `a3s_code`, and `A3S Code` evidence agree without
  weakening token boundaries. Chinese query extraction now removes research
  instruction phrases before forming bigrams, avoiding synthetic terms that
  span words such as “全面调研” or “最新”.
- Bounded direct-web relevance analysis to 48 substantive query terms and an
  8,192-character input window. Truncation is explicit in workflow and synthesis
  metadata and disables the fast path, so adversarial high-cardinality queries
  cannot grow matching work without limit or complete from partial coverage.
- Safely projected search-result URLs before invoking `web_fetch` or composing
  fetch diagnostics, preventing credentials, query tokens, and fragments from
  entering tool arguments or workflow logs.
- Safely projected HTTP(S) URLs embedded in the user's research query before
  constructing `web_search` requests. Search engines receive the useful base
  URL while credentials, query parameters, and fragments stay out of tool
  arguments and bounded failure diagnostics.
- Deduplicated search results by the same safe canonical URL used for evidence
  and fetch calls, so credential/query/fragment variants of one resource cannot
  inflate `source_count` or satisfy the multi-source fast path. Canonical keys
  also normalize trailing slashes to match final citation validation, while
  preserving case-sensitive resource paths and normalizing only scheme/host
  casing. Search-result scheme validation is case-insensitive, so valid
  `HTTPS://` results reach canonicalization instead of being discarded. Explicit
  HTTP `:80` and HTTPS `:443` default ports are removed before deduplication,
  fetch, evidence, and host coverage accounting.
- Added distinct `host_count` coverage to direct-web metadata and require at
  least two hosts for the narrow-query fast path, preventing multiple pages
  from one site from being mistaken for independent corroboration. Host coverage
  ignores ports (including non-default ports) while canonical source URLs retain
  meaningful non-default port distinctions.
- Prioritized one result per host when selecting direct-web fetch candidates and
  added `fetched_host_count`. Fast-path completion now requires verified page
  text from at least two hosts, rather than allowing the second host to exist
  only as an unfetched search snippet; the TUI summary exposes both host counts.
- Added an end-to-end positive fast-path contract: a narrow run with two hosts,
  at least one relevant fetched page, and no partial failure completes directly
  without scheduling delegated local research.
- Updated TUI, CLI, and Code Web dynamic workflow/tool registration call sites
  for the fallible registration API, eliminating unused `Result` warnings
  introduced by the core registry hardening.
- Preserved direct-web search, source, host, fetch, and verified-fetch counts in
  the bounded synthesis/diagnostics digest, so coverage and confidence decisions
  remain visible after raw workflow metadata is compacted.
- Preserved balanced parentheses in URLs extracted from non-JSON search output
  while trimming unmatched closing punctuation, aligning direct-web fallback
  parsing with final report citation handling.
- Kept DeepResearch workflow complexity, host timeout budgets, and local-only
  wording in sync; `no web` and `no-web` now disable both direct and delegated
  web collection as documented.
- Expanded explicit offline/local-only intent recognition across common English
  and Chinese phrases such as `local-only`, `without web`, `stay offline`,
  `仅本地`, `离线调研`, and `不联网`. Product discussions about an “offline
  mode” still use web evidence when the query explicitly requests current web
  documentation.
- Disambiguated double-negative network language: phrases such as “cannot
  research without web”, “requires web access”, and `需要联网` retain web
  evidence, while an explicit `do not use web`/`不联网` instruction still wins
  when the query discusses those phrases as a topic.
- Waited briefly for timed-out DeepResearch workflows to quiesce before reading
  durable Flow recovery state, and added random nonces to workflow run IDs to
  avoid concurrent same-process collisions.
- Stored all project-local dynamic workflow history exclusively under
  `.a3s/workflow`; DeepResearch no longer reads or references the retired
  sibling workflow-state directory.
- Reported signed-in Codex `usage_limit_reached` responses with the plan and
  local reset time, while skipping duplicate streaming fallbacks and circuit
  retries that cannot succeed before the account quota resets.
- Normalized the Codex catalog's product-only `ultra` tier to the Responses wire
  value `max`, preventing `reasoning.effort=ultra` HTTP 400 failures.
- Prevented inactivity review from treating UI status notices as conversation,
  re-running after navigation keys, or displaying stale background results
  after a new turn or session change.
- Added the Responses Lite request contract and catalog-provided context
  windows for GPT-5.6 account models, so newly discovered models can execute
  tools and compact at the correct context limit instead of only appearing in
  the picker.
- Honored `$CODEX_HOME` when resolving Codex auth and model-cache files.
- Added Codex-style `Calling`/`Called` fallback cards for dynamically registered
  tools and dedicated structured-output and skill-search verbs. `Ctrl+T` now
  opens the complete live semantic session transcript with user and assistant
  messages, plans, every tool lifecycle and full tool output, subagent state,
  and the current streaming tail.
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
  transcripts, workflow diagnostic paths, truncated tool-output artifact notices, and
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
