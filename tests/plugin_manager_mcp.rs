mod support;

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use support::{a3s_bin, configure_component_env, TempWorkspace};

#[test]
fn standard_mcp_inventory_is_exact_v4_and_apply_fails_closed() {
    let temp = TempWorkspace::new("plugin-manager-mcp-contract");
    let config = temp.path("config/config.acl");
    let workspace = temp.path("workspace");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(&config, "").unwrap();

    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let child = command
        .arg("--config")
        .arg(&config)
        .arg("--directory")
        .arg(&workspace)
        .args([
            "--offline",
            "--non-interactive",
            "--no-progress",
            "plugin",
            "mcp-serve",
        ])
        .env("A3S_USE_INSTALL_DIR", temp.path("missing-use"))
        .env("A3S_NO_AUTO_INSTALL", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch Plugin Manager MCP");
    let mut child = ChildGuard(child);
    let mut stdin = child.0.stdin.take().expect("Plugin Manager MCP stdin");
    let stdout = child.0.stdout.take().expect("Plugin Manager MCP stdout");
    let mut reader = BufReader::new(stdout);

    write_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "a3s-plugin-manager-contract", "version": "1"}
            }
        }),
    );
    let initialized = read_response(&mut reader, 1);
    assert_eq!(
        initialized["result"]["serverInfo"]["name"],
        "a3s-use-plugin-manager"
    );

    write_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    );
    write_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );
    let listed = read_response(&mut reader, 2);
    let tools = listed["result"]["tools"]
        .as_array()
        .expect("Plugin Manager MCP tools");
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "plugin_search",
            "plugin_inspect",
            "plugin_list_installed",
            "plugin_status",
            "plugin_plan_install",
            "plugin_plan_upgrade",
            "plugin_plan_uninstall",
            "plugin_apply_plan",
            "plugin_plan_enable",
            "plugin_plan_disable",
        ])
    );
    assert!(tools
        .iter()
        .filter(|tool| tool["name"] != "plugin_apply_plan")
        .all(|tool| {
            tool["annotations"]["readOnlyHint"] == true
                && tool["annotations"]["destructiveHint"] == false
        }));
    let apply = tools
        .iter()
        .find(|tool| tool["name"] == "plugin_apply_plan")
        .unwrap();
    assert_eq!(apply["annotations"]["readOnlyHint"], false);
    assert_eq!(apply["annotations"]["destructiveHint"], true);
    let install = tools
        .iter()
        .find(|tool| tool["name"] == "plugin_plan_install")
        .unwrap();
    assert_eq!(
        install["inputSchema"]["properties"]["registryName"]["pattern"],
        "^[a-z][a-z0-9-]{0,62}$"
    );
    let upgrade = tools
        .iter()
        .find(|tool| tool["name"] == "plugin_plan_upgrade")
        .unwrap();
    assert!(upgrade["inputSchema"]["properties"]["registryName"].is_null());

    write_tool_call(
        &mut stdin,
        3,
        "plugin_list_installed",
        serde_json::json!({
            "scopeKind": "user",
            "scopeId": "user/current",
            "limit": 1
        }),
    );
    let installed = read_response(&mut reader, 3);
    assert_eq!(installed["result"]["isError"], false);
    assert_eq!(
        installed["result"]["structuredContent"]["scope"],
        serde_json::json!({"kind": "user", "id": "user/current"})
    );
    assert_eq!(
        installed["result"]["structuredContent"]["packages"],
        serde_json::json!([])
    );

    write_tool_call(
        &mut stdin,
        4,
        "plugin_search",
        serde_json::json!({
            "query": "science",
            "registryUrl": "https://example.invalid"
        }),
    );
    let arbitrary_source = read_response(&mut reader, 4);
    assert_eq!(arbitrary_source["error"]["code"], -32602);

    write_tool_call(
        &mut stdin,
        5,
        "plugin_apply_plan",
        serde_json::json!({
            "operationId": "plugin-install-forbidden",
            "planDigest": format!("sha256:{}", "a".repeat(64))
        }),
    );
    let forbidden = read_response(&mut reader, 5);
    assert_eq!(forbidden["result"]["isError"], true);
    assert_eq!(
        forbidden["result"]["structuredContent"]["code"],
        "use.plugin.host_plan_missing"
    );
    assert!(
        !temp.path("state/use/plugin-host-manager").exists(),
        "an unknown apply identity must not create a host plan or apply intent"
    );
}

fn write_tool_call(stdin: &mut impl Write, id: u64, name: &str, arguments: serde_json::Value) {
    write_message(
        stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }),
    );
}

fn write_message(stdin: &mut impl Write, value: serde_json::Value) {
    serde_json::to_writer(&mut *stdin, &value).expect("write Plugin Manager MCP message");
    stdin
        .write_all(b"\n")
        .expect("terminate Plugin Manager MCP message");
    stdin.flush().expect("flush Plugin Manager MCP message");
}

fn read_response(reader: &mut impl BufRead, id: u64) -> serde_json::Value {
    for _ in 0..20 {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .expect("read Plugin Manager MCP response");
        assert!(
            bytes > 0,
            "Plugin Manager MCP closed before responding to request {id}"
        );
        let value: serde_json::Value = serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!("invalid Plugin Manager MCP response ({error}): {line}")
        });
        if value["id"].as_u64() == Some(id) {
            return value;
        }
    }
    panic!("Plugin Manager MCP did not respond to request {id}");
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
