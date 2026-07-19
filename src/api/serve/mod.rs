use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_boot::{BootApplication, BootError};
use a3s_code_core::{Agent, CodeConfig};
use anyhow::Context;
use axum::routing::{get, post};
use axum::Json;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::config;

use self::api_gateway::ApiGateway;
use self::options::ServeOptions;
use super::code_web::{CodeWebModule, CodeWebSessionRepository, CodeWebState, KernelService};
use super::web::{api_only_fallback, find_default_web_dir, serve_static};

mod api_gateway;
mod background;
mod options;

pub(crate) use background::{
    open as open_instance, read_log_tail, status as instance_status, stop as stop_instance,
    WebEndpoint, WebInstanceRecord, WebInstanceStatus,
};

const API_PREFIX: &str = "/api";
const BODY_LIMIT_BYTES: usize = 2 * 1024 * 1024;

pub(crate) enum ServeOutcome {
    Help,
    ForegroundStopped,
    Detached {
        instance: WebInstanceRecord,
        reused: bool,
    },
    Existing(WebEndpoint),
}

pub(crate) fn usage_text() -> String {
    [
        "a3s web".to_string(),
        String::new(),
        "usage:".to_string(),
        "  a3s web [-d] [--replace] [--host 127.0.0.1] [--port 29653]".to_string(),
        "          [--workspace <path>] [--config <path>] [--web-dir <path>] [--api-only]"
            .to_string(),
        String::new(),
        "Starts the local Boot-backed A3S Code API and serves the Shu Xiao'an web UI.".to_string(),
        "Use -d to start in the background; the command prints its PID, URL, and log path."
            .to_string(),
    ]
    .join("\n")
        + "\n"
}

pub(crate) async fn run(args: &[String]) -> anyhow::Result<ServeOutcome> {
    let options = ServeOptions::parse(args)?;
    if options.help {
        print!("{}", usage_text());
        return Ok(ServeOutcome::Help);
    }
    if options.background {
        return Ok(match background::start(args, &options).await? {
            background::BackgroundStart::Started(instance) => ServeOutcome::Detached {
                instance,
                reused: false,
            },
            background::BackgroundStart::Reused(instance) => ServeOutcome::Detached {
                instance,
                reused: true,
            },
            background::BackgroundStart::Existing(instance) => ServeOutcome::Existing(instance),
        });
    }

    match run_foreground(options).await? {
        Some(instance) => Ok(ServeOutcome::Existing(instance)),
        None => Ok(ServeOutcome::ForegroundStopped),
    }
}

