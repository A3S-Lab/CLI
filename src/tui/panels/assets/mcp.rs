//! `/mcp` — local multi-turn development for MCP server assets.
//!
//! Bare `/mcp` opens a picker over `mcp_dir()` (`~/.a3s/mcps` or the `mcp_dir`
//! config key). Enter puts the TUI into a local MCP-development context. Later
//! OS subcommands can publish/debug/test the asset through Function as a
//! Service, but local selection itself never opens OS or RemoteUI.

use super::super::os_progressive;
use super::super::*;
use a3s_tui::components::{MenuItem, MenuPanel, MenuPanelMsg};
use a3s_tui::event::MouseEvent;

const MCP_MANIFEST_PATH: &str = ".a3s/mcp.asset.json";
const MCP_SERVER_CONFIG_PATH: &str = ".a3s/mcp.server.json";
const MCP_RUNTIME_BINDING_PATH: &str = ".a3s/mcp.runtime-binding.json";
const MAX_MCP_SOURCE_FILES: usize = 200;
const MAX_MCP_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
const MCP_OVERLAY_ROWS_BELOW: usize = 5;

#[derive(Clone)]
pub(crate) struct McpProject {
    pub(crate) rel: String,
    pub(crate) path: std::path::PathBuf,
    pub(crate) name: String,
    pub(crate) description: String,
}

/// `/mcp` selection panel: local MCP assets + cursor.
pub(crate) struct McpPanel {
    /// Absolute path of the MCP root (config `mcp_dir`).
    pub(crate) root: std::path::PathBuf,
    /// MCP assets under the root, sorted by relative path.
    pub(crate) projects: Vec<McpProject>,
    pub(crate) sel: usize,
}

/// The local MCP asset currently being developed in the TUI.
#[derive(Clone)]
pub(crate) struct McpDevSession {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) rel: String,
    pub(crate) path: std::path::PathBuf,
    pub(crate) root: std::path::PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum McpOsAction {
    Publish,
    Deploy,
    Debug,
    Test,
    Open,
    Logs,
    Status,
}

impl McpOsAction {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Publish => "publish",
            Self::Deploy => "deploy",
            Self::Debug => "debug",
            Self::Test => "test",
            Self::Open => "open",
            Self::Logs => "logs",
            Self::Status => "status",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum McpSubcommand {
    Exit,
    Clone(String),
    List(String),
    Review,
    Activity(String),
    Publish,
    Deploy,
    Debug,
    Test,
    Open,
    Logs,
    Status,
}

impl McpSubcommand {
    pub(crate) fn os_action(&self) -> Option<McpOsAction> {
        match self {
            Self::Exit | Self::Clone(_) | Self::List(_) | Self::Review | Self::Activity(_) => None,
            Self::Publish => Some(McpOsAction::Publish),
            Self::Deploy => Some(McpOsAction::Deploy),
            Self::Debug => Some(McpOsAction::Debug),
            Self::Test => Some(McpOsAction::Test),
            Self::Open => Some(McpOsAction::Open),
            Self::Logs => Some(McpOsAction::Logs),
            Self::Status => Some(McpOsAction::Status),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct McpOsResult {
    pub(crate) action: McpOsAction,
    pub(crate) asset_name: String,
    pub(crate) asset_id: String,
    pub(crate) view: remote_ui::ViewSpec,
    pub(crate) note: String,
    pub(crate) open_view: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct McpAssetRef {
    id: String,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct McpSourceFile {
    path: String,
    bytes: Vec<u8>,
}

const MCP_MARKERS: &[&str] = &[
    ".a3s/mcp.server.json",
    "mcp.server.json",
    "mcp.json",
    "package.json",
    "pyproject.toml",
];

/// List MCP assets recursively, skipping dot-directories except the asset-local
/// `.a3s` metadata folder. An asset is any directory with an MCP marker.
pub(crate) fn list_mcp_projects(root: &std::path::Path) -> Vec<McpProject> {
    let mut out = Vec::new();
    list_mcp_projects_inner(root, root, &mut out);
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.dedup_by(|a, b| a.rel == b.rel);
    out
}

fn list_mcp_projects_inner(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<McpProject>,
) {
    if let Some(project) = mcp_project_from_dir(root, dir) {
        out.push(project);
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        list_mcp_projects_inner(root, &path, out);
    }
}

fn mcp_project_from_dir(root: &std::path::Path, dir: &std::path::Path) -> Option<McpProject> {
    if !MCP_MARKERS.iter().any(|marker| dir.join(marker).is_file()) {
        return None;
    }
    let rel = dir
        .strip_prefix(root)
        .ok()
        .and_then(|p| {
            let s = p.to_string_lossy().replace('\\', "/");
            (!s.is_empty()).then_some(s)
        })
        .unwrap_or_else(|| ".".to_string());
    let (name, description) = mcp_project_metadata(dir).unwrap_or_else(|| {
        (
            mcp_name_from_rel(&rel),
            "Local MCP server asset".to_string(),
        )
    });
    Some(McpProject {
        rel,
        path: dir.to_path_buf(),
        name,
        description,
    })
}

fn mcp_project_metadata(dir: &std::path::Path) -> Option<(String, String)> {
    for marker in [".a3s/mcp.server.json", "mcp.server.json", "mcp.json"] {
        let path = dir.join(marker);
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(name) = value
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let description = value
                .get("description")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("Local MCP server asset")
                .to_string();
            return Some((name.to_string(), description));
        }
    }
    if let Ok(text) = std::fs::read_to_string(dir.join("package.json")) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(name) = value
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let description = value
                    .get("description")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Local MCP server asset")
                    .to_string();
                return Some((name.to_string(), description));
            }
        }
    }
    None
}

fn mcp_name_from_rel(rel: &str) -> String {
    rel.rsplit('/')
        .find(|part| !part.trim().is_empty() && *part != ".")
        .unwrap_or("mcp-server")
        .to_string()
}

pub(crate) fn parse_mcp_subcommand(input: &str) -> Option<Result<McpSubcommand, String>> {
    let mut parts = input.split_whitespace();
    let head = parts.next()?.to_ascii_lowercase();
    match head.as_str() {
        "off" => {
            if parts.next().is_some() {
                return Some(Err("usage: /mcp off".to_string()));
            }
            Some(Ok(McpSubcommand::Exit))
        }
        "exit" | "normal" | "clear" | "stop" => Some(Err("usage: /mcp off".to_string())),
        "clone" => {
            let Some(url) = parts.next() else {
                return Some(Err("usage: /mcp clone <git-url>".to_string()));
            };
            if parts.next().is_some() {
                return Some(Err("usage: /mcp clone <git-url>".to_string()));
            }
            Some(Ok(McpSubcommand::Clone(url.to_string())))
        }
        "list" => Some(Ok(McpSubcommand::List(parts.collect::<Vec<_>>().join(" ")))),
        "review" => {
            if parts.next().is_some() {
                return Some(Err("usage: /mcp review".to_string()));
            }
            Some(Ok(McpSubcommand::Review))
        }
        "activity" => Some(Ok(McpSubcommand::Activity(
            parts.collect::<Vec<_>>().join(" "),
        ))),
        "ps" | "runs" | "jobs" => Some(Err("usage: /mcp activity [query]".to_string())),
        "publish" => {
            if parts.next().is_some() {
                return Some(Err("usage: /mcp publish".to_string()));
            }
            Some(Ok(McpSubcommand::Publish))
        }
        "deploy" => {
            if parts.next().is_some() {
                return Some(Err("usage: /mcp deploy".to_string()));
            }
            Some(Ok(McpSubcommand::Deploy))
        }
        "debug" => {
            if parts.next().is_some() {
                return Some(Err("usage: /mcp debug".to_string()));
            }
            Some(Ok(McpSubcommand::Debug))
        }
        "run" | "invoke" => Some(Err(
            "MCP assets use /mcp debug for single tool calls and /mcp test for batch tests"
                .to_string(),
        )),
        "test" => {
            if parts.next().is_some() {
                return Some(Err("usage: /mcp test".to_string()));
            }
            Some(Ok(McpSubcommand::Test))
        }
        "batch" => Some(Err("usage: /mcp test".to_string())),
        "open" => {
            if parts.next().is_some() {
                return Some(Err("usage: /mcp open".to_string()));
            }
            Some(Ok(McpSubcommand::Open))
        }
        "logs" => {
            if parts.next().is_some() {
                return Some(Err("usage: /mcp logs".to_string()));
            }
            Some(Ok(McpSubcommand::Logs))
        }
        "status" => {
            if parts.next().is_some() {
                return Some(Err("usage: /mcp status".to_string()));
            }
            Some(Ok(McpSubcommand::Status))
        }
        "inspect" => Some(Err("usage: /mcp status".to_string())),
        "view" | "remote" => Some(Err("usage: /mcp open".to_string())),
        "os" => Some(Err("usage: /mcp status".to_string())),
        "dashboard" => Some(Err("usage: /mcp list [query] · /mcp status".to_string())),
        _ => None,
    }
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

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())
}

fn items_of(v: &serde_json::Value) -> Vec<serde_json::Value> {
    v.pointer("/data/items")
        .or_else(|| v.pointer("/data"))
        .or_else(|| v.pointer("/items"))
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default()
}

fn envelope_data(body: &serde_json::Value) -> &serde_json::Value {
    body.get("data").unwrap_or(body)
}

fn response_message(body: &serde_json::Value) -> &str {
    body.get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("request failed")
}

fn envelope_json_is_error(value: &serde_json::Value) -> bool {
    let code = value
        .get("code")
        .and_then(|v| v.as_i64())
        .or_else(|| value.get("statusCode").and_then(|v| v.as_i64()))
        .unwrap_or(200);
    code >= 400
}

fn json_str_at<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        value
            .pointer(key)
            .or_else(|| value.get(*key))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })
}

fn mcp_asset_name(name: &str) -> String {
    format!("mcp-{}", asset_slug(name))
}

fn mcp_asset_url(origin: &str, asset_id: &str) -> String {
    format!(
        "{}/admin/assets/{}?embed=1",
        origin.trim_end_matches('/'),
        path_segment(asset_id)
    )
}

fn mcp_asset_search_url(origin: &str, asset_name: &str) -> String {
    format!(
        "{}/admin/kernel/assets?focus=1&category=mcp&search={}&embed=1",
        origin.trim_end_matches('/'),
        path_segment(asset_name)
    )
}

fn mcp_logs_url(origin: &str, asset_id: &str) -> String {
    format!(
        "{}/admin/infrastructure/batch?asset={}&agentKind=tool&category=mcp&logs=1&embed=1",
        origin.trim_end_matches('/'),
        path_segment(asset_id)
    )
}

fn mcp_function_view_url(origin: &str, asset_id: &str) -> String {
    format!(
        "{}/admin/infrastructure/batch?asset={}&agentKind=tool&category=mcp&embed=1",
        origin.trim_end_matches('/'),
        path_segment(asset_id)
    )
}

