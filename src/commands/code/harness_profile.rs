//! Host session profile for `a3s code harness`.
//!
//! Without a profile the Harness opens sessions with `SessionOptions::new()`
//! and the Agent from the active CLI configuration.  That is the right default
//! for an interactive install, but a *host* that drives the Harness over the
//! Agent protocol (a workflow engine dispatching pinned Agent artifacts) needs
//! to state, and have the Harness enforce, what a session may look like:
//!
//! * which Agent directory (`instructions.md`, `agent.acl`, `tools/`) is
//!   loaded, optionally pinned by a content digest;
//! * that the MCP servers declared in the Agent directory's `tools/` are
//!   connected before any run starts, not merely attempted;
//! * how tools are presented to the model and which tools it may call;
//! * whether HITL confirmation is required;
//! * how large a Tool result may grow before it is folded, so a single large
//!   result cannot exceed the Agent protocol's per-event payload bound;
//! * how many run events the session retains and where sessions persist.
//!
//! All of these are existing `a3s-code-core` session capabilities; the profile
//! is only a declarative, checked way to hand them to the Harness.  Every
//! violation is refused before the release port is bound, with a structured
//! error code, so a host can never observe a Harness that is "ready" but
//! configured differently from what it asked for.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_code_core::config::{AgentDir, CodeConfig, ToolSpec};
use a3s_code_core::hitl::ConfirmationPolicy;
use a3s_code_core::permissions::{PermissionDecision, PermissionPolicy};
use a3s_code_core::retention::SessionRetentionLimits;
use a3s_code_core::tools::{ToolPresentationProfileV1, ToolResultTransformPolicyV1};
use a3s_code_core::{Agent, SessionOptions};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cli::output::{coded_error, ExitClass};

pub(crate) const HARNESS_SESSION_PROFILE_SCHEMA_V1: &str = "a3s.code.harness-session-profile.v1";
/// Upper bound on a profile document; profiles are small declarative files.
const MAX_PROFILE_BYTES: u64 = 64 * 1024;
/// Upper bound on the Agent directory a digest is computed over.
const MAX_DIGESTED_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) const CODE_INVALID: &str = "a3s.code.harness_profile.invalid";
pub(crate) const CODE_AGENT_DIR: &str = "a3s.code.harness_profile.agent_dir";
pub(crate) const CODE_DIGEST_MISMATCH: &str = "a3s.code.harness_profile.agent_dir_digest_mismatch";
pub(crate) const CODE_TOOLS_UNAVAILABLE: &str = "a3s.code.harness_profile.tools_unavailable";
pub(crate) const CODE_PRESENTATION: &str = "a3s.code.harness_profile.presentation_violation";

