use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use a3s_updater::{ComponentReceipt, InstallProvenance};
use anyhow::{bail, Context};

use super::catalog::{self, ComponentKind, ComponentSpec, Distribution, ReleaseSpec};
use super::id::ComponentId;
use super::paths::ComponentPaths;
use super::probe::{is_executable, probe_version, run_bounded};
use super::state::{
    ComponentReport, ComponentState, ExternalTool, Health, Presence, Trust, UpdateState,
};

pub fn discover(paths: &ComponentPaths) -> anyhow::Result<ComponentReport> {
    let store = paths.receipt_store();
    let receipts = store.list()?;
    let by_id = receipts
        .iter()
        .map(|receipt| (receipt.component_id.as_str(), receipt))
        .collect::<BTreeMap<_, _>>();

    let mut components = Vec::new();
    for spec in catalog::all() {
        components.push(discover_registered(
            spec,
            by_id.get(spec.id).copied(),
            paths,
        )?);
    }

    if let Some(use_binary) = components
        .iter()
        .find(|component| component.id.as_str() == "use" && component.is_ready())
        .and_then(|component| component.path.as_deref())
    {
        if let Ok(children) = discover_dynamic_use_extensions(use_binary) {
            components.extend(children);
        }
    }

    for receipt in receipts {
        let receipt_id = ComponentId::parse(&receipt.component_id)?;
        if catalog::find(&receipt_id).is_some()
            || components
                .iter()
                .any(|component| component.id == receipt_id)
        {
            continue;
        }
        if receipt.component_id.split('/').count() >= 3 && receipt.component_id.starts_with("use/")
        {
            components.push(discover_receipt_extension(&receipt, receipt_id));
        }
    }
    components.sort_by(|left, right| left.id.cmp(&right.id));

    let registered_binaries = catalog::all()
        .iter()
        .filter_map(catalog::release)
        .map(|release| release.binary)
        .collect::<BTreeSet<_>>();
    let external_tools = discover_external_tools(paths.path_env.clone(), &registered_binaries);

    Ok(ComponentReport {
        schema_version: 1,
        components,
        external_tools,
    })
}

fn discover_dynamic_use_extensions(parent_binary: &Path) -> anyhow::Result<Vec<ComponentState>> {
    let output = run_bounded(
        parent_binary.as_os_str(),
        &[
            OsString::from("component"),
            OsString::from("list"),
            OsString::from("--json"),
        ],
    )?;
    if !output.success {
        bail!("Use component list exited unsuccessfully");
    }
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("invalid Use component list JSON")?;
    let children = value
        .get("components")
        .or_else(|| value.get("data").and_then(|data| data.get("components")))
        .and_then(serde_json::Value::as_array)
        .context("Use component list has no components array")?;
    let mut extensions = Vec::new();
    let mut seen = BTreeSet::new();
    for component in children {
        let Some(returned_id) = component.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let full_id = if returned_id.starts_with("use/") {
            returned_id.to_string()
        } else {
            format!("use/{returned_id}")
        };
        if full_id.split('/').count() != 3 {
            continue;
        }
        let Ok(id) = ComponentId::parse(&full_id) else {
            continue;
        };
        if catalog::find(&id).is_some() || !seen.insert(id.clone()) {
            continue;
        }
        extensions.push(ComponentState {
            id,
            kind: ComponentKind::Extension,
            description: component
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Externally implemented A3S Use domain")
                .to_string(),
            presence: parse_presence(component.get("presence")),
            health: parse_health(component.get("health")),
            update: UpdateState::Unknown,
            trust: match component.get("trust").and_then(serde_json::Value::as_str) {
                Some("local-explicit") => Trust::LocalExplicit,
                _ => Trust::Untrusted,
            },
            provenance: Some(InstallProvenance::Delegated),
            version: component
                .get("version")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            path: component
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from),
            message: None,
        });
    }
    Ok(extensions)
}

pub fn find_state(id: &ComponentId, paths: &ComponentPaths) -> anyhow::Result<ComponentState> {
    if let Some(spec) = catalog::find(id) {
        let receipt = paths.receipt_store().read(id.as_str())?;
        return discover_registered(spec, receipt.as_ref(), paths);
    }
    let report = discover(paths)?;
    report
        .components
        .into_iter()
        .find(|component| &component.id == id)
        .with_context(|| format!("component '{}' is not registered", id))
}

