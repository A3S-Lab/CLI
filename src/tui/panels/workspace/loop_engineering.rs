//! `/loop` engineered loops: persisted loop specs, state, run logs, audit, and
//! an OS-aware run directive.

use super::super::*;
use super::agent::{self, AgentDevSession};
use a3s_tui::components::{DetailPanel, DetailRow, KeyValue, SectionHeader};
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(crate) use a3s_deep_research::planner::deep_research_loop_contract;

include!("loop_engineering/core.rs");
include!("loop_engineering/runtime.rs");
include!("loop_engineering/tests.rs");
