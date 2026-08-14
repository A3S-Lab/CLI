use std::path::Path;

use a3s_acl::{Block, Value};
use a3s_code_core::{
    ChunkingConfig, FixedWindowChunkingOptions, RecursiveChunkingOptions, WorkspaceChunkingStrategy,
};
use anyhow::{anyhow, bail, Context};

use super::config::usize_value;

pub(super) const CHUNKING_BLOCK: &str = "chunking";
const LINE_BLOCK: &str = "line";
const FIXED_WINDOW_BLOCK: &str = "fixed_window";
const RECURSIVE_BLOCK: &str = "recursive";

/// Trusted-host selection for Core-owned text chunking strategies.
///
/// The ACL uses mutually exclusive typed child blocks. Omission preserves the
/// Core-compatible line strategy; no primitive strategy selector is accepted.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum WorkspaceChunkingStrategyConfig {
    #[default]
    Lines,
    FixedWindow {
        target_bytes: usize,
        overlap_bytes: usize,
    },
    Recursive {
        target_bytes: usize,
        overlap_bytes: usize,
        separators: Option<Vec<String>>,
    },
}

impl WorkspaceChunkingStrategyConfig {
    pub(super) fn apply_block(&mut self, block: &Block, source: &Path) -> anyhow::Result<()> {
        if !block.labels.is_empty() || !block.attributes.is_empty() {
            bail!(
                "workspace_retrieval {CHUNKING_BLOCK} in A3S ACL {} must be an unlabeled typed block without attributes",
                source.display()
            );
        }
        if block.blocks.len() != 1 {
            bail!(
                "workspace_retrieval {CHUNKING_BLOCK} in A3S ACL {} must contain exactly one line, fixed_window, or recursive block",
                source.display()
            );
        }
        let strategy = &block.blocks[0];
        validate_strategy_block_shape(strategy, source)?;
        let parsed = match strategy.name.as_str() {
            LINE_BLOCK => {
                reject_unknown_fields(strategy, &[], source)?;
                Self::Lines
            }
            FIXED_WINDOW_BLOCK => {
                reject_unknown_fields(strategy, &["target_bytes", "overlap_bytes"], source)?;
                Self::FixedWindow {
                    target_bytes: required_usize(strategy, "target_bytes", source)?,
                    overlap_bytes: optional_usize(strategy, "overlap_bytes", source)?.unwrap_or(0),
                }
            }
            RECURSIVE_BLOCK => {
                reject_unknown_fields(
                    strategy,
                    &["target_bytes", "overlap_bytes", "separators"],
                    source,
                )?;
                Self::Recursive {
                    target_bytes: required_usize(strategy, "target_bytes", source)?,
                    overlap_bytes: optional_usize(strategy, "overlap_bytes", source)?.unwrap_or(0),
                    separators: strategy
                        .attributes
                        .get("separators")
                        .map(|value| string_list_value(value, "separators", source))
                        .transpose()?,
                }
            }
            name => bail!(
                "unsupported workspace_retrieval {CHUNKING_BLOCK} strategy block `{name}` in A3S ACL {}; expected line, fixed_window, or recursive",
                source.display()
            ),
        };
        *self = parsed;
        Ok(())
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        self.core_strategy().map(|_| ())
    }

    pub(crate) fn core_strategy(&self) -> anyhow::Result<WorkspaceChunkingStrategy> {
        let strategy = match self {
            Self::Lines => WorkspaceChunkingStrategy::Lines,
            Self::FixedWindow {
                target_bytes,
                overlap_bytes,
            } => WorkspaceChunkingStrategy::FixedWindow(
                FixedWindowChunkingOptions::new(*target_bytes, *overlap_bytes)
                    .map_err(chunking_error)?,
            ),
            Self::Recursive {
                target_bytes,
                overlap_bytes,
                separators,
            } => {
                let mut options = RecursiveChunkingOptions::new(*target_bytes, *overlap_bytes)
                    .map_err(chunking_error)?;
                if let Some(separators) = separators {
                    options = options
                        .with_separators(separators.clone())
                        .map_err(chunking_error)?;
                }
                WorkspaceChunkingStrategy::Recursive(options)
            }
        };
        strategy
            .validate_for(ChunkingConfig::default())
            .map_err(chunking_error)?;
        Ok(strategy)
    }

