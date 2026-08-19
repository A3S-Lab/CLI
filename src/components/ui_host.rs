//! A3S Code lifecycle composition for cognitive-package UI surfaces.

use std::path::PathBuf;
use std::sync::Arc;

use a3s_use::plugin_lifecycle::{
    PluginLifecycleAction, PluginLifecycleEvidence, PluginLifecycleIntent, PluginUiLifecycleHost,
    PluginUiLifecycleHostFactory, StaticPluginSurfaceLifecycleHost,
};
use a3s_use_core::{UseError, UseResult};
use a3s_use_extension::PluginUiSurface;
use async_trait::async_trait;
use sha2::{Digest, Sha256};

use super::{CodePluginUiStateError, CodePluginUiStateStore, ComponentPaths};

#[derive(Clone)]
pub(crate) struct CodePluginUiLifecycleHostFactory {
    store: CodePluginUiStateStore,
}

impl CodePluginUiLifecycleHostFactory {
    pub(crate) fn from_component_paths(paths: &ComponentPaths) -> Self {
        Self {
            store: CodePluginUiStateStore::from_component_paths(paths),
        }
    }
}

impl PluginUiLifecycleHostFactory for CodePluginUiLifecycleHostFactory {
    fn create(&self, package_root: PathBuf) -> Arc<dyn PluginUiLifecycleHost> {
        Arc::new(CodePluginUiLifecycleHost {
            static_host: StaticPluginSurfaceLifecycleHost::new(package_root),
            store: self.store.clone(),
        })
    }
}

struct CodePluginUiLifecycleHost {
    static_host: StaticPluginSurfaceLifecycleHost,
    store: CodePluginUiStateStore,
}

#[async_trait]
impl PluginUiLifecycleHost for CodePluginUiLifecycleHost {
    async fn prepare_ui(
        &self,
        intent: &PluginLifecycleIntent,
        surface: &PluginUiSurface,
        idempotency_key: &str,
    ) -> UseResult<PluginLifecycleEvidence> {
        self.static_host
            .prepare_ui(intent, surface, idempotency_key)
            .await
    }

    async fn stop_ui(
        &self,
        intent: &PluginLifecycleIntent,
        surface: &PluginUiSurface,
        idempotency_key: &str,
    ) -> UseResult<PluginLifecycleEvidence> {
        self.static_host
            .stop_ui(intent, surface, idempotency_key)
            .await
    }

    async fn remove_ui(
        &self,
        intent: &PluginLifecycleIntent,
        surface: &PluginUiSurface,
        idempotency_key: &str,
    ) -> UseResult<PluginLifecycleEvidence> {
        let static_evidence = self
            .static_host
            .remove_ui(intent, surface, idempotency_key)
            .await?;
        if intent.action != PluginLifecycleAction::Uninstall {
            return Ok(static_evidence);
        }
        let retained = intent
            .retained_ui_state_surfaces
            .binary_search(&surface.id)
            .is_ok();
        if !retained {
            self.store
                .clear_surface(&intent.scope, &intent.package_id, &surface.id)
                .await
                .map_err(ui_state_cleanup_error)?;
        }
        state_cleanup_evidence(
            &static_evidence,
            if retained { "retained" } else { "cleared" },
            intent,
            surface,
            idempotency_key,
        )
    }
}

fn state_cleanup_evidence(
    static_evidence: &PluginLifecycleEvidence,
    outcome: &str,
    intent: &PluginLifecycleIntent,
    surface: &PluginUiSurface,
    idempotency_key: &str,
) -> UseResult<PluginLifecycleEvidence> {
    let identity = format!(
        "ui-state-{outcome}\n{}\n{}\n{}\n{}\n{}\n{}",
        static_evidence.digest(),
        idempotency_key,
        intent.scope.kind.as_str(),
        intent.scope.id,
        intent.package_id,
        surface.id,
    );
    PluginLifecycleEvidence::new(format!("sha256:{:x}", Sha256::digest(identity.as_bytes())))
}