fn discover_registered(
    spec: &ComponentSpec,
    receipt: Option<&ComponentReceipt>,
    paths: &ComponentPaths,
) -> anyhow::Result<ComponentState> {
    let id = ComponentId::parse(spec.id)?;
    Ok(match spec.distribution {
        Distribution::Bundled => ComponentState {
            id,
            kind: spec.kind,
            description: spec.description.to_string(),
            presence: Presence::Bundled,
            health: Health::Ready,
            update: UpdateState::Current,
            trust: Trust::FirstParty,
            provenance: Some(InstallProvenance::Bundled),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            path: Some(paths.current_exe.clone()),
            message: None,
        },
        Distribution::Release(release) => discover_product(spec, id, release, receipt, paths),
        Distribution::Delegated { parent } => discover_delegated(spec, id, parent, paths),
    })
}

fn discover_product(
    spec: &ComponentSpec,
    id: ComponentId,
    release: ReleaseSpec,
    receipt: Option<&ComponentReceipt>,
    paths: &ComponentPaths,
) -> ComponentState {
    if let Some(receipt) = receipt {
        let executable = receipt.executable_path.clone();
        let probe = executable
            .as_deref()
            .and_then(|path| probe_version(path).ok());
        let probe_succeeded = probe.is_some();
        return ComponentState {
            id,
            kind: spec.kind,
            description: spec.description.to_string(),
            presence: Presence::Managed,
            health: if probe_succeeded {
                Health::Ready
            } else {
                Health::Broken
            },
            update: UpdateState::Unknown,
            trust: Trust::FirstParty,
            provenance: Some(receipt.provenance),
            version: probe.or_else(|| Some(receipt.version.clone())),
            path: executable,
            message: (!probe_succeeded).then(|| {
                "The installed receipt exists, but its executable failed the version probe."
                    .to_string()
            }),
        };
    }

    let candidates = [
        paths.configured_binary(release),
        paths.sibling_binary(release.binary),
        find_on_path(release.binary, paths.path_env.clone()),
        paths.fallback_binary(release.binary),
    ];
    for candidate in candidates.into_iter().flatten() {
        if !is_executable(&candidate) {
            continue;
        }
        let version = probe_version(&candidate).ok();
        let system = is_system_path(&candidate);
        return ComponentState {
            id,
            kind: spec.kind,
            description: spec.description.to_string(),
            presence: if system {
                Presence::System
            } else {
                Presence::External
            },
            health: if version.is_some() {
                Health::Ready
            } else {
                Health::Broken
            },
            update: UpdateState::Unknown,
            trust: Trust::FirstParty,
            provenance: Some(if system {
                InstallProvenance::System
            } else {
                InstallProvenance::ExternalPath
            }),
            version,
            path: Some(candidate),
            message: None,
        };
    }

    ComponentState {
        id,
        kind: spec.kind,
        description: spec.description.to_string(),
        presence: Presence::Missing,
        health: Health::Unknown,
        update: UpdateState::Unknown,
        trust: Trust::FirstParty,
        provenance: None,
        version: None,
        path: None,
        message: None,
    }
}

fn discover_delegated(
    spec: &ComponentSpec,
    id: ComponentId,
    parent: &str,
    paths: &ComponentPaths,
) -> ComponentState {
    let Ok(parent_id) = ComponentId::parse(parent) else {
        return ComponentState {
            id,
            kind: spec.kind,
            description: spec.description.to_string(),
            presence: Presence::Missing,
            health: Health::Broken,
            update: UpdateState::Unknown,
            trust: Trust::FirstParty,
            provenance: Some(InstallProvenance::Delegated),
            version: None,
            path: None,
            message: Some("The built-in delegated parent ID is invalid.".to_string()),
        };
    };
    let parent_receipt = paths.receipt_store().read(parent).ok().flatten();
    let parent_state = catalog::find(&parent_id).and_then(|parent_spec| {
        discover_registered(parent_spec, parent_receipt.as_ref(), paths).ok()
    });
    let Some(parent_binary) = parent_state
        .filter(ComponentState::is_ready)
        .and_then(|state| state.path)
    else {
        return ComponentState {
            id,
            kind: spec.kind,
            description: spec.description.to_string(),
            presence: Presence::Missing,
            health: Health::Unknown,
            update: UpdateState::Unknown,
            trust: Trust::FirstParty,
            provenance: Some(InstallProvenance::Delegated),
            version: None,
            path: None,
            message: Some(format!("Parent component '{parent}' is not ready.")),
        };
    };

    let relative = id.relative_to(&parent_id).unwrap_or(id.as_str());
    match delegated_status(&parent_binary, relative, &id) {
        Ok(state) => state,
        Err(error) => ComponentState {
            id,
            kind: spec.kind,
            description: spec.description.to_string(),
            presence: Presence::Missing,
            health: Health::Unknown,
            update: UpdateState::Unknown,
            trust: Trust::FirstParty,
            provenance: Some(InstallProvenance::Delegated),
            version: None,
            path: None,
            message: Some(format!("Delegated status is unavailable: {error}")),
        },
    }
}

