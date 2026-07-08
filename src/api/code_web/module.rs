use std::sync::Arc;

use a3s_boot::{Module, ProviderDefinition, ProviderToken, Result as BootResult};

use super::capabilities::CapabilitiesModule;
use super::config::ConfigModule;
use super::context::ContextModule;
use super::health::HealthModule;
use super::kernel::KernelModule;
use super::knowledge::KnowledgeModule;
use super::loops::LoopsModule;
use super::os::OsModule;
use super::plugins::PluginsModule;
use super::processes::ProcessesModule;
use super::state::CodeWebState;
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
            Arc::new(WorkspaceModule),
            Arc::new(CapabilitiesModule),
            Arc::new(KnowledgeModule),
            Arc::new(ContextModule),
            Arc::new(KernelModule),
            Arc::new(ProcessesModule),
            Arc::new(LoopsModule),
            Arc::new(PluginsModule),
            Arc::new(OsModule),
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
