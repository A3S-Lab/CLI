use std::path::PathBuf;

use a3s_use_core::{PluginManagedScope, UseError, UseResult};

use crate::plugin_manager::operation::lock::PluginMutationLock;
use crate::plugin_manager::operation::store::io::{
    read_optional_record, write_new_record, write_replace_record, WriteDisposition,
};
use crate::plugin_manager::{PluginManagerError, PluginManagerResult};

/// Durable source of truth for one remote workspace mutation fence.
///
/// Provisioning is deliberately separate from [`super::ManagedPluginHostManager`]
/// request handling. A remote request can verify this record but can never
/// create, replace, or advance it implicitly.
#[derive(Clone, Debug)]
pub struct PluginManagedScopeFenceStore {
    root: PathBuf,
}

impl PluginManagedScopeFenceStore {
    pub(crate) fn from_state_root(state_root: &std::path::Path) -> Self {
        Self {
            root: state_root.join("plugin-manager/managed-host"),
        }
    }

    /// Initialize an absent fence from a trusted local enrollment boundary.
    /// Repeating the exact value is idempotent; a different value fails.
    pub async fn initialize(&self, scope: PluginManagedScope) -> UseResult<()> {
        scope.validate()?;
        let _guard = self.acquire().await?;
        let path = self.fence_path();
        run_store(
            move || match read_optional_record::<PluginManagedScope>(&path)? {
                Some(current) => {
                    current.validate().map_err(manager_contract_error)?;
                    if current == scope {
                        Ok(())
                    } else {
                        Err(PluginManagerError::InvalidRequest(
                            "a different managed scope fence is already provisioned".to_string(),
                        ))
                    }
                }
                None => match write_new_record(&path, &scope)? {
                    WriteDisposition::Created => Ok(()),
                    WriteDisposition::AlreadyExists => Err(PluginManagerError::Infrastructure(
                        "managed scope fence appeared during initialization".to_string(),
                    )),
                },
            },
        )
        .await
        .map_err(store_error)
    }

    /// Compare and advance the trusted local fence without changing its host
    /// or workspace identity. Authority rotation is explicit and generation
    /// monotonic; request handling never calls this method.
    pub async fn compare_and_advance(
        &self,
        expected: PluginManagedScope,
        next: PluginManagedScope,
    ) -> UseResult<()> {
        expected.validate()?;
        next.validate()?;
        if next.host_id != expected.host_id
            || next.scope_id != expected.scope_id
            || next.fence_generation <= expected.fence_generation
        {
            return Err(UseError::new(
                "use.plugin.managed_scope_rotation_invalid",
                "A managed scope fence advance must retain host and workspace identity and increase its generation.",
            ));
        }
        let _guard = self.acquire().await?;
        let path = self.fence_path();
        run_store(move || {
            let current = read_optional_record::<PluginManagedScope>(&path)?.ok_or_else(|| {
                PluginManagerError::InvalidRequest(
                    "the managed scope fence is not provisioned".to_string(),
                )
            })?;
            current.validate().map_err(manager_contract_error)?;
            if current != expected {
                return Err(PluginManagerError::InvalidRequest(
                    "the expected managed scope fence is stale".to_string(),
                ));
            }
            write_replace_record(&path, &next)
        })
        .await
        .map_err(store_error)
    }

    /// Read the currently provisioned fence. Absence remains explicit.
    pub async fn current(&self) -> UseResult<Option<PluginManagedScope>> {
        let path = self.fence_path();
        let current = run_store(move || read_optional_record::<PluginManagedScope>(&path))
            .await
            .map_err(store_error)?;
        if let Some(scope) = &current {
            scope.validate()?;
        }
        Ok(current)
    }

    pub(super) async fn lock_and_verify(
        &self,
        requested: &PluginManagedScope,
    ) -> UseResult<PluginMutationLock> {
        requested.validate()?;
        let guard = self.acquire().await?;
        let current = self.current().await?.ok_or_else(|| {
            UseError::new(
                "use.plugin.managed_scope_unavailable",
                "No managed workspace mutation fence is provisioned on this host.",
            )
        })?;
        requested.verify_current_fence(&current)?;
        Ok(guard)
    }

    async fn acquire(&self) -> UseResult<PluginMutationLock> {
        PluginMutationLock::acquire(self.root.join("mutation.lock"))
            .await
            .map_err(store_error)
    }

    fn fence_path(&self) -> PathBuf {
        self.root.join("fence.json")
    }
}

async fn run_store<T: Send + 'static>(
    operation: impl FnOnce() -> PluginManagerResult<T> + Send + 'static,
) -> PluginManagerResult<T> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!("managed scope store task failed: {error}"))
        })?
}

fn manager_contract_error(error: UseError) -> PluginManagerError {
    PluginManagerError::Infrastructure(error.to_string())
}

fn store_error(error: PluginManagerError) -> UseError {
    let (code, message) = match error {
        PluginManagerError::InvalidRequest(_) => (
            "use.plugin.managed_scope_conflict",
            "The managed workspace mutation fence conflicts with durable host state.",
        ),
        PluginManagerError::Timeout(_)
        | PluginManagerError::OperationFailed(_)
        | PluginManagerError::Upstream(_)
        | PluginManagerError::Infrastructure(_) => (
            "use.plugin.managed_scope_store_unavailable",
            "The durable managed workspace mutation fence is unavailable.",
        ),
    };
    UseError::new(code, message)
}
