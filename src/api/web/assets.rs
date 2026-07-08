use std::path::{Path, PathBuf};

pub(in crate::api) fn find_default_web_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.extend(upward_candidates(&cwd));
        candidates.push(cwd.join("dist/workspace"));
        candidates.push(cwd.join("dist"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist/workspace"));
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist"));

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
