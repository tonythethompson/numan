use clap::{Parser, Subcommand};
use std::path::PathBuf;

use numan_cli::cmd;
use numan_cli::config;
use numan_cli::core;

#[derive(Parser)]
#[command(
    name = "numan",
    about = "A cross-platform package manager for Nushell",
    version,
    after_help = "Run 'numan <command> --help' for more information on a command."
)]
struct Cli {
    /// Path to numan root directory
    #[arg(long, global = true)]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Search registry by name/description/tags
    Search {
        /// Search query
        query: String,
    },
    /// Show package details, versions, platforms
    Info {
        /// Package ID (owner/name)
        id: String,
    },
    /// Install a package
    Install(cmd::install::InstallArgs),
    /// List all installed packages
    List,
    /// Registry management
    #[command(subcommand)]
    Registry(cmd::registry::RegistryCommands),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let platform = core::platform::Platform::detect();
    let root = cli.root.unwrap_or_else(|| config::Config::resolve_root(&platform));

    // Ensure root directory exists
    std::fs::create_dir_all(&root)?;

    match cli.command {
        Commands::Search { query } => cmd::search::execute(&query, &root),
        Commands::Info { id } => cmd::info::execute(&id, &root),
        Commands::Install(args) => cmd::install::execute(&args, &root),
        Commands::List => cmd::list::execute(&root),
        Commands::Registry(cmd) => cmd::registry::execute(cmd, &root),
    }
}
