use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_boot::{
    AxumAdapter, BootApplication, BootError, ControllerDefinition, HttpAdapter, Module, ModuleRef,
    ProviderDefinition, Result as BootResult,
};
use a3s_code_core::{Agent, AgentSession, CodeConfig, SessionOptions, TokenUsage};
use axum::body::Body;
use axum::http::{
    header::{CACHE_CONTROL, CONTENT_TYPE},
    HeaderValue, StatusCode, Uri,
};
use axum::response::Response;
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::config;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 29653;
const BODY_LIMIT_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn usage_text() -> String {
    [
        "a3s code serve".to_string(),
        String::new(),
        "usage:".to_string(),
        "  a3s code serve [--host 127.0.0.1] [--port 29653]".to_string(),
        "                 [--workspace <path>] [--web-dir <path>] [--api-only]".to_string(),
        String::new(),
        "Starts the local Boot-backed A3S Code API and serves the Shu Xiao'an web UI.".to_string(),
    ]
    .join("\n")
        + "\n"
}

pub(crate) async fn run(args: &[String]) -> anyhow::Result<()> {
    let options = ServeOptions::parse(args)?;
    if options.help {
        print!("{}", usage_text());
        return Ok(());
    }

    let config_path = ensure_config_path()?;
    let code_config = CodeConfig::from_file(Path::new(&config_path))
        .map_err(|e| anyhow::anyhow!("failed to parse {config_path}: {e}"))?;
    let agent = Arc::new(
        Agent::new(config_path.clone())
            .await
            .map_err(|e| anyhow::anyhow!("failed to load A3S Code from {config_path}: {e}"))?,
    );
    let state = Arc::new(CodeWebState::new(
        agent,
        PathBuf::from(&config_path),
        options.workspace.clone(),
        code_config.default_model.clone(),
    ));

    let app = BootApplication::builder()
        .import(CodeWebModule {
            state: Arc::clone(&state),
        })
        .build()
        .map_err(boot_to_anyhow)?;

    app.bootstrap().await.map_err(boot_to_anyhow)?;
    let api_router = AxumAdapter::new()
        .with_body_limit(BODY_LIMIT_BYTES)
        .build(app.clone())
        .map_err(boot_to_anyhow)?;

    let router = if options.api_only {
        Router::new()
            .nest("/api", api_router)
            .fallback(api_only_fallback)
    } else {
        let web_dir = options
            .web_dir
            .clone()
            .or_else(find_default_web_dir)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "web assets were not found; run `npm --prefix apps/web run build` or pass --web-dir"
                )
            })?;
        if !web_dir.join("index.html").is_file() {
            anyhow::bail!(
                "web assets at {} do not contain index.html; run `npm --prefix apps/web run build`",
                web_dir.display()
            );
        }
        let web_root = Arc::new(web_dir);
        Router::new().nest("/api", api_router).fallback({
            let web_root = Arc::clone(&web_root);
            move |uri: Uri| serve_static(uri, Arc::clone(&web_root))
        })
    };

    let listener = tokio::net::TcpListener::bind(options.addr)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind {}: {e}", options.addr))?;
    let actual_addr = listener.local_addr()?;
    println!("A3S Code API:  http://{actual_addr}/api/health");
    if options.api_only {
        println!("A3S Code Web:  disabled (--api-only)");
    } else {
        println!("A3S Code Web:  http://{actual_addr}/");
    }
    println!("Workspace:     {}", options.workspace.display());
    println!("Config:        {config_path}");
    println!("Press Ctrl+C to stop.");

    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| anyhow::anyhow!("server failed: {e}"));
    let shutdown_result = app.shutdown().await.map_err(boot_to_anyhow);

    match (serve_result, shutdown_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[derive(Debug)]
struct ServeOptions {
    addr: SocketAddr,
    workspace: PathBuf,
    web_dir: Option<PathBuf>,
    api_only: bool,
    help: bool,
}

impl ServeOptions {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut host = std::env::var("A3S_CODE_WEB_HOST").unwrap_or_else(|_| DEFAULT_HOST.into());
        let mut port = std::env::var("A3S_CODE_WEB_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);
        let mut workspace = std::env::current_dir()?;
        let mut web_dir = std::env::var_os("A3S_CODE_WEB_DIR").map(PathBuf::from);
        let mut api_only = false;
        let mut help = false;

        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "-h" | "--help" | "help" => {
                    help = true;
                    index += 1;
                }
                "--host" => {
                    host = take_value(args, &mut index, "--host")?;
                }
                "--port" => {
                    let value = take_value(args, &mut index, "--port")?;
                    port = value
                        .parse::<u16>()
                        .map_err(|_| anyhow::anyhow!("--port must be a number from 0 to 65535"))?;
                }
                "--workspace" | "-w" => {
                    workspace = PathBuf::from(take_value(args, &mut index, "--workspace")?);
                }
                "--web-dir" => {
                    web_dir = Some(PathBuf::from(take_value(args, &mut index, "--web-dir")?));
                }
                "--api-only" => {
                    api_only = true;
                    index += 1;
                }
                other => anyhow::bail!("unknown a3s code serve option `{other}`"),
            }
        }

        let addr = resolve_addr(&host, port)?;
        Ok(Self {
            addr,
            workspace,
            web_dir,
            api_only,
            help,
        })
    }
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> anyhow::Result<String> {
    let value_index = *index + 1;
    let value = args
        .get(value_index)
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))?;
    *index += 2;
    Ok(value.clone())
}

