use a3s_code_core::config::CodeConfig;
use std::path::Path;

use crate::{a3s_os, claude, codex, config};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountSource {
    ClaudeCode,
    Codex,
    A3sOs,
}

impl AccountSource {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::A3sOs => "a3s-os",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccountStatus {
    pub(crate) source: AccountSource,
    pub(crate) signed_in: bool,
    pub(crate) label: Option<String>,
    pub(crate) detail: Option<String>,
}

pub(crate) fn discover() -> Vec<AccountStatus> {
    vec![
        AccountStatus {
            source: AccountSource::ClaudeCode,
            signed_in: claude::has_claude_login(),
            label: None,
            detail: None,
        },
        AccountStatus {
            source: AccountSource::Codex,
            signed_in: codex::has_codex_login(),
            label: None,
            detail: None,
        },
        os_status(),
    ]
}

fn os_status() -> AccountStatus {
    let Some(path) = config::find_config() else {
        return AccountStatus {
            source: AccountSource::A3sOs,
            signed_in: false,
            label: None,
            detail: Some("not configured".to_string()),
        };
    };
    let Ok(config) = CodeConfig::from_file(Path::new(&path)) else {
        return AccountStatus {
            source: AccountSource::A3sOs,
            signed_in: false,
            label: None,
            detail: Some("config.acl is invalid".to_string()),
        };
    };
    let Some(os_config) = config.os else {
        return AccountStatus {
            source: AccountSource::A3sOs,
            signed_in: false,
            label: None,
            detail: Some("not configured".to_string()),
        };
    };
    let session = a3s_os::current_session(&os_config);
    AccountStatus {
        source: AccountSource::A3sOs,
        signed_in: session.is_some(),
        label: session.as_ref().map(a3s_os::StoredOsSession::display_label),
        detail: Some(os_config.address),
    }
}
