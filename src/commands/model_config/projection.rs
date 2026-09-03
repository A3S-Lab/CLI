use std::path::Path;

use a3s_acl::{Block, Document, Value as AclValue};
use a3s_code_core::config::{ModelConfig, ProviderConfig};
use a3s_code_core::CodeConfig;
use serde_json::{json, Value};

pub(super) fn configuration_projection(
    path: &Path,
    scope: &str,
    source: &str,
    config: &CodeConfig,
) -> anyhow::Result<Value> {
    let document = if source.trim().is_empty() {
        Document::default()
    } else {
        a3s_acl::parse_acl(source)?
    };
    let providers = config
        .providers
        .iter()
        .map(|provider| {
            let raw = provider_block(&document, &provider.name);
            provider_projection(provider, raw)
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "configPath": path,
        "scope": scope,
        "defaultModel": config.default_model,
        "runtime": {
            "thinkingBudget": config.thinking_budget,
            "llmApiTimeoutMs": config.llm_api_timeout_ms,
        },
        "providers": providers,
    }))
}

fn provider_projection(provider: &ProviderConfig, raw: Option<&Block>) -> Value {
    let credential = secret_state(
        raw.and_then(|block| attribute(block, &["api_key", "apiKey"])),
        false,
    );
    let inherited_credential_available = credential
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let models = provider
        .models
        .iter()
        .map(|model| {
            let raw_model = raw.and_then(|block| model_block(block, &model.id));
            model_projection(model, raw_model, inherited_credential_available)
        })
        .collect::<Vec<_>>();
    json!({
        "id": provider.name,
        "protocol": provider_protocol(&provider.name),
        "baseUrl": provider.base_url,
        "credential": credential,
        "sessionIdHeader": provider.session_id_header,
        "headers": header_states(raw.and_then(|block| attribute(block, &["headers"]))),
        "models": models,
    })
}

fn model_projection(
    model: &ModelConfig,
    raw: Option<&Block>,
    inherited_credential_available: bool,
) -> Value {
    json!({
        "id": model.id,
        "name": model.name,
        "family": model.family,
        "credential": model_secret_state(
            raw.and_then(|block| attribute(block, &["api_key", "apiKey"])),
            inherited_credential_available,
        ),
        "baseUrl": model.base_url,
        "sessionIdHeader": model.session_id_header,
        "headers": header_states(raw.and_then(|block| attribute(block, &["headers"]))),
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
            "input": optional_cost(model.cost.input),
            "output": optional_cost(model.cost.output),
            "cacheRead": optional_cost(model.cost.cache_read),
            "cacheWrite": optional_cost(model.cost.cache_write),
        },
        "limit": {
            "context": optional_limit(model.limit.context),
            "output": optional_limit(model.limit.output),
        },
    })
}

fn provider_protocol(provider: &str) -> &'static str {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openai" | "gpt" => "openai",
        "anthropic" | "claude" => "anthropic",
        "glm" | "zhipu" | "bigmodel" => "zhipu",
        _ => "openaiCompatible",
    }
}

fn provider_block<'a>(document: &'a Document, id: &str) -> Option<&'a Block> {
    document
        .blocks
        .iter()
        .find(|block| block.name == "providers" && block.labels.first().is_some_and(|v| v == id))
}

fn model_block<'a>(provider: &'a Block, id: &str) -> Option<&'a Block> {
    provider
        .blocks
        .iter()
        .find(|block| block.name == "models" && block.labels.first().is_some_and(|v| v == id))
}

fn attribute<'a>(block: &'a Block, names: &[&str]) -> Option<&'a AclValue> {
    names.iter().find_map(|name| block.attributes.get(*name))
}

fn model_secret_state(value: Option<&AclValue>, inherited_available: bool) -> Value {
    if value.is_none() {
        return json!({
            "configured": false,
            "available": inherited_available,
            "source": "inherited",
            "reference": Value::Null,
        });
    }
    secret_state(value, false)
}

fn secret_state(value: Option<&AclValue>, inherited: bool) -> Value {
    match value {
        Some(AclValue::Call(name, args)) if name == "env" => {
            let reference = args.first().and_then(|value| match value {
                AclValue::String(value) => Some(value.as_str()),
                _ => None,
            });
            json!({
                "configured": true,
                "available": reference.is_some_and(|name| std::env::var_os(name).is_some()),
                "source": "environment",
                "reference": reference,
            })
        }
        Some(AclValue::String(value)) => json!({
            "configured": !value.is_empty(),
            "available": !value.is_empty(),
            "source": "inline",
            "reference": Value::Null,
        }),
        Some(_) => json!({
            "configured": true,
            "available": false,
            "source": "unsupported",
            "reference": Value::Null,
        }),
        None => json!({
            "configured": false,
            "available": inherited,
            "source": if inherited { "inherited" } else { "none" },
            "reference": Value::Null,
        }),
    }
}

fn header_states(value: Option<&AclValue>) -> Vec<Value> {
    let Some(AclValue::Object(entries)) = value else {
        return Vec::new();
    };
    let mut headers = entries
        .iter()
        .map(|(name, value)| {
            let mut state = secret_state(Some(value), false);
            if let Some(object) = state.as_object_mut() {
                object.insert("name".to_string(), json!(name));
            }
            state
        })
        .collect::<Vec<_>>();
    headers.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .cmp(&right.get("name").and_then(Value::as_str))
    });
    headers
}

fn optional_limit(value: u32) -> Value {
    if value == 0 {
        Value::Null
    } else {
        json!(value)
    }
}

fn optional_cost(value: f64) -> Value {
    if value == 0.0 {
        Value::Null
    } else {
        json!(value)
    }
}
