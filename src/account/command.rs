use std::process::Stdio;

use super::catalog;

pub(crate) const USAGE: &str = "usage:\n\
  a3s account list\n\
  a3s account status\n\
  a3s account login <claude-code|codex|a3s-os> [token]\n\
  a3s account logout <claude-code|codex|a3s-os>\n";

pub(crate) async fn run(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        None | Some("list" | "status") => {
            if args.len() > 1 {
                anyhow::bail!("account list/status does not accept arguments\n\n{USAGE}");
            }
            print_accounts();
            Ok(())
        }
        Some("login") => login(&args[1..]).await,
        Some("logout") => logout(&args[1..]).await,
        Some("-h" | "--help" | "help") if args.len() == 1 => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => anyhow::bail!("unknown account command `{other}`\n\n{USAGE}"),
    }
}

fn print_accounts() {
    println!("SOURCE\tSTATUS\tACCOUNT");
    for account in catalog::discover() {
        let status = if account.signed_in {
            "signed-in"
        } else {
            "signed-out"
        };
        let account_label = account
            .label
            .or(account.detail)
            .unwrap_or_else(|| "-".to_string());
        println!("{}\t{status}\t{account_label}", account.source.id());
    }
}

async fn login(args: &[String]) -> anyhow::Result<()> {
    let Some(source) = args.first().map(String::as_str) else {
        anyhow::bail!("account login requires an account source\n\n{USAGE}");
    };
    match source {
        "a3s-os" => crate::os_cmd::login(&args[1..]).await,
        "claude-code" => {
            reject_extra_account_args(source, &args[1..])?;
            run_product_command("claude", &["auth", "login"]).await
        }
        "codex" => {
            reject_extra_account_args(source, &args[1..])?;
            run_product_command("codex", &["login"]).await
        }
        _ => anyhow::bail!("unknown account source `{source}`\n\n{USAGE}"),
    }
}

async fn logout(args: &[String]) -> anyhow::Result<()> {
    let [source] = args else {
        anyhow::bail!("account logout requires exactly one account source\n\n{USAGE}");
    };
    match source.as_str() {
        "a3s-os" => crate::os_cmd::logout(&[]),
        "claude-code" => run_product_command("claude", &["auth", "logout"]).await,
        "codex" => run_product_command("codex", &["logout"]).await,
        _ => anyhow::bail!("unknown account source `{source}`\n\n{USAGE}"),
    }
}

fn reject_extra_account_args(source: &str, args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{source} login does not accept a token; its product CLI owns authentication")
    }
}

async fn run_product_command(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .map_err(|error| anyhow::anyhow!("could not run `{program}`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("`{program} {}` exited with {status}", args.join(" "))
    }
}
