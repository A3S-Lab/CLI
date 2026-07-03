//! `/list` and `/ps` OS resource panels.
//!
//! `/list` shows the signed-in user's OS digital assets (agents, workflows,
//! applications, etc.) through the assets REST API. `/ps` shows deployed runtime
//! services / process-like resources through a small set of runtime endpoint
//! candidates, parsed leniently so the panel survives OS API shape drift.

use super::super::*;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OsAssetRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) category: String,
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) visibility: String,
    pub(crate) owner: String,
    pub(crate) updated: String,
    pub(crate) access_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OsServiceRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) image: String,
    pub(crate) access_url: Option<String>,
    pub(crate) updated: String,
    pub(crate) source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OsAssetFetch {
    pub(crate) rows: Vec<OsAssetRow>,
    pub(crate) note: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OsServiceFetch {
    pub(crate) rows: Vec<OsServiceRow>,
    pub(crate) note: String,
}

pub(crate) struct OsListPanel {
    pub(crate) rows: Vec<OsAssetRow>,
    pub(crate) sel: usize,
    pub(crate) scroll: usize,
    pub(crate) query: String,
    pub(crate) searching: bool,
    pub(crate) loading: bool,
    pub(crate) note: String,
    pub(crate) armed_delete: Option<String>,
}

pub(crate) struct OsPsPanel {
    pub(crate) rows: Vec<OsServiceRow>,
    pub(crate) sel: usize,
    pub(crate) scroll: usize,
    pub(crate) query: String,
    pub(crate) searching: bool,
    pub(crate) loading: bool,
    pub(crate) note: String,
    pub(crate) armed_stop: Option<String>,
}

struct ResourcePanelRender<'a> {
    header: &'a str,
    query: &'a str,
    searching: bool,
    note: &'a str,
    sel: usize,
    scroll: usize,
    total: usize,
    hint: &'a str,
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

fn with_query(origin: &str, path: &str, query: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(&format!("{}{}", origin.trim_end_matches('/'), path))
        .map_err(|e| e.to_string())?;
    let has_limit = url.query_pairs().any(|(key, _)| key == "limit");
    {
        let mut pairs = url.query_pairs_mut();
        if !has_limit {
            pairs.append_pair("limit", "100");
        }
        if !query.trim().is_empty() {
            pairs.append_pair("search", query.trim());
        }
    }
    Ok(url)
}

fn path_segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn envelope_data(v: &Value) -> &Value {
    v.get("data").unwrap_or(v)
}

fn items_of(v: &Value) -> Vec<Value> {
    let data = envelope_data(v);
    for ptr in [
        "/items",
        "/list",
        "/results",
        "/rows",
        "/assets",
        "/services",
        "/deployments",
        "/processes",
        "/invocations",
        "/batches",
    ] {
        if let Some(items) = data.pointer(ptr).and_then(|a| a.as_array()) {
            return items
                .iter()
                .filter(|item| item.is_object())
                .cloned()
                .collect();
        }
    }
    if let Some(items) = data.as_array() {
        return items
            .iter()
            .filter(|item| item.is_object())
            .cloned()
            .collect();
    }
    Vec::new()
}

fn str_at<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(s) = v
            .get(*key)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
        {
            return Some(s);
        }
    }
    None
}

fn nested_str_at<'a>(v: &'a Value, paths: &[&str]) -> Option<&'a str> {
    for path in paths {
        if let Some(s) = v
            .pointer(path)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
        {
            return Some(s);
        }
    }
    None
}

fn bool_flag(v: &Value, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(b) = v.get(*key).and_then(|x| x.as_bool()) {
            return Some(b);
        }
    }
    None
}

fn short_time(s: &str) -> String {
    s.replace('T', " ")
        .trim_end_matches('Z')
        .chars()
        .take(19)
        .collect()
}

fn asset_from_value(v: &Value) -> Option<OsAssetRow> {
    let id = str_at(v, &["id", "_id", "uuid", "assetId", "ref"])
        .or_else(|| str_at(v, &["name"]))
        .unwrap_or("asset")
        .to_string();
    let name = str_at(v, &["name", "title", "displayName", "label"])
        .unwrap_or(&id)
        .to_string();
    let category = str_at(v, &["category", "assetCategory", "type", "kind"])
        .unwrap_or("asset")
        .to_string();
    let kind = str_at(v, &["agentKind", "appKind", "assetKind", "runtimeKind"])
        .unwrap_or("")
        .to_string();
    let status = str_at(v, &["status", "state", "phase", "publishStatus"])
        .map(str::to_string)
        .or_else(|| {
            bool_flag(v, &["published", "isPublished"])
                .map(|b| if b { "published" } else { "draft" }.to_string())
        })
        .unwrap_or_else(|| "available".to_string());
    let visibility = str_at(v, &["visibility", "scope"])
        .unwrap_or("")
        .to_string();
    let owner = nested_str_at(v, &["/owner/name", "/owner/displayName", "/owner/id"])
        .or_else(|| str_at(v, &["ownerName", "createdBy", "userId"]))
        .unwrap_or("")
        .to_string();
    let updated = str_at(v, &["updatedAt", "modifiedAt", "createdAt", "lastSeenAt"])
        .map(short_time)
        .unwrap_or_default();
    let access_url = str_at(
        v,
        &["accessUrl", "gatewayUrl", "publicUrl", "url", "endpoint"],
    )
    .map(str::to_string);
    Some(OsAssetRow {
        id,
        name,
        category,
        kind,
        status,
        visibility,
        owner,
        updated,
        access_url,
    })
}

