use std::path::{Path, PathBuf};
use std::process::Command;

use a3s_code_core::config::{CodeConfig, OsConfig};

use super::{asset_clone, config, kbutil, memutil, panels, remote_ui};

const TOP_LEVEL_COMMANDS: &[&str] = &[
    "agent", "mcp", "skill", "flow", "okf", "login", "logout", "auth", "config", "dirs", "models",
    "model", "kb", "ctx", "memory", "mem", "top", "view", "serve",
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
        "  a3s code mcp publish|deploy|debug|test|open|logs|status [path]".to_string(),
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
        "  a3s code view <url> [--width N] [--height N]".to_string(),
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
        Some("kb") => run_kb(&args[1..]),
        Some("ctx") => run_ctx(&args[1..]).await,
        Some("memory" | "mem") => run_memory(&args[1..]),
        Some("top") => crate::top::run(args[1..].to_vec()).await,
        Some("view") => run_view(&args[1..]).await,
        Some("serve") => super::code_serve::run(&args[1..]).await,
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
            let path = single_path_arg("agent run", &args[1..])?;
            run_agent_os(panels::agent::AgentOsAction::Run, path.as_deref(), false).await
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
        "publish" | "deploy" | "debug" | "test" | "open" | "logs" | "status" => {
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
            println!("  lists config.acl models, local Claude/Codex account models, and OS gateway models when signed in");
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

async fn run_view(args: &[String]) -> anyhow::Result<()> {
    if matches!(
        args.first().map(String::as_str),
        Some("-h" | "--help" | "help")
    ) {
        println!("a3s code view <url> [--width N] [--height N]");
        println!("  opens an explicit OS RemoteUI/viewUrl in a3s-webview or the system browser");
        return Ok(());
    }
    let spec = parse_view_spec(args)?;
    let _ = current_os_session_if_configured().await;
    let opened = remote_ui::open_window(&spec)
        .map_err(|e| anyhow::anyhow!("could not open RemoteUI view: {e}"))?;
    println!("opened: {:?}", opened);
    Ok(())
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

fn parse_view_spec(args: &[String]) -> anyhow::Result<remote_ui::ViewSpec> {
    if args.is_empty() {
        anyhow::bail!("usage: a3s code view <url> [--width N] [--height N]");
    }
    let url = args[0].clone();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        anyhow::bail!("RemoteUI url must start with http:// or https://");
    }
    let mut width = None;
    let mut height = None;
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--width" => {
                width = Some(parse_u32_option("--width", args.get(i + 1))?);
                i += 2;
            }
            "--height" => {
                height = Some(parse_u32_option("--height", args.get(i + 1))?);
                i += 2;
            }
            "--size" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("--size requires WIDTHxHEIGHT"))?;
                let (w, h) = value
                    .split_once('x')
                    .or_else(|| value.split_once('X'))
                    .ok_or_else(|| anyhow::anyhow!("--size expects WIDTHxHEIGHT"))?;
                width = Some(w.parse().map_err(|_| anyhow::anyhow!("invalid width"))?);
                height = Some(h.parse().map_err(|_| anyhow::anyhow!("invalid height"))?);
                i += 2;
            }
            other => anyhow::bail!("unknown view option `{other}`"),
        }
    }
    Ok(remote_ui::ViewSpec {
        url,
        width,
        height,
        embeddable: width.is_some() || height.is_some(),
    })
}

fn parse_u32_option(name: &str, value: Option<&String>) -> anyhow::Result<u32> {
    let value = value.ok_or_else(|| anyhow::anyhow!("{name} requires a value"))?;
    let parsed: u32 = value
        .parse()
        .map_err(|_| anyhow::anyhow!("{name} must be a positive integer"))?;
    if parsed == 0 {
        anyhow::bail!("{name} must be greater than zero");
    }
    Ok(parsed)
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
    let design = if matches!(
        action,
        panels::flow::FlowOsAction::Open
            | panels::flow::FlowOsAction::Logs
            | panels::flow::FlowOsAction::Status
    ) {
        String::new()
    } else {
        read_flow_design(&flow.path)?
    };
    let session = load_os_session().await?;
    let result = panels::flow::publish_flow_to_os(session, flow.rel, design, action)
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
        "mcp" => println!("  publish|deploy|debug|test|open|logs|status [path]"),
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
        assert!(is_code_cli_command(&["view".into()]));
        assert!(is_code_cli_command(&["--help".into()]));
        assert!(!is_code_cli_command(&["resume".into(), "abc".into()]));
        assert!(!is_code_cli_command(&["some-prompt".into()]));
    }

    #[test]
    fn usage_mentions_noninteractive_asset_commands() {
        let text = code_cli_usage_text();
        assert!(text.contains("a3s code <family> local"));
        assert!(text.contains("a3s code agent publish agentic|application|tool"));
        assert!(text.contains("a3s code mcp publish|deploy|debug|test"));
        assert!(text.contains("families: agent, mcp, skill, flow, okf"));
        assert!(text.contains("a3s code login [token]"));
        assert!(text.contains("a3s code kb stats|add|import|search|vault"));
        assert!(text.contains("a3s code ctx search <query>"));
        assert!(text.contains("a3s code view <url>"));
    }

    #[test]
    fn parses_agent_kind_and_path_without_losing_default_kind() {
        let (kind, path) = parse_agent_kind_path("open", &[]).unwrap();
        assert_eq!(kind, panels::agent::AgentOsKind::Agentic);
        assert_eq!(path, None);

        let (kind, path) = parse_agent_kind_path("open", &["tool".into(), "agents/tooler".into()])
            .expect("kind plus path");
        assert_eq!(kind, panels::agent::AgentOsKind::Tool);
        assert_eq!(path.as_deref(), Some("agents/tooler"));

        let (kind, path) = parse_agent_kind_path("open", &["agents/reviewer".into()])
            .expect("path only uses default kind");
        assert_eq!(kind, panels::agent::AgentOsKind::Agentic);
        assert_eq!(path.as_deref(), Some("agents/reviewer"));
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
    fn parses_view_spec_with_size_without_accepting_relative_urls() {
        let spec = parse_view_spec(&[
            "https://os.example/view".into(),
            "--size".into(),
            "1024x768".into(),
        ])
        .expect("sized view");
        assert_eq!(spec.url, "https://os.example/view");
        assert_eq!((spec.width, spec.height), (Some(1024), Some(768)));
        assert!(spec.embeddable);

        assert!(parse_view_spec(&["/admin/view".into()]).is_err());
        assert!(parse_view_spec(&["https://os.example/view".into(), "--width".into()]).is_err());
        assert!(parse_view_spec(&[
            "https://os.example/view".into(),
            "--height".into(),
            "0".into()
        ])
        .is_err());
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
    fn resolves_okf_package_from_manifest_file_path() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir("code-cli-okf");
        let workspace = dir.join("workspace");
        let package = workspace.join(".a3s/okf/ops");
        std::fs::create_dir_all(&package).unwrap();
        let manifest = package.join("package.okf.json");
        std::fs::write(
            &manifest,
            r#"{"name":"ops-knowledge","description":"Operations knowledge"}"#,
        )
        .unwrap();
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&workspace).unwrap();

        let dev = resolve_okf_dev(Some(manifest.to_string_lossy().to_string()))
            .expect("manifest file should resolve to package dir");
        let dev_path = std::fs::canonicalize(&dev.path).unwrap();
        let package_path = std::fs::canonicalize(&package).unwrap();

        std::env::set_current_dir(old_cwd).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(dev.name, "ops-knowledge");
        assert_eq!(dev.rel, "ops");
        assert_eq!(dev_path, package_path);
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
