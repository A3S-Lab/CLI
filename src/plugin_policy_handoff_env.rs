//! Process-local environment contract for forwarding a validated plugin policy.

/// Digest key for an internal A3S-to-A3S policy handoff.
pub const PLUGIN_POLICY_HANDOFF_DIGEST_ENV: &str = "A3S_INTERNAL_PLUGIN_POLICY_HANDOFF_DIGEST";
/// ACL source key paired with [`PLUGIN_POLICY_HANDOFF_DIGEST_ENV`].
pub const PLUGIN_POLICY_HANDOFF_SOURCE_ENV: &str = "A3S_INTERNAL_PLUGIN_POLICY_HANDOFF_SOURCE";
