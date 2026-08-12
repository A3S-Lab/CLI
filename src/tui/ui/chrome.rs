//! Shared TUI palette, chrome, command metadata, model routing, and context hints.

use super::*;

/// Codex-aligned semantic palette for the dark terminal surface.
///
/// Keep roles distinct: accent is interactive, green/red are outcomes, muted
/// text is quieter than borders, and selected rows use a neutral surface
/// instead of a saturated full-width fill.
pub(super) const CANVAS: Color = Color::Rgb(21, 25, 31);
pub(super) const ACCENT: Color = Color::Rgb(125, 182, 255);
pub(super) const TN_GREEN: Color = Color::Rgb(78, 201, 139);
pub(super) const TN_YELLOW: Color = Color::Rgb(215, 168, 75);
pub(super) const TN_RED: Color = Color::Rgb(224, 108, 117);
pub(super) const TN_CYAN: Color = Color::Rgb(110, 198, 217);
pub(super) const TN_ORANGE: Color = TN_YELLOW;
pub(super) const TN_PURPLE: Color = Color::Rgb(182, 155, 241);
pub(super) const TN_FG: Color = Color::Rgb(220, 220, 220);
pub(super) const TN_GRAY: Color = Color::Rgb(120, 123, 125);
pub(super) const TN_SUBTLE: Color = Color::Rgb(95, 99, 104);
pub(super) const BORDER_SUBTLE: Color = Color::Rgb(52, 58, 64);
pub(super) const SURFACE_SOFT: Color = Color::Rgb(27, 31, 37);
pub(super) const SURFACE_USER: Color = Color::Rgb(49, 53, 58);
pub(super) const SURFACE_SELECTED: Color = Color::Rgb(42, 46, 52);

/// Low-chroma palette for the persistent surfaces around the composer.
///
/// These panels remain visible while the user reads the transcript, so their
/// active and outcome colors are intentionally quieter than the global accent.
/// Color communicates state on glyphs; text hierarchy stays neutral.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ComposerChromePalette {
    pub(super) primary: Color,
    pub(super) secondary: Color,
    pub(super) faint: Color,
    pub(super) active: Color,
    pub(super) success: Color,
    pub(super) warning: Color,
    pub(super) error: Color,
}

pub(super) const COMPOSER_CHROME: ComposerChromePalette = ComposerChromePalette {
    primary: Color::Rgb(210, 214, 220),
    secondary: Color::Rgb(139, 147, 158),
    faint: Color::Rgb(94, 103, 114),
    active: Color::Rgb(137, 161, 199),
    success: Color::Rgb(126, 164, 143),
    warning: Color::Rgb(188, 157, 105),
    error: Color::Rgb(197, 120, 128),
};

// A3S brand color is intentionally separate from the neutral Codex-aligned
// semantic palette above. It is reserved for short, explicit Ultracode
// transitions so ordinary transcript and composer chrome stay calm.
pub(super) const BRAND_GRADIENT: [Color; 8] = [
    Color::Rgb(86, 156, 255),
    Color::Rgb(70, 214, 255),
    Color::Rgb(76, 230, 190),
    Color::Rgb(249, 211, 92),
    Color::Rgb(255, 139, 92),
    Color::Rgb(255, 101, 155),
    Color::Rgb(190, 124, 255),
    Color::Rgb(116, 133, 255),
];
pub(super) const ULTRACODE_ANIMATION_TICK: Duration = Duration::from_millis(60);
pub(super) const ULTRACODE_CONFIRM_ANIMATION: Duration = Duration::from_millis(1_140);
pub(super) const ULTRACODE_BORDER_ANIMATION: Duration = Duration::from_millis(2_520);

pub(super) fn agent_chrome_theme() -> TuiTheme {
    TuiTheme {
        primary: ACCENT,
        secondary: TN_CYAN,
        bg: CANVAS,
        fg: TN_FG,
        muted: TN_GRAY,
        border: BORDER_SUBTLE,
        success: TN_GREEN,
        warning: TN_ORANGE,
        error: TN_RED,
        info: TN_CYAN,
        surface: SURFACE_SOFT,
        highlight: SURFACE_SELECTED,
    }
}

