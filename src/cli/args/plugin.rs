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
}
