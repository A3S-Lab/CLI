//! Config-file discovery and the first-launch starter template.

/// A starter `config.acl` (HCL-like ACL) with placeholders, generated on first
/// launch so a new user has something to edit instead of an error.
pub(crate) fn config_template() -> &'static str {
    r#"# A3S coding-agent config (HCL-like ACL).
# Fill in your provider apiKey/baseUrl + a model, set default_model, then save
# with Ctrl+S. Docs: https://a3s-lab.github.io/a3s/

default_model = "openai/my-model"

# Optional OS endpoint. When set, a3s code enables /login and /logout.
# os = "https://os.example.com"

# Optional: where /flow DAG JSONs are stored (default ~/.a3s/flows).
# flow_dir = "~/.a3s/flows"

# Optional: where /agent agent definitions are stored (default ~/.a3s/agents).
# agent_dir = "~/.a3s/agents"

# Optional: where /mcp MCP server assets are stored (default ~/.a3s/mcps).
# mcp_dir = "~/.a3s/mcps"

# Optional: where /skill local skill assets are stored (default ~/.a3s/skills).
# skill_dir = "~/.a3s/skills"

# Optional: where long-term memory is stored (default ~/.a3s/memory).
# memory_dir = "~/.a3s/memory"
#
# Optional: tune memory extraction. LLM extraction is enabled by default and
# runs only after significant completed turns.
# memory {
#   llmExtraction = true
#   llmExtractionMaxItems = 5
#   llmExtractionMaxInputChars = 8000
# }

providers "openai" {
  apiKey  = "sk-REPLACE-ME"
  baseUrl = "https://api.openai.com/v1/"   # or any OpenAI-compatible endpoint

  models "my-model" {
    name        = "My Model"
    toolCall    = true
    temperature = true
    modalities  = { input = ["text"], output = ["text"] }
    limit       = { context = 128000, output = 4096 }
  }
}
"#
}

/// `~/.a3s/config.acl` — the default user-global config location.
pub(crate) fn default_config_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".a3s/config.acl"))
}

/// Where long-term memory is stored: `$A3S_MEMORY_DIR`, else a top-level
/// `memory_dir = "..."` / `memoryDir = "..."` in config.acl, else
/// `~/.a3s/memory`. Read at use time so `/config` edits take effect without a
/// restart, and so the TUI session and `/memory` panel browse the same store.
pub(crate) fn memory_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("A3S_MEMORY_DIR") {
        if !d.is_empty() {
            return std::path::PathBuf::from(d);
        }
    }
    if let Some(path) = find_config() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(d) =
                top_level_str(&text, "memory_dir").or_else(|| top_level_str(&text, "memoryDir"))
            {
                return expand_home(&d);
            }
        }
    }
    std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".a3s/memory"))
        .unwrap_or_else(|| std::path::PathBuf::from(".a3s/memory"))
}

/// Where `/flow` DAG JSONs are stored: `$A3S_FLOW_DIR`, else a top-level
/// `flow_dir = "…"` in config.acl, else `~/.a3s/flows`. Read at use time so a
/// `/config` edit takes effect without a restart.
pub(crate) fn flow_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("A3S_FLOW_DIR") {
        if !d.is_empty() {
            return std::path::PathBuf::from(d);
        }
    }
    if let Some(path) = find_config() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(d) = top_level_str(&text, "flow_dir") {
                return expand_home(&d);
            }
        }
    }
    std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".a3s/flows"))
        .unwrap_or_else(|| std::path::PathBuf::from(".a3s/flows"))
}

/// Where `/agent` definitions are stored: `$A3S_AGENT_DIR`, else a top-level
/// `agent_dir = "…"` in config.acl, else `~/.a3s/agents`. Read at use time so
/// a `/config` edit takes effect without a restart.
pub(crate) fn agent_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("A3S_AGENT_DIR") {
        if !d.is_empty() {
            return std::path::PathBuf::from(d);
        }
    }
    if let Some(path) = find_config() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(d) = top_level_str(&text, "agent_dir") {
                return expand_home(&d);
            }
        }
    }
    std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".a3s/agents"))
        .unwrap_or_else(|| std::path::PathBuf::from(".a3s/agents"))
}

