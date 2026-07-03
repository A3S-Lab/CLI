//! Per-panel `impl App` method groups, split out of the god-file.
//!
//! Each submodule adds an additional `impl App { … }` block. A descendant
//! module can see the ancestor `App`'s private fields, so these blocks compile
//! exactly as if they were still inline in `tui/mod.rs`.

mod banner;
mod btw;
mod chat;
pub(crate) mod ctx;
mod effort;
pub(crate) mod evolve;
mod files;
pub(crate) mod flow;
mod git;
mod help;
mod ide;
pub(crate) mod login;
mod memory;
mod menu;
mod model;
mod plan;
mod plugins;
mod relay;
pub(crate) mod repos;
pub(crate) mod review;
pub(crate) mod sleep;
pub(crate) mod spf;
mod theme;
mod top;
