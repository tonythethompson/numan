//! Canonical CLI fix hints aligned with `docs/numan-doctor.md`.

/// `numan init`
pub const CMD_INIT: &str = "numan init";

/// `numan init --refresh`
pub const CMD_INIT_REFRESH: &str = "numan init --refresh";

/// `numan activate`
pub const CMD_ACTIVATE: &str = "numan activate";

/// `numan activate --check`
pub const CMD_ACTIVATE_CHECK: &str = "numan activate --check";

/// `numan registry sync`
pub const CMD_REGISTRY_SYNC: &str = "numan registry sync";

/// `numan registry add …`
pub const CMD_REGISTRY_ADD: &str = "numan registry add <name> <url> --key <base64-public-key>";

/// `numan install <owner/name>`
pub const CMD_INSTALL: &str = "numan install <owner/name>";

/// Install command for a concrete package id.
pub fn install_pkg(package_id: &str) -> String {
    format!("numan install {package_id}")
}

/// `numan remove <owner/name>`
pub const CMD_REMOVE: &str = "numan remove <owner/name>";

pub fn remove_pkg(package_id: &str) -> String {
    format!("numan remove {package_id}")
}

/// `numan nupm inspect`
pub const CMD_NUPM_INSPECT: &str = "numan nupm inspect <path>";

pub fn nupm_diff_pkg(package_id: &str) -> String {
    format!("numan nupm diff {package_id}")
}

/// Format a single-command fix hint: `Run 'numan …'.`
pub fn run(cmd: &str) -> String {
    format!("Run '{cmd}'.")
}

/// Format a two-step fix hint: `Run '…', then '…'.`
pub fn run_then(first: &str, second: &str) -> String {
    format!("Run '{first}', then '{second}'.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_formats_single_command() {
        assert_eq!(run(CMD_INIT), "Run 'numan init'.");
    }

    #[test]
    fn run_then_formats_two_commands() {
        assert_eq!(
            run_then(CMD_INIT_REFRESH, CMD_ACTIVATE),
            "Run 'numan init --refresh', then 'numan activate'."
        );
    }
}
