//! `/model` picker (with account tabs) + `/effort` rebuild logic + overlays.

use super::super::*;
use super::login::{claude_models, has_local_login, AuthProvider};

/// A tab in the `/model` picker: config models, or a signed-in account's models.
struct ModelTab {
    label: &'static str,
    color: Color,
    models: Vec<String>,
    provider: Option<AuthProvider>, // None = config.acl
    os_gateway: bool,               // the 书安OS unified AI gateway tab
}

fn selected_model_location(tabs: &[ModelTab], current: Option<&str>) -> (usize, usize) {
    let current = current.map(crate::claude::canonical_model_name);
    current
        .as_deref()
        .and_then(|current| {
            tabs.iter().enumerate().find_map(|(tab_idx, tab)| {
                tab.models
                    .iter()
                    .position(|model| model == current)
                    .map(|model_idx| (tab_idx, model_idx))
            })
        })
        .unwrap_or((0, 0))
}

// Per-source accents, tuned to the Tokyo Night palette (blue / orange / teal).
const A3S_COLOR: Color = ACCENT;
const CLAUDE_COLOR: Color = TN_ORANGE;
const CODEX_COLOR: Color = Color::Rgb(115, 218, 202); // tokyo teal

/// Ultracode system-prompt steer: keep the model focused on decomposition and
/// synthesis while the core planning runtime turns independent plan waves into
/// visible `parallel_task` subagents.
const ULTRACODE_GUIDELINES: &str = "\
[ultracode] Dynamic-workflow mode is available — you decide whether a turn needs \
it. Match the effort to the task: answer trivial or conversational input (a \
greeting, a single question, a one-step edit) directly, with no plan and no \
fan-out. When a task genuinely splits into independent branches, decompose it, \
run those branches as parallel background subagents via `parallel_task` (keep \
each child prompt bounded and evidence-oriented), then synthesize their results \
before continuing dependent work.";

impl App {
    /// Tabs: a3s-code always; Claude Code / Codex appear when that local login
    /// is detected.
    fn model_tabs(&self) -> Vec<ModelTab> {
        let mut tabs = vec![ModelTab {
            label: "a3s-code",
            color: A3S_COLOR,
            models: self.models.clone(),
            provider: None,
            os_gateway: false,
        }];
        if has_local_login(AuthProvider::Claude) {
            tabs.push(ModelTab {
                label: "Claude Code",
                color: CLAUDE_COLOR,
                models: claude_models(), // from ~/.claude.json
                provider: Some(AuthProvider::Claude),
                os_gateway: false,
            });
        }
        if has_local_login(AuthProvider::Codex) {
            tabs.push(ModelTab {
                label: "Codex",
                color: CODEX_COLOR,
                models: crate::codex::codex_models(), // from ~/.codex/models_cache.json
                provider: Some(AuthProvider::Codex),
                os_gateway: false,
            });
        }
        // Signed in to 书安OS → offer its unified AI gateway (gateway-managed:
        // we send the OS token + a model id; the gateway holds provider keys).
        if self.os_session.is_some() {
            let models = match &self.os_gateway_models {
                Some(m) if !m.is_empty() => m.clone(),
                Some(_) => vec!["(gateway unavailable)".to_string()],
                None => vec!["(loading…)".to_string()],
            };
            tabs.push(ModelTab {
                label: "OS网关",
                color: TN_CYAN,
                models,
                provider: None,
                os_gateway: true,
            });
        }
        tabs
    }

    /// Open the /model picker on the current model + matching tab.
    pub(crate) fn open_model_menu(&mut self) {
        let tabs = self.model_tabs();
        if tabs.iter().all(|t| t.models.is_empty()) {
            self.push_line(
                &Style::new()
                    .fg(TN_RED)
                    .render("  no models configured in config.acl"),
            );
            return;
        }
        let (tab, idx) = selected_model_location(&tabs, self.model.as_deref());
        self.model_tab = tab;
        self.model_menu = Some(idx);
    }

