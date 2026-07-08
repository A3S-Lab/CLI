use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::Arc;

use a3s_code_core::config::{CodeConfig, OsConfig};
use a3s_code_core::{Agent, AgentSession, SessionOptions, ToolCallResult};

use crate::config;

use super::{asset_clone, kbutil, memutil, panels, remote_ui};

const TOP_LEVEL_COMMANDS: &[&str] = &[
    "agent",
    "mcp",
    "skill",
    "flow",
    "okf",
    "deepresearch",
    "deep-research",
    "login",
    "logout",
    "auth",
    "config",
    "dirs",
    "models",
    "model",
    "kb",
    "ctx",
    "memory",
    "mem",
    "top",
    "serve",
];

pub(crate) fn is_code_cli_command(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some("-h" | "--help" | "help")
    ) || args
        .first()
        .is_some_and(|arg| TOP_LEVEL_COMMANDS.contains(&arg.as_str()))
}

pub(crate) fn code_cli_usage_text() -> String {
    [
        "a3s code subcommands".to_string(),
        String::new(),
        "usage:".to_string(),
        "  a3s code                         launch the interactive coding agent (TUI)".to_string(),
        "  a3s code resume <id>             resume a saved TUI session".to_string(),
        "  a3s code update                  check for and install a newer version".to_string(),
        "  a3s code login [token]           sign in to the configured OS account".to_string(),
        "  a3s code logout                  sign out from the configured OS account".to_string(),
        "  a3s code auth status             show configured OS login status".to_string(),
        "  a3s code config path|init|cat    inspect or create config.acl".to_string(),
        "  a3s code dirs                    print local asset and memory roots".to_string(),
        "  a3s code models                  list configured and account-backed models".to_string(),
        "  a3s code serve                   start local API and Shu Xiao'an web UI".to_string(),
        "  a3s code deepresearch <query>    run DeepResearch and write .md/.html report artifacts"
            .to_string(),
        "  a3s code <family> local [query]  list local assets for a family".to_string(),
        "  a3s code <family> clone <url>    clone an asset source into the configured root"
            .to_string(),
        "  a3s code <family> list [query]   list OS digital assets for a family".to_string(),
        "  a3s code <family> activity [q]   list OS runtime activity for a family".to_string(),
        String::new(),
        "families: agent, mcp, skill, flow, okf".to_string(),
        String::new(),
        "lifecycle commands:".to_string(),
        "  a3s code agent publish agentic|application|tool [package]".to_string(),
        "  a3s code agent run|deploy|open|logs|status [kind] [package]".to_string(),
        "  a3s code mcp publish|run|test|deploy|open|logs|status [path]".to_string(),
        "  a3s code skill publish|deploy|open|status [path]".to_string(),
        "  a3s code flow publish|run|deploy|open|logs|status [file]".to_string(),
        "  a3s code okf publish|deploy|status [path]".to_string(),
        String::new(),
        "review prompts:".to_string(),
        "  a3s code <family> review [path]  print the same review prompt the TUI uses".to_string(),
        String::new(),
        "local knowledge and diagnostics:".to_string(),
        "  a3s code kb stats|add|import|search|vault".to_string(),
        "  a3s code ctx search <query>      search local ctx history".to_string(),
        "  a3s code memory list [query]     list long-term memory entries".to_string(),
        "  a3s code top [--json]            alias for a3s top".to_string(),
    ]
    .join("\n")
        + "\n"
}

pub(crate) async fn run_code_cli(args: Vec<String>) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => {
            print!("{}", code_cli_usage_text());
            Ok(())
        }
        Some("agent") => run_agent(&args[1..]).await,
        Some("mcp") => run_mcp(&args[1..]).await,
        Some("skill") => run_skill(&args[1..]).await,
        Some("flow") => run_flow(&args[1..]).await,
        Some("okf") => run_okf(&args[1..]).await,
        Some("login") => run_login(&args[1..]).await,
        Some("logout") => run_logout().await,
        Some("auth") => run_auth(&args[1..]).await,
        Some("config") => run_config(&args[1..]).await,
        Some("dirs") => {
            print_code_dirs()?;
            Ok(())
        }
        Some("models" | "model") => run_models(&args[1..]).await,
        Some("deepresearch" | "deep-research") => run_deepresearch(&args[1..]).await,
        Some("kb") => run_kb(&args[1..]),
        Some("ctx") => run_ctx(&args[1..]).await,
        Some("memory" | "mem") => run_memory(&args[1..]),
        Some("top") => crate::top::run(args[1..].to_vec()).await,
        Some("serve") => crate::api::run(&args[1..]).await,
        Some(other) => anyhow::bail!(
            "unknown a3s code subcommand `{other}`; expected one of {}",
            TOP_LEVEL_COMMANDS.join(", ")
        ),
    }
}

async fn run_agent(args: &[String]) -> anyhow::Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print_family_help("agent");
        return Ok(());
    };
    match command {
        "-h" | "--help" | "help" => {
            print_family_help("agent");
            Ok(())
        }
        "local" | "ls" => {
            ensure_no_more_than_query("agent local", &args[1..])?;
            print_local_agents(&join_args(&args[1..]));
            Ok(())
        }
        "clone" => {
            let url = parse_clone_url("agent", &args[1..])?;
            clone_asset("agent", url, config::agent_dir()).await
        }
        "list" => list_assets("agent", &join_args(&args[1..])).await,
        "activity" => runtime_activity("agent", &join_args(&args[1..])).await,
        "review" => {
            let dev = resolve_agent_dev(single_path_arg("agent review", &args[1..])?)?;
            println!("{}", panels::agent::agent_review_prompt(&dev));
            Ok(())
        }
        "publish" => {
            let (kind, path) = parse_agent_publish_args(&args[1..])?;
            run_agent_os(
                panels::agent::AgentOsAction::Publish(kind),
                path.as_deref(),
                false,
            )
            .await
        }
        "run" => {
            let (kind, path) = parse_agent_kind_path(command, &args[1..])?;
            run_agent_os(
                panels::agent::AgentOsAction::Run(kind),
                path.as_deref(),
                false,
            )
            .await
        }
        "deploy" => {
            let path = single_path_arg("agent deploy", &args[1..])?;
            run_agent_os(panels::agent::AgentOsAction::Deploy, path.as_deref(), false).await
        }
        "open" | "logs" | "status" => {
            let (kind, path) = parse_agent_kind_path(command, &args[1..])?;
            let action = match command {
                "open" => panels::agent::AgentOsAction::Open(kind),
                "logs" => panels::agent::AgentOsAction::Logs(kind),
                "status" => panels::agent::AgentOsAction::Status(kind),
                _ => unreachable!(),
            };
            run_agent_os(action, path.as_deref(), command == "open").await
        }
        other => unknown_family_command("agent", other),
    }
}

async fn run_mcp(args: &[String]) -> anyhow::Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print_family_help("mcp");
        return Ok(());
    };
    match command {
        "-h" | "--help" | "help" => {
            print_family_help("mcp");
            Ok(())
        }
        "local" | "ls" => {
            ensure_no_more_than_query("mcp local", &args[1..])?;
            print_local_mcps(&join_args(&args[1..]));
            Ok(())
        }
        "clone" => {
            let url = parse_clone_url("mcp", &args[1..])?;
            clone_asset("mcp", url, config::mcp_dir()).await
        }
        "list" => list_assets("mcp", &join_args(&args[1..])).await,
        "activity" => runtime_activity("mcp", &join_args(&args[1..])).await,
        "review" => {
            let dev = resolve_mcp_dev(single_path_arg("mcp review", &args[1..])?)?;
            println!("{}", panels::mcp::mcp_review_prompt(&dev));
            Ok(())
        }
        "publish" | "run" | "test" | "deploy" | "open" | "logs" | "status" => {
            let path = single_path_arg(&format!("mcp {command}"), &args[1..])?;
            let action = parse_mcp_action(command)?;
            run_mcp_os(action, path.as_deref(), command == "open").await
        }
        other => unknown_family_command("mcp", other),
    }
}

async fn run_skill(args: &[String]) -> anyhow::Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print_family_help("skill");
        return Ok(());
    };
    match command {
        "-h" | "--help" | "help" => {
            print_family_help("skill");
            Ok(())
        }
        "local" | "ls" => {
            ensure_no_more_than_query("skill local", &args[1..])?;
            print_local_skills(&join_args(&args[1..]));
            Ok(())
        }
        "clone" => {
            let url = parse_clone_url("skill", &args[1..])?;
            clone_asset("skill", url, config::skill_dir()).await
        }
        "list" => list_assets("skill", &join_args(&args[1..])).await,
        "activity" => runtime_activity("skill", &join_args(&args[1..])).await,
        "review" => {
            let dev = resolve_skill_dev(single_path_arg("skill review", &args[1..])?)?;
            let body = std::fs::read_to_string(&dev.path)
                .map_err(|e| anyhow::anyhow!("could not read {}: {e}", dev.path.display()))?;
            println!("{}", panels::skill::skill_review_prompt(&dev.path, &body));
            Ok(())
        }
        "publish" | "deploy" | "open" | "status" => {
            let path = single_path_arg(&format!("skill {command}"), &args[1..])?;
            let action = parse_skill_action(command)?;
            run_skill_os(action, path.as_deref(), command == "open").await
        }
        other => unknown_family_command("skill", other),
    }
}

async fn run_flow(args: &[String]) -> anyhow::Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print_family_help("flow");
        return Ok(());
    };
    match command {
        "-h" | "--help" | "help" => {
            print_family_help("flow");
            Ok(())
        }
        "local" | "ls" => {
            ensure_no_more_than_query("flow local", &args[1..])?;
            print_local_flows(&join_args(&args[1..]));
            Ok(())
        }
        "clone" => {
            let url = parse_clone_url("flow", &args[1..])?;
            clone_asset("workflow", url, config::flow_dir()).await
        }
        "list" => list_assets("workflow", &join_args(&args[1..])).await,
        "activity" => runtime_activity("workflow", &join_args(&args[1..])).await,
        "review" => {
            let flow = resolve_flow_file(single_path_arg("flow review", &args[1..])?)?;
            let design = read_flow_design(&flow.path)?;
            println!("{}", panels::flow::flow_review_prompt(&flow.path, &design));
            Ok(())
        }
        "publish" | "run" | "deploy" | "open" | "logs" | "status" => {
            let path = single_path_arg(&format!("flow {command}"), &args[1..])?;
            let action = parse_flow_action(command)?;
            run_flow_os(action, path.as_deref(), command == "open").await
        }
        other => unknown_family_command("flow", other),
    }
}

async fn run_okf(args: &[String]) -> anyhow::Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print_family_help("okf");
        return Ok(());
    };
    match command {
        "-h" | "--help" | "help" => {
            print_family_help("okf");
            Ok(())
        }
        "local" | "ls" => {
            ensure_no_more_than_query("okf local", &args[1..])?;
            print_local_okf(&join_args(&args[1..]))?;
            Ok(())
        }
        "clone" => {
            let url = parse_clone_url("okf", &args[1..])?;
            let cwd = std::env::current_dir()?;
            clone_asset(
                "okf",
                url,
                panels::okf::okf_package_dir(&cwd.to_string_lossy()),
            )
            .await
        }
        "list" => list_assets("knowledge", &join_args(&args[1..])).await,
        "activity" => runtime_activity("knowledge", &join_args(&args[1..])).await,
        "review" => {
            let dev = resolve_okf_dev(single_path_arg("okf review", &args[1..])?)?;
            println!(
                "{}",
                panels::okf::okf_lifecycle_prompt("review", &dev, load_os_session().await.is_ok())
            );
            Ok(())
        }
        "publish" | "deploy" | "status" => {
            let path = single_path_arg(&format!("okf {command}"), &args[1..])?;
            let action = parse_okf_action(command)?;
            run_okf_os(action, path.as_deref()).await
        }
        other => unknown_family_command("okf", other),
    }
}

async fn run_login(args: &[String]) -> anyhow::Result<()> {
    if matches!(
        args.first().map(String::as_str),
        Some("-h" | "--help" | "help")
    ) {
        println!("a3s code login [token]");
        println!("  no token: open the configured OS OAuth login in your browser");
        println!("  token:    store an existing OS bearer token");
        return Ok(());
    }
    let (_, os_config) = load_os_config()?;
    let token = optional_single_arg("login", args)?;
    let session = match token {
        Some(token) => crate::a3s_os::login_with_token(&os_config, &token)?,
        None => crate::a3s_os::login_via_browser(os_config.clone()).await?,
    };
    crate::a3s_os::export_os_env(&session);
    let skill_ready = crate::a3s_os::ensure_capability_skill_dir(&os_config).is_some();
    println!("signed in to OS as {}", session.display_label());
    if skill_ready {
        println!("capabilities skill: active");
    } else {
        println!("capabilities skill: not installed (check ~/.a3s permissions)");
    }
    print_ssh_key_outcome(crate::a3s_os::sync_ssh_key(session).await);
    Ok(())
}

async fn run_logout() -> anyhow::Result<()> {
    let (_, os_config) = load_os_config()?;
    let removed = crate::a3s_os::logout(&os_config)?;
    crate::a3s_os::clear_os_env();
    crate::a3s_os::remove_capability_skill_dir();
    if removed {
        println!("signed out from OS");
    } else {
        println!("no stored OS login for {}", os_config.address);
    }
    Ok(())
}

async fn run_auth(args: &[String]) -> anyhow::Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        return run_auth_status().await;
    };
    match command {
        "-h" | "--help" | "help" => {
            println!("a3s code auth status|login|logout");
            Ok(())
        }
        "status" => run_auth_status().await,
        "login" => run_login(&args[1..]).await,
        "logout" => run_logout().await,
        other => anyhow::bail!("unknown auth command `{other}`; expected status, login, or logout"),
    }
}

async fn run_auth_status() -> anyhow::Result<()> {
    let (config_path, os_config) = load_os_config()?;
    println!("config: {config_path}");
    println!("os: {}", os_config.address);
    let Some(mut session) = crate::a3s_os::current_session(&os_config) else {
        println!("status: signed out");
        return Ok(());
    };
    if crate::a3s_os::needs_refresh(&session) {
        session = crate::a3s_os::refresh_session(&session).await?;
    }
    crate::a3s_os::export_os_env(&session);
    println!("status: signed in");
    println!("account: {}", session.display_label());
    println!("login_at: {}", format_unix_ms(session.login_at_ms));
    if let Some(expires_at) = session.expires_at_ms {
        println!("expires_at: {}", format_unix_ms(expires_at));
    } else {
        println!("expires_at: unknown");
    }
    Ok(())
}