pub(super) fn agent_chrome(theme: &TuiTheme) -> AgentChrome<'_> {
    AgentChrome::new(theme)
}

/// Self-contained system-prompt directive injected ONLY when signed in to the OS
/// platform. It disambiguates "OS" (the user means the signed-in OS open
/// platform, not this machine's operating system) AND inlines exactly how to call
/// the progressive API, so the model can act immediately — without first
/// discovering/loading the `a3s-os-capabilities` skill (that extra hop is why a
/// passive catalog entry rarely triggered: the model fell back to `whoami`).
/// `base_url` is the signed-in address so the endpoint is concrete.
pub(super) fn os_platform_guide(base_url: &str) -> String {
    format!(
        "[OS platform] You are signed in to the OS open platform at {base_url} (via /login). \
DEFAULT RULE: while signed in, \"OS\" in the user's questions ALWAYS means THIS OS platform — \
never this machine's operating system. So \"what's my OS account\", \"what modules does OS have\", \
etc. are about the platform. Answer them via the platform's progressive API; do NOT answer from \
this machine (whoami / hostname / paths / working directory describe the local box, not the \
platform — they are the WRONG answer). The endpoint and auth token are ALREADY in your shell \
environment (exported at login) — use them directly; do NOT read ~/.a3s/os-auth.json or any config \
file on each call:\n\
  curl -s -X POST \"$A3S_OS_BASE_URL/api/v1/kernel/capabilities\" \
-H \"Authorization: Bearer $A3S_OS_TOKEN\" -H 'Content-Type: application/json' \
-d '{{\"action\":\"list\"}}'\n\
Body fields: `action` = list|search|describe|execute, plus `module` / `operation` / `params`. \
Go broad→narrow: `list` (modules) → `describe`/`search` for the one operation → `execute`. \
For `list`/`search`/`describe`, pipe through `jq` to extract only the fields you need so output \
stays a few lines (e.g. `| jq -r '.data.modules[].name'`). \
For `execute`, ALWAYS add `\"shaped\":true` to the request body — that is what makes the response \
carry the `.view` popup deep-link — and do NOT jq-narrow an execute response: pipe it whole (it is \
already compact), so `.view` survives. If you strip `.view` (or omit `\"shaped\":true`), the user \
loses the Open view button. \
Summarize the result for the user in a few lines; do NOT paste the whole raw JSON back. \
After your summary, ALWAYS output the trace on its own line, exactly \
`↳ requestId <requestId> · <timestamp>`. \
You do NOT print the view link yourself: whenever the execute output carries a `.view`, the host \
automatically shows a one-click `Open view` button that opens the authenticated progressive UI popup \
(the user's OS login is injected, no re-login). Never print the raw URL. The \
`a3s-os-capabilities` skill has full examples."
    )
}

