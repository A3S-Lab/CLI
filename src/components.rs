//! Component management shared by `a3s install`, `a3s update`, and `a3s list`.

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComponentId {
    Code,
    Box,
    Bench,
}

impl ComponentId {
    pub(crate) const ALL: [Self; 3] = [Self::Code, Self::Box, Self::Bench];

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "code" => Some(Self::Code),
            "box" => Some(Self::Box),
            "bench" => Some(Self::Bench),
            _ => None,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Box => "box",
            Self::Bench => "bench",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComponentStatus {
    pub(crate) id: ComponentId,
    pub(crate) installed: bool,
    pub(crate) version: Option<String>,
    pub(crate) source: String,
    pub(crate) path: Option<PathBuf>,
    pub(crate) health: String,
}

pub(crate) fn install(component: ComponentId) -> anyhow::Result<()> {
    match component {
        ComponentId::Code => install_code(),
        ComponentId::Box => {
            crate::box_cmd::install()?;
            Ok(())
        }
        ComponentId::Bench => {
            crate::bench_component::install()?;
            Ok(())
        }
    }
}

pub(crate) fn update(component: ComponentId) -> anyhow::Result<()> {
    match component {
        ComponentId::Code => crate::self_update(),
        ComponentId::Box => {
            crate::box_cmd::update()?;
            Ok(())
        }
        ComponentId::Bench => {
            crate::bench_component::update()?;
            Ok(())
        }
    }
}

fn install_code() -> anyhow::Result<()> {
    println!(
        "✓ a3s code {} is installed (included with a3s)",
        env!("CARGO_PKG_VERSION")
    );
    match crate::update::repair_installation() {
        Ok(repaired) if repaired.is_empty() => println!("✓ installation looks healthy"),
        Ok(repaired) => {
            for item in repaired {
                println!("✓ {item}");
            }
        }
        Err(error) => eprintln!("warning: optional Code companion repair failed: {error}"),
    }
    Ok(())
}

pub(crate) fn run_bench(args: Vec<String>) -> anyhow::Result<()> {
    let installed = crate::bench_component::ensure()?;
    run_bench_binary(&installed.path, args)
}

pub(crate) fn run_bench_installed(args: Vec<String>) -> anyhow::Result<bool> {
    let crate::bench_component::BenchState::Installed(installed) =
        crate::bench_component::inspect()
    else {
        return Ok(false);
    };
    run_bench_binary(&installed.path, args)?;
    Ok(true)
}

fn run_bench_binary(path: &std::path::Path, args: Vec<String>) -> anyhow::Result<()> {
    let status = Command::new(path).args(args).status().map_err(|error| {
        anyhow::anyhow!(
            "failed to run Bench control component at {}: {error}",
            path.display()
        )
    })?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

pub(crate) fn statuses() -> Vec<ComponentStatus> {
    ComponentId::ALL.into_iter().map(status).collect()
}

fn status(component: ComponentId) -> ComponentStatus {
    match component {
        ComponentId::Code => code_status(),
        ComponentId::Box => box_status(),
        ComponentId::Bench => bench_status(),
    }
}

fn code_status() -> ComponentStatus {
    let path = std::env::current_exe().ok();
    ComponentStatus {
        id: ComponentId::Code,
        installed: true,
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        source: "bundled with a3s".to_string(),
        path,
        health: "ready".to_string(),
    }
}

fn box_status() -> ComponentStatus {
    use crate::box_cmd::{BoxSource, BoxState};

    match crate::box_cmd::inspect_read_only() {
        BoxState::Installed(installed) => {
            let source = match installed.source {
                BoxSource::ConfiguredInstallDir => "configured install directory",
                BoxSource::Sibling => "a3s installation",
                BoxSource::Path => "PATH",
                BoxSource::LegacyUserBin => "user install directory",
            };
            let health = if installed.version.is_some() {
                "ready"
            } else {
                "ready (version unavailable)"
            };
            ComponentStatus {
                id: ComponentId::Box,
                installed: true,
                version: installed.version,
                source: source.to_string(),
                path: Some(installed.path),
                health: health.to_string(),
            }
        }
        BoxState::Missing => missing_status(ComponentId::Box, "on-demand release"),
        BoxState::Broken(error) => broken_status(ComponentId::Box, "local installation", error),
    }
}

fn bench_status() -> ComponentStatus {
    use crate::bench_component::BenchState;

    match crate::bench_component::inspect() {
        BenchState::Installed(installed) => ComponentStatus {
            id: ComponentId::Bench,
            installed: true,
            version: Some(installed.version),
            source: format!("managed release ({})", installed.target),
            path: Some(installed.path),
            health: "ready".to_string(),
        },
        BenchState::Missing => missing_status(ComponentId::Bench, "on-demand managed release"),
        BenchState::Broken(error) => broken_status(ComponentId::Bench, "managed release", error),
    }
}

fn missing_status(id: ComponentId, source: &str) -> ComponentStatus {
    ComponentStatus {
        id,
        installed: false,
        version: None,
        source: source.to_string(),
        path: None,
        health: "not installed".to_string(),
    }
}

fn broken_status(id: ComponentId, source: &str, error: String) -> ComponentStatus {
    ComponentStatus {
        id,
        installed: false,
        version: None,
        source: source.to_string(),
        path: None,
        health: format!("broken: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_ids_are_fixed_and_parse_exactly() {
        assert_eq!(
            ComponentId::ALL.map(ComponentId::as_str),
            ["code", "box", "bench"]
        );
        assert_eq!(ComponentId::parse("code"), Some(ComponentId::Code));
        assert_eq!(ComponentId::parse("Bench"), None);
        assert_eq!(ComponentId::parse("other"), None);
    }

    #[test]
    fn missing_component_status_has_no_fake_install_metadata() {
        let status = missing_status(ComponentId::Bench, "managed release");
        assert!(!status.installed);
        assert_eq!(status.version, None);
        assert_eq!(status.path, None);
        assert_eq!(status.health, "not installed");
    }

    #[test]
    fn broken_component_is_not_reported_as_installed() {
        let status = broken_status(
            ComponentId::Bench,
            "managed release",
            "invalid receipt".to_string(),
        );
        assert!(!status.installed);
        assert!(status.health.starts_with("broken:"));
    }
}
