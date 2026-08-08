use a3s::components::ComponentId;
use clap::{Args, Subcommand, ValueEnum};

#[derive(Clone, Debug, Args)]
pub(crate) struct InfoArgs {
    #[arg(value_name = "COMPONENT")]
    pub component: ComponentId,
    /// Query and include known versions.
    #[arg(long)]
    pub versions: bool,
    /// Include every catalog-declared source.
    #[arg(long)]
    pub sources: bool,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct DoctorArgs {
    #[arg(value_name = "COMPONENT")]
    pub component: Option<ComponentId>,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct RegistryArgs {
    #[command(subcommand)]
    pub command: RegistryCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum RegistryCommand {
    /// List the A3S Use Registry sources and configuration revision.
    List,
    /// Show one Registry source and its trust identity.
    Show(RegistryNameArgs),
    /// Trust and add a named Registry source.
    Add(RegistryAddArgs),
    /// Replace one source under an exact reviewed configuration revision.
    Replace(RegistryReplaceArgs),
    /// Select the default Registry source.
    Default(RegistryMutationArgs),
    /// Enable one Registry source.
    Enable(RegistryMutationArgs),
    /// Disable one Registry source without deleting its trust configuration.
    Disable(RegistryMutationArgs),
    /// Remove one Registry source.
    Remove(RegistryRemoveArgs),
    /// Check registry reachability and metadata freshness.
    Refresh(RegistryRefreshArgs),
}

#[derive(Clone, Debug, Args)]
pub(crate) struct RegistryNameArgs {
    #[arg(value_name = "NAME")]
    pub name: String,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct RegistryAddArgs {
    /// Stable lowercase source name.
    #[arg(value_name = "NAME")]
    pub name: String,
    #[arg(value_name = "URL")]
    pub url: String,
    /// SHA-256 digest of the trusted TUF root metadata.
    #[arg(long, value_name = "SHA256")]
    pub root_sha256: String,
    /// Import a regular TUF root metadata file into A3S Use-owned state.
    #[arg(long, value_name = "FILE")]
    pub trusted_root: Option<std::path::PathBuf>,
    /// Accept the explicit trust operation without prompting.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct RegistryReplaceArgs {
    #[arg(value_name = "NAME")]
    pub name: String,
    #[arg(value_name = "URL")]
    pub url: String,
    /// SHA-256 digest of the replacement trusted TUF root metadata.
    #[arg(long, value_name = "SHA256")]
    pub root_sha256: String,
    /// Import a regular replacement TUF root metadata file.
    #[arg(long, value_name = "FILE")]
    pub trusted_root: Option<std::path::PathBuf>,
    /// Exact source configuration revision returned by `a3s registry list`.
    #[arg(long, value_name = "SHA256")]
    pub revision: String,
    /// Accept the explicit trust replacement without prompting.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct RegistryMutationArgs {
    #[arg(value_name = "NAME")]
    pub name: String,
    /// Exact source configuration revision returned by `a3s registry list`.
    #[arg(long, value_name = "SHA256")]
    pub revision: String,
    /// Accept the source-state mutation without prompting.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct RegistryRemoveArgs {
    #[arg(value_name = "NAME")]
    pub name: String,
    /// Exact source configuration revision returned by `a3s registry list`.
    #[arg(long, value_name = "SHA256")]
    pub revision: String,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct RegistryRefreshArgs {
    #[arg(value_name = "NAME")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct CacheArgs {
    #[command(subcommand)]
    pub command: CacheCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CacheCommand {
    /// Print the recreatable cache root.
    Path,
    /// Report cache size and entry counts.
    Status,
    /// Remove expired and unreferenced temporary entries.
    Prune(CacheMutationArgs),
    /// Remove all recreatable A3S cache content.
    Clean(CacheCleanArgs),
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct CacheMutationArgs {
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct CacheCleanArgs {
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
    Elvish,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct HelpArgs {
    #[arg(value_name = "COMMAND")]
    pub command: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ComponentKindArg {
    #[value(name = "built-in")]
    BuiltIn,
    Product,
    Capability,
    Extension,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum ReleaseChannelArg {
    #[default]
    Stable,
    Beta,
    Nightly,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum InstallScopeArg {
    #[default]
    User,
    System,
}
