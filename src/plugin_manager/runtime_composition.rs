//! Trusted A3S Code Runtime provider composition.
//!
//! Provider configuration is read only from the same explicit or user-level
//! ACL source that owns plugin authorization. Workspace configuration cannot
//! select a host Runtime provider.

use std::path::Path;
#[cfg(target_os = "linux")]
use std::time::Duration;

use a3s_acl::{Block, Value};
use tokio::io::AsyncReadExt;

use super::{PluginManagerError, PluginManagerResult, PluginRuntimeHost};
use crate::components::ComponentPaths;

const PLUGIN_RUNTIME_HOST_SCHEMA: &str = "a3s.plugin-runtime-host.v1";
const MAX_RUNTIME_CONFIG_BYTES: usize = 256 * 1024;
const DEFAULT_CONTROL_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_TASK_POLL_INTERVAL_MS: u64 = 50;
const MAX_CONTROL_TIMEOUT_MS: u64 = 5 * 60 * 1000;
const MAX_TASK_POLL_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginRuntimeHostConfig {
    Disabled,
    Box(BoxRuntimeConfig),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoxRuntimeConfig {
    isolation: BoxRuntimeIsolation,
    control_timeout_ms: u64,
    task_poll_interval_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoxRuntimeIsolation {
    Microvm,
    Sandbox,
}

pub(super) async fn compose(
    source: Option<&Path>,
    paths: &ComponentPaths,
) -> PluginManagerResult<PluginRuntimeHost> {
    let Some(source) = source else {
        return Ok(PluginRuntimeHost::default());
    };
    let config = PluginRuntimeHostConfig::from_acl_file(source).await?;
    compose_config(config, paths)
}

impl PluginRuntimeHostConfig {
    fn from_acl(source: &str) -> PluginManagerResult<Self> {
        if source.len() > MAX_RUNTIME_CONFIG_BYTES {
            return Err(config_error(format!(
                "ACL input must not exceed {MAX_RUNTIME_CONFIG_BYTES} bytes"
            )));
        }
        if source.trim().is_empty() {
            return Ok(Self::Disabled);
        }
        let document = a3s_acl::parse_acl(source)
            .map_err(|error| config_error(format!("ACL parsing failed: {error}")))?;
        let blocks = document
            .blocks
            .iter()
            .filter(|block| block.name == "plugin_runtime")
            .collect::<Vec<_>>();
        match blocks.as_slice() {
            [] => Ok(Self::Disabled),
            [block] => parse_runtime_block(block),
            _ => Err(config_error(
                "the A3S ACL document contains more than one `plugin_runtime` block",
            )),
        }
    }

    async fn from_acl_file(path: &Path) -> PluginManagerResult<Self> {
        let file = tokio::fs::File::open(path).await.map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "could not open plugin Runtime configuration {}: {error}",
                path.display()
            ))
        })?;
        let mut bytes = Vec::new();
        file.take((MAX_RUNTIME_CONFIG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| {
                PluginManagerError::Infrastructure(format!(
                    "could not read plugin Runtime configuration {}: {error}",
                    path.display()
                ))
            })?;
        if bytes.len() > MAX_RUNTIME_CONFIG_BYTES {
            return Err(config_error(format!(
                "ACL input must not exceed {MAX_RUNTIME_CONFIG_BYTES} bytes"
            )));
        }
        let source = std::str::from_utf8(&bytes).map_err(|_| {
            config_error(format!(
                "plugin Runtime configuration {} must be valid UTF-8",
                path.display()
            ))
        })?;
        Self::from_acl(source)
    }
}

fn parse_runtime_block(block: &Block) -> PluginManagerResult<PluginRuntimeHostConfig> {
    if !block.labels.is_empty() {
        return Err(config_error(
            "the `plugin_runtime` block does not accept labels",
        ));
    }
    reject_unknown_attributes(block, &["schema"], "plugin_runtime")?;
    let schema = required_string(block, "schema")?;
    if schema != PLUGIN_RUNTIME_HOST_SCHEMA {
        return Err(config_error(format!(
            "unsupported plugin Runtime host schema `{schema}`"
        )));
    }
    if block.blocks.len() != 1 || block.blocks[0].name != "box" {
        return Err(config_error(
            "the `plugin_runtime` block requires exactly one typed `box` provider block",
        ));
    }
    parse_box_block(&block.blocks[0]).map(PluginRuntimeHostConfig::Box)
}

