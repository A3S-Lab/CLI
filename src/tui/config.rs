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

/// `~/.a3s/memory` — the agent's long-term memory store (created on demand).
pub(crate) fn memory_dir() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".a3s/memory"))
        .unwrap_or_else(|| std::path::PathBuf::from(".a3s/memory"))
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
