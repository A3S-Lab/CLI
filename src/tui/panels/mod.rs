//! Per-panel `impl App` method groups, split out of the god-file.
//!
//! Each submodule adds an additional `impl App { … }` block. A descendant
//! module can see the ancestor `App`'s private fields, so these blocks compile
//! exactly as if they were still inline in `tui/mod.rs`.

pub(crate) mod agent;
mod banner;
mod btw;
mod chat;
pub(crate) mod ctx;
mod effort;
mod files;
pub(crate) mod flow;
mod git;
mod help;
mod ide;
pub(crate) mod kb;
pub(crate) mod login;
pub(crate) mod loop_engineering;
mod memory;
mod menu;
mod model;
pub(crate) mod os_resources;
mod plan;
mod plugins;
mod relay;
pub(crate) mod repos;
pub(crate) mod review;
pub(crate) mod sleep;
pub(crate) mod spf;
mod theme;
mod top;
