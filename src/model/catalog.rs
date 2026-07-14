use a3s_code_core::config::CodeConfig;
use std::path::Path;

use crate::account_providers::{codex, AccountProvider};
use crate::{a3s_os, config};

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
        catalog.add_local_account_models(AccountProvider::Claude);
        let os_config = code_config
            .as_ref()
            .and_then(|config| config.os.as_ref())
            .cloned();
        let (codex, workbuddy, os) = tokio::join!(
            discover_codex_models(refresh_remote),
            discover_account_models(AccountProvider::CodeBuddy, refresh_remote),
            discover_os_models(os_config)
        );
        catalog.extend(codex);
        catalog.extend(workbuddy);
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
            ModelSource::Claude | ModelSource::CodeBuddy => {
                let Some(provider) = route.source.account_provider() else {
                    return false;
                };
                if !provider.is_available() {
                    return false;
                }
                provider
                    .discover_models()
                    .await
                    .unwrap_or_else(|_| provider.local_models())
                    .iter()
                    .any(|model| provider.canonical_model(model) == route.model)
            }
            ModelSource::Codex => {
                codex::has_valid_codex_login()
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

    fn add_local_account_models(&mut self, provider: AccountProvider) {
        if !provider.is_available() {
            return;
        }
        let source = ModelSource::from_account_provider(provider);
        for model in provider.local_models() {
            let model = provider.canonical_model(&model);
            if let Ok(route) = ModelRoute::new(source, &model) {
                let context_window = provider.model_context(&model);
                self.entries.push(ModelEntry {
                    route,
                    display_name: model,
                    context_window,
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
    if !codex::has_valid_codex_login() {
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

async fn discover_account_models(provider: AccountProvider, refresh_remote: bool) -> Discovery {
    let mut discovery = Discovery::default();
    if !provider.is_available() {
        return discovery;
    }
    let models = if refresh_remote {
        match provider.discover_models().await {
            Ok(models) if !models.is_empty() => models,
            Ok(_) => {
                discovery.warnings.push(format!(
                    "{} returned no account models; using its compatibility list",
                    provider.label()
                ));
                provider.local_models()
            }
            Err(error) => {
                discovery.warnings.push(format!(
                    "{} model discovery failed; using its compatibility list: {error}",
                    provider.label()
                ));
                provider.local_models()
            }
        }
    } else {
        provider.local_models()
    };
    let source = ModelSource::from_account_provider(provider);
    for model in models {
        let model = provider.canonical_model(&model);
        if let Ok(route) = ModelRoute::new(source, &model) {
            discovery.entries.push(ModelEntry {
                route,
                display_name: model.clone(),
                context_window: provider.model_context(&model),
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
        ModelSource::CodeBuddy => 3,
        ModelSource::OsGateway => 4,
    }
}
