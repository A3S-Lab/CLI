//! Shared AI-native asset ACL helpers used by `/evolution` materialization.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum OsService {
    AgentAsAService,
    FunctionAsAService,
    WorkflowAsAService,
    KnowledgeService,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeBindingIntent {
    pub(crate) kind: &'static str,
    pub(crate) isolation: &'static str,
    pub(crate) runtime_kind: &'static str,
    pub(crate) protocol: Option<&'static str>,
    pub(crate) agent_kind: Option<&'static str>,
}

pub(crate) const ASSET_ACL_PATH: &str = ".a3s/asset.acl";

pub(crate) struct AssetAclDocument<'a> {
    pub(crate) category: &'a str,
    pub(crate) kind: Option<&'a str>,
    pub(crate) name: &'a str,
    pub(crate) description: &'a str,
    pub(crate) local_path: Option<&'a str>,
    pub(crate) service: OsService,
    pub(crate) runtime: RuntimeBindingIntent,
    pub(crate) source: &'a [(&'a str, &'a str)],
    pub(crate) metadata: &'a [(&'a str, &'a str)],
}

pub(crate) fn render_asset_acl(doc: AssetAclDocument<'_>) -> String {
    let mut out = String::new();
    out.push_str("version = \"a3s.asset.v1\"\n");
    out.push_str(&format!("category = {}\n", acl_string(doc.category)));
    if let Some(kind) = doc.kind {
        out.push_str(&format!("kind = {}\n", acl_string(kind)));
    }
    out.push_str(&format!("name = {}\n", acl_string(doc.name)));
    out.push_str(&format!("description = {}\n", acl_string(doc.description)));
    if let Some(local_path) = doc.local_path {
        out.push_str(&format!("local_path = {}\n", acl_string(local_path)));
    }
    out.push_str(&format!(
        "service = {}\n",
        acl_string(service_label(doc.service))
    ));
    out.push_str("created_by = \"a3s-code-tui\"\n\n");

    out.push_str("source {\n");
    for (key, value) in doc.source {
        out.push_str(&format!("  {} = {}\n", acl_key(key), acl_string(value)));
    }
    out.push_str("}\n\n");

    out.push_str("metadata {\n");
    out.push_str(&format!(
        "  asset_acl_path = {}\n",
        acl_string(ASSET_ACL_PATH)
    ));
    for (key, value) in doc.metadata {
        out.push_str(&format!("  {} = {}\n", acl_key(key), acl_string(value)));
    }
    out.push_str("}\n\n");

    out.push_str("runtime {\n");
    out.push_str(&format!("  kind = {}\n", acl_string(doc.runtime.kind)));
    out.push_str(&format!(
        "  isolation = {}\n",
        acl_string(doc.runtime.isolation)
    ));
    out.push_str(&format!(
        "  runtime_kind = {}\n",
        acl_string(doc.runtime.runtime_kind)
    ));
    if let Some(protocol) = doc.runtime.protocol {
        out.push_str(&format!("  protocol = {}\n", acl_string(protocol)));
    }
    if let Some(agent_kind) = doc.runtime.agent_kind {
        out.push_str(&format!("  agent_kind = {}\n", acl_string(agent_kind)));
    }
    out.push_str("}\n");
    out
}

pub(crate) fn write_asset_acl(
    asset_root_or_file: &std::path::Path,
    content: &str,
) -> Result<(), String> {
    let asset_root = if asset_root_or_file.is_file() {
        asset_root_or_file
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
    } else {
        asset_root_or_file
    };
    let path = asset_root.join(ASSET_ACL_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("could not write {}: {e}", path.display()))
}

fn acl_key(key: &str) -> String {
    key.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn acl_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

pub(crate) fn service_label(service: OsService) -> &'static str {
    match service {
        OsService::AgentAsAService => "Agent as a Service",
        OsService::FunctionAsAService => "Function as a Service",
        OsService::WorkflowAsAService => "Workflow as a Service",
        OsService::KnowledgeService => "Knowledge service",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_asset_acl_includes_runtime_binding() {
        let rendered = render_asset_acl(AssetAclDocument {
            category: "skill",
            kind: Some("tool"),
            name: "demo",
            description: "demo skill",
            local_path: Some(".a3s/skills/demo"),
            service: OsService::FunctionAsAService,
            runtime: RuntimeBindingIntent {
                kind: "tool",
                isolation: "serving",
                runtime_kind: "a3s-function-service",
                protocol: Some("skill"),
                agent_kind: Some("tool"),
            },
            source: &[("origin", "evolution")],
            metadata: &[("candidate_id", "c1")],
        });
        assert!(rendered.contains("category = \"skill\""));
        assert!(rendered.contains("runtime_kind = \"a3s-function-service\""));
        assert!(rendered.contains("protocol = \"skill\""));
        assert!(rendered.contains("asset_acl_path"));
    }
}