fn ui_state_cleanup_error(error: CodePluginUiStateError) -> UseError {
    UseError::new(
        "use.plugin.ui_state_cleanup_failed",
        format!("The host could not remove retired UI state: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use a3s_use::plugin_lifecycle::{PluginLifecycleIntentSpec, PluginUiLifecycleHost};
    use a3s_use_core::{PlanScope, PlanScopeKind};
    use a3s_use_extension::ExtensionManifest;

    use super::*;

    #[tokio::test]
    async fn lifecycle_preserves_disable_and_upgrade_state_but_clears_true_uninstall() {
        let temp = tempfile::tempdir().unwrap();
        let store = CodePluginUiStateStore::new(temp.path().join("state"));
        let host = CodePluginUiLifecycleHost {
            static_host: StaticPluginSurfaceLifecycleHost::new(temp.path().join("package")),
            store: store.clone(),
        };
        let manifest = ui_manifest();
        let surface = &manifest.ui[0];
        let scope = scope(PlanScopeKind::User, "user/current");
        store
            .set(
                &scope,
                &manifest.package_id,
                &surface.id,
                "draft",
                serde_json::json!({"query": "hot swap"}),
            )
            .await
            .unwrap();

        let disable = lifecycle_intent(&manifest, PluginLifecycleAction::Disable, Vec::new());
        host.stop_ui(&disable, surface, "disable-stop")
            .await
            .unwrap();
        assert!(store
            .get(&scope, &manifest.package_id, &surface.id, "draft")
            .await
            .unwrap()
            .is_some());

        let rollback = lifecycle_intent(
            &manifest,
            PluginLifecycleAction::Upgrade,
            vec![surface.id.clone()],
        );
        host.remove_ui(&rollback, surface, "upgrade-rollback")
            .await
            .unwrap();
        assert!(store
            .get(&scope, &manifest.package_id, &surface.id, "draft")
            .await
            .unwrap()
            .is_some());

        let replacement_retirement = lifecycle_intent(
            &manifest,
            PluginLifecycleAction::Uninstall,
            vec![surface.id.clone()],
        );
        host.remove_ui(&replacement_retirement, surface, "replacement-retirement")
            .await
            .unwrap();
        assert!(store
            .get(&scope, &manifest.package_id, &surface.id, "draft")
            .await
            .unwrap()
            .is_some());

        let uninstall = lifecycle_intent(&manifest, PluginLifecycleAction::Uninstall, Vec::new());
        host.remove_ui(&uninstall, surface, "uninstall-remove")
            .await
            .unwrap();
        host.remove_ui(&uninstall, surface, "uninstall-remove")
            .await
            .unwrap();
        assert_eq!(
            store
                .get(&scope, &manifest.package_id, &surface.id, "draft")
                .await
                .unwrap(),
            None
        );
    }

    fn scope(kind: PlanScopeKind, id: &str) -> PlanScope {
        PlanScope {
            kind,
            id: id.to_string(),
        }
    }

    fn lifecycle_intent(
        manifest: &ExtensionManifest,
        action: PluginLifecycleAction,
        retained_ui_state_surfaces: Vec<String>,
    ) -> PluginLifecycleIntent {
        PluginLifecycleIntent::from_manifest(
            PluginLifecycleIntentSpec {
                operation_id: format!("ui-state-{}", action_name(action)),
                plan_digest: format!("sha256:{}", "1".repeat(64)),
                scope: scope(PlanScopeKind::User, "user/current"),
                package_id: manifest.package_id.clone(),
                package_digest: format!("sha256:{}", "2".repeat(64)),
                manifest_digest: format!("sha256:{}", "3".repeat(64)),
                generation: 7,
                action,
                retained_ui_state_surfaces,
            },
            manifest,
        )
        .unwrap()
    }

    fn action_name(action: PluginLifecycleAction) -> &'static str {
        match action {
            PluginLifecycleAction::Install => "install",
            PluginLifecycleAction::Upgrade => "upgrade",
            PluginLifecycleAction::Enable => "enable",
            PluginLifecycleAction::Disable => "disable",
            PluginLifecycleAction::Uninstall => "uninstall",
        }
    }

    fn ui_manifest() -> ExtensionManifest {
        ExtensionManifest::parse_acl(
            r#"
extension "acme/research" {
  schema_version = 3
  version = "1.0.0"
  route = "research"
  requires_use = ">=0.3.0, <0.4.0"
  actions = ["read"]

  repository {
    url = "https://github.com/acme/research"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }

  ui "review" {
    entry = "ui/review.html"
    styles = []
    scripts = []
    bind_tool = []
    bind_mcp = []
    bind_flow = []
    optional = false
  }
}
"#,
        )
        .unwrap()
    }
}
