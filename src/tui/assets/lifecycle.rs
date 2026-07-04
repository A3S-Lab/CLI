//! Shared AI-native lifecycle metadata for A3S Code team assets.
//!
//! The TUI has different slash commands for different asset families. The
//! product contract is intentionally shared, while each command surface only
//! exposes actions that are real for that asset family.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleStage {
    Create,
    Develop,
    Debug,
    Test,
    Run,
    Publish,
    Deploy,
    Observe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AssetLifecycle {
    pub(crate) family: &'static str,
    pub(crate) command: &'static str,
    pub(crate) os_category: &'static str,
    pub(crate) service: OsService,
    pub(crate) runtime_binding: RuntimeBindingIntent,
    pub(crate) stages: &'static [LifecycleStage],
}

pub(crate) const MCP_STAGES: &[LifecycleStage] = &[
    LifecycleStage::Create,
    LifecycleStage::Develop,
    LifecycleStage::Debug,
    LifecycleStage::Test,
    LifecycleStage::Publish,
    LifecycleStage::Deploy,
    LifecycleStage::Observe,
];

pub(crate) const AGENTIC_AGENT_STAGES: &[LifecycleStage] = &[
    LifecycleStage::Create,
    LifecycleStage::Develop,
    LifecycleStage::Run,
    LifecycleStage::Publish,
    LifecycleStage::Observe,
];

pub(crate) const APPLICATION_AGENT_STAGES: &[LifecycleStage] = &[
    LifecycleStage::Create,
    LifecycleStage::Develop,
    LifecycleStage::Publish,
    LifecycleStage::Deploy,
    LifecycleStage::Observe,
];

pub(crate) const TOOL_AGENT_STAGES: &[LifecycleStage] = &[
    LifecycleStage::Create,
    LifecycleStage::Develop,
    LifecycleStage::Publish,
    LifecycleStage::Observe,
];

pub(crate) const PUBLISH_DEPLOY_OBSERVE_STAGES: &[LifecycleStage] = &[
    LifecycleStage::Create,
    LifecycleStage::Develop,
    LifecycleStage::Publish,
    LifecycleStage::Deploy,
    LifecycleStage::Observe,
];

pub(crate) const WORKFLOW_STAGES: &[LifecycleStage] = &[
    LifecycleStage::Create,
    LifecycleStage::Develop,
    LifecycleStage::Publish,
    LifecycleStage::Run,
    LifecycleStage::Deploy,
    LifecycleStage::Observe,
];

pub(crate) const ASSET_LIFECYCLES: &[AssetLifecycle] = &[
    AssetLifecycle {
        family: "agentic agent",
        command: "/agent",
        os_category: "agent",
        service: OsService::AgentAsAService,
        runtime_binding: RuntimeBindingIntent {
            kind: "agent",
            isolation: "serving",
            runtime_kind: "a3s-agent-service",
            protocol: None,
            agent_kind: Some("agentic"),
        },
        stages: AGENTIC_AGENT_STAGES,
    },
    AssetLifecycle {
        family: "application agent",
        command: "/agent",
        os_category: "agent",
        service: OsService::AgentAsAService,
        runtime_binding: RuntimeBindingIntent {
            kind: "agent",
            isolation: "container",
            runtime_kind: "a3s-agent-service",
            protocol: None,
            agent_kind: Some("application"),
        },
        stages: APPLICATION_AGENT_STAGES,
    },
    AssetLifecycle {
        family: "tool agent",
        command: "/agent",
        os_category: "agent",
        service: OsService::FunctionAsAService,
        runtime_binding: RuntimeBindingIntent {
            kind: "tool",
            isolation: "serving",
            runtime_kind: "a3s-function-service",
            protocol: Some("agent-tool"),
            agent_kind: Some("tool"),
        },
        stages: TOOL_AGENT_STAGES,
    },
    AssetLifecycle {
        family: "MCP server",
        command: "/mcp",
        os_category: "mcp",
        service: OsService::FunctionAsAService,
        runtime_binding: RuntimeBindingIntent {
            kind: "mcp",
            isolation: "serving",
            runtime_kind: "a3s-function-service",
            protocol: Some("mcp"),
            agent_kind: Some("tool"),
        },
        stages: MCP_STAGES,
    },
    AssetLifecycle {
        family: "skill",
        command: "/skill",
        os_category: "skill",
        service: OsService::FunctionAsAService,
        runtime_binding: RuntimeBindingIntent {
            kind: "tool",
            isolation: "serving",
            runtime_kind: "a3s-function-service",
            protocol: Some("skill"),
            agent_kind: Some("tool"),
        },
        stages: PUBLISH_DEPLOY_OBSERVE_STAGES,
    },
    AssetLifecycle {
        family: "OKF knowledge package",
        command: "/okf",
        os_category: "knowledge",
        service: OsService::KnowledgeService,
        runtime_binding: RuntimeBindingIntent {
            kind: "knowledge",
            isolation: "serving",
            runtime_kind: "a3s-knowledge-service",
            protocol: Some("okf"),
            agent_kind: None,
        },
        stages: PUBLISH_DEPLOY_OBSERVE_STAGES,
    },
    AssetLifecycle {
        family: "workflow flow",
        command: "/flow",
        os_category: "workflow",
        service: OsService::WorkflowAsAService,
        runtime_binding: RuntimeBindingIntent {
            kind: "workflow",
            isolation: "native",
            runtime_kind: "a3s-workflow-service",
            protocol: Some("workflow"),
            agent_kind: None,
        },
        stages: WORKFLOW_STAGES,
    },
];

