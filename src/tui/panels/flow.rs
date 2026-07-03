//! `/flow` — workflow DAGs as local JSON files, edited and debug-run in the
//! OS workflow designer.
//!
//! Bare `/flow` (login-gated) opens a picker over `flow_dir()` (`~/.a3s/flows`
//! or the `flow_dir` config key); Enter pushes the picked DAG into an OS
//! workflow asset (find-or-create by name, then commit it as
//! `.a3s/workflows/main.design.json` — the designer's canonical load path)
//! and opens `/workflow-designer/<asset-id>` in the authenticated
//! RemoteUI window, where it can be edited and debug-run.
//!
//! `/flow <natural language>` asks the agent to orchestrate a BASIC DAG in
//! the designer-document schema and save it into `flow_dir()` — no login
//! needed (it's a local file until opened).

use super::super::*;

/// Where the designer loads/saves a workflow inside the asset repo — its
/// canonical, first-probed document path.
pub(crate) const DESIGN_DOCUMENT_PATH: &str = ".a3s/workflows/main.design.json";

/// `/flow` selection panel: the DAG JSONs under the flows folder + cursor.
pub(crate) struct FlowPanel {
    /// Absolute path of the flows root (config `flow_dir`).
    pub(crate) root: std::path::PathBuf,
    /// JSON file names (with extension), sorted for a stable panel.
    pub(crate) flows: Vec<String>,
    pub(crate) sel: usize,
}

/// List the `*.json` files directly under `root`, skipping dotfiles. Sorted.
pub(crate) fn list_flows(root: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            (!n.starts_with('.') && n.to_ascii_lowercase().ends_with(".json")).then_some(n)
        })
        .collect();
    v.sort();
    v
}

/// The OS asset a flow file maps to — a deterministic name so re-opening the
/// same file updates the same asset instead of piling up duplicates.
pub(crate) fn flow_asset_name(file_stem: &str) -> String {
    format!("flow-{}", super::repos::slug(file_stem))
}

/// The STANDALONE designer page for an asset (login-guarded, no admin
/// chrome — mirrors the OS `/assistant` standalone surface). Route param
/// only; no query support.
pub(crate) fn designer_url(origin: &str, asset_id: &str) -> String {
    format!(
        "{}/workflow-designer/{}",
        origin.trim_end_matches('/'),
        asset_id
    )
}

pub(crate) fn designer_view_spec(url: String) -> remote_ui::ViewSpec {
    remote_ui::ViewSpec {
        url,
        width: Some(1440),
        height: Some(900),
        embeddable: true,
    }
}

fn flow_picker_header(total: usize, root: &std::path::Path, width: usize) -> String {
    truncate(
        &format!(
            "  ⧉ flow — pick a DAG ({total} in {})",
            root.to_string_lossy()
        ),
        width,
    )
}

fn flow_picker_hint(width: usize) -> String {
    truncate(
        "  ↑/↓ select · Enter open in the OS workflow designer · Esc cancel",
        width,
    )
}

fn flow_picker_row(name: &str, width: usize) -> String {
    pad_to(&truncate(&format!("  {name}"), width), width)
}

/// Directive for `/flow <description>`: orchestrate a BASIC DAG in the
/// designer-document schema and save it under the flows folder.
pub(crate) fn flow_gen_prompt(description: &str, dir: &str) -> String {
    format!(
        "Create a basic workflow DAG JSON from the description below and save it under \
         {dir}. This is a SMALL single-file task: do it directly in this turn — do NOT \
         plan, delegate, or fan out subagents.\n\
         Description: {description}\n\
         IMPORTANT: {dir} is OUTSIDE this session's workspace, so the path-scoped file \
         tools will reject it — use the `bash` tool (`mkdir -p {dir}`, then write the \
         file with a heredoc).\n\
         The file MUST follow the OS workflow-designer document schema exactly — this \
         minimal example shows every required field:\n\
         {{\"version\":\"a3s.workflow.design.v1\",\"name\":\"<name>\",\"description\":\
         \"<one line>\",\"triggerEvents\":[],\"variables\":[],\"outputs\":[],\
         \"nodes\":[{{\"id\":\"start\",\"kind\":\"start\",\"name\":\"Start\",\"data\":{{}},\
         \"x\":0,\"y\":0}},{{\"id\":\"step-1\",\"kind\":\"llm\",\"name\":\"<step>\",\
         \"data\":{{}},\"x\":320,\"y\":0}},{{\"id\":\"end\",\"kind\":\"end\",\
         \"name\":\"End\",\"data\":{{}},\"x\":640,\"y\":0}}],\
         \"edges\":[{{\"id\":\"e1\",\"sourceNodeID\":\"start\",\"targetNodeID\":\
         \"step-1\"}},{{\"id\":\"e2\",\"sourceNodeID\":\"step-1\",\"targetNodeID\":\
         \"end\"}}]}}\n\
         Rules: exactly one `start` and one `end` node; node kinds ONLY from: start, \
         end, llm, http, code, condition, loop, template, answer, knowledge-retrieval, \
         question-classifier, parameter-extractor, aggregator; unique kebab-case ids; \
         edges use sourceNodeID/targetNodeID (capital D); lay nodes left-to-right \
         (x += 320 per step, y ±180 for branches); keep it BASIC — 3-7 nodes that \
         match the description, no speculative extras.\n\
         Save as {dir}/<kebab-case-name>.json (if that file exists, append -2, -3, …). \
         Validate it with `python3 -m json.tool \"$FILE\" > /dev/null && echo OK` \
         (always pass the file path — never run a command that waits on stdin). Then \
         report the saved path and tell the user `/flow` opens it in the OS workflow \
         designer."
    )
}