/// Built-in slash commands shown in the `/` menu.
pub(super) const SLASH_COMMANDS: &[(&str, &str)] = &[
    (
        "/status",
        "show session, workspace, model, permission mode, and token usage",
    ),
    (
        "/model",
        "switch configured/account models (←/→ provider)",
    ),
    (
        "/permissions",
        "change the next-turn permission mode or inspect exact grants",
    ),
    (
        "/review",
        "review working tree, commit, or branch without changing files",
    ),
    ("/ide", "superfile-style file browser + editor"),
    (
        "/tasks",
        "inspect delegated work · search, view output, or cancel safely",
    ),
    (
        "/queue",
        "inspect pending follow-ups · send now, remove, or clear",
    ),
    (
        "/history",
        "fuzzy-search prompts from the current session",
    ),
    ("/help", "show grouped commands and shortcuts"),
    ("/init", "analyze the project and generate AGENTS.md"),
    ("/config", "edit config.acl in the built-in editor"),
    (
        "/terminal",
        "inspect terminal capabilities, fallbacks, and multiplexer passthrough",
    ),
    (
        "/checkup",
        "audit setup, then review proposed fixes before applying them",
    ),
    (
        "/copy",
        "copy the latest response · add `transcript` for the semantic session",
    ),
    (
        "/export",
        "write a new Markdown session file · optional workspace-relative path",
    ),
    (
        "/use",
        "inspect Browser/Office/OCR readiness · /use [status|repair]",
    ),
    ("/theme", "cycle the code-highlight theme (Codex Dark …)"),
    (
        "/island",
        "show or persist Agent Island on/off · /island [on|off|status]",
    ),
    (
        "/flow",
        "select a workflow asset → OS Workflow as a Service designer (needs /login) · /flow <text> drafts one",
    ),
    (
        "/agent",
        "select an agent definition → local dev · Agent as a Service or Function as a Service by kind",
    ),
    (
        "/mcp",
        "select an MCP server asset → local dev · publish/run/test via OS Function as a Service",
    ),
    (
        "/skill",
        "select a skill asset → local dev · publish/deploy via OS Function as a Service",
    ),
    (
        "/okf",
        "select an OKF package → local dev · publish/deploy via OS Knowledge service",
    ),
    ("/login", "sign in to the configured OS account"),
    ("/logout", "sign out from the configured OS account"),
    ("/plugin", "enable/disable Claude skills & plugins"),
    (
        "/packages",
        "review enable/disable plans for installed A3S Use cognitive packages",
    ),
    ("/reload", "re-scan skills/plugins (hot-reload the $ menu)"),
    ("/update", "upgrade a3s to the latest release"),
    (
        "/preview",
        "open a live artifact preview · path/localhost URL, status, or stop",
    ),
    (
        "/memory",
        "browse memory as an event/entity graph with tiers and forget candidates",
    ),
    (
        "/evolution",
        "review learned preferences, recurring skills, and versioned OKF candidates",
    ),
    (
        "/research",
        "inspect DeepResearch event state · status, explain, or strict replay",
    ),
    (
        "/kb",
        "open the local personal knowledge base · add/import/search/vault",
    ),
    (
        "/ctx",
        "search past sessions (ctx) · /ctx <n> attach · /ctx save <n> keep as memory",
    ),
    ("/effort", "adjust model effort (low … ultracode)"),
    ("/compact", "summarize + compact the conversation context"),
    ("/goal", "run a durable Ultracode goal until verified"),
    (
        "/loop",
        "engineered loop dashboard · agent-aware in /agent mode · /loop <task> quick loop",
    ),
    (
        "/sleep",
        "consolidate today's work into memory (experience · preferences · knowledge)",
    ),
    (
        "/relay",
        "search, inspect, and resume workspace sessions or background work",
    ),
    (
        "/fork",
        "branch this session · add `worktree` for an isolated workspace",
    ),
    (
        "/worktree",
        "inspect an isolated fork, create a patch handoff, or preview cleanup",
    ),
    (
        "/rewind",
        "undo the last completed turn when its files still match",
    ),
    ("/clear", "reset the conversation"),
    ("/auto", "make future turns non-interactive"),
    ("/exit", "quit a3s code"),
];

/// Stable information-architecture buckets shared by the slash menu and
/// `/help`. Command execution remains owned by the existing dispatch paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SlashCommandGroup {
    Workflow,
    Session,
    Context,
    Assets,
    System,
}

impl SlashCommandGroup {
    pub(super) const ALL: [Self; 5] = [
        Self::Workflow,
        Self::Session,
        Self::Context,
        Self::Assets,
        Self::System,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Workflow => "Workflow",
            Self::Session => "Session & control",
            Self::Context => "Context & memory",
            Self::Assets => "Assets & services",
            Self::System => "System & interface",
        }
    }

    pub(super) fn menu_label(self) -> &'static str {
        match self {
            Self::Workflow => "work",
            Self::Session => "session",
            Self::Context => "context",
            Self::Assets => "assets",
            Self::System => "system",
        }
    }
}