pub(crate) fn workflow_flow_lifecycle() -> &'static AssetLifecycle {
    ASSET_LIFECYCLES
        .iter()
        .find(|item| item.family == "workflow flow")
        .expect("workflow flow lifecycle is part of the built-in contract")
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
    use super::super::panels;
    use super::*;

    #[test]
    fn all_team_asset_families_have_capability_scoped_lifecycle_metadata() {
        let expected = [
            ("agentic agent", AGENTIC_AGENT_STAGES),
            ("application agent", APPLICATION_AGENT_STAGES),
            ("tool agent", TOOL_AGENT_STAGES),
            ("MCP server", MCP_STAGES),
            ("skill", PUBLISH_DEPLOY_OBSERVE_STAGES),
            ("OKF knowledge package", PUBLISH_DEPLOY_OBSERVE_STAGES),
            ("workflow flow", WORKFLOW_STAGES),
        ];

        assert_eq!(ASSET_LIFECYCLES.len(), expected.len());
        for (family, stages) in expected {
            let lifecycle = ASSET_LIFECYCLES
                .iter()
                .find(|item| item.family == family)
                .unwrap_or_else(|| panic!("missing lifecycle row for {family}"));
            assert_eq!(
                lifecycle.stages, stages,
                "{family} must expose only lifecycle stages backed by real commands or OS surfaces"
            );
        }
    }

    #[test]
    fn lifecycle_metadata_does_not_claim_unsupported_debug_or_deploy_surfaces() {
        let lifecycle = |family: &str| {
            ASSET_LIFECYCLES
                .iter()
                .find(|item| item.family == family)
                .unwrap_or_else(|| panic!("missing lifecycle row for {family}"))
        };

        for family in [
            "agentic agent",
            "application agent",
            "skill",
            "OKF knowledge package",
            "tool agent",
            "workflow flow",
        ] {
            assert!(
                !lifecycle(family).stages.contains(&LifecycleStage::Debug),
                "{family} should not claim a direct debug stage"
            );
        }
        for family in ["agentic agent", "tool agent"] {
            assert!(
                !lifecycle(family).stages.contains(&LifecycleStage::Deploy),
                "{family} should not claim the application deploy stage"
            );
        }
        assert!(
            lifecycle("MCP server")
                .stages
                .contains(&LifecycleStage::Debug),
            "MCP servers expose direct debug through /mcp debug"
        );
        assert!(
            lifecycle("MCP server")
                .stages
                .contains(&LifecycleStage::Test),
            "MCP servers expose batch testing through /mcp test"
        );
        for family in ["agentic agent", "workflow flow"] {
            assert!(
                lifecycle(family).stages.contains(&LifecycleStage::Run),
                "{family} should claim the run stage it exposes"
            );
        }
        for family in [
            "application agent",
            "tool agent",
            "MCP server",
            "skill",
            "OKF knowledge package",
        ] {
            assert!(
                !lifecycle(family).stages.contains(&LifecycleStage::Run),
                "{family} should not claim a direct run stage"
            );
        }
    }

    #[test]
    fn workflow_flow_uses_workflow_as_a_service() {
        let flow = ASSET_LIFECYCLES
            .iter()
            .find(|item| item.family == "workflow flow")
            .expect("flow lifecycle row");

        assert_eq!(flow.command, "/flow");
        assert_eq!(flow.os_category, "workflow");
        assert_eq!(flow.service, OsService::WorkflowAsAService);
        assert_eq!(flow.runtime_binding.kind, "workflow");
        assert_eq!(flow.runtime_binding.runtime_kind, "a3s-workflow-service");
        assert_eq!(flow.runtime_binding.protocol, Some("workflow"));
        assert_eq!(flow.runtime_binding.agent_kind, None);
    }

    #[test]
    fn lifecycle_runtime_binding_intents_match_asset_generators() {
        let lifecycle = |family: &str| {
            ASSET_LIFECYCLES
                .iter()
                .find(|item| item.family == family)
                .unwrap_or_else(|| panic!("missing lifecycle row for {family}"))
        };

        for (family, kind, isolation, runtime_kind, protocol, agent_kind) in [
            (
                "agentic agent",
                "agent",
                "serving",
                "a3s-agent-service",
                None,
                Some("agentic"),
            ),
            (
                "application agent",
                "agent",
                "container",
                "a3s-agent-service",
                None,
                Some("application"),
            ),
            (
                "tool agent",
                "tool",
                "serving",
                "a3s-function-service",
                Some("agent-tool"),
                Some("tool"),
            ),
            (
                "MCP server",
                "mcp",
                "serving",
                "a3s-function-service",
                Some("mcp"),
                Some("tool"),
            ),
            (
                "skill",
                "tool",
                "serving",
                "a3s-function-service",
                Some("skill"),
                Some("tool"),
            ),
            (
                "OKF knowledge package",
                "knowledge",
                "serving",
                "a3s-knowledge-service",
                Some("okf"),
                None,
            ),
            (
                "workflow flow",
                "workflow",
                "native",
                "a3s-workflow-service",
                Some("workflow"),
                None,
            ),
        ] {
            let actual = lifecycle(family).runtime_binding;
            assert_eq!(actual.kind, kind, "{family} kind");
            assert_eq!(actual.isolation, isolation, "{family} isolation");
            assert_eq!(actual.runtime_kind, runtime_kind, "{family} runtime kind");
            assert_eq!(actual.protocol, protocol, "{family} protocol");
            assert_eq!(actual.agent_kind, agent_kind, "{family} agent kind");
        }

        let app = lifecycle("application agent");
        assert_eq!(app.runtime_binding.kind, "agent");
        assert_eq!(app.runtime_binding.isolation, "container");
        assert_eq!(app.runtime_binding.runtime_kind, "a3s-agent-service");
        assert_eq!(app.runtime_binding.protocol, None);
        assert_eq!(app.runtime_binding.agent_kind, Some("application"));

        let tool = lifecycle("tool agent");
        assert_eq!(tool.runtime_binding.kind, "tool");
        assert_eq!(tool.runtime_binding.isolation, "serving");
        assert_eq!(tool.runtime_binding.runtime_kind, "a3s-function-service");
        assert_eq!(tool.runtime_binding.protocol, Some("agent-tool"));
        assert_eq!(tool.runtime_binding.agent_kind, Some("tool"));

        let okf = lifecycle("OKF knowledge package");
        assert_eq!(okf.runtime_binding.kind, "knowledge");
        assert_eq!(okf.runtime_binding.isolation, "serving");
        assert_eq!(okf.runtime_binding.runtime_kind, "a3s-knowledge-service");
        assert_eq!(okf.runtime_binding.protocol, Some("okf"));
        assert_eq!(okf.runtime_binding.agent_kind, None);
    }

    #[test]
    fn tool_shaped_assets_stay_on_function_as_a_service() {
        for family in ["tool agent", "MCP server", "skill"] {
            let lifecycle = ASSET_LIFECYCLES
                .iter()
                .find(|item| item.family == family)
                .unwrap_or_else(|| panic!("missing lifecycle row for {family}"));
            assert_eq!(lifecycle.service, OsService::FunctionAsAService);
            assert_eq!(
                lifecycle.runtime_binding.runtime_kind,
                "a3s-function-service"
            );
            assert_eq!(lifecycle.runtime_binding.isolation, "serving");
            assert_eq!(lifecycle.runtime_binding.agent_kind, Some("tool"));
        }
    }

    #[test]
    fn asset_commands_are_capability_based() {
        for input in [
            "clone https://github.com/a/asset.git",
            "list",
            "review",
            "activity",
            "publish agentic",
            "publish application",
            "publish tool",
            "run",
            "deploy",
            "open",
            "logs",
            "status",
        ] {
            assert!(
                panels::agent::parse_agent_subcommand(input).is_some(),
                "/agent should parse lifecycle subcommand `{input}`"
            );
        }
        assert!(
            matches!(panels::agent::parse_agent_subcommand("debug"), Some(Err(_))),
            "/agent should reject debug instead of creating a prototype"
        );
        assert!(
            matches!(panels::agent::parse_agent_subcommand("ps"), Some(Err(_))),
            "/agent should reject legacy ps and point to activity"
        );
        for input in [
            "clone https://github.com/a/asset.git",
            "list",
            "review",
            "activity",
            "publish",
            "debug",
            "test",
            "deploy",
            "open",
            "logs",
            "status",
        ] {
            assert!(
                panels::mcp::parse_mcp_subcommand(input).is_some(),
                "/mcp should parse lifecycle subcommand `{input}`"
            );
        }
        assert!(
            matches!(panels::mcp::parse_mcp_subcommand("run"), Some(Err(_))),
            "/mcp should reject run instead of creating a prototype"
        );
        assert!(
            matches!(panels::mcp::parse_mcp_subcommand("ps"), Some(Err(_))),
            "/mcp should reject legacy ps and point to activity"
        );
        for input in [
            "clone https://github.com/a/asset.git",
            "list",
            "review",
            "activity",
            "publish",
            "deploy",
            "open",
            "status",
        ] {
            assert!(
                matches!(panels::skill::parse_skill_subcommand(input), Some(Ok(_))),
                "/skill should parse lifecycle subcommand `{input}`"
            );
        }
        for input in ["run", "debug", "logs"] {
            assert!(
                matches!(panels::skill::parse_skill_subcommand(input), Some(Err(_))),
                "/skill should reject unsupported subcommand `{input}`"
            );
        }
        assert!(
            matches!(panels::skill::parse_skill_subcommand("ps"), Some(Err(_))),
            "/skill should reject legacy ps and point to activity"
        );
        for input in ["review ops", "deploy ops"] {
            assert!(
                matches!(panels::skill::parse_skill_subcommand(input), Some(Err(_))),
                "/skill should reject target arguments it cannot honor"
            );
        }
        for input in [
            "clone https://github.com/a/asset.git",
            "list",
            "review",
            "activity",
            "publish",
            "run",
            "deploy",
            "open",
            "logs",
            "status",
        ] {
            assert!(
                panels::flow::parse_flow_subcommand(input).is_some(),
                "/flow should parse lifecycle subcommand `{input}`"
            );
        }
        assert!(
            matches!(panels::flow::parse_flow_subcommand("debug"), Some(Err(_))),
            "/flow should reject debug instead of creating a prototype"
        );
        assert!(
            matches!(panels::flow::parse_flow_subcommand("ps"), Some(Err(_))),
            "/flow should reject legacy ps and point to activity"
        );
        for input in [
            "clone https://github.com/a/asset.git",
            "list",
            "review",
            "activity",
            "publish",
            "deploy",
            "status",
        ] {
            assert!(
                !matches!(
                    panels::kb::parse_okf_command(input),
                    panels::kb::KbCommand::Prototype(_)
                ),
                "/okf should parse lifecycle subcommand `{input}`"
            );
        }
        for input in [
            "run",
            "debug",
            "os",
            "open",
            "view",
            "dashboard",
            "add",
            "import",
            "search",
            "vault",
            "logs",
            "ps",
        ] {
            assert!(
                matches!(
                    panels::kb::parse_okf_command(input),
                    panels::kb::KbCommand::Usage(_)
                ),
                "/okf should reject unsupported subcommand `{input}`"
            );
        }
    }
}