// ---------------------------------------------------------------------------
// OS assets REST (lenient: parse ids out of serde_json::Value, not rigid DTOs)
// ---------------------------------------------------------------------------

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

/// Read `data.items[]` (paginated envelope) or any nested `items[]` array.
fn items_of(v: &serde_json::Value) -> Vec<serde_json::Value> {
    v.pointer("/data/items")
        .or_else(|| v.pointer("/data"))
        .or_else(|| v.pointer("/items"))
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default()
}

/// Find a workflow asset by exact name, else create one (private, user-owned;
/// creation auto-initializes its git repository). Returns the asset id.
pub(crate) async fn ensure_flow_asset(
    origin: &str,
    token: &str,
    name: &str,
) -> Result<String, String> {
    let client = http()?;
    let base = format!("{}/api/v1/assets", origin.trim_end_matches('/'));
    // Search is ILIKE-broad; require the exact name client-side.
    let found: serde_json::Value = client
        .get(format!("{base}?search={name}&category=workflow&limit=50"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(id) = items_of(&found)
        .iter()
        .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(name))
        .and_then(|a| a.get("id").and_then(|i| i.as_str()))
    {
        return Ok(id.to_string());
    }
    let resp = client
        .post(&base)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": name,
            "ownerType": "user",
            "category": "workflow",
            "visibility": "private",
            "description": "Created by a3s code /flow",
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "create asset failed ({status}): {}",
            body.get("message").and_then(|m| m.as_str()).unwrap_or("?")
        ));
    }
    body.pointer("/data/id")
        .or_else(|| body.get("id"))
        .and_then(|i| i.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "create asset: no id in response".to_string())
}