fn service_from_value(v: &Value, source: &str) -> Option<OsServiceRow> {
    let id = str_at(
        v,
        &[
            "id",
            "_id",
            "uuid",
            "serviceId",
            "deploymentId",
            "invocationId",
            "batchId",
            "name",
        ],
    )?
    .to_string();
    let name = str_at(
        v,
        &[
            "name",
            "serviceName",
            "appName",
            "project",
            "assetName",
            "functionName",
            "worker",
        ],
    )
    .unwrap_or(&id)
    .to_string();
    let kind = str_at(v, &["kind", "type", "category", "resourceType"])
        .unwrap_or_else(|| {
            if source.contains("function") {
                "function"
            } else if source.contains("deployment") {
                "deployment"
            } else {
                "service"
            }
        })
        .to_string();
    let status = str_at(v, &["status", "state", "phase", "runtimeStatus"])
        .unwrap_or("unknown")
        .to_string();
    let image = str_at(v, &["image", "containerImage", "ref", "asset", "worker"])
        .unwrap_or("")
        .to_string();
    let access_url = str_at(
        v,
        &["accessUrl", "gatewayUrl", "publicUrl", "url", "endpoint"],
    )
    .map(str::to_string);
    let updated = str_at(
        v,
        &[
            "updatedAt",
            "modifiedAt",
            "startedAt",
            "createdAt",
            "lastSeenAt",
        ],
    )
    .map(short_time)
    .unwrap_or_default();
    Some(OsServiceRow {
        id,
        name,
        kind,
        status,
        image,
        access_url,
        updated,
        source: source.to_string(),
    })
}

pub(crate) async fn fetch_os_assets(
    address: &str,
    token: &str,
    query: &str,
) -> Result<OsAssetFetch, String> {
    let origin = crate::a3s_os::os_origin(address);
    let url = with_query(&origin, "/api/v1/assets", query)?;
    let resp = http()?
        .get(url.clone())
        .bearer_auth(token)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("request to {url} failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "assets returned HTTP {}: {}",
            status.as_u16(),
            truncate(&text, 160)
        ));
    }
    let json: Value =
        serde_json::from_str(&text).map_err(|_| "assets returned non-JSON".to_string())?;
    let rows = items_of(&json)
        .iter()
        .filter_map(asset_from_value)
        .collect::<Vec<_>>();
    Ok(OsAssetFetch {
        note: format!("{} asset(s) · {}", rows.len(), url.path()),
        rows,
    })
}

const SERVICE_ENDPOINTS: &[&str] = &[
    "/api/v1/runtime/services",
    "/api/v1/runtime/deployments",
    "/api/v1/runtime/jobs",
    "/api/v1/deployments",
    "/api/v1/services",
    "/api/v1/functions/batches",
    "/api/v1/functions/invocations",
];

pub(crate) async fn fetch_os_services(
    address: &str,
    token: &str,
    query: &str,
) -> Result<OsServiceFetch, String> {
    let origin = crate::a3s_os::os_origin(address);
    let client = http()?;
    let mut rows = Vec::new();
    let mut ok_sources = Vec::new();
    let mut errors = Vec::new();
    for endpoint in SERVICE_ENDPOINTS {
        let url = with_query(&origin, endpoint, query)?;
        let resp = match client
            .get(url.clone())
            .bearer_auth(token)
            .header("accept", "application/json")
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                errors.push(format!("{endpoint}: {e}"));
                continue;
            }
        };
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            errors.push(format!("{endpoint}: HTTP {}", status.as_u16()));
            continue;
        }
        ok_sources.push(endpoint.trim_start_matches("/api/v1/").to_string());
        if let Ok(json) = serde_json::from_str::<Value>(&text) {
            rows.extend(
                items_of(&json)
                    .iter()
                    .filter_map(|item| service_from_value(item, endpoint)),
            );
        }
    }
    if ok_sources.is_empty() {
        return Err(if errors.is_empty() {
            "no OS runtime service endpoint responded".to_string()
        } else {
            errors.join(" · ")
        });
    }
    rows.sort_by_key(|row| row.name.to_lowercase());
    rows.dedup_by(|a, b| a.id == b.id && a.name == b.name);
    Ok(OsServiceFetch {
        note: format!(
            "{} service/process row(s) · {}",
            rows.len(),
            ok_sources.join(", ")
        ),
        rows,
    })
}

