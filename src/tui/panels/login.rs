//! Detect a local Claude Code / Codex login so `/model` can surface those
//! accounts as tabs.

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum AuthProvider {
    Claude,
    Codex,
}

/// Claude models seen by the local Claude Code install. Project `"model"`
/// values are listed first, followed by recent usage stats.
pub(crate) fn claude_models() -> Vec<String> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::Path::new(&home);
        for path in [
            home.join(".claude.json"),
            home.join(".claude").join("stats-cache.json"),
        ] {
            if let Ok(txt) = std::fs::read_to_string(path) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&txt) {
                    collect_claude_models(&value, &mut out);
                }
            }
        }
    }
    if out.is_empty() {
        out.push("claude-sonnet-4".to_string());
    }
    out
}

fn collect_claude_models(value: &serde_json::Value, out: &mut Vec<String>) {
    fn push_model(out: &mut Vec<String>, model: &str) {
        let model = crate::claude::canonical_model_name(model);
        if model.starts_with("claude") && !out.iter().any(|m| m == &model) {
            out.push(model);
        }
    }

    fn walk_model_values(
        value: &serde_json::Value,
        parent_key: Option<&str>,
        out: &mut Vec<String>,
    ) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    if key == "model" {
                        if let Some(model) = child.as_str() {
                            push_model(out, model);
                        }
                    }
                    walk_model_values(child, Some(key), out);
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    walk_model_values(child, parent_key, out);
                }
            }
            serde_json::Value::String(model) if parent_key == Some("model") => {
                push_model(out, model);
            }
            _ => {}
        }
    }

    fn walk_usage_maps(
        value: &serde_json::Value,
        parent_key: Option<&str>,
        target_key: &str,
        out: &mut Vec<String>,
    ) {
        match value {
            serde_json::Value::Object(map) => {
                if parent_key == Some(target_key) {
                    for key in map.keys() {
                        push_model(out, key);
                    }
                }
                for (key, child) in map {
                    walk_usage_maps(child, Some(key), target_key, out);
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    walk_usage_maps(child, parent_key, target_key, out);
                }
            }
            _ => {}
        }
    }

    walk_model_values(value, None, out);
    walk_usage_maps(value, None, "lastModelUsage", out);
    walk_usage_maps(value, None, "tokensByModel", out);
}

/// True when the local Claude Code / Codex CLI has a stored login.
pub(crate) fn has_local_login(provider: AuthProvider) -> bool {
    match provider {
        AuthProvider::Claude => crate::claude::has_claude_login(),
        AuthProvider::Codex => {
            let Some(home) = std::env::var_os("HOME") else {
                return false;
            };
            let home = std::path::Path::new(&home);
            let path = home.join(".codex/auth.json");
            std::fs::read_to_string(path)
                .ok()
                .and_then(|txt| serde_json::from_str::<serde_json::Value>(&txt).ok())
                .map(|v| {
                    v.pointer("/tokens/access_token").is_some() || v.get("access_token").is_some()
                })
                .unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collects_claude_models_from_model_values_and_usage_maps() {
        let value = json!({
            "projects": {
                "/repo": {
                    "model": "claude-sonnet-4-6",
                    "lastModelUsage": {
                        "claude-opus-4-8[1m]": {"inputTokens": 10},
                        "claude-opus-4-8": {"inputTokens": 5},
                        "not-claude": {"inputTokens": 1}
                    }
                }
            },
            "dailyModelTokens": [
                {
                    "tokensByModel": {
                        "claude-haiku-4-5-20251001": 5,
                        "MiniMax-M2.7-highspeed": 5
                    }
                }
            ]
        });
        let mut models = Vec::new();

        collect_claude_models(&value, &mut models);

        assert_eq!(
            models,
            vec![
                "claude-sonnet-4-6",
                "claude-opus-4-8",
                "claude-haiku-4-5-20251001"
            ]
        );
    }
}