fn resolve_addr(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    let mut addrs = format!("{host}:{port}")
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("invalid host/port {host}:{port}: {e}"))?;
    addrs
        .next()
        .ok_or_else(|| anyhow::anyhow!("could not resolve {host}:{port}"))
}

fn ensure_config_path() -> anyhow::Result<String> {
    if let Some(path) = config::find_config() {
        return Ok(path);
    }

    let path = config::default_config_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    config::write_template_config(&path)?;
    anyhow::bail!(
        "created starter config at {}; fill in a provider/model, then rerun `a3s code serve`",
        path.display()
    );
}

fn find_default_web_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.extend(upward_candidates(&cwd));
        candidates.push(cwd.join("dist/workspace"));
        candidates.push(cwd.join("dist"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist/workspace"));
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist"));

    candidates
        .into_iter()
        .map(clean_path)
        .find(|candidate| candidate.join("index.html").is_file())
}

fn upward_candidates(start: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut current = Some(start);
    while let Some(dir) = current {
        candidates.push(dir.join("apps/web/dist/workspace"));
        candidates.push(dir.join("apps/web/dist"));
        current = dir.parent();
    }
    candidates
}

fn clean_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn boot_to_anyhow(error: BootError) -> anyhow::Error {
    anyhow::anyhow!("{error}")
}

struct CodeWebState {
    agent: Arc<Agent>,
    config_path: PathBuf,
    default_workspace: PathBuf,
    default_model: Option<String>,
    sessions: Mutex<HashMap<String, Arc<AgentSession>>>,
}

impl CodeWebState {
    fn new(
        agent: Arc<Agent>,
        config_path: PathBuf,
        default_workspace: PathBuf,
        default_model: Option<String>,
    ) -> Self {
        Self {
            agent,
            config_path,
            default_workspace,
            default_model,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn health(&self) -> HealthResponse {
        HealthResponse {
            ok: true,
            app: "书小安".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            config_path: self.config_path.display().to_string(),
            workspace: self.default_workspace.display().to_string(),
            model: self.default_model.clone(),
        }
    }

    async fn create_session(&self, request: CreateSessionRequest) -> BootResult<SessionResponse> {
        let session = self.create_or_get_session(None, request).await?;
        Ok(SessionResponse::from_session(
            &session,
            self.default_model.clone(),
            None,
            None,
        ))
    }

    async fn create_kernel_session(
        &self,
        request: CreateSessionRequest,
    ) -> BootResult<KernelSessionResponse> {
        let title = request.title.clone();
        let agent_id = request.agent_id.clone();
        let session = self.create_or_get_session(None, request).await?;
        Ok(KernelSessionResponse {
            success: true,
            session: SessionResponse::from_session(
                &session,
                self.default_model.clone(),
                title,
                agent_id,
            ),
        })
    }

    async fn list_kernel_sessions(&self) -> BootResult<SessionListResponse> {
        let sessions: Vec<SessionResponse> = self
            .sessions
            .lock()
            .await
            .values()
            .map(|session| {
                SessionResponse::from_session(
                    session.as_ref(),
                    self.default_model.clone(),
                    None,
                    None,
                )
            })
            .collect();
        Ok(SessionListResponse {
            total: sessions.len(),
            items: sessions,
        })
    }

    async fn chat(&self, request: ChatRequest) -> BootResult<ChatResponse> {
        let message = request.message.trim().to_string();
        if message.is_empty() {
            return Err(BootError::BadRequest("message cannot be empty".to_string()));
        }

        let session_request = CreateSessionRequest {
            workspace: request.workspace,
            cwd: None,
            model: request.model,
            title: None,
            agent_id: None,
        };
        let session = self
            .create_or_get_session(request.session_id, session_request)
            .await?;
        let result = session
            .send(&message, None)
            .await
            .map_err(|e| BootError::Internal(e.to_string()))?;

        Ok(ChatResponse {
            session_id: session.session_id().to_string(),
            workspace: session.workspace().display().to_string(),
            model: self.default_model.clone(),
            text: result.text,
            usage: UsageResponse::from_usage(result.usage),
            tool_calls_count: result.tool_calls_count,
        })
    }

    async fn create_or_get_session(
        &self,
        requested_id: Option<String>,
        request: CreateSessionRequest,
    ) -> BootResult<Arc<AgentSession>> {
        if let Some(id) = requested_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            if let Some(session) = self.sessions.lock().await.get(id).cloned() {
                return Ok(session);
            }
        }

        let workspace = request
            .workspace
            .as_deref()
            .or(request.cwd.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.default_workspace.clone());
        let mut options = SessionOptions::new()
            .with_auto_save(false)
            .with_auto_compact(true)
            .with_file_memory(config::memory_dir());
        if let Some(id) = requested_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            options = options.with_session_id(id.to_string());
        }
        if let Some(model) = request
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            options = options.with_model(model.to_string());
        }

        let session = Arc::new(
            self.agent
                .session(workspace.display().to_string(), Some(options))
                .map_err(|e| BootError::Internal(e.to_string()))?,
        );
        self.sessions
            .lock()
            .await
            .insert(session.session_id().to_string(), Arc::clone(&session));
        Ok(session)
    }

    async fn close(&self) {
        self.agent.close().await;
    }
}

