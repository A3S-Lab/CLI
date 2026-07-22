use std::collections::HashSet;
use std::path::Path;

use a3s_acl::{Block, Value};

use super::dto::SafeBlocker;
use super::ilink::{production_transports, IlinkError, IlinkProductionTransports};

const PRIMARY_ILINK_HOST: &str = "ilinkai.weixin.qq.com";
const ENV_APP_ID: &str = "A3S_WEIXIN_ILINK_APP_ID";
const ENV_BOT_TYPE: &str = "A3S_WEIXIN_ILINK_BOT_TYPE";
const ENV_CLIENT_VERSION: &str = "A3S_WEIXIN_ILINK_CLIENT_VERSION";
const ENV_BOT_AGENT: &str = "A3S_WEIXIN_ILINK_BOT_AGENT";
const ENV_ALLOWED_HOSTS: &str = "A3S_WEIXIN_ILINK_ALLOWED_HOSTS";
const MAX_ALLOWED_HOSTS: usize = 16;

const COMPILED_APP_ID: Option<&str> = option_env!("A3S_WEIXIN_ILINK_APP_ID");
const COMPILED_BOT_TYPE: Option<&str> = option_env!("A3S_WEIXIN_ILINK_BOT_TYPE");
const COMPILED_CLIENT_VERSION: Option<&str> = option_env!("A3S_WEIXIN_ILINK_CLIENT_VERSION");
const COMPILED_BOT_AGENT: Option<&str> = option_env!("A3S_WEIXIN_ILINK_BOT_AGENT");
const COMPILED_ALLOWED_HOSTS: Option<&str> = option_env!("A3S_WEIXIN_ILINK_ALLOWED_HOSTS");

pub(super) enum IlinkEntitlementLoad {
    Ready(IlinkEntitlement),
    Unavailable(SafeBlocker),
}

pub(super) struct IlinkEntitlement {
    app_id: String,
    bot_type: String,
    client_version: String,
    bot_agent: String,
    allowed_hosts: Vec<String>,
}

impl IlinkEntitlement {
    pub(super) fn load(config_path: &Path) -> IlinkEntitlementLoad {
        let source = match std::fs::read_to_string(config_path) {
            Ok(source) => Some(source),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return IlinkEntitlementLoad::Unavailable(blocker(
                    "ilink_configuration_unreadable",
                    "The local Weixin iLink configuration could not be read.",
                ))
            }
        };
        load_from_source(source.as_deref(), &|name| std::env::var(name).ok())
    }

    pub(super) fn build_transports(self) -> Result<IlinkProductionTransports, IlinkError> {
        production_transports(
            self.app_id,
            self.bot_type,
            &self.client_version,
            self.bot_agent,
            &self.allowed_hosts,
        )
    }
}