    pub(crate) fn strategy_name(&self) -> &'static str {
        match self {
            Self::Lines => LINE_BLOCK,
            Self::FixedWindow { .. } => FIXED_WINDOW_BLOCK,
            Self::Recursive { .. } => RECURSIVE_BLOCK,
        }
    }

    pub(crate) fn target_bytes(&self) -> Option<usize> {
        match self {
            Self::Lines => None,
            Self::FixedWindow { target_bytes, .. } | Self::Recursive { target_bytes, .. } => {
                Some(*target_bytes)
            }
        }
    }

    pub(crate) fn overlap_bytes(&self) -> Option<usize> {
        match self {
            Self::Lines => None,
            Self::FixedWindow { overlap_bytes, .. } | Self::Recursive { overlap_bytes, .. } => {
                Some(*overlap_bytes)
            }
        }
    }

    pub(crate) fn separators(&self) -> Option<&[String]> {
        match self {
            Self::Recursive {
                separators: Some(separators),
                ..
            } => Some(separators),
            _ => None,
        }
    }

    pub(crate) fn uses_default_separators(&self) -> bool {
        matches!(
            self,
            Self::Recursive {
                separators: None,
                ..
            }
        )
    }
}

fn validate_strategy_block_shape(block: &Block, source: &Path) -> anyhow::Result<()> {
    if !block.labels.is_empty() || !block.blocks.is_empty() {
        bail!(
            "workspace_retrieval {CHUNKING_BLOCK} strategy `{}` in A3S ACL {} must be an unlabeled flat block",
            block.name,
            source.display()
        );
    }
    Ok(())
}

fn reject_unknown_fields(
    block: &Block,
    known_fields: &[&str],
    source: &Path,
) -> anyhow::Result<()> {
    for field in block.attributes.keys() {
        if !known_fields.contains(&field.as_str()) {
            bail!(
                "unknown workspace_retrieval {CHUNKING_BLOCK} {} field `{field}` in A3S ACL {}",
                block.name,
                source.display()
            );
        }
    }
    Ok(())
}

fn required_usize(block: &Block, field: &str, source: &Path) -> anyhow::Result<usize> {
    let value = block.attributes.get(field).with_context(|| {
        format!(
            "workspace_retrieval {CHUNKING_BLOCK} {} in A3S ACL {} requires `{field}`",
            block.name,
            source.display()
        )
    })?;
    usize_value(
        value,
        &format!("{CHUNKING_BLOCK}.{}.{field}", block.name),
        source,
    )
}

fn optional_usize(block: &Block, field: &str, source: &Path) -> anyhow::Result<Option<usize>> {
    block
        .attributes
        .get(field)
        .map(|value| {
            usize_value(
                value,
                &format!("{CHUNKING_BLOCK}.{}.{field}", block.name),
                source,
            )
        })
        .transpose()
}

fn string_list_value(value: &Value, field: &str, source: &Path) -> anyhow::Result<Vec<String>> {
    let Value::List(items) = value else {
        bail!(
            "workspace_retrieval {CHUNKING_BLOCK}.{RECURSIVE_BLOCK}.{field} in A3S ACL {} must be a list of strings",
            source.display()
        );
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str().map(ToOwned::to_owned).with_context(|| {
                format!(
                    "workspace_retrieval {CHUNKING_BLOCK}.{RECURSIVE_BLOCK}.{field}[{index}] in A3S ACL {} must be a string",
                    source.display()
                )
            })
        })
        .collect()
}

fn chunking_error(error: a3s_code_core::WorkspaceChunkingError) -> anyhow::Error {
    anyhow!("invalid workspace_retrieval chunking: {error}")
}
