use std::collections::HashMap;
use std::sync::Arc;

use a3s_boot::{BootError, Result as BootResult};
use a3s_code_core::mcp::{McpServerConfig, McpTransportConfig};
use a3s_code_core::{ModelConfig, ProviderConfig};
use serde::Serialize;
use serde_json::{json, Value};

use crate::api::code_web::state::CodeWebState;
use crate::config;

pub(in crate::api::code_web) struct ConfigService {
    state: Arc<CodeWebState>,
}

impl ConfigService {
    pub(in crate::api::code_web) fn new(state: Arc<CodeWebState>) -> Self {
        Self { state }
    }

    pub(in crate::api::code_web) fn system_info(&self) -> serde_json::Value {
        json!({
            "appName": "书小安",
            "logoUrl": "/logo.png",
            "version": env!("CARGO_PKG_VERSION"),
        })
    }

    pub(in crate::api::code_web) fn assistant_settings(&self) -> serde_json::Value {
        let code_config = self.state.code_config_snapshot();
        json!({
            "name": "书小安",
            "avatar": "",
            "description": "A3S Code local assistant",
            "model": code_config.default_model,
        })
    }

    pub(in crate::api::code_web) fn app_settings(&self) -> serde_json::Value {
        let code_config = self.state.code_config_snapshot();
        let workspace = self.state.default_workspace.display().to_string();
        let storage_path = config::memory_dir().display().to_string();
        json!({
            "general": {
                "appName": "书小安",
                "language": "zh-CN",
                "splashScreen": true,
                "restoreWorkspace": true,
                "workspacePath": workspace,
            },
            "appearance": {
                "theme": "system",
                "sideBarPosition": "left",
                "statusBar": true,
                "activityBar": true,
                "zoomLevel": 1,
            },
            "editor": {},
            "llm": Self::llm_settings_from_config(&code_config),
            "ocr": {
                "defaultBackend": "",
                "backends": [],
            },
            "security": {
                "allowTelemetry": false,
                "checkUpdates": true,
            },
            "network": {
                "connectionTimeout": 30000,
                "readTimeout": 30000,
                "proxyPool": [],
            },
            "search": {
                "enabledEngines": ["ddg"],
                "language": "zh-CN",
                "safesearch": "moderate",
                "timeout": 10,
                "limit": 8,
            },
            "storage": {
                "defaultProvider": "local",
                "localStoragePath": storage_path,
            },
        })
    }

    pub(in crate::api::code_web) fn config_category(
        &self,
        name: &str,
    ) -> BootResult<serde_json::Value> {
        let normalized = normalize_category_name(name);
        if normalized == "llm" || normalized == "ai" {
            let code_config = self.state.code_config_snapshot();
            return Ok(Self::llm_settings_from_config(&code_config));
        }

        let settings = self.app_settings();
        Ok(settings
            .get(normalized)
            .cloned()
            .unwrap_or_else(|| json!({})))
    }

    pub(in crate::api::code_web) fn update_app_settings(
        &self,
        patch: Value,
    ) -> BootResult<serde_json::Value> {
        if let Some(llm) = patch.get("llm").or_else(|| patch.get("ai")) {
            self.update_llm_settings(llm)?;
        }
        Ok(self.app_settings())
    }

    pub(in crate::api::code_web) fn update_config_category(
        &self,
        name: &str,
        patch: Value,
    ) -> BootResult<serde_json::Value> {
        let normalized = normalize_category_name(name);
        if normalized == "llm" || normalized == "ai" {
            self.update_llm_settings(&patch)?;
        }
        self.config_category(normalized)
    }

    pub(in crate::api::code_web) fn llm_diagnostics(&self) -> serde_json::Value {
        let code_config = self.state.code_config_snapshot();
        let default_model = code_config.default_model.clone().unwrap_or_default();
        let default_ref = split_model_ref(&default_model);
        let mut issues = Vec::new();

        if code_config.providers.is_empty() {
            issues.push("no providers are configured".to_string());
        }
        if default_model.is_empty() {
            issues.push("defaultModel is not configured".to_string());
        } else if let Some((provider_name, model_id)) = default_ref.as_ref() {
            match code_config
                .providers
                .iter()
                .find(|provider| provider.name == *provider_name)
            {
                Some(provider) if provider.models.iter().any(|model| model.id == *model_id) => {}
                Some(_) => issues.push(format!(
                    "default model `{model_id}` was not found in provider `{provider_name}`"
                )),
                None => issues.push(format!("default provider `{provider_name}` was not found")),
            }
        }

        let providers = code_config
            .providers
            .iter()
            .map(|provider| {
                json!({
                    "name": provider.name,
                    "modelCount": provider.models.len(),
                    "apiKeyConfigured": has_secret(provider.api_key.as_deref()),
                    "baseUrlConfigured": provider.base_url.as_deref().is_some_and(|value| !value.trim().is_empty()),
                    "isDefault": default_ref
                        .as_ref()
                        .is_some_and(|(provider_name, _)| provider_name == &provider.name),
                })
            })
            .collect::<Vec<_>>();

        json!({
            "ok": issues.is_empty(),
            "defaultModel": default_model,
            "providers": providers,
            "issues": issues,
        })
    }