fn parse_box_block(block: &Block) -> PluginManagerResult<BoxRuntimeConfig> {
    if !block.labels.is_empty() || !block.blocks.is_empty() {
        return Err(config_error(
            "the `plugin_runtime.box` block accepts no labels or nested blocks",
        ));
    }
    reject_unknown_attributes(
        block,
        &["isolation", "control_timeout_ms", "task_poll_interval_ms"],
        "plugin_runtime.box",
    )?;
    let isolation = match required_string(block, "isolation")? {
        "microvm" => BoxRuntimeIsolation::Microvm,
        "sandbox" => BoxRuntimeIsolation::Sandbox,
        value => {
            return Err(config_error(format!(
                "`plugin_runtime.box.isolation` must be `microvm` or `sandbox`, not `{value}`"
            )))
        }
    };
    let control_timeout_ms = optional_positive_integer(
        block,
        "control_timeout_ms",
        DEFAULT_CONTROL_TIMEOUT_MS,
        MAX_CONTROL_TIMEOUT_MS,
    )?;
    let task_poll_interval_ms = optional_positive_integer(
        block,
        "task_poll_interval_ms",
        DEFAULT_TASK_POLL_INTERVAL_MS,
        MAX_TASK_POLL_INTERVAL_MS,
    )?;
    if task_poll_interval_ms >= control_timeout_ms {
        return Err(config_error(
            "`plugin_runtime.box.task_poll_interval_ms` must be smaller than `control_timeout_ms`",
        ));
    }
    Ok(BoxRuntimeConfig {
        isolation,
        control_timeout_ms,
        task_poll_interval_ms,
    })
}

fn required_string<'a>(block: &'a Block, name: &str) -> PluginManagerResult<&'a str> {
    match block.attributes.get(name) {
        Some(Value::String(value)) if !value.is_empty() && value.trim() == value => Ok(value),
        Some(_) => Err(config_error(format!(
            "`{name}` must be a non-empty trimmed string"
        ))),
        None => Err(config_error(format!("`{name}` is required"))),
    }
}

fn optional_positive_integer(
    block: &Block,
    name: &str,
    default: u64,
    maximum: u64,
) -> PluginManagerResult<u64> {
    let Some(value) = block.attributes.get(name) else {
        return Ok(default);
    };
    let Value::Number(value) = value else {
        return Err(config_error(format!("`{name}` must be an integer")));
    };
    if !value.is_finite() || value.fract() != 0.0 || *value < 1.0 || *value > maximum as f64 {
        return Err(config_error(format!(
            "`{name}` must be an integer between 1 and {maximum}"
        )));
    }
    Ok(*value as u64)
}

fn reject_unknown_attributes(
    block: &Block,
    allowed: &[&str],
    label: &str,
) -> PluginManagerResult<()> {
    let mut unknown = block
        .attributes
        .keys()
        .filter(|name| !allowed.contains(&name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    unknown.sort();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(config_error(format!(
            "`{label}` contains unsupported attribute(s): {}",
            unknown.join(", ")
        )))
    }
}

fn compose_config(
    config: PluginRuntimeHostConfig,
    paths: &ComponentPaths,
) -> PluginManagerResult<PluginRuntimeHost> {
    match config {
        PluginRuntimeHostConfig::Disabled => Ok(PluginRuntimeHost::default()),
        PluginRuntimeHostConfig::Box(config) => compose_box(config, paths),
    }
}

#[cfg(target_os = "linux")]
fn compose_box(
    config: BoxRuntimeConfig,
    paths: &ComponentPaths,
) -> PluginManagerResult<PluginRuntimeHost> {
    use std::sync::Arc;

    use a3s_box_runtime::{BoxRuntimeDriver, BoxRuntimeDriverConfig, ExecutionIsolation};
    use a3s_runtime::{
        FileRuntimeStateStore, ManagedRuntimeClient, ProviderId, RuntimeClient,
        RuntimeClientRegistry, RuntimeDriver, RuntimeProviderFactory, RuntimeResult,
        RuntimeStateStore,
    };
    use async_trait::async_trait;

    struct SharedRuntimeProviderFactory {
        provider_id: ProviderId,
        client: Arc<dyn RuntimeClient>,
    }

    #[async_trait]
    impl RuntimeProviderFactory for SharedRuntimeProviderFactory {
        fn provider_id(&self) -> &ProviderId {
            &self.provider_id
        }

        async fn create(&self) -> RuntimeResult<Arc<dyn RuntimeClient>> {
            Ok(self.client.clone())
        }
    }

    let driver_config = BoxRuntimeDriverConfig {
        control_timeout: Duration::from_millis(config.control_timeout_ms),
        task_poll_interval: Duration::from_millis(config.task_poll_interval_ms),
        ..BoxRuntimeDriverConfig::default()
    };
    let isolation = match config.isolation {
        BoxRuntimeIsolation::Microvm => ExecutionIsolation::Microvm,
        BoxRuntimeIsolation::Sandbox => ExecutionIsolation::Sandbox,
    };
    let driver =
        BoxRuntimeDriver::new_with_isolation(driver_config, isolation).map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "could not construct the configured A3S Box Runtime provider: {error}"
            ))
        })?;
    let provider_id = driver.provider_id().clone();
    let state: Arc<dyn RuntimeStateStore> = Arc::new(FileRuntimeStateStore::new(
        paths
            .state_root
            .join("use/runtime/providers")
            .join(provider_id.as_str()),
    ));
    let driver: Arc<dyn RuntimeDriver> = Arc::new(driver);
    let client: Arc<dyn RuntimeClient> = Arc::new(ManagedRuntimeClient::new(state, driver));
    let mut registry = RuntimeClientRegistry::new();
    registry
        .register(Arc::new(SharedRuntimeProviderFactory {
            provider_id: provider_id.clone(),
            client,
        }))
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "could not register the configured A3S Box Runtime provider: {error}"
            ))
        })?;
    PluginRuntimeHost::new_task_provider(
        registry,
        provider_id,
        Arc::new(crate::components::UnavailableRuntimeServiceHost),
    )
    .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))
}