pub(crate) async fn delete_os_asset(
    address: &str,
    token: &str,
    id: &str,
) -> Result<String, String> {
    if id.contains('/') || id.contains('\\') || id.trim().is_empty() {
        return Err("invalid asset id".to_string());
    }
    let origin = crate::a3s_os::os_origin(address);
    let id_path = path_segment(id);
    let url = format!("{}/api/v1/assets/{id_path}", origin.trim_end_matches('/'));
    let resp = http()?
        .delete(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(format!("deleted asset {id}"))
    } else {
        Err(format!(
            "delete asset HTTP {}: {}",
            status.as_u16(),
            truncate(&body, 160)
        ))
    }
}

pub(crate) async fn stop_os_service(
    address: &str,
    token: &str,
    row: &OsServiceRow,
) -> Result<String, String> {
    if row.id.contains('/') || row.id.contains('\\') || row.id.trim().is_empty() {
        return Err("invalid service id".to_string());
    }
    let origin = crate::a3s_os::os_origin(address);
    let id_path = path_segment(&row.id);
    let candidates = [
        format!("/api/v1/runtime/services/{id_path}/stop"),
        format!("/api/v1/runtime/deployments/{id_path}/stop"),
        format!("/api/v1/runtime/jobs/{id_path}/cancel"),
        format!("/api/v1/services/{id_path}/stop"),
        format!("/api/v1/deployments/{id_path}/stop"),
        format!("/api/v1/functions/batches/{id_path}/cancel"),
        format!("/api/v1/functions/invocations/{id_path}/cancel"),
    ];
    let client = http()?;
    let mut errors = Vec::new();
    for path in candidates {
        let url = format!("{}{}", origin.trim_end_matches('/'), path);
        let resp = match client.post(&url).bearer_auth(token).send().await {
            Ok(resp) => resp,
            Err(e) => {
                errors.push(format!("{path}: {e}"));
                continue;
            }
        };
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(format!("stop/cancel requested for {}", row.name));
        }
        if !matches!(status.as_u16(), 404 | 405) {
            errors.push(format!(
                "{path}: HTTP {} {}",
                status.as_u16(),
                truncate(&body, 120)
            ));
        }
    }
    Err(if errors.is_empty() {
        "no stop/cancel operation is available for this row; press Enter to manage it in OS"
            .to_string()
    } else {
        errors.join(" · ")
    })
}

fn asset_matches(row: &OsAssetRow, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || [
            &row.id,
            &row.name,
            &row.category,
            &row.kind,
            &row.status,
            &row.visibility,
            &row.owner,
        ]
        .iter()
        .any(|s| s.to_lowercase().contains(&q))
}

fn service_matches(row: &OsServiceRow, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || [
            &row.id,
            &row.name,
            &row.kind,
            &row.status,
            &row.image,
            &row.source,
        ]
        .iter()
        .any(|s| s.to_lowercase().contains(&q))
}

fn selected_asset(panel: &OsListPanel) -> Option<OsAssetRow> {
    panel
        .rows
        .iter()
        .filter(|row| asset_matches(row, &panel.query))
        .nth(panel.sel)
        .cloned()
}

fn selected_service(panel: &OsPsPanel) -> Option<OsServiceRow> {
    panel
        .rows
        .iter()
        .filter(|row| service_matches(row, &panel.query))
        .nth(panel.sel)
        .cloned()
}

fn edit_search(query: &mut String, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Backspace => {
            query.pop();
            true
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            query.push(c);
            true
        }
        _ => false,
    }
}