fn mcp_view_spec(url: String) -> remote_ui::ViewSpec {
    remote_ui::ViewSpec {
        url,
        width: Some(1440),
        height: Some(900),
        embeddable: true,
    }
}

fn mcp_metadata_value(dev: &McpDevSession) -> serde_json::Value {
    for marker in [".a3s/mcp.server.json", "mcp.server.json", "mcp.json"] {
        let path = dev.path.join(marker);
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if value.is_object() {
            return value;
        }
    }
    if let Ok(text) = std::fs::read_to_string(dev.path.join("package.json")) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            let name = json_str_at(&value, &["/name", "name"]).unwrap_or(&dev.name);
            let description =
                json_str_at(&value, &["/description", "description"]).unwrap_or(&dev.description);
            let main = json_str_at(&value, &["/main", "main"]).unwrap_or("index.js");
            return serde_json::json!({
                "name": name,
                "description": description,
                "transport": "stdio",
                "entrypoint": main,
                "tools": [],
            });
        }
    }
    serde_json::json!({
        "name": dev.name,
        "description": dev.description,
        "transport": "stdio",
        "entrypoint": "README.md",
        "tools": [],
    })
}

fn mcp_config_name(config: &serde_json::Value, dev: &McpDevSession) -> String {
    json_str_at(config, &["/name", "name"])
        .unwrap_or(&dev.name)
        .to_string()
}

fn mcp_config_description(config: &serde_json::Value, dev: &McpDevSession) -> String {
    json_str_at(config, &["/description", "description"])
        .unwrap_or(&dev.description)
        .to_string()
}

fn mcp_server_config_json(dev: &McpDevSession, raw: &serde_json::Value) -> serde_json::Value {
    let mut config = raw.clone();
    if !config.is_object() {
        config = serde_json::json!({});
    }
    let name = mcp_config_name(&config, dev);
    let description = mcp_config_description(&config, dev);
    let obj = config.as_object_mut().expect("object ensured");
    obj.entry("name".to_string())
        .or_insert_with(|| serde_json::json!(name));
    obj.entry("description".to_string())
        .or_insert_with(|| serde_json::json!(description));
    obj.entry("category".to_string())
        .or_insert_with(|| serde_json::json!("mcp"));
    obj.entry("runtimeIntent".to_string()).or_insert_with(|| {
        serde_json::json!({
            "service": "Function as a Service",
            "runtimeBinding": {
                "kind": "mcp",
                "isolation": "serving",
                "agentKind": "tool",
                "runtimeKind": "a3s-function-service",
                "protocol": "mcp",
            },
        })
    });
    config
}

fn mcp_tool_names(config: &serde_json::Value) -> Vec<String> {
    let Some(tools) = config.get("tools") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match tools {
        serde_json::Value::Array(items) => {
            for item in items {
                match item {
                    serde_json::Value::String(name) => out.push(name.trim().to_string()),
                    serde_json::Value::Object(_) => {
                        if let Some(name) = json_str_at(item, &["/name", "name", "/id", "id"]) {
                            out.push(name.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        serde_json::Value::Object(map) => {
            out.extend(map.keys().cloned());
        }
        _ => {}
    }
    out.retain(|name| !name.trim().is_empty());
    out.sort();
    out.dedup();
    out
}

fn mcp_manifest_json(
    dev: &McpDevSession,
    asset_name: &str,
    server_config: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "version": "a3s.mcp.asset.v1",
        "category": "mcp",
        "name": asset_name,
        "mcpName": mcp_config_name(server_config, dev),
        "description": mcp_config_description(server_config, dev),
        "serverConfigPath": MCP_SERVER_CONFIG_PATH,
        "runtimeBindingPath": MCP_RUNTIME_BINDING_PATH,
        "localPath": dev.rel,
        "service": "Function as a Service",
        "runtimeIntent": {
            "kind": "mcp",
            "isolation": "serving",
            "agentKind": "tool",
            "runtimeKind": "a3s-function-service",
            "protocol": "mcp",
        },
        "createdBy": "a3s-code-tui",
        "tools": mcp_tool_names(server_config),
    })
}

fn mcp_runtime_binding_json(
    dev: &McpDevSession,
    asset_name: &str,
    server_config: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "version": "a3s.mcp.runtime-binding.v1",
        "kind": "mcp",
        "enabled": true,
        "isolation": "serving",
        "target": {
            "kind": "asset",
            "ref": "main",
            "packageRoot": ".",
            "configPath": MCP_SERVER_CONFIG_PATH,
        },
        "runtime": {
            "kind": "a3s-function-service",
            "protocol": "mcp",
            "agentKind": "tool",
        },
        "env": [],
        "requiredSecrets": [],
        "resources": {
            "concurrency": 1,
        },
        "network": {
            "ingress": false,
        },
        "metadata": {
            "source": "a3s-code-tui",
            "assetCategory": "mcp",
            "assetName": asset_name,
            "mcpName": mcp_config_name(server_config, dev),
            "description": mcp_config_description(server_config, dev),
            "serverConfigPath": MCP_SERVER_CONFIG_PATH,
            "localPath": dev.rel,
            "tools": mcp_tool_names(server_config),
        },
    })
}

fn collect_mcp_source_files(root: &std::path::Path) -> Result<Vec<McpSourceFile>, String> {
    let mut out = Vec::new();
    let mut total = 0_u64;
    collect_mcp_source_files_inner(root, root, &mut out, &mut total)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn collect_mcp_source_files_inner(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<McpSourceFile>,
    total: &mut u64,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("could not read MCP asset directory {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if should_skip_mcp_dir(&name) {
                continue;
            }
            collect_mcp_source_files_inner(root, &path, out, total)?;
            continue;
        }
        if !path.is_file() || should_skip_mcp_file(&name) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if [
            MCP_MANIFEST_PATH,
            MCP_SERVER_CONFIG_PATH,
            MCP_RUNTIME_BINDING_PATH,
        ]
        .contains(&rel.as_str())
        {
            continue;
        }
        let meta = std::fs::metadata(&path)
            .map_err(|e| format!("could not stat MCP asset file {}: {e}", path.display()))?;
        *total = total.saturating_add(meta.len());
        if out.len() >= MAX_MCP_SOURCE_FILES || *total > MAX_MCP_SOURCE_BYTES {
            return Err(format!(
                "MCP asset is too large for direct asset upload (limit: {MAX_MCP_SOURCE_FILES} files / {MAX_MCP_SOURCE_BYTES} bytes)"
            ));
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("could not read MCP asset file {}: {e}", path.display()))?;
        out.push(McpSourceFile { path: rel, bytes });
    }
    Ok(())
}

fn should_skip_mcp_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "target" | "dist" | "build" | ".venv" | "__pycache__"
    )
}

fn should_skip_mcp_file(name: &str) -> bool {
    name == ".DS_Store" || name.starts_with(".env")
}

fn mcp_asset_ref_from_value(asset: &serde_json::Value, fallback_name: &str) -> Option<McpAssetRef> {
    let id = json_str_at(asset, &["/id", "id"])?.to_string();
    Some(McpAssetRef {
        id,
        name: json_str_at(asset, &["/name", "name"])
            .unwrap_or(fallback_name)
            .to_string(),
    })
}

fn asset_category(value: &serde_json::Value) -> Option<&str> {
    json_str_at(
        value,
        &[
            "/category",
            "category",
            "/assetCategory",
            "assetCategory",
            "/assetType",
            "assetType",
            "/asset/category",
            "/metadata/category",
        ],
    )
}

fn category_conflict_error(name: &str, actual: &str, expected: &str) -> String {
    format!("asset `{name}` already exists with category={actual}; expected {expected}")
}

fn find_mcp_asset(found: &serde_json::Value, name: &str) -> Result<Option<McpAssetRef>, String> {
    let exact = items_of(found)
        .into_iter()
        .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(name));
    let Some(asset) = exact else {
        return Ok(None);
    };
    if let Some(actual) = asset_category(&asset) {
        if !actual.eq_ignore_ascii_case("mcp") {
            return Err(category_conflict_error(name, actual, "mcp"));
        }
    }
    mcp_asset_ref_from_value(&asset, name)
        .map(Some)
        .ok_or_else(|| format!("asset `{name}` matched but had no id"))
}

async fn lookup_mcp_asset(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    name: &str,
) -> Result<Option<McpAssetRef>, String> {
    let base = format!("{}/api/v1/assets", origin.trim_end_matches('/'));
    let found: serde_json::Value = client
        .get(&base)
        .query(&[("search", name), ("category", "mcp"), ("limit", "50")])
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    find_mcp_asset(&found, name)
}

async fn ensure_mcp_asset(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    name: &str,
    description: &str,
) -> Result<McpAssetRef, String> {
    if let Some(asset) = lookup_mcp_asset(client, origin, token, name).await? {
        return Ok(asset);
    }
    let resp = client
        .post(format!("{}/api/v1/assets", origin.trim_end_matches('/')))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": name,
            "ownerType": "user",
            "category": "mcp",
            "visibility": "private",
            "description": description,
            "metadata": {
                "runtimeBindingKind": "mcp",
                "service": "Function as a Service",
                "agentKind": "tool",
                "runtimeKind": "a3s-function-service",
                "protocol": "mcp",
                "createdBy": "a3s-code-tui",
            },
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() || envelope_json_is_error(&body) {
        return Err(format!(
            "create MCP asset failed ({status}): {}",
            response_message(&body)
        ));
    }
    mcp_asset_ref_from_value(envelope_data(&body), name)
        .ok_or_else(|| "create MCP asset: no id in response".to_string())
}

