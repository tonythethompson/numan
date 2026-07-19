use anyhow::{Context, Result};
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
    let script = generate_script(args.shell)?;
    print!("{script}");
    // stderr so `numan completions … | Add-Content` / redirects stay script-only
    eprint!("{}", install_hint(args.shell));
    Ok(())
}

/// Copy-ready install instructions for the generated completion script.
///
/// Written to stderr after the script so stdout remains safe to pipe into a
/// profile or completions file.
pub fn install_hint(shell: CompletionShell) -> String {
    match shell {
        CompletionShell::Bash => "\
# Install:
numan completions bash > ~/.local/share/bash-completion/completions/numan
"
        .to_string(),
        CompletionShell::Zsh => "\
# Install:
numan completions zsh > ~/.zfunc/_numan
"
        .to_string(),
        CompletionShell::Fish => "\
# Install:
numan completions fish > ~/.config/fish/completions/numan.fish
"
        .to_string(),
        CompletionShell::PowerShell => "\
# Install (append to your PowerShell profile):
numan completions powershell | Add-Content -Encoding utf8 $PROFILE
"
        .to_string(),
    }
}

/// Generate a completion script for `shell`.
///
/// PowerShell output is rewritten so it can be appended to an existing
/// `$PROFILE` that already contains statements (see
/// [`make_powershell_profile_safe`]).
pub fn generate_script(shell: CompletionShell) -> Result<String> {
    let mut cmd = Cli::command();
    let mut buf = Vec::new();
    generate(shell.to_clap_shell(), &mut cmd, "numan", &mut buf);
    let script = String::from_utf8(buf).context("completion script was not valid UTF-8")?;
    Ok(match shell {
        CompletionShell::PowerShell => make_powershell_profile_safe(&script),
        _ => script,
    })
}

/// Rewrite clap_complete's PowerShell script so it can be appended to an
/// existing `$PROFILE` that already contains statements.
///
/// clap_complete emits `using namespace ...` directives. PowerShell requires
/// those at the top of a script, so pasting the raw output below other
/// profile content fails with:
/// `A 'using' statement must appear before any other statements in a script.`
fn make_powershell_profile_safe(script: &str) -> String {
    let mut out = String::with_capacity(script.len());
    for line in script.lines() {
        let trimmed = line.trim_start();
        if trimmed == "using namespace System.Management.Automation"
            || trimmed == "using namespace System.Management.Automation.Language"
        {
            continue;
        }
        // Replace short type names (dependent on the removed `using` lines)
        // with fully-qualified names. Replace `CompletionResultType` before
        // `CompletionResult` so the longer name is not partially rewritten.
        let rewritten = line
            .replace(
                "[StringConstantExpressionAst]",
                "[System.Management.Automation.Language.StringConstantExpressionAst]",
            )
            .replace(
                "[StringConstantType]",
                "[System.Management.Automation.Language.StringConstantType]",
            )
            .replace(
                "[CompletionResultType]",
                "[System.Management.Automation.CompletionResultType]",
            )
            .replace(
                "[CompletionResult]::",
                "[System.Management.Automation.CompletionResult]::",
            );
        out.push_str(&rewritten);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_script_is_profile_safe() {
        let script = generate_script(CompletionShell::PowerShell).expect("generate");
        assert!(
            !script.contains("using namespace"),
            "profile-safe script must not emit using namespace directives"
        );
        assert!(script.contains("Register-ArgumentCompleter"));
        assert!(
            script.contains("[System.Management.Automation.Language.StringConstantExpressionAst]")
        );
        assert!(script.contains("[System.Management.Automation.Language.StringConstantType]"));
        assert!(script.contains("[System.Management.Automation.CompletionResult]::"));
        assert!(script.contains("[System.Management.Automation.CompletionResultType]"));
        // Short names must not remain (would fail without `using`).
        assert!(!script.contains("[StringConstantExpressionAst]"));
        assert!(!script.contains("[StringConstantType]"));
        assert!(!script.contains("[CompletionResult]::"));
        assert!(!script.contains("[CompletionResultType]"));
    }

    #[test]
    fn install_hint_is_copy_ready_and_not_part_of_script() {
        let script = generate_script(CompletionShell::PowerShell).expect("generate");
        let hint = install_hint(CompletionShell::PowerShell);
        assert!(
            !script.contains("Add-Content"),
            "install hint must not be mixed into the completion script"
        );
        assert!(hint.contains("numan completions powershell | Add-Content -Encoding utf8 $PROFILE"));
        assert!(install_hint(CompletionShell::Bash).contains("bash-completion/completions/numan"));
        assert!(install_hint(CompletionShell::Zsh).contains("~/.zfunc/_numan"));
        assert!(install_hint(CompletionShell::Fish).contains("numan.fish"));
    }

    #[test]
    fn make_powershell_profile_safe_preserves_trailing_newline_and_body() {
        let raw = "\
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'numan' -ScriptBlock {
    if ($element -isnot [StringConstantExpressionAst] -or
        $element.StringConstantType -ne [StringConstantType]::BareWord) { }
    [CompletionResult]::new('x', 'x', [CompletionResultType]::ParameterName, 'd')
}
";
        let safe = make_powershell_profile_safe(raw);
        assert!(!safe.contains("using namespace"));
        assert!(safe.contains("Register-ArgumentCompleter"));
        assert!(safe.ends_with('\n'));
        assert!(safe.contains(
            "[System.Management.Automation.CompletionResult]::new('x', 'x', [System.Management.Automation.CompletionResultType]::ParameterName, 'd')"
        ));
    }
}