impl App {
    pub(crate) fn open_os_list_panel(&mut self, query: String) -> Option<Cmd<Msg>> {
        let Some(session) = self.os_session.clone() else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render(&os_required_message("/list", self.os_config.is_some())),
            );
            return None;
        };
        let q = query.clone();
        self.os_list = Some(OsListPanel {
            rows: Vec::new(),
            sel: 0,
            scroll: 0,
            query,
            searching: false,
            loading: true,
            note: "loading OS assets…".to_string(),
            armed_delete: None,
        });
        Some(cmd::cmd(move || async move {
            Msg::OsAssets(fetch_os_assets(&session.address, &session.access_token, &q).await)
        }))
    }

    pub(crate) fn open_os_ps_panel(&mut self, query: String) -> Option<Cmd<Msg>> {
        let Some(session) = self.os_session.clone() else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render(&os_required_message("/ps", self.os_config.is_some())),
            );
            return None;
        };
        let q = query.clone();
        self.os_ps = Some(OsPsPanel {
            rows: Vec::new(),
            sel: 0,
            scroll: 0,
            query,
            searching: false,
            loading: true,
            note: "loading OS process services…".to_string(),
            armed_stop: None,
        });
        Some(cmd::cmd(move || async move {
            Msg::OsServices(fetch_os_services(&session.address, &session.access_token, &q).await)
        }))
    }

    fn reload_os_assets(&self) -> Option<Cmd<Msg>> {
        let session = self.os_session.clone()?;
        let query = self
            .os_list
            .as_ref()
            .map(|p| p.query.clone())
            .unwrap_or_default();
        Some(cmd::cmd(move || async move {
            Msg::OsAssets(fetch_os_assets(&session.address, &session.access_token, &query).await)
        }))
    }

    fn reload_os_services(&self) -> Option<Cmd<Msg>> {
        let session = self.os_session.clone()?;
        let query = self
            .os_ps
            .as_ref()
            .map(|p| p.query.clone())
            .unwrap_or_default();
        Some(cmd::cmd(move || async move {
            Msg::OsServices(
                fetch_os_services(&session.address, &session.access_token, &query).await,
            )
        }))
    }

    pub(crate) fn handle_os_list_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        if self.os_list.as_ref().is_some_and(|p| p.searching) {
            match key.code {
                KeyCode::Esc => {
                    if let Some(p) = self.os_list.as_mut() {
                        p.searching = false;
                    }
                }
                KeyCode::Enter => {
                    if let Some(p) = self.os_list.as_mut() {
                        p.searching = false;
                        p.loading = true;
                        p.note = format!("searching `{}`…", p.query);
                    }
                    return self.reload_os_assets();
                }
                _ => {
                    if let Some(p) = self.os_list.as_mut() {
                        if edit_search(&mut p.query, key) {
                            p.sel = 0;
                            p.scroll = 0;
                            p.armed_delete = None;
                        }
                    }
                }
            }
            return None;
        }
        match key.code {
            KeyCode::Esc => {
                self.os_list = None;
                None
            }
            KeyCode::Char('/') | KeyCode::Char('s') => {
                if let Some(p) = self.os_list.as_mut() {
                    p.searching = true;
                    p.armed_delete = None;
                }
                None
            }
            KeyCode::Char('r') => {
                if let Some(p) = self.os_list.as_mut() {
                    p.loading = true;
                    p.note = "refreshing…".to_string();
                }
                self.reload_os_assets()
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = self.os_list.as_mut() {
                    p.sel = p.sel.saturating_sub(1);
                    p.scroll = p.scroll.min(p.sel);
                    p.armed_delete = None;
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(p) = self.os_list.as_mut() {
                    let last = p
                        .rows
                        .iter()
                        .filter(|row| asset_matches(row, &p.query))
                        .count()
                        .saturating_sub(1);
                    p.sel = (p.sel + 1).min(last);
                    p.armed_delete = None;
                }
                None
            }
            KeyCode::PageUp => {
                if let Some(p) = self.os_list.as_mut() {
                    p.sel = p.sel.saturating_sub(10);
                    p.scroll = p.scroll.saturating_sub(10);
                }
                None
            }
            KeyCode::PageDown => {
                if let Some(p) = self.os_list.as_mut() {
                    let last = p
                        .rows
                        .iter()
                        .filter(|row| asset_matches(row, &p.query))
                        .count()
                        .saturating_sub(1);
                    p.sel = (p.sel + 10).min(last);
                }
                None
            }
            KeyCode::Enter | KeyCode::Char('o') => {
                let row = self.os_list.as_ref().and_then(selected_asset);
                self.open_asset_manager(row.as_ref());
                None
            }
            KeyCode::Char('O') => {
                self.open_asset_manager(None);
                None
            }
            KeyCode::Char('x') => {
                let row = self.os_list.as_ref().and_then(selected_asset)?;
                let armed =
                    self.os_list.as_ref().and_then(|p| p.armed_delete.as_ref()) == Some(&row.id);
                if !armed {
                    if let Some(p) = self.os_list.as_mut() {
                        p.armed_delete = Some(row.id.clone());
                        p.note = format!("press x again to delete `{}`", row.name);
                    }
                    return None;
                }
                if let Some(p) = self.os_list.as_mut() {
                    p.armed_delete = None;
                    p.loading = true;
                    p.note = format!("deleting `{}`…", row.name);
                }
                let session = self.os_session.clone()?;
                Some(cmd::cmd(move || async move {
                    Msg::OsAssetDeleted(
                        delete_os_asset(&session.address, &session.access_token, &row.id).await,
                    )
                }))
            }
            _ => None,
        }
    }

    pub(crate) fn handle_os_ps_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        if self.os_ps.as_ref().is_some_and(|p| p.searching) {
            match key.code {
                KeyCode::Esc => {
                    if let Some(p) = self.os_ps.as_mut() {
                        p.searching = false;
                    }
                }
                KeyCode::Enter => {
                    if let Some(p) = self.os_ps.as_mut() {
                        p.searching = false;
                        p.loading = true;
                        p.note = format!("searching `{}`…", p.query);
                    }
                    return self.reload_os_services();
                }
                _ => {
                    if let Some(p) = self.os_ps.as_mut() {
                        if edit_search(&mut p.query, key) {
                            p.sel = 0;
                            p.scroll = 0;
                            p.armed_stop = None;
                        }
                    }
                }
            }
            return None;
        }
        match key.code {
            KeyCode::Esc => {
                self.os_ps = None;
                None
            }
            KeyCode::Char('/') | KeyCode::Char('s') => {
                if let Some(p) = self.os_ps.as_mut() {
                    p.searching = true;
                    p.armed_stop = None;
                }
                None
            }
            KeyCode::Char('r') => {
                if let Some(p) = self.os_ps.as_mut() {
                    p.loading = true;
                    p.note = "refreshing…".to_string();
                }
                self.reload_os_services()
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = self.os_ps.as_mut() {
                    p.sel = p.sel.saturating_sub(1);
                    p.scroll = p.scroll.min(p.sel);
                    p.armed_stop = None;
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(p) = self.os_ps.as_mut() {
                    let last = p
                        .rows
                        .iter()
                        .filter(|row| service_matches(row, &p.query))
                        .count()
                        .saturating_sub(1);
                    p.sel = (p.sel + 1).min(last);
                    p.armed_stop = None;
                }
                None
            }
            KeyCode::PageUp => {
                if let Some(p) = self.os_ps.as_mut() {
                    p.sel = p.sel.saturating_sub(10);
                    p.scroll = p.scroll.saturating_sub(10);
                }
                None
            }
            KeyCode::PageDown => {
                if let Some(p) = self.os_ps.as_mut() {
                    let last = p
                        .rows
                        .iter()
                        .filter(|row| service_matches(row, &p.query))
                        .count()
                        .saturating_sub(1);
                    p.sel = (p.sel + 10).min(last);
                }
                None
            }
            KeyCode::Enter | KeyCode::Char('o') => {
                let row = self.os_ps.as_ref().and_then(selected_service);
                self.open_service_manager(row.as_ref());
                None
            }
            KeyCode::Char('O') => {
                self.open_service_manager(None);
                None
            }
            KeyCode::Char('x') => {
                let row = self.os_ps.as_ref().and_then(selected_service)?;
                let armed =
                    self.os_ps.as_ref().and_then(|p| p.armed_stop.as_ref()) == Some(&row.id);
                if !armed {
                    if let Some(p) = self.os_ps.as_mut() {
                        p.armed_stop = Some(row.id.clone());
                        p.note = format!("press x again to stop/cancel `{}`", row.name);
                    }
                    return None;
                }
                if let Some(p) = self.os_ps.as_mut() {
                    p.armed_stop = None;
                    p.loading = true;
                    p.note = format!("requesting stop/cancel for `{}`…", row.name);
                }
                let session = self.os_session.clone()?;
                Some(cmd::cmd(move || async move {
                    Msg::OsServiceStopped(
                        stop_os_service(&session.address, &session.access_token, &row).await,
                    )
                }))
            }
            _ => None,
        }
    }

    pub(crate) fn on_os_assets(&mut self, result: Result<OsAssetFetch, String>) {
        let Some(panel) = self.os_list.as_mut() else {
            return;
        };
        panel.loading = false;
        panel.armed_delete = None;
        match result {
            Ok(fetch) => {
                panel.rows = fetch.rows;
                panel.sel = panel.sel.min(panel.rows.len().saturating_sub(1));
                panel.note = fetch.note;
            }
            Err(e) => panel.note = format!("✗ {e}"),
        }
    }

    pub(crate) fn on_os_services(&mut self, result: Result<OsServiceFetch, String>) {
        let Some(panel) = self.os_ps.as_mut() else {
            return;
        };
        panel.loading = false;
        panel.armed_stop = None;
        match result {
            Ok(fetch) => {
                panel.rows = fetch.rows;
                panel.sel = panel.sel.min(panel.rows.len().saturating_sub(1));
                panel.note = fetch.note;
            }
            Err(e) => panel.note = format!("✗ {e}"),
        }
    }

    pub(crate) fn on_os_asset_deleted(
        &mut self,
        result: Result<String, String>,
    ) -> Option<Cmd<Msg>> {
        if let Some(panel) = self.os_list.as_mut() {
            panel.loading = true;
            panel.note = match result {
                Ok(msg) => format!("✔ {msg}"),
                Err(e) => format!("✗ {e}"),
            };
        }
        self.reload_os_assets()
    }

    pub(crate) fn on_os_service_stopped(
        &mut self,
        result: Result<String, String>,
    ) -> Option<Cmd<Msg>> {
        if let Some(panel) = self.os_ps.as_mut() {
            panel.loading = true;
            panel.note = match result {
                Ok(msg) => format!("✔ {msg}"),
                Err(e) => format!("✗ {e}"),
            };
        }
        self.reload_os_services()
    }

    fn open_asset_manager(&mut self, row: Option<&OsAssetRow>) {
        let Some(session) = self.os_session.as_ref() else {
            return;
        };
        let origin = crate::a3s_os::os_origin(&session.address);
        let (url, label) = match row {
            Some(row) => {
                let id_path = path_segment(&row.id);
                (
                    format!("{origin}/admin/assets/{id_path}?embed=1"),
                    format!("asset manager · {}", truncate(&row.name, 48)),
                )
            }
            None => (
                format!("{origin}/admin/kernel/assets?embed=1"),
                "asset manager".to_string(),
            ),
        };
        let spec = remote_ui::ViewSpec {
            url,
            width: Some(1440),
            height: Some(900),
            embeddable: true,
        };
        self.last_view = Some(spec.clone());
        self.push_line(&gutter(
            ACCENT,
            &remote_view_button(&format!("{label} · click or /view reopens")),
        ));
        self.open_remote_view(&spec);
    }

    fn open_service_manager(&mut self, row: Option<&OsServiceRow>) {
        let Some(session) = self.os_session.as_ref() else {
            return;
        };
        let origin = crate::a3s_os::os_origin(&session.address);
        let (url, label) = match row {
            Some(row) => {
                let id_path = path_segment(&row.id);
                (
                    format!("{origin}/admin/infrastructure/batch?service={id_path}&embed=1"),
                    format!("runtime manager · {}", truncate(&row.name, 48)),
                )
            }
            None => (
                format!("{origin}/admin/infrastructure/batch?embed=1"),
                "runtime manager".to_string(),
            ),
        };
        let spec = remote_ui::ViewSpec {
            url,
            width: Some(1440),
            height: Some(900),
            embeddable: true,
        };
        self.last_view = Some(spec.clone());
        self.push_line(&gutter(
            ACCENT,
            &remote_view_button(&format!("{label} · click or /view reopens")),
        ));
        self.open_remote_view(&spec);
    }

    pub(crate) fn render_os_list(&self, panel: &OsListPanel) -> String {
        let rows = panel
            .rows
            .iter()
            .filter(|row| asset_matches(row, &panel.query))
            .collect::<Vec<_>>();
        let header = format!(
            "/list — OS assets · {} shown / {} total{}",
            rows.len(),
            panel.rows.len(),
            if panel.loading { " · loading" } else { "" }
        );
        let hint = "↑↓/jk select · / search · Enter/o manage · O console · x delete · r refresh · Esc close";
        let spec = ResourcePanelRender {
            header: &header,
            query: &panel.query,
            searching: panel.searching,
            note: &panel.note,
            sel: panel.sel,
            scroll: panel.scroll,
            total: rows.len(),
            hint,
        };
        self.render_resource_panel(
            &spec,
            |idx, selected, width| asset_list_row(rows[idx], selected, width),
            |width| asset_detail(rows.get(panel.sel).copied(), width),
        )
    }

    pub(crate) fn render_os_ps(&self, panel: &OsPsPanel) -> String {
        let rows = panel
            .rows
            .iter()
            .filter(|row| service_matches(row, &panel.query))
            .collect::<Vec<_>>();
        let header = format!(
            "/ps — OS process services · {} shown / {} total{}",
            rows.len(),
            panel.rows.len(),
            if panel.loading { " · loading" } else { "" }
        );
        let hint = "↑↓/jk select · / search · Enter/o manage · O console · x stop/cancel · r refresh · Esc close";
        let spec = ResourcePanelRender {
            header: &header,
            query: &panel.query,
            searching: panel.searching,
            note: &panel.note,
            sel: panel.sel,
            scroll: panel.scroll,
            total: rows.len(),
            hint,
        };
        self.render_resource_panel(
            &spec,
            |idx, selected, width| service_list_row(rows[idx], selected, width),
            |width| service_detail(rows.get(panel.sel).copied(), width),
        )
    }

    fn render_resource_panel<Row, Detail>(
        &self,
        spec: &ResourcePanelRender<'_>,
        row: Row,
        detail: Detail,
    ) -> String
    where
        Row: Fn(usize, bool, usize) -> String,
        Detail: Fn(usize) -> Vec<String>,
    {
        let width = self.width as usize;
        let h = self.height as usize;
        let left_w = (width / 2).clamp(34, 70);
        let right_w = width.saturating_sub(left_w + 3);
        let sep = Style::new().fg(TN_GRAY).render(" │ ");
        let mut out = vec![
            pad_to(
                &Style::new()
                    .fg(ACCENT)
                    .bold()
                    .render(&format!("  {}", spec.header)),
                width,
            ),
            pad_to(
                &Style::new().fg(TN_GRAY).render(&format!(
                    "  {}{}",
                    if spec.searching {
                        "search> "
                    } else {
                        "search: "
                    },
                    if spec.query.is_empty() {
                        "—"
                    } else {
                        spec.query
                    }
                )),
                width,
            ),
            pad_to(
                &Style::new()
                    .fg(if spec.note.starts_with('✗') {
                        TN_RED
                    } else {
                        TN_GRAY
                    })
                    .render(&format!("  {}", spec.note)),
                width,
            ),
            pad_to(&Style::new().fg(TN_GRAY).render(&"─".repeat(width)), width),
        ];
        let body = h.saturating_sub(5);
        let sel = spec.sel.min(spec.total.saturating_sub(1));
        let start = if sel < spec.scroll {
            sel
        } else if sel >= spec.scroll + body {
            sel + 1 - body
        } else {
            spec.scroll
        }
        .min(spec.total.saturating_sub(1));
        let detail_lines = detail(right_w);
        for i in 0..body {
            let idx = start + i;
            let left = if idx < spec.total {
                row(idx, idx == sel, left_w)
            } else if spec.total == 0 && i == 0 {
                Style::new()
                    .fg(TN_GRAY)
                    .render(&pad_to("  no rows match", left_w))
            } else {
                " ".repeat(left_w)
            };
            let right = detail_lines.get(i).cloned().unwrap_or_default();
            out.push(format!("{left}{sep}{}", pad_to(&right, right_w)));
        }
        out.push(pad_to(
            &Style::new().fg(TN_GRAY).render(&format!("  {}", spec.hint)),
            width,
        ));
        out.truncate(h);
        while out.len() < h {
            out.push(String::new());
        }
        out.join("\n")
    }
}

