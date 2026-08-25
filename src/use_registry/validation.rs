//! Validation and managed Skill loading for the A3S Use registry contract.

use super::{
    CapabilityOrigin, ProjectedManagedAsset, ProjectedMcpLaunch, ProjectedMcpServer,
    RegistrySnapshot, JSON_ENVELOPE_SCHEMA_VERSION, SCHEMA_VERSION,
};
use a3s_code_core::capability::{
    Sha256Digest, UiAsset, UiAssetKind, MAX_UI_ASSETS_PER_KIND, MAX_UI_ASSET_BYTES,
};
use a3s_code_core::skills::Skill;
use a3s_use_core::{metadata_is_link_or_reparse_point, PluginSurfaceKind};
use anyhow::{bail, Context};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncReadExt;

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
    let mut mcp_server_names = std::collections::BTreeMap::new();
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
        if (binding.mcp.is_some() || !binding.mcp_servers.is_empty())
            && !binding.surfaces.iter().any(|surface| surface == "mcp")
        {
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
        if !binding.activity_bar.is_empty()
            && !binding.surfaces.iter().any(|surface| surface == "ui")
        {
            bail!(
                "A3S Use capability '{}' projects UI without declaring the surface",
                binding.id
            );
        }
        if !binding.knowledge.is_empty() && !binding.surfaces.iter().any(|surface| surface == "okf")
        {
            bail!(
                "A3S Use capability '{}' projects OKF Knowledge without declaring the surface",
                binding.id
            );
        }
        for projection in &binding.knowledge {
            projection.validate().map_err(|error| {
                anyhow::anyhow!(
                    "A3S Use capability '{}' has invalid OKF Knowledge evidence: {}: {}",
                    binding.id,
                    error.code,
                    error.message
                )
            })?;
        }
        super::runtime_tasks::validate_projected_runtime_tasks(binding)?;
        validate_projected_mcp(binding)?;
        for projection in &binding.mcp_servers {
            if let Some(owner) = mcp_server_names.insert(&projection.server_name, &binding.id) {
                bail!(
                    "A3S Use capabilities '{}' and '{}' project the same MCP server name '{}'",
                    owner,
                    binding.id,
                    projection.server_name
                );
            }
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
        let mut skill_ids = std::collections::BTreeSet::new();
        for skill in &binding.skills {
            if !skill.id.is_empty()
                && (!valid_surface_id(&skill.id) || !skill_ids.insert(skill.id.as_str()))
            {
                bail!(
                    "A3S Use capability '{}' projects an invalid or duplicate Skill surface ID '{}'",
                    binding.id,
                    skill.id
                );
            }
        }
        validate_projected_ui(binding)?;
        if let Some(mcp) = &binding.mcp {
            if mcp.target.is_empty() {
                bail!(
                    "A3S Use capability '{}' has an empty MCP target",
                    binding.id
                );
            }
        }
        super::flow::validate_projected_flows(binding)?;
    }
    Ok(())
}

fn validate_projected_mcp(binding: &super::CapabilityBinding) -> anyhow::Result<()> {
    if binding.mcp_servers.is_empty() {
        return Ok(());
    }
    if binding.origin != CapabilityOrigin::Extension
        || !binding.enabled
        || binding.mcp.is_some()
        || binding.package_root.as_os_str().is_empty()
        || !binding.package_root.is_absolute()
    {
        bail!(
            "A3S Use capability '{}' has managed MCP evidence outside an enabled extension package",
            binding.id
        );
    }
    if binding.mcp_servers.len() > 64 {
        bail!(
            "A3S Use capability '{}' exceeds the managed MCP surface bound",
            binding.id
        );
    }
    let planner = binding.planner_evidence.as_ref().with_context(|| {
        format!(
            "A3S Use capability '{}' projects managed MCP without exact package evidence",
            binding.id
        )
    })?;
    let lifecycle_generation = binding.lifecycle_generation.with_context(|| {
        format!(
            "A3S Use capability '{}' projects managed MCP without a lifecycle generation",
            binding.id
        )
    })?;
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for projection in &binding.mcp_servers {
        validate_projected_mcp_server(&binding.id, lifecycle_generation, planner, projection)?;
        if !ids.insert(projection.id.as_str()) {
            bail!(
                "A3S Use capability '{}' projects duplicate MCP surface ID '{}'",
                binding.id,
                projection.id
            );
        }
        if !names.insert(projection.server_name.as_str()) {
            bail!(
                "A3S Use capability '{}' projects duplicate MCP server name '{}'",
                binding.id,
                projection.server_name
            );
        }
    }
    Ok(())
}

