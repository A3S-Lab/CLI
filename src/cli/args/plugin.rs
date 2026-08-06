use clap::{Args, Subcommand, ValueEnum};

#[derive(Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct PluginArgs {
    #[command(subcommand)]
    pub command: PluginCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum PluginCommand {
    /// Search verified plugin metadata without downloading package payloads.
    Search(PluginSearchArgs),
    /// Inspect matching verified releases for one plugin package.
    Inspect(PluginInspectArgs),
    /// List plugins installed through A3S Use.
    List,
    /// Plan and install one verified plugin package.
    Install(PluginInstallArgs),
    /// Plan and upgrade one installed plugin package.
    Upgrade(PluginMutationArgs),
    /// Apply a previously reviewed immutable plugin plan.
    Apply(PluginApplyArgs),
    /// Enable one installed plugin package.
    Enable(PluginToggleArgs),
    /// Disable one installed plugin package.
    Disable(PluginToggleArgs),
    /// Plan and uninstall one plugin package while retaining its data.
    Uninstall(PluginMutationArgs),
    /// Serve the host-owned read-only Plugin Manager over standard MCP.
    #[command(hide = true)]
    McpServe,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct PluginSearchArgs {
    /// Search package identity, display name, description, publisher, and tags.
    #[arg(value_name = "QUERY")]
    pub query: String,

    /// Require one declared surface kind.
    #[arg(long, value_enum)]
    pub surface: Option<PluginSurfaceArg>,

    /// Require one release channel.
    #[arg(long, value_enum)]
    pub channel: Option<PluginChannelArg>,

    /// Require an exact publisher ID or signed publisher name.
    #[arg(long, value_name = "PUBLISHER")]
    pub publisher: Option<String>,

    /// Require an exact signed category.
    #[arg(long, value_name = "CATEGORY")]
    pub category: Option<String>,

    /// Bound the number of returned matches.
    #[arg(long, default_value_t = 50, value_parser = plugin_result_limit)]
    pub limit: usize,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct PluginInspectArgs {
    /// Canonical plugin package ID in publisher/name form.
    #[arg(value_name = "PUBLISHER/NAME")]
    pub package_id: String,

    /// Restrict the inspected release channel.
    #[arg(long, value_enum)]
    pub channel: Option<PluginChannelArg>,
}

#[derive(Clone, Debug, Args)]
#[command(disable_version_flag = true)]
pub(crate) struct PluginInstallArgs {
    /// Canonical plugin package ID in publisher/name form.
    #[arg(value_name = "PUBLISHER/NAME")]
    pub package_id: String,

    /// Install one exact canonical semantic version.
    #[arg(long, value_name = "VERSION", value_parser = plugin_version)]
    pub version: Option<String>,

    /// Select one release channel.
    #[arg(long, value_enum)]
    pub channel: Option<PluginChannelArg>,

    #[command(flatten)]
    pub review: PluginReviewArgs,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct PluginMutationArgs {
    /// Canonical plugin package ID in publisher/name form.
    #[arg(value_name = "PUBLISHER/NAME")]
    pub package_id: String,

    #[command(flatten)]
    pub review: PluginReviewArgs,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct PluginApplyArgs {
    /// Operation ID returned by a previous plugin dry-run.
    #[arg(value_name = "OPERATION_ID", value_parser = plugin_operation_id)]
    pub operation_id: String,

    /// Canonical SHA-256 digest returned with the reviewed plan.
    #[arg(long, value_name = "SHA256", value_parser = plugin_plan_digest)]
    pub plan_digest: String,

    /// Apply the exact reviewed plan without another prompt.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct PluginToggleArgs {
    /// Canonical plugin package ID in publisher/name form.
    #[arg(value_name = "PUBLISHER/NAME")]
    pub package_id: String,

    #[command(flatten)]
    pub review: PluginReviewArgs,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct PluginReviewArgs {
    /// Persist and print the immutable operation plan without mutation.
    #[arg(long, conflicts_with = "yes")]
    pub dry_run: bool,

    /// Apply the newly resolved exact plan without prompting.
    #[arg(long, conflicts_with = "dry_run")]
    pub yes: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum PluginSurfaceArg {
    Skill,
    Tool,
    Mcp,
    Ui,
}

impl PluginSurfaceArg {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Tool => "tool",
            Self::Mcp => "mcp",
            Self::Ui => "ui",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum PluginChannelArg {
    Stable,
    Beta,
    Nightly,
}

impl PluginChannelArg {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Nightly => "nightly",
        }
    }
}

fn plugin_result_limit(value: &str) -> Result<usize, String> {
    let limit = value
        .parse::<usize>()
        .map_err(|_| "limit must be an integer from 1 to 100".to_string())?;
    if !(1..=100).contains(&limit) {
        return Err("limit must be an integer from 1 to 100".to_string());
    }
    Ok(limit)
}

fn plugin_version(value: &str) -> Result<String, String> {
    let version = semver::Version::parse(value)
        .map_err(|error| format!("version must use semantic version syntax: {error}"))?;
    if version.to_string() != value {
        return Err("version must use canonical semantic version syntax".to_string());
    }
    Ok(value.to_string())
}

fn plugin_operation_id(value: &str) -> Result<String, String> {
    let mut characters = value.chars();
    let valid = value.len() <= 256
        && matches!(characters.next(), Some(first) if first.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '_' | ':' | '/' | '@' | '-')
        });
    if valid {
        Ok(value.to_string())
    } else {
        Err("operation ID contains unsupported characters".to_string())
    }
}

fn plugin_plan_digest(value: &str) -> Result<String, String> {
    let digest = value.strip_prefix("sha256:").unwrap_or(value);
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("plan digest must be 64 lowercase hexadecimal characters".to_string());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_limit_is_bounded() {
        assert_eq!(plugin_result_limit("1").unwrap(), 1);
        assert_eq!(plugin_result_limit("100").unwrap(), 100);
        assert!(plugin_result_limit("0").is_err());
        assert!(plugin_result_limit("101").is_err());
    }

    #[test]
    fn lifecycle_identifiers_are_canonical_and_bounded() {
        assert_eq!(plugin_version("1.2.3").unwrap(), "1.2.3");
        assert!(plugin_version("1.2").is_err());
        assert_eq!(plugin_version("1.2.3+BUILD").unwrap(), "1.2.3+BUILD");

        assert_eq!(
            plugin_operation_id("plugin-install-a3").unwrap(),
            "plugin-install-a3"
        );
        assert!(plugin_operation_id("../operation").is_err());

        let digest = "a".repeat(64);
        assert_eq!(plugin_plan_digest(&digest).unwrap(), digest);
        assert!(plugin_plan_digest(&format!("sha256:{}", "b".repeat(64))).is_ok());
        assert!(plugin_plan_digest(&"A".repeat(64)).is_err());
    }
}
