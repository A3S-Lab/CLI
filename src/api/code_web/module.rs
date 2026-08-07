use std::sync::Arc;

use a3s_boot::{Module, ProviderDefinition, ProviderToken, Result as BootResult};

use super::capabilities::CapabilitiesModule;
use super::code_intelligence::CodeIntelligenceModule;
use super::config::ConfigModule;
use super::context::ContextModule;
use super::evolution::EvolutionModule;
use super::health::HealthModule;
use super::kernel::KernelModule;
use super::knowledge::KnowledgeModule;
use super::loops::LoopsModule;
use super::os::OsModule;
use super::plugins::PluginsModule;
use super::previews::PreviewsModule;
use super::processes::ProcessesModule;
use super::state::CodeWebState;
use super::weixin::WeixinModule;
use super::work::WorkModule;
use super::workspace::WorkspaceModule;

pub(in crate::api) struct CodeWebModule {
    state: Arc<CodeWebState>,
}

impl CodeWebModule {
    pub(in crate::api) fn new(state: Arc<CodeWebState>) -> Self {
        Self { state }
    }
}

impl Module for CodeWebModule {
    fn name(&self) -> &'static str {
        "a3s-code-web"
    }

    fn imports(&self) -> Vec<Arc<dyn Module>> {
        vec![
            Arc::new(CodeWebStateModule::new(Arc::clone(&self.state))),
            Arc::new(HealthModule),
            Arc::new(ConfigModule),
            Arc::new(WorkModule),
            Arc::new(WorkspaceModule),
            Arc::new(CodeIntelligenceModule),
            Arc::new(CapabilitiesModule),
            Arc::new(KnowledgeModule),
            Arc::new(ContextModule),
            Arc::new(EvolutionModule),
            Arc::new(KernelModule),
            Arc::new(ProcessesModule),
            Arc::new(PreviewsModule),
            Arc::new(LoopsModule),
            Arc::new(PluginsModule),
            Arc::new(OsModule),
            Arc::new(WeixinModule::configured()),
        ]
    }
}

struct CodeWebStateModule {
    state: Arc<CodeWebState>,
}

impl CodeWebStateModule {
    fn new(state: Arc<CodeWebState>) -> Self {
        Self { state }
    }
}