async fn run_config(args: &[String]) -> anyhow::Result<()> {
    let command = args.first().map(String::as_str).unwrap_or("path");
    match command {
        "-h" | "--help" | "help" => {
            println!("a3s code config path|init [path]|cat|check|edit|dirs");
            Ok(())
        }
        "path" => {
            ensure_no_args("config path", &args[1..])?;
            match config::find_config() {
                Some(path) => println!("{path}"),
                None => {
                    let path = config::default_config_path()
                        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
                    println!("{}", path.display());
                    println!("(not created yet; run `a3s code config init`)");
                }
            }
            Ok(())
        }
        "init" => {
            if args.len() > 2 {
                anyhow::bail!("usage: a3s code config init [path]");
            }
            let path = match args.get(1) {
                Some(path) => expand_home(path),
                None => preferred_config_init_path()?,
            };
            let existed = path.exists();
            config::write_template_config(&path)
                .map_err(|e| anyhow::anyhow!("could not write {}: {e}", path.display()))?;
            if existed {
                println!("config already exists: {}", path.display());
            } else {
                println!("created config: {}", path.display());
            }
            Ok(())
        }
        "cat" => {
            ensure_no_args("config cat", &args[1..])?;
            let path = config::find_config()
                .ok_or_else(|| anyhow::anyhow!("no config found; run `a3s code config init`"))?;
            print!(
                "{}",
                std::fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("could not read {path}: {e}"))?
            );
            Ok(())
        }
        "check" => {
            ensure_no_args("config check", &args[1..])?;
            let (path, cfg) = load_code_config()?;
            println!("config: {path}");
            println!(
                "default_model: {}",
                cfg.default_model.as_deref().unwrap_or("(not set)")
            );
            println!("providers: {}", cfg.providers.len());
            println!("models: {}", cfg.list_models().len());
            println!(
                "os: {}",
                cfg.os
                    .as_ref()
                    .map(|os| os.address.as_str())
                    .unwrap_or("(not configured)")
            );
            Ok(())
        }
        "edit" => {
            ensure_no_args("config edit", &args[1..])?;
            let path = match config::find_config() {
                Some(path) => PathBuf::from(path),
                None => {
                    let path = config::default_config_path()
                        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
                    config::write_template_config(&path)
                        .map_err(|e| anyhow::anyhow!("could not create {}: {e}", path.display()))?;
                    path
                }
            };
            open_editor_or_print_path(&path)
        }
        "dirs" => {
            ensure_no_args("config dirs", &args[1..])?;
            print_code_dirs()
        }
        other => anyhow::bail!(
            "unknown config command `{other}`; expected path, init, cat, check, edit, or dirs"
        ),
    }
}

async fn run_models(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        None | Some("list") => {}
        Some("-h" | "--help" | "help") => {
            println!("a3s code models");
            println!(
                "  lists config.acl models, local Claude/Codex account models, and OS gateway models when signed in"
            );
            return Ok(());
        }
        Some(other) => anyhow::bail!("unknown models command `{other}`; expected list"),
    }
    let (_, cfg) = load_code_config()?;
    println!(
        "default: {}",
        cfg.default_model.as_deref().unwrap_or("(not set)")
    );
    println!("config.acl models:");
    if cfg.list_models().is_empty() {
        println!("  (none)");
    } else {
        for (provider, model) in cfg.list_models() {
            let id = format!("{}/{}", provider.name, model.id);
            let marker = if Some(id.as_str()) == cfg.default_model.as_deref() {
                "*"
            } else {
                " "
            };
            let display = if model.name.is_empty() {
                model.id.as_str()
            } else {
                model.name.as_str()
            };
            println!(
                "  {marker} {:<42} {}{}{}",
                id,
                display,
                if model.reasoning { " · reasoning" } else { "" },
                if model.tool_call { " · tools" } else { "" }
            );
        }
    }

    if panels::login::has_local_login(panels::login::AuthProvider::Claude) {
        println!("Claude Code account models:");
        for model in panels::login::claude_models() {
            println!("  {model}");
        }
    }
    if panels::login::has_local_login(panels::login::AuthProvider::Codex) {
        println!("Codex account models:");
        for model in crate::codex::codex_models() {
            println!("  {model}");
        }
    }
    if let Some(session) = current_os_session_if_configured().await {
        println!("OS gateway models:");
        match crate::a3s_os::fetch_gateway_models(&session.address, &session.access_token).await {
            Ok(models) if models.is_empty() => println!("  (none configured)"),
            Ok(models) => {
                for model in models {
                    match model.context {
                        Some(context) => println!("  {} · context {}", model.id, context),
                        None => println!("  {}", model.id),
                    }
                }
            }
            Err(error) => println!("  unavailable: {error}"),
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeepResearchRuntimeMode {
    Auto,
    Local,
    Os,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeepResearchCliOptions {
    query: String,
    runtime_mode: DeepResearchRuntimeMode,
}

fn parse_deepresearch_args(args: &[String]) -> anyhow::Result<DeepResearchCliOptions> {
    let mut runtime_mode = DeepResearchRuntimeMode::Auto;
    let mut query_parts = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--local" => runtime_mode = DeepResearchRuntimeMode::Local,
            "--os" => runtime_mode = DeepResearchRuntimeMode::Os,
            "-h" | "--help" | "help" => {
                anyhow::bail!("usage: a3s code deepresearch [--local|--os] <query>");
            }
            value if value.starts_with('-') => {
                anyhow::bail!("unknown a3s code deepresearch option `{value}`")
            }
            value => query_parts.push(value.to_string()),
        }
    }
    let query = query_parts.join(" ").trim().to_string();
    if query.is_empty() {
        anyhow::bail!("usage: a3s code deepresearch [--local|--os] <query>");
    }
    Ok(DeepResearchCliOptions {
        query,
        runtime_mode,
    })
}

async fn run_deepresearch(args: &[String]) -> anyhow::Result<()> {
    if matches!(
        args.first().map(String::as_str),
        Some("-h" | "--help" | "help")
    ) {
        print_deepresearch_help();
        return Ok(());
    }
    let opts = parse_deepresearch_args(args)?;
    if opts.runtime_mode == DeepResearchRuntimeMode::Os {
        anyhow::bail!(
            "--os is temporarily disabled for DeepResearch; OS Runtime support should use Function-as-a-Service instead of remote tool-call fan-out"
        );
    }
    let workspace = std::env::current_dir()?;
    let workspace_text = workspace.to_string_lossy().to_string();
    let session = build_deepresearch_session(&workspace_text).await?;
    let os_runtime = match opts.runtime_mode {
        DeepResearchRuntimeMode::Local => false,
        DeepResearchRuntimeMode::Os => false,
        DeepResearchRuntimeMode::Auto => false,
    };

    eprintln!(
        "deepresearch: gathering evidence via {} workflow…",
        if os_runtime { "OS Runtime" } else { "local" }
    );
    let workflow_args = super::deep_research_workflow_args(&opts.query, os_runtime);
    let workflow = run_deepresearch_workflow(&session, workflow_args.clone()).await;
    let (workflow_output, exit_code, metadata) = match workflow {
        Ok(result) => (result.output, result.exit_code, result.metadata),
        Err(error) => (error, 1, None),
    };

    let synthesis = synthesize_deepresearch_report(
        &session,
        &workspace,
        &opts.query,
        os_runtime,
        &workflow_output,
        exit_code,
        metadata.as_ref(),
    )
    .await?;

    print!("{}", synthesis.text);
    if !synthesis.text.ends_with('\n') {
        println!();
    }
    println!("report.md: {}", synthesis.artifacts.markdown.display());
    println!("index.html: {}", synthesis.artifacts.html.display());
    if synthesis.status == DeepResearchReportStatus::FallbackDraft {
        anyhow::bail!(
            "DeepResearch did not complete; fallback draft written at {}",
            synthesis.artifacts.html.display()
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeepResearchReportStatus {
    Completed,
    FallbackDraft,
}

#[derive(Debug)]
struct DeepResearchReportSynthesis {
    text: String,
    artifacts: super::ResearchReportArtifacts,
    status: DeepResearchReportStatus,
}

async fn synthesize_deepresearch_report(
    session: &AgentSession,
    workspace: &Path,
    query: &str,
    os_runtime: bool,
    workflow_output: &str,
    exit_code: i32,
    metadata: Option<&serde_json::Value>,
) -> anyhow::Result<DeepResearchReportSynthesis> {
    eprintln!("deepresearch: synthesizing report artifacts…");
    let prompt = if exit_code == 0 {
        super::deep_research_synthesis_prompt(query, os_runtime, workflow_output, metadata)
    } else {
        super::deep_research_recovery_prompt(query, os_runtime, workflow_output, metadata)
    };
    let (mut final_text, synthesis_completed) = send_deepresearch_text(
        session,
        &prompt,
        super::DEEP_RESEARCH_SYNTHESIS_TIMEOUT_MS,
        "synthesis",
    )
    .await;
    let mut artifacts =
        super::research_report_artifacts_from_output_for_query(&final_text, workspace, query);
    let mut status = DeepResearchReportStatus::Completed;

    if artifacts.is_none() && synthesis_completed {
        eprintln!("deepresearch: report marker/artifacts missing, running focused repair pass…");
        let repair = super::deep_research_repair_prompt(
            query,
            os_runtime,
            workflow_output,
            metadata,
            &final_text,
        );
        let (repair_text, repair_completed) = send_deepresearch_text(
            session,
            &repair,
            super::DEEP_RESEARCH_REPAIR_TIMEOUT_MS,
            "repair",
        )
        .await;
        final_text = repair_text;
        artifacts =
            super::research_report_artifacts_from_output_for_query(&final_text, workspace, query);
        if !repair_completed {
            eprintln!("deepresearch: repair pass did not complete, using host fallback…");
        }
    }

    if artifacts.is_none() {
        eprintln!(
            "deepresearch: report artifacts still missing, materializing host fallback draft…"
        );
        let fallback_artifacts = super::materialize_deep_research_fallback_draft(
            workspace,
            query,
            &final_text,
            workflow_output,
        )
        .map_err(anyhow::Error::msg)?;
        if !final_text.ends_with('\n') {
            final_text.push('\n');
        }
        final_text.push_str(&format!(
            "DeepResearch fallback draft written at {}\n",
            fallback_artifacts.html.display()
        ));
        artifacts = Some(fallback_artifacts);
        status = DeepResearchReportStatus::FallbackDraft;
    }

    let artifacts = artifacts.ok_or_else(|| {
        anyhow::anyhow!(
            "DeepResearch did not produce the required report artifacts: expected `A3S_RESEARCH_VIEW: .a3s/research/<slug>/index.html`, plus sibling report.md"
        )
    })?;
    Ok(DeepResearchReportSynthesis {
        text: final_text,
        artifacts,
        status,
    })
}

async fn send_deepresearch_text(
    session: &AgentSession,
    prompt: &str,
    timeout_ms: u64,
    phase: &str,
) -> (String, bool) {
    match tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        session.send(prompt, Some(&[])),
    )
    .await
    {
        Ok(Ok(result)) => (result.text, true),
        Ok(Err(error)) => (
            format!("DeepResearch {phase} model call failed: {error}"),
            false,
        ),
        Err(_) => (
            format!("DeepResearch {phase} model call timed out after {timeout_ms} ms."),
            false,
        ),
    }
}

fn print_deepresearch_help() {
    println!("a3s code deepresearch [--local|--os] <query>");
    println!("  run DeepResearch from the CLI and write:");
    println!("    .a3s/research/<slug>/report.md");
    println!("    .a3s/research/<slug>/index.html");
    println!("  --local  force local parallel_task research");
    println!("  --os     temporarily disabled; future OS Runtime support should use FaaS");
}

fn deepresearch_cli_permission_policy() -> a3s_code_core::permissions::PermissionPolicy {
    let mut policy = a3s_code_core::permissions::PermissionPolicy::new()
        .deny_all(&[
            "Write(/**)",
            "Edit(/**)",
            "Write(**/../**)",
            "Edit(**/../**)",
        ])
        .allow_all(&[
            "Read(*)",
            "Grep(*)",
            "Glob(*)",
            "LS(*)",
            "read(*)",
            "grep(*)",
            "glob(*)",
            "ls(*)",
            "web_search(*)",
            "web_fetch(*)",
            "Write(.a3s/research/**)",
            "Edit(.a3s/research/**)",
            "write(.a3s/research/**)",
            "edit(.a3s/research/**)",
        ]);
    policy.default_decision = a3s_code_core::permissions::PermissionDecision::Deny;
    policy
}

async fn build_deepresearch_session(workspace: &str) -> anyhow::Result<AgentSession> {
    let (config_path, _) = load_code_config()?;
    let agent = Agent::new(config_path.clone())
        .await
        .map_err(|e| anyhow::anyhow!("failed to load agent from {config_path}: {e}"))?;
    let budget = super::deep_research_default_budget();
    let opts = SessionOptions::new()
        .with_confirmation_policy(a3s_code_core::hitl::ConfirmationPolicy::default())
        .with_permission_policy(deepresearch_cli_permission_policy())
        .with_tool_timeout(super::TOOL_EXEC_TIMEOUT_MS)
        .with_duplicate_tool_call_threshold(super::TUI_DUPLICATE_TOOL_CALL_THRESHOLD)
        .with_file_memory(config::memory_dir())
        .with_max_parallel_tasks(budget.max_parallel_tasks)
        .with_max_tool_rounds(budget.max_tool_rounds)
        .with_max_continuation_turns(budget.max_continuation_turns)
        .with_auto_delegation_enabled(true)
        .with_auto_parallel_delegation(true)
        .with_manual_delegation_enabled(true);
    let session = agent.session(workspace.to_string(), Some(opts))?;
    session.register_dynamic_workflow_runtime();
    Ok(session)
}

async fn run_deepresearch_workflow(
    session: &AgentSession,
    args: serde_json::Value,
) -> Result<ToolCallResult, String> {
    let (mut progress_rx, workflow_join) = session.tool_with_events("dynamic_workflow", args);
    let workflow_abort = workflow_join.abort_handle();
    let progress_drain = tokio::spawn(async move { while progress_rx.recv().await.is_some() {} });
    let timeout_ms = super::DEEP_RESEARCH_SCRIPT_TIMEOUT_MS + 30_000;
    let result = match tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        workflow_join,
    )
    .await
    {
        Ok(Ok(result)) => result.map_err(|err| err.to_string()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => {
            workflow_abort.abort();
            Err(format!(
                "dynamic_workflow timed out after {timeout_ms} ms while gathering DeepResearch evidence"
            ))
        }
    };
    progress_drain.abort();
    result
}

fn run_kb(args: &[String]) -> anyhow::Result<()> {
    let command = args.first().map(String::as_str).unwrap_or("stats");
    match command {
        "-h" | "--help" | "help" => {
            print_kb_help();
            Ok(())
        }
        "stats" | "home" => {
            ensure_no_args("kb stats", &args[1..])?;
            print_kb_stats()
        }
        "vault" | "dir" => {
            ensure_no_args("kb vault", &args[1..])?;
            let cwd = cwd_string()?;
            println!("{}", kbutil::kb_dir(&cwd).display());
            Ok(())
        }
        "add" => {
            let text = join_required("kb add", &args[1..], "<text>")?;
            let cwd = cwd_string()?;
            println!(
                "{}",
                kbutil::add_text_to_kb(&cwd, &text, &chrono::Utc::now().to_rfc3339())
            );
            Ok(())
        }
        "import" => {
            let path = single_required("kb import", &args[1..], "<file|folder>")?;
            let cwd = cwd_string()?;
            println!(
                "{}",
                kbutil::import_to_kb(&cwd, &path, &chrono::Utc::now().to_rfc3339())
            );
            Ok(())
        }
        "search" => {
            let query = join_required("kb search", &args[1..], "<query>")?;
            let cwd = cwd_string()?;
            let hits = kbutil::search_kb(&cwd, &query);
            println!("{} hit(s) for `{query}`", hits.len());
            for hit in hits {
                println!("{}:{}\t{}", hit.path, hit.line, hit.snippet);
            }
            Ok(())
        }
        other => anyhow::bail!(
            "unknown kb command `{other}`; expected stats, add, import, search, or vault"
        ),
    }
}

async fn run_ctx(args: &[String]) -> anyhow::Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print_ctx_help();
        return Ok(());
    };
    match command {
        "-h" | "--help" | "help" => {
            print_ctx_help();
            Ok(())
        }
        "search" => {
            let query = join_required("ctx search", &args[1..], "<query>")?;
            ctx_search(&query).await
        }
        "show" | "event" => {
            let (event_id, window) = parse_ctx_show_args(&args[1..])?;
            ctx_show_event(&event_id, window).await
        }
        "session" => {
            let session_id = single_required("ctx session", &args[1..], "<session-id>")?;
            ctx_show_session(&session_id).await
        }
        other => ctx_search(&join_args(args)).await.map_err(|error| {
            error.context(format!(
                "`{other}` was treated as a ctx search term; use `a3s code ctx --help` for commands"
            ))
        }),
    }
}

fn run_memory(args: &[String]) -> anyhow::Result<()> {
    let command = args.first().map(String::as_str).unwrap_or("list");
    match command {
        "-h" | "--help" | "help" => {
            println!("a3s code memory list [query]|stats|dir");
            Ok(())
        }
        "dir" => {
            ensure_no_args("memory dir", &args[1..])?;
            println!("{}", config::memory_dir().display());
            Ok(())
        }
        "stats" => {
            ensure_no_args("memory stats", &args[1..])?;
            print_memory_stats();
            Ok(())
        }
        "list" => {
            print_memory_list(&join_args(&args[1..]));
            Ok(())
        }
        other => {
            print_memory_list(&join_args(args));
            if other.starts_with('-') {
                anyhow::bail!("unknown memory option `{other}`");
            }
            Ok(())
        }
    }
}

fn optional_single_arg(command: &str, args: &[String]) -> anyhow::Result<Option<String>> {
    match args {
        [] => Ok(None),
        [value] => Ok(Some(value.clone())),
        _ => anyhow::bail!("usage: a3s code {command} [value]"),
    }
}

fn ensure_no_args(command: &str, args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("usage: a3s code {command}")
    }
}

