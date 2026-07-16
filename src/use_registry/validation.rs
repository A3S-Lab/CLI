//! Validation and managed Skill loading for the A3S Use registry contract.

use super::{RegistrySnapshot, SCHEMA_VERSION};
use a3s_code_core::skills::Skill;
use anyhow::{bail, Context};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;

pub(super) fn validate_snapshot(snapshot: &RegistrySnapshot) -> anyhow::Result<()> {
    if snapshot.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported A3S Use registry schema version {}",
            snapshot.schema_version
        );
    }
    if snapshot.revision.len() != 64
        || !snapshot
            .revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("A3S Use capability registry has an invalid revision");
    }
    let mut routes = std::collections::BTreeSet::new();
    let mut capabilities = std::collections::BTreeSet::new();
    for binding in &snapshot.capabilities {
        if binding.route.is_empty()
            || !binding.route.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '-' | '_')
            })
        {
            bail!("invalid A3S Use route '{}'", binding.route);
        }
        if !routes.insert(&binding.route) {
            bail!("duplicate A3S Use route '{}'", binding.route);
        }
        if !capabilities.insert(&binding.id) {
            bail!("duplicate A3S Use capability '{}'", binding.id);
        }
        if !binding.id.starts_with("use/") {
            bail!(
                "A3S Use capability '{}' has a non-component identity",
                binding.id
            );
        }
        if binding.mcp.is_some() && !binding.surfaces.iter().any(|surface| surface == "mcp") {
            bail!(
                "A3S Use capability '{}' projects MCP without declaring the surface",
                binding.id
            );
        }
        if !binding.skills.is_empty() && !binding.surfaces.iter().any(|surface| surface == "skill")
        {
            bail!(
                "A3S Use capability '{}' projects Skills without declaring the surface",
                binding.id
            );
        }
        if !binding.skills.is_empty()
            && (binding.package_root.as_os_str().is_empty() || !binding.package_root.is_absolute())
        {
            bail!(
                "A3S Use capability '{}' has Skills without an absolute package root",
                binding.id
            );
        }
        if binding.skills.iter().any(|skill| !skill.path.is_absolute()) {
            bail!(
                "A3S Use capability '{}' projects a non-absolute Skill path",
                binding.id
            );
        }
        if let Some(skill) = binding
            .skills
            .iter()
            .find(|skill| !skill.sha256.is_empty() && !is_lower_sha256(&skill.sha256))
        {
            bail!(
                "A3S Use capability '{}' projects an invalid Skill digest '{}'",
                binding.id,
                skill.sha256
            );
        }
        if let Some(mcp) = &binding.mcp {
            if mcp.target.is_empty() {
                bail!(
                    "A3S Use capability '{}' has an empty MCP target",
                    binding.id
                );
            }
        }
    }
    Ok(())
}

pub(super) async fn load_managed_skill(
    package_root: &Path,
    skill_path: &Path,
    expected_sha256: Option<&str>,
) -> anyhow::Result<Arc<Skill>> {
    if !package_root.is_absolute() || !skill_path.is_absolute() {
        bail!("A3S Use Skill paths and package roots must be absolute");
    }
    let root = tokio::fs::canonicalize(package_root)
        .await
        .with_context(|| format!("failed to resolve package root {}", package_root.display()))?;
    let metadata = tokio::fs::symlink_metadata(skill_path)
        .await
        .with_context(|| format!("failed to inspect A3S Use Skill {}", skill_path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "A3S Use Skill '{}' is not a regular package file",
            skill_path.display()
        );
    }
    let canonical = tokio::fs::canonicalize(skill_path)
        .await
        .with_context(|| format!("failed to resolve A3S Use Skill {}", skill_path.display()))?;
    if !canonical.starts_with(&root) {
        bail!(
            "A3S Use Skill '{}' escapes its managed package",
            skill_path.display()
        );
    }
    let bytes = tokio::fs::read(&canonical)
        .await
        .with_context(|| format!("failed to read A3S Use skill {}", canonical.display()))?;
    let actual_sha256 = format!("{:x}", Sha256::digest(&bytes));
    if let Some(expected_sha256) = expected_sha256 {
        if actual_sha256 != expected_sha256 {
            bail!(
                "A3S Use Skill '{}' digest does not match the capability registry",
                canonical.display()
            );
        }
    }

    let shown = canonical.clone();
    tokio::task::spawn_blocking(move || parse_skill_bytes(&canonical, bytes))
        .await
        .context("A3S Use skill loader task failed")?
        .with_context(|| format!("failed to load A3S Use skill {}", shown.display()))
        .map(Arc::new)
}

fn parse_skill_bytes(path: &Path, bytes: Vec<u8>) -> anyhow::Result<Skill> {
    let content = String::from_utf8(bytes).context("A3S Use Skill must be UTF-8")?;
    let mut skill = Skill::parse(&content).context("failed to parse skill file")?;
    if skill.name.is_empty() {
        if let Some(stem) = path.file_stem() {
            skill.name = stem.to_string_lossy().to_string();
        }
    }
    Ok(skill)
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn concise_stderr_suffix(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        let concise = stderr.chars().take(500).collect::<String>();
        format!(": {concise}")
    }
}

pub(super) fn validate_envelope_schema(value: &serde_json::Value) -> anyhow::Result<()> {
    let schema_version = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64);
    if schema_version != Some(u64::from(SCHEMA_VERSION)) {
        bail!(
            "A3S Use returned unsupported JSON schema version {}",
            schema_version
                .map(|version| version.to_string())
                .unwrap_or_else(|| "missing".to_string())
        );
    }
    Ok(())
}