struct CodeWebModule {
    state: Arc<CodeWebState>,
}

impl Module for CodeWebModule {
    fn name(&self) -> &'static str {
        "a3s-code-web"
    }

    fn providers(&self) -> BootResult<Vec<ProviderDefinition>> {
        Ok(vec![ProviderDefinition::from_arc(Arc::clone(&self.state))])
    }

    fn controllers(&self, module_ref: &ModuleRef) -> BootResult<Vec<ControllerDefinition>> {
        let state = module_ref.get::<CodeWebState>()?;
        let health_state = Arc::clone(&state);
        let session_state = Arc::clone(&state);
        let chat_state = Arc::clone(&state);
        let v1_health_state = Arc::clone(&state);
        let kernel_create_state = Arc::clone(&state);
        let kernel_list_state = Arc::clone(&state);

        Ok(vec![ControllerDefinition::new("/")?
            .get_json("/health", move |_| {
                let state = Arc::clone(&health_state);
                async move { Ok(state.health()) }
            })?
            .get_json("/v1/health", move |_| {
                let state = Arc::clone(&v1_health_state);
                async move { Ok(state.health()) }
            })?
            .post_json("/sessions", move |request: CreateSessionRequest| {
                let state = Arc::clone(&session_state);
                async move { state.create_session(request).await }
            })?
            .get_json("/v1/kernel/sessions", move |_| {
                let state = Arc::clone(&kernel_list_state);
                async move { state.list_kernel_sessions().await }
            })?
            .post_json(
                "/v1/kernel/sessions",
                move |request: CreateSessionRequest| {
                    let state = Arc::clone(&kernel_create_state);
                    async move { state.create_kernel_session(request).await }
                },
            )?
            .post_json("/chat", move |request: ChatRequest| {
                let state = Arc::clone(&chat_state);
                async move { state.chat(request).await }
            })?])
    }

    fn on_application_shutdown(
        &self,
        _module_ref: ModuleRef,
    ) -> a3s_boot::BoxFuture<'static, BootResult<()>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            state.close().await;
            Ok(())
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSessionRequest {
    workspace: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    title: Option<String>,
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatRequest {
    session_id: Option<String>,
    workspace: Option<String>,
    model: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    ok: bool,
    app: String,
    version: String,
    config_path: String,
    workspace: String,
    model: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionResponse {
    session_id: String,
    workspace: String,
    cwd: String,
    model: Option<String>,
    state: String,
    title: Option<String>,
    agent_id: Option<String>,
    created_at: i64,
}

impl SessionResponse {
    fn from_session(
        session: &AgentSession,
        model: Option<String>,
        title: Option<String>,
        agent_id: Option<String>,
    ) -> Self {
        let workspace = session.workspace().display().to_string();
        Self {
            session_id: session.session_id().to_string(),
            workspace: workspace.clone(),
            cwd: workspace,
            model,
            state: "connected".to_string(),
            title,
            agent_id: Some(agent_id.unwrap_or_else(|| "default".to_string())),
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct KernelSessionResponse {
    success: bool,
    session: SessionResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionListResponse {
    items: Vec<SessionResponse>,
    total: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatResponse {
    session_id: String,
    workspace: String,
    model: Option<String>,
    text: String,
    usage: UsageResponse,
    tool_calls_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageResponse {
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
}

impl UsageResponse {
    fn from_usage(usage: TokenUsage) -> Self {
        Self {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        }
    }
}

async fn api_only_fallback() -> Response {
    response_with_status(
        StatusCode::NOT_FOUND,
        "text/plain; charset=utf-8",
        "A3S Code API is running. Web assets are disabled.",
    )
}

async fn serve_static(uri: Uri, root: Arc<PathBuf>) -> Response {
    let Some(path) = static_path(&root, uri.path()) else {
        return response_with_status(
            StatusCode::BAD_REQUEST,
            "text/plain; charset=utf-8",
            "invalid static path",
        );
    };

    match tokio::fs::read(&path).await {
        Ok(body) => response_with_headers(StatusCode::OK, content_type_for(&path), body),
        Err(_) => response_with_status(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            "not found",
        ),
    }
}

fn static_path(root: &Path, request_path: &str) -> Option<PathBuf> {
    let trimmed = request_path.trim_start_matches('/');
    let mut candidate = root.to_path_buf();
    if !trimmed.is_empty() {
        for segment in trimmed.split('/') {
            if segment.is_empty() || segment == "." {
                continue;
            }
            if segment == ".." || segment.contains('\\') {
                return None;
            }
            candidate.push(segment);
        }
    }

    if candidate.is_dir() {
        return Some(candidate.join("index.html"));
    }
    if candidate.is_file() {
        return Some(candidate);
    }
    Some(root.join("index.html"))
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "css" => "text/css; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn response_with_status(
    status: StatusCode,
    content_type: &'static str,
    body: &'static str,
) -> Response {
    response_with_headers(status, content_type, body.as_bytes().to_vec())
}

fn response_with_headers(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    response
}
