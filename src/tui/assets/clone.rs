//! Shared `git clone` helper for local asset source workspaces.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AssetCloneResult {
    pub(crate) family: &'static str,
    pub(crate) url: String,
    pub(crate) path: std::path::PathBuf,
}

pub(crate) async fn clone_asset_source(
    family: &'static str,
    url: String,
    root: std::path::PathBuf,
) -> Result<AssetCloneResult, String> {
    if !looks_like_git_url(&url) {
        return Err("expected a git URL".to_string());
    }
    let name = source_name_from_git_url(&url);
    let path = unique_clone_path(&root.join(name));
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("could not create {}: {e}", root.display()))?;
    let out = tokio::process::Command::new("git")
        .arg("clone")
        .arg(&url)
        .arg(&path)
        .output()
        .await
        .map_err(|e| format!("git clone failed to start: {e}"))?;
    if !out.status.success() {
        let mut text = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if text.is_empty() {
            text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        return Err(format!(
            "git clone failed: {}",
            super::util::truncate(&text, 220)
        ));
    }
    Ok(AssetCloneResult { family, url, path })
}

fn looks_like_git_url(url: &str) -> bool {
    let value = url.trim();
    value.starts_with("https://")
        || value.starts_with("http://")
        || value.starts_with("ssh://")
        || value.starts_with("git@")
        || value.starts_with("file://")
}

fn source_name_from_git_url(url: &str) -> String {
    let mut tail = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("asset")
        .trim_end_matches(".git")
        .to_string();
    tail.retain(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    if tail.trim_matches(['-', '_', '.']).is_empty() {
        "asset".to_string()
    } else {
        tail
    }
}

fn unique_clone_path(path: &std::path::Path) -> std::path::PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("asset");
    for i in 2..10_000 {
        let candidate = parent.join(format!("{stem}-{i}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_are_derived_from_common_git_urls() {
        assert_eq!(
            source_name_from_git_url("https://github.com/acme/weather-agent.git"),
            "weather-agent"
        );
        assert_eq!(
            source_name_from_git_url("git@github.com:acme/ops.skill.git"),
            "ops.skill"
        );
        assert_eq!(
            source_name_from_git_url("https://example.com/"),
            "example.com"
        );
    }

    #[test]
    fn git_url_detection_accepts_remote_and_file_urls() {
        assert!(looks_like_git_url("https://github.com/acme/a.git"));
        assert!(looks_like_git_url("git@github.com:acme/a.git"));
        assert!(looks_like_git_url("file:///tmp/a.git"));
        assert!(!looks_like_git_url("make a new agent"));
    }
}
