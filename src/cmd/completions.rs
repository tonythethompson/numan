use anyhow::Result;
use clap::{CommandFactory, ValueEnum};
use clap_complete::{generate, Shell};

use crate::cli::Cli;

/// Generate shell completion scripts
#[derive(clap::Parser)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Fish,
    Zsh,
    #[value(name = "powershell")]
    PowerShell,
}

impl CompletionShell {
    fn to_clap_shell(self) -> Shell {
        match self {
            Self::Bash => Shell::Bash,
            Self::Fish => Shell::Fish,
            Self::Zsh => Shell::Zsh,
            Self::PowerShell => Shell::PowerShell,
        }
    }
}

pub fn execute(args: &CompletionsArgs) -> Result<()> {
    let mut cmd = Cli::command();
    generate(
        args.shell.to_clap_shell(),
        &mut cmd,
        "numan",
        &mut std::io::stdout(),
    );
    Ok(())
}
