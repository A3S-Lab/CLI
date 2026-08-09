#![cfg(all(windows, target_arch = "x86_64"))]

mod support;

#[path = "support/tuf_test_support.rs"]
mod tuf_test_support;

#[path = "web_plugin_marketplace/generic_real_e2e.rs"]
mod generic_real_e2e;
