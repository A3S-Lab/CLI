//! Per-panel `impl App` method groups, split out of the god-file.
//!
//! Each submodule adds an additional `impl App { … }` block. A descendant
//! module can see the ancestor `App`'s private fields, so these blocks compile
//! exactly as if they were still inline in `tui/mod.rs`.

mod git;
mod ide;
mod relay;
mod top;
