use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{bail, Context};
use serde_json::json;

use crate::cli::args::{CodeSessionArgs, CodeSessionCommand, OutputMode};
use crate::cli::context::InvocationContext;
use crate::cli::output::render_value;

pub(super) fn run(args: CodeSessionArgs, context: &InvocationContext) -> anyhow::Result<()> {
    match args.command {
        CodeSessionCommand::List => list(context),
        CodeSessionCommand::Show(args) => show(&args.session_id, context),
        CodeSessionCommand::Export(args) => export(&args.session_id, args.output_file, context),
        CodeSessionCommand::Delete(args) => delete(&args.session_id, args.yes, context),
    }
}

fn list(context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    let root = session_root(context);
    let mut sessions = Vec::new();
    if root.is_dir() {
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let metadata = entry.metadata()?;
            let Some(id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let modified_at_ms = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_millis());
            sessions.push(json!({
                "id": id,
                "bytes": metadata.len(),
                "modifiedAtMs": modified_at_ms,
            }));
        }
    }
    sessions.sort_by(|left, right| {
        right["modifiedAtMs"]
            .as_u64()
            .cmp(&left["modifiedAtMs"].as_u64())
    });
    render_value(
        output,
        "code.session.list",
        json!({"workspace": context.directory, "sessions": sessions}),
        || {
            if sessions.is_empty() {
                println!("no saved sessions");
            } else {
                println!("SESSION ID                              SIZE");
                for session in &sessions {
                    println!(
                        "{:<39} {}",
                        session["id"].as_str().unwrap_or_default(),
                        session["bytes"].as_u64().unwrap_or_default()
                    );
                }
            }
        },
    )
}

fn show(id: &str, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    let path = session_path(id, context)?;
    let document = read_document(&path)?;
    render_value(
        output,
        "code.session.show",
        json!({"id": id, "path": path, "document": document}),
        || {
            println!(
                "{}",
                serde_json::to_string_pretty(&document).unwrap_or_default()
            )
        },
    )
}

fn export(
    id: &str,
    output_file: Option<PathBuf>,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    let output = context.output_mode();
    let path = session_path(id, context)?;
    let bytes = std::fs::read(&path)
        .with_context(|| format!("could not read session `{id}` from {}", path.display()))?;
    if let Some(destination) = output_file.map(|path| context.resolve_path(path)) {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&destination, &bytes)?;
        return render_value(
            output,
            "code.session.export",
            json!({"id": id, "outputFile": destination, "bytes": bytes.len()}),
            || println!("exported session `{id}` to {}", destination.display()),
        );
    }
    let document: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("session `{id}` is not valid JSON"))?;
    if output == OutputMode::Human {
        println!("{}", serde_json::to_string_pretty(&document)?);
        Ok(())
    } else {
        render_value(
            output,
            "code.session.export",
            json!({"id": id, "document": document}),
            || {},
        )
    }
}

fn delete(id: &str, yes: bool, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    let path = session_path(id, context)?;
    if !path.is_file() {
        bail!("session `{id}` does not exist in this workspace");
    }
    if !yes {
        if output != OutputMode::Human
            || context.interaction.non_interactive
            || !context.terminal.stdin
            || !context.terminal.stderr
        {
            bail!("session deletion requires `--yes` in non-interactive mode");
        }
        eprint!("Delete session `{id}`? [y/N] ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            bail!("session deletion cancelled");
        }
    }
    std::fs::remove_file(&path).with_context(|| format!("could not delete session `{id}`"))?;
    render_value(
        output,
        "code.session.delete",
        json!({"id": id, "deleted": true}),
        || println!("deleted session `{id}`"),
    )
}

fn session_root(context: &InvocationContext) -> PathBuf {
    context.directory.join(".a3s/tui-sessions")
}

fn session_path(id: &str, context: &InvocationContext) -> anyhow::Result<PathBuf> {
    validate_id(id)?;
    Ok(session_root(context).join(format!("{id}.json")))
}

fn validate_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("session ID may contain only ASCII letters, digits, `-`, and `_`");
    }
    Ok(())
}

fn read_document(path: &Path) -> anyhow::Result<serde_json::Value> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("could not read session {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("session {} is not valid JSON", path.display()))
}
