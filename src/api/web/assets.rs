use std::path::{Path, PathBuf};

pub(in crate::api) fn find_default_web_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        candidates.extend(packaged_candidates(&executable));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.extend(upward_candidates(&cwd));
        candidates.push(cwd.join("dist/workspace"));
        candidates.push(cwd.join("dist"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist/workspace"));
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist"));

    find_existing_web_dir(candidates)
}

fn packaged_candidates(executable: &Path) -> Vec<PathBuf> {
    let mut candidates = packaged_layout_candidates(executable);
    if let Ok(canonical) = executable.canonicalize() {
        if canonical != executable {
            candidates.extend(packaged_layout_candidates(&canonical));
        }
    }
    candidates
}

fn packaged_layout_candidates(executable: &Path) -> Vec<PathBuf> {
    let Some(bin_dir) = executable.parent() else {
        return Vec::new();
    };

    let mut candidates = vec![bin_dir.join("web")];
    if let Some(prefix) = bin_dir.parent() {
        candidates.push(prefix.join("share/a3s/web"));
    }
    candidates
}

fn find_existing_web_dir(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .map(clean_path)
        .find(|candidate| candidate.join("index.html").is_file())
}

fn upward_candidates(start: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut current = Some(start);
    while let Some(dir) = current {
        candidates.push(dir.join("apps/web/dist/workspace"));
        candidates.push(dir.join("apps/web/dist"));
        current = dir.parent();
    }
    candidates
}

fn clean_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_layouts_cover_archives_and_install_prefixes() {
        let executable = Path::new("prefix").join("bin").join("a3s");

        assert_eq!(
            packaged_layout_candidates(&executable),
            vec![
                Path::new("prefix").join("bin/web"),
                Path::new("prefix").join("share/a3s/web"),
            ]
        );
    }

    #[test]
    fn packaged_layouts_are_checked_before_development_fallbacks() {
        let executable = Path::new("prefix").join("bin").join("a3s");
        let mut candidates = packaged_layout_candidates(&executable);
        candidates.extend(upward_candidates(Path::new("workspace/project")));

        assert_eq!(candidates[0], Path::new("prefix").join("bin/web"));
        assert_eq!(candidates[1], Path::new("prefix").join("share/a3s/web"));
        assert!(candidates[2].ends_with("apps/web/dist/workspace"));
    }

    #[test]
    fn existing_packaged_assets_win_over_development_fallbacks() {
        let root = tempfile::tempdir().expect("temporary asset layouts");
        let packaged = root.path().join("bin/web");
        let development = root.path().join("apps/web/dist/workspace");
        std::fs::create_dir_all(&packaged).expect("packaged Web directory");
        std::fs::create_dir_all(&development).expect("development Web directory");
        std::fs::write(packaged.join("index.html"), "packaged").expect("packaged index");
        std::fs::write(development.join("index.html"), "development").expect("development index");

        let found =
            find_existing_web_dir([packaged.clone(), development]).expect("existing Web directory");

        assert_eq!(found, packaged.canonicalize().expect("canonical package"));
    }

    #[cfg(unix)]
    #[test]
    fn executable_symlink_can_find_assets_in_its_cellar_prefix() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary Homebrew layout");
        let linked_bin = root.path().join("bin");
        let cellar_bin = root.path().join("Cellar/a3s/0.9.2/bin");
        let cellar_web = root.path().join("Cellar/a3s/0.9.2/share/a3s/web");
        std::fs::create_dir_all(&linked_bin).expect("linked bin directory");
        std::fs::create_dir_all(&cellar_bin).expect("Cellar bin directory");
        std::fs::create_dir_all(&cellar_web).expect("Cellar Web directory");
        std::fs::write(cellar_bin.join("a3s"), "binary").expect("Cellar binary");
        std::fs::write(cellar_web.join("index.html"), "web").expect("Cellar Web index");
        let executable = linked_bin.join("a3s");
        symlink(cellar_bin.join("a3s"), &executable).expect("binary symlink");

        let found = find_existing_web_dir(packaged_candidates(&executable))
            .expect("Web assets beside the canonical executable prefix");

        assert_eq!(
            found,
            cellar_web.canonicalize().expect("canonical Web path")
        );
    }
}