fn single_required(command: &str, args: &[String], placeholder: &str) -> anyhow::Result<String> {
    match args {
        [value] => Ok(value.clone()),
        _ => anyhow::bail!("usage: a3s code {command} {placeholder}"),
    }
}

fn join_required(command: &str, args: &[String], placeholder: &str) -> anyhow::Result<String> {
    let value = join_args(args);
    if value.trim().is_empty() {
        anyhow::bail!("usage: a3s code {command} {placeholder}");
    }
    Ok(value)
}

fn load_code_config() -> anyhow::Result<(String, CodeConfig)> {
    let config_path = config::find_config().ok_or_else(|| {
        anyhow::anyhow!(
            "config.acl was not found; run `a3s code config init` or set A3S_CONFIG_FILE"
        )
    })?;
    let cfg = CodeConfig::from_file(Path::new(&config_path))
        .map_err(|e| anyhow::anyhow!("failed to parse {config_path}: {e}"))?;
    Ok((config_path, cfg))
}

fn load_os_config() -> anyhow::Result<(String, OsConfig)> {
    let (config_path, cfg) = load_code_config()?;
    let os_config = cfg.os.ok_or_else(|| {
        anyhow::anyhow!("OS is not configured in {config_path}; set os = \"https://your-os-host\"")
    })?;
    Ok((config_path, os_config))
}

fn preferred_config_init_path() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var("A3S_CONFIG_FILE") {
        if !path.trim().is_empty() {
            return Ok(expand_home(&path));
        }
    }
    config::default_config_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

async fn current_os_session_if_configured() -> Option<crate::a3s_os::StoredOsSession> {
    let (_, os_config) = load_os_config().ok()?;
    let mut session = crate::a3s_os::current_session(&os_config)?;
    if crate::a3s_os::needs_refresh(&session) {
        session = crate::a3s_os::refresh_session(&session).await.ok()?;
    }
    crate::a3s_os::export_os_env(&session);
    Some(session)
}

fn print_ssh_key_outcome(outcome: crate::a3s_os::SshKeyOutcome) {
    match outcome {
        crate::a3s_os::SshKeyOutcome::Registered(fp) => {
            println!("ssh key: registered with OS ({fp})")
        }
        crate::a3s_os::SshKeyOutcome::AlreadyRegistered => {
            println!("ssh key: already registered")
        }
        crate::a3s_os::SshKeyOutcome::NoLocalKey => {
            println!("ssh key: none found (create one with `ssh-keygen -t ed25519`)")
        }
        crate::a3s_os::SshKeyOutcome::Failed(error) => {
            println!("ssh key: sync skipped: {error}")
        }
    }
}

fn format_unix_ms(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let nanos = ((ms % 1000) * 1_000_000) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| ms.to_string())
}

