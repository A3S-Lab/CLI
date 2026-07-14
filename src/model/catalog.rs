use a3s_code_core::config::CodeConfig;
use serde_json::Value;
use std::path::Path;

use crate::{a3s_os, claude, codex, config};

use super::route::{ModelRoute, ModelSource};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelEntry {
    pub(crate) route: ModelRoute,
    pub(crate) display_name: String,
    pub(crate) context_window: Option<u32>,
    pub(crate) reasoning: bool,
    pub(crate) tool_call: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ModelCatalog {
    pub(crate) entries: Vec<ModelEntry>,
    pub(crate) warnings: Vec<String>,
    pub(crate) config_default: Option<String>,
}

impl ModelCatalog {
    pub(crate) async fn discover(refresh_remote: bool) -> Self {
        let mut catalog = Self::default();
        let code_config = load_config(&mut catalog.warnings);
        if let Some(config) = code_config.as_ref() {
            catalog.config_default = config.default_model.clone();
            catalog.add_config_models(config);
        }
        catalog.add_claude_models();
        let os_config = code_config
            .as_ref()
            .and_then(|config| config.os.as_ref())
            .cloned();
        let (codex, os) = tokio::join!(
            discover_codex_models(refresh_remote),
            discover_os_models(os_config)
        );
        catalog.extend(codex);
        catalog.extend(os);
        catalog.sort_and_deduplicate();
        catalog
    }

    /// Validate one route without probing unrelated credential sources.
    pub(crate) async fn route_available(route: &ModelRoute) -> bool {
        match route.source {
            ModelSource::Config => {
                let mut warnings = Vec::new();
                let Some(config) = load_config(&mut warnings) else {
                    return false;
                };
                config.list_models().into_iter().any(|(provider, model)| {
                    format!("{}/{}", provider.name, model.id) == route.model
                })
            }
            ModelSource::Claude => {
                claude::has_claude_login() && claude_models().contains(&route.model)
            }
            ModelSource::Codex => {
                codex::has_codex_login()
                    && codex::cached_codex_models()
                        .iter()
                        .any(|model| model.slug == route.model)
            }
            ModelSource::OsGateway => {
                discover_os_models(load_config(&mut Vec::new()).and_then(|config| config.os))
                    .await
                    .entries
                    .iter()
                    .any(|entry| &entry.route == route)
            }
        }
    }

    fn extend(&mut self, discovery: Discovery) {
        self.entries.extend(discovery.entries);
        self.warnings.extend(discovery.warnings);
    }

    fn add_config_models(&mut self, config: &CodeConfig) {
        for (provider, model) in config.list_models() {
            let route = match ModelRoute::new(
                ModelSource::Config,
                format!("{}/{}", provider.name, model.id),
            ) {
                Ok(route) => route,
                Err(error) => {
                    self.warnings.push(format!(
                        "ignored invalid config model {}/{}: {error}",
                        provider.name, model.id
                    ));
                    continue;
                }
            };
            self.entries.push(ModelEntry {
                route,
                display_name: if model.name.trim().is_empty() {
                    model.id.clone()
                } else {
                    model.name.clone()
                },
                context_window: (model.limit.context > 0).then_some(model.limit.context),
                reasoning: model.reasoning,
                tool_call: model.tool_call,
            });
        }
    }

    fn add_claude_models(&mut self) {
        if !claude::has_claude_login() {
            return;
        }
        for model in claude_models() {
            if let Ok(route) = ModelRoute::new(ModelSource::Claude, &model) {
                self.entries.push(ModelEntry {
                    route,
                    display_name: model,
                    context_window: None,
                    reasoning: true,
                    tool_call: true,
                });
            }
        }
    }

    fn sort_and_deduplicate(&mut self) {
        self.entries.sort_by(|left, right| {
            source_rank(left.route.source)
                .cmp(&source_rank(right.route.source))
                .then_with(|| left.route.model.cmp(&right.route.model))
        });
        self.entries
            .dedup_by(|left, right| left.route == right.route);
    }
}

#[derive(Debug, Default)]
struct Discovery {
    entries: Vec<ModelEntry>,
    warnings: Vec<String>,
}

async fn discover_codex_models(refresh_remote: bool) -> Discovery {
    let mut discovery = Discovery::default();
    if !codex::has_codex_login() {
        return discovery;
    }
    let models = if refresh_remote {
        match codex::refresh_codex_models().await {
            Ok(models) => models,
            Err(error) => {
                discovery
                    .warnings
                    .push(format!("Codex model refresh failed; using cache: {error}"));
                codex::cached_codex_models()
            }
        }
    } else {
        codex::cached_codex_models()
    };
    for model in models {
        if let Ok(route) = ModelRoute::new(ModelSource::Codex, &model.slug) {
            discovery.entries.push(ModelEntry {
                route,
                display_name: model.slug,
                context_window: model.context_window,
                reasoning: true,
                tool_call: true,
            });
        }
    }
    discovery
}

async fn discover_os_models(os_config: Option<a3s_code_core::config::OsConfig>) -> Discovery {
    let mut discovery = Discovery::default();
    let Some(os_config) = os_config else {
        return discovery;
    };
    let Some(mut session) = a3s_os::current_session(&os_config) else {
        return discovery;
    };
    if a3s_os::needs_refresh(&session) {
        match a3s_os::refresh_session(&session).await {
            Ok(refreshed) => session = refreshed,
            Err(error) => {
                discovery
                    .warnings
                    .push(format!("A3S OS session refresh failed: {error}"));
                return discovery;
            }
        }
    }
    match a3s_os::fetch_gateway_models(&session.address, &session.access_token).await {
        Ok(models) => {
            for model in models {
                if let Ok(route) = ModelRoute::new(ModelSource::OsGateway, &model.id) {
                    discovery.entries.push(ModelEntry {
                        route,
                        display_name: model.id,
                        context_window: model.context,
                        reasoning: true,
                        tool_call: true,
                    });
                }
            }
        }
        Err(error) => discovery
            .warnings
            .push(format!("A3S OS model discovery failed: {error}")),
    }
    discovery
}

fn load_config(warnings: &mut Vec<String>) -> Option<CodeConfig> {
    let path = config::find_config()?;
    match CodeConfig::from_file(Path::new(&path)) {
        Ok(config) => Some(config),
        Err(error) => {
            warnings.push(format!("could not parse {path}: {error}"));
            None
        }
    }
}

fn source_rank(source: ModelSource) -> usize {
    match source {
        ModelSource::Config => 0,
        ModelSource::Claude => 1,
        ModelSource::Codex => 2,
        ModelSource::OsGateway => 3,
    }
}

/// Claude models observed in local Claude Code project state and usage caches.
pub(crate) fn claude_models() -> Vec<String> {
    let mut models = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        for path in [
            home.join(".claude.json"),
            home.join(".claude").join("stats-cache.json"),
        ] {
            if let Ok(raw) = std::fs::read_to_string(path) {
                if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                    collect_claude_models(&value, &mut models);
                }
            }
        }
    }
    if models.is_empty() {
        models.push("claude-sonnet-4".to_string());
    }
    models
}

