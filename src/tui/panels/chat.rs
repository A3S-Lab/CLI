//! `/im` chat page — a STANDALONE OS instant-messaging surface.
//!
//! This is deliberately NOT part of the agent loop: it opens its own full-screen
//! page, talks to the OS IM REST API directly, and never feeds chat content to
//! the coding agent. Transport is REST + polling (every ~2s while open); a later
//! revision can swap in the socket.io `/ws/im` push channel.

use super::super::*;

impl App {
    /// Open the chat page (requires an OS login). Kicks off the first fetch.
    /// `dm_with` (from `/im <userId>`) additionally opens/creates that DM.
    pub(crate) fn open_chat(&mut self, dm_with: Option<String>) -> Option<Cmd<Msg>> {
        let Some(sess) = self.os_session.clone() else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  sign in with /login before using /im"),
            );
            return None;
        };
        self.chat = Some(ChatPanel {
            convs: Vec::new(),
            sel: 0,
            active: None,
            messages: Vec::new(),
            me: None,
            error: None,
            loading: true,
            mode: ChatMode::Chat,
            contacts: Vec::new(),
            contact_sel: 0,
        });
        // Resolve who "I" am (for own-message alignment) + load conversations.
        let mut cmds = vec![
            {
                let s = sess.clone();
                cmd::cmd(move || async move {
                    Msg::ImMe(
                        os_im::whoami(&s.address, &s.access_token)
                            .await
                            .unwrap_or_default(),
                    )
                })
            },
            {
                let s = sess.clone();
                cmd::cmd(move || async move {
                    Msg::ImConvs(os_im::list_conversations(&s.address, &s.access_token).await)
                })
            },
        ];
        if let Some(user_id) = dm_with {
            cmds.push(cmd::cmd(move || async move {
                Msg::ImDmOpened(
                    os_im::open_dm(&sess.address, &sess.access_token, &user_id)
                        .await
                        .map(Box::new),
                )
            }));
        }
        Some(cmd::batch(cmds))
    }

    /// A DM opened via `/im <userId>`: surface + select it, then load its thread.
    pub(crate) fn on_im_dm_opened(
        &mut self,
        result: Result<Box<os_im::ImConversation>, String>,
    ) -> Option<Cmd<Msg>> {
        self.chat.as_ref()?;
        match result {
            Ok(conv) => {
                let id = conv.id.clone();
                {
                    let chat = self.chat.as_mut().unwrap();
                    if let Some(pos) = chat.convs.iter().position(|c| c.id == id) {
                        chat.sel = pos;
                    } else {
                        chat.convs.insert(0, *conv);
                        chat.sel = 0;
                    }
                    chat.active = None; // force a history load for the selected DM
                    chat.messages.clear();
                }
                Some(self.chat_load_history(id))
            }
            Err(e) => {
                if let Some(chat) = &mut self.chat {
                    chat.error = Some(e);
                }
                None
            }
        }
    }

    /// Conversation list arrived: store it, load the selected thread on first
    /// open, and re-arm the poll tick.
    pub(crate) fn on_im_convs(
        &mut self,
        result: Result<Vec<os_im::ImConversation>, String>,
    ) -> Option<Cmd<Msg>> {
        self.chat.as_ref()?;
        match result {
            Ok(convs) => {
                let want = {
                    let chat = self.chat.as_mut().unwrap();
                    chat.loading = false;
                    chat.error = None;
                    chat.convs = convs;
                    if chat.sel >= chat.convs.len() {
                        chat.sel = chat.convs.len().saturating_sub(1);
                    }
                    if chat.active.is_none() {
                        chat.convs.get(chat.sel).map(|c| c.id.clone())
                    } else {
                        None
                    }
                };
                let mut cmds = vec![cmd::tick(Duration::from_millis(2000), Msg::ImPoll)];
                if let Some(id) = want {
                    cmds.push(self.chat_load_history(id));
                }
                Some(cmd::batch(cmds))
            }
            Err(e) => {
                if let Some(chat) = &mut self.chat {
                    chat.loading = false;
                    chat.error = Some(e);
                }
                // Keep polling — the error may be transient (network / token refresh).
                Some(cmd::tick(Duration::from_millis(3000), Msg::ImPoll))
            }
        }
    }

    /// Message history arrived for `id`: replace the pane and advance the read
    /// cursor (fire-and-forget).
    pub(crate) fn on_im_history(
        &mut self,
        id: String,
        result: Result<Vec<os_im::ImMessage>, String>,
    ) -> Option<Cmd<Msg>> {
        self.chat.as_ref()?;
        match result {
            Ok(msgs) => {
                let last = msgs.last().map(|m| m.id.clone());
                {
                    let chat = self.chat.as_mut().unwrap();
                    chat.active = Some(id.clone());
                    chat.messages = msgs;
                    chat.error = None;
                }
                if let (Some(mid), Some(sess)) = (last, self.os_session.clone()) {
                    return Some(cmd::cmd(move || async move {
                        let _ =
                            os_im::mark_read(&sess.address, &sess.access_token, &id, &mid).await;
                        Msg::Noop
                    }));
                }
                None
            }
            Err(e) => {
                if let Some(chat) = &mut self.chat {
                    chat.error = Some(e);
                }
                None
            }
        }
    }

    /// Poll: refresh the conversation list (re-arms the tick via `ImConvs`) and
    /// the active thread.
    pub(crate) fn chat_refresh(&self) -> Cmd<Msg> {
        let mut cmds = Vec::new();
        if let Some(sess) = self.os_session.clone() {
            cmds.push(cmd::cmd(move || async move {
                Msg::ImConvs(os_im::list_conversations(&sess.address, &sess.access_token).await)
            }));
        }
        if let Some(id) = self.chat.as_ref().and_then(|c| c.active.clone()) {
            cmds.push(self.chat_load_history(id));
        }
        cmd::batch(cmds)
    }

    fn chat_load_history(&self, id: String) -> Cmd<Msg> {
        let sess = self.os_session.clone();
        cmd::cmd(move || async move {
            let Some(s) = sess else {
                return Msg::ImHistory(id, Err("not signed in".to_string()));
            };
            let result = os_im::history(&s.address, &s.access_token, &id).await;
            Msg::ImHistory(id, result)
        })
    }

    /// Select conversation `sel` and load its messages.
    fn chat_select(&mut self, sel: usize) -> Option<Cmd<Msg>> {
        let id = {
            let chat = self.chat.as_mut()?;
            if chat.convs.is_empty() {
                return None;
            }
            chat.sel = sel.min(chat.convs.len() - 1);
            chat.messages.clear();
            chat.convs.get(chat.sel).map(|c| c.id.clone())
        };
        id.map(|id| self.chat_load_history(id))
    }

    /// Send the composer's text to the active conversation.
    fn chat_send(&mut self, text: String) -> Option<Cmd<Msg>> {
        let body = text.trim().to_string();
        if body.is_empty() {
            return None;
        }
        let (Some(sess), Some(id)) = (
            self.os_session.clone(),
            self.chat.as_ref().and_then(|c| c.active.clone()),
        ) else {
            return None;
        };
        self.textarea.clear();
        Some(cmd::cmd(move || async move {
            Msg::ImSent(
                os_im::send(&sess.address, &sess.access_token, &id, &body)
                    .await
                    .map(Box::new),
            )
        }))
    }

    /// Fetch the contact directory filtered by `query` (empty = everyone in my
    /// orgs).
    fn chat_fetch_contacts(&self, query: String) -> Cmd<Msg> {
        let sess = self.os_session.clone();
        cmd::cmd(move || async move {
            let Some(s) = sess else {
                return Msg::ImContacts(Err("not signed in".to_string()));
            };
            Msg::ImContacts(os_im::contacts(&s.address, &s.access_token, &query).await)
        })
    }

    /// Start (or open) a DM with the selected contact, then return to chat mode.
    fn chat_open_contact_dm(&mut self) -> Option<Cmd<Msg>> {
        let user_id = {
            let chat = self.chat.as_mut()?;
            let uid = chat.contacts.get(chat.contact_sel)?.id.clone();
            chat.mode = ChatMode::Chat;
            uid
        };
        self.textarea.clear();
        let sess = self.os_session.clone()?;
        Some(cmd::cmd(move || async move {
            Msg::ImDmOpened(
                os_im::open_dm(&sess.address, &sess.access_token, &user_id)
                    .await
                    .map(Box::new),
            )
        }))
    }

    /// Chat page owns all keys while open. In Chat mode: ↑/↓ pick a conversation,
    /// Enter sends, Tab → contact search, Esc closes. In Search mode: type to
    /// filter contacts, ↑/↓ pick one, Enter opens the DM, Tab/Esc → back.
    pub(crate) fn handle_chat_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let mode = self.chat.as_ref().map(|c| c.mode)?;
        match (mode, key.code) {
            // Tab toggles Chat ⇄ Search (clears the shared input either way).
            (_, KeyCode::Tab) => {
                self.textarea.clear();
                let now_search = {
                    let chat = self.chat.as_mut()?;
                    chat.mode = if chat.mode == ChatMode::Chat {
                        ChatMode::Search
                    } else {
                        ChatMode::Chat
                    };
                    chat.mode == ChatMode::Search
                };
                now_search.then(|| self.chat_fetch_contacts(String::new()))
            }
            (ChatMode::Search, KeyCode::Esc) => {
                self.textarea.clear();
                if let Some(chat) = &mut self.chat {
                    chat.mode = ChatMode::Chat;
                }
                None
            }
            (ChatMode::Chat, KeyCode::Esc) => {
                self.chat = None;
                self.textarea.clear();
                None
            }
            (ChatMode::Chat, KeyCode::Up) => {
                let sel = self.chat.as_ref().map(|c| c.sel).unwrap_or(0);
                self.chat_select(sel.saturating_sub(1))
            }
            (ChatMode::Chat, KeyCode::Down) => {
                let (sel, len) = self
                    .chat
                    .as_ref()
                    .map(|c| (c.sel, c.convs.len()))
                    .unwrap_or((0, 0));
                if len == 0 {
                    return None;
                }
                self.chat_select((sel + 1).min(len - 1))
            }
            (ChatMode::Search, KeyCode::Up) => {
                if let Some(chat) = &mut self.chat {
                    chat.contact_sel = chat.contact_sel.saturating_sub(1);
                }
                None
            }
            (ChatMode::Search, KeyCode::Down) => {
                if let Some(chat) = &mut self.chat {
                    let last = chat.contacts.len().saturating_sub(1);
                    chat.contact_sel = (chat.contact_sel + 1).min(last);
                }
                None
            }
            (ChatMode::Chat, _) => {
                if let Some(TextareaMsg::Submit(text)) = self.textarea.handle_key(key) {
                    return self.chat_send(text);
                }
                None
            }
            (ChatMode::Search, _) => {
                // Enter opens the selected contact's DM; any other key edits the
                // query and re-runs the search.
                if let Some(TextareaMsg::Submit(_)) = self.textarea.handle_key(key) {
                    return self.chat_open_contact_dm();
                }
                Some(self.chat_fetch_contacts(self.textarea.value()))
            }
        }
    }

    pub(crate) fn render_chat(&self, chat: &ChatPanel) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let tw = (width / 3).clamp(20, 40);
        let clip = |s: &str, n: usize| -> String { s.chars().take(n).collect() };
        let sep = Style::new().fg(TN_GRAY).render(" │ ");
        let search = chat.mode == ChatMode::Search;

        // Header: title + a mode-aware hint (errors surface here in red).
        let title = if search {
            "OS chat · find people"
        } else {
            "OS chat"
        };
        let hint = if let Some(err) = &chat.error {
            Style::new().fg(TN_RED).render(&format!("⚠ {err}"))
        } else if search {
            Style::new()
                .fg(TN_GRAY)
                .render("type to filter · ↑↓ pick · Enter to DM · Tab/Esc back")
        } else {
            Style::new()
                .fg(TN_GRAY)
                .render("Tab find people · ↑↓ pick · Enter send · Esc close")
        };
        let mut out = vec![
            pad_to(
                &format!(
                    "  {}    {hint}",
                    Style::new().fg(ACCENT).bold().render(title)
                ),
                width,
            ),
            pad_to(&Style::new().fg(TN_GRAY).render(&"─".repeat(width)), width),
        ];

        // Body: left pane (conversations or contact results) | right pane (the
        // active conversation's messages, bottom-aligned).
        let rows = h.saturating_sub(4).max(1);
        let msg_start = chat.messages.len().saturating_sub(rows);
        for i in 0..rows {
            let left = if search {
                match chat.contacts.get(i) {
                    Some(c) => {
                        let label = if c.name.is_empty() {
                            &c.username
                        } else {
                            &c.name
                        };
                        let detail = if c.email.is_empty() {
                            format!("@{}", c.username)
                        } else {
                            c.email.clone()
                        };
                        let plain = pad_to(
                            &clip(&format!("  {}  {}", clip(label, 14), clip(&detail, 20)), tw),
                            tw,
                        );
                        if i == chat.contact_sel {
                            Style::new().fg(Color::Black).bg(ACCENT).render(&plain)
                        } else {
                            Style::new().fg(TN_FG).render(&plain)
                        }
                    }
                    None if i == 0 && chat.contacts.is_empty() => Style::new()
                        .fg(TN_GRAY)
                        .render(&pad_to("  no matching contacts", tw)),
                    None => " ".repeat(tw),
                }
            } else {
                match chat.convs.get(i) {
                    Some(c) => {
                        let base = c
                            .title
                            .clone()
                            .filter(|t| !t.is_empty())
                            .unwrap_or_else(|| {
                                if c.kind == "group" {
                                    "group".into()
                                } else {
                                    "DM".into()
                                }
                            });
                        let name = if c.kind == "group" {
                            format!("{base} ({})", c.member_ids.len())
                        } else {
                            base
                        };
                        let unread = if c.unread_count > 0 {
                            format!(" ({})", c.unread_count)
                        } else {
                            String::new()
                        };
                        let preview = c
                            .last_message
                            .as_ref()
                            .map(|m| format!("  {}", clip(&m.content, 14)))
                            .unwrap_or_default();
                        let mark = if i == chat.sel { "▸" } else { " " };
                        let plain = pad_to(
                            &clip(
                                &format!("  {mark} {}{unread}{preview}", clip(&name, 16)),
                                tw,
                            ),
                            tw,
                        );
                        if i == chat.sel {
                            Style::new().fg(ACCENT).bold().render(&plain)
                        } else {
                            Style::new().fg(TN_FG).render(&plain)
                        }
                    }
                    None if i == 0 && chat.convs.is_empty() => {
                        let msg = if chat.loading {
                            "  loading…"
                        } else {
                            "  no conversations · Tab to find people"
                        };
                        Style::new().fg(TN_GRAY).render(&pad_to(msg, tw))
                    }
                    None => " ".repeat(tw),
                }
            };
            let right = match chat.messages.get(msg_start + i) {
                Some(m) => {
                    let mine = chat.me.as_deref() == Some(m.sender_id.as_str());
                    // Prefer the server-resolved display name; fall back to a short id.
                    let who = if mine {
                        "me".to_string()
                    } else {
                        clip(m.sender_name.as_deref().unwrap_or(&m.sender_id), 12)
                    };
                    let time = m.created_at.get(11..16).unwrap_or("");
                    let tag = match m.kind.as_str() {
                        "code" => "⧉ ",
                        "view" => "🔗 ",
                        _ => "",
                    };
                    let body = clip(&m.content, width.saturating_sub(tw + 16));
                    let line = format!("{time} {who}: {tag}{body}");
                    if mine {
                        Style::new().fg(TN_CYAN).render(&line)
                    } else {
                        Style::new().fg(TN_FG).render(&line)
                    }
                }
                None => String::new(),
            };
            out.push(format!("{left}{sep}{right}"));
        }

        // Bottom: the shared input — composer in Chat mode, search box in Search.
        out.push(pad_to(
            &Style::new().fg(TN_GRAY).render(&"─".repeat(width)),
            width,
        ));
        let inp = clip(&self.textarea.value(), width.saturating_sub(8));
        let bottom = if search {
            format!("  🔍 {}▌", Style::new().fg(TN_FG).render(&inp))
        } else {
            format!("  › {}▌", Style::new().fg(TN_FG).render(&inp))
        };
        out.push(pad_to(&bottom, width));

        out.truncate(h);
        while out.len() < h {
            out.push(String::new());
        }
        out.join("\n")
    }
}