fn load_from_source(
    source: Option<&str>,
    environment: &dyn Fn(&str) -> Option<String>,
) -> IlinkEntitlementLoad {
    let block = match source.map(parse_weixin_block).transpose() {
        Ok(block) => block.flatten(),
        Err(issue) => return IlinkEntitlementLoad::Unavailable(issue),
    };

    if let Some(block) = block.as_ref() {
        match optional_bool(block, &["enabled"]) {
            Ok(Some(false)) => {
                return IlinkEntitlementLoad::Unavailable(blocker(
                    "ilink_channel_disabled",
                    "The Weixin channel is disabled in the local A3S configuration.",
                ))
            }
            Ok(_) => {}
            Err(issue) => return IlinkEntitlementLoad::Unavailable(issue),
        }
        if !block.blocks.is_empty() {
            return IlinkEntitlementLoad::Unavailable(invalid_configuration());
        }
    }

    let app_id = match configured_string(
        block.as_ref(),
        &["app_id", "appId"],
        ENV_APP_ID,
        COMPILED_APP_ID,
        environment,
    ) {
        Ok(value) => value,
        Err(issue) => return IlinkEntitlementLoad::Unavailable(issue),
    };
    let bot_type = match configured_string(
        block.as_ref(),
        &["bot_type", "botType"],
        ENV_BOT_TYPE,
        COMPILED_BOT_TYPE,
        environment,
    ) {
        Ok(value) => value,
        Err(issue) => return IlinkEntitlementLoad::Unavailable(issue),
    };

    let Some(app_id) = app_id else {
        return IlinkEntitlementLoad::Unavailable(missing_configuration());
    };
    let Some(bot_type) = bot_type else {
        return IlinkEntitlementLoad::Unavailable(missing_configuration());
    };
    if app_id.eq_ignore_ascii_case("bot") {
        return IlinkEntitlementLoad::Unavailable(blocker(
            "ilink_identity_not_a3s",
            "The configured iLink application identity is reserved for another product.",
        ));
    }

    let client_version = match configured_string(
        block.as_ref(),
        &["client_version", "clientVersion"],
        ENV_CLIENT_VERSION,
        COMPILED_CLIENT_VERSION,
        environment,
    ) {
        Ok(value) => value.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        Err(issue) => return IlinkEntitlementLoad::Unavailable(issue),
    };
    let bot_agent = match configured_string(
        block.as_ref(),
        &["bot_agent", "botAgent"],
        ENV_BOT_AGENT,
        COMPILED_BOT_AGENT,
        environment,
    ) {
        Ok(value) => value.unwrap_or_else(|| format!("A3S/{}", env!("CARGO_PKG_VERSION"))),
        Err(issue) => return IlinkEntitlementLoad::Unavailable(issue),
    };
    if bot_agent.to_ascii_lowercase().contains("openclaw") {
        return IlinkEntitlementLoad::Unavailable(blocker(
            "ilink_identity_not_a3s",
            "The configured iLink client identity must identify A3S.",
        ));
    }

    let allowed_hosts = match configured_hosts(block.as_ref(), environment) {
        Ok(hosts) => hosts,
        Err(issue) => return IlinkEntitlementLoad::Unavailable(issue),
    };
    if !allowed_hosts
        .iter()
        .any(|host| host.eq_ignore_ascii_case(PRIMARY_ILINK_HOST))
    {
        return IlinkEntitlementLoad::Unavailable(invalid_configuration());
    }

    let entitlement = IlinkEntitlement {
        app_id,
        bot_type,
        client_version,
        bot_agent,
        allowed_hosts,
    };
    if entitlement_fields_are_valid(&entitlement).is_err() {
        return IlinkEntitlementLoad::Unavailable(invalid_configuration());
    }
    IlinkEntitlementLoad::Ready(entitlement)
}

fn parse_weixin_block(source: &str) -> Result<Option<Block>, SafeBlocker> {
    let document = a3s_acl::parse_acl(source).map_err(|_| invalid_configuration())?;
    let channels = document
        .blocks
        .iter()
        .filter(|block| block.name == "channels")
        .collect::<Vec<_>>();
    if channels.len() > 1 {
        return Err(invalid_configuration());
    }
    let Some(channels) = channels.first() else {
        return Ok(None);
    };
    let weixin = channels
        .blocks
        .iter()
        .filter(|block| block.name == "weixin")
        .collect::<Vec<_>>();
    if weixin.len() > 1 {
        return Err(invalid_configuration());
    }
    Ok(weixin.first().map(|block| (*block).clone()))
}