    pub(in crate::api::code_web) fn model_catalog(&self) -> serde_json::Value {
        model_catalog_from_config(&self.state.code_config_snapshot())
    }

    pub(in crate::api::code_web) fn fetch_provider_models(
        &self,
        request: Value,
    ) -> BootResult<serde_json::Value> {
        let provider_name = request
            .get("providerName")
            .or_else(|| request.get("provider_name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| BootError::BadRequest("providerName is required".to_string()))?;

        let code_config = self.state.code_config_snapshot();
        let provider = code_config
            .providers
            .iter()
            .find(|provider| provider.name == provider_name);
        let base_url = request
            .get("baseUrl")
            .or_else(|| request.get("base_url"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| provider.and_then(|provider| provider.base_url.clone()))
            .unwrap_or_default();
        let models = provider
            .map(|provider| {
                provider
                    .models
                    .iter()
                    .map(|model| {
                        json!({
                            "id": model.id,
                            "name": display_model_name(model),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(json!({
            "providerName": provider_name,
            "baseUrl": base_url,
            "models": models,
        }))
    }

    fn llm_settings_from_config(config: &a3s_code_core::CodeConfig) -> serde_json::Value {
        json!({
            "defaultModel": config.default_model.clone().unwrap_or_default(),
            "providers": config
                .providers
                .iter()
                .map(provider_settings)
                .collect::<Vec<_>>(),
            "mcpServers": sanitized_json_value(&config.mcp_servers),
            "maxToolRounds": config.max_tool_rounds,
            "maxParallelTasks": config.max_parallel_tasks,
            "autoParallel": config.auto_parallel,
            "thinkingBudget": config.thinking_budget,
            "llmApiTimeoutMs": config.llm_api_timeout_ms,
        })
    }

    fn update_llm_settings(&self, value: &Value) -> BootResult<()> {
        let mut config = self
            .state
            .code_config
            .write()
            .map_err(|_| BootError::Internal("code config lock was poisoned".to_string()))?;

        if let Some(default_model) = value
            .get("defaultModel")
            .or_else(|| value.get("default_model"))
            .and_then(Value::as_str)
            .map(str::trim)
        {
            config.default_model = (!default_model.is_empty()).then(|| default_model.to_string());
        }

        if let Some(providers_value) = value.get("providers") {
            let mut providers: Vec<ProviderConfig> =
                serde_json::from_value(providers_value.clone()).map_err(|error| {
                    BootError::BadRequest(format!("invalid llm.providers: {error}"))
                })?;
            preserve_provider_secrets(&mut providers, &config.providers);
            config.providers = providers;
        }

        if let Some(mcp_servers_value) =
            value.get("mcpServers").or_else(|| value.get("mcp_servers"))
        {
            let mut servers: Vec<McpServerConfig> =
                serde_json::from_value(mcp_servers_value.clone()).map_err(|error| {
                    BootError::BadRequest(format!("invalid llm.mcpServers: {error}"))
                })?;
            preserve_mcp_secrets(&mut servers, &config.mcp_servers);
            config.mcp_servers = servers;
        }

        if let Some(max_tool_rounds) = optional_usize(
            value
                .get("maxToolRounds")
                .or_else(|| value.get("max_tool_rounds")),
        ) {
            config.max_tool_rounds = Some(max_tool_rounds);
        }
        if let Some(max_parallel_tasks) = optional_usize(
            value
                .get("maxParallelTasks")
                .or_else(|| value.get("max_parallel_tasks")),
        ) {
            config.max_parallel_tasks = Some(max_parallel_tasks);
        }
        if let Some(thinking_budget) = optional_usize(
            value
                .get("thinkingBudget")
                .or_else(|| value.get("thinking_budget")),
        ) {
            config.thinking_budget = Some(thinking_budget);
        }
        if let Some(timeout_ms) = optional_u64(
            value
                .get("llmApiTimeoutMs")
                .or_else(|| value.get("llm_api_timeout_ms")),
        ) {
            config.llm_api_timeout_ms = Some(timeout_ms);
        }
        if let Some(auto_parallel) = optional_bool(
            value
                .get("autoParallel")
                .or_else(|| value.get("auto_parallel")),
        ) {
            config.auto_parallel = Some(auto_parallel);
        }

        Ok(())
    }
}

const REDACTED_SECRET: &str = "[configured]";

fn normalize_category_name(name: &str) -> &str {
    name.trim()
}

fn provider_settings(provider: &ProviderConfig) -> serde_json::Value {
    json!({
        "name": provider.name,
        "apiKey": redact_secret(provider.api_key.as_deref()),
        "baseUrl": provider.base_url,
        "headers": redact_headers(&provider.headers),
        "sessionIdHeader": provider.session_id_header,
        "models": provider
            .models
            .iter()
            .map(model_settings)
            .collect::<Vec<_>>(),
    })
}

fn model_settings(model: &ModelConfig) -> serde_json::Value {
    json!({
        "id": model.id,
        "name": display_model_name(model),
        "family": optional_text(&model.family),
        "apiKey": redact_secret(model.api_key.as_deref()),
        "baseUrl": model.base_url,
        "headers": redact_headers(&model.headers),
        "sessionIdHeader": model.session_id_header,
        "attachment": model.attachment,
        "reasoning": model.reasoning,
        "toolCall": model.tool_call,
        "temperature": model.temperature,
        "releaseDate": model.release_date,
        "modalities": {
            "input": model.modalities.input,
            "output": model.modalities.output,
        },
        "cost": {
            "input": model.cost.input,
            "output": model.cost.output,
            "cacheRead": model.cost.cache_read,
            "cacheWrite": model.cost.cache_write,
        },
        "limit": {
            "context": model.limit.context,
            "output": model.limit.output,
        },
    })
}

fn display_model_name(model: &ModelConfig) -> &str {
    optional_text(&model.name).unwrap_or(model.id.as_str())
}

fn model_catalog_from_config(config: &a3s_code_core::CodeConfig) -> serde_json::Value {
    let items = config
        .providers
        .iter()
        .flat_map(|provider| {
            provider.models.iter().map(move |model| {
                json!({
                    "id": format!("{}/{}", provider.name, model.id),
                    "name": display_model_name(model),
                    "source": provider.name,
                    "contextWindow": (model.limit.context > 0).then_some(model.limit.context),
                    "reasoning": model.reasoning,
                    "toolCall": model.tool_call,
                })
            })
        })
        .collect::<Vec<_>>();
    let default_model = config.default_model.clone();
    let mut warnings = Vec::new();
    if items.is_empty() {
        warnings
            .push("No models are configured. Add a provider and model in Settings.".to_string());
    }
    if let Some(default_model) = default_model.as_deref() {
        let default_exists = items
            .iter()
            .any(|item| item.get("id").and_then(Value::as_str) == Some(default_model));
        if !default_exists {
            warnings.push(format!(
                "The default model `{default_model}` is not present in the configured model catalog."
            ));
        }
    }

    json!({
        "items": items,
        "warnings": warnings,
        "defaultModel": default_model,
    })
}

fn optional_text(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn redact_secret(value: Option<&str>) -> Option<&'static str> {
    value
        .filter(|secret| has_secret(Some(secret)))
        .map(|_| REDACTED_SECRET)
}

fn has_secret(value: Option<&str>) -> bool {
    value.is_some_and(|secret| !secret.trim().is_empty())
}

fn redact_headers(headers: &HashMap<String, String>) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(key, value)| {
            if is_sensitive_key(key) && !value.trim().is_empty() {
                (key.clone(), REDACTED_SECRET.to_string())
            } else {
                (key.clone(), value.clone())
            }
        })
        .collect()
}

fn sanitized_json_value<T: Serialize>(value: &T) -> serde_json::Value {
    let mut value = serde_json::to_value(value).unwrap_or_else(|_| json!([]));
    redact_secrets_in_value(&mut value);
    value
}

fn redact_secrets_in_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if is_sensitive_key(key) {
                    if child.as_str().is_some_and(|text| !text.trim().is_empty()) {
                        *child = Value::String(REDACTED_SECRET.to_string());
                    } else {
                        redact_secrets_in_value(child);
                    }
                } else {
                    redact_secrets_in_value(child);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                redact_secrets_in_value(child);
            }
        }
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>();
    normalized.contains("apikey")
        || normalized.contains("authorization")
        || normalized.contains("accesstoken")
        || normalized.contains("refreshtoken")
        || normalized.contains("clientsecret")
        || normalized.contains("secretkey")
        || normalized == "token"
        || normalized == "password"
        || normalized.ends_with("token")
        || normalized.ends_with("secret")
}

fn split_model_ref(value: &str) -> Option<(String, String)> {
    let (provider, model) = value.split_once('/')?;
    let provider = provider.trim();
    let model = model.trim();
    if provider.is_empty() || model.is_empty() {
        None
    } else {
        Some((provider.to_string(), model.to_string()))
    }
}

fn optional_usize(value: Option<&Value>) -> Option<usize> {
    match value {
        Some(Value::Number(number)) => number.as_u64().map(|value| value as usize),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

fn optional_u64(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

fn optional_bool(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::String(text)) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Some(true),
            "false" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn preserve_provider_secrets(providers: &mut [ProviderConfig], existing: &[ProviderConfig]) {
    for provider in providers {
        let Some(previous) = existing.iter().find(|item| item.name == provider.name) else {
            continue;
        };
        preserve_secret(&mut provider.api_key, previous.api_key.as_deref());
        preserve_headers(&mut provider.headers, &previous.headers);
        for model in &mut provider.models {
            let Some(previous_model) = previous.models.iter().find(|item| item.id == model.id)
            else {
                continue;
            };
            preserve_secret(&mut model.api_key, previous_model.api_key.as_deref());
            preserve_headers(&mut model.headers, &previous_model.headers);
        }
    }
}

fn preserve_mcp_secrets(servers: &mut [McpServerConfig], existing: &[McpServerConfig]) {
    for server in servers {
        let Some(previous) = existing.iter().find(|item| item.name == server.name) else {
            continue;
        };
        preserve_headers(&mut server.env, &previous.env);
        preserve_mcp_transport_secrets(&mut server.transport, &previous.transport);
    }
}

fn preserve_mcp_transport_secrets(
    transport: &mut McpTransportConfig,
    previous: &McpTransportConfig,
) {
    match (transport, previous) {
        (
            McpTransportConfig::Http { headers, .. },
            McpTransportConfig::Http {
                headers: previous_headers,
                ..
            },
        )
        | (
            McpTransportConfig::StreamableHttp { headers, .. },
            McpTransportConfig::StreamableHttp {
                headers: previous_headers,
                ..
            },
        ) => preserve_headers(headers, previous_headers),
        _ => {}
    }
}

fn preserve_secret(value: &mut Option<String>, previous: Option<&str>) {
    let should_restore = match value.as_deref().map(str::trim) {
        None | Some("") => previous.is_some_and(|text| !text.trim().is_empty()),
        Some(REDACTED_SECRET) => true,
        Some(_) => false,
    };
    if should_restore {
        *value = previous.map(str::to_string);
    }
}

fn preserve_headers(headers: &mut HashMap<String, String>, previous: &HashMap<String, String>) {
    for (key, previous_value) in previous {
        match headers.get_mut(key) {
            Some(value) if value.trim() == REDACTED_SECRET => {
                *value = previous_value.clone();
            }
            Some(_) => {}
            None if !previous_value.trim().is_empty() => {
                headers.insert(key.clone(), previous_value.clone());
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::model_catalog_from_config;

    #[test]
    fn model_catalog_uses_qualified_ids_and_provider_sources() {
        let config: a3s_code_core::CodeConfig = serde_json::from_value(serde_json::json!({
            "defaultModel": "openai/gpt-test",
            "providers": [{
                "name": "openai",
                "models": [{
                    "id": "gpt-test",
                    "name": "GPT Test",
                    "reasoning": true,
                    "toolCall": true,
                    "limit": { "context": 128000 }
                }]
            }]
        }))
        .expect("valid config");

        let catalog = model_catalog_from_config(&config);
        assert_eq!(catalog["defaultModel"], "openai/gpt-test");
        assert_eq!(catalog["items"][0]["id"], "openai/gpt-test");
        assert_eq!(catalog["items"][0]["source"], "openai");
        assert_eq!(catalog["items"][0]["contextWindow"], 128000);
        assert_eq!(catalog["items"][0]["reasoning"], true);
        assert_eq!(catalog["warnings"].as_array().map(Vec::len), Some(0));
    }
}