pub(super) fn slash_command_group(command: &str) -> SlashCommandGroup {
    match command {
        "/init" | "/checkup" | "/review" | "/ide" | "/preview" | "/goal" | "/loop" => {
            SlashCommandGroup::Workflow
        }
        "/status" | "/model" | "/effort" | "/permissions" | "/auto" | "/queue" | "/history"
        | "/tasks" | "/compact" | "/fork" | "/worktree" | "/rewind" | "/clear" => {
            SlashCommandGroup::Session
        }
        "/copy" | "/export" | "/relay" | "/ctx" | "/memory" | "/research" | "/kb" | "/sleep"
        | "/evolution" => SlashCommandGroup::Context,
        "/use" | "/flow" | "/agent" | "/mcp" | "/skill" | "/okf" => SlashCommandGroup::Assets,
        "/config" | "/terminal" | "/login" | "/logout" | "/plugin" | "/packages" | "/reload"
        | "/theme" | "/island" | "/update" | "/help" | "/exit" => SlashCommandGroup::System,
        _ => SlashCommandGroup::System,
    }
}

fn slash_command_keywords(command: &str) -> &'static str {
    match command {
        "/status" => "session info workspace branch model tokens usage policy authority",
        "/permissions" => "approval authorization safety mode grants revoke",
        "/review" => "git diff changes patch inspect",
        "/worktree" => "git isolated branch patch handoff cleanup",
        "/ide" => "files tree editor workspace",
        "/tasks" => "delegation subagent background cancel",
        "/queue" => "pending followup send later",
        "/history" => "previous prompts recall search",
        "/ctx" => "past sessions attach recall",
        "/relay" => "resume background remote session",
        "/copy" | "/export" => "share transcript markdown clipboard",
        "/model" | "/effort" => "reasoning provider intelligence",
        "/auto" => "noninteractive approval execution mode",
        "/terminal" => "shell capabilities multiplexer",
        "/checkup" => "doctor diagnose setup fixes",
        "/use" => "browser office ocr integrations readiness",
        "/flow" | "/agent" | "/mcp" | "/skill" | "/okf" => "asset service publish deploy develop",
        "/login" | "/logout" => "account authentication auth sign in out",
        "/help" => "commands shortcuts keys discover",
        _ => "",
    }
}

/// Slash commands that mutate the session / conversation and so must NOT run
/// mid-stream — hidden from the menu and rejected while a turn is in flight.
pub(super) const IDLE_ONLY: &[&str] = &[
    "/clear",
    "/compact",
    "/model",
    "/effort",
    "/goal",
    "/loop",
    "/reload",
    "/update",
    "/init",
    "/checkup",
    "/review",
    "/fork",
    "/worktree",
    "/rewind",
    "/sleep",
    "/relay",
    "/flow",
    "/agent",
    "/mcp",
    "/skill",
    "/okf",
    "/kb",
    "/packages",
];

/// Slash commands matching `input` (which begins with `/`), ranked by exact
/// name, name prefix, semantic metadata, then a bounded typo match.
pub(super) fn slash_candidates(input: &str) -> Vec<(&'static str, &'static str)> {
    let Some(needle) = input.strip_prefix('/') else {
        return Vec::new();
    };
    if needle.chars().any(char::is_whitespace) {
        return Vec::new();
    }
    let needle = needle.to_ascii_lowercase();
    let mut matches = SLASH_COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, (command, description))| {
            slash_command_match_score(&needle, command, description)
                .map(|score| (score, index, *command, *description))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(score, index, _, _)| (*score, *index));
    matches
        .into_iter()
        .map(|(_, _, command, description)| (command, description))
        .collect()
}