fn collect_claude_models(value: &Value, models: &mut Vec<String>) {
    fn push(models: &mut Vec<String>, candidate: &str) {
        let model = claude::canonical_model_name(candidate);
        if model.starts_with("claude") && !models.contains(&model) {
            models.push(model);
        }
    }

    fn walk(value: &Value, parent: Option<&str>, models: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if matches!(parent, Some("lastModelUsage" | "tokensByModel")) {
                    for model in map.keys() {
                        push(models, model);
                    }
                }
                for (key, child) in map {
                    if key == "model" {
                        if let Some(model) = child.as_str() {
                            push(models, model);
                        }
                    }
                    walk(child, Some(key), models);
                }
            }
            Value::Array(items) => {
                for child in items {
                    walk(child, parent, models);
                }
            }
            Value::String(model) if parent == Some("model") => push(models, model),
            _ => {}
        }
    }
    walk(value, None, models);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collects_claude_models_without_duplicates() {
        let value = json!({
            "projects": [{"model": "claude-opus-4-6"}],
            "lastModelUsage": {"claude-sonnet-4": 10},
            "tokensByModel": {"not-claude": 1, "claude-sonnet-4": 2}
        });
        let mut models = Vec::new();
        collect_claude_models(&value, &mut models);
        assert!(models.contains(&"claude-opus-4-6".to_string()));
        assert_eq!(
            models
                .iter()
                .filter(|model| model.as_str() == "claude-sonnet-4")
                .count(),
            1
        );
        assert!(!models.contains(&"not-claude".to_string()));
    }
}
