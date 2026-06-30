//! Shell completion generation tests.

use clap::CommandFactory;
use clap_complete::{generate, Shell};
use numan_cli::cli::Cli;

#[test]
fn bash_completions_include_core_commands() {
    let mut cmd = Cli::command();
    let mut buf = Vec::new();
    generate(Shell::Bash, &mut cmd, "numan", &mut buf);
    let script = String::from_utf8(buf).expect("valid utf-8");
    for needle in [
        "numan",
        "init",
        "install",
        "activate",
        "completions",
        "nupm",
    ] {
        assert!(
            script.contains(needle),
            "bash completion script missing '{needle}'"
        );
    }
}

#[test]
fn all_completion_shells_generate_non_empty_output() {
    for shell in [
        Shell::Bash,
        Shell::Fish,
        Shell::Zsh,
        Shell::PowerShell,
    ] {
        let mut cmd = Cli::command();
        let mut buf = Vec::new();
        generate(shell, &mut cmd, "numan", &mut buf);
        assert!(
            !buf.is_empty(),
            "{shell:?} completion script should not be empty"
        );
    }
}
