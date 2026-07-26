use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "hydian",
    version,
    about = "One endpoint. Every MCP server.",
    long_about = "Configure MCP servers once. Point every client at Hydian."
)]
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    /// Override the Hydian home directory.
    #[arg(long, global = true, env = "HYDIAN_HOME")]
    pub home: Option<PathBuf>,

    /// Override the Hydian TOML configuration path.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Override the MCP server JSON configuration path.
    #[arg(long = "mcp-config", global = true)]
    pub mcp_config: Option<PathBuf>,

    /// Override the active profile for this invocation.
    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Emit stable machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,

    /// Use plain non-interactive output and never launch the TUI.
    #[arg(long, global = true)]
    pub plain: bool,

    /// Disable ANSI color.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Suppress successful human output.
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Increase diagnostic detail.
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,

    /// Include internal error chains.
    #[arg(long, global = true)]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Create Hydian's default configuration and directories.
    Init(InitArgs),
    /// Import MCP servers from another client's configuration.
    Import(ImportArgs),
    /// Run the Streamable HTTP gateway in the foreground.
    Serve(ServeArgs),
    /// Run Hydian as an MCP stdio server.
    Stdio,
    /// Launch the full-screen terminal interface.
    Tui,
    /// Diagnose configuration, security, paths, and dependencies.
    Doctor(DoctorArgs),
    /// Show the latest runtime status.
    Status,
    /// Print the configured MCP endpoint.
    Endpoint(EndpointArgs),
    /// Inspect or control configured servers.
    Servers(ServersArgs),
    /// Search or inspect the aggregated tool catalog.
    Tools(ToolsArgs),
    /// Inspect or activate profiles.
    Profiles(ProfilesArgs),
    /// Install or control background service integration.
    Service(ServiceArgs),
    /// Plan or control an external exposure provider.
    Expose(ExposeArgs),
    /// Explain a security or exposure choice in plain language.
    Explain(ExplainArgs),
    /// Generate a shell completion script.
    Completion(CompletionArgs),
}

#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    /// Show the files and directories that would be created.
    #[arg(long)]
    pub dry_run: bool,

    /// Replace existing Hydian files after creating backups.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ImportArgs {
    pub path: PathBuf,

    #[arg(long, value_enum, default_value_t = ImportFormat::Auto)]
    pub format: ImportFormat,

    /// Preview the import without changing Hydian's configuration.
    #[arg(long)]
    pub dry_run: bool,

    /// Apply the import. Without this flag, import is a preview.
    #[arg(long, conflicts_with = "dry_run")]
    pub apply: bool,

    #[arg(long = "on-conflict", value_enum, default_value_t = ConflictChoice::Skip)]
    pub on_conflict: ConflictChoice,

    /// Suffix used by `--on-conflict rename`.
    #[arg(long, default_value = "imported")]
    pub rename_suffix: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImportFormat {
    Auto,
    Claude,
    Vscode,
    Cursor,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConflictChoice {
    Skip,
    Replace,
    Rename,
}

#[derive(Debug, Clone, Args)]
pub struct ServeArgs {
    /// Attach the terminal dashboard to the foreground runtime.
    #[arg(long)]
    pub tui: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DoctorArgs {
    /// Return failure when advisory security checks warn.
    #[arg(long)]
    pub strict: bool,
}

#[derive(Debug, Clone, Args)]
pub struct EndpointArgs {
    #[arg(long, value_enum, default_value_t = EndpointFormat::Url)]
    pub format: EndpointFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EndpointFormat {
    Url,
    Json,
    Openai,
}

#[derive(Debug, Clone, Args)]
pub struct ServersArgs {
    #[command(subcommand)]
    pub command: ServerCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ServerCommand {
    List,
    Show { name: String },
    Start(RuntimeMutationArgs),
    Stop(RuntimeMutationArgs),
    Restart(RuntimeMutationArgs),
}

#[derive(Debug, Clone, Args)]
pub struct RuntimeMutationArgs {
    pub name: String,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ToolsArgs {
    #[command(subcommand)]
    pub command: ToolCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ToolCommand {
    List,
    Search { query: String },
    Show { qualified_name: String },
}

#[derive(Debug, Clone, Args)]
pub struct ProfilesArgs {
    #[command(subcommand)]
    pub command: ProfileCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ProfileCommand {
    List,
    Show {
        name: String,
    },
    Use {
        name: String,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub command: ServiceCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ServiceCommand {
    Install {
        #[arg(long)]
        system: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        acknowledge_temporary_path: bool,
    },
    Uninstall {
        #[arg(long)]
        system: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Start {
        #[arg(long)]
        system: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Stop {
        #[arg(long)]
        system: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Restart {
        #[arg(long)]
        system: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Status {
        #[arg(long)]
        system: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct ExposeArgs {
    #[command(subcommand)]
    pub command: ExposeCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ExposeCommand {
    Plan(ProviderArgs),
    Start(ProviderArgs),
    Stop(ProviderStopArgs),
    Status,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderArgs {
    pub provider: String,

    #[arg(long)]
    pub scope: Option<String>,

    #[arg(long)]
    pub mode: Option<String>,

    #[arg(long)]
    pub dry_run: bool,

    /// Arguments passed directly to the provider after `--`.
    #[arg(last = true)]
    pub provider_args: Vec<String>,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderStopArgs {
    pub provider: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ExplainArgs {
    #[arg(value_enum)]
    pub topic: ExplainTopic,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExplainTopic {
    NonLoopbackWithoutAuth,
    OriginValidation,
    PlaintextSecrets,
    ProviderExposure,
}

#[derive(Debug, Clone, Args)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: Shell,
}