async fn upload_mcp_project(
    origin: &str,
    token: &str,
    asset_id: &str,
    source_files: &[McpSourceFile],
    manifest: &serde_json::Value,
    server_config: &serde_json::Value,
    runtime_binding: &serde_json::Value,
) -> Result<(), String> {
    use base64::Engine;

    let mut files = Vec::new();
    for file in source_files {
        files.push(serde_json::json!({
            "path": file.path,
            "contentBase64": base64::engine::general_purpose::STANDARD.encode(&file.bytes),
        }));
    }
    files.push(serde_json::json!({
        "path": MCP_MANIFEST_PATH,
        "contentBase64": base64::engine::general_purpose::STANDARD.encode(
            serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?
        ),
    }));
    files.push(serde_json::json!({
        "path": MCP_SERVER_CONFIG_PATH,
        "contentBase64": base64::engine::general_purpose::STANDARD.encode(
            serde_json::to_vec_pretty(server_config).map_err(|e| e.to_string())?
        ),
    }));
    files.push(serde_json::json!({
        "path": MCP_RUNTIME_BINDING_PATH,
        "contentBase64": base64::engine::general_purpose::STANDARD.encode(
            serde_json::to_vec_pretty(runtime_binding).map_err(|e| e.to_string())?
        ),
    }));

    let resp = http()?
        .post(format!(
            "{}/api/v1/assets/{}/repository/files",
            origin.trim_end_matches('/'),
            path_segment(asset_id)
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "overwrite": true,
            "message": "a3s code /mcp: update MCP server asset",
            "files": files,
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
        "upload MCP asset failed ({status}): {}",
        truncate(&body, 200)
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum McpRuntimeBindingSync {
    Synced,
    Unsupported,
    Failed(String),
}

async fn sync_mcp_runtime_binding(
    origin: &str,
    token: &str,
    asset_id: &str,
    runtime_binding: &serde_json::Value,
) -> McpRuntimeBindingSync {
    match sync_mcp_runtime_binding_inner(origin, token, asset_id, runtime_binding).await {
        Ok(true) => McpRuntimeBindingSync::Synced,
        Ok(false) => McpRuntimeBindingSync::Unsupported,
        Err(err) => McpRuntimeBindingSync::Failed(err),
    }
}

async fn sync_mcp_runtime_binding_inner(
    origin: &str,
    token: &str,
    asset_id: &str,
    runtime_binding: &serde_json::Value,
) -> Result<bool, String> {
    let client = http()?;
    let base = format!(
        "{}/api/v1/assets/{}/runtime-binding",
        origin.trim_end_matches('/'),
        path_segment(asset_id)
    );
    let put_resp = client
        .put(&base)
        .bearer_auth(token)
        .json(&mcp_runtime_binding_upsert_body(runtime_binding))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let put_status = put_resp.status();
    let put_text = put_resp.text().await.unwrap_or_default();
    if matches!(put_status.as_u16(), 404 | 405) {
        return Ok(false);
    }
    if !put_status.is_success() {
        return Err(format!("OS runtime-binding sync failed ({put_status})"));
    }
    if serde_json::from_str::<serde_json::Value>(&put_text)
        .ok()
        .is_some_and(|value| envelope_json_is_error(&value))
    {
        return Err("OS runtime-binding sync failed".to_string());
    }

    let validate_resp = client
        .post(format!("{base}/validate"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let validate_status = validate_resp.status();
    let validate_text = validate_resp.text().await.unwrap_or_default();
    if matches!(validate_status.as_u16(), 404 | 405) {
        return Ok(true);
    }
    if !validate_status.is_success() {
        return Err(format!(
            "OS runtime-binding validation failed ({validate_status})"
        ));
    }
    let validate_json: serde_json::Value =
        serde_json::from_str(&validate_text).map_err(|e| e.to_string())?;
    if envelope_json_is_error(&validate_json) {
        return Err("OS runtime-binding validation failed".to_string());
    }
    if validate_json
        .pointer("/data/valid")
        .and_then(|value| value.as_bool())
        == Some(false)
    {
        let issues = validate_json
            .pointer("/data/issues")
            .map(|value| truncate(&value.to_string(), 180))
            .unwrap_or_else(|| "no issues".to_string());
        return Err(format!(
            "OS runtime-binding validation reported invalid binding: {issues}"
        ));
    }
    Ok(true)
}

fn mcp_runtime_binding_upsert_body(runtime_binding: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "kind": runtime_binding
            .get("kind")
            .and_then(|value| value.as_str())
            .unwrap_or("mcp"),
        "isolation": runtime_binding
            .get("isolation")
            .and_then(|value| value.as_str())
            .unwrap_or("serving"),
        "target": runtime_binding
            .get("target")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"kind": "asset", "ref": "main"})),
        "runtime": runtime_binding
            .get("runtime")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| serde_json::json!({
                "kind": "a3s-function-service",
                "protocol": "mcp",
                "agentKind": "tool",
            })),
        "env": runtime_binding
            .get("env")
            .filter(|value| value.is_array())
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
        "requiredSecrets": runtime_binding
            .get("requiredSecrets")
            .filter(|value| value.is_array())
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
        "resources": runtime_binding
            .get("resources")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
        "network": runtime_binding
            .get("network")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
        "enabled": runtime_binding
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        "metadata": runtime_binding
            .get("metadata")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    })
}

async fn runtime_binding_validation_status(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    asset_id: &str,
) -> String {
    let base = format!(
        "{}/api/v1/assets/{}/runtime-binding",
        origin.trim_end_matches('/'),
        path_segment(asset_id)
    );
    let resp = match client.get(&base).bearer_auth(token).send().await {
        Ok(resp) => resp,
        Err(err) => {
            return format!(
                "runtime-binding check failed: {}",
                truncate(&err.to_string(), 120)
            )
        }
    };
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if matches!(status.as_u16(), 404 | 405) {
        return "runtime-binding endpoint unavailable".to_string();
    }
    if !status.is_success() {
        return format!("runtime-binding read failed ({status})");
    }
    let Ok(body) = serde_json::from_str::<serde_json::Value>(&text) else {
        return "runtime-binding read returned unreadable JSON".to_string();
    };
    if envelope_json_is_error(&body) {
        return "runtime-binding read failed".to_string();
    }
    if envelope_data(&body)
        .get("configured")
        .and_then(|value| value.as_bool())
        == Some(false)
    {
        return "runtime-binding missing".to_string();
    }

    let resp = match client
        .post(format!("{base}/validate"))
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(err) => {
            return format!(
                "runtime-binding validation failed: {}",
                truncate(&err.to_string(), 120)
            )
        }
    };
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if matches!(status.as_u16(), 404 | 405) {
        return "runtime-binding saved; validation endpoint unavailable".to_string();
    }
    if !status.is_success() {
        return format!("runtime-binding validation failed ({status})");
    }
    let Ok(body) = serde_json::from_str::<serde_json::Value>(&text) else {
        return "runtime-binding validation returned unreadable JSON".to_string();
    };
    if envelope_json_is_error(&body) {
        return "runtime-binding validation failed".to_string();
    }
    if body
        .pointer("/data/valid")
        .and_then(|value| value.as_bool())
        == Some(false)
    {
        let issues = body
            .pointer("/data/issues")
            .map(|value| truncate(&value.to_string(), 140))
            .unwrap_or_else(|| "no issues".to_string());
        return format!("runtime-binding invalid: {issues}");
    }
    "runtime-binding valid".to_string()
}

async fn inspect_mcp_asset(
    origin: &str,
    token: &str,
    action: McpOsAction,
    asset_name: &str,
    mcp_name: &str,
) -> Result<McpOsResult, String> {
    let client = http()?;
    let Some(asset) = lookup_mcp_asset(&client, origin, token, asset_name).await? else {
        return Ok(McpOsResult {
            action,
            asset_name: asset_name.to_string(),
            asset_id: "not-published".to_string(),
            view: mcp_view_spec(mcp_asset_search_url(origin, asset_name)),
            note: format!(
                "OS status for `{mcp_name}`: no Function as a Service MCP asset named `{asset_name}` was found. Run `/mcp publish` first."
            ),
            open_view: false,
        });
    };
    let runtime_binding_status =
        runtime_binding_validation_status(&client, origin, token, &asset.id).await;
    if matches!(action, McpOsAction::Open | McpOsAction::Logs) {
        let fallback = match action {
            McpOsAction::Open => mcp_view_spec(mcp_asset_url(origin, &asset.id)),
            McpOsAction::Logs => mcp_view_spec(mcp_logs_url(origin, &asset.id)),
            _ => unreachable!(),
        };
        let (view, note) =
            try_mcp_progressive_observe(&client, origin, token, action, &asset, mcp_name)
                .await
                .unwrap_or_else(|| {
                    let surface = match action {
                        McpOsAction::Open => "Function as a Service MCP asset view",
                        McpOsAction::Logs => "Function as a Service MCP logs view",
                        _ => unreachable!(),
                    };
                    (
                        fallback,
                        format!("Opened OS MCP asset through the {surface}."),
                    )
                });
        return Ok(McpOsResult {
            action,
            asset_name: asset.name,
            asset_id: asset.id,
            view,
            note,
            open_view: true,
        });
    }
    Ok(McpOsResult {
        action,
        asset_name: asset.name,
        asset_id: asset.id.clone(),
        view: mcp_view_spec(mcp_asset_url(origin, &asset.id)),
        note: format!("OS status for `{mcp_name}`: asset exists; {runtime_binding_status}."),
        open_view: false,
    })
}

fn mcp_function_input(
    mode: &str,
    asset: &McpAssetRef,
    server_config: &serde_json::Value,
    tool_name: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "assetId": asset.id,
        "assetName": asset.name,
        "mode": mode,
        "protocol": "mcp",
        "tool": tool_name,
        "arguments": {},
        "source": "a3s-code-tui",
        "serverConfig": server_config,
    })
}

fn mcp_function_ref(asset: &McpAssetRef) -> &str {
    &asset.name
}

fn mcp_progressive_params(
    asset: &McpAssetRef,
    server_config: &serde_json::Value,
    input: serde_json::Value,
    inputs: Option<Vec<serde_json::Value>>,
) -> serde_json::Value {
    let config = serde_json::json!({
        "assetId": asset.id,
        "assetName": asset.name,
        "category": "mcp",
        "protocol": "mcp",
    });
    let mut params = serde_json::json!({
        "ref": mcp_function_ref(asset),
        "functionRef": mcp_function_ref(asset),
        "worker": mcp_function_ref(asset),
        "name": mcp_function_ref(asset),
        "assetId": asset.id,
        "assetName": asset.name,
        "input": input,
        "agentKind": "tool",
        "config": config,
        "timeoutMs": 120000,
        "idempotencyKey": format!("a3s-code-mcp-progressive-{}", unix_timestamp_secs()),
        "serverConfig": server_config,
    });
    if let Some(inputs) = inputs {
        params["inputs"] = serde_json::Value::Array(inputs);
    }
    params
}

fn mcp_observe_progressive_params(
    asset: &McpAssetRef,
    mcp_name: &str,
    action: McpOsAction,
) -> serde_json::Value {
    serde_json::json!({
        "assetId": asset.id,
        "assetName": asset.name,
        "functionRef": asset.name,
        "ref": asset.name,
        "worker": asset.name,
        "name": asset.name,
        "mcpName": mcp_name,
        "category": "mcp",
        "agentKind": "tool",
        "operation": action.label(),
        "input": {
            "assetId": asset.id,
            "assetName": asset.name,
            "mcpName": mcp_name,
            "operation": action.label(),
            "source": "a3s-code-tui",
        },
        "payload": {
            "assetId": asset.id,
            "assetName": asset.name,
            "mcpName": mcp_name,
            "operation": action.label(),
            "source": "a3s-code-tui",
        },
        "timeoutMs": 120000,
        "idempotencyKey": format!("a3s-code-mcp-{}-{}", action.label(), unix_timestamp_secs()),
    })
}

