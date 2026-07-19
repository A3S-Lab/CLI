use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context};
use sha2::{Digest, Sha256};

const MAX_MANAGED_TREE_ENTRIES: usize = 5_000;
const MAX_MANAGED_TREE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PACKAGED_TREE_ENTRIES: usize = 2_000;
const MAX_PACKAGED_TREE_BYTES: u64 = 32 * 1024 * 1024;

pub(super) fn hash_install_tree(root: &Path) -> anyhow::Result<String> {
    hash_install_tree_with_policy(root, true, MAX_MANAGED_TREE_ENTRIES, MAX_MANAGED_TREE_BYTES)
}

pub(super) fn hash_packaged_install_tree(root: &Path) -> anyhow::Result<String> {
    hash_install_tree_with_policy(
        root,
        false,
        MAX_PACKAGED_TREE_ENTRIES,
        MAX_PACKAGED_TREE_BYTES,
    )
}

fn hash_install_tree_with_policy(
    root: &Path,
    allow_symlinks: bool,
    max_entries: usize,
    max_bytes: u64,
) -> anyhow::Result<String> {
    if !root.is_dir() {
        bail!(
            "managed SRT installation root is not a directory: {}",
            root.display()
        );
    }
    let mut hasher = Sha256::new();
    let mut budget = TreeBudget::default();
    hash_directory(
        root,
        root,
        &mut hasher,
        allow_symlinks,
        max_entries,
        max_bytes,
        &mut budget,
    )?;
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Default)]
struct TreeBudget {
    entries: usize,
    bytes: u64,
}

#[allow(clippy::too_many_arguments)]
fn hash_directory(
    root: &Path,
    directory: &Path,
    hasher: &mut Sha256,
    allow_symlinks: bool,
    max_entries: usize,
    max_bytes: u64,
    budget: &mut TreeBudget,
) -> anyhow::Result<()> {
    let mut entries = std::fs::read_dir(directory)
        .with_context(|| format!("failed to read {}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        budget.entries = budget
            .entries
            .checked_add(1)
            .context("managed SRT tree entry count overflowed")?;
        if budget.entries > max_entries {
            bail!("managed SRT tree exceeds the {max_entries} entry limit");
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .context("managed SRT tree entry escaped its root")?;
        let relative = normalized_relative_path(relative)?;
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            if !allow_symlinks {
                bail!(
                    "packaged managed SRT tree contains a symbolic link: {}",
                    path.display()
                );
            }
            hasher.update(b"link\0");
            hash_field(hasher, relative.as_bytes());
            let target = std::fs::read_link(&path)
                .with_context(|| format!("failed to read link {}", path.display()))?;
            hash_field(hasher, target.to_string_lossy().as_bytes());
        } else if metadata.is_dir() {
            hasher.update(b"dir\0");
            hash_field(hasher, relative.as_bytes());
            hash_directory(
                root,
                &path,
                hasher,
                allow_symlinks,
                max_entries,
                max_bytes,
                budget,
            )?;
        } else if metadata.is_file() {
            #[cfg(unix)]
            if !allow_symlinks {
                use std::os::unix::fs::MetadataExt;
                if metadata.nlink() > 1 {
                    bail!(
                        "packaged managed SRT tree contains a hard-linked file: {}",
                        path.display()
                    );
                }
            }
            hasher.update(b"file\0");
            hash_field(hasher, relative.as_bytes());
            let mut file = std::fs::File::open(&path)
                .with_context(|| format!("failed to open {}", path.display()))?;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = file
                    .read(&mut buffer)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                if read == 0 {
                    break;
                }
                budget.bytes = budget
                    .bytes
                    .checked_add(read as u64)
                    .context("managed SRT tree byte count overflowed")?;
                if budget.bytes > max_bytes {
                    bail!("managed SRT tree exceeds the {max_bytes} byte limit");
                }
                hasher.update(&buffer[..read]);
            }
            hasher.update(b"\0");
        } else {
            bail!(
                "unsupported file type in managed SRT tree: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn normalized_relative_path(path: &Path) -> anyhow::Result<String> {
    let mut normalized = String::new();
    for component in path.components() {
        let std::path::Component::Normal(segment) = component else {
            bail!(
                "managed SRT tree contains an invalid relative path: {}",
                path.display()
            );
        };
        let segment = segment
            .to_str()
            .with_context(|| format!("managed SRT path is not UTF-8: {}", path.display()))?;
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(segment);
    }
    if normalized.is_empty() {
        bail!("managed SRT tree contains an empty relative path");
    }
    Ok(normalized)
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}
