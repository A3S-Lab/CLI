pub(crate) mod compactor;
pub(crate) mod projection;

pub(crate) use compactor::{compact_history, MANUAL_COMPACT_TIMEOUT};
pub(crate) use projection::is_compact_message;
#[cfg(test)]
pub(crate) use projection::A3S_COMPACT_ROLE;