fn mcp_progressive_score(action: McpOsAction, text: &str, operation: &str) -> i32 {
    let combined = format!("{text} {operation}").to_ascii_lowercase();
    let mut score = 0;
    if combined.contains("function") || combined.contains("faas") {
        score += 8;
    }
    if combined.contains("mcp") {
        score += 8;
    }
    if combined.contains("tool") {
        score += 3;
    }
    let mut action_hit = false;
    match action {
        McpOsAction::Debug => {
            if combined.contains("invoke") || combined.contains("invocation") {
                score += 12;
                action_hit = true;
            }
            if combined.contains("batch") {
                score -= 5;
            }
        }
        McpOsAction::Test => {
            if combined.contains("batch") {
                score += 12;
                action_hit = true;
            }
            if combined.contains("invoke") && !combined.contains("batch") {
                score -= 4;
            }
        }
        McpOsAction::Open => {
            if combined.contains("open")
                || combined.contains("view")
                || combined.contains("remoteui")
                || combined.contains("manage")
                || combined.contains("asset view")
            {
                score += 10;
                action_hit = true;
            }
            if combined.contains("logs") || combined.contains("log view") {
                score -= 4;
            }
        }
        McpOsAction::Logs
            if combined.contains("log")
                || combined.contains("trace")
                || combined.contains("job")
                || combined.contains("process")
                || combined.contains("observability") =>
        {
            score += 10;
            action_hit = true;
        }
        _ => {}
    }
    if combined.contains("agent as a service") || combined.contains("workflow") {
        score -= 10;
    }
    if matches!(
        action,
        McpOsAction::Debug | McpOsAction::Test | McpOsAction::Open | McpOsAction::Logs
    ) && !action_hit
    {
        0
    } else if score >= 8 {
        score
    } else {
        0
    }
}

async fn try_mcp_progressive_observe(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    action: McpOsAction,
    asset: &McpAssetRef,
    mcp_name: &str,
) -> Option<(remote_ui::ViewSpec, String)> {
    let query = match action {
        McpOsAction::Open => "Function as a Service open MCP asset shaped ViewLink",
        McpOsAction::Logs => "Function as a Service MCP runtime logs shaped ViewLink",
        _ => return None,
    };
    let execution = os_progressive::execute_first_matching(
        client,
        origin,
        token,
        query,
        mcp_observe_progressive_params(asset, mcp_name, action),
        |text, operation| mcp_progressive_score(action, text, operation),
    )
    .await?;
    let fallback = match action {
        McpOsAction::Open => mcp_view_spec(mcp_asset_url(origin, &asset.id)),
        McpOsAction::Logs => mcp_view_spec(mcp_logs_url(origin, &asset.id)),
        _ => unreachable!(),
    };
    Some((
        execution.view.unwrap_or(fallback),
        format!(
            "OS Function as a Service accepted MCP `{}` through progressive capabilities (`{}`).",
            action.label(),
            execution.operation.operation
        ),
    ))
}

#[allow(clippy::too_many_arguments)]
async fn try_mcp_progressive_function(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    action: McpOsAction,
    asset: &McpAssetRef,
    server_config: &serde_json::Value,
    input: serde_json::Value,
    inputs: Option<Vec<serde_json::Value>>,
) -> Option<(remote_ui::ViewSpec, String)> {
    let query = match action {
        McpOsAction::Debug => "Function as a Service invoke MCP tool asset shaped view",
        McpOsAction::Test => "Function as a Service batch MCP tools shaped view",
        _ => return None,
    };
    let params = mcp_progressive_params(asset, server_config, input, inputs);
    let execution = os_progressive::execute_first_matching(
        client,
        origin,
        token,
        query,
        params,
        |text, operation| mcp_progressive_score(action, text, operation),
    )
    .await?;
    let view = execution
        .view
        .unwrap_or_else(|| mcp_view_spec(mcp_function_view_url(origin, &asset.id)));
    let note = match action {
        McpOsAction::Debug => format!(
            "OS Function as a Service accepted the MCP debug invoke through progressive capabilities (`{}`).",
            execution.operation.operation
        ),
        McpOsAction::Test => format!(
            "OS Function as a Service accepted the MCP batch test through progressive capabilities (`{}`).",
            execution.operation.operation
        ),
        _ => unreachable!(),
    };
    Some((view, note))
}

async fn invoke_mcp_function(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    asset: &McpAssetRef,
    server_config: &serde_json::Value,
) -> Option<(remote_ui::ViewSpec, String)> {
    let tool = mcp_tool_names(server_config).into_iter().next();
    let input = mcp_function_input("debug", asset, server_config, tool.as_deref());
    if let Some(result) = try_mcp_progressive_function(
        client,
        origin,
        token,
        McpOsAction::Debug,
        asset,
        server_config,
        input.clone(),
        None,
    )
    .await
    {
        return Some(result);
    }
    let resp = client
        .post(format!(
            "{}/api/v1/functions/{}/invoke",
            origin.trim_end_matches('/'),
            path_segment(mcp_function_ref(asset))
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "input": input,
            "agentKind": "tool",
            "config": {
                "assetId": asset.id,
                "assetName": asset.name,
                "category": "mcp",
                "protocol": "mcp",
            },
            "timeoutMs": 120000,
            "idempotencyKey": format!("a3s-code-mcp-debug-{}", unix_timestamp_secs()),
        }))
        .send()
        .await
        .ok()?;
    let status = resp.status();
    let text = resp.text().await.ok()?;
    let fallback = mcp_view_spec(mcp_function_view_url(origin, &asset.id));
    if !status.is_success() || envelope_text_is_error(&text) {
        return Some((
            fallback,
            format!(
                "Published `{}`; OS Function as a Service debug invoke was unavailable ({}).",
                asset.name,
                truncate(&text, 160)
            ),
        ));
    }
    Some((
        remote_ui::find_view_url(&text, Some(origin)).unwrap_or(fallback),
        format!(
            "OS Function as a Service accepted the MCP debug invoke for `{}`.",
            asset.name
        ),
    ))
}

async fn batch_test_mcp_function(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    asset: &McpAssetRef,
    server_config: &serde_json::Value,
) -> Option<(remote_ui::ViewSpec, String)> {
    let mut tools = mcp_tool_names(server_config);
    if tools.is_empty() {
        tools.push("list-tools".to_string());
    }
    let inputs = tools
        .iter()
        .take(3)
        .map(|tool| mcp_function_input("test", asset, server_config, Some(tool)))
        .collect::<Vec<_>>();
    let primary_input = inputs
        .first()
        .cloned()
        .unwrap_or_else(|| mcp_function_input("test", asset, server_config, None));
    if let Some(result) = try_mcp_progressive_function(
        client,
        origin,
        token,
        McpOsAction::Test,
        asset,
        server_config,
        primary_input,
        Some(inputs.clone()),
    )
    .await
    {
        return Some(result);
    }
    let resp = client
        .post(format!(
            "{}/api/v1/functions/{}/batch",
            origin.trim_end_matches('/'),
            path_segment(mcp_function_ref(asset))
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "inputs": inputs,
            "agentKind": "tool",
            "config": {
                "assetId": asset.id,
                "assetName": asset.name,
                "category": "mcp",
                "protocol": "mcp",
            },
            "timeoutMs": 120000,
            "idempotencyKey": format!("a3s-code-mcp-test-{}", unix_timestamp_secs()),
        }))
        .send()
        .await
        .ok()?;
    let status = resp.status();
    let text = resp.text().await.ok()?;
    let fallback = mcp_view_spec(mcp_function_view_url(origin, &asset.id));
    if !status.is_success() || envelope_text_is_error(&text) {
        return Some((
            fallback,
            format!(
                "Published `{}`; OS Function as a Service batch test was unavailable ({}).",
                asset.name,
                truncate(&text, 160)
            ),
        ));
    }
    Some((
        remote_ui::find_view_url(&text, Some(origin)).unwrap_or(fallback),
        format!(
            "OS Function as a Service accepted the MCP batch test for `{}`.",
            asset.name
        ),
    ))
}

fn envelope_text_is_error(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .is_some_and(|value| envelope_json_is_error(&value))
}

fn unix_timestamp_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn append_mcp_runtime_binding_sync_note(
    mut note: String,
    runtime_binding_synced: &McpRuntimeBindingSync,
) -> String {
    match runtime_binding_synced {
        McpRuntimeBindingSync::Synced => {
            note.push_str(" OS runtime binding was synced.");
        }
        McpRuntimeBindingSync::Unsupported => {
            note.push_str(
                " OS runtime-binding endpoint was unavailable; runtime-binding intent was saved.",
            );
        }
        McpRuntimeBindingSync::Failed(err) => {
            note.push_str(&format!(
                " OS runtime binding could not be synced: {}; runtime-binding intent was saved.",
                truncate(err, 160)
            ));
        }
    }
    note
}

pub(crate) async fn publish_mcp_to_os(
    session: crate::a3s_os::StoredOsSession,
    dev: McpDevSession,
    action: McpOsAction,
) -> Result<McpOsResult, String> {
    let origin = crate::a3s_os::os_origin(&session.address);
    let raw_config = mcp_metadata_value(&dev);
    let server_config = mcp_server_config_json(&dev, &raw_config);
    let mcp_name = mcp_config_name(&server_config, &dev);
    let description = mcp_config_description(&server_config, &dev);
    let asset_name = mcp_asset_name(&mcp_name);

    if matches!(
        action,
        McpOsAction::Status | McpOsAction::Open | McpOsAction::Logs
    ) {
        return inspect_mcp_asset(
            &origin,
            &session.access_token,
            action,
            &asset_name,
            &mcp_name,
        )
        .await;
    }

    let client = http()?;
    let asset = ensure_mcp_asset(
        &client,
        &origin,
        &session.access_token,
        &asset_name,
        &description,
    )
    .await?;
    let source_files = collect_mcp_source_files(&dev.path)?;
    let manifest = mcp_manifest_json(&dev, &asset_name, &server_config);
    let runtime_binding = mcp_runtime_binding_json(&dev, &asset_name, &server_config);
    upload_mcp_project(
        &origin,
        &session.access_token,
        &asset.id,
        &source_files,
        &manifest,
        &server_config,
        &runtime_binding,
    )
    .await?;
    let runtime_binding_synced =
        sync_mcp_runtime_binding(&origin, &session.access_token, &asset.id, &runtime_binding).await;

    let (view, note) = match action {
        McpOsAction::Publish => (
            mcp_view_spec(mcp_asset_url(&origin, &asset.id)),
            format!(
                "Published `{mcp_name}` as an OS MCP asset backed by Function as a Service."
            ),
        ),
        McpOsAction::Deploy => (
            mcp_view_spec(mcp_asset_url(&origin, &asset.id)),
            format!(
                "Deployed `{mcp_name}` by publishing its serving MCP runtime binding for Function as a Service."
            ),
        ),
        McpOsAction::Debug => invoke_mcp_function(
            &client,
            &origin,
            &session.access_token,
            &asset,
            &server_config,
        )
        .await
        .unwrap_or_else(|| {
            (
                mcp_view_spec(mcp_function_view_url(&origin, &asset.id)),
                format!("Published `{mcp_name}`; opened the Function as a Service view because debug invoke was unavailable."),
            )
        }),
        McpOsAction::Test => batch_test_mcp_function(
            &client,
            &origin,
            &session.access_token,
            &asset,
            &server_config,
        )
        .await
        .unwrap_or_else(|| {
            (
                mcp_view_spec(mcp_function_view_url(&origin, &asset.id)),
                format!("Published `{mcp_name}`; opened the Function as a Service view because batch test was unavailable."),
            )
        }),
        McpOsAction::Open | McpOsAction::Logs | McpOsAction::Status => {
            unreachable!("read-only MCP actions return before publish flow")
        }
    };
    let note = append_mcp_runtime_binding_sync_note(note, &runtime_binding_synced);
    Ok(McpOsResult {
        action,
        asset_name,
        asset_id: asset.id,
        view,
        note,
        open_view: true,
    })
}