fn slash_command_match_score(needle: &str, command: &str, description: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let name = command.trim_start_matches('/').to_ascii_lowercase();
    if name == needle {
        return Some(0);
    }
    if name.starts_with(needle) {
        return Some(10 + name.len().saturating_sub(needle.len()));
    }
    if name.contains(needle) {
        return Some(30 + name.len().saturating_sub(needle.len()));
    }

    let metadata = format!(
        "{} {} {}",
        description.to_ascii_lowercase(),
        slash_command_group(command).label().to_ascii_lowercase(),
        slash_command_keywords(command)
    );
    if metadata.contains(needle) {
        return Some(60);
    }

    let distance = edit_distance(&name, needle);
    let typo_limit = if needle.len() <= 4 { 1 } else { 2 };
    (distance <= typo_limit).then_some(90 + distance)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_character) in left.chars().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(left_index + 1);
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + usize::from(left_character != *right_character);
            current.push(
                substitution
                    .min(previous[right_index + 1] + 1)
                    .min(current[right_index] + 1),
            );
        }
        previous = current;
    }
    previous[right.len()]
}

pub(super) fn unknown_slash_command_message(input: &str) -> String {
    let token = input
        .split_whitespace()
        .next()
        .unwrap_or("/")
        .chars()
        .filter(|character| !character.is_control())
        .take(40)
        .collect::<String>();
    if SLASH_COMMANDS.iter().any(|(command, _)| *command == token) {
        return format!("{token} does not accept those arguments · type /help for supported forms");
    }

    let suggestions = slash_candidates(&token)
        .into_iter()
        .map(|(command, _)| command)
        .take(3)
        .collect::<Vec<_>>();
    match suggestions.as_slice() {
        [] => format!("unknown slash command {token} · type / to browse or /help for all commands"),
        [only] => format!(
            "unknown slash command {token} · did you mean {only}? · type /help for all commands"
        ),
        many => format!(
            "unknown slash command {token} · closest: {} · type /help for all commands",
            many.join(", ")
        ),
    }
}

pub(super) fn slash_tail<'a>(input: &'a str, command: &str) -> Option<&'a str> {
    input
        .strip_prefix(command)
        .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

pub(super) fn os_asset_category_query(category: &str, query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        format!("category:{category}")
    } else {
        format!("category:{category} {query}")
    }
}

pub(super) fn runtime_asset_query(category: &str, asset_hint: &str, query: &str) -> String {
    let category = category.trim();
    let asset_hint = asset_hint.trim();
    let query = query.trim();
    let mut parts = Vec::new();
    if !category.is_empty() {
        parts.push(format!("category:{category}"));
    }
    if !asset_hint.is_empty() {
        parts.push(asset_hint.to_string());
    }
    if !query.is_empty() {
        parts.push(query.to_string());
    }
    parts.join(" ")
}

pub(super) fn cancel_pending_picker<Panel, Pending>(
    picker: &mut Option<Panel>,
    pending: &mut Option<Pending>,
) {
    *picker = None;
    *pending = None;
}

pub(super) fn os_required_message(cmd: &str, os_configured: bool) -> String {
    if os_configured {
        format!("  {cmd} needs OS — sign in with /login first")
    } else {
        format!(
            "  {cmd} needs OS — configure `os = \"https://your-os-host\"` in config.acl, then /login"
        )
    }
}

pub(super) fn os_required_alert(cmd: &str, os_configured: bool) -> String {
    let body = os_required_message(cmd, os_configured)
        .trim_start()
        .to_string();
    format!(
        "  {}",
        Alert::new(AlertKind::Warning, body).color(TN_YELLOW).view()
    )
}

pub(super) fn ide_flash_line(kind: ToastKind, message: impl Into<String>) -> String {
    let color = match kind {
        ToastKind::Info => TN_CYAN,
        ToastKind::Success => TN_GREEN,
        ToastKind::Warning => TN_YELLOW,
        ToastKind::Error => TN_RED,
    };
    Toast::new(kind, message).color(color).view()
}