/// Commit the DAG into the asset repo at the designer's canonical path.
pub(crate) async fn upload_flow_document(
    origin: &str,
    token: &str,
    asset_id: &str,
    design_json: &str,
) -> Result<(), String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(design_json.as_bytes());
    let resp = http()?
        .post(format!(
            "{}/api/v1/assets/{asset_id}/repository/files",
            origin.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "overwrite": true,
            "message": "a3s code /flow: update workflow design",
            "files": [{ "path": DESIGN_DOCUMENT_PATH, "contentBase64": b64 }],
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    Err(format!(
        "upload failed ({status}): {}",
        truncate(&body, 200)
    ))
}

impl App {
    /// Open the `/flow` picker (login-gated by the caller).
    pub(crate) fn open_flow_panel(&mut self) {
        let root = flow_dir();
        let flows = list_flows(&root);
        if flows.is_empty() {
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  no flows in {} — draft one with `/flow <description>` first",
                root.display()
            )));
            return;
        }
        self.flow = Some(FlowPanel {
            root,
            flows,
            sel: 0,
        });
    }

    /// Keys while the `/flow` picker is open — consumes everything.
    pub(crate) fn handle_flow_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let p = self.flow.as_mut()?;
        let last = p.flows.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => p.sel = p.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => p.sel = (p.sel + 1).min(last),
            KeyCode::Esc => self.flow = None,
            KeyCode::Enter => {
                let panel = self.flow.take()?;
                let file = panel.flows.get(panel.sel.min(last))?.clone();
                let path = panel.root.join(&file);
                let design = match std::fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render(&format!("  could not read {}: {e}", path.display())),
                        );
                        return None;
                    }
                };
                if serde_json::from_str::<serde_json::Value>(&design).is_err() {
                    self.push_line(&Style::new().fg(TN_RED).render(&format!(
                        "  {} is not valid JSON — fix it (or redraft with /flow <description>)",
                        file
                    )));
                    return None;
                }
                let Some(session) = self.os_session.clone() else {
                    return None; // opener is login-gated; belt and suspenders
                };
                let stem = file.trim_end_matches(".json").to_string();
                let asset_name = flow_asset_name(&stem);
                let origin = crate::a3s_os::os_origin(&session.address);
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                    "  ⧉ {file} → OS workflow asset `{asset_name}` → designer…"
                )));
                return Some(cmd::cmd(move || async move {
                    let res = async {
                        let id =
                            ensure_flow_asset(&origin, &session.access_token, &asset_name).await?;
                        upload_flow_document(&origin, &session.access_token, &id, &design).await?;
                        Ok(designer_url(&origin, &id))
                    }
                    .await;
                    Msg::FlowOpened(res.map(|url| (stem, url)))
                }));
            }
            _ => {}
        }
        None
    }

    /// The upload finished: open the designer in the RemoteUI window (and keep
    /// it as `last_view` so `/view` / clicking the Open view button reopens it).
    pub(crate) fn on_flow_opened(&mut self, res: Result<(String, String), String>) {
        match res {
            Ok((name, url)) => {
                let spec = designer_view_spec(url);
                self.last_view = Some(spec.clone());
                self.push_line(&gutter(
                    ACCENT,
                    &remote_view_button(&format!("workflow designer · {name} · edit + debug run")),
                ));
                self.open_remote_view(&spec);
            }
            Err(e) => {
                self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  /flow failed: {e}")),
                );
            }
        }
    }

    /// Overlay the `/flow` picker above the input.
    pub(crate) fn overlay_flow_menu(&self, composed: String) -> String {
        let Some(p) = self.flow.as_ref() else {
            return composed;
        };
        let width = self.width as usize;
        let total = p.flows.len();
        let mut menu = vec![
            pad_to(
                &Style::new()
                    .fg(ACCENT)
                    .bold()
                    .render(&flow_picker_header(total, &p.root, width)),
                width,
            ),
            pad_to(
                &Style::new().fg(TN_GRAY).render(&flow_picker_hint(width)),
                width,
            ),
        ];
        let sel = p.sel.min(total.saturating_sub(1));
        let max_rows = (self.height as usize).saturating_sub(8).clamp(3, 12);
        let start = if sel < max_rows {
            0
        } else {
            sel + 1 - max_rows
        };
        let end = (start + max_rows).min(total);
        for (row, name) in p.flows.iter().enumerate().take(end).skip(start) {
            let raw = flow_picker_row(name, width);
            menu.push(if row == sel {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(TN_FG).render(&raw)
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
    fn lists_json_flows_sorted_skipping_dotfiles_and_nonjson() {
        let root = std::env::temp_dir().join(format!("a3s-flows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("zeta.json"), "{}").unwrap();
        std::fs::write(root.join("alpha.json"), "{}").unwrap();
        std::fs::write(root.join(".hidden.json"), "{}").unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();
        std::fs::create_dir_all(root.join("dir.json")).unwrap(); // dir, not a flow
        let fs = list_flows(&root);
        assert_eq!(fs, vec!["alpha.json", "zeta.json"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn designer_url_and_asset_name_follow_the_rules() {
        // The standalone (chrome-less) designer page, not the /admin one.
        assert_eq!(
            designer_url("http://180.163.156.38:49164/", "abc-123"),
            "http://180.163.156.38:49164/workflow-designer/abc-123"
        );
        assert_eq!(flow_asset_name("Daily Report 2"), "flow-daily-report-2");
    }

    #[test]
    fn designer_view_spec_auto_opens_workflow_designer_size() {
        let spec = designer_view_spec(designer_url("https://os.example.com", "asset-1"));
        assert_eq!(spec.url, "https://os.example.com/workflow-designer/asset-1");
        assert_eq!((spec.width, spec.height), (Some(1440), Some(900)));
        assert!(spec.embeddable);
    }

    #[test]
    fn flow_picker_rows_fit_fixed_width() {
        let root = std::path::PathBuf::from(
            "/Users/example/.a3s/flows/a/path/that/is/far/too/long/for/a/picker/header",
        );
        let header = flow_picker_header(9, &root, 40);
        let hint = flow_picker_hint(40);
        let row = flow_picker_row(
            "very-long-workflow-file-name-that-would-overflow-the-panel.json",
            40,
        );
        assert!(a3s_tui::style::visible_len(&header) <= 40, "{header}");
        assert!(a3s_tui::style::visible_len(&hint) <= 40, "{hint}");
        assert_eq!(a3s_tui::style::visible_len(&row), 40);
        assert!(row.contains('…'), "{row}");
    }

    #[test]
    fn flow_gen_prompt_carries_schema_rules_and_dir() {
        let p = flow_gen_prompt("fetch news daily and summarize", "/Users/x/.a3s/flows");
        assert!(p.contains("fetch news daily and summarize"));
        assert!(p.contains("/Users/x/.a3s/flows"));
        assert!(p.contains("a3s.workflow.design.v1")); // the schema anchor
        assert!(p.contains("sourceNodeID")); // capital-D edge fields
        assert!(p.contains("exactly one `start` and one `end`"));
        assert!(p.contains("OUTSIDE this session's workspace") && p.contains("bash"));
        // The example block itself must be valid JSON (models copy it).
        let start = p.find("{\"version\"").expect("example present");
        let end = p[start..].find("}]}").expect("example closes") + start + 3;
        assert!(serde_json::from_str::<serde_json::Value>(&p[start..end]).is_ok());
    }

    #[test]
    fn items_of_reads_paginated_and_bare_shapes() {
        let paged: serde_json::Value =
            serde_json::json!({"code":200,"data":{"items":[{"id":"a"}],"total":1}});
        assert_eq!(items_of(&paged).len(), 1);
        let bare: serde_json::Value = serde_json::json!({"data":[{"id":"b"}]});
        assert_eq!(items_of(&bare).len(), 1);
        assert!(items_of(&serde_json::json!({"data":{}})).is_empty());
    }
}