fn configured_string(
    block: Option<&Block>,
    keys: &[&str],
    environment_name: &str,
    compiled: Option<&str>,
    environment: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<String>, SafeBlocker> {
    if let Some(value) = block.and_then(|block| attribute(block, keys)) {
        return resolve_string(value, environment).map(Some);
    }
    Ok(nonempty(environment(environment_name)).or_else(|| nonempty(compiled.map(str::to_string))))
}

fn configured_hosts(
    block: Option<&Block>,
    environment: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<String>, SafeBlocker> {
    let configured = if let Some(value) =
        block.and_then(|block| attribute(block, &["allowed_hosts", "allowedHosts"]))
    {
        match value {
            Value::List(values) => values
                .iter()
                .map(|value| resolve_string(value, environment))
                .collect::<Result<Vec<_>, _>>()?,
            Value::Call(_, _) | Value::String(_) => resolve_string(value, environment)?
                .split(',')
                .map(str::trim)
                .filter(|host| !host.is_empty())
                .map(str::to_string)
                .collect(),
            _ => return Err(invalid_configuration()),
        }
    } else if let Some(value) =
        environment(ENV_ALLOWED_HOSTS).or_else(|| COMPILED_ALLOWED_HOSTS.map(str::to_string))
    {
        value
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![PRIMARY_ILINK_HOST.to_string()]
    };

    let mut seen = HashSet::new();
    let hosts = configured
        .into_iter()
        .filter(|host| seen.insert(host.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    if hosts.is_empty() || hosts.len() > MAX_ALLOWED_HOSTS {
        return Err(invalid_configuration());
    }
    Ok(hosts)
}

fn optional_bool(block: &Block, keys: &[&str]) -> Result<Option<bool>, SafeBlocker> {
    match attribute(block, keys) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(invalid_configuration()),
    }
}

fn attribute<'a>(block: &'a Block, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| block.attributes.get(*key))
}

fn resolve_string(
    value: &Value,
    environment: &dyn Fn(&str) -> Option<String>,
) -> Result<String, SafeBlocker> {
    let value = match value {
        Value::String(value) => Some(value.clone()),
        Value::Call(name, arguments) if name == "env" && arguments.len() == 1 => arguments
            .first()
            .and_then(Value::as_str)
            .and_then(environment),
        _ => return Err(invalid_configuration()),
    };
    nonempty(value).ok_or_else(missing_configuration)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn entitlement_fields_are_valid(entitlement: &IlinkEntitlement) -> Result<(), IlinkError> {
    production_transports(
        entitlement.app_id.clone(),
        entitlement.bot_type.clone(),
        &entitlement.client_version,
        entitlement.bot_agent.clone(),
        &entitlement.allowed_hosts,
    )
    .map(|_| ())
}

fn missing_configuration() -> SafeBlocker {
    blocker(
        "ilink_configuration_missing",
        "Configure an A3S iLink application identity and permitted bot type, then restart A3S Web.",
    )
}

fn invalid_configuration() -> SafeBlocker {
    blocker(
        "ilink_configuration_invalid",
        "The local Weixin iLink configuration is invalid.",
    )
}

fn blocker(code: &str, message: &str) -> SafeBlocker {
    SafeBlocker {
        code: code.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_environment(_name: &str) -> Option<String> {
        None
    }

    #[test]
    fn missing_identity_keeps_production_transport_unavailable() {
        let result = load_from_source(None, &no_environment);
        let IlinkEntitlementLoad::Unavailable(blocker) = result else {
            panic!("missing identity must not enable iLink");
        };
        assert_eq!(blocker.code, "ilink_configuration_missing");
    }

    #[test]
    fn a3s_acl_identity_enables_strict_production_transport() {
        let result = load_from_source(
            Some(
                r#"
                    channels {
                      weixin {
                        enabled = true
                        app_id = "a3s-production"
                        bot_type = "3"
                        client_version = "1.2.3"
                        bot_agent = "A3S/1.2.3"
                        allowed_hosts = ["ilinkai.weixin.qq.com", "weixin-a3s.example.com"]
                      }
                    }
                "#,
            ),
            &no_environment,
        );
        let IlinkEntitlementLoad::Ready(entitlement) = result else {
            panic!("valid A3S entitlement must enable iLink");
        };
        assert_eq!(entitlement.app_id, "a3s-production");
        assert_eq!(entitlement.bot_type, "3");
        assert!(entitlement.build_transports().is_ok());
    }

    #[test]
    fn openclaw_application_identity_is_rejected() {
        let result = load_from_source(
            Some(
                r#"
                    channels {
                      weixin { app_id = "bot" bot_type = "3" }
                    }
                "#,
            ),
            &no_environment,
        );
        let IlinkEntitlementLoad::Unavailable(blocker) = result else {
            panic!("another product identity must not enable A3S iLink");
        };
        assert_eq!(blocker.code, "ilink_identity_not_a3s");
    }

    #[test]
    fn environment_references_resolve_without_browser_configuration() {
        let environment = |name: &str| match name {
            "A3S_APP_ID" => Some("a3s-env-app".to_string()),
            "A3S_BOT_TYPE" => Some("3".to_string()),
            _ => None,
        };
        let result = load_from_source(
            Some(
                r#"
                    channels {
                      weixin {
                        app_id = env("A3S_APP_ID")
                        bot_type = env("A3S_BOT_TYPE")
                      }
                    }
                "#,
            ),
            &environment,
        );
        assert!(matches!(result, IlinkEntitlementLoad::Ready(_)));
    }

    #[test]
    fn explicit_disable_wins_over_process_identity() {
        let environment = |name: &str| match name {
            ENV_APP_ID => Some("a3s-env-app".to_string()),
            ENV_BOT_TYPE => Some("3".to_string()),
            _ => None,
        };
        let result = load_from_source(
            Some("channels { weixin { enabled = false } }"),
            &environment,
        );
        let IlinkEntitlementLoad::Unavailable(blocker) = result else {
            panic!("explicit disable must win");
        };
        assert_eq!(blocker.code, "ilink_channel_disabled");
    }
}