/// A turn that delegated work (tools / subagents / planning) but stopped without
/// a final user-facing answer should auto-synthesize one. This applies in EVERY
/// mode, not just ultracode: parallel fan-out and planning run at all efforts, so
/// the "did work, produced no answer" gap can happen anywhere (e.g. a high-effort
/// plan that fans out to subagents which return artifacts-only). Fires at most
/// once per turn (`synthesis_used`).
pub(super) fn needs_synthesis(
    synthesis_inflight: bool,
    synthesis_used: bool,
    had_agent_activity: bool,
    text_after_activity: bool,
) -> bool {
    !synthesis_inflight && !synthesis_used && had_agent_activity && !text_after_activity
}

/// Rough in-flight token estimate for text that's still streaming, before the
/// provider's exact `usage` arrives on End. ASCII text averages ~4 chars/token,
/// but wide scripts are closer to ~1 token/char — so a flat `chars / 4`
/// under-counts them by 3-4x and makes the live counter lurch
/// upward when it snaps to the real number. Count the two classes separately.
pub(super) fn estimate_tokens(s: &str) -> usize {
    let (ascii, wide) = s.chars().fold((0usize, 0usize), |(a, w), c| {
        if c.is_ascii() {
            (a + 1, w)
        } else {
            (a, w + 1)
        }
    });
    ascii / 4 + wide
}

pub(super) fn ctx_limit_for_model(
    model_ctx: &std::collections::HashMap<String, u32>,
    model: &str,
) -> u32 {
    let codex_model = model
        .strip_prefix("codex/")
        .or_else(|| model.strip_prefix("openai-codex/"))
        .unwrap_or(model);
    let kimi_model = model.strip_prefix("kimi/").unwrap_or(model);
    context_limit_for_model(
        model,
        model_ctx.get(model).copied(),
        crate::account_providers::codex::codex_model_context(codex_model)
            .or_else(|| crate::account_providers::AccountProvider::Kimi.model_context(kimi_model)),
    )
}

#[derive(Clone)]
pub(super) enum LlmOverride {
    Static(Arc<dyn a3s_code_core::llm::LlmClient>),
    Codex(crate::account_providers::codex::CodexClient),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodexEffortStatus {
    pub(super) effective: String,
    pub(super) capped: bool,
}

impl LlmOverride {
    pub(super) fn client_for_effort(
        &self,
        a3s_effort: &str,
    ) -> Arc<dyn a3s_code_core::llm::LlmClient> {
        match self {
            Self::Static(client) => client.clone(),
            Self::Codex(client) => Arc::new(client.with_a3s_effort(a3s_effort)),
        }
    }