fn validate_projected_mcp_server(
    capability_id: &str,
    lifecycle_generation: u64,
    planner: &super::ProjectedPluginPlannerEvidence,
    projection: &ProjectedMcpServer,
) -> anyhow::Result<()> {
    if !valid_surface_id(&projection.id)
        || !projection.server_name.starts_with("use_mcp_")
        || projection.server_name.len() > 128
        || !projection
            .server_name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        || !is_prefixed_lower_sha256(&projection.file_evidence_digest)
    {
        bail!("A3S Use capability '{capability_id}' projects invalid MCP identity evidence");
    }
    let identity = projection.lifecycle_identity.validated("MCP")?;
    if identity.package_id() != planner.package_id
        || identity.package_digest() != canonical_digest(&planner.package_sha256)
        || identity.manifest_digest() != canonical_digest(&planner.manifest_sha256)
        || identity.generation() != lifecycle_generation
    {
        bail!(
            "A3S Use capability '{capability_id}' projects MCP for a different package generation"
        );
    }
    match &projection.launch {
        ProjectedMcpLaunch::Stdio { executable, args } => {
            validate_relative_projection_path(capability_id, "MCP executable", executable)?;
            if args.len() > 256
                || args
                    .iter()
                    .any(|argument| argument.len() > 8 * 1024 || argument.contains('\0'))
                || args.iter().map(String::len).sum::<usize>() > 64 * 1024
            {
                bail!(
                    "A3S Use capability '{capability_id}' projects MCP stdio arguments outside the host bound"
                );
            }
        }
        ProjectedMcpLaunch::StreamableHttp { release, runtime } => {
            validate_relative_projection_path(capability_id, "MCP release", release)?;
            a3s_use::plugin_runtime::RuntimeEndpointRef::parse(runtime.endpoint_ref.clone())
                .map_err(|error| {
                    anyhow::anyhow!(
                        "A3S Use capability '{capability_id}' projects invalid MCP endpoint evidence: {}: {}",
                        error.code,
                        error.message
                    )
                })?;
            if !valid_http_path(&runtime.endpoint_path)
                || runtime.protocol_version.is_empty()
                || runtime.protocol_version.len() > 64
                || runtime.protocol_version.chars().any(char::is_control)
                || runtime.initialized_at_ms == 0
                || a3s_runtime::ProviderId::parse(&runtime.provider_id).is_err()
                || runtime.provider_build_id.is_empty()
                || runtime.provider_build_id.len() > 256
                || runtime.provider_build_id.chars().any(char::is_control)
                || runtime.runtime_generation != lifecycle_generation
                || !is_prefixed_lower_sha256(&runtime.descriptor_digest)
                || !is_prefixed_lower_sha256(&runtime.binding_digest)
            {
                bail!(
                    "A3S Use capability '{capability_id}' projects invalid MCP Runtime readiness evidence"
                );
            }
        }
    }
    Ok(())
}

fn validate_relative_projection_path(
    capability_id: &str,
    label: &str,
    path: &Path,
) -> anyhow::Result<()> {
    use std::path::Component;

    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("A3S Use capability '{capability_id}' projects an invalid relative {label} path");
    }
    Ok(())
}

fn valid_http_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 2048
        && !value.contains("//")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'?' | b'#' | b'\\'))
        && !value
            .split('/')
            .any(|segment| matches!(segment, "." | ".."))
}

fn canonical_digest(value: &str) -> String {
    if value.starts_with("sha256:") {
        value.to_owned()
    } else {
        format!("sha256:{value}")
    }
}

fn is_prefixed_lower_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(is_lower_sha256)
}

fn validate_projected_ui(binding: &super::CapabilityBinding) -> anyhow::Result<()> {
    if binding.activity_bar.is_empty() {
        return Ok(());
    }
    if !binding.enabled {
        bail!(
            "A3S Use capability '{}' projects UI while disabled",
            binding.id
        );
    }
    if binding.package_root.as_os_str().is_empty() || !binding.package_root.is_absolute() {
        bail!(
            "A3S Use capability '{}' has UI without an absolute package root",
            binding.id
        );
    }
    let mut ids = std::collections::BTreeSet::new();
    for contribution in &binding.activity_bar {
        if contribution.dependency_evidence_schema != super::UI_DEPENDENCY_EVIDENCE_SCHEMA {
            bail!(
                "A3S Use UI contribution '{}:{}' has missing or unsupported dependency evidence schema '{}'",
                binding.id,
                contribution.id,
                contribution.dependency_evidence_schema
            );
        }
        if !valid_surface_id(&contribution.id) || !ids.insert(contribution.id.as_str()) {
            bail!(
                "A3S Use capability '{}' projects an invalid or duplicate UI surface ID '{}'",
                binding.id,
                contribution.id
            );
        }
        if contribution.styles.len() > MAX_UI_ASSETS_PER_KIND
            || contribution.scripts.len() > MAX_UI_ASSETS_PER_KIND
        {
            bail!(
                "A3S Use UI contribution '{}:{}' exceeds the static asset-count bound",
                binding.id,
                contribution.id
            );
        }
        validate_projected_asset(binding, &contribution.entry, UiAssetKind::Html)?;
        for asset in &contribution.styles {
            validate_projected_asset(binding, asset, UiAssetKind::Style)?;
        }
        for asset in &contribution.scripts {
            validate_projected_asset(binding, asset, UiAssetKind::Script)?;
        }
        if let Some(skill) = contribution.skill.as_deref() {
            if !valid_surface_id(skill) {
                bail!(
                    "A3S Use UI contribution '{}:{}' has an invalid legacy Skill selector '{}'",
                    binding.id,
                    contribution.id,
                    skill
                );
            }
            if !contribution.dependencies.iter().any(|dependency| {
                dependency.kind == PluginSurfaceKind::Skill && dependency.id == skill
            }) {
                bail!(
                    "A3S Use UI contribution '{}:{}' selects Skill '{}' without canonical dependency evidence",
                    binding.id,
                    contribution.id,
                    skill
                );
            }
        }
        for dependency in &contribution.dependencies {
            if !valid_surface_id(&dependency.id)
                || !matches!(
                    dependency.kind,
                    PluginSurfaceKind::Tool
                        | PluginSurfaceKind::Skill
                        | PluginSurfaceKind::Mcp
                        | PluginSurfaceKind::Flow
                )
            {
                bail!(
                    "A3S Use UI contribution '{}:{}' has an invalid dependency '{}:{}'",
                    binding.id,
                    contribution.id,
                    format_args!("{:?}", dependency.kind),
                    dependency.id
                );
            }
        }
        if contribution
            .dependencies
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            bail!(
                "A3S Use UI contribution '{}:{}' dependencies are not canonical sorted unique evidence",
                binding.id,
                contribution.id
            );
        }
    }
    Ok(())
}

