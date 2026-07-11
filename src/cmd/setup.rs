use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::nu::paths::{find_nu_executable, probe_nu_config_path};
use crate::util::atomic::write_bytes_atomic;
use crate::util::fs_safety::assert_not_symlink;

const VENDOR_LOADER: &str = include_str!("../../assets/nushell-loader/loader.nu");

const CONFIG_SOURCE_LINE: &str = "source ($nu.config-path | path dirname | path join 'loader.nu')";

const CONFIG_SNIPPET: &str = r#"
# Cached third-party init files (numan setup loader)
source ($nu.config-path | path dirname | path join 'loader.nu')
"#;

#[derive(Debug, Subcommand)]
pub enum SetupCommands {
    /// Install the vendored nushell-loader script and print a config.nu snippet
    Loader(LoaderArgs),
}

#[derive(Debug, Args)]
pub struct LoaderArgs {
    /// Overwrite an existing loader.nu without prompting
    #[arg(long)]
    pub force: bool,

    /// Append the loader source line to config.nu when it is not already present
    #[arg(long)]
    pub configure: bool,

    /// Skip confirmation prompts (required for non-TTY when overwriting or configuring)
    #[arg(long)]
    pub yes: bool,
}

pub fn execute(cmd: SetupCommands) -> Result<()> {
    match cmd {
        SetupCommands::Loader(args) => execute_loader(&args),
    }
}

pub fn execute_loader(args: &LoaderArgs) -> Result<()> {
    execute_loader_with_probe(args, || {
        let nu_exe = find_nu_executable()?;
        probe_nu_config_path(&nu_exe)
    })
}

pub fn execute_loader_with_probe<F>(args: &LoaderArgs, probe: F) -> Result<()>
where
    F: FnOnce() -> Result<PathBuf>,
{
    let config_path = probe()?;
    let config_dir = config_path
        .parent()
        .context("Nu config path has no parent directory")?;
    let loader_path = config_dir.join("loader.nu");

    std::fs::create_dir_all(config_dir).with_context(|| {
        format!(
            "Failed to create Nu config directory '{}'",
            config_dir.display()
        )
    })?;

    install_loader_file(&loader_path, args)?;

    if args.configure {
        configure_config_nu(&config_path, args)?;
    } else {
        print_manual_snippet(&config_path);
    }

    print_next_steps(&loader_path, args.configure);
    Ok(())
}

fn install_loader_file(loader_path: &Path, args: &LoaderArgs) -> Result<()> {
    if loader_path.exists() && !args.force {
        if !loader_path.is_file() {
            bail!(
                "Refusing to overwrite non-file at '{}'.",
                loader_path.display()
            );
        }

        let existing = std::fs::read_to_string(loader_path).with_context(|| {
            format!(
                "Failed to read existing loader at '{}'",
                loader_path.display()
            )
        })?;
        if existing == VENDOR_LOADER {
            println!(
                "Loader already installed at '{}' (unchanged).",
                loader_path.display()
            );
            return Ok(());
        }

        if !std::io::stdin().is_terminal() && !args.yes {
            bail!(
                "loader.nu already exists at '{}'. Pass --force to overwrite, or --yes in non-TTY sessions.",
                loader_path.display()
            );
        }

        if !args.yes {
            print!(
                "loader.nu already exists at '{}'. Overwrite with the vendored copy? [y/N] ",
                loader_path.display()
            );
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                bail!("Loader install cancelled.");
            }
        }
    }

    write_bytes_atomic(loader_path, VENDOR_LOADER.as_bytes()).with_context(|| {
        format!(
            "Failed to write loader script to '{}'",
            loader_path.display()
        )
    })?;

    println!("Installed nushell-loader to '{}'.", loader_path.display());
    Ok(())
}

fn configure_config_nu(config_path: &Path, args: &LoaderArgs) -> Result<()> {
    if config_path.exists() {
        assert_not_symlink(config_path, "config.nu")?;
    }
    if config_path.exists() && !config_path.is_file() {
        bail!(
            "Refusing to modify non-file config at '{}'.",
            config_path.display()
        );
    }

    if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read '{}'", config_path.display()))?;
        if config_already_sources_loader(&content) {
            println!(
                "'{}' already sources loader.nu (unchanged).",
                config_path.display()
            );
            return Ok(());
        }

        if !std::io::stdin().is_terminal() && !args.yes {
            bail!(
                "Interactive confirmation required to modify config.nu in non-TTY sessions. \
                 Pass --yes to append the loader source line."
            );
        }

        if !args.yes {
            print!(
                "Append loader source line to '{}'? [y/N] ",
                config_path.display()
            );
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                print_manual_snippet(config_path);
                return Ok(());
            }
        }

        let updated = format!("{}{CONFIG_SNIPPET}", content.trim_end());
        write_bytes_atomic(config_path, updated.as_bytes())
            .with_context(|| format!("Failed to update '{}'", config_path.display()))?;
        println!(
            "Appended loader source line to '{}'.",
            config_path.display()
        );
        return Ok(());
    }

    write_bytes_atomic(
        config_path,
        format!("{CONFIG_SNIPPET}\n").trim_start().as_bytes(),
    )
    .with_context(|| format!("Failed to create '{}'", config_path.display()))?;
    println!(
        "Created '{}' with loader source line.",
        config_path.display()
    );
    Ok(())
}