    pub(super) fn codex_effort_status(&self, a3s_effort: &str) -> Option<CodexEffortStatus> {
        let Self::Codex(client) = self else {
            return None;
        };
        let requested =
            crate::account_providers::codex::native_reasoning_effort_for_a3s(a3s_effort)?;
        let effective = client.resolve_reasoning_effort(a3s_effort)?;
        Some(CodexEffortStatus {
            capped: effective != requested,
            effective,
        })
    }
}

pub(super) fn os_gateway_llm_override(
    session: &crate::a3s_os::StoredOsSession,
    model: &str,
) -> LlmOverride {
    let origin = crate::a3s_os::os_origin(&session.address);
    // Route through the OS backend's authenticated LLM proxy (validates the OS
    // token, forwards to the internal gateway) rather than a bare `/v1`.
    LlmOverride::Static(Arc::new(
        a3s_code_core::llm::OpenAiClient::new(session.access_token.clone(), model.to_string())
            .with_base_url(origin)
            .with_chat_completions_path("/api/v1/llm/chat/completions")
            .with_provider_name("OS Gateway"),
    ))
}

/// Materialize one already-resolved model preference for a session.
///
/// The caller owns the fallback order (session sidecar, then global defaults).
/// Keeping that policy out of this function is important on resume: loading the
/// global model or effort here would silently mix settings from another session.
pub(super) fn restore_model_selection(
    preference: &ModelSelectionPreference,
    models: &[String],
    os_session: Option<&crate::a3s_os::StoredOsSession>,
    session_id: &str,
    effort: usize,
) -> Option<(String, Option<LlmOverride>)> {
    match preference.source {
        ModelSelectionSource::Config => models
            .iter()
            .any(|model| model == &preference.model)
            .then(|| (preference.model.clone(), None)),
        ModelSelectionSource::Claude
        | ModelSelectionSource::Kimi
        | ModelSelectionSource::CodeBuddy => {
            let provider = preference.source.account_provider()?;
            if !provider.is_available() {
                return None;
            }
            let model = provider.canonical_model(&preference.model);
            let client = provider.client(&model, session_id).ok()?;
            Some((model, Some(LlmOverride::Static(client))))
        }
        ModelSelectionSource::Codex => {
            let effort = EFFORT_LEVELS.get(effort)?;
            if !crate::account_providers::AccountProvider::Codex.is_available() {
                return None;
            }
            let client =
                crate::account_providers::codex::CodexClient::from_codex_login_with_effort(
                    &preference.model,
                    session_id,
                    effort.id,
                )
                .ok()?;
            Some((preference.model.clone(), Some(LlmOverride::Codex(client))))
        }
        ModelSelectionSource::OsGateway => {
            let session = os_session?;
            let client = os_gateway_llm_override(session, &preference.model);
            Some((preference.model.clone(), Some(client)))
        }
    }
}

pub(super) fn apply_launch_model_options(
    opts: SessionOptions,
    model: Option<&str>,
    llm_override: Option<&LlmOverride>,
    effort: &str,
    code_config: &CodeConfig,
    session_id: &str,
) -> SessionOptions {
    let opts = match model {
        Some(model) => opts.with_model(model),
        None => opts,
    };
    match llm_override {
        Some(client) => opts.with_llm_client(client.client_for_effort(effort)),
        None => match crate::session_llm::resolve_config_llm_client(code_config, &opts, session_id)
        {
            Ok(client) => opts.with_llm_client(client),
            // Preserve the core's normal configuration error at session
            // creation. A valid configured model takes the host-created path,
            // preserving v5.2.2's provider-specific structured-output signal.
            Err(_) => opts,
        },
    }
}

/// Context-fill warning latch: maps `pct` to its tier (0/70/85) and says
/// whether crossing INTO a higher tier than `warned` should warn now. The
/// returned tier becomes the new latch, so dropping back (compaction, /clear,
/// model switch) re-arms the warning. Pure for unit-testing.
pub(super) fn ctx_warn_tier(pct: usize, warned: u8) -> (u8, Option<u8>) {
    let tier: u8 = if pct >= 85 {
        85
    } else if pct >= 70 {
        70
    } else {
        0
    };
    (tier, (tier > warned).then_some(tier))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_model_preference_does_not_require_a_global_preference() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_home = std::env::var_os("HOME");
        let home = tempfile::tempdir().expect("temporary HOME");
        std::env::set_var("HOME", home.path());

        let preference = ModelSelectionPreference {
            source: ModelSelectionSource::Config,
            model: "openai/session-model".to_string(),
        };
        let restored = restore_model_selection(
            &preference,
            std::slice::from_ref(&preference.model),
            None,
            "session-id",
            DEFAULT_TUI_EFFORT_INDEX,
        );

        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }

        let (model, client) = restored.expect("session preference should restore");
        assert_eq!(model, preference.model);
        assert!(client.is_none());
    }

    #[test]
    fn invalid_codex_effort_is_rejected_without_panicking() {
        let preference = ModelSelectionPreference {
            source: ModelSelectionSource::Codex,
            model: "gpt-test".to_string(),
        };

        assert!(
            restore_model_selection(&preference, &[], None, "session-id", usize::MAX,).is_none()
        );
    }
}
