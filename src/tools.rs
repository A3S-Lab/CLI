//! Read-only component status and local A3S tool discovery for `a3s list`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolEntry {
    command: String,
    binary: String,
    path: PathBuf,
}

pub(crate) fn print_tool_list() {
    println!("managed components");
    for status in crate::components::statuses() {
        println!("  {}", status.id.as_str());
        println!(
            "    installed: {}",
            if status.installed { "yes" } else { "no" }
        );
        println!(
            "    version:   {}",
            status.version.as_deref().unwrap_or("-")
        );
        println!("    source:    {}", status.source);
        println!(
            "    path:      {}",
            status
                .path
                .as_deref()
                .map(Path::display)
                .map(|path| path.to_string())
                .unwrap_or_else(|| "-".to_string())
        );
        println!("    health:    {}", status.health);
    }

    println!("\nmanaged search runtimes");
    for status in a3s_search::browser_management::browser_statuses() {
        crate::search_cmd::print_browser_status(&status);
    }

    let tools = discover_a3s_tools(std::env::var_os("PATH"))
        .into_iter()
        // The Box adapter already reports the selected a3s-box above. A
        // standalone a3s-code or a3s-bench on PATH is still an external tool
        // and must remain visible rather than being mistaken for a managed
        // component.
        .filter(|tool| tool.command != crate::components::ComponentId::Box.as_str())
        .collect::<Vec<_>>();
    println!("\nother a3s-* tools on PATH");
    if tools.is_empty() {
        println!("  none found");
        return;
    }
    for tool in tools {
        println!(
            "  {:<10} {} ({})",
            tool.command,
            tool.binary,
            tool.path.display()
        );
    }
}

fn discover_a3s_tools(path_env: Option<OsString>) -> Vec<ToolEntry> {
    let Some(path_env) = path_env else {
        return Vec::new();
    };

    let mut by_command = BTreeMap::new();
    for dir in std::env::split_paths(&path_env) {
        let Ok(entries) = std::fs::read_dir(dir) else {
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
            let Some(command) = binary.strip_prefix("a3s-") else {
                continue;
            };
            if command.is_empty() {
                continue;
            }
            by_command.entry(command.to_string()).or_insert(ToolEntry {
                command: command.to_string(),
                binary: binary.to_string(),
                path,
            });
        }
    }

    by_command.into_values().collect()
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_a3s_tools_on_path_once_sorted_by_command() {
        let root = std::env::temp_dir().join(format!("a3s-tools-test-{}", std::process::id()));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        make_executable(&first.join("a3s-box"));
        make_executable(&first.join("a3s-search"));
        make_executable(&second.join("a3s-box"));
        make_executable(&second.join("not-a3s-tool"));

        let path = std::env::join_paths([first.clone(), second]).unwrap();
        let tools = discover_a3s_tools(Some(path));

        let names = tools
            .iter()
            .map(|tool| (tool.command.as_str(), tool.binary.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(names, vec![("box", "a3s-box"), ("search", "a3s-search")]);
        assert_eq!(tools[0].path, first.join("a3s-box"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn discover_handles_missing_path() {
        assert!(discover_a3s_tools(None).is_empty());
    }

    #[test]
    fn only_the_selected_box_adapter_is_removed_from_legacy_discovery() {
        let tools = vec![
            ToolEntry {
                command: "box".to_string(),
                binary: "a3s-box".to_string(),
                path: PathBuf::from("/tmp/a3s-box"),
            },
            ToolEntry {
                command: "code".to_string(),
                binary: "a3s-code".to_string(),
                path: PathBuf::from("/tmp/a3s-code"),
            },
            ToolEntry {
                command: "bench".to_string(),
                binary: "a3s-bench".to_string(),
                path: PathBuf::from("/tmp/a3s-bench"),
            },
            ToolEntry {
                command: "search".to_string(),
                binary: "a3s-search".to_string(),
                path: PathBuf::from("/tmp/a3s-search"),
            },
        ];
        let external = tools
            .into_iter()
            .filter(|tool| tool.command != crate::components::ComponentId::Box.as_str())
            .map(|tool| tool.command)
            .collect::<Vec<_>>();
        assert_eq!(external, vec!["code", "bench", "search"]);
    }

    fn make_executable(path: &Path) {
        std::fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}
