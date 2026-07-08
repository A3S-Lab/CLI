mod assets;
mod static_files;

pub(super) use assets::find_default_web_dir;
pub(super) use static_files::{api_only_fallback, serve_static};