fn mcp_picker_header(total: usize, root: &std::path::Path, width: usize) -> String {
    truncate(
        &format!(
            "  ◆ mcp — select a server asset ({total} in {})",
            root.to_string_lossy()
        ),
        width,
    )
}

fn mcp_picker_hint(width: usize) -> String {
    truncate("  ↑/↓ select · Enter local MCP dev · Esc cancel", width)
}

fn mcp_picker_lines(
    projects: &[McpProject],
    selected: usize,
    root: &std::path::Path,
    width: usize,
    height: usize,
) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let Some((panel, panel_height)) = mcp_picker_panel(projects, selected, root, width, height)
    else {
        return Vec::new();
    };

    panel
        .view(width.min(u16::MAX as usize) as u16, panel_height)
        .lines()
        .map(str::to_string)
        .collect()
}

fn mcp_picker_panel(
    projects: &[McpProject],
    selected: usize,
    root: &std::path::Path,
    width: usize,
    height: usize,
) -> Option<(MenuPanel, usize)> {
    let total = projects.len();
    if total == 0 {
        return None;
    }
    let max_items = height.saturating_sub(8).clamp(3, 12);
    let selected = selected.min(total.saturating_sub(1));
    let scroll = selected.saturating_add(1).saturating_sub(max_items);
    let items = projects
        .iter()
        .map(|project| MenuItem::new(project.rel.clone()).description(project.description.clone()))
        .collect::<Vec<_>>();

    let panel = MenuPanel::new(mcp_picker_header(total, root, width).trim_start())
        .subtitle(mcp_picker_hint(width).trim_start())
        .items(items)
        .selected(selected)
        .scroll(scroll)
        .max_items(max_items)
        .show_scroll(total > max_items)
        .indent(2)
        .marker("▸")
        .title_color(ACCENT)
        .subtitle_color(TN_GRAY)
        .text_color(TN_FG)
        .muted_color(TN_GRAY)
        .selected_colors(Color::BrightWhite, ACCENT);
    Some((panel, max_items + 3))
}

fn mcp_overlay_y_offset(screen_height: usize, row_count: usize) -> u16 {
    screen_height
        .saturating_sub(MCP_OVERLAY_ROWS_BELOW)
        .saturating_sub(row_count)
        .min(u16::MAX as usize) as u16
}

/// Directive for `/mcp <description>`: create a local MCP server asset.
pub(crate) fn mcp_gen_prompt(description: &str, dir: &str) -> String {
    format!(
        "Create a local MCP server asset from the description below and save it under \
         {dir}. This is a local authoring task: do not open OS, RemoteUI, or a browser.\n\
         Description: {description}\n\
         IMPORTANT: {dir} is OUTSIDE this session's workspace, so path-scoped file tools \
         may reject it. Use non-interactive bash commands with full quoted paths. Never run \
         a command that waits on stdin.\n\
         Create {dir}/<kebab-case-name>/ with a minimal runnable MCP server, README.md, \
         package.json or pyproject.toml, tool schema examples, .a3s/mcp.asset.json, \
         .a3s/mcp.server.json, and .a3s/mcp.runtime-binding.json. \
         The metadata file should include name, description, transport, entrypoint, tools, \
         category=mcp, service=Function as a Service, runtimeIntent.kind=mcp, isolation=serving, \
         runtime.kind=a3s-function-service, protocol=mcp, and agentKind=tool for future Function \
         invoke/batch calls.\n\
         Prefer stdio for local development. Add a small test or smoke script that lists \
         tools and calls one example tool. Validate JSON files with python3 -m json.tool, \
         then report the saved asset path and tell the user `/mcp` starts local MCP dev."
    )
}

pub(crate) fn mcp_dev_prompt(session: &McpDevSession, request: &str) -> String {
    format!(
        "You are in A3S Code local MCP-development mode.\n\
         Current MCP asset: {name}\n\
         Description: {description}\n\
         Asset path: {path}\n\
         MCP root: {root}\n\n\
         User request:\n{request}\n\n\
         Work on this local MCP server asset iteratively. Read the current asset files \
         before editing. Keep the server runnable locally, preserve valid tool schemas, and \
         keep metadata ready for OS Function as a Service: category=mcp, runtime binding \
         kind=mcp, serving isolation by default, and tool calls that can map to Function \
         invoke/batch with agentKind=tool. Do not open OS, WebIDE, RemoteUI, or browser pages \
         for this local MCP-dev turn. Validate changed JSON and run a lightweight local smoke \
         check when practical. End with a concise summary and the next best MCP Function as a Service step.\n\n\
         The TUI remains in MCP-development mode for `{name}` after this turn; the user can \
         press Esc or run `/mcp off` to return to normal mode.",
        name = session.name.as_str(),
        description = session.description.as_str(),
        path = session.path.display(),
        root = session.root.display(),
    )
}

pub(crate) fn mcp_review_prompt(session: &McpDevSession) -> String {
    let contract = super::review::review_report_contract(&session.path);
    format!(
        "Review this local MCP server asset without changing files unless the user explicitly asks \
         for fixes.\n\
         MCP name: {name}\n\
         Description: {description}\n\
         Project path: {path}\n\
         MCP root: {root}\n\n\
         Read the project metadata and key implementation files. Report concise findings on: MCP \
         protocol correctness, tool schemas, entrypoint/run command, local smoke tests, secret \
         handling, packaging, .a3s metadata, and readiness for Function as a Service. Mention the \
         smallest recommended improvements and whether `/mcp debug` or `/mcp deploy` is the right \
         next lifecycle step.{contract}",
        name = session.name.as_str(),
        description = session.description.as_str(),
        path = session.path.display(),
        root = session.root.display(),
    )
}

