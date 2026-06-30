use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::cmd;

#[derive(Parser)]
#[command(
    name = "numan",
    about = "A cross-platform package manager for Nushell",
    version,
    after_help = "Run 'numan <command> --help' for more information on a command."
)]
pub struct Cli {
    /// Path to numan root directory
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
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
    /// Update installed packages to their latest compatible versions
    Update(cmd::update::UpdateArgs),
    /// Remove an installed package
    Remove(cmd::remove::RemoveArgs),
    /// Garbage-collect orphaned package directories
    Gc(cmd::gc::GcArgs),
    /// Activate installed plugins with Nu
    Activate(cmd::activate::ActivateArgs),
    /// Deactivate active modules
    Deactivate(cmd::deactivate::DeactivateArgs),
    /// List all installed packages
    List,
    /// Initialize Numan and probe the local Nu installation
    Init(cmd::init::InitArgs),
    /// Registry management
    #[command(subcommand)]
    Registry(cmd::registry::RegistryCommands),
    /// Read-only nupm discovery and inspection
    Nupm(cmd::nupm::NupmArgs),
    /// Generate shell completion scripts
    Completions(cmd::completions::CompletionsArgs),
    /// Diagnose Numan root health and optionally apply safe repairs
    Doctor(cmd::doctor::DoctorArgs),
}