fn validate_projected_asset(
    binding: &super::CapabilityBinding,
    asset: &ProjectedManagedAsset,
    kind: UiAssetKind,
) -> anyhow::Result<()> {
    if !asset.path.is_absolute() {
        bail!(
            "A3S Use UI contribution in '{}' projects a non-absolute {} asset path",
            binding.id,
            kind
        );
    }
    if asset.media_type != kind.media_type() {
        bail!(
            "A3S Use UI contribution in '{}' projects {} with media type '{}'",
            binding.id,
            kind,
            asset.media_type
        );
    }
    if !is_lower_sha256(&asset.sha256) {
        bail!(
            "A3S Use UI contribution in '{}' projects an invalid {} asset digest",
            binding.id,
            kind
        );
    }
    Ok(())
}

fn valid_surface_id(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
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

pub(super) async fn load_managed_ui_asset(
    package_root: &Path,
    asset: &ProjectedManagedAsset,
    kind: UiAssetKind,
) -> anyhow::Result<UiAsset> {
    if !package_root.is_absolute() || !asset.path.is_absolute() {
        bail!("A3S Use UI asset paths and package roots must be absolute");
    }
    if asset.media_type != kind.media_type() {
        bail!(
            "A3S Use UI {} asset '{}' has media type '{}'",
            kind,
            asset.path.display(),
            asset.media_type
        );
    }
    let root = tokio::fs::canonicalize(package_root)
        .await
        .with_context(|| format!("failed to resolve package root {}", package_root.display()))?;
    let metadata = tokio::fs::symlink_metadata(&asset.path)
        .await
        .with_context(|| {
            format!(
                "failed to inspect A3S Use UI asset {}",
                asset.path.display()
            )
        })?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        bail!(
            "A3S Use UI asset '{}' is not a regular package file",
            asset.path.display()
        );
    }
    if metadata.len() > MAX_UI_ASSET_BYTES as u64 {
        bail!(
            "A3S Use UI {} asset '{}' exceeds the byte bound",
            kind,
            asset.path.display()
        );
    }
    let canonical = tokio::fs::canonicalize(&asset.path)
        .await
        .with_context(|| {
            format!(
                "failed to resolve A3S Use UI asset {}",
                asset.path.display()
            )
        })?;
    if !canonical.starts_with(&root) {
        bail!(
            "A3S Use UI asset '{}' escapes its managed package",
            asset.path.display()
        );
    }
    let file = tokio::fs::File::open(&canonical)
        .await
        .with_context(|| format!("failed to open A3S Use UI asset {}", canonical.display()))?;
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(MAX_UI_ASSET_BYTES));
    file.take((MAX_UI_ASSET_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .with_context(|| format!("failed to read A3S Use UI asset {}", canonical.display()))?;
    if bytes.len() > MAX_UI_ASSET_BYTES {
        bail!(
            "A3S Use UI {} asset '{}' changed beyond the byte bound while loading",
            kind,
            canonical.display()
        );
    }
    let content = String::from_utf8(bytes)
        .with_context(|| format!("A3S Use UI {} asset must be UTF-8", kind))?;
    let expected = Sha256Digest::new(format!("sha256:{}", asset.sha256))
        .context("A3S Use UI asset has an invalid reviewed digest")?;
    UiAsset::new_verified(kind, content, expected).with_context(|| {
        format!(
            "A3S Use UI {} asset '{}' does not match its reviewed evidence",
            kind,
            canonical.display()
        )
    })
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
    if schema_version != Some(u64::from(JSON_ENVELOPE_SCHEMA_VERSION)) {
        bail!(
            "A3S Use returned unsupported JSON schema version {}",
            schema_version
                .map(|version| version.to_string())
                .unwrap_or_else(|| "missing".to_string())
        );
    }
    Ok(())
}