fn delegated_status(
    parent_binary: &Path,
    relative: &str,
    id: &ComponentId,
) -> anyhow::Result<ComponentState> {
    let output = run_bounded(
        parent_binary.as_os_str(),
        &[
            OsString::from("component"),
            OsString::from("status"),
            OsString::from(relative),
            OsString::from("--json"),
        ],
    )?;
    if !output.success {
        bail!("parent command exited unsuccessfully");
    }
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("invalid delegated JSON output")?;
    let component = value
        .get("component")
        .or_else(|| value.get("data").and_then(|data| data.get("component")))
        .context("delegated output has no component object")?;
    let returned_id = component
        .get("id")
        .and_then(serde_json::Value::as_str)
        .context("delegated component has no ID")?;
    if returned_id != id.as_str() && returned_id != relative {
        bail!("delegated component ID mismatch: {returned_id}");
    }

    Ok(ComponentState {
        id: id.clone(),
        kind: ComponentKind::Capability,
        description: component
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Delegated runtime capability")
            .to_string(),
        presence: parse_presence(component.get("presence")),
        health: parse_health(component.get("health")),
        update: UpdateState::Unknown,
        trust: Trust::FirstParty,
        provenance: Some(InstallProvenance::Delegated),
        version: component
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        path: component
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from),
        message: None,
    })
}

fn discover_receipt_extension(receipt: &ComponentReceipt, id: ComponentId) -> ComponentState {
    let executable = receipt.executable_path.clone();
    let version = executable
        .as_deref()
        .and_then(|path| probe_version(path).ok())
        .or_else(|| Some(receipt.version.clone()));
    ComponentState {
        id,
        kind: ComponentKind::Extension,
        description: "Externally implemented A3S Use domain".to_string(),
        presence: Presence::Managed,
        health: if executable.as_deref().is_some_and(is_executable) {
            Health::Ready
        } else {
            Health::Broken
        },
        update: UpdateState::Unknown,
        trust: if receipt.provenance == InstallProvenance::LocalPackage {
            Trust::LocalExplicit
        } else {
            Trust::Untrusted
        },
        provenance: Some(receipt.provenance),
        version,
        path: executable,
        message: None,
    }
}

fn discover_external_tools(
    path_env: Option<OsString>,
    registered_binaries: &BTreeSet<&str>,
) -> Vec<ExternalTool> {
    let Some(path_env) = path_env else {
        return Vec::new();
    };
    let mut by_command = BTreeMap::new();
    for directory in std::env::split_paths(&path_env) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_executable(&path) {
                continue;
            }
            let Some(binary) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if registered_binaries.contains(binary) {
                continue;
            }
            let Some(command) = binary.strip_prefix("a3s-") else {
                continue;
            };
            if command.is_empty() {
                continue;
            }
            let binary = binary.to_string();
            let command = command.to_string();
            by_command
                .entry(command.clone())
                .or_insert_with(|| ExternalTool {
                    command,
                    binary,
                    path,
                });
        }
    }
    by_command.into_values().collect()
}

fn find_on_path(binary: &str, path_env: Option<OsString>) -> Option<PathBuf> {
    let paths = path_env?;
    std::env::split_paths(&paths)
        .map(|directory| directory.join(binary))
        .find(|path| is_executable(path))
}

