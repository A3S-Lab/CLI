//! Config-file discovery and the first-launch starter template.

pub(crate) mod persistence;
pub(crate) mod validation;

/// A starter A3S ACL `config.acl` with placeholders, generated on first
/// launch so a new user has something to edit instead of an error.
pub(crate) fn config_template() -> &'static str {
    r#"# A3S coding-agent config (A3S ACL).
# Fill in a provider apiKey/baseUrl and a model, set default_model, then save.
# Docs: https://a3s-lab.github.io/a3s/

default_model = "openai/my-model"

# Optional OS endpoint. Sign in with `a3s auth login os`.
# Signed-in models use `a3s-os/<model>`.
# os = "https://os.example.com"

# Optional: where local Skills are discovered (default ~/.a3s/skills).
# skill_dir = "~/.a3s/skills"

# Optional: where long-term memory is stored (default: workspace .a3s/memory).
# memory_dir = ".a3s/memory"
#
# Optional: tune memory extraction. LLM extraction is enabled by default and
# runs only after significant completed turns.
# memory {
#   llmExtraction = true
#   llmExtractionMaxItems = 5
#   llmExtractionMaxInputChars = 8000
# }

# Optional: a3s-search configuration. Without explicit engine entries,
# web_search uses the Core cascade: Moli headless discovery first, then
# HTTP/RSS and native API fallbacks. Engine entries replace the built-in
# selection, so AnySearch is used only when it is explicitly enabled here.
# Moli is the default headless backend and is discovered from the release
# sidecar or installed once into the shared Code cache. Chrome and Lightpanda
# remain explicit compatibility backends.
# search {
#   timeout = 20
#   engine {
#     ddg   { enabled = true  weight = 1.0 }
#     brave { enabled = true  weight = 1.0 }
#     wiki  { enabled = true  weight = 0.8 }
#     # anysearch { enabled = true weight = 1.0 } # opt in; ANYSEARCH_API_KEY is optional
#     # baidu  { enabled = true weight = 1.0 }
#     # bing_cn { enabled = true weight = 1.0 }
#   }
#   # headless { backend = "moli" max_tabs = 4 auto_download_moli = true }
#   # headless { backend = "chrome" max_tabs = 4 } # explicit compatibility
# }

providers "openai" {
  apiKey  = "sk-REPLACE-ME"
  baseUrl = "https://api.openai.com/v1/"   # or any OpenAI-compatible endpoint

  models "my-model" {
    name        = "My Model"
    toolCall    = true
    temperature = true
    modalities  = { input = ["text"], output = ["text"] }
    limit       = { context = 200000, output = 4096 }
  }
}
"#
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_template_documents_current_auth() {
        let template = config_template();
        assert!(!template.contains("codex login"));
        assert!(!template.contains("/login"));
        assert!(!template.contains("/logout"));
        assert!(!template.contains("codex/"));
        assert!(template.contains("a3s auth login os"));
        assert!(template.contains("a3s-os/<model>"));
        a3s_code_core::CodeConfig::from_acl(template).expect("starter template parses");
    }
}