impl App {
    pub(crate) fn on_mcp_os_completed(&mut self, res: Result<McpOsResult, String>) {
        match res {
            Ok(result) => {
                self.last_view = Some(result.view.clone());
                self.push_line(&gutter(
                    TN_CYAN,
                    &format!(
                        "◆ /mcp {} · `{}` ({})",
                        result.action.label(),
                        result.asset_name,
                        result.asset_id
                    ),
                ));
                self.push_line(
                    &Style::new()
                        .fg(TN_GRAY)
                        .render(&format!("  {}", result.note)),
                );
                if result.open_view {
                    self.push_line(&gutter(
                        ACCENT,
                        &remote_view_button("MCP Function as a Service · click or /view reopens"),
                    ));
                    self.open_remote_view(&result.view);
                } else {
                    self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  /view opens the related OS MCP asset view"),
                    );
                }
            }
            Err(e) => {
                self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  /mcp OS operation failed: {e}")),
                );
            }
        }
    }

    pub(crate) fn exit_mcp_dev(&mut self) {
        match self.mcp_dev.take() {
            Some(session) => self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  mcp dev off — {} ({})",
                session.name, session.rel
            ))),
            None => self.push_line(&Style::new().fg(TN_GRAY).render("  mcp dev is not active")),
        }
        self.relayout();
    }

    /// Open the `/mcp` picker.
    pub(crate) fn open_mcp_panel(&mut self) {
        let root = mcp_dir();
        let projects = list_mcp_projects(&root);
        if projects.is_empty() {
            self.pending_mcp_subcommand = None;
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  no MCP assets in {} — draft one with `/mcp <description>` first",
                root.display()
            )));
            return;
        }
        self.mcp_picker = Some(McpPanel {
            root,
            projects,
            sel: 0,
        });
    }

    /// Keys while the `/mcp` picker is open — consumes everything.
    pub(crate) fn handle_mcp_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let p = self.mcp_picker.as_mut()?;
        let last = p.projects.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => p.sel = p.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => p.sel = (p.sel + 1).min(last),
            KeyCode::Esc => {
                cancel_pending_picker(&mut self.mcp_picker, &mut self.pending_mcp_subcommand)
            }
            KeyCode::Enter => {
                let panel = self.mcp_picker.take()?;
                let selected = panel.sel.min(last);
                return self.activate_mcp_panel_selection(panel, selected);
            }
            _ => {}
        }
        None
    }

    pub(crate) fn handle_mcp_mouse(&mut self, mouse: &MouseEvent) -> Option<Cmd<Msg>> {
        let panel_state = self.mcp_picker.as_ref()?;
        let total = panel_state.projects.len();
        if total == 0 {
            return None;
        }
        let width = (self.width as usize).min(u16::MAX as usize);
        if width == 0 {
            return None;
        }
        let selected = panel_state.sel.min(total - 1);
        let (mut panel, panel_height) = mcp_picker_panel(
            &panel_state.projects,
            selected,
            &panel_state.root,
            width,
            self.height as usize,
        )?;
        let row_count = panel.view(width as u16, panel_height).lines().count();
        if row_count == 0 {
            return None;
        }
        let y_offset = mcp_overlay_y_offset(self.height as usize, row_count);
        let row = mouse.row as usize;
        let start = y_offset as usize;
        if row < start || row >= start.saturating_add(row_count) {
            return None;
        }
        panel.set_y_offset(y_offset);
        let before = panel.selected_index();

        match panel.handle_mouse(mouse) {
            Some(MenuPanelMsg::Selected(index)) | Some(MenuPanelMsg::Toggled(index)) => {
                let panel_state = self.mcp_picker.take()?;
                self.activate_mcp_panel_selection(panel_state, index.min(total - 1))
            }
            Some(MenuPanelMsg::Cancelled) => {
                cancel_pending_picker(&mut self.mcp_picker, &mut self.pending_mcp_subcommand);
                None
            }
            None => {
                let after = panel.selected_index().min(total - 1);
                if after != before {
                    if let Some(open) = self.mcp_picker.as_mut() {
                        open.sel = after;
                    }
                }
                None
            }
        }
    }

    fn activate_mcp_panel_selection(
        &mut self,
        panel: McpPanel,
        selected: usize,
    ) -> Option<Cmd<Msg>> {
        let last = panel.projects.len().saturating_sub(1);
        let picked = panel.projects.get(selected.min(last))?.clone();
        self.agent_dev = None;
        self.skill_dev = None;
        self.okf_dev = None;
        self.mcp_dev = Some(McpDevSession {
            name: picked.name.clone(),
            description: picked.description.clone(),
            rel: picked.rel.clone(),
            path: picked.path.clone(),
            root: panel.root,
        });
        self.push_line(&gutter(
            TN_CYAN,
            &format!(
                "◆ mcp dev: {} ({}) · Esc or /mcp off returns to normal mode",
                picked.name, picked.rel
            ),
        ));
        self.relayout();
        if let Some(pending) = self.pending_mcp_subcommand.take() {
            return self.execute_mcp_subcommand(pending);
        }
        None
    }

    /// Overlay the `/mcp` picker above the input.
    pub(crate) fn overlay_mcp_menu(&self, composed: String) -> String {
        let Some(p) = self.mcp_picker.as_ref() else {
            return composed;
        };
        let width = self.width as usize;
        let menu = mcp_picker_lines(&p.projects, p.sel, &p.root, width, self.height as usize);
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("a3s-code-{name}-{}", std::process::id()))
    }

    #[test]
    fn existing_mcp_asset_must_match_mcp_category() {
        let found = serde_json::json!({
            "data": {
                "items": [
                    {
                        "id": "skill-asset",
                        "name": "mcp-weather-tools",
                        "category": "skill"
                    }
                ]
            }
        });

        let err = find_mcp_asset(&found, "mcp-weather-tools").unwrap_err();
        assert!(err.contains("category=skill"), "{err}");
        assert!(err.contains("expected mcp"), "{err}");
    }

    #[tokio::test]
    async fn ensure_mcp_asset_create_payload_carries_function_service_metadata() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_mcp_asset_create_mock(captured.clone()).await;
        let client = http().expect("client");

        let asset = ensure_mcp_asset(
            &client,
            &origin,
            "token",
            "mcp-weather-tools",
            "Weather tools",
        )
        .await
        .expect("asset should be created");

        assert_eq!(asset.id, "mcp-asset-1");
        let requests = captured.lock().unwrap().clone();
        let create = request_body(&requests, "POST /api/v1/assets HTTP/1.1");
        let create_json: serde_json::Value = serde_json::from_str(&create).unwrap();
        assert_eq!(create_json["category"], "mcp");
        assert_eq!(create_json["metadata"]["runtimeBindingKind"], "mcp");
        assert_eq!(create_json["metadata"]["service"], "Function as a Service");
        assert_eq!(create_json["metadata"]["agentKind"], "tool");
        assert_eq!(
            create_json["metadata"]["runtimeKind"],
            "a3s-function-service"
        );
        assert_eq!(create_json["metadata"]["protocol"], "mcp");
        assert_eq!(create_json["metadata"]["createdBy"], "a3s-code-tui");
    }

    async fn spawn_mcp_asset_create_mock(captured: Arc<Mutex<Vec<String>>>) -> String {
        spawn_mcp_os_mock(captured, mcp_asset_create_mock_response).await
    }

    async fn spawn_mcp_os_mock<F>(captured: Arc<Mutex<Vec<String>>>, responder: F) -> String
    where
        F: Fn(&str, &str) -> (&'static str, String) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let responder = Arc::new(responder);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let captured = captured.clone();
                let responder = responder.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let line = req.lines().next().unwrap_or("").to_string();
                    let body = req.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                    captured.lock().unwrap().push(format!("{line}\n{body}"));
                    let (status, payload) = responder(&line, &body);
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

    fn request_body(requests: &[String], prefix: &str) -> String {
        requests
            .iter()
            .find(|request| request.starts_with(prefix))
            .and_then(|request| request.split_once('\n').map(|(_, body)| body.to_string()))
            .unwrap_or_else(|| panic!("missing request {prefix}; got:\n{}", requests.join("\n")))
    }

    fn has_request(requests: &[String], prefix: &str) -> bool {
        requests.iter().any(|request| request.starts_with(prefix))
    }

    fn request_count(requests: &[String], prefix: &str) -> usize {
        requests
            .iter()
            .filter(|request| request.starts_with(prefix))
            .count()
    }

    fn assert_no_mcp_asset_mutation(requests: &[String]) {
        assert!(
            !has_request(requests, "POST /api/v1/assets HTTP/1.1"),
            "read-only MCP action must not create assets:\n{}",
            requests.join("\n")
        );
        assert!(
            !requests.iter().any(
                |request| request.starts_with("POST ") && request.contains("/repository/files")
            ),
            "read-only MCP action must not upload repository files:\n{}",
            requests.join("\n")
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.starts_with("PUT ") && request.contains("/runtime-binding")),
            "read-only MCP action must not write runtime binding:\n{}",
            requests.join("\n")
        );
    }

    fn mcp_asset_create_mock_response(line: &str, body: &str) -> (&'static str, String) {
        if line.starts_with("GET /api/v1/assets?") {
            return ("200 OK", r#"{"data":{"items":[]}}"#.to_string());
        }
        if line.starts_with("POST /api/v1/assets HTTP/1.1") {
            if body.contains(r#""category":"mcp""#)
                && body.contains(r#""runtimeBindingKind":"mcp""#)
                && body.contains(r#""service":"Function as a Service""#)
                && body.contains(r#""agentKind":"tool""#)
                && body.contains(r#""runtimeKind":"a3s-function-service""#)
                && body.contains(r#""protocol":"mcp""#)
            {
                return (
                    "200 OK",
                    r#"{"data":{"id":"mcp-asset-1","name":"mcp-weather-tools"}}"#.to_string(),
                );
            }
            return (
                "422 Unprocessable Entity",
                r#"{"message":"bad MCP asset body"}"#.to_string(),
            );
        }
        ("404 Not Found", r#"{"message":"not found"}"#.to_string())
    }

    fn mcp_existing_asset_mock_response(line: &str, _body: &str) -> (&'static str, String) {
        if line.starts_with("GET /api/v1/assets?") {
            return (
                "200 OK",
                r#"{"data":{"items":[{"id":"mcp-asset-1","name":"mcp-weather-tools","category":"mcp"}]}}"#.to_string(),
            );
        }
        if line.starts_with("GET /api/v1/assets/mcp-asset-1/runtime-binding ") {
            return (
                "200 OK",
                r#"{"data":{"configured":true,"kind":"mcp"}}"#.to_string(),
            );
        }
        if line.starts_with("POST /api/v1/assets/mcp-asset-1/runtime-binding/validate ") {
            return ("200 OK", r#"{"data":{"valid":true}}"#.to_string());
        }
        if line.starts_with("POST /api/v1/kernel/capabilities ") {
            return (
                "404 Not Found",
                r#"{"code":404,"message":"capabilities unavailable in mock"}"#.to_string(),
            );
        }
        ("404 Not Found", r#"{"message":"not found"}"#.to_string())
    }

    fn mcp_publish_function_mock_response(line: &str, body: &str) -> (&'static str, String) {
        if line.starts_with("GET /api/v1/assets?") {
            return ("200 OK", r#"{"data":{"items":[]}}"#.to_string());
        }
        if line.starts_with("POST /api/v1/assets HTTP/1.1") {
            return mcp_asset_create_mock_response(line, body);
        }
        if line.starts_with("POST /api/v1/assets/mcp-asset-1/repository/files ") {
            return ("200 OK", r#"{"data":{"ok":true}}"#.to_string());
        }
        if line.starts_with("PUT /api/v1/assets/mcp-asset-1/runtime-binding ") {
            return ("200 OK", r#"{"data":{"configured":true}}"#.to_string());
        }
        if line.starts_with("POST /api/v1/assets/mcp-asset-1/runtime-binding/validate ") {
            return ("200 OK", r#"{"data":{"valid":true}}"#.to_string());
        }
        if line.starts_with("POST /api/v1/kernel/capabilities ") {
            return (
                "404 Not Found",
                r#"{"code":404,"message":"capabilities unavailable in mock"}"#.to_string(),
            );
        }
        if line.starts_with("POST /api/v1/functions/mcp-weather-tools/invoke ") {
            return (
                "200 OK",
                r#"{"data":{"viewUrl":"/admin/infrastructure/batch?asset=mcp-asset-1&debug=1&embed=1"}}"#.to_string(),
            );
        }
        if line.starts_with("POST /api/v1/functions/mcp-weather-tools/batch ") {
            return (
                "200 OK",
                r#"{"data":{"viewUrl":"/admin/infrastructure/batch?asset=mcp-asset-1&batch=1&embed=1"}}"#.to_string(),
            );
        }
        ("404 Not Found", r#"{"message":"not found"}"#.to_string())
    }

    fn test_os_session(origin: String) -> crate::a3s_os::StoredOsSession {
        crate::a3s_os::StoredOsSession {
            address: origin,
            access_token: "token".into(),
            refresh_token: None,
            token_type: Some("Bearer".into()),
            expires_at_ms: None,
            account_label: Some("test-os".into()),
            login_at_ms: 1,
        }
    }

    fn write_mcp_fixture(name: &str) -> (std::path::PathBuf, McpDevSession) {
        let root = temp_root(name);
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("weather-tools");
        std::fs::create_dir_all(project.join(".a3s")).unwrap();
        std::fs::write(
            project.join(".a3s/mcp.server.json"),
            r#"{
              "name": "weather-tools",
              "description": "Weather MCP tools",
              "transport": "stdio",
              "entrypoint": "server.js",
              "tools": [{"name":"forecast"},{"name":"current"}]
            }"#,
        )
        .unwrap();
        std::fs::write(project.join("server.js"), "console.log('weather tools');\n").unwrap();
        let dev = McpDevSession {
            name: "weather-tools".into(),
            description: "Weather MCP tools".into(),
            rel: "weather-tools".into(),
            path: project,
            root: root.clone(),
        };
        (root, dev)
    }

    #[tokio::test]
    async fn mcp_status_checks_existing_asset_without_mutating_it() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_mcp_os_mock(captured.clone(), mcp_existing_asset_mock_response).await;
        let (root, dev) = write_mcp_fixture("mcp-status-os");

        let result = publish_mcp_to_os(test_os_session(origin), dev, McpOsAction::Status)
            .await
            .expect("status should inspect existing MCP asset");
        let requests = captured.lock().unwrap().clone();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(result.action, McpOsAction::Status);
        assert_eq!(result.asset_name, "mcp-weather-tools");
        assert_eq!(result.asset_id, "mcp-asset-1");
        assert!(!result.open_view);
        assert!(result.note.contains("asset exists"), "{}", result.note);
        assert!(
            result.note.contains("runtime-binding valid"),
            "{}",
            result.note
        );
        assert!(result.view.url.contains("/admin/assets/mcp-asset-1"));
        assert_eq!(request_count(&requests, "GET /api/v1/assets?"), 1);
        assert_no_mcp_asset_mutation(&requests);
    }

    #[tokio::test]
    async fn mcp_open_observes_existing_asset_without_mutating_it() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_mcp_os_mock(captured.clone(), mcp_existing_asset_mock_response).await;
        let (root, dev) = write_mcp_fixture("mcp-open-os");

        let result = publish_mcp_to_os(test_os_session(origin), dev, McpOsAction::Open)
            .await
            .expect("open should inspect existing MCP asset");
        let requests = captured.lock().unwrap().clone();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(result.action, McpOsAction::Open);
        assert!(result.open_view);
        assert!(result.view.url.contains("/admin/assets/mcp-asset-1"));
        assert!(result.view.url.contains("embed=1"));
        assert!(
            result.note.contains("Function as a Service MCP asset view"),
            "{}",
            result.note
        );
        assert_eq!(
            request_count(&requests, "POST /api/v1/kernel/capabilities "),
            1,
            "/mcp open should try the progressive ViewLink path before fallback"
        );
        assert_no_mcp_asset_mutation(&requests);
    }

    #[tokio::test]
    async fn mcp_logs_observes_existing_asset_without_mutating_it() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_mcp_os_mock(captured.clone(), mcp_existing_asset_mock_response).await;
        let (root, dev) = write_mcp_fixture("mcp-logs-os");

        let result = publish_mcp_to_os(test_os_session(origin), dev, McpOsAction::Logs)
            .await
            .expect("logs should inspect existing MCP asset");
        let requests = captured.lock().unwrap().clone();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(result.action, McpOsAction::Logs);
        assert!(result.open_view);
        assert!(result.view.url.contains("/admin/infrastructure/batch"));
        assert!(result.view.url.contains("asset=mcp-asset-1"));
        assert!(result.view.url.contains("logs=1"));
        assert!(
            result.note.contains("Function as a Service MCP logs view"),
            "{}",
            result.note
        );
        assert_eq!(
            request_count(&requests, "POST /api/v1/kernel/capabilities "),
            1,
            "/mcp logs should try the progressive ViewLink path before fallback"
        );
        assert_no_mcp_asset_mutation(&requests);
    }

    #[tokio::test]
    async fn publish_mcp_to_os_debug_invokes_function_service() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_mcp_os_mock(captured.clone(), mcp_publish_function_mock_response).await;
        let (root, dev) = write_mcp_fixture("mcp-debug-os");

        let result = publish_mcp_to_os(test_os_session(origin.clone()), dev, McpOsAction::Debug)
            .await
            .expect("debug should publish and invoke MCP function");
        let requests = captured.lock().unwrap().clone();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(result.action, McpOsAction::Debug);
        assert_eq!(result.asset_id, "mcp-asset-1");
        assert!(result.open_view);
        assert!(result.view.url.starts_with(&origin), "{}", result.view.url);
        assert!(result.view.url.contains("debug=1"), "{}", result.view.url);
        assert!(
            result.note.contains("MCP debug invoke")
                && result.note.contains("runtime binding was synced"),
            "{}",
            result.note
        );

        assert!(has_request(&requests, "POST /api/v1/assets HTTP/1.1"));
        assert!(has_request(
            &requests,
            "POST /api/v1/assets/mcp-asset-1/repository/files "
        ));
        assert!(has_request(
            &requests,
            "PUT /api/v1/assets/mcp-asset-1/runtime-binding "
        ));
        assert!(has_request(
            &requests,
            "POST /api/v1/functions/mcp-weather-tools/invoke "
        ));
        assert!(!has_request(
            &requests,
            "POST /api/v1/functions/mcp-weather-tools/batch "
        ));

        let upload = request_body(
            &requests,
            "POST /api/v1/assets/mcp-asset-1/repository/files ",
        );
        let upload_json: serde_json::Value = serde_json::from_str(&upload).unwrap();
        let uploaded_paths = upload_json["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|file| file["path"].as_str())
            .collect::<Vec<_>>();
        assert!(uploaded_paths.contains(&"server.js"), "{uploaded_paths:?}");
        assert!(
            uploaded_paths.contains(&MCP_SERVER_CONFIG_PATH),
            "{uploaded_paths:?}"
        );
        assert!(
            uploaded_paths.contains(&MCP_RUNTIME_BINDING_PATH),
            "{uploaded_paths:?}"
        );

        let invoke = request_body(
            &requests,
            "POST /api/v1/functions/mcp-weather-tools/invoke ",
        );
        let invoke_json: serde_json::Value = serde_json::from_str(&invoke).unwrap();
        assert_eq!(invoke_json["agentKind"], "tool");
        assert_eq!(invoke_json["config"]["category"], "mcp");
        assert_eq!(invoke_json["config"]["protocol"], "mcp");
        assert_eq!(invoke_json["input"]["mode"], "debug");
        assert_eq!(invoke_json["input"]["protocol"], "mcp");
        assert_eq!(invoke_json["input"]["assetId"], "mcp-asset-1");
        assert_eq!(invoke_json["input"]["assetName"], "mcp-weather-tools");
        assert_eq!(
            invoke_json["input"]["serverConfig"]["entrypoint"],
            "server.js"
        );
        assert_eq!(invoke_json["input"]["tool"], "current");
    }

    #[tokio::test]
    async fn publish_mcp_to_os_test_batches_function_service() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_mcp_os_mock(captured.clone(), mcp_publish_function_mock_response).await;
        let (root, dev) = write_mcp_fixture("mcp-test-os");

        let result = publish_mcp_to_os(test_os_session(origin.clone()), dev, McpOsAction::Test)
            .await
            .expect("test should publish and batch MCP function inputs");
        let requests = captured.lock().unwrap().clone();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(result.action, McpOsAction::Test);
        assert_eq!(result.asset_id, "mcp-asset-1");
        assert!(result.open_view);
        assert!(result.view.url.starts_with(&origin), "{}", result.view.url);
        assert!(result.view.url.contains("batch=1"), "{}", result.view.url);
        assert!(
            result.note.contains("MCP batch test")
                && result.note.contains("runtime binding was synced"),
            "{}",
            result.note
        );

        assert!(has_request(&requests, "POST /api/v1/assets HTTP/1.1"));
        assert!(has_request(
            &requests,
            "POST /api/v1/assets/mcp-asset-1/repository/files "
        ));
        assert!(has_request(
            &requests,
            "PUT /api/v1/assets/mcp-asset-1/runtime-binding "
        ));
        assert!(has_request(
            &requests,
            "POST /api/v1/functions/mcp-weather-tools/batch "
        ));
        assert!(!has_request(
            &requests,
            "POST /api/v1/functions/mcp-weather-tools/invoke "
        ));

        let batch = request_body(&requests, "POST /api/v1/functions/mcp-weather-tools/batch ");
        let batch_json: serde_json::Value = serde_json::from_str(&batch).unwrap();
        assert_eq!(batch_json["agentKind"], "tool");
        assert_eq!(batch_json["config"]["category"], "mcp");
        assert_eq!(batch_json["config"]["protocol"], "mcp");
        let inputs = batch_json["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 2);
        let modes = inputs
            .iter()
            .map(|input| input["mode"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(modes, vec!["test", "test"]);
        let tools = inputs
            .iter()
            .map(|input| input["tool"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(tools, vec!["current", "forecast"]);
        assert!(inputs.iter().all(|input| {
            input["assetId"] == "mcp-asset-1"
                && input["assetName"] == "mcp-weather-tools"
                && input["protocol"] == "mcp"
        }));
    }

    #[test]
    fn lists_mcp_projects_by_marker_and_metadata() {
        let root = temp_root("mcp-list");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("calc/.a3s")).unwrap();
        std::fs::create_dir_all(root.join("nested/search")).unwrap();
        std::fs::create_dir_all(root.join(".hidden/skip")).unwrap();
        std::fs::write(
            root.join("calc/.a3s/mcp.server.json"),
            r#"{"name":"calc-tools","description":"Calculator tools"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("nested/search/package.json"),
            r#"{"name":"search-tools","description":"Search tools"}"#,
        )
        .unwrap();
        std::fs::write(root.join(".hidden/skip/mcp.json"), "{}").unwrap();

        let projects = list_mcp_projects(&root);
        let rels = projects.iter().map(|p| p.rel.as_str()).collect::<Vec<_>>();
        assert_eq!(rels, vec!["calc", "nested/search"]);
        assert_eq!(projects[0].name, "calc-tools");
        assert_eq!(projects[0].description, "Calculator tools");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mcp_picker_lines_use_bounded_shared_menu_rows() {
        let root = std::path::PathBuf::from(
            "/Users/example/.a3s/mcps/a/path/that/is/far/too/long/for/a/picker/header",
        );
        let projects = vec![
            McpProject {
                rel: "nested/very-long-mcp-server-name-that-would-overflow-the-panel".into(),
                path: root.join("nested/very-long-mcp-server-name-that-would-overflow-the-panel"),
                name: "long-mcp".into(),
                description: "A long MCP server description that should be trimmed cleanly".into(),
            },
            McpProject {
                rel: "weather-tools".into(),
                path: root.join("weather-tools"),
                name: "weather-tools".into(),
                description: "Weather tools".into(),
            },
        ];
        let lines = mcp_picker_lines(&projects, 0, &root, 40, 20);
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(plain.contains("mcp"), "{plain}");
        assert!(plain.contains("select a server asset"), "{plain}");
        assert!(plain.contains("very-long-mcp-server"), "{plain}");
        assert!(plain.contains('…'), "{plain}");
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 40),
            "{plain}"
        );
    }

    #[test]
    fn mcp_picker_lines_scroll_selected_project_into_view() {
        let root = std::path::PathBuf::from("/tmp/mcps");
        let projects = (0..16)
            .map(|index| McpProject {
                rel: format!("mcp-{index}"),
                path: root.join(format!("mcp-{index}")),
                name: format!("mcp-{index}"),
                description: format!("MCP server {index}"),
            })
            .collect::<Vec<_>>();
        let plain = mcp_picker_lines(&projects, 14, &root, 48, 16)
            .into_iter()
            .map(|line| a3s_tui::style::strip_ansi(&line))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(plain.contains("mcp-14"), "{plain}");
        assert!(plain.contains("↑↓ 15/16"), "{plain}");
    }

    #[test]
    fn mcp_picker_header_and_hint_fit_fixed_width() {
        let root = std::path::PathBuf::from(
            "/Users/example/.a3s/mcps/a/path/that/is/far/too/long/for/a/picker/header",
        );
        let header = mcp_picker_header(9, &root, 40);
        let hint = mcp_picker_hint(40);
        assert!(a3s_tui::style::visible_len(&header) <= 40, "{header}");
        assert!(a3s_tui::style::visible_len(&hint) <= 40, "{hint}");
    }

    #[test]
    fn mcp_picker_mouse_wheel_moves_selection_at_overlay_offset() {
        use a3s_tui::event::MouseEventKind;

        let root = std::path::PathBuf::from("/tmp/mcps");
        let projects = (0..4)
            .map(|index| McpProject {
                rel: format!("mcp-{index}"),
                path: root.join(format!("mcp-{index}")),
                name: format!("mcp-{index}"),
                description: format!("MCP server {index}"),
            })
            .collect::<Vec<_>>();
        let width = 48;
        let height = 18;
        let row_count = mcp_picker_lines(&projects, 0, &root, width, height).len();
        let y_offset = mcp_overlay_y_offset(height, row_count);
        let (mut panel, _) = mcp_picker_panel(&projects, 0, &root, width, height).expect("panel");
        panel.set_y_offset(y_offset);

        let msg = panel.handle_mouse(&MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: y_offset + 2,
            modifiers: a3s_tui::KeyModifiers::NONE,
        });

        assert_eq!(msg, None);
        assert_eq!(panel.selected_index(), 1);
    }

    #[test]
    fn mcp_picker_click_selects_visible_row_at_overlay_offset() {
        use a3s_tui::event::{MouseButton, MouseEventKind};

        let root = std::path::PathBuf::from("/tmp/mcps");
        let projects = (0..4)
            .map(|index| McpProject {
                rel: format!("mcp-{index}"),
                path: root.join(format!("mcp-{index}")),
                name: format!("mcp-{index}"),
                description: format!("MCP server {index}"),
            })
            .collect::<Vec<_>>();
        let width = 48;
        let height = 18;
        let row_count = mcp_picker_lines(&projects, 0, &root, width, height).len();
        let y_offset = mcp_overlay_y_offset(height, row_count);
        let (mut panel, _) = mcp_picker_panel(&projects, 0, &root, width, height).expect("panel");
        panel.set_y_offset(y_offset);

        let msg = panel.handle_mouse(&MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: y_offset + 3,
            modifiers: a3s_tui::KeyModifiers::NONE,
        });

        assert_eq!(msg, Some(MenuPanelMsg::Selected(1)));
    }

    #[test]
    fn parses_mcp_exit_subcommand_only() {
        assert_eq!(
            parse_mcp_subcommand("off").unwrap().unwrap(),
            McpSubcommand::Exit
        );
        assert!(parse_mcp_subcommand("off now").unwrap().is_err());
        assert!(parse_mcp_subcommand("weather server").is_none());
    }

    #[test]
    fn parses_mcp_lifecycle_subcommands() {
        let cases = [
            ("publish", McpSubcommand::Publish, McpOsAction::Publish),
            ("deploy", McpSubcommand::Deploy, McpOsAction::Deploy),
            ("debug", McpSubcommand::Debug, McpOsAction::Debug),
            ("test", McpSubcommand::Test, McpOsAction::Test),
            ("open", McpSubcommand::Open, McpOsAction::Open),
            ("logs", McpSubcommand::Logs, McpOsAction::Logs),
            ("status", McpSubcommand::Status, McpOsAction::Status),
        ];
        for (input, expected, action) in cases {
            let parsed = parse_mcp_subcommand(input).unwrap().unwrap();
            assert_eq!(parsed, expected, "{input}");
            assert_eq!(parsed.os_action(), Some(action), "{input}");
        }
        assert_eq!(
            parse_mcp_subcommand("activity failed invocations")
                .unwrap()
                .unwrap(),
            McpSubcommand::Activity("failed invocations".into())
        );
        assert!(parse_mcp_subcommand("ps").unwrap().is_err());
        assert!(parse_mcp_subcommand("run").unwrap().is_err());
        assert!(parse_mcp_subcommand("batch").unwrap().is_err());
        assert!(parse_mcp_subcommand("inspect").unwrap().is_err());
        assert!(parse_mcp_subcommand("jobs").unwrap().is_err());
        assert!(parse_mcp_subcommand("debug weather").unwrap().is_err());
        assert!(parse_mcp_subcommand("publish now").unwrap().is_err());
        for removed in ["view", "remote", "os", "dashboard"] {
            assert!(
                parse_mcp_subcommand(removed).unwrap().is_err(),
                "/mcp {removed} should not create an MCP prototype"
            );
        }
        assert!(parse_mcp_subcommand("weather server").is_none());
    }

    #[test]
    fn mcp_gen_prompt_carries_faas_contract_and_dir() {
        let prompt = mcp_gen_prompt("weather tools", "/Users/x/.a3s/mcps");
        assert!(prompt.contains("/Users/x/.a3s/mcps"));
        assert!(prompt.contains(".a3s/mcp.asset.json"));
        assert!(prompt.contains(".a3s/mcp.server.json"));
        assert!(prompt.contains(".a3s/mcp.runtime-binding.json"));
        assert!(prompt.contains("Function as a Service"));
        assert!(prompt.contains("runtimeIntent.kind=mcp"));
        assert!(prompt.contains("runtime.kind=a3s-function-service"));
        assert!(prompt.contains("protocol=mcp"));
        assert!(prompt.contains("agentKind=tool"));
    }

    #[test]
    fn mcp_dev_prompt_keeps_work_local_and_names_exit_path() {
        let session = McpDevSession {
            name: "weather-tools".into(),
            description: "Weather MCP tools".into(),
            rel: "weather-tools".into(),
            path: std::path::PathBuf::from("/Users/x/.a3s/mcps/weather-tools"),
            root: std::path::PathBuf::from("/Users/x/.a3s/mcps"),
        };
        let prompt = mcp_dev_prompt(&session, "add a forecast tool");
        assert!(prompt.contains("local MCP-development mode"));
        assert!(prompt.contains("Function as a Service"));
        assert!(prompt.contains("/mcp off") && prompt.contains("Esc"));
    }

    #[test]
    fn mcp_progressive_score_prefers_faas_observe_viewlinks() {
        let asset = McpAssetRef {
            id: "asset-1".into(),
            name: "mcp-weather-tools".into(),
        };
        let params = mcp_observe_progressive_params(&asset, "weather-tools", McpOsAction::Open);
        assert_eq!(params["assetId"], "asset-1");
        assert_eq!(params["operation"], "open");
        assert_eq!(params["input"]["mcpName"], "weather-tools");

        let value = serde_json::json!({
            "data": {
                "items": [
                    {
                        "module": "functions",
                        "operation": "FunctionController_getAsset",
                        "description": "Function as a Service MCP asset metadata"
                    },
                    {
                        "module": "functions",
                        "operation": "McpFunctionController_openView",
                        "description": "Function as a Service MCP RemoteUI ViewLink open"
                    },
                    {
                        "module": "agents",
                        "operation": "AgentController_open",
                        "description": "Agent as a Service RemoteUI ViewLink"
                    }
                ]
            }
        });
        let candidates = os_progressive::operation_candidates(&value, |text, operation| {
            mcp_progressive_score(McpOsAction::Open, text, operation)
        });

        assert_eq!(candidates[0].operation, "McpFunctionController_openView");
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.operation != "FunctionController_getAsset"),
            "asset metadata without an open/view hint should not drive /mcp open: {candidates:?}"
        );
        assert_eq!(
            mcp_progressive_score(
                McpOsAction::Logs,
                "Function as a Service MCP runtime logs shaped ViewLink",
                "McpFunctionController_logs"
            )
            .cmp(&0),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn mcp_asset_payloads_keep_function_as_a_service_contract() {
        let session = McpDevSession {
            name: "weather-tools".into(),
            description: "Weather MCP tools".into(),
            rel: "weather-tools".into(),
            path: std::path::PathBuf::from("/Users/x/.a3s/mcps/weather-tools"),
            root: std::path::PathBuf::from("/Users/x/.a3s/mcps"),
        };
        let raw = serde_json::json!({
            "name": "weather-tools",
            "description": "Weather MCP tools",
            "transport": "stdio",
            "entrypoint": "server.js",
            "tools": [
                { "name": "forecast" },
                { "name": "current" }
            ]
        });
        let config = mcp_server_config_json(&session, &raw);
        let asset_name = mcp_asset_name(&mcp_config_name(&config, &session));
        let manifest = mcp_manifest_json(&session, &asset_name, &config);
        let binding = mcp_runtime_binding_json(&session, &asset_name, &config);
        let upsert = mcp_runtime_binding_upsert_body(&binding);

        assert_eq!(asset_name, "mcp-weather-tools");
        assert_eq!(
            config["runtimeIntent"]["runtimeBinding"]["runtimeKind"],
            "a3s-function-service"
        );
        assert_eq!(config["runtimeIntent"]["runtimeBinding"]["protocol"], "mcp");
        assert_eq!(manifest["category"], "mcp");
        assert_eq!(manifest["service"], "Function as a Service");
        assert_eq!(manifest["runtimeIntent"]["kind"], "mcp");
        assert_eq!(manifest["runtimeIntent"]["isolation"], "serving");
        assert_eq!(manifest["runtimeIntent"]["agentKind"], "tool");
        assert_eq!(
            manifest["runtimeIntent"]["runtimeKind"],
            "a3s-function-service"
        );
        assert_eq!(manifest["runtimeIntent"]["protocol"], "mcp");
        assert_eq!(manifest["serverConfigPath"], MCP_SERVER_CONFIG_PATH);
        assert_eq!(binding["kind"], "mcp");
        assert_eq!(binding["isolation"], "serving");
        assert_eq!(binding["runtime"]["kind"], "a3s-function-service");
        assert_eq!(binding["runtime"]["protocol"], "mcp");
        assert_eq!(binding["runtime"]["agentKind"], "tool");
        assert_eq!(binding["metadata"]["tools"][0], "current");
        assert_eq!(upsert["kind"], "mcp");
        assert_eq!(upsert["runtime"]["agentKind"], "tool");
        assert_eq!(upsert["target"]["configPath"], MCP_SERVER_CONFIG_PATH);
    }

    #[test]
    fn mcp_source_upload_skips_generated_and_heavy_local_files() {
        let root = temp_root("mcp-source");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".a3s")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("src/server.js"), "console.log('ok')").unwrap();
        std::fs::write(root.join(".env"), "SECRET=1").unwrap();
        std::fs::write(root.join(".a3s/mcp.asset.json"), "{}").unwrap();
        std::fs::write(root.join(".a3s/mcp.server.json"), "{}").unwrap();
        std::fs::write(root.join(".a3s/mcp.runtime-binding.json"), "{}").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "ignored").unwrap();

        let files = collect_mcp_source_files(&root).unwrap();
        let paths = files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["src/server.js"]);

        let _ = std::fs::remove_dir_all(&root);
    }
}