/// Where `/mcp` assets are stored: `$A3S_MCP_DIR`, else a top-level
/// `mcp_dir = "..."` in config.acl, else `~/.a3s/mcps`. Read at use time so
/// a `/config` edit takes effect without a restart.
pub(crate) fn mcp_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("A3S_MCP_DIR") {
        if !d.is_empty() {
            return std::path::PathBuf::from(d);
        }
    }
    if let Some(path) = find_config() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(d) = top_level_str(&text, "mcp_dir") {
                return expand_home(&d);
            }
        }
    }
    std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".a3s/mcps"))
        .unwrap_or_else(|| std::path::PathBuf::from(".a3s/mcps"))
}

/// Where `/skill` skill assets are stored: `$A3S_SKILL_DIR`, else a top-level
/// `skill_dir = "..."` in config.acl, else `~/.a3s/skills`. Read at use time so
/// a `/config` edit takes effect without a restart.
pub(crate) fn skill_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("A3S_SKILL_DIR") {
        if !d.is_empty() {
            return std::path::PathBuf::from(d);
        }
    }
    if let Some(path) = find_config() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(d) = top_level_str(&text, "skill_dir") {
                return expand_home(&d);
            }
        }
    }
    std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".a3s/skills"))
        .unwrap_or_else(|| std::path::PathBuf::from(".a3s/skills"))
}

/// Extract a top-level `key = "value"` scalar from HCL-ish text. Only lines at
/// brace depth 0 count, so a same-named key inside a `providers { … }` block
/// can't shadow it. The core's CodeConfig ignores unknown keys, so the option
/// lives in the same config.acl without breaking its typed parse.
fn top_level_str(text: &str, key: &str) -> Option<String> {
    let mut depth = 0i64;
    for line in text.lines() {
        let t = line.trim();
        if depth == 0 && !t.starts_with('#') {
            if let Some(rest) = t.strip_prefix(key) {
                if let Some(v) = rest.trim_start().strip_prefix('=') {
                    let v = v.trim().trim_matches('"');
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
        depth += t.matches('{').count() as i64 - t.matches('}').count() as i64;
    }
    None
}

/// Expand a leading `~/` to `$HOME` (config values are user-typed paths).
fn expand_home(p: &str) -> std::path::PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = std::env::var_os("HOME") {
            return std::path::Path::new(&h).join(rest);
        }
    }
    std::path::PathBuf::from(p)
}

/// Write the starter config to `path` (creating parent dirs). Never overwrites.
pub(crate) fn write_template_config(path: &std::path::Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, config_template())
}

/// Find the A3S config: `$A3S_CONFIG_FILE`, then `.a3s/config.acl` walking up
/// from the current directory (project-local), then `~/.a3s/config.acl`
/// (user-global) — so `a3s code` works from anywhere once a global config exists.
pub(crate) fn find_config() -> Option<String> {
    if let Ok(p) = std::env::var("A3S_CONFIG_FILE") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir: Option<&std::path::Path> = Some(cwd.as_path());
        while let Some(d) = dir {
            let candidate = d.join(".a3s/config.acl");
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
            dir = d.parent();
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = std::path::Path::new(&home).join(".a3s/config.acl");
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_str_reads_only_depth_zero_keys() {
        let text = r#"
# flow_dir = "/commented/out"
providers "x" {
  flow_dir = "/inside/a/block"
}
flow_dir = "~/flows"
memory_dir = "~/memories"
memoryDir = "~/camel-memories"
"#;
        assert_eq!(top_level_str(text, "flow_dir").as_deref(), Some("~/flows"));
        assert_eq!(
            top_level_str(text, "memory_dir").as_deref(),
            Some("~/memories")
        );
        assert_eq!(
            top_level_str(text, "memoryDir").as_deref(),
            Some("~/camel-memories")
        );
        assert_eq!(top_level_str(text, "missing"), None);
        // A longer identifier sharing the prefix does not match.
        assert_eq!(top_level_str("flow_dirx = \"/y\"", "flow_dir"), None);
    }

    #[test]
    fn expand_home_resolves_tilde() {
        let home = std::env::var("HOME").expect("HOME set in tests");
        assert_eq!(
            expand_home("~/clones"),
            std::path::Path::new(&home).join("clones")
        );
        assert_eq!(expand_home("/abs/path"), std::path::Path::new("/abs/path"));
    }
}
