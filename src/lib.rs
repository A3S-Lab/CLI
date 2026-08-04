//! Reusable infrastructure for the umbrella A3S CLI.

pub mod components;
pub mod plugin_manager;
pub mod plugin_manager_mcp;
pub mod registry;
pub mod research;

// Compile the process adapter in the library test target as well, so its
// hermetic fake-process contract tests do not depend on the TUI test graph.
#[cfg(test)]
#[path = "use_registry.rs"]
mod use_registry;

// Keep the signed Registry fixture in one library-test module. Individual
// unit-test modules import it from the crate root so Clippy and the compiler do
// not build independent copies of the same support implementation.
#[cfg(test)]
#[path = "../tests/support/tuf_test_support.rs"]
mod tuf_test_support;
