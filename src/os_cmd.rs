//! A3S OS account commands shared by the root CLI and Code compatibility aliases.

use std::path::Path;

use a3s_code_core::config::{CodeConfig, OsConfig};

use crate::{a3s_os, config};

pub(crate) const LOGIN_USAGE: &str = "usage: a3s login [token]\n\n\
With no token, open the configured A3S OS OAuth login in a browser.\n\
With a token, store an existing A3S OS bearer token.\n";

pub(crate) const LOGOUT_USAGE: &str = "usage: a3s logout\n\n\
Remove the login for the configured A3S OS account.\n";

pub(crate) async fn login(args: &[String]) -> anyhow::Result<()> {
    if help_request(args) {
        print!("{LOGIN_USAGE}");
        return Ok(());
    }
    let token = optional_token(args)?;
    let os_config = load_os_config()?;
    let session = match token {
        Some(token) => a3s_os::login_with_token(&os_config, token).await?,
        None => a3s_os::login_via_browser(os_config.clone()).await?,
    };

    a3s_os::export_os_env(&session);
    let skill_ready = a3s_os::ensure_capability_skill_dir(&os_config).is_some();
    println!("signed in to A3S OS as {}", session.display_label());
    if skill_ready {
        println!("capabilities skill: active");
    } else {
        println!("capabilities skill: not installed (check ~/.a3s permissions)");
    }
    print_ssh_key_outcome(a3s_os::sync_ssh_key(session).await);
    Ok(())
}

pub(crate) fn logout(args: &[String]) -> anyhow::Result<()> {
    if help_request(args) {
        print!("{LOGOUT_USAGE}");
        return Ok(());
    }
    if !args.is_empty() {
        anyhow::bail!("logout does not accept arguments\n\n{LOGOUT_USAGE}");
    }

    let os_config = load_os_config()?;
    let removed = a3s_os::logout(&os_config)?;
    a3s_os::clear_os_env();
    a3s_os::remove_capability_skill_dir();
    if removed {
        println!("signed out from A3S OS");
    } else {
        println!("no stored A3S OS login for {}", os_config.address);
    }
    Ok(())
}

fn load_os_config() -> anyhow::Result<OsConfig> {
    let config_path = config::find_config().ok_or_else(|| {
        anyhow::anyhow!("config.acl was not found; create ~/.a3s/config.acl or set A3S_CONFIG_FILE")
    })?;
    let config = CodeConfig::from_file(Path::new(&config_path))
        .map_err(|error| anyhow::anyhow!("failed to parse {config_path}: {error}"))?;
    config.os.ok_or_else(|| {
        anyhow::anyhow!(
            "A3S OS is not configured in {config_path}; set os = \"https://your-os-host\""
        )
    })
}

fn optional_token(args: &[String]) -> anyhow::Result<Option<&str>> {
    match args {
        [] => Ok(None),
        [token] => Ok(Some(token)),
        _ => anyhow::bail!("login accepts at most one token\n\n{LOGIN_USAGE}"),
    }
}

fn help_request(args: &[String]) -> bool {
    matches!(args, [arg] if matches!(arg.as_str(), "-h" | "--help" | "help"))
}

fn print_ssh_key_outcome(outcome: a3s_os::SshKeyOutcome) {
    match outcome {
        a3s_os::SshKeyOutcome::Registered(fingerprint) => {
            println!("ssh key: registered with A3S OS ({fingerprint})")
        }
        a3s_os::SshKeyOutcome::AlreadyRegistered => {
            println!("ssh key: already registered")
        }
        a3s_os::SshKeyOutcome::NoLocalKey => {
            println!("ssh key: none found (create one with `ssh-keygen -t ed25519`)")
        }
        a3s_os::SshKeyOutcome::Failed(error) => {
            println!("ssh key: sync skipped: {error}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_optional_but_singular() {
        assert_eq!(optional_token(&[]).unwrap(), None);
        assert_eq!(optional_token(&["token".into()]).unwrap(), Some("token"));
        assert!(optional_token(&["one".into(), "two".into()]).is_err());
    }

    #[test]
    fn help_must_be_the_only_argument() {
        assert!(help_request(&["--help".into()]));
        assert!(!help_request(&["token".into(), "--help".into()]));
    }
}