    /// Keys while the /model panel is open: ↑/↓ select, ←/→/Tab switch tab,
    /// Enter activate (config model, or sign in with the tab's account), Esc.
    pub(crate) fn handle_model_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let sel = self.model_menu?;
        let tabs = self.model_tabs();
        let tab_count = tabs.len().max(1);
        let t = self.model_tab.min(tab_count - 1);
        let last = tabs[t].models.len().saturating_sub(1);
        match key.code {
            KeyCode::Up => {
                self.model_menu = Some(sel.saturating_sub(1));
                Some(None)
            }
            KeyCode::Down => {
                self.model_menu = Some((sel + 1).min(last));
                Some(None)
            }
            KeyCode::Left => {
                self.model_tab = t.saturating_sub(1);
                self.model_menu = Some(0);
                Some(None)
            }
            KeyCode::Right | KeyCode::Tab => {
                self.model_tab = (t + 1).min(tab_count - 1);
                self.model_menu = Some(0);
                Some(None)
            }
            KeyCode::Enter => {
                let model = tabs[t].models.get(sel.min(last)).cloned();
                let provider = tabs[t].provider;
                let os_gateway = tabs[t].os_gateway;
                self.model_menu = None;
                if os_gateway {
                    if let Some(model) = model {
                        self.use_os_gateway(&model);
                    }
                    return Some(None);
                }
                match provider {
                    None => {
                        self.llm_override = None; // config.acl credentials
                        if let Some(model) = model {
                            self.switch_model(&model);
                        }
                    }
                    Some(AuthProvider::Claude) => {
                        if let Some(model) = model {
                            self.sign_in_claude(&model);
                        }
                    }
                    Some(AuthProvider::Codex) => {
                        if let Some(model) = model {
                            self.sign_in_codex(&model);
                        }
                    }
                }
                Some(None)
            }
            KeyCode::Esc => {
                self.model_menu = None;
                Some(None)
            }
            _ => None,
        }
    }

    /// Sign in with the local Claude Code login and switch to one of its models
    /// by injecting the Claude account client (OAuth Bearer auth).
    fn sign_in_claude(&mut self, model: &str) {
        let model = crate::claude::canonical_model_name(model);
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  finish the current turn before switching models"),
            );
            return;
        }
        match crate::claude::ClaudeClient::from_claude_login(&model) {
            Ok(client) => {
                self.llm_override = Some(Arc::new(client));
                self.model = Some(model.clone());
                match self.rebuild_session(Some(&model)) {
                    Ok((session, _)) => {
                        self.session = Arc::new(session);
                        self.context_limit = resolve_ctx_limit(self.model_ctx.get(&model).copied());
                        self.push_line(
                            &Style::new()
                                .fg(TN_GREEN)
                                .render(&format!("  ⇄ Claude Code · {model}")),
                        );
                    }
                    Err(error) => {
                        self.llm_override = None;
                        self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render(&format!("  failed to switch: {error}")),
                        );
                    }
                }
            }
            Err(error) => self.push_line(
                &Style::new()
                    .fg(TN_RED)
                    .render(&format!("  Claude Code sign-in failed: {error}")),
            ),
        }
    }

    /// Sign in with the local Codex login and switch to one of its models by
    /// injecting the custom Codex client (talks to the ChatGPT backend).
    fn sign_in_codex(&mut self, model: &str) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  finish the current turn before switching models"),
            );
            return;
        }
        match crate::codex::CodexClient::from_codex_login(model, &self.session_id) {
            Ok(client) => {
                self.llm_override = Some(Arc::new(client));
                self.model = Some(model.to_string()); // before rebuild
                self.context_limit = resolve_ctx_limit(self.model_ctx.get(model).copied());
                match self.rebuild_session(Some(model)) {
                    Ok((s, _)) => {
                        self.session = Arc::new(s);
                        self.push_line(
                            &Style::new()
                                .fg(TN_GREEN)
                                .render(&format!("  ⇄ Codex · {model}")),
                        );
                    }
                    Err(e) => {
                        self.llm_override = None;
                        self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render(&format!("  failed to switch: {e}")),
                        );
                    }
                }
            }
            Err(e) => self.push_line(
                &Style::new()
                    .fg(TN_RED)
                    .render(&format!("  Codex sign-in failed: {e}")),
            ),
        }
    }

    /// Route the agent's LLM through the 书安OS **unified AI gateway**: an
    /// OpenAI-compatible client at `{OS origin}/v1/chat/completions`, authed with
    /// the OS Bearer token (the gateway is "gateway-managed" — it holds the real
    /// provider keys). `model` is a gateway model id from its `/v1/models`.
    fn use_os_gateway(&mut self, model: &str) {
        if model.starts_with('(') {
            // a placeholder row ("(loading…)" / "(gateway unavailable)").
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  OS网关暂无可用模型（确认 OS 已配置统一 AI 网关后重试 /model）"),
            );
            return;
        }
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  finish the current turn before switching models"),
            );
            return;
        }
        let Some(session) = self.os_session.clone() else {
            return;
        };
        let origin = crate::a3s_os::os_origin(&session.address);
        let client =
            a3s_code_core::llm::OpenAiClient::new(session.access_token.clone(), model.to_string())
                .with_base_url(origin)
                .with_provider_name("OS网关");
        self.llm_override = Some(Arc::new(client));
        self.model = Some(model.to_string());
        self.context_limit = resolve_ctx_limit(self.model_ctx.get(model).copied());
        match self.rebuild_session(Some(model)) {
            Ok((s, _)) => {
                self.session = Arc::new(s);
                self.push_line(
                    &Style::new()
                        .fg(TN_GREEN)
                        .render(&format!("  ⇄ OS网关 · {model}")),
                );
            }
            Err(e) => {
                self.llm_override = None;
                self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  failed to switch: {e}")),
                );
            }
        }
    }

    /// Switch the active model by resuming the session under it (history kept).
    /// Base session options carrying the current effort. `ultracode` adds a
    /// planning + goal tracking + a wider tool-round budget so a turn plans,
    /// then fans independent work out to visible parallel subagents.
    pub(crate) fn effort_session_opts(&self, thinking: bool) -> SessionOptions {
        let mut opts = with_recent_workspace_context(
            SessionOptions::new()
                .with_session_store(self.store.clone())
                .with_session_id(self.session_id.as_str())
                .with_confirmation_policy(self.confirmation.clone())
                .with_workspace_backend(self.workspace_services.clone())
                // Includes the login-gated OS `a3s-os-capabilities` skill.
                .with_skill_dirs(self.skill_dirs())
                .with_auto_save(true)
                // Auto-compact the context when it nears the window (Claude-style).
                // The threshold is scaled to THIS model's real window because the
                // core triggers off a fixed 200k (see `auto_compact_threshold_for`).
                .with_auto_compact(true)
                .with_auto_compact_threshold(auto_compact_threshold_for(self.context_limit))
                .with_file_memory(memory_dir())
                // Parallel fan-out available in every mode (not just ultracode).
                .with_max_parallel_tasks(8)
                .with_auto_delegation_enabled(true)
                .with_auto_parallel_delegation(true)
                // Pin manual delegation on so `parallel_task`/`task` stay registered
                // even if config.acl disables them — else ultracode's fan-out calls
                // an unregistered tool ("Unknown tool: parallel_task").
                .with_manual_delegation_enabled(true)
                // Generous tool-round budget for every effort — Claude Code runs
                // effectively unbounded; the old ~50 default cut real multi-step work
                // (and many parallel subagents) short. ultracode widens it further.
                .with_max_tool_rounds(200),
            &self.workspace_manifest,
        );
        // Keep project instructions (CLAUDE.md) + any /compact summary across
        // model/effort/compact rebuilds, injected into the system prompt. When
        // signed in, also steer the model to the progressive-API skill for OS
        // questions (else "OS" reads as the local operating system → `whoami`).
        let mut extra_parts: Vec<String> = Vec::new();
        if let Some(i) = &self.instructions {
            extra_parts.push(i.clone());
        }
        if let Some(s) = &self.compact_summary {
            extra_parts.push(format!("# Earlier conversation (compacted)\n\n{s}"));
        }
        if let Some(s) = &self.os_session {
            extra_parts.push(os_platform_guide(&s.address));
        }
        let extra = (!extra_parts.is_empty()).then(|| extra_parts.join("\n\n"));
        let ultra = self.effort == ULTRACODE;
        if extra.is_some() || ultra {
            let mut slots = SystemPromptSlots::default();
            if let Some(e) = extra {
                slots = slots.with_extra(e);
            }
            if ultra {
                slots = slots.with_guidelines(ULTRACODE_GUIDELINES);
            }
            opts = opts.with_prompt_slots(slots);
        }
        // Extended thinking is Anthropic-only; only request it when asked.
        if thinking {
            opts = opts.with_thinking_budget(EFFORT_LEVELS[self.effort].1);
        }
        if ultra {
            // Dynamic-workflow mode: planning is message-gated (Auto), so a turn
            // plans + fans out only when the core's pre-analysis judges the task to
            // warrant it — a trivial "hi" stays a direct answer. `Enabled` forced a
            // plan every turn, which is what made ultracode explore on a greeting.
            // The core runtime still upgrades independent plan waves into
            // `parallel_task` subagents when auto-parallel delegation is enabled.
            opts = opts
                .with_planning_mode(a3s_code_core::PlanningMode::Auto)
                .with_goal_tracking(true)
                .with_max_tool_rounds(500);
        }
        // Signed in via a /model account tab → route through that account client.
        if let Some(client) = &self.llm_override {
            opts = opts.with_llm_client(client.clone());
        }
        opts
    }

    /// Rebuild the session under the current effort. Tries with the thinking
    /// budget, then falls back without it (so models that don't support extended
    /// thinking don't error). Returns (session, thinking_dropped).
    pub(crate) fn rebuild_session(
        &self,
        model: Option<&str>,
    ) -> Result<(AgentSession, bool), String> {
        let build = |thinking: bool| {
            let o = self.effort_session_opts(thinking);
            match model {
                Some(m) => o.with_model(m),
                None => o,
            }
        };
        // Resume keeps history if the session was saved. Before the first turn
        // it isn't in the store ("Session not found"), so fall back to a fresh
        // session with the same id (no turns yet = no history to lose). Each is
        // also retried without the thinking budget for non-Anthropic models.
        for thinking in [true, false] {
            if let Ok(s) = self
                .agent
                .resume_session(self.session_id.as_str(), build(thinking))
            {
                return Ok((s, !thinking));
            }
            if let Ok(s) = self.agent.session(self.cwd.clone(), Some(build(thinking))) {
                return Ok((s, !thinking));
            }
        }
        Err("could not rebuild the session".into())
    }

    pub(crate) fn switch_model(&mut self, model: &str) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  finish the current turn before switching models"),
            );
            return;
        }
        match self.rebuild_session(Some(model)) {
            Ok((s, _)) => {
                self.session = Arc::new(s);
                self.model = Some(model.to_string());
                self.context_limit = resolve_ctx_limit(self.model_ctx.get(model).copied());
                self.push_line(
                    &Style::new()
                        .fg(TN_GREEN)
                        .render(&format!("  ⇄ switched to {model}")),
                );
            }
            Err(e) => self.push_line(
                &Style::new()
                    .fg(TN_RED)
                    .render(&format!("  failed to switch model: {e}")),
            ),
        }
    }

    /// Apply the selected effort by rebuilding the session (keeps model + history).
    pub(crate) fn apply_effort(&mut self) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  finish the current turn before changing effort"),
            );
            return;
        }
        let model = self.model.clone();
        match self.rebuild_session(model.as_deref()) {
            Ok((s, dropped)) => {
                self.session = Arc::new(s);
                if self.effort == ULTRACODE {
                    // Unattended fan-out: auto-approve so subagents run freely.
                    self.mode = Mode::Auto;
                    self.rainbow_until = Some(Instant::now()); // rainbow flourish
                    self.rainbow_frame = 0;
                    self.push_line(&Style::new().fg(ACCENT).bold().render(
                        "  ◆ ultracode — planning a dynamic workflow + parallel subagents (auto-approve on)",
                    ));
                } else if dropped {
                    self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                        "  ◇ effort: {} (this model uses its default depth)",
                        EFFORT_LEVELS[self.effort].0
                    )));
                } else {
                    self.push_line(
                        &Style::new()
                            .fg(TN_GREEN)
                            .render(&format!("  ◇ effort: {}", EFFORT_LEVELS[self.effort].0)),
                    );
                }
            }
            Err(e) => self.push_line(
                &Style::new()
                    .fg(TN_RED)
                    .render(&format!("  failed to set effort: {e}")),
            ),
        }
    }

    pub(crate) fn overlay_model_menu(&self, composed: String) -> String {
        let Some(sel) = self.model_menu else {
            return composed;
        };
        let tabs = self.model_tabs();
        if tabs.is_empty() {
            return composed;
        }
        let t = self.model_tab.min(tabs.len() - 1);
        let width = self.width as usize;
        let mut menu = vec![pad_to(
            &Style::new()
                .fg(tabs[t].color)
                .bold()
                .render("  Select model — ↑/↓ · ←/→ account · Enter · Esc"),
            width,
        )];
        // Tab strip (relay-style): each source in its brand colour, active boxed.
        if tabs.len() > 1 {
            let mut bar = String::from("  ");
            for (i, tab) in tabs.iter().enumerate() {
                let chip = format!(" {} ", tab.label);
                bar.push_str(&if i == t {
                    Style::new()
                        .fg(Color::Black)
                        .bg(tab.color)
                        .bold()
                        .render(&chip)
                } else {
                    Style::new().fg(tab.color).render(&chip)
                });
                bar.push(' ');
            }
            menu.push(pad_to(&bar, width));
        }
        let models = &tabs[t].models;
        let total = models.len();
        // Scroll a window around the selection so a pick past row 12 stays visible
        // and reachable (the list used to render a fixed first-12 only).
        let sel = sel.min(total.saturating_sub(1));
        let max_rows = (self.height as usize).saturating_sub(8).clamp(3, 12);
        let start = if sel < max_rows {
            0
        } else {
            sel + 1 - max_rows
        };
        let end = (start + max_rows).min(total);
        for (i, m) in models.iter().enumerate().take(end).skip(start) {
            let cur = Some(m.as_str()) == self.model.as_deref();
            let raw = pad_to(&format!("  {} {m}", if cur { "●" } else { " " }), width);
            menu.push(if i == sel {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(TN_GRAY).render(&raw)
            });
        }
        if total > max_rows {
            menu.push(pad_to(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("  {}/{total}", sel + 1)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_model_location_finds_account_tab_model() {
        let tabs = vec![
            ModelTab {
                label: "a3s-code",
                color: A3S_COLOR,
                models: vec!["openai/gpt-5".into()],
                provider: None,
                os_gateway: false,
            },
            ModelTab {
                label: "Claude Code",
                color: CLAUDE_COLOR,
                models: vec!["claude-sonnet-4".into()],
                provider: Some(AuthProvider::Claude),
                os_gateway: false,
            },
        ];

        assert_eq!(
            selected_model_location(&tabs, Some("claude-sonnet-4")),
            (1, 0)
        );
        assert_eq!(
            selected_model_location(&tabs, Some("claude-sonnet-4[1m]")),
            (1, 0)
        );
        assert_eq!(selected_model_location(&tabs, Some("missing")), (0, 0));
    }
}