impl Module for CodeWebStateModule {
    fn name(&self) -> &'static str {
        "a3s-code-web-state"
    }

    fn providers(&self) -> BootResult<Vec<ProviderDefinition>> {
        Ok(vec![ProviderDefinition::from_arc(Arc::clone(&self.state))])
    }

    fn exports(&self) -> BootResult<Vec<ProviderToken>> {
        Ok(vec![ProviderToken::of::<CodeWebState>()])
    }

    fn is_global(&self) -> bool {
        true
    }

    fn on_application_shutdown(
        &self,
        _module_ref: a3s_boot::ModuleRef,
    ) -> a3s_boot::BoxFuture<'static, BootResult<()>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            state.close().await;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use a3s_boot::{BootApplication, BootRequest, HttpMethod};

    use super::*;

    async fn staged_managed_knowledge(
        paths: &a3s_use_extension::ExtensionPaths,
    ) -> a3s_use_core::OkfCapabilityProjection {
        use a3s_use::okf_knowledge::{
            OkfKnowledgeClient, OkfKnowledgeStageRequest, OkfKnowledgeStageSpec,
            SqliteOkfKnowledgeAdapter,
        };
        use a3s_use_core::{
            inspect_okf_bundle_files, OkfBundleContract, OkfBundleFile, OkfBundleLimits,
            OkfCapabilityProjection, OkfFormatVersion, PlanQualifiedSurfaceRef, PlanScope,
            PlanScopeKind, PluginSurfaceKind, PluginSurfaceRef, OKF_BUNDLE_CONTRACT_SCHEMA,
        };

        let files = vec![OkfBundleFile::new(
            "concepts/web.md",
            "---\ntype: Decision\n---\n\n# Web projection\n\nThe API records webmanagedknowledgeneedle.\n",
        )];
        let limits = OkfBundleLimits::default();
        let inspection =
            inspect_okf_bundle_files(OkfFormatVersion::V0_2, limits.clone(), &files).unwrap();
        let client = OkfKnowledgeClient::new(Arc::new(
            SqliteOkfKnowledgeAdapter::from_extension_paths(paths),
        ));
        let staged = client
            .stage(
                OkfKnowledgeStageRequest::new(
                    OkfKnowledgeStageSpec {
                        operation_id: "fixture-web-knowledge-install".to_string(),
                        scope: PlanScope {
                            kind: PlanScopeKind::Workspace,
                            id: "fixture-workspace".to_string(),
                        },
                        surface: PlanQualifiedSurfaceRef {
                            package_id: "acme/web-knowledge".to_string(),
                            surface: PluginSurfaceRef {
                                kind: PluginSurfaceKind::Okf,
                                id: "domain".to_string(),
                            },
                        },
                        generation: 4,
                        package_digest: format!("sha256:{}", "c".repeat(64)),
                        manifest_digest: format!("sha256:{}", "d".repeat(64)),
                        bundle: OkfBundleContract {
                            schema: OKF_BUNDLE_CONTRACT_SCHEMA.to_string(),
                            format_version: inspection.format_version,
                            root: "knowledge".to_string(),
                            content_digest: inspection.content_digest,
                            concept_count: inspection.concept_count,
                            file_count: inspection.file_count,
                            expanded_bytes: inspection.expanded_bytes,
                            limits,
                        },
                    },
                    files,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let binding = client.promote(&staged.receipt).await.unwrap();
        OkfCapabilityProjection::from_promoted(&binding.receipt, &binding.observation).unwrap()
    }

    #[tokio::test]
    async fn complete_code_web_module_builds_with_nested_remote_kernel_imports() {
        let temporary = tempfile::tempdir().expect("create Code Web module fixture");
        let workspace = temporary.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create fixture workspace");
        let code_config = a3s_code_core::CodeConfig::from_acl(
            r#"
                default_model = "openai/test-model"
                providers "openai" {
                  apiKey = "sk-test"
                  baseUrl = "https://example.com/v1"
                  models "test-model" {}
                }
            "#,
        )
        .expect("parse fixture config");
        let agent = Arc::new(
            a3s_code_core::Agent::from_config(code_config.clone())
                .await
                .expect("create fixture agent"),
        );
        let repository = Arc::new(
            super::super::session_store::CodeWebSessionRepository::open(
                temporary.path().join("sessions"),
            )
            .await
            .expect("open fixture session repository"),
        );
        let state = Arc::new(CodeWebState::new_for_test(
            agent,
            temporary.path().join("config.acl"),
            workspace,
            code_config,
            repository,
        ));
        let app = BootApplication::builder()
            .global_prefix("/api")
            .import(CodeWebModule::new(Arc::clone(&state)))
            .build()
            .expect("build complete Code Web application");

        let capability = app
            .call(BootRequest::new(
                HttpMethod::Get,
                "/api/v1/weixin/capability",
            ))
            .await
            .expect("read built-in Weixin capability")
            .body_json::<serde_json::Value>()
            .expect("decode capability");
        assert_eq!(capability["state"], "unbound");
        assert_eq!(capability["protocolMode"], "tencent");
        assert_eq!(capability["schemaVersion"], 2);
        assert_eq!(capability["releaseBlockers"], serde_json::json!([]));

        let office = app
            .call(BootRequest::new(
                HttpMethod::Get,
                "/api/v1/capabilities/office",
            ))
            .await
            .expect("read Office automation capability")
            .body_json::<serde_json::Value>()
            .expect("decode Office automation capability");
        assert_eq!(office["schemaVersion"], 1);
        assert_eq!(office["status"], "preparing");
        assert_eq!(office["route"], "use/office");
        assert_eq!(office["cli"]["name"], "a3s-office");
        assert_eq!(office["skill"]["name"], "a3s-office");
        assert_eq!(office["editors"]["office"], true);
        assert_eq!(office["editors"]["code"], true);

        let targets = app
            .call(BootRequest::new(HttpMethod::Get, "/api/v1/weixin/targets"))
            .await
            .expect("read remote target snapshot")
            .body_json::<serde_json::Value>()
            .expect("decode target snapshot");
        assert_eq!(targets["schemaVersion"], 1);
        assert!(
            targets["items"].is_array(),
            "system-agent discovery is host-dependent but must return an item array"
        );
        assert_eq!(targets["warnings"], serde_json::json!([]));

        let knowledge_paths = a3s_use_extension::ExtensionPaths::new(
            temporary.path().join("use-data"),
            temporary.path().join("use-state"),
        );
        let projection = staged_managed_knowledge(&knowledge_paths).await;
        state
            .install_use_registry(
                crate::use_registry::UseRegistryHandle::for_test_knowledge(
                    knowledge_paths,
                    7,
                    vec![projection],
                ),
                None,
            )
            .await;
        let managed_knowledge = app
            .call(BootRequest::new(
                HttpMethod::Get,
                "/api/v1/knowledge/packages",
            ))
            .await
            .expect("read managed Knowledge catalog through Code Web")
            .body_json::<serde_json::Value>()
            .expect("decode managed Knowledge catalog");
        assert_eq!(managed_knowledge["schemaVersion"], 1);
        assert_eq!(managed_knowledge["generation"], 7);
        assert_eq!(managed_knowledge["projections"][0]["generation"], 4);

        let managed_search = app
            .call(
                BootRequest::new(HttpMethod::Post, "/api/v1/knowledge/packages/search")
                    .with_content_type("application/json")
                    .with_body(
                        r#"{"query":"webmanagedknowledgeneedle","limit":5,"scopeKind":"workspace","scopeId":"fixture-workspace"}"#,
                    ),
            )
            .await
            .expect("search managed Knowledge through Code Web")
            .body_json::<serde_json::Value>()
            .expect("decode managed Knowledge search results");
        assert_eq!(managed_search["registryGeneration"], 7);
        assert_eq!(managed_search["scope"]["kind"], "workspace");
        assert_eq!(managed_search["hits"][0]["citation"]["generation"], 4);

        let incomplete_scope = app
            .call(
                BootRequest::new(HttpMethod::Post, "/api/v1/knowledge/packages/search")
                    .with_content_type("application/json")
                    .with_body(r#"{"query":"fixture","scopeKind":"workspace"}"#),
            )
            .await
            .expect_err("an incomplete managed Knowledge scope must fail validation");
        assert!(matches!(
            incomplete_scope,
            a3s_boot::BootError::BadRequest(message) if message.contains("provided together")
        ));

        app.shutdown().await.expect("shutdown Code Web application");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn complete_code_web_module_honors_explicit_weixin_enable() {
        let temporary = tempfile::tempdir().expect("create Code Web module fixture");
        let workspace = temporary.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create fixture workspace");
        let source = r#"
            default_model = "openai/test-model"
            providers "openai" {
              apiKey = "sk-test"
              baseUrl = "https://example.com/v1"
              models "test-model" {}
            }
            channels {
              weixin {
                enabled = true
              }
            }
        "#;
        let config_path = temporary.path().join("config.acl");
        std::fs::write(&config_path, source).expect("write configured fixture");
        let code_config =
            a3s_code_core::CodeConfig::from_acl(source).expect("parse configured fixture");
        let agent = Arc::new(
            a3s_code_core::Agent::from_config(code_config.clone())
                .await
                .expect("create fixture agent"),
        );
        let repository = Arc::new(
            super::super::session_store::CodeWebSessionRepository::open(
                temporary.path().join("sessions"),
            )
            .await
            .expect("open fixture session repository"),
        );
        let state = Arc::new(CodeWebState::new_for_test(
            agent,
            config_path,
            workspace,
            code_config,
            repository,
        ));
        let app = BootApplication::builder()
            .global_prefix("/api")
            .import(CodeWebModule::new(state))
            .build()
            .expect("build configured Code Web application");

        app.bootstrap()
            .await
            .expect("bootstrap configured Code Web application");
        let capability = app
            .call(BootRequest::new(
                HttpMethod::Get,
                "/api/v1/weixin/capability",
            ))
            .await
            .expect("read configured Weixin capability")
            .body_json::<serde_json::Value>()
            .expect("decode configured capability");
        assert_eq!(capability["state"], "unbound");
        assert_eq!(capability["protocolMode"], "tencent");
        assert_eq!(capability["schemaVersion"], 2);
        assert_eq!(capability["releaseBlockers"], serde_json::json!([]));

        let account = app
            .call(BootRequest::new(HttpMethod::Get, "/api/v1/weixin/account"))
            .await
            .expect("read configured Weixin account")
            .body_json::<serde_json::Value>()
            .expect("decode configured account");
        assert_eq!(account["bound"], false);
        assert_eq!(account["protocolMode"], "tencent");

        app.shutdown()
            .await
            .expect("shutdown configured Code Web application");
    }
}
