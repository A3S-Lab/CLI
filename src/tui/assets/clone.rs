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
    let root = absolute_clone_root(root)?;
    let name = source_name_from_git_url(&url);
    let path = unique_clone_path(&root.join(name));
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("could not create {}: {e}", root.display()))?;
    let out = tokio::process::Command::new("git")
        .arg("clone")
        .arg(&url)
        .arg(&path)
        .current_dir(&root)
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

fn absolute_clone_root(root: std::path::PathBuf) -> Result<std::path::PathBuf, String> {
    if root.is_absolute() {
        return Ok(root);
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(&root))
        .map_err(|e| format!("could not resolve clone root {}: {e}", root.display()))
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
    use std::process::Command;

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("a3s-code-clone-{name}-{}", std::process::id()))
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|err| panic!("git {args:?} failed to start: {err}"));
        assert!(
            output.status.success(),
            "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

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

    #[tokio::test]
    async fn clone_asset_source_clones_local_file_repo_and_uses_unique_path() {
        if !git_available() {
            return;
        }
        let root = temp_root("local-file-repo");
        let _ = std::fs::remove_dir_all(&root);
        let source = root.join("weather-agent.git");
        let clones = root.join("clones");
        std::fs::create_dir_all(&source).unwrap();
        run_git(&source, &["init"]);
        std::fs::write(source.join("README.md"), "weather agent\n").unwrap();
        run_git(&source, &["add", "README.md"]);
        run_git(
            &source,
            &[
                "-c",
                "user.name=A3S Test",
                "-c",
                "user.email=a3s-test@example.invalid",
                "commit",
                "-m",
                "init",
            ],
        );

        let url = format!("file://{}", source.display());
        let first = clone_asset_source("agent", url.clone(), clones.clone())
            .await
            .expect("first local clone should succeed");
        let second = clone_asset_source("agent", url.clone(), clones)
            .await
            .expect("second local clone should pick a unique path");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(first.family, "agent");
        assert_eq!(first.url, url);
        assert!(first.path.ends_with("weather-agent"));
        assert!(second.path.ends_with("weather-agent-2"));
        assert_ne!(first.path, second.path);
    }

    #[tokio::test]
    async fn clone_asset_source_rejects_non_git_text() {
        let root = temp_root("rejects-text");
        let result = clone_asset_source("mcp", "make a local MCP server".to_string(), root).await;
        assert_eq!(result.unwrap_err(), "expected a git URL");
    }
}