fn open_editor_or_print_path(path: &Path) -> anyhow::Result<()> {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|v| !v.trim().is_empty())
        });
    let Some(editor) = editor else {
        println!("{}", path.display());
        println!("set VISUAL or EDITOR to edit from the CLI");
        return Ok(());
    };
    let mut parts = editor.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("editor command is empty"))?;
    let status = Command::new(program)
        .args(parts)
        .arg(path)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to launch editor `{editor}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("editor `{editor}` exited with {status}")
    }
}

fn print_code_dirs() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let cwd_text = cwd.to_string_lossy();
    let config_path = config::find_config().unwrap_or_else(|| {
        config::default_config_path()
            .map(|p| format!("{} (not created)", p.display()))
            .unwrap_or_else(|| "(not found; HOME is not set)".to_string())
    });
    let rows = [
        ("config", config_path),
        ("agent", config::agent_dir().display().to_string()),
        ("mcp", config::mcp_dir().display().to_string()),
        ("skill", config::skill_dir().display().to_string()),
        ("flow", config::flow_dir().display().to_string()),
        ("memory", config::memory_dir().display().to_string()),
        ("kb", kbutil::kb_dir(&cwd_text).display().to_string()),
        (
            "okf",
            panels::okf::okf_package_dir(&cwd_text)
                .display()
                .to_string(),
        ),
    ];
    for (name, path) in rows {
        println!("{name:<8} {path}");
    }
    Ok(())
}

fn print_kb_help() {
    println!("a3s code kb stats");
    println!("a3s code kb add <text>");
    println!("a3s code kb import <file|folder>");
    println!("a3s code kb search <query>");
    println!("a3s code kb vault");
}

fn print_kb_stats() -> anyhow::Result<()> {
    let cwd = cwd_string()?;
    let stats = kbutil::kb_stats(&cwd);
    println!("kb: {}", kbutil::kb_dir(&cwd).display());
    println!(
        "sources: {} · concepts: {} · imports: {} · size: {}",
        stats.sources,
        stats.concepts,
        stats.imports,
        format_bytes(stats.bytes)
    );
    let recent = kbutil::recent_sources(&cwd, 8);
    if recent.is_empty() {
        println!("recent: (none)");
    } else {
        println!("recent:");
        for item in recent {
            println!("  {item}");
        }
    }
    Ok(())
}

fn cwd_string() -> anyhow::Result<String> {
    Ok(std::env::current_dir()?.to_string_lossy().into_owned())
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn print_ctx_help() {
    println!("a3s code ctx search <query>");
    println!("a3s code ctx show <event-id> [--window N]");
    println!("a3s code ctx session <session-id>");
    println!("note: /ctx <n> attach is TUI session state and stays interactive-only");
}

async fn ctx_search(query: &str) -> anyhow::Result<()> {
    let out = tokio::process::Command::new("ctx")
        .args([
            "search",
            "--refresh",
            "off",
            "--limit",
            "8",
            "--json",
            "--",
            query,
        ])
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run ctx: {e}"))?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let hits = panels::ctx::parse_ctx_search(&stdout).map_err(anyhow::Error::msg)?;
    println!("{} ctx hit(s) for `{query}`", hits.len());
    for (idx, hit) in hits.iter().enumerate() {
        println!(
            "{}. {} · {} · {}",
            idx + 1,
            hit.provider,
            hit.time,
            hit.title
        );
        println!("   event: {}", hit.event_id);
        if !hit.session_id.is_empty() {
            println!("   session: {}", hit.session_id);
        }
        if !hit.snippet.is_empty() {
            println!("   {}", hit.snippet);
        }
    }
    Ok(())
}

fn parse_ctx_show_args(args: &[String]) -> anyhow::Result<(String, usize)> {
    if args.is_empty() {
        anyhow::bail!("usage: a3s code ctx show <event-id> [--window N]");
    }
    let event_id = args[0].clone();
    let mut window = 5usize;
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--window" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("--window requires a value"))?;
                window = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("--window must be a positive integer"))?;
                if window == 0 {
                    anyhow::bail!("--window must be greater than zero");
                }
                i += 2;
            }
            other => anyhow::bail!("unknown ctx show option `{other}`"),
        }
    }
    Ok((event_id, window))
}

async fn ctx_show_event(event_id: &str, window: usize) -> anyhow::Result<()> {
    let window = window.to_string();
    let out = tokio::process::Command::new("ctx")
        .args(["show", "event", event_id, "--window", &window])
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run ctx: {e}"))?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    print!("{}", String::from_utf8_lossy(&out.stdout));
    Ok(())
}

async fn ctx_show_session(session_id: &str) -> anyhow::Result<()> {
    let out = tokio::process::Command::new("ctx")
        .args(["show", "session", session_id])
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run ctx: {e}"))?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    print!("{}", String::from_utf8_lossy(&out.stdout));
    Ok(())
}

fn print_memory_stats() {
    let dir = config::memory_dir();
    let data = memutil::load_panel_data(&dir);
    let stats = data.graph.stats;
    println!("memory: {}", dir.display());
    println!("entries: {}", data.entries.len());
    println!(
        "graph: {} event(s) · {} entity(ies) · {} relation(s) · {} alias(es)",
        stats.events, stats.entities, stats.relations, stats.aliases
    );
    println!(
        "tiers: short {} · mid {} · long {} · forget candidates {}",
        stats.short, stats.mid, stats.long, stats.forget_candidates
    );
}

fn print_memory_list(query: &str) {
    let dir = config::memory_dir();
    let data = memutil::load_panel_data(&dir);
    let rows = data
        .entries
        .iter()
        .filter(|entry| {
            let content = memory_content(&data, entry);
            let haystack = format!("{} {} {}", entry.id, entry.tags.join(" "), content);
            matches_query(&haystack, query)
        })
        .collect::<Vec<_>>();
    println!(
        "{} memory entr{} in {}",
        rows.len(),
        if rows.len() == 1 { "y" } else { "ies" },
        dir.display()
    );
    for entry in rows {
        let content = memory_content(&data, entry);
        println!(
            "{}\t{}\t{:.2}\t{}\t{}",
            trim_col(&entry.id, 8),
            if entry.memory_type.is_empty() {
                "memory"
            } else {
                entry.memory_type.as_str()
            },
            entry.importance,
            entry.timestamp.format("%Y-%m-%d"),
            trim_col(&content.replace('\n', " "), 120)
        );
    }
}

fn memory_content(data: &memutil::MemPanelData, entry: &memutil::MemEntry) -> String {
    data.details
        .get(&entry.id)
        .map(|detail| detail.content.trim().to_string())
        .filter(|content| !content.is_empty())
        .unwrap_or_else(|| entry.content_lower.clone())
}

async fn clone_asset(family: &'static str, url: String, root: PathBuf) -> anyhow::Result<()> {
    let result = asset_clone::clone_asset_source(family, url, root)
        .await
        .map_err(anyhow::Error::msg)?;
    println!(
        "cloned {} asset source\nurl: {}\npath: {}",
        result.family,
        result.url,
        result.path.display()
    );
    Ok(())
}

async fn list_assets(category: &str, query: &str) -> anyhow::Result<()> {
    let session = load_os_session().await?;
    let query = os_asset_category_query(category, query);
    let result =
        panels::asset_resources::fetch_asset_list(&session.address, &session.access_token, &query)
            .await
            .map_err(anyhow::Error::msg)?;
    println!("{}", result.note);
    if result.rows.is_empty() {
        println!("(no assets)");
        return Ok(());
    }
    println!(
        "{:<28} {:<30} {:<12} {:<14} {:<12} updated",
        "id", "name", "category", "kind", "status"
    );
    for row in result.rows {
        println!(
            "{:<28} {:<30} {:<12} {:<14} {:<12} {}",
            trim_col(&row.id, 28),
            trim_col(&row.name, 30),
            trim_col(&row.category, 12),
            trim_col(&row.kind, 14),
            trim_col(&row.status, 12),
            row.updated
        );
    }
    Ok(())
}

async fn runtime_activity(category: &str, query: &str) -> anyhow::Result<()> {
    let session = load_os_session().await?;
    let query = runtime_asset_query(category, "", query);
    let result = panels::asset_resources::fetch_runtime_activity(
        &session.address,
        &session.access_token,
        &query,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    println!("{}", result.note);
    if result.rows.is_empty() {
        println!("(no runtime activity)");
        return Ok(());
    }
    println!(
        "{:<28} {:<30} {:<12} {:<14} {:<12} updated",
        "id", "name", "category", "kind", "status"
    );
    for row in result.rows {
        println!(
            "{:<28} {:<30} {:<12} {:<14} {:<12} {}",
            trim_col(&row.id, 28),
            trim_col(&row.name, 30),
            trim_col(&row.asset_category, 12),
            trim_col(&row.kind, 14),
            trim_col(&row.status, 12),
            row.updated
        );
    }
    Ok(())
}

async fn run_agent_os(
    action: panels::agent::AgentOsAction,
    path_arg: Option<&str>,
    open_requested: bool,
) -> anyhow::Result<()> {
    let dev = resolve_agent_dev(path_arg.map(str::to_string))?;
    let session = load_os_session().await?;
    let result = panels::agent::publish_agent_to_os(session, dev, action)
        .await
        .map_err(anyhow::Error::msg)?;
    println!(
        "agent {} {}: {} ({})",
        result.action.label(),
        result.kind.label(),
        result.asset_name,
        result.asset_id
    );
    print_os_note(&result.note, &result.view);
    open_if_requested(open_requested, &result.view)?;
    Ok(())
}

async fn run_mcp_os(
    action: panels::mcp::McpOsAction,
    path_arg: Option<&str>,
    open_requested: bool,
) -> anyhow::Result<()> {
    let dev = resolve_mcp_dev(path_arg.map(str::to_string))?;
    let session = load_os_session().await?;
    let result = panels::mcp::publish_mcp_to_os(session, dev, action)
        .await
        .map_err(anyhow::Error::msg)?;
    println!(
        "mcp {}: {} ({})",
        result.action.label(),
        result.asset_name,
        result.asset_id
    );
    print_os_note(&result.note, &result.view);
    open_if_requested(open_requested, &result.view)?;
    Ok(())
}

async fn run_skill_os(
    action: panels::skill::SkillOsAction,
    path_arg: Option<&str>,
    open_requested: bool,
) -> anyhow::Result<()> {
    let dev = resolve_skill_dev(path_arg.map(str::to_string))?;
    let session = load_os_session().await?;
    let result = panels::skill::publish_skill_to_os(session, dev, action)
        .await
        .map_err(anyhow::Error::msg)?;
    println!(
        "skill {}: {} ({})",
        result.action.label(),
        result.asset_name,
        result.asset_id
    );
    print_os_note(&result.note, &result.view);
    open_if_requested(open_requested, &result.view)?;
    Ok(())
}

async fn run_flow_os(
    action: panels::flow::FlowOsAction,
    path_arg: Option<&str>,
    open_requested: bool,
) -> anyhow::Result<()> {
    let flow = resolve_flow_file(path_arg.map(str::to_string))?;
    let design = read_flow_design(&flow.path)?;
    let session = load_os_session().await?;
    let result = panels::flow::publish_flow_to_os_with_local_path(
        session,
        flow.rel,
        Some(flow.path.clone()),
        design,
        action,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    println!(
        "flow {}: {} ({})",
        result.action.label(),
        result.asset_name,
        result.asset_id
    );
    print_os_note(&result.note, &result.view);
    open_if_requested(open_requested, &result.view)?;
    Ok(())
}

async fn run_okf_os(
    action: panels::okf::OkfOsAction,
    path_arg: Option<&str>,
) -> anyhow::Result<()> {
    let dev = resolve_okf_dev(path_arg.map(str::to_string))?;
    let session = load_os_session().await?;
    let result = panels::okf::publish_okf_to_os(session, dev, action)
        .await
        .map_err(anyhow::Error::msg)?;
    println!(
        "okf {}: {} ({})",
        result.action.label(),
        result.asset_name,
        result.asset_id
    );
    print_os_note(&result.note, &result.view);
    Ok(())
}

async fn load_os_session() -> anyhow::Result<crate::a3s_os::StoredOsSession> {
    let (_, os_config) = load_os_config()?;
    let session = crate::a3s_os::current_session(&os_config)
        .ok_or_else(|| anyhow::anyhow!("not signed in to OS; run `a3s code` and /login first"))?;
    let session = if crate::a3s_os::needs_refresh(&session) {
        crate::a3s_os::refresh_session(&session).await?
    } else {
        session
    };
    crate::a3s_os::export_os_env(&session);
    Ok(session)
}

fn resolve_agent_dev(path_arg: Option<String>) -> anyhow::Result<panels::agent::AgentDevSession> {
    let config_root = config::agent_dir();
    let target = resolve_optional_target(path_arg)?;
    let file = choose_agent_file(&target)?;
    let root = asset_root_for_file(&config_root, &file);
    panels::agent::agent_dev_session_from_file(&root, &file).map_err(anyhow::Error::msg)
}

fn resolve_mcp_dev(path_arg: Option<String>) -> anyhow::Result<panels::mcp::McpDevSession> {
    let config_root = config::mcp_dir();
    let target = asset_dir_target(resolve_optional_target(path_arg)?);
    let project = choose_mcp_project(&target, &config_root)?;
    Ok(panels::mcp::McpDevSession {
        name: project.name,
        description: project.description,
        rel: project.rel,
        path: project.path,
        root: asset_root_for_dir(&config_root, &target),
    })
}

fn resolve_skill_dev(path_arg: Option<String>) -> anyhow::Result<panels::skill::SkillDevSession> {
    let config_root = config::skill_dir();
    let target = resolve_optional_target(path_arg)?;
    let file = choose_skill_file(&target)?;
    let root = asset_root_for_file(&config_root, &file);
    Ok(panels::skill::skill_dev_session_from_file(&root, &file))
}

#[derive(Debug, Clone)]
struct FlowFile {
    rel: String,
    path: PathBuf,
}

fn resolve_flow_file(path_arg: Option<String>) -> anyhow::Result<FlowFile> {
    let config_root = config::flow_dir();
    let target = resolve_optional_target(path_arg)?;
    let file = choose_flow_file(&target)?;
    let root = asset_root_for_file(&config_root, &file);
    let rel = rel_to_root(&root, &file);
    Ok(FlowFile { rel, path: file })
}

fn resolve_okf_dev(path_arg: Option<String>) -> anyhow::Result<panels::okf::OkfDevSession> {
    let cwd = std::env::current_dir()?;
    let default_root = panels::okf::okf_package_dir(&cwd.to_string_lossy());
    let target = match path_arg {
        Some(path) => resolve_path(&path)?,
        None => {
            if panels::okf::okf_package_asset_from_dir(&cwd, &cwd).is_some() {
                cwd
            } else {
                default_root.clone()
            }
        }
    };
    let target = asset_dir_target(target);
    let package = choose_okf_package(&target, &default_root)?;
    Ok(panels::okf::OkfDevSession {
        name: package.name,
        description: package.description,
        rel: package.rel,
        path: package.path,
        root: asset_root_for_dir(&default_root, &target),
    })
}

fn choose_agent_file(target: &Path) -> anyhow::Result<PathBuf> {
    if target.is_file() {
        return Ok(target.to_path_buf());
    }
    if let Some(entry) = panels::agent::agent_entry_file(target) {
        return Ok(entry);
    }
    let agents = panels::agent::list_agents(target);
    match agents.as_slice() {
        [agent] => Ok(agent.definition_path.clone()),
        [] => anyhow::bail!("no agent package found in {}", target.display()),
        _ => anyhow::bail!(
            "multiple agent packages found in {}; pass one package path",
            target.display()
        ),
    }
}

fn choose_mcp_project(
    target: &Path,
    config_root: &Path,
) -> anyhow::Result<panels::mcp::McpProject> {
    let root = if target.is_dir() {
        asset_root_for_dir(config_root, target)
    } else {
        anyhow::bail!("MCP path must be a directory: {}", target.display());
    };
    let projects = panels::mcp::list_mcp_projects(target);
    match projects.as_slice() {
        [project] => {
            let rel = rel_to_root(&root, &project.path);
            Ok(panels::mcp::McpProject {
                rel,
                path: project.path.clone(),
                name: project.name.clone(),
                description: project.description.clone(),
            })
        }
        [] => anyhow::bail!("no MCP asset found in {}", target.display()),
        _ => anyhow::bail!(
            "multiple MCP assets found in {}; pass one asset directory",
            target.display()
        ),
    }
}

fn choose_skill_file(target: &Path) -> anyhow::Result<PathBuf> {
    if target.is_file() {
        return Ok(target.to_path_buf());
    }
    let skills = panels::skill::list_skill_assets(target);
    match skills.as_slice() {
        [skill] => Ok(skill.path.clone()),
        [] => anyhow::bail!("no skill asset found in {}", target.display()),
        _ => anyhow::bail!(
            "multiple skill assets found in {}; pass one skill file or directory",
            target.display()
        ),
    }
}

fn choose_flow_file(target: &Path) -> anyhow::Result<PathBuf> {
    if target.is_file() {
        return Ok(target.to_path_buf());
    }
    let flows = panels::flow::list_flows(target);
    match flows.as_slice() {
        [flow] => Ok(target.join(flow)),
        [] => anyhow::bail!("no workflow JSON found in {}", target.display()),
        _ => anyhow::bail!(
            "multiple workflow JSON files found in {}; pass one file path",
            target.display()
        ),
    }
}

fn choose_okf_package(
    target: &Path,
    default_root: &Path,
) -> anyhow::Result<panels::okf::OkfPackageAsset> {
    let root = asset_root_for_dir(default_root, target);
    if target.is_dir() {
        if let Some(package) = panels::okf::okf_package_asset_from_dir(&root, target) {
            return Ok(package);
        }
    }
    let packages = panels::okf::list_okf_packages(target);
    match packages.as_slice() {
        [package] => {
            let rel = rel_to_root(&root, &package.path);
            Ok(panels::okf::OkfPackageAsset {
                rel,
                path: package.path.clone(),
                name: package.name.clone(),
                description: package.description.clone(),
            })
        }
        [] => anyhow::bail!("no OKF package found in {}", target.display()),
        _ => anyhow::bail!(
            "multiple OKF packages found in {}; pass one package directory",
            target.display()
        ),
    }
}

fn asset_dir_target(path: PathBuf) -> PathBuf {
    if path.is_file() {
        return path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.clone());
    }
    path
}

fn resolve_optional_target(path_arg: Option<String>) -> anyhow::Result<PathBuf> {
    match path_arg {
        Some(path) => resolve_path(&path),
        None => std::env::current_dir().map_err(anyhow::Error::from),
    }
}

fn resolve_path(path: &str) -> anyhow::Result<PathBuf> {
    let expanded = expand_home(path);
    let path = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()?.join(expanded)
    };
    std::fs::canonicalize(&path)
        .map_err(|e| anyhow::anyhow!("could not resolve {}: {e}", path.display()))
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Path::new(&home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn asset_root_for_file(config_root: &Path, file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    asset_root_for_dir(config_root, dir)
}

fn asset_root_for_dir(config_root: &Path, dir: &Path) -> PathBuf {
    if let (Ok(config_root), Ok(dir)) = (
        std::fs::canonicalize(config_root),
        std::fs::canonicalize(dir),
    ) {
        if dir.starts_with(&config_root) {
            return config_root;
        }
    }
    dir.to_path_buf()
}

fn rel_to_root(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn read_flow_design(path: &Path) -> anyhow::Result<String> {
    let design = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("could not read {}: {e}", path.display()))?;
    serde_json::from_str::<serde_json::Value>(&design)
        .map_err(|e| anyhow::anyhow!("{} is not valid workflow JSON: {e}", path.display()))?;
    Ok(design)
}

fn parse_clone_url(family: &str, args: &[String]) -> anyhow::Result<String> {
    if args.len() != 1 {
        anyhow::bail!("usage: a3s code {family} clone <git-url>");
    }
    Ok(args[0].clone())
}

fn parse_agent_publish_args(
    args: &[String],
) -> anyhow::Result<(panels::agent::AgentOsKind, Option<String>)> {
    if args.is_empty() || args.len() > 2 {
        anyhow::bail!("usage: a3s code agent publish agentic|application|tool [package]");
    }
    let parsed = panels::agent::parse_agent_subcommand(&format!("publish {}", args[0]))
        .ok_or_else(|| {
            anyhow::anyhow!("usage: a3s code agent publish agentic|application|tool [package]")
        })?
        .map_err(anyhow::Error::msg)?;
    let kind = match parsed {
        panels::agent::AgentSubcommand::Publish(kind) => kind,
        _ => unreachable!(),
    };
    Ok((kind, args.get(1).cloned()))
}

fn parse_agent_kind_path(
    command: &str,
    args: &[String],
) -> anyhow::Result<(panels::agent::AgentOsKind, Option<String>)> {
    if args.len() > 2 {
        anyhow::bail!("usage: a3s code agent {command} [agentic|application|tool] [package]");
    }
    let default = panels::agent::AgentOsKind::Agentic;
    let Some(first) = args.first() else {
        return Ok((default, None));
    };
    if is_agent_kind(first) {
        if command == "run" {
            let kind = panels::agent::AgentOsKind::parse(first).map_err(anyhow::Error::msg)?;
            return Ok((kind, args.get(1).cloned()));
        }
        let parsed = panels::agent::parse_agent_subcommand(&format!("{command} {first}"))
            .ok_or_else(|| anyhow::anyhow!("usage: a3s code agent {command} [kind] [package]"))?
            .map_err(anyhow::Error::msg)?;
        let kind = match parsed {
            panels::agent::AgentSubcommand::Open(kind)
            | panels::agent::AgentSubcommand::Logs(kind)
            | panels::agent::AgentSubcommand::Status(kind) => kind,
            _ => unreachable!(),
        };
        return Ok((kind, args.get(1).cloned()));
    }
    if args.len() > 1 {
        anyhow::bail!("usage: a3s code agent {command} [kind] [package]");
    }
    Ok((default, Some(first.clone())))
}

fn parse_mcp_action(command: &str) -> anyhow::Result<panels::mcp::McpOsAction> {
    let parsed = panels::mcp::parse_mcp_subcommand(command)
        .ok_or_else(|| anyhow::anyhow!("unknown MCP action `{command}`"))?
        .map_err(anyhow::Error::msg)?;
    parsed
        .os_action()
        .ok_or_else(|| anyhow::anyhow!("`{command}` is not an OS MCP action"))
}

fn parse_skill_action(command: &str) -> anyhow::Result<panels::skill::SkillOsAction> {
    let parsed = panels::skill::parse_skill_subcommand(command)
        .ok_or_else(|| anyhow::anyhow!("unknown skill action `{command}`"))?
        .map_err(anyhow::Error::msg)?;
    match parsed {
        panels::skill::SkillSubcommand::Publish => Ok(panels::skill::SkillOsAction::Publish),
        panels::skill::SkillSubcommand::Deploy => Ok(panels::skill::SkillOsAction::Deploy),
        panels::skill::SkillSubcommand::Open => Ok(panels::skill::SkillOsAction::Open),
        panels::skill::SkillSubcommand::Status => Ok(panels::skill::SkillOsAction::Status),
        _ => anyhow::bail!("`{command}` is not an OS skill action"),
    }
}

fn parse_flow_action(command: &str) -> anyhow::Result<panels::flow::FlowOsAction> {
    let parsed = panels::flow::parse_flow_subcommand(command)
        .ok_or_else(|| anyhow::anyhow!("unknown flow action `{command}`"))?
        .map_err(anyhow::Error::msg)?;
    match parsed {
        panels::flow::FlowSubcommand::Publish => Ok(panels::flow::FlowOsAction::Publish),
        panels::flow::FlowSubcommand::Run => Ok(panels::flow::FlowOsAction::Run),
        panels::flow::FlowSubcommand::Deploy => Ok(panels::flow::FlowOsAction::Deploy),
        panels::flow::FlowSubcommand::Open => Ok(panels::flow::FlowOsAction::Open),
        panels::flow::FlowSubcommand::Logs => Ok(panels::flow::FlowOsAction::Logs),
        panels::flow::FlowSubcommand::Status => Ok(panels::flow::FlowOsAction::Status),
        _ => anyhow::bail!("`{command}` is not an OS flow action"),
    }
}

fn parse_okf_action(command: &str) -> anyhow::Result<panels::okf::OkfOsAction> {
    match panels::okf::parse_okf_command(command) {
        panels::okf::OkfCommand::Publish => Ok(panels::okf::OkfOsAction::Publish),
        panels::okf::OkfCommand::Deploy => Ok(panels::okf::OkfOsAction::Deploy),
        panels::okf::OkfCommand::Status => Ok(panels::okf::OkfOsAction::Status),
        panels::okf::OkfCommand::Usage(usage) => anyhow::bail!("{usage}"),
        _ => anyhow::bail!("`{command}` is not an OS OKF action"),
    }
}

fn is_agent_kind(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "agentic" | "application" | "tool"
    )
}

fn single_path_arg(command: &str, args: &[String]) -> anyhow::Result<Option<String>> {
    match args {
        [] => Ok(None),
        [path] => Ok(Some(path.clone())),
        _ => anyhow::bail!("usage: a3s code {command} [path]"),
    }
}

fn ensure_no_more_than_query(_command: &str, _args: &[String]) -> anyhow::Result<()> {
    Ok(())
}

fn join_args(args: &[String]) -> String {
    args.join(" ")
}

fn os_asset_category_query(category: &str, query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        format!("category:{category}")
    } else {
        format!("category:{category} {query}")
    }
}

fn runtime_asset_query(category: &str, asset_hint: &str, query: &str) -> String {
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

fn print_local_agents(query: &str) {
    let root = config::agent_dir();
    let rows = panels::agent::list_agents(&root)
        .into_iter()
        .filter(|row| matches_query(&row.rel, query))
        .collect::<Vec<_>>();
    println!(
        "{} local agent package(s) in {}",
        rows.len(),
        root.display()
    );
    for row in rows {
        println!(
            "{}\t{}\t{}",
            row.rel,
            row.definition_rel,
            row.path.display()
        );
    }
}

fn print_local_mcps(query: &str) {
    let root = config::mcp_dir();
    let rows = panels::mcp::list_mcp_projects(&root)
        .into_iter()
        .filter(|row| matches_query(&format!("{} {}", row.rel, row.name), query))
        .collect::<Vec<_>>();
    println!("{} local MCP asset(s) in {}", rows.len(), root.display());
    for row in rows {
        println!("{}\t{}\t{}", row.rel, row.name, row.path.display());
    }
}

fn print_local_skills(query: &str) {
    let root = config::skill_dir();
    let rows = panels::skill::list_skill_assets(&root)
        .into_iter()
        .filter(|row| matches_query(&format!("{} {}", row.rel, row.name), query))
        .collect::<Vec<_>>();
    println!("{} local skill asset(s) in {}", rows.len(), root.display());
    for row in rows {
        println!("{}\t{}\t{}", row.rel, row.name, row.path.display());
    }
}

fn print_local_flows(query: &str) {
    let root = config::flow_dir();
    let rows = panels::flow::list_flows(&root)
        .into_iter()
        .filter(|row| matches_query(row, query))
        .collect::<Vec<_>>();
    println!(
        "{} local workflow file(s) in {}",
        rows.len(),
        root.display()
    );
    for row in rows {
        println!("{}\t{}", row, root.join(&row).display());
    }
}

fn print_local_okf(query: &str) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let root = panels::okf::okf_package_dir(&cwd.to_string_lossy());
    let rows = panels::okf::list_okf_packages(&root)
        .into_iter()
        .filter(|row| matches_query(&format!("{} {}", row.rel, row.name), query))
        .collect::<Vec<_>>();
    println!("{} local OKF package(s) in {}", rows.len(), root.display());
    for row in rows {
        println!("{}\t{}\t{}", row.rel, row.name, row.path.display());
    }
    Ok(())
}

fn matches_query(text: &str, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    query.is_empty() || text.to_ascii_lowercase().contains(&query)
}

fn print_os_note(note: &str, view: &remote_ui::ViewSpec) {
    println!("{note}");
    println!("view: {}", view.url);
    if let Some(width) = view.width {
        print!("width: {width}");
        if let Some(height) = view.height {
            print!(", height: {height}");
        }
        println!();
    }
}

fn open_if_requested(open_requested: bool, view: &remote_ui::ViewSpec) -> anyhow::Result<()> {
    if !open_requested {
        return Ok(());
    }
    let opened = remote_ui::open_window(view)
        .map_err(|e| anyhow::anyhow!("could not open RemoteUI view: {e}"))?;
    println!("opened: {:?}", opened);
    Ok(())
}

fn trim_col(value: &str, width: usize) -> String {
    let mut out = value.chars().take(width).collect::<String>();
    if value.chars().count() > width && width >= 1 {
        out.pop();
        out.push('~');
    }
    out
}

fn print_family_help(family: &str) {
    println!("a3s code {family}");
    println!("  local [query]");
    println!("  clone <git-url>");
    println!("  list [query]");
    println!("  activity [query]");
    println!("  review [path]");
    match family {
        "agent" => {
            println!("  publish agentic|application|tool [package]");
            println!("  run|deploy|open|logs|status [agentic|application|tool] [package]");
        }
        "mcp" => println!("  publish|run|test|deploy|open|logs|status [path]"),
        "skill" => println!("  publish|deploy|open|status [path]"),
        "flow" => println!("  publish|run|deploy|open|logs|status [file]"),
        "okf" => println!("  publish|deploy|status [path]"),
        _ => {}
    }
}

fn unknown_family_command(family: &str, command: &str) -> anyhow::Result<()> {
    anyhow::bail!("unknown a3s code {family} command `{command}`; run `a3s code {family} --help`")
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_code_core::llm::{
        ContentBlock, LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
    };
    use a3s_code_core::tools::{Tool, ToolContext, ToolOutput};
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    struct ScriptedLlmClient {
        responses: Mutex<VecDeque<LlmResponse>>,
    }

    #[async_trait]
    impl LlmClient for ScriptedLlmClient {
        async fn complete(
            &self,
            messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(self.response_for_messages(messages, system, tools))
        }

        async fn complete_streaming(
            &self,
            messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
            _cancel_token: CancellationToken,
        ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
            let response = self.response_for_messages(messages, system, tools);
            let (tx, rx) = mpsc::channel(1);
            tokio::spawn(async move {
                let _ = tx.send(StreamEvent::Done(response)).await;
            });
            Ok(rx)
        }
    }

    impl ScriptedLlmClient {
        fn new(responses: Vec<LlmResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
        }

        fn response_for_messages(
            &self,
            messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
        ) -> LlmResponse {
            if tools.iter().any(|tool| tool.name == "emit_step_output") {
                return tool_call_response(
                    "toolu_emit_step_output",
                    "emit_step_output",
                    serde_json::json!({
                        "summary": "Structured DeepResearch track evidence confirms local fan-out completed before synthesis.",
                        "sources": [{
                            "title": "Example research source",
                            "url_or_path": "https://example.com/research",
                            "date": "2026-07-08",
                            "quote_or_fact": "Local DeepResearch fan-out completed before synthesis.",
                            "reliability": "deterministic test evidence"
                        }],
                        "key_evidence": [
                            "Local parallel_task fan-out produced deterministic evidence."
                        ],
                        "contradictions": [],
                        "confidence": "high for deterministic test evidence",
                        "gaps": []
                    }),
                );
            }
            let last = message_text(messages.last());
            if system.is_some_and(|system| system.contains("pre-analysis assistant"))
                || last.contains("ONLY the JSON object")
            {
                return text_response(
                    r#"{"intent":"GeneralPurpose","requires_planning":false,"goal":{"description":"DeepResearch child task","success_criteria":["evidence returned"]},"execution_plan":{"complexity":"Simple","steps":[],"required_tools":[]},"optimized_input":"DeepResearch child task"}"#,
                );
            }
            if last.contains("Deep-research track for:")
                && !last.contains("DynamicWorkflowRuntime")
                && !last.contains("DeepResearch verification layer")
            {
                return text_response(
                    "Track evidence: https://example.com/research confirms the local \
                     DeepResearch fan-out completed before synthesis.",
                );
            }
            self.next_response()
        }

        fn next_response(&self) -> LlmResponse {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| text_response("DONE"))
        }
    }

    fn message_text(message: Option<&Message>) -> String {
        message
            .map(|message| {
                message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    fn text_response(text: impl Into<String>) -> LlmResponse {
        LlmResponse {
            message: Message {
                role: "assistant".into(),
                content: vec![ContentBlock::Text { text: text.into() }],
                reasoning_content: None,
            },
            usage: TokenUsage::default(),
            stop_reason: Some("stop".into()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }

    fn tool_call_response(id: &str, name: &str, input: serde_json::Value) -> LlmResponse {
        LlmResponse {
            message: Message {
                role: "assistant".into(),
                content: vec![ContentBlock::ToolUse {
                    id: id.into(),
                    name: name.into(),
                    input,
                }],
                reasoning_content: None,
            },
            usage: TokenUsage::default(),
            stop_reason: Some("tool_use".into()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }

    struct StructuredRuntimeTool {
        seen_args: std::sync::Arc<Mutex<Vec<serde_json::Value>>>,
    }

    #[async_trait]
    impl Tool for StructuredRuntimeTool {
        fn name(&self) -> &str {
            "runtime"
        }

        fn description(&self) -> &str {
            "Returns completed structured runtime output for DeepResearch tests."
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }

        async fn execute(
            &self,
            args: &serde_json::Value,
            _ctx: &ToolContext,
        ) -> anyhow::Result<ToolOutput> {
            self.seen_args.lock().unwrap().push(args.clone());
            let structured = serde_json::json!({
                "summary": "Runtime structured evidence confirms OS fan-out completed before synthesis.",
                "sources": [{
                    "title": "Runtime Evidence",
                    "url_or_path": "https://example.com/runtime-evidence",
                    "date": "2026-07-08",
                    "quote_or_fact": "OS Runtime returned a schema-shaped evidence object.",
                    "reliability": "deterministic test fixture"
                }],
                "key_evidence": ["OS Runtime results are normalized into structured evidence."],
                "contradictions": [],
                "confidence": "high",
                "gaps": []
            });
            Ok(ToolOutput::success(
                serde_json::json!({
                    "batchId": "batch-structured",
                    "results": [{
                        "invocationId": "inv-1",
                        "state": "completed",
                        "output": structured.to_string(),
                        "error": null
                    }]
                })
                .to_string(),
            ))
        }
    }

    fn test_config(path: &std::path::Path) {
        std::fs::write(
            path,
            "default_model = \"openai/x\"\n\
             providers \"openai\" {\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
             models \"x\" { name = \"x\" }\n}\n\
             memory {\n  llmExtraction = false\n}\n",
        )
        .unwrap();
    }

    #[test]
    fn recognizes_code_cli_top_level_commands_without_capturing_prompts() {
        assert!(is_code_cli_command(&["agent".into()]));
        assert!(is_code_cli_command(&["mcp".into()]));
        assert!(is_code_cli_command(&["login".into()]));
        assert!(is_code_cli_command(&["config".into()]));
        assert!(is_code_cli_command(&["kb".into()]));
        assert!(is_code_cli_command(&["ctx".into()]));
        assert!(is_code_cli_command(&["memory".into()]));
        assert!(is_code_cli_command(&["top".into()]));
        assert!(is_code_cli_command(&["deepresearch".into()]));
        assert!(is_code_cli_command(&["deep-research".into()]));
        assert!(is_code_cli_command(&["--help".into()]));
        assert!(!is_code_cli_command(&["view".into()]));
        assert!(!is_code_cli_command(&["research".into(), "this".into()]));
        assert!(!is_code_cli_command(&["resume".into(), "abc".into()]));
        assert!(!is_code_cli_command(&["some-prompt".into()]));
    }

    #[test]
    fn usage_mentions_noninteractive_asset_commands() {
        let text = code_cli_usage_text();
        assert!(text.contains("a3s code <family> local"));
        assert!(text.contains("a3s code agent publish agentic|application|tool"));
        assert!(text.contains("a3s code mcp publish|run|test|deploy"));
        assert!(text.contains("families: agent, mcp, skill, flow, okf"));
        assert!(text.contains("a3s code login [token]"));
        assert!(text.contains("a3s code deepresearch <query>"));
        assert!(text.contains("a3s code kb stats|add|import|search|vault"));
        assert!(text.contains("a3s code ctx search <query>"));
        assert!(!text.contains("a3s code view <url>"));
    }

    #[test]
    fn parses_deepresearch_cli_options() {
        let opts = parse_deepresearch_args(&["--local".into(), "rust".into(), "async".into()])
            .expect("local deepresearch args");
        assert_eq!(opts.query, "rust async");
        assert_eq!(opts.runtime_mode, DeepResearchRuntimeMode::Local);

        let opts = parse_deepresearch_args(&["--os".into(), "market".into()])
            .expect("os deepresearch args");
        assert_eq!(opts.query, "market");
        assert_eq!(opts.runtime_mode, DeepResearchRuntimeMode::Os);

        let opts = parse_deepresearch_args(&["compare".into(), "runtimes".into()])
            .expect("auto deepresearch args");
        assert_eq!(opts.query, "compare runtimes");
        assert_eq!(opts.runtime_mode, DeepResearchRuntimeMode::Auto);
    }

    #[tokio::test]
    async fn deepresearch_cli_os_mode_is_temporarily_disabled() {
        let err = run_deepresearch(&["--os".into(), "market".into()])
            .await
            .expect_err("--os should be disabled before touching OS Runtime");
        let message = err.to_string();
        assert!(message.contains("temporarily disabled"), "{message}");
        assert!(message.contains("Function-as-a-Service"), "{message}");
    }

    #[test]
    fn deepresearch_cli_policy_only_allows_report_artifact_writes() {
        use a3s_code_core::permissions::PermissionDecision;

        let policy = deepresearch_cli_permission_policy();

        assert_eq!(
            policy.check(
                "write",
                &serde_json::json!({
                    "file_path": ".a3s/research/local-test/report.md",
                    "content": "# Report"
                })
            ),
            PermissionDecision::Allow
        );
        assert_eq!(
            policy.check(
                "Write",
                &serde_json::json!({
                    "file_path": ".a3s/research/local-test/index.html",
                    "content": "<!doctype html><html><body></body></html>"
                })
            ),
            PermissionDecision::Allow
        );
        assert_eq!(
            policy.check("web_search", &serde_json::json!({"query": "a3s"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            policy.check("bash", &serde_json::json!({"command": "ls -la"})),
            PermissionDecision::Deny
        );
        assert_eq!(
            policy.check(
                "write",
                &serde_json::json!({"file_path": "README.md", "content": "oops"})
            ),
            PermissionDecision::Deny
        );
        assert_eq!(
            policy.check(
                "write",
                &serde_json::json!({
                    "file_path": "/tmp/workspace/.a3s/research/local-test/index.html",
                    "content": "ambiguous absolute path"
                })
            ),
            PermissionDecision::Deny
        );
        assert_eq!(
            policy.check(
                "write",
                &serde_json::json!({
                    "file_path": ".a3s/research/local-test/../../README.md",
                    "content": "path traversal"
                })
            ),
            PermissionDecision::Deny
        );
        assert_eq!(
            policy.check(
                "edit",
                &serde_json::json!({
                    "file_path": ".a3s/research/local-test/..\\..\\README.md",
                    "old_string": "before",
                    "new_string": "after"
                })
            ),
            PermissionDecision::Deny
        );
        assert_eq!(
            policy.check("bash", &serde_json::json!({"command": "rm -rf target"})),
            PermissionDecision::Deny
        );
    }

    #[tokio::test]
    async fn deepresearch_cli_synthesis_denies_non_report_writes_before_fallback() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-cli-denied-write-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let cfg = workspace.join("config.acl");
        test_config(&cfg);
        let agent = Agent::new(cfg.to_string_lossy().to_string()).await.unwrap();
        let llm = Arc::new(ScriptedLlmClient::new(vec![
            tool_call_response(
                "toolu_write_readme",
                "write",
                serde_json::json!({
                    "file_path": "README.md",
                    "content": "DeepResearch should not write ordinary workspace files.",
                }),
            ),
            text_response("Synthesis recovered after a denied workspace write but did not write report files."),
            text_response("Repair also did not write report files."),
        ]));
        let opts = SessionOptions::new()
            .with_llm_client(llm)
            .with_permission_policy(deepresearch_cli_permission_policy())
            .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
            .with_max_tool_rounds(4);
        let session = agent
            .session(workspace.to_string_lossy().to_string(), Some(opts))
            .unwrap();

        let synthesis = synthesize_deepresearch_report(
            &session,
            &workspace,
            "denied write fallback",
            false,
            r#"{"mode":"local_parallel_task","research":"evidence after denied write"}"#,
            0,
            None,
        )
        .await
        .expect("host fallback should materialize after denied non-report write");
        let DeepResearchReportSynthesis {
            text: final_text,
            artifacts,
            status,
        } = synthesis;

        assert_eq!(status, DeepResearchReportStatus::FallbackDraft);
        assert!(
            !workspace.join("README.md").exists(),
            "DeepResearch CLI policy must block non-report writes"
        );
        assert!(
            final_text.contains("DeepResearch fallback draft written at"),
            "{final_text}"
        );
        assert!(!final_text.contains("A3S_RESEARCH_VIEW"), "{final_text}");
        assert_eq!(
            artifacts.markdown,
            workspace
                .join(".a3s/research/denied-write-fallback/report.md")
                .canonicalize()
                .unwrap()
        );
        assert_eq!(
            artifacts.html,
            workspace
                .join(".a3s/research/denied-write-fallback/index.html")
                .canonicalize()
                .unwrap()
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn deepresearch_cli_repair_pass_writes_required_markdown_and_html_artifacts() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-cli-artifacts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let cfg = workspace.join("config.acl");
        test_config(&cfg);
        let agent = Agent::new(cfg.to_string_lossy().to_string()).await.unwrap();
        let llm = Arc::new(ScriptedLlmClient::new(vec![
            text_response("Initial synthesis without a report marker."),
            tool_call_response(
                "toolu_write_markdown",
                "write",
                serde_json::json!({
                    "file_path": ".a3s/research/local-test/report.md",
                    "content": "# Local Test\n\n## Findings\n\nThis source-backed markdown report summarizes the gathered DeepResearch evidence, explains the main finding, and records caveats for review.\n\n## Sources\n\n- https://example.com/research\n\n## Confidence\n\nConfidence is medium because this deterministic test evidence is compact but traceable.\n",
                }),
            ),
            tool_call_response(
                "toolu_write_html",
                "write",
                serde_json::json!({
                    "file_path": ".a3s/research/local-test/index.html",
                    "content": "<!doctype html><html><body><h1>Local Test</h1><section><h2>Findings</h2><p>This source-backed report summarizes gathered DeepResearch evidence, caveats, and the main finding for review.</p></section><section><h2>Sources</h2><p>Evidence source: https://example.com/research. Confidence is medium.</p></section></body></html>",
                }),
            ),
            text_response(
                "Repair complete.\nA3S_RESEARCH_VIEW: .a3s/research/local-test/index.html",
            ),
        ]));
        let opts = SessionOptions::new()
            .with_llm_client(llm)
            .with_permission_policy(deepresearch_cli_permission_policy())
            .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
            .with_max_tool_rounds(6);
        let session = agent
            .session(workspace.to_string_lossy().to_string(), Some(opts))
            .unwrap();

        let synthesis = synthesize_deepresearch_report(
            &session,
            &workspace,
            "local test",
            false,
            r#"{"mode":"local_parallel_task","research":"evidence"}"#,
            0,
            None,
        )
        .await
        .unwrap_or_else(|error| {
            let markdown = workspace.join(".a3s/research/local-test/report.md");
            let html = workspace.join(".a3s/research/local-test/index.html");
            panic!(
                "{error}; markdown_exists={}; html_exists={}",
                markdown.exists(),
                html.exists()
            )
        });
        let DeepResearchReportSynthesis {
            text: final_text,
            artifacts,
            status,
        } = synthesis;

        assert_eq!(status, DeepResearchReportStatus::Completed);
        assert!(
            final_text.contains("A3S_RESEARCH_VIEW: .a3s/research/local-test/index.html"),
            "{final_text}"
        );
        assert_eq!(
            artifacts.markdown,
            workspace
                .join(".a3s/research/local-test/report.md")
                .canonicalize()
                .unwrap()
        );
        assert_eq!(
            artifacts.html,
            workspace
                .join(".a3s/research/local-test/index.html")
                .canonicalize()
                .unwrap()
        );
        assert!(std::fs::metadata(&artifacts.markdown).unwrap().len() > 0);
        assert!(std::fs::metadata(&artifacts.html).unwrap().len() > 0);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn deepresearch_cli_materializes_fallback_artifacts_when_model_never_writes_report() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-cli-fallback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let cfg = workspace.join("config.acl");
        test_config(&cfg);
        let agent = Agent::new(cfg.to_string_lossy().to_string()).await.unwrap();
        let llm = Arc::new(ScriptedLlmClient::new(vec![
            text_response("Initial synthesis without report files."),
            text_response("Repair also forgot to write the report files."),
        ]));
        let opts = SessionOptions::new()
            .with_llm_client(llm)
            .with_planning_mode(a3s_code_core::PlanningMode::Disabled);
        let session = agent
            .session(workspace.to_string_lossy().to_string(), Some(opts))
            .unwrap();

        let synthesis = synthesize_deepresearch_report(
            &session,
            &workspace,
            "fallback only",
            false,
            r#"{"mode":"local_parallel_task","research":"fallback evidence"}"#,
            0,
            None,
        )
        .await
        .expect("host fallback should materialize draft artifacts");
        let DeepResearchReportSynthesis {
            text: final_text,
            artifacts,
            status,
        } = synthesis;

        assert_eq!(status, DeepResearchReportStatus::FallbackDraft);
        assert!(
            final_text.contains("DeepResearch fallback draft written at"),
            "{final_text}"
        );
        assert!(!final_text.contains("A3S_RESEARCH_VIEW"), "{final_text}");
        assert_eq!(
            artifacts.markdown,
            workspace
                .join(".a3s/research/fallback-only/report.md")
                .canonicalize()
                .unwrap()
        );
        assert_eq!(
            artifacts.html,
            workspace
                .join(".a3s/research/fallback-only/index.html")
                .canonicalize()
                .unwrap()
        );
        let markdown = std::fs::read_to_string(&artifacts.markdown).unwrap();
        assert!(markdown.contains("Repair also forgot"));
        assert!(markdown.contains("fallback evidence"));
        assert!(markdown.contains("DeepResearch Fallback Draft"));
        assert!(!markdown.contains("A3S_RESEARCH_VIEW"));
        let html = std::fs::read_to_string(&artifacts.html).unwrap();
        assert!(html.contains("DeepResearch Fallback Draft"));
        assert!(html.contains("fallback evidence"));
        assert!(!html.contains("A3S_RESEARCH_VIEW"));

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn deepresearch_cli_local_workflow_to_report_artifacts_e2e() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-cli-e2e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let cfg = workspace.join("config.acl");
        test_config(&cfg);
        let agent = Agent::new(cfg.to_string_lossy().to_string()).await.unwrap();
        let llm = Arc::new(ScriptedLlmClient::new(vec![
            text_response("Initial synthesis without a report marker."),
            tool_call_response(
                "toolu_write_markdown",
                "write",
                serde_json::json!({
                    "file_path": ".a3s/research/local-workflow-e2e/report.md",
                    "content": "# Local Workflow E2E\n\n## Findings\n\nThe workflow produced deterministic evidence and completed fan-out before synthesis, giving the report enough source-backed material to explain the result.\n\n## Sources\n\n- https://example.com/local-workflow-e2e\n\n## Confidence\n\nConfidence is high for this test because the evidence path is deterministic and verified by workflow metadata.\n",
                }),
            ),
            tool_call_response(
                "toolu_write_html",
                "write",
                serde_json::json!({
                    "file_path": ".a3s/research/local-workflow-e2e/index.html",
                    "content": "<!doctype html><html><body><h1>Local Workflow E2E</h1><section><h2>Findings</h2><p>The workflow produced deterministic evidence and completed fan-out before synthesis.</p></section><section><h2>Sources</h2><p>Evidence source: https://example.com/local-workflow-e2e. Confidence is high for this deterministic test.</p></section></body></html>",
                }),
            ),
            text_response(
                "Report complete.\nA3S_RESEARCH_VIEW: .a3s/research/local-workflow-e2e/index.html",
            ),
        ]));
        let opts = SessionOptions::new()
            .with_llm_client(llm)
            .with_permission_policy(deepresearch_cli_permission_policy())
            .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
            .with_max_tool_rounds(6);
        let session = agent
            .session(workspace.to_string_lossy().to_string(), Some(opts))
            .unwrap();
        session.register_dynamic_workflow_runtime();

        let mut workflow_args =
            super::super::deep_research_workflow_args("local workflow e2e", false);
        workflow_args["input"]["tracks"] = serde_json::json!([
            {
                "title": "Local evidence",
                "focus": "Inspect local workflow evidence for the report."
            },
            {
                "title": "Source confidence",
                "focus": "Check source confidence and caveats independently."
            },
            {
                "title": "Sequential synthesis",
                "focus": "This should not run as a parallel child.",
                "parallelizable": false
            }
        ]);
        let workflow = run_deepresearch_workflow(&session, workflow_args)
            .await
            .expect("local DeepResearch workflow should complete");
        assert_eq!(workflow.exit_code, 0, "{}", workflow.output);
        assert!(
            workflow.output.contains("local_parallel_task"),
            "{}",
            workflow.output
        );
        let metadata = workflow.metadata.as_ref().expect("workflow metadata");
        assert_eq!(metadata["dynamic_workflow"]["status"], "Completed");
        assert_eq!(
            metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["status"],
            "completed"
        );
        assert_eq!(
            metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]["tool"],
            "parallel_task"
        );
        assert_eq!(
            metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]
                ["metadata"]["task_count"],
            serde_json::json!(2)
        );
        assert_eq!(
            metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]
                ["metadata"]["result_count"],
            serde_json::json!(2)
        );
        assert_eq!(
            metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]
                ["metadata"]["results"][0]["structured"]["summary"],
            "Structured DeepResearch track evidence confirms local fan-out completed before synthesis."
        );

        let synthesis = synthesize_deepresearch_report(
            &session,
            &workspace,
            "local workflow e2e",
            false,
            &workflow.output,
            workflow.exit_code,
            workflow.metadata.as_ref(),
        )
        .await
        .unwrap();
        let DeepResearchReportSynthesis {
            text: final_text,
            artifacts,
            status,
        } = synthesis;

        assert_eq!(status, DeepResearchReportStatus::Completed);
        assert!(
            final_text.contains("A3S_RESEARCH_VIEW: .a3s/research/local-workflow-e2e/index.html"),
            "{final_text}"
        );
        assert_eq!(
            artifacts.markdown,
            workspace
                .join(".a3s/research/local-workflow-e2e/report.md")
                .canonicalize()
                .unwrap()
        );
        assert_eq!(
            artifacts.html,
            workspace
                .join(".a3s/research/local-workflow-e2e/index.html")
                .canonicalize()
                .unwrap()
        );
        assert!(std::fs::read_to_string(&artifacts.markdown)
            .unwrap()
            .contains("workflow produced deterministic evidence"));
        assert!(std::fs::read_to_string(&artifacts.html)
            .unwrap()
            .contains("Local Workflow E2E"));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn deepresearch_workflow_forces_local_when_os_runtime_requested() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-cli-runtime-disabled-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let cfg = workspace.join("config.acl");
        test_config(&cfg);
        let agent = Agent::new(cfg.to_string_lossy().to_string()).await.unwrap();
        let llm = Arc::new(ScriptedLlmClient::new(vec![]));
        let opts = SessionOptions::new()
            .with_llm_client(llm)
            .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
            .with_max_tool_rounds(4);
        let session = agent
            .session(workspace.to_string_lossy().to_string(), Some(opts))
            .unwrap();
        session.register_dynamic_workflow_runtime();
        let seen_args = std::sync::Arc::new(Mutex::new(Vec::new()));
        session.register_dynamic_tool(Arc::new(StructuredRuntimeTool {
            seen_args: std::sync::Arc::clone(&seen_args),
        }));

        let args = super::super::deep_research_workflow_args("runtime disabled", true);
        let budget = super::super::deep_research_default_budget();
        assert_eq!(args["input"]["os_runtime"], false);
        assert_eq!(args["allowed_tools"], serde_json::json!([]));
        assert_eq!(
            args["input"]["local_max_parallel_tasks"],
            serde_json::json!(budget.max_parallel_tasks)
        );
        let workflow = run_deepresearch_workflow(&session, args)
            .await
            .expect("DeepResearch workflow should stay local even if runtime was requested");

        assert_eq!(workflow.exit_code, 0, "{}", workflow.output);
        let output: serde_json::Value =
            serde_json::from_str(&workflow.output).expect("workflow output should be JSON");
        assert_eq!(output["mode"], "local_parallel_task");
        assert_eq!(
            output["research"]["metadata"]["results"][0]["structured"]["summary"],
            "Structured DeepResearch track evidence confirms local fan-out completed before synthesis."
        );
        assert_eq!(
            seen_args.lock().unwrap().len(),
            0,
            "DeepResearch must not call the OS Runtime tool-call fan-out path"
        );
        let metadata = workflow.metadata.as_ref().expect("workflow metadata");
        assert_eq!(
            metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["status"],
            "completed"
        );
        assert_eq!(
            metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]
                ["metadata"]["task_count"],
            serde_json::json!(4)
        );
        assert_eq!(
            metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]
                ["metadata"]["result_count"],
            serde_json::json!(4)
        );
        assert!(
            metadata["dynamic_workflow"]["snapshot"]["steps"]
                .get("runtime_preflight")
                .is_none()
                && metadata["dynamic_workflow"]["snapshot"]["steps"]
                    .get("runtime_research")
                    .is_none(),
            "runtime tool-call fan-out steps should not be scheduled"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn parses_agent_kind_and_path_without_losing_default_kind() {
        let (kind, path) = parse_agent_kind_path("open", &[]).unwrap();
        assert_eq!(kind, panels::agent::AgentOsKind::Agentic);
        assert_eq!(path, None);

        let (kind, path) =
            parse_agent_kind_path("open", &["application".into()]).expect("kind without path");
        assert_eq!(kind, panels::agent::AgentOsKind::Application);
        assert_eq!(path, None);

        let (kind, path) = parse_agent_kind_path("open", &["tool".into(), "agents/tooler".into()])
            .expect("kind plus path");
        assert_eq!(kind, panels::agent::AgentOsKind::Tool);
        assert_eq!(path.as_deref(), Some("agents/tooler"));

        let (kind, path) = parse_agent_kind_path("open", &["agents/reviewer".into()])
            .expect("path only uses default kind");
        assert_eq!(kind, panels::agent::AgentOsKind::Agentic);
        assert_eq!(path.as_deref(), Some("agents/reviewer"));

        assert!(
            parse_agent_kind_path("open", &["unknown-kind".into(), "agents/reviewer".into()])
                .is_err()
        );
        assert!(parse_agent_kind_path(
            "open",
            &["agentic".into(), "agents/reviewer".into(), "extra".into()]
        )
        .is_err());
    }

    #[test]
    fn parses_agent_lifecycle_publish_and_path_shapes() {
        let (kind, path) = parse_agent_publish_args(&["agentic".into()]).unwrap();
        assert_eq!(kind, panels::agent::AgentOsKind::Agentic);
        assert_eq!(path, None);

        let (kind, path) =
            parse_agent_publish_args(&["application".into(), "agents/portal".into()]).unwrap();
        assert_eq!(kind, panels::agent::AgentOsKind::Application);
        assert_eq!(path.as_deref(), Some("agents/portal"));

        let (kind, path) =
            parse_agent_publish_args(&["tool".into(), "agents/sql-checker".into()]).unwrap();
        assert_eq!(kind, panels::agent::AgentOsKind::Tool);
        assert_eq!(path.as_deref(), Some("agents/sql-checker"));

        assert!(parse_agent_publish_args(&[]).is_err());
        assert!(parse_agent_publish_args(&["agents/reviewer".into()]).is_err());
        assert!(parse_agent_publish_args(&["service".into()]).is_err());
        assert!(parse_agent_publish_args(&[
            "agentic".into(),
            "agents/reviewer".into(),
            "extra".into()
        ])
        .is_err());

        let (run_kind, run_path) = parse_agent_kind_path("run", &[]).unwrap();
        assert_eq!(run_kind, panels::agent::AgentOsKind::Agentic);
        assert_eq!(run_path, None);

        let (run_kind, run_path) =
            parse_agent_kind_path("run", &["tool".into(), "agents/sql-checker".into()]).unwrap();
        assert_eq!(run_kind, panels::agent::AgentOsKind::Tool);
        assert_eq!(run_path.as_deref(), Some("agents/sql-checker"));

        assert_eq!(
            parse_agent_kind_path("run", &["agents/reviewer".into()])
                .unwrap()
                .1
                .as_deref(),
            Some("agents/reviewer")
        );
        assert!(parse_agent_kind_path("run", &["agents/reviewer".into(), "extra".into()]).is_err());
        assert!(
            single_path_arg("agent deploy", &["agents/portal".into(), "extra".into()]).is_err()
        );
    }

    #[test]
    fn parses_lifecycle_asset_cli_actions_to_os_actions() {
        let mcp = [
            ("publish", panels::mcp::McpOsAction::Publish),
            ("run", panels::mcp::McpOsAction::Run),
            ("deploy", panels::mcp::McpOsAction::Deploy),
            ("test", panels::mcp::McpOsAction::Test),
            ("open", panels::mcp::McpOsAction::Open),
            ("logs", panels::mcp::McpOsAction::Logs),
            ("status", panels::mcp::McpOsAction::Status),
        ];
        for (command, expected) in mcp {
            assert_eq!(parse_mcp_action(command).unwrap(), expected, "{command}");
        }
        for command in ["debug", "invoke", "batch", "activity", "review"] {
            assert!(parse_mcp_action(command).is_err(), "{command}");
        }

        let skill = [
            ("publish", panels::skill::SkillOsAction::Publish),
            ("deploy", panels::skill::SkillOsAction::Deploy),
            ("open", panels::skill::SkillOsAction::Open),
            ("status", panels::skill::SkillOsAction::Status),
        ];
        for (command, expected) in skill {
            assert_eq!(parse_skill_action(command).unwrap(), expected, "{command}");
        }
        for command in ["run", "debug", "test", "logs", "activity", "review"] {
            assert!(parse_skill_action(command).is_err(), "{command}");
        }

        let flow = [
            ("publish", panels::flow::FlowOsAction::Publish),
            ("run", panels::flow::FlowOsAction::Run),
            ("deploy", panels::flow::FlowOsAction::Deploy),
            ("open", panels::flow::FlowOsAction::Open),
            ("logs", panels::flow::FlowOsAction::Logs),
            ("status", panels::flow::FlowOsAction::Status),
        ];
        for (command, expected) in flow {
            assert_eq!(parse_flow_action(command).unwrap(), expected, "{command}");
        }
        for command in ["debug", "test", "activity", "review", "view"] {
            assert!(parse_flow_action(command).is_err(), "{command}");
        }

        let okf = [
            ("publish", panels::okf::OkfOsAction::Publish),
            ("deploy", panels::okf::OkfOsAction::Deploy),
            ("status", panels::okf::OkfOsAction::Status),
        ];
        for (command, expected) in okf {
            assert_eq!(parse_okf_action(command).unwrap(), expected, "{command}");
        }
        for command in ["run", "debug", "test", "open", "logs", "activity", "review"] {
            assert!(parse_okf_action(command).is_err(), "{command}");
        }
    }

    #[test]
    fn usage_lists_all_asset_lifecycle_command_families() {
        let text = code_cli_usage_text();
        for line in [
            "a3s code agent publish agentic|application|tool [package]",
            "a3s code agent run|deploy|open|logs|status [kind] [package]",
            "a3s code mcp publish|run|test|deploy|open|logs|status [path]",
            "a3s code skill publish|deploy|open|status [path]",
            "a3s code flow publish|run|deploy|open|logs|status [file]",
            "a3s code okf publish|deploy|status [path]",
        ] {
            assert!(text.contains(line), "missing usage line: {line}\n{text}");
        }
    }

    #[tokio::test]
    async fn code_cli_asset_lifecycle_commands_use_os_api_from_cli_entrypoint() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_cli_lifecycle_os_mock(captured.clone()).await;
        let root = temp_dir("code-cli-lifecycle-os");
        let env = CliLifecycleEnv::new(&root, &origin);

        run_code_cli(vec![
            "agent".into(),
            "publish".into(),
            "agentic".into(),
            env.agent_package.display().to_string(),
        ])
        .await
        .expect("agent publish should run through CLI entrypoint");
        run_code_cli(vec![
            "mcp".into(),
            "publish".into(),
            env.mcp_package.display().to_string(),
        ])
        .await
        .expect("mcp publish should run through CLI entrypoint");
        run_code_cli(vec![
            "skill".into(),
            "publish".into(),
            env.skill_package.display().to_string(),
        ])
        .await
        .expect("skill publish should run through CLI entrypoint");
        run_code_cli(vec![
            "flow".into(),
            "publish".into(),
            env.flow_file.display().to_string(),
        ])
        .await
        .expect("flow publish should run through CLI entrypoint");
        run_code_cli(vec![
            "okf".into(),
            "publish".into(),
            env.okf_package.display().to_string(),
        ])
        .await
        .expect("okf publish should run through CLI entrypoint");

        let requests = captured.lock().unwrap().clone();
        let joined = requests.join("\n---\n");
        for expected in [
            r#""category":"agent""#,
            r#""agentKind":"agentic""#,
            r#""category":"mcp""#,
            r#""category":"skill""#,
            r#""category":"workflow""#,
            r#""category":"knowledge""#,
            r#""path":"agent.md""#,
            r#""path":"server.js""#,
            r#""path":"SKILL.md""#,
            r#""path":"flow.json""#,
            r#""path":"README.md""#,
            r#""path":".a3s/asset.acl""#,
        ] {
            assert!(
                joined.contains(expected),
                "missing `{expected}` in:\n{joined}"
            );
        }
        for forbidden in [
            "agent.runtime-binding.json",
            "mcp.runtime-binding.json",
            "skill.runtime-binding.json",
            "knowledge.runtime-binding.json",
            "runtime-binding.json",
            "debug",
            "/runtime/functions/mcp-asset-1/run",
            "/runtime/functions/mcp-asset-1/batch",
            "/run-mcp",
        ] {
            assert!(
                !joined.contains(forbidden),
                "unexpected legacy/config fragment `{forbidden}` in:\n{joined}"
            );
        }

        drop(env);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn code_cli_rejects_removed_mcp_debug_and_invoke_without_os_requests() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_cli_lifecycle_os_mock(captured.clone()).await;
        let root = temp_dir("code-cli-lifecycle-rejects");
        let env = CliLifecycleEnv::new(&root, &origin);

        for command in ["debug", "invoke"] {
            let err = run_code_cli(vec![
                "mcp".into(),
                command.into(),
                env.mcp_package.display().to_string(),
            ])
            .await
            .expect_err("removed MCP command should be rejected at CLI entrypoint");
            assert!(
                err.to_string().contains("unknown a3s code mcp command"),
                "{command}: {err}"
            );
        }

        assert!(
            captured.lock().unwrap().is_empty(),
            "removed MCP commands must not call OS"
        );
        drop(env);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn code_cli_mcp_run_requires_mcp_runner_without_runtime_function_fallback() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
        let origin = spawn_cli_lifecycle_os_mock(captured.clone()).await;
        let root = temp_dir("code-cli-mcp-run-no-fallback");
        let env = CliLifecycleEnv::new(&root, &origin);

        let err = run_code_cli(vec![
            "mcp".into(),
            "run".into(),
            env.mcp_package.display().to_string(),
        ])
        .await
        .expect_err("mcp run should not fall back to Runtime Function run");
        assert!(
            err.to_string()
                .contains("did not expose a runnable MCP capability"),
            "{err}"
        );

        let requests = captured.lock().unwrap().clone();
        let joined = requests.join("\n---\n");
        assert!(joined.contains(r#""category":"mcp""#), "{joined}");
        assert!(
            !joined.contains("/runtime/functions/mcp-asset-1/run"),
            "{joined}"
        );
        assert!(
            !joined.contains("/runtime/functions/mcp-asset-1/batch"),
            "{joined}"
        );
        drop(env);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scoped_queries_match_tui_shape() {
        assert_eq!(os_asset_category_query("mcp", ""), "category:mcp");
        assert_eq!(
            os_asset_category_query("mcp", "weather"),
            "category:mcp weather"
        );
        assert_eq!(
            runtime_asset_query("workflow", "flow-demo", "failed"),
            "category:workflow flow-demo failed"
        );
    }

    #[test]
    fn parses_ctx_show_window() {
        let (event, window) = parse_ctx_show_args(&["evt-1".into(), "--window".into(), "9".into()])
            .expect("ctx show args");
        assert_eq!(event, "evt-1");
        assert_eq!(window, 9);
        assert!(parse_ctx_show_args(&["evt-1".into(), "--window".into(), "0".into()]).is_err());
        assert!(parse_ctx_show_args(&[]).is_err());
    }

    #[test]
    fn resolves_single_agent_from_current_directory() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir("code-cli-agent");
        let package = dir.join("reviewer");
        std::fs::create_dir_all(&package).unwrap();
        let agent = package.join("agent.md");
        std::fs::write(
            &agent,
            "---\nname: reviewer\ndescription: Review code changes carefully\n---\nReview.\n",
        )
        .unwrap();
        let old_cwd = std::env::current_dir().unwrap();
        let old_agent_dir = std::env::var_os("A3S_AGENT_DIR");
        std::env::set_var("A3S_AGENT_DIR", &dir);
        std::env::set_current_dir(&dir).unwrap();

        let dev = resolve_agent_dev(None).expect("single agent in cwd");
        let dev_path = std::fs::canonicalize(&dev.path).unwrap();
        let agent_path = std::fs::canonicalize(&agent).unwrap();

        std::env::set_current_dir(old_cwd).unwrap();
        restore_env("A3S_AGENT_DIR", old_agent_dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(dev.name, "reviewer");
        assert_eq!(dev.rel, "reviewer");
        assert_eq!(dev.definition_rel, "agent.md");
        assert_eq!(dev_path, agent_path);
    }

    #[test]
    fn resolves_agent_package_from_entry_file_path_for_compatibility() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir("code-cli-agent-entry");
        let package = dir.join("agents/reviewer");
        std::fs::create_dir_all(&package).unwrap();
        let agent = package.join("agent.md");
        std::fs::write(
            &agent,
            "---\nname: reviewer\ndescription: Review code changes carefully\n---\nReview.\n",
        )
        .unwrap();
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();

        let dev = resolve_agent_dev(Some(agent.to_string_lossy().to_string()))
            .expect("entry file should resolve to package");
        let dev_path = std::fs::canonicalize(&dev.path).unwrap();
        let agent_path = std::fs::canonicalize(&agent).unwrap();
        let package_path = std::fs::canonicalize(&dev.package_path).unwrap();
        let expected_package = std::fs::canonicalize(&package).unwrap();

        std::env::set_current_dir(old_cwd).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(dev.name, "reviewer");
        assert_eq!(dev.rel, "reviewer");
        assert_eq!(dev.definition_rel, "agent.md");
        assert_eq!(dev_path, agent_path);
        assert_eq!(package_path, expected_package);
    }

    #[test]
    fn resolves_okf_package_from_visible_readme_path() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir("code-cli-okf");
        let workspace = dir.join("workspace");
        let package = workspace.join("okf/ops");
        std::fs::create_dir_all(package.join("sources")).unwrap();
        let readme = package.join("README.md");
        std::fs::write(&readme, "# ops-knowledge\n\nOperations knowledge\n").unwrap();
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&workspace).unwrap();

        let dev = resolve_okf_dev(Some(readme.to_string_lossy().to_string()))
            .expect("README path should resolve to package dir");
        let dev_path = std::fs::canonicalize(&dev.path).unwrap();
        let package_path = std::fs::canonicalize(&package).unwrap();

        std::env::set_current_dir(old_cwd).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(dev.name, "ops-knowledge");
        assert_eq!(dev.rel, "ops");
        assert_eq!(dev_path, package_path);
    }

    struct CliLifecycleEnv {
        root: PathBuf,
        agent_package: PathBuf,
        mcp_package: PathBuf,
        skill_package: PathBuf,
        flow_file: PathBuf,
        okf_package: PathBuf,
        old_cwd: PathBuf,
        old_env: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl CliLifecycleEnv {
        fn new(root: &Path, origin: &str) -> Self {
            let workspace = root.join("workspace");
            let home = root.join("home");
            let agent_root = root.join("agents");
            let mcp_root = root.join("mcps");
            let skill_root = root.join("skills");
            let flow_root = root.join("flows");
            let memory_root = root.join("memory");
            let agent_package = agent_root.join("reviewer");
            let mcp_package = mcp_root.join("weather");
            let skill_package = skill_root.join("sql-checker");
            let flow_package = flow_root.join("daily-digest");
            let flow_file = flow_package.join("flow.json");
            let okf_package = workspace.join("okf").join("ops-runbook");
            for dir in [
                &workspace,
                &home,
                &agent_package,
                &mcp_package,
                &skill_package,
                &flow_package,
                &okf_package,
                &memory_root,
            ] {
                std::fs::create_dir_all(dir).unwrap();
            }
            std::fs::create_dir_all(agent_package.join(".a3s")).unwrap();
            std::fs::create_dir_all(mcp_package.join(".a3s")).unwrap();
            std::fs::create_dir_all(skill_package.join(".a3s")).unwrap();
            std::fs::create_dir_all(flow_package.join(".a3s")).unwrap();
            std::fs::create_dir_all(okf_package.join(".a3s")).unwrap();
            std::fs::create_dir_all(okf_package.join("sources")).unwrap();
            for dir in ["prompts", "workflows", "examples", "eval", "tests"] {
                std::fs::create_dir_all(agent_package.join(dir)).unwrap();
            }

            std::fs::write(
                agent_package.join("agent.md"),
                "---\nname: reviewer\ndescription: Review code changes carefully\nprompt: Review the target carefully.\n---\nReview code.\n",
            )
            .unwrap();
            std::fs::write(agent_package.join("README.md"), "# reviewer\n").unwrap();
            std::fs::write(
                agent_package.join("prompts/system.md"),
                "Review the target carefully.\n",
            )
            .unwrap();
            std::fs::write(
                agent_package.join("workflows/operating-procedure.md"),
                "Inspect, plan, execute, and report.\n",
            )
            .unwrap();
            std::fs::write(
                agent_package.join("examples/example-input.md"),
                "Review this diff.\n",
            )
            .unwrap();
            std::fs::write(
                agent_package.join("examples/example-output.md"),
                "Review complete.\n",
            )
            .unwrap();
            std::fs::write(agent_package.join("eval/smoke.md"), "Smoke eval.\n").unwrap();
            std::fs::write(agent_package.join("tests/smoke.md"), "Smoke test.\n").unwrap();
            std::fs::write(
                agent_package.join(".a3s/asset.acl"),
                "category = \"agent\"\n",
            )
            .unwrap();

            std::fs::write(
                mcp_package.join("README.md"),
                "# weather\n\nWeather MCP tools\n",
            )
            .unwrap();
            std::fs::write(mcp_package.join("server.js"), "process.stdin.resume();\n").unwrap();
            std::fs::write(mcp_package.join(".a3s/asset.acl"), "category = \"mcp\"\n").unwrap();

            std::fs::write(
                skill_package.join("SKILL.md"),
                "---\nname: sql-checker\ndescription: Check SQL safely\nkind: instruction\n---\nCheck SQL for risky patterns.\n",
            )
            .unwrap();
            std::fs::write(
                skill_package.join(".a3s/asset.acl"),
                "category = \"skill\"\n",
            )
            .unwrap();

            std::fs::write(
                &flow_file,
                r#"{"version":"a3s.workflow.design.v1","name":"daily-digest","description":"Daily digest","nodes":[{"id":"start","kind":"start"},{"id":"end","kind":"end"}],"edges":[{"id":"e1","sourceNodeID":"start","targetNodeID":"end"}]}"#,
            )
            .unwrap();
            std::fs::write(
                flow_package.join(".a3s/asset.acl"),
                "category = \"workflow\"\n",
            )
            .unwrap();

            std::fs::write(
                okf_package.join("README.md"),
                "# ops-runbook\n\nOperations response knowledge\n",
            )
            .unwrap();
            std::fs::write(
                okf_package.join("sources/overview.md"),
                "# Operations\n\nRestart and escalation notes.\n",
            )
            .unwrap();
            std::fs::write(
                okf_package.join(".a3s/asset.acl"),
                "category = \"knowledge\"\n",
            )
            .unwrap();

            let config = root.join("config.acl");
            write_lifecycle_config(&config, origin);
            write_lifecycle_auth_store(&home, origin);

            let keys = vec![
                "HOME",
                "A3S_CONFIG_FILE",
                "A3S_AGENT_DIR",
                "A3S_MCP_DIR",
                "A3S_SKILL_DIR",
                "A3S_FLOW_DIR",
                "A3S_MEMORY_DIR",
                crate::a3s_os::OS_ENV_BASE_URL,
                crate::a3s_os::OS_ENV_TOKEN,
                crate::a3s_os::OS_ENV_REFRESH_TOKEN,
            ];
            let old_env = keys
                .into_iter()
                .map(|key| (key, std::env::var_os(key)))
                .collect::<Vec<_>>();
            let old_cwd = std::env::current_dir().unwrap();

            std::env::set_var("HOME", &home);
            std::env::set_var("A3S_CONFIG_FILE", &config);
            std::env::set_var("A3S_AGENT_DIR", &agent_root);
            std::env::set_var("A3S_MCP_DIR", &mcp_root);
            std::env::set_var("A3S_SKILL_DIR", &skill_root);
            std::env::set_var("A3S_FLOW_DIR", &flow_root);
            std::env::set_var("A3S_MEMORY_DIR", &memory_root);
            std::env::remove_var(crate::a3s_os::OS_ENV_BASE_URL);
            std::env::remove_var(crate::a3s_os::OS_ENV_TOKEN);
            std::env::remove_var(crate::a3s_os::OS_ENV_REFRESH_TOKEN);
            std::env::set_current_dir(&workspace).unwrap();

            Self {
                root: root.to_path_buf(),
                agent_package,
                mcp_package,
                skill_package,
                flow_file,
                okf_package,
                old_cwd,
                old_env,
            }
        }
    }

    impl Drop for CliLifecycleEnv {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.old_cwd);
            for (key, value) in self.old_env.drain(..) {
                restore_env(key, value);
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn write_lifecycle_config(path: &Path, origin: &str) {
        std::fs::write(
            path,
            format!(
                "default_model = \"openai/x\"\n\
                 os = \"{origin}\"\n\
                 providers \"openai\" {{\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
                 models \"x\" {{ name = \"x\" }}\n}}\n\
                 memory {{\n  llmExtraction = false\n}}\n"
            ),
        )
        .unwrap();
    }

    fn write_lifecycle_auth_store(home: &Path, origin: &str) {
        let store = home.join(".a3s").join("os-auth.json");
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        std::fs::write(
            store,
            serde_json::to_string_pretty(&serde_json::json!({
                "sessions": [{
                    "address": origin,
                    "access_token": "token",
                    "token_type": "Bearer",
                    "login_at_ms": 1
                }]
            }))
            .unwrap(),
        )
        .unwrap();
    }

    async fn spawn_cli_lifecycle_os_mock(captured: std::sync::Arc<Mutex<Vec<String>>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let captured = captured.clone();
                tokio::spawn(async move {
                    let request = read_http_request(&mut sock).await;
                    let line = request.lines().next().unwrap_or("").to_string();
                    let body = request.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                    captured.lock().unwrap().push(format!("{line}\n{body}"));
                    let (status, payload) = cli_lifecycle_mock_response(&line, &body);
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                        payload.len()
                    );
                    let _ = sock.write_all(response.as_bytes()).await;
                    let _ = sock.flush().await;
                });
            }
        });
        origin
    }

    async fn read_http_request(sock: &mut tokio::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut tmp = [0_u8; 8192];
        let mut expected_len = None;
        loop {
            let Ok(n) = sock.read(&mut tmp).await else {
                break;
            };
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if expected_len.is_none() {
                expected_len = expected_http_request_len(&buf);
            }
            if expected_len.is_some_and(|len| buf.len() >= len) {
                break;
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn expected_http_request_len(buf: &[u8]) -> Option<usize> {
        let header_end = buf.windows(4).position(|window| window == b"\r\n\r\n")? + 4;
        let headers = String::from_utf8_lossy(&buf[..header_end]);
        let content_len = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        Some(header_end + content_len)
    }

    fn cli_lifecycle_mock_response(line: &str, body: &str) -> (&'static str, String) {
        if line.starts_with("GET /api/v1/assets?") {
            return ("200 OK", r#"{"data":{"items":[]}}"#.to_string());
        }
        if line.starts_with("PATCH /api/v1/assets/") {
            return ("200 OK", r#"{"data":{"ok":true}}"#.to_string());
        }
        if line.starts_with("POST /api/v1/assets HTTP/1.1") {
            let (id, name) = if body.contains(r#""category":"agent""#) {
                ("asset-agentic-1", "agentic-reviewer")
            } else if body.contains(r#""category":"mcp""#) {
                ("mcp-asset-1", "mcp-weather")
            } else if body.contains(r#""category":"skill""#) {
                ("skill-asset-1", "skill-sql-checker")
            } else if body.contains(r#""category":"workflow""#) {
                ("workflow-asset-1", "flow-daily-digest")
            } else if body.contains(r#""category":"knowledge""#) {
                ("knowledge-asset-1", "knowledge-ops-runbook")
            } else {
                return (
                    "422 Unprocessable Entity",
                    r#"{"code":422,"message":"unknown category"}"#.to_string(),
                );
            };
            return (
                "200 OK",
                format!(
                    r#"{{"data":{{"id":"{id}","name":"{name}","ownerName":"admin","defaultBranch":"main"}}}}"#
                ),
            );
        }
        if line.contains("/repository/files ") {
            return ("200 OK", r#"{"data":{"ok":true}}"#.to_string());
        }
        if line.contains("/agent-config/validate ") {
            return (
                "200 OK",
                r#"{"code":200,"data":{"valid":true,"diagnostics":[]}}"#.to_string(),
            );
        }
        if line.contains("/agent-config ") {
            return (
                "200 OK",
                r#"{"code":200,"data":{"configured":true}}"#.to_string(),
            );
        }
        if line.contains("/runtime-binding/validate ") {
            return (
                "200 OK",
                r#"{"code":200,"data":{"configured":true,"valid":true,"requiredSecrets":[],"missingSecrets":[],"expiredSecrets":[],"issues":[]}}"#.to_string(),
            );
        }
        if line.contains("/runtime-binding ") {
            return (
                "200 OK",
                r#"{"code":200,"data":{"configured":true}}"#.to_string(),
            );
        }
        if line.starts_with("POST /api/v1/kernel/capabilities ") {
            return (
                "404 Not Found",
                r#"{"code":404,"message":"capabilities unavailable in mock"}"#.to_string(),
            );
        }
        (
            "404 Not Found",
            format!(r#"{{"code":404,"message":"unhandled mock request: {line}"}}"#),
        )
    }

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "a3s-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}