/// Declarative session contract handed to the Harness with `--session-profile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HarnessSessionProfile {
    pub schema: String,
    /// Agent directory to load instead of the active CLI configuration.
    /// Relative paths resolve against the profile file's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_dir: Option<PathBuf>,
    /// `sha256:<hex>` over the Agent directory (see [`agent_dir_digest`]);
    /// a mismatch refuses to start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_dir_digest: Option<String>,
    /// Connect the MCP servers declared in the Agent directory's `tools/` and
    /// require each to contribute tools.  Defaults to `true` when `agent_dir`
    /// is set.
    #[serde(default = "default_true")]
    pub install_agent_dir_tools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_presentation: Option<ToolPresentation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<PermissionProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<ConfirmationProfile>,
    /// Code's own Tool-result transform policy (`a3s.code.tool-result-transform-policy.v1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result_transform: Option<ToolResultTransformPolicyV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<RetentionProfile>,
    /// Directory for Code's file session store.  Relative paths resolve
    /// against the profile file's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_store_dir: Option<PathBuf>,
    /// Assertions checked on a probe session before the port is bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_tools: Option<ExpectedTools>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolPresentation {
    /// Code's default: tools selected by prompt relevance.
    Adaptive,
    /// Every permitted tool is presented on every call.
    Direct,
    /// No tool definitions reach the model.
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionProfile {
    /// Allow rules, in Code's permission rule syntax (`mcp__orchestrator__*`, `read`).
    #[serde(default)]
    pub allow: Vec<String>,
    /// Deny rules (a full deny also hides the tool from the model).
    #[serde(default)]
    pub deny: Vec<String>,
    /// Decision for tools no rule matches.
    #[serde(default)]
    pub default_decision: DefaultDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DefaultDecision {
    Allow,
    #[default]
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConfirmationProfile {
    /// Code's default confirmation manager.
    Default,
    /// Tools the permission policy allows execute without a human confirmation
    /// (HITL disabled); a headless host has nobody to confirm.
    AutoApproveAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetentionProfile {
    Default,
    /// Keep every run event in memory so a host paging a long run never sees a
    /// retention gap.
    Unbounded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectedTools {
    /// Each prefix must match at least one tool presented to the model.
    #[serde(default)]
    pub present_prefixes: Vec<String>,
    /// Every presented tool must match one of these prefixes.
    #[serde(default)]
    pub only_prefixes: Vec<String>,
}

/// A profile together with the directory its relative paths resolve against.
#[derive(Debug, Clone)]
pub(crate) struct LoadedProfile {
    pub profile: HarnessSessionProfile,
    pub base_dir: PathBuf,
}

/// Everything the Harness needs from a profile: the Agent, the session options
/// to open every session with, and the MCP prefixes the probe must see.
pub(crate) struct MaterializedProfile {
    pub agent: Arc<Agent>,
    pub session_options: SessionOptions,
    pub mcp_prefixes: Vec<String>,
}

impl HarnessSessionProfile {
    pub(crate) fn load(path: &Path) -> anyhow::Result<LoadedProfile> {
        let metadata = std::fs::metadata(path)
            .map_err(|error| invalid(format!("cannot read {}: {error}", path.display())))?;
        if metadata.len() > MAX_PROFILE_BYTES {
            return Err(invalid(format!(
                "{} exceeds {MAX_PROFILE_BYTES} bytes",
                path.display()
            )));
        }
        let bytes = std::fs::read(path)
            .map_err(|error| invalid(format!("cannot read {}: {error}", path.display())))?;
        let profile: Self = serde_json::from_slice(&bytes).map_err(|error| {
            invalid(format!(
                "{} is not a valid profile: {error}",
                path.display()
            ))
        })?;
        profile.validate()?;
        let base_dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(LoadedProfile { profile, base_dir })
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        if self.schema != HARNESS_SESSION_PROFILE_SCHEMA_V1 {
            return Err(invalid(format!(
                "schema must be {HARNESS_SESSION_PROFILE_SCHEMA_V1}, found {:?}",
                self.schema
            )));
        }
        if let Some(digest) = &self.agent_dir_digest {
            if self.agent_dir.is_none() {
                return Err(invalid("agent_dir_digest requires agent_dir"));
            }
            if !is_sha256_reference(digest) {
                return Err(invalid(
                    "agent_dir_digest must be sha256:<64 lowercase hex>",
                ));
            }
        }
        if let Some(permissions) = &self.permissions {
            for rule in permissions.allow.iter().chain(&permissions.deny) {
                if rule.trim().is_empty() {
                    return Err(invalid("permission rules must not be empty"));
                }
            }
        }
        if let Some(policy) = &self.tool_result_transform {
            policy
                .validate()
                .map_err(|error| invalid(format!("tool_result_transform: {error}")))?;
        }
        if let Some(expected) = &self.expected_tools {
            for prefix in expected
                .present_prefixes
                .iter()
                .chain(&expected.only_prefixes)
            {
                if prefix.is_empty() || prefix.contains('*') {
                    return Err(invalid(
                        "expected_tools prefixes must be non-empty literals",
                    ));
                }
            }
            if self.tool_presentation == Some(ToolPresentation::Disabled)
                && !expected.present_prefixes.is_empty()
            {
                return Err(invalid(
                    "expected_tools.present_prefixes cannot be satisfied with tool_presentation = disabled",
                ));
            }
        }
        Ok(())
    }

    /// Apply the profile's session-level choices on top of `base`.
    pub(crate) fn apply(&self, mut options: SessionOptions) -> SessionOptions {
        if let Some(presentation) = self.tool_presentation {
            options = options.with_tool_presentation_profile(match presentation {
                ToolPresentation::Adaptive => ToolPresentationProfileV1::adaptive(),
                ToolPresentation::Direct => ToolPresentationProfileV1::direct(),
                ToolPresentation::Disabled => ToolPresentationProfileV1::disabled(),
            });
        }
        if let Some(permissions) = &self.permissions {
            options = options.with_permission_policy(permissions.policy());
        }
        if self.confirmation == Some(ConfirmationProfile::AutoApproveAllowed) {
            options = options.with_confirmation_policy(ConfirmationPolicy::default());
        }
        if let Some(policy) = &self.tool_result_transform {
            options = options.with_tool_result_transform_policy(policy.clone());
        }
        if self.retention == Some(RetentionProfile::Unbounded) {
            options = options.with_retention_limits(SessionRetentionLimits::unbounded());
        }
        options
    }

    /// Build the Agent and session options this profile describes.
    ///
    /// `base` carries whatever the caller already decided (tests inject a
    /// deterministic model client here); the profile only adds to it.
    /// `fallback_config` supplies the Agent configuration when the profile
    /// names no `agent_dir`.
    pub(crate) async fn materialize(
        loaded: &LoadedProfile,
        base: SessionOptions,
        fallback_config: impl FnOnce() -> anyhow::Result<CodeConfig>,
    ) -> anyhow::Result<MaterializedProfile> {
        let profile = &loaded.profile;
        let mut options = profile.apply(base);
        if let Some(dir) = &profile.session_store_dir {
            options = options.with_file_session_store(resolve(&loaded.base_dir, dir));
        }
        let (config, mcp_prefixes) = match &profile.agent_dir {
            Some(dir) => {
                let dir = resolve(&loaded.base_dir, dir);
                if let Some(expected) = &profile.agent_dir_digest {
                    let actual = agent_dir_digest(&dir)?;
                    if &actual != expected {
                        return Err(coded_error(
                            CODE_DIGEST_MISMATCH,
                            format!(
                                "agent directory {} has digest {actual}, profile pins {expected}",
                                dir.display()
                            ),
                            ExitClass::Failure,
                        ));
                    }
                }
                let agent_dir = AgentDir::load(&dir).map_err(|error| {
                    coded_error(
                        CODE_AGENT_DIR,
                        format!("cannot load agent directory {}: {error}", dir.display()),
                        ExitClass::Failure,
                    )
                })?;
                options = options.with_prompt_slots(agent_dir.prompt_slots.clone());
                let mut config = agent_dir.config.clone();
                let mut prefixes = Vec::new();
                if profile.install_agent_dir_tools {
                    for spec in &agent_dir.tools {
                        if let ToolSpec::Mcp(server) = spec {
                            if !server.enabled {
                                continue;
                            }
                            prefixes.push(format!("mcp__{}__", server.name));
                            if !config
                                .mcp_servers
                                .iter()
                                .any(|existing| existing.name == server.name)
                            {
                                config.mcp_servers.push(server.clone());
                            }
                        }
                    }
                }
                (config, prefixes)
            }
            None => (fallback_config()?, Vec::new()),
        };
        let agent = Arc::new(
            Agent::from_config(config)
                .await
                .context("could not initialize the Agent Harness runtime")?,
        );
        Ok(MaterializedProfile {
            agent,
            session_options: options,
            mcp_prefixes,
        })
    }

    /// Open one probe session and assert that the model-facing tool surface is
    /// exactly what the profile declared.  `Agent::from_config` only warns when
    /// an MCP server fails to connect; a host that declared the server must be
    /// refused instead of silently running without it.
    pub(crate) async fn verify_probe(
        &self,
        materialized: &MaterializedProfile,
        workspace: &Path,
    ) -> anyhow::Result<()> {
        let nothing_to_check = materialized.mcp_prefixes.is_empty()
            && self.expected_tools.is_none()
            && self.tool_presentation != Some(ToolPresentation::Disabled);
        if nothing_to_check {
            return Ok(());
        }
        let probe = materialized
            .agent
            .session_async(
                workspace.to_string_lossy().into_owned(),
                Some(materialized.session_options.clone()),
            )
            .await
            .context("open probe session for harness profile verification")?;
        let names = probe.tool_names();
        for prefix in &materialized.mcp_prefixes {
            if !names.iter().any(|name| name.starts_with(prefix)) {
                return Err(coded_error(
                    CODE_TOOLS_UNAVAILABLE,
                    format!(
                        "MCP server declared in the agent directory contributed no tools (expected prefix {prefix})"
                    ),
                    ExitClass::Failure,
                ));
            }
        }
        let presented: Vec<String> = probe
            .presented_tool_definitions("{}")
            .map_err(|error| {
                coded_error(
                    CODE_PRESENTATION,
                    format!("tool presentation failed: {error}"),
                    ExitClass::Failure,
                )
            })?
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        if self.tool_presentation == Some(ToolPresentation::Disabled) {
            if let Some(stray) = presented.first() {
                return Err(coded_error(
                    CODE_PRESENTATION,
                    format!(
                        "tool_presentation is disabled but {stray} is still presented to the model"
                    ),
                    ExitClass::Failure,
                ));
            }
        }
        if let Some(expected) = &self.expected_tools {
            for prefix in &expected.present_prefixes {
                if !presented.iter().any(|name| name.starts_with(prefix)) {
                    return Err(coded_error(
                        CODE_PRESENTATION,
                        format!("no tool with prefix {prefix} is presented to the model"),
                        ExitClass::Failure,
                    ));
                }
            }
            if !expected.only_prefixes.is_empty() {
                if let Some(stray) = presented
                    .iter()
                    .find(|name| !expected.only_prefixes.iter().any(|p| name.starts_with(p)))
                {
                    return Err(coded_error(
                        CODE_PRESENTATION,
                        format!(
                            "tool {stray} is presented to the model but matches no allowed prefix"
                        ),
                        ExitClass::Failure,
                    ));
                }
            }
        }
        Ok(())
    }
}

impl PermissionProfile {
    pub(crate) fn policy(&self) -> PermissionPolicy {
        let mut policy = PermissionPolicy::new();
        for rule in &self.allow {
            policy = policy.allow(rule);
        }
        let denied: Vec<&str> = self.deny.iter().map(String::as_str).collect();
        policy = policy.deny_all(&denied);
        policy.default_decision = match self.default_decision {
            DefaultDecision::Allow => PermissionDecision::Allow,
            DefaultDecision::Ask => PermissionDecision::Ask,
            DefaultDecision::Deny => PermissionDecision::Deny,
        };
        policy
    }
}

fn invalid(message: impl Into<String>) -> anyhow::Error {
    coded_error(CODE_INVALID, message, ExitClass::Failure)
}

fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn is_sha256_reference(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

/// Content digest of an Agent directory: `sha256:` over every regular file,
/// sorted by its `/`-joined relative path, each contributed as
/// `<path>\0<len>\0<bytes>`.  Symlinks are refused (they could point outside
/// the directory), and directories are represented only through their files.
pub(crate) fn agent_dir_digest(dir: &Path) -> anyhow::Result<String> {
    let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut total: u64 = 0;
    collect_files(dir, dir, &mut files, &mut total)?;
    let mut hasher = Sha256::new();
    for (relative, path) in &files {
        let bytes = std::fs::read(path).map_err(|error| {
            coded_error(
                CODE_AGENT_DIR,
                format!("cannot read {}: {error}", path.display()),
                ExitClass::Failure,
            )
        })?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update([0]);
        hasher.update(&bytes);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn collect_files(
    root: &Path,
    dir: &Path,
    files: &mut BTreeMap<String, PathBuf>,
    total: &mut u64,
) -> anyhow::Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|error| {
        coded_error(
            CODE_AGENT_DIR,
            format!("cannot read {}: {error}", dir.display()),
            ExitClass::Failure,
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            coded_error(
                CODE_AGENT_DIR,
                format!("cannot read {}: {error}", dir.display()),
                ExitClass::Failure,
            )
        })?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            coded_error(
                CODE_AGENT_DIR,
                format!("cannot stat {}: {error}", path.display()),
                ExitClass::Failure,
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(coded_error(
                CODE_AGENT_DIR,
                format!("agent directory contains a symlink: {}", path.display()),
                ExitClass::Failure,
            ));
        }
        if metadata.is_dir() {
            collect_files(root, &path, files, total)?;
            continue;
        }
        *total += metadata.len();
        if *total > MAX_DIGESTED_BYTES {
            return Err(coded_error(
                CODE_AGENT_DIR,
                format!("agent directory exceeds {MAX_DIGESTED_BYTES} bytes"),
                ExitClass::Failure,
            ));
        }
        let relative = path
            .strip_prefix(root)
            .expect("entry is below root")
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        files.insert(relative, path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal() -> HarnessSessionProfile {
        serde_json::from_value(json!({"schema": HARNESS_SESSION_PROFILE_SCHEMA_V1})).unwrap()
    }

    #[test]
    fn schema_and_unknown_fields_are_refused() {
        let wrong: Result<HarnessSessionProfile, _> =
            serde_json::from_value(json!({"schema": "a3s.code.harness-session-profile.v0"}));
        assert!(wrong.unwrap().validate().is_err());
        let unknown: Result<HarnessSessionProfile, _> = serde_json::from_value(
            json!({"schema": HARNESS_SESSION_PROFILE_SCHEMA_V1, "surprise": true}),
        );
        assert!(
            unknown.is_err(),
            "unknown fields must not be silently ignored"
        );
        assert!(minimal().validate().is_ok());
    }

    #[test]
    fn digest_requires_agent_dir_and_sha256_shape() {
        let mut profile = minimal();
        profile.agent_dir_digest = Some(format!("sha256:{}", "a".repeat(64)));
        assert!(profile.validate().is_err(), "digest without agent_dir");
        profile.agent_dir = Some(PathBuf::from("agent"));
        assert!(profile.validate().is_ok());
        profile.agent_dir_digest = Some("sha256:ABC".into());
        assert!(profile.validate().is_err(), "malformed digest");
    }

    #[test]
    fn expected_tools_cannot_contradict_disabled_presentation() {
        let mut profile = minimal();
        profile.tool_presentation = Some(ToolPresentation::Disabled);
        profile.expected_tools = Some(ExpectedTools {
            present_prefixes: vec!["read".into()],
            only_prefixes: vec![],
        });
        assert!(profile.validate().is_err());
        profile.expected_tools = Some(ExpectedTools {
            present_prefixes: vec![],
            only_prefixes: vec!["mcp__x__".into()],
        });
        assert!(profile.validate().is_ok());
        profile.expected_tools = Some(ExpectedTools {
            present_prefixes: vec![],
            only_prefixes: vec!["mcp__*".into()],
        });
        assert!(profile.validate().is_err(), "globs are not prefixes");
    }

    #[test]
    fn tool_result_transform_is_validated_by_code() {
        let mut profile = minimal();
        profile.tool_result_transform = serde_json::from_value(json!({
            "schema": "a3s.code.tool-result-transform-policy.v1",
            "max_output_bytes": 32768, "head_bytes": 24576, "tail_bytes": 7168,
            "fold_repeated_lines": true, "repeated_line_threshold": 3, "structured_sample_items": 32
        }))
        .unwrap();
        assert!(profile.validate().is_ok());
        profile
            .tool_result_transform
            .as_mut()
            .unwrap()
            .max_output_bytes = 16;
        assert!(profile.validate().is_err(), "below Code's minimum");
    }

    #[test]
    fn permission_profile_maps_onto_codes_policy() {
        let permissions = PermissionProfile {
            allow: vec!["mcp__orchestrator__*".into()],
            deny: vec!["bash".into(), "write".into()],
            default_decision: DefaultDecision::Deny,
        };
        let policy = permissions.policy();
        let no_args = json!({});
        assert_eq!(policy.default_decision, PermissionDecision::Deny);
        assert_eq!(
            policy.check("mcp__orchestrator__shortlist", &no_args),
            PermissionDecision::Allow
        );
        assert_eq!(policy.check("bash", &no_args), PermissionDecision::Deny);
        assert_eq!(policy.check("read", &no_args), PermissionDecision::Deny);
        let ask = PermissionProfile::default().policy();
        assert_eq!(ask.default_decision, PermissionDecision::Ask);
        assert_eq!(ask.check("read", &no_args), PermissionDecision::Ask);
    }

    #[test]
    fn apply_leaves_a_profile_free_base_untouched_and_maps_every_choice() {
        let applied = minimal().apply(SessionOptions::new());
        assert!(applied.tool_presentation_profile.is_none());
        assert!(applied.permission_policy.is_none());
        assert!(applied.confirmation_policy.is_none());
        assert!(applied.tool_result_transform_policy.is_none());
        assert!(applied.retention_limits.is_none());

        let mut profile = minimal();
        profile.tool_presentation = Some(ToolPresentation::Direct);
        profile.permissions = Some(PermissionProfile {
            allow: vec!["read".into()],
            deny: vec![],
            default_decision: DefaultDecision::Deny,
        });
        profile.confirmation = Some(ConfirmationProfile::AutoApproveAllowed);
        profile.retention = Some(RetentionProfile::Unbounded);
        profile.tool_result_transform = Some(ToolResultTransformPolicyV1::context_efficient());
        let applied = profile.apply(SessionOptions::new());
        assert_eq!(
            applied.tool_presentation_profile,
            Some(ToolPresentationProfileV1::direct())
        );
        assert_eq!(
            applied
                .permission_policy
                .as_ref()
                .map(|p| p.default_decision),
            Some(PermissionDecision::Deny)
        );
        assert!(applied
            .confirmation_policy
            .is_some_and(|policy| !policy.enabled));
        assert_eq!(
            applied.retention_limits,
            Some(SessionRetentionLimits::unbounded())
        );
        assert_eq!(
            applied.tool_result_transform_policy,
            Some(ToolResultTransformPolicyV1::context_efficient())
        );
    }

    #[test]
    fn agent_dir_digest_is_deterministic_and_content_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(
            dir.path().join("instructions.md"),
            "You are a test agent.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("agent.acl"),
            "default_model = \"openai/test\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("tools/x.md"), "---\nkind: mcp\n---\n").unwrap();
        let first = agent_dir_digest(dir.path()).unwrap();
        let second = agent_dir_digest(dir.path()).unwrap();
        assert_eq!(first, second);
        assert!(is_sha256_reference(&first));
        std::fs::write(
            dir.path().join("instructions.md"),
            "You are a different agent.\n",
        )
        .unwrap();
        assert_ne!(
            agent_dir_digest(dir.path()).unwrap(),
            first,
            "content change must change the digest"
        );
        // Renaming a file changes the digest too: the path is part of the input.
        std::fs::rename(dir.path().join("tools/x.md"), dir.path().join("tools/y.md")).unwrap();
        let renamed = agent_dir_digest(dir.path()).unwrap();
        std::fs::rename(dir.path().join("tools/y.md"), dir.path().join("tools/x.md")).unwrap();
        assert_ne!(renamed, agent_dir_digest(dir.path()).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn agent_dir_digest_refuses_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("instructions.md"), "x").unwrap();
        std::os::unix::fs::symlink("/etc/hosts", dir.path().join("agent.acl")).unwrap();
        let error = agent_dir_digest(dir.path()).unwrap_err().to_string();
        assert!(error.contains("symlink"), "{error}");
    }

    #[test]
    fn load_resolves_relative_paths_against_the_profile_directory() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = dir.path().join("profile.json");
        std::fs::write(
            &profile_path,
            serde_json::to_vec(&json!({
                "schema": HARNESS_SESSION_PROFILE_SCHEMA_V1,
                "agent_dir": "agent",
                "session_store_dir": "sessions"
            }))
            .unwrap(),
        )
        .unwrap();
        let loaded = HarnessSessionProfile::load(&profile_path).unwrap();
        assert_eq!(loaded.base_dir, dir.path());
        assert_eq!(
            resolve(&loaded.base_dir, loaded.profile.agent_dir.as_ref().unwrap()),
            dir.path().join("agent")
        );
        std::fs::write(&profile_path, b"{not json").unwrap();
        let error = HarnessSessionProfile::load(&profile_path).unwrap_err();
        assert!(error.to_string().contains("not a valid profile"), "{error}");
    }
}
