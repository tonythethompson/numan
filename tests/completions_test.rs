//! Shell completion generation tests.

use numan_cli::cmd::completions::{generate_script, CompletionShell};

#[test]
fn bash_completions_include_core_commands() {
    let script = generate_script(CompletionShell::Bash).expect("generate bash");
    for needle in [
        "numan",
        "init",
        "install",
        "activate",
        "try",
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
        CompletionShell::Bash,
        CompletionShell::Fish,
        CompletionShell::Zsh,
        CompletionShell::PowerShell,
    ] {
        let script = generate_script(shell).expect("generate");
        assert!(
            !script.is_empty(),
            "{shell:?} completion script should not be empty"
        );
    }
}

#[test]
fn powershell_completions_can_append_to_existing_profile() {
    let script = generate_script(CompletionShell::PowerShell).expect("generate powershell");
    assert!(
        !script
            .lines()
            .any(|line| line.trim_start().starts_with("using ")),
        "PowerShell completions must not require top-of-script `using` directives"
    );
    assert!(script.contains("Register-ArgumentCompleter"));
    assert!(script.contains("[System.Management.Automation.CompletionResult]::"));
    assert!(script.contains("numan"));
}

#[test]
fn powershell_install_hint_is_ready_to_copy() {
    use numan_cli::cmd::completions::install_hint;

    let hint = install_hint(CompletionShell::PowerShell);
    assert!(hint.contains("Add-Content -Encoding utf8 $PROFILE"));
    assert!(
        !generate_script(CompletionShell::PowerShell)
            .expect("generate")
            .contains("Add-Content"),
        "hint must stay on stderr / separate from script stdout"
    );
}