#[cfg(not(target_os = "linux"))]
fn compose_box(
    _config: BoxRuntimeConfig,
    _paths: &ComponentPaths,
) -> PluginManagerResult<PluginRuntimeHost> {
    Err(config_error(
        "the A3S Box plugin Runtime provider is currently supported only on Linux",
    ))
}

fn config_error(message: impl Into<String>) -> PluginManagerError {
    PluginManagerError::InvalidRequest(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX_CONFIG: &str = r#"
plugin_runtime {
  schema = "a3s.plugin-runtime-host.v1"

  box {
    isolation = "microvm"
    control_timeout_ms = 120000
    task_poll_interval_ms = 25
  }
}
"#;

    #[test]
    fn absent_runtime_block_preserves_fail_closed_default() {
        assert_eq!(
            PluginRuntimeHostConfig::from_acl("providers \"example\" {}").unwrap(),
            PluginRuntimeHostConfig::Disabled
        );
    }

    #[test]
    fn parses_one_typed_box_provider() {
        assert_eq!(
            PluginRuntimeHostConfig::from_acl(BOX_CONFIG).unwrap(),
            PluginRuntimeHostConfig::Box(BoxRuntimeConfig {
                isolation: BoxRuntimeIsolation::Microvm,
                control_timeout_ms: 120_000,
                task_poll_interval_ms: 25,
            })
        );
    }

    #[test]
    fn rejects_implicit_or_ambiguous_provider_configuration() {
        for source in [
            r#"plugin_runtime { schema = "a3s.plugin-runtime-host.v1" }"#,
            r#"plugin_runtime { schema = "a3s.plugin-runtime-host.v1" provider = "a3s-box" box { isolation = "microvm" } }"#,
            r#"plugin_runtime { schema = "a3s.plugin-runtime-host.v1" box { isolation = "auto" } }"#,
            r#"plugin_runtime { schema = "a3s.plugin-runtime-host.v1" box { isolation = "microvm" } box { isolation = "sandbox" } }"#,
            r#"plugin_runtime { schema = "a3s.plugin-runtime-host.v1" box { isolation = "sandbox" task_poll_interval_ms = 100 control_timeout_ms = 100 } }"#,
        ] {
            assert!(
                PluginRuntimeHostConfig::from_acl(source).is_err(),
                "{source}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn configured_box_provider_is_registered_without_eager_runtime_mutation() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = ComponentPaths::for_test(temporary.path());
        let host = compose_config(
            PluginRuntimeHostConfig::from_acl(BOX_CONFIG).unwrap(),
            &paths,
        )
        .unwrap();

        assert!(host.has_provider("a3s-box"));
        assert!(!paths.state_root.exists());
        assert!(!paths.data_root.exists());
    }
}