fn is_system_path(path: &Path) -> bool {
    [Path::new("/usr/bin"), Path::new("/bin")]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

fn parse_presence(value: Option<&serde_json::Value>) -> Presence {
    match value.and_then(serde_json::Value::as_str) {
        Some("bundled") => Presence::Bundled,
        Some("managed") => Presence::Managed,
        Some("external") => Presence::External,
        Some("system") => Presence::System,
        _ => Presence::Missing,
    }
}

fn parse_health(value: Option<&serde_json::Value>) -> Health {
    match value.and_then(serde_json::Value::as_str) {
        Some("ready") => Health::Ready,
        Some("broken") => Health::Broken,
        _ => Health::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn registered_products_and_external_tools_are_separate() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        write_executable(&bin.join("a3s-use"), "#!/bin/sh\necho 'a3s-use 0.1.0'\n");
        let marker = temp.path().join("marker");
        write_executable(
            &bin.join("a3s-unknown"),
            &format!("#!/bin/sh\necho ran > '{}'\n", marker.display()),
        );
        let mut paths = ComponentPaths::for_test(temp.path());
        paths.path_env = Some(std::env::join_paths([&bin]).unwrap());
        std::fs::create_dir_all(paths.current_exe.parent().unwrap()).unwrap();
        std::fs::write(&paths.current_exe, "").unwrap();
        std::fs::set_permissions(&paths.current_exe, std::fs::Permissions::from_mode(0o755))
            .unwrap();

        let report = discover(&paths).unwrap();
        let use_state = report
            .components
            .iter()
            .find(|component| component.id.as_str() == "use")
            .unwrap();
        assert_eq!(use_state.presence, Presence::External);
        assert_eq!(use_state.health, Health::Ready);
        assert_eq!(use_state.version.as_deref(), Some("0.1.0"));
        assert_eq!(report.external_tools.len(), 1);
        assert_eq!(report.external_tools[0].command, "unknown");
        assert!(!marker.exists(), "unregistered tools must not be executed");
    }

    #[test]
    #[cfg(unix)]
    fn trusted_use_parent_reports_dynamic_extension_children() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        write_executable(
            &bin.join("a3s-use"),
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf 'a3s-use 0.1.0\n'
elif [ "$1" = "component" ] && [ "$2" = "list" ]; then
  printf '%s\n' '{"schemaVersion":1,"ok":true,"data":{"components":[{"id":"browser"},{"id":"acme/slack","description":"Slack domain","presence":"managed","health":"ready","trust":"local-explicit","version":"1.2.0","path":"/tmp/slack"}]}}'
else
  exit 1
fi
"#,
        );
        let mut paths = ComponentPaths::for_test(temp.path());
        paths.path_env = Some(std::env::join_paths([&bin]).unwrap());
        std::fs::create_dir_all(paths.current_exe.parent().unwrap()).unwrap();
        write_executable(&paths.current_exe, "#!/bin/sh\nexit 0\n");

        let report = discover(&paths).unwrap();
        let extension = report
            .components
            .iter()
            .find(|component| component.id.as_str() == "use/acme/slack")
            .unwrap();
        assert_eq!(extension.kind, ComponentKind::Extension);
        assert_eq!(extension.presence, Presence::Managed);
        assert_eq!(extension.health, Health::Ready);
        assert_eq!(extension.trust, Trust::LocalExplicit);
        assert_eq!(extension.version.as_deref(), Some("1.2.0"));
    }

    #[test]
    #[cfg(unix)]
    fn a_receipt_has_priority_and_a_missing_owned_binary_is_broken() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ComponentPaths::for_test(temp.path());
        let id = ComponentId::parse("use").unwrap();
        let install_root = paths.version_root(&id, "0.1.0");
        let receipt = ComponentReceipt {
            schema_version: a3s_updater::RECEIPT_SCHEMA_VERSION,
            component_id: "use".to_string(),
            version: "0.1.0".to_string(),
            provenance: InstallProvenance::GithubRelease,
            install_root: install_root.clone(),
            executable_path: Some(install_root.join("a3s-use")),
            owned_paths: vec![install_root],
            source: None,
            artifact_checksums: BTreeMap::new(),
            installed_at: "2026-07-14T00:00:00Z".to_string(),
        };
        paths.receipt_store().write(&receipt).unwrap();

        let state = find_state(&id, &paths).unwrap();
        assert_eq!(state.presence, Presence::Managed);
        assert_eq!(state.health, Health::Broken);
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