fn asset_list_row(row: &OsAssetRow, selected: bool, width: usize) -> String {
    let raw = pad_to(
        &format!(
            "  {:<13} {:<11} {}",
            truncate(&row.category, 13),
            truncate(&row.status, 11),
            truncate(&row.name, width.saturating_sub(30))
        ),
        width,
    );
    if selected {
        Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
    } else {
        Style::new().fg(TN_FG).render(&raw)
    }
}

fn service_list_row(row: &OsServiceRow, selected: bool, width: usize) -> String {
    let raw = pad_to(
        &format!(
            "  {:<12} {:<11} {}",
            truncate(&row.kind, 12),
            truncate(&row.status, 11),
            truncate(&row.name, width.saturating_sub(29))
        ),
        width,
    );
    if selected {
        Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
    } else {
        Style::new().fg(TN_FG).render(&raw)
    }
}

fn detail_line(label: &str, value: &str, width: usize) -> String {
    if value.is_empty() {
        return String::new();
    }
    let label = format!("{label}: ");
    let value = truncate(value, width.saturating_sub(label.len()));
    format!(
        "{}{}",
        Style::new().fg(TN_GRAY).render(&label),
        Style::new().fg(TN_FG).render(&value)
    )
}

fn asset_detail(row: Option<&OsAssetRow>, width: usize) -> Vec<String> {
    let Some(row) = row else {
        return vec![Style::new().fg(TN_GRAY).render("select an asset")];
    };
    let mut lines = vec![Style::new()
        .fg(TN_FG)
        .bold()
        .render(&truncate(&row.name, width))];
    lines.push(detail_line("id", &row.id, width));
    lines.push(detail_line("category", &row.category, width));
    lines.push(detail_line("kind", &row.kind, width));
    lines.push(detail_line("status", &row.status, width));
    lines.push(detail_line("visibility", &row.visibility, width));
    lines.push(detail_line("owner", &row.owner, width));
    lines.push(detail_line("updated", &row.updated, width));
    if let Some(url) = &row.access_url {
        lines.push(detail_line("access", url, width));
    }
    lines.into_iter().filter(|line| !line.is_empty()).collect()
}

