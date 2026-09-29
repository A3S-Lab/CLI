use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The credential/provider boundary used to execute a model route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelSource {
    Config,
    OsGateway,
}

impl ModelSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Config => "config.acl",
            Self::OsGateway => "A3S OS",
        }
    }

    pub(crate) fn route_prefix(self) -> Option<&'static str> {
        match self {
            Self::Config => None,
            Self::OsGateway => Some("a3s-os"),
        }
    }
}

/// Stable, user-facing identity of a selectable model and its credential source.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ModelRoute {
    pub(crate) source: ModelSource,
    pub(crate) model: String,
}

impl ModelRoute {
    pub(crate) fn new(source: ModelSource, model: impl Into<String>) -> anyhow::Result<Self> {
        let model = model.into();
        validate_model_id(&model)?;
        Ok(Self { source, model })
    }

    pub(crate) fn id(&self) -> String {
        match self.source.route_prefix() {
            Some(prefix) => format!("{prefix}/{}", self.model),
            None if has_reserved_prefix(&self.model) => format!("config/{}", self.model),
            None => self.model.clone(),
        }
    }
}

impl fmt::Display for ModelRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.id())
    }
}

impl FromStr for ModelRoute {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if let Some(model) = value.strip_prefix("config/") {
            return Self::new(ModelSource::Config, model);
        }
        if let Some(model) = value.strip_prefix("a3s-os/") {
            return Self::new(ModelSource::OsGateway, model);
        }
        if let Some(prefix) = borrowed_login_prefix(value) {
            anyhow::bail!(
                "model route `{value}` borrows a local `{prefix}` login, which is no longer a model route; configure the provider in config.acl and select `provider/model`, or `config/{prefix}/<model>` when the ACL provider uses that name"
            );
        }
        Self::new(ModelSource::Config, value)
    }
}

fn borrowed_login_prefix(value: &str) -> Option<&'static str> {
    ["claude-code", "codex", "kimi", "workbuddy", "codebuddy"]
        .into_iter()
        .find(|prefix| value.starts_with(*prefix) && value[prefix.len()..].starts_with('/'))
}

fn has_reserved_prefix(model: &str) -> bool {
    [
        "config",
        "claude-code",
        "codex",
        "kimi",
        "workbuddy",
        "codebuddy",
        "a3s-os",
    ]
    .iter()
    .any(|prefix| model.split('/').next() == Some(prefix))
}

fn validate_model_id(model: &str) -> anyhow::Result<()> {
    let model = model.trim();
    if model.is_empty() {
        anyhow::bail!("model route is empty");
    }
    if model.chars().any(char::is_whitespace) {
        anyhow::bail!("model route cannot contain whitespace");
    }
    if model.starts_with('/') || model.ends_with('/') || model.contains("//") {
        anyhow::bail!("model route contains an empty path segment");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_roundtrip_config_and_os_prefixes() {
        for (text, source, model) in [
            ("openai/gpt-5", ModelSource::Config, "openai/gpt-5"),
            ("a3s-os/team/model", ModelSource::OsGateway, "team/model"),
            ("config/codex/custom", ModelSource::Config, "codex/custom"),
        ] {
            let route: ModelRoute = text.parse().unwrap();
            assert_eq!(route.source, source);
            assert_eq!(route.model, model);
            assert_eq!(route.to_string(), text);
        }
    }

    #[test]
    fn borrowed_login_prefixes_are_rejected() {
        for route in [
            "claude-code/claude-opus-4-6",
            "codex/gpt-5.2-codex",
            "kimi/kimi-for-coding",
            "workbuddy/glm-5.1",
            "codebuddy/glm-5.1",
        ] {
            let error = route.parse::<ModelRoute>().unwrap_err().to_string();
            assert!(
                error.contains("no longer a model route"),
                "accepted {route:?}: {error}"
            );
        }
    }

    #[test]
    fn malformed_routes_are_rejected() {
        for route in ["", "a3s-os/", "a3s-os//model", "model with spaces"] {
            assert!(route.parse::<ModelRoute>().is_err(), "accepted {route:?}");
        }
    }
}