fn print_manual_snippet(config_path: &Path) {
    println!();
    println!("Add this at the end of '{}':", config_path.display());
    println!("{CONFIG_SNIPPET}");
}

fn print_next_steps(loader_path: &Path, configured: bool) {
    println!();
    println!("Next steps:");
    println!(
        "  1. Edit '{}' and add entries to aidnem_loader_configs.",
        loader_path.display()
    );
    println!("     Example:");
    println!("       {{name: 'starship', command: \"starship init nu\"}}");
    if !configured {
        println!("  2. Source loader.nu from config.nu (see snippet above).");
        println!("  3. Restart Nu. First startup generates caches; later startups are faster.");
    } else {
        println!("  2. Restart Nu. First startup generates caches; later startups are faster.");
    }
    println!();
    println!(
        "Numan module autoloads use the same vendor/autoload directory via numan.nu \
         and are unaffected by loader caches."
    );
    println!("Upstream: https://github.com/aidnem/nushell-loader");
}

pub fn config_already_sources_loader(content: &str) -> bool {
    content.contains(CONFIG_SOURCE_LINE)
        || content.contains("path join 'loader.nu'")
        || content.contains("path join \"loader.nu\"")
        || content.lines().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("source ") && trimmed.to_ascii_lowercase().contains("loader.nu")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn config_detection_finds_exact_source_line() {
        let content = format!("export-env {{}}\n{CONFIG_SOURCE_LINE}\n");
        assert!(config_already_sources_loader(&content));
    }

    #[test]
    fn config_detection_finds_literal_loader_source() {
        let content = "source ~/.config/nushell/loader.nu\n";
        assert!(config_already_sources_loader(content));
    }

    #[test]
    fn config_detection_false_when_absent() {
        assert!(!config_already_sources_loader("use std/log\n"));
    }

    #[test]
    fn install_loader_writes_vendored_copy() {
        let dir = TempDir::new().unwrap();
        let loader_path = dir.path().join("loader.nu");
        let args = LoaderArgs {
            force: false,
            configure: false,
            yes: true,
        };

        install_loader_file(&loader_path, &args).unwrap();
        let written = std::fs::read_to_string(&loader_path).unwrap();
        assert_eq!(written, VENDOR_LOADER);
    }

    #[test]
    fn install_loader_skips_when_unchanged() {
        let dir = TempDir::new().unwrap();
        let loader_path = dir.path().join("loader.nu");
        write_bytes_atomic(&loader_path, VENDOR_LOADER.as_bytes()).unwrap();

        let args = LoaderArgs {
            force: false,
            configure: false,
            yes: true,
        };
        install_loader_file(&loader_path, &args).unwrap();
        assert_eq!(
            std::fs::read(&loader_path).unwrap(),
            VENDOR_LOADER.as_bytes()
        );
    }

    #[cfg(unix)]
    #[test]
    fn configure_rejects_symlinked_config() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let target = dir.path().join("config.real.nu");
        std::fs::write(&target, "export-env {}\n").unwrap();
        let config_path = dir.path().join("config.nu");
        symlink(&target, &config_path).unwrap();

        let args = LoaderArgs {
            force: false,
            configure: true,
            yes: true,
        };
        let err = configure_config_nu(&config_path, &args).unwrap_err();
        assert!(err.to_string().contains("symlink"));
    }
    #[test]
    fn configure_appends_snippet_to_existing_config() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.nu");
        std::fs::write(&config_path, "export-env {}\n").unwrap();

        let args = LoaderArgs {
            force: false,
            configure: true,
            yes: true,
        };
        configure_config_nu(&config_path, &args).unwrap();

        let updated = std::fs::read_to_string(&config_path).unwrap();
        assert!(config_already_sources_loader(&updated));
        assert!(updated.starts_with("export-env {}\n"));
    }

    #[test]
    fn execute_loader_with_probe_installs_next_to_config() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.nu");
        std::fs::write(&config_path, "export-env {}\n").unwrap();

        let args = LoaderArgs {
            force: false,
            configure: true,
            yes: true,
        };

        execute_loader_with_probe(&args, || Ok(config_path.clone())).unwrap();
        assert!(dir.path().join("loader.nu").is_file());
        let config = std::fs::read_to_string(&config_path).unwrap();
        assert!(config_already_sources_loader(&config));
    }
}