async fn run_foreground(options: ServeOptions) -> anyhow::Result<Option<WebEndpoint>> {
    if options.replace {
        background::replace_managed(&options.workspace).await?;
    }
    let listener = match tokio::net::TcpListener::bind(options.addr).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if let Some(existing) = background::discover_requested_instance(&options).await? {
                if options.replace {
                    anyhow::bail!(
                        "A3S Web {} is healthy but is not managed by this CLI state; no process \
                         was stopped. Stop its original command or managed service before using \
                         --replace",
                        existing.address
                    );
                }
                return Ok(Some(existing));
            }
            anyhow::bail!(
                "{} is already in use by another application; no process was stopped. Stop that \
                 application or select an available port with --port 0",
                options.addr
            );
        }
        Err(error) => {
            return Err(anyhow::anyhow!(
                "failed to bind {} before A3S Web startup: {error}",
                options.addr
            ))
        }
    };
    let actual_addr = listener.local_addr()?;
    let web_root = resolve_web_root(&options)?;
    let config_path = ensure_config_path(&options)?;
    let code_config = CodeConfig::from_file(Path::new(&config_path))
        .map_err(|e| anyhow::anyhow!("failed to parse {config_path}: {e}"))?;
    let agent = Arc::new(
        Agent::new(config_path.clone())
            .await
            .map_err(|e| anyhow::anyhow!("failed to load A3S Code from {config_path}: {e}"))?,
    );
    let session_repository = Arc::new(
        CodeWebSessionRepository::open_default()
            .await
            .context("failed to open A3S Code Web session store")?,
    );
    let state = Arc::new(CodeWebState::new(
        agent,
        PathBuf::from(&config_path),
        options.workspace.clone(),
        code_config,
        session_repository,
    ));
    match a3s::components::ComponentPaths::from_env_at(&options.workspace)
        .and_then(|paths| a3s::components::find_ready_executable_with("use", &paths))
    {
        Ok(Some(executable)) => {
            let (registry, warning) = crate::use_registry::start_detached(
                executable,
                options.workspace.clone(),
                CancellationToken::new(),
            )
            .await;
            if let Some(warning) = warning {
                eprintln!("warning: A3S Use capabilities will continue loading: {warning}");
            }
            state.install_use_registry(registry);
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("warning: A3S Use hot-plug is unavailable for Code Web: {error}");
        }
    }
    let restore_report = KernelService::new(Arc::clone(&state))
        .restore_persisted_sessions()
        .await
        .map_err(boot_to_anyhow)?;

    let app = BootApplication::builder()
        .global_prefix(API_PREFIX)
        .import(CodeWebModule::new(Arc::clone(&state)))
        .build()
        .map_err(boot_to_anyhow)?;

    app.bootstrap().await.map_err(boot_to_anyhow)?;
    let api_router = ApiGateway::new(app.clone(), BODY_LIMIT_BYTES).router();

    let router = if options.api_only {
        api_router.fallback(api_only_fallback)
    } else {
        let web_root = web_root.ok_or_else(|| {
            anyhow::anyhow!("internal error: Web root validation did not produce a directory")
        })?;
        api_router.fallback({
            let web_root = Arc::clone(&web_root);
            move |uri| serve_static(uri, Arc::clone(&web_root))
        })
    };

    let shutdown = Arc::new(Notify::new());
    let instance_nonce = std::env::var(background::INSTANCE_NONCE_ENV).ok();
    let router = if let Some(nonce) = instance_nonce.as_deref() {
        let status_path = format!("/.a3s/web/{nonce}/status");
        let stop_path = format!("/.a3s/web/{nonce}/stop");
        let status_nonce = nonce.to_string();
        let stop_signal = Arc::clone(&shutdown);
        router
            .route(
                &status_path,
                get(move || {
                    let nonce = status_nonce.clone();
                    async move {
                        Json(serde_json::json!({
                            "schemaVersion": 1,
                            "pid": std::process::id(),
                            "nonce": nonce,
                        }))
                    }
                }),
            )
            .route(
                &stop_path,
                post(move || {
                    let signal = Arc::clone(&stop_signal);
                    async move {
                        signal.notify_one();
                        Json(serde_json::json!({"ok": true}))
                    }
                }),
            )
    } else {
        router
    };

    background::notify_ready(actual_addr)?;
    println!("A3S Code API:  http://{actual_addr}/api/health");
    if options.api_only {
        println!("A3S Web:       disabled (--api-only)");
    } else {
        println!("A3S Web:       http://{actual_addr}/");
    }
    println!("Workspace:     {}", options.workspace.display());
    println!("Config:        {config_path}");
    println!("Sessions restored: {}", restore_report.restored);
    if restore_report.unavailable > 0 || restore_report.failed > 0 {
        println!(
            "Sessions unavailable: {}",
            restore_report.unavailable + restore_report.failed
        );
    }
    println!("Press Ctrl+C to stop.");

    let shutdown_signal = Arc::clone(&shutdown);
    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = shutdown_signal.notified() => {}
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("server failed: {e}"));
    let shutdown_result = app.shutdown().await.map_err(boot_to_anyhow);
    if let (Some(path), Some(nonce)) = (
        std::env::var_os(background::INSTANCE_FILE_ENV),
        instance_nonce.as_deref(),
    ) {
        background::remove_instance_if_owned(Path::new(&path), nonce);
    }

    match (serve_result, shutdown_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(None),
    }
}

fn resolve_web_root(options: &ServeOptions) -> anyhow::Result<Option<Arc<PathBuf>>> {
    if options.api_only {
        return Ok(None);
    }
    let web_dir = options
        .web_dir
        .clone()
        .or_else(find_default_web_dir)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "web assets were not found; reinstall A3S, run `npm --prefix apps/web run build`, \
                 or pass --web-dir"
            )
        })?;
    if !web_dir.join("index.html").is_file() {
        anyhow::bail!(
            "web assets at {} do not contain index.html; reinstall A3S, run `npm --prefix apps/web \
             run build`, or pass --web-dir",
            web_dir.display()
        );
    }
    Ok(Some(Arc::new(web_dir)))
}

fn ensure_config_path(options: &ServeOptions) -> anyhow::Result<String> {
    if let Some(path) = options.config_path.as_ref() {
        return Ok(path.to_string_lossy().into_owned());
    }
    for directory in options.workspace.ancestors() {
        let candidate = directory.join(".a3s/config.acl");
        if candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }

    let path = config::default_config_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    config::write_template_config(&path)?;
    anyhow::bail!(
        "created starter config at {}; fill in a provider/model, then rerun `a3s web`",
        path.display()
    );
}

fn boot_to_anyhow(error: BootError) -> anyhow::Error {
    anyhow::anyhow!("{error}")
}