fn service_detail(row: Option<&OsServiceRow>, width: usize) -> Vec<String> {
    let Some(row) = row else {
        return vec![Style::new().fg(TN_GRAY).render("select a service/process")];
    };
    let mut lines = vec![Style::new()
        .fg(TN_FG)
        .bold()
        .render(&truncate(&row.name, width))];
    lines.push(detail_line("id", &row.id, width));
    lines.push(detail_line("kind", &row.kind, width));
    lines.push(detail_line("status", &row.status, width));
    lines.push(detail_line("image/ref", &row.image, width));
    lines.push(detail_line("source", &row.source, width));
    lines.push(detail_line("updated", &row.updated, width));
    if let Some(url) = &row.access_url {
        lines.push(detail_line("access", url, width));
    }
    lines.into_iter().filter(|line| !line.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn v(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    async fn spawn_mock_os(captured: Arc<Mutex<Vec<String>>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let requests = captured.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let line = req.lines().next().unwrap_or("").to_string();
                    requests.lock().unwrap().push(req);
                    let (status, payload) = mock_response(&line);
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                        payload.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.flush().await;
                });
            }
        });
        origin
    }

    fn mock_response(line: &str) -> (&'static str, &'static str) {
        if line.starts_with("GET /api/v1/assets?") {
            return (
                "200 OK",
                r#"{"data":{"items":[{"id":"asset 1?","name":"Payment Agent","category":"agent","status":"published"}]}}"#,
            );
        }
        if line.starts_with("GET /api/v1/runtime/services?") {
            return (
                "200 OK",
                r#"{"data":{"services":[{"serviceId":"svc 1?","serviceName":"api","state":"running","image":"img:v1"}]}}"#,
            );
        }
        if line.starts_with("DELETE /api/v1/assets/asset%201%3F ") {
            return ("200 OK", r#"{"ok":true}"#);
        }
        if line.starts_with("POST /api/v1/runtime/services/svc%201%3F/stop ") {
            return ("200 OK", r#"{"ok":true}"#);
        }
        ("404 Not Found", r#"{"error":"not found"}"#)
    }

    fn captured_lines(captured: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        captured
            .lock()
            .unwrap()
            .iter()
            .map(|req| req.lines().next().unwrap_or("").to_string())
            .collect()
    }

    #[test]
    fn items_of_accepts_common_envelopes() {
        assert_eq!(items_of(&v(r#"{"data":{"items":[{"id":"a"}]}}"#)).len(), 1);
        assert_eq!(
            items_of(&v(r#"{"items":[{"id":"a"},{"id":"b"}]}"#)).len(),
            2
        );
        assert_eq!(items_of(&v(r#"[{"id":"a"}]"#)).len(), 1);
    }

    #[test]
    fn parses_asset_rows_leniently() {
        let row = asset_from_value(&v(
            r#"{"id":"a1","name":"demo","category":"application","agentKind":"tool","published":true,"owner":{"name":"me"},"updatedAt":"2026-07-03T01:02:03Z"}"#,
        ))
        .unwrap();
        assert_eq!(row.id, "a1");
        assert_eq!(row.category, "application");
        assert_eq!(row.kind, "tool");
        assert_eq!(row.status, "published");
        assert_eq!(row.owner, "me");
        assert_eq!(row.updated, "2026-07-03 01:02:03");
    }

    #[test]
    fn parses_service_rows_leniently() {
        let row = service_from_value(
            &v(r#"{"deploymentId":"d1","serviceName":"api","state":"running","containerImage":"img:v1","accessUrl":"https://x"}"#),
            "/api/v1/runtime/deployments",
        )
        .unwrap();
        assert_eq!(row.id, "d1");
        assert_eq!(row.name, "api");
        assert_eq!(row.kind, "deployment");
        assert_eq!(row.status, "running");
        assert_eq!(row.access_url.as_deref(), Some("https://x"));
    }

    #[test]
    fn search_matches_across_visible_fields() {
        let row = OsAssetRow {
            id: "a1".into(),
            name: "Payment Agent".into(),
            category: "agent".into(),
            kind: "tool".into(),
            status: "published".into(),
            visibility: "private".into(),
            owner: "roy".into(),
            updated: String::new(),
            access_url: None,
        };
        assert!(asset_matches(&row, "payment"));
        assert!(asset_matches(&row, "tool"));
        assert!(!asset_matches(&row, "workflow"));
    }

    #[test]
    fn path_segment_encodes_reserved_chars() {
        assert_eq!(path_segment("asset 1?#"), "asset%201%3F%23");
        assert_eq!(path_segment("café"), "caf%C3%A9");
    }

    #[tokio::test]
    async fn fetch_os_assets_uses_bearer_token_and_search_query() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_mock_os(captured.clone()).await;
        let fetch = fetch_os_assets(&origin, "tok-asset", "Payment Agent")
            .await
            .unwrap();
        assert_eq!(fetch.rows.len(), 1);
        assert_eq!(fetch.rows[0].name, "Payment Agent");
        let requests = captured.lock().unwrap();
        let req = requests.first().unwrap();
        assert!(req
            .lines()
            .next()
            .unwrap()
            .contains("/api/v1/assets?limit=100&search=Payment+Agent"));
        assert!(req
            .to_ascii_lowercase()
            .contains("authorization: bearer tok-asset"));
    }

    #[tokio::test]
    async fn fetch_os_services_reads_runtime_services_endpoint() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_mock_os(captured.clone()).await;
        let fetch = fetch_os_services(&origin, "tok-service", "api")
            .await
            .unwrap();
        assert_eq!(fetch.rows.len(), 1);
        assert_eq!(fetch.rows[0].id, "svc 1?");
        assert_eq!(fetch.rows[0].status, "running");
        let lines = captured_lines(&captured);
        assert!(lines
            .iter()
            .any(|line| line.contains("/api/v1/runtime/services?limit=100&search=api")));
    }

    #[tokio::test]
    async fn management_calls_encode_ids_and_use_bearer_token() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_mock_os(captured.clone()).await;
        delete_os_asset(&origin, "tok-manage", "asset 1?")
            .await
            .unwrap();
        let row = OsServiceRow {
            id: "svc 1?".into(),
            name: "api".into(),
            kind: "service".into(),
            status: "running".into(),
            image: String::new(),
            access_url: None,
            updated: String::new(),
            source: "/api/v1/runtime/services".into(),
        };
        stop_os_service(&origin, "tok-manage", &row).await.unwrap();
        let requests = captured.lock().unwrap();
        let joined = requests.join("\n---\n");
        assert!(joined.contains("DELETE /api/v1/assets/asset%201%3F HTTP/1.1"));
        assert!(joined.contains("POST /api/v1/runtime/services/svc%201%3F/stop HTTP/1.1"));
        assert!(
            joined
                .to_ascii_lowercase()
                .matches("authorization: bearer tok-manage")
                .count()
                >= 2
        );
    }
}
