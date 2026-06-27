use anyhow::{bail, Context, Result};
use clap::Args;
use std::io::{IsTerminal, Write};
use std::path::{Component, Path, PathBuf};

use crate::nu::paths::NuPaths;
use crate::state::journal::{PendingActivation, PendingActivationEntry, PendingStatus};
use crate::state::lockfile::{Lockfile, PluginActivation};
use crate::util::format_timestamp;

#[derive(Args, Debug)]
pub struct ActivateArgs {
    /// Package IDs (owner/name) to activate. Omit to activate all installed inactive plugins.
    pub packages: Vec<String>,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Show detailed output
    #[arg(long)]
    pub verbose: bool,
}

/// Resolved plugin target ready for registration.
struct ActivateTarget {
    package_id: String,
    payload_path: String,
    executable_path: String,
    /// Canonicalized, root-anchored absolute path to the plugin binary.
    absolute_binary_path: PathBuf,
}

/// `plugin add` Nu program — paths come only from environment variables.
/// Never interpolated into the command string.
const ADD_PLUGIN: &str =
    "plugin add --plugin-config $env.NUMAN_PLUGIN_CONFIG $env.NUMAN_PLUGIN_BINARY";

pub fn execute(args: &ActivateArgs, root: &Path) -> Result<()> {
    execute_with_registrar(args, root, &run_plugin_add)
}

/// Testability entry point — accepts an injectable plugin registrar.
///
/// The registrar receives `(nu_executable, plugin_binary_path, plugin_config_path)`
/// and returns `Ok(())` on success. Production code calls `run_plugin_add`.
/// Tests inject a fake registrar to exercise all flow paths without invoking Nu.
pub fn execute_with_registrar(
    args: &ActivateArgs,
    root: &Path,
    registrar: &dyn Fn(&str, &str, &str) -> Result<()>,
) -> Result<()> {
    // 1. Load cached Nu identity — never re-discover here
    let nu_paths = NuPaths::load(root)?;

    // 2. Validate drift — refuse if nu binary changed since init
    nu_paths.validate_drift()?;

    // 3. Load lockfile
    let mut lockfile = Lockfile::load(root)?;

    // 4. Reconcile any interrupted journal
    if let Some(existing) = PendingActivation::load(root)? {
        if !existing.matches_nu_identity(
            &nu_paths.nu_executable_hash,
            &nu_paths.nu_version,
            &nu_paths.plugin_registry_path,
        ) {
            bail!(
                "A pending activation journal exists from a different Nu identity.\n\
                 Run 'numan init --refresh' to clear stale state, then retry.\n\
                 Journal: {}",
                root.join("state/pending-activation.json").display()
            );
        }
        eprintln!(
            "{}  Reconciling interrupted activation journal…",
            console::style("⚠").yellow()
        );
        // Persist any `registered` entries to the lockfile.
        // Unconditionally overwrite — the journal was written for the current Nu
        // identity (checked above), so a stale activation from a prior identity
        // must be replaced with the current one.
        for entry in &existing.entries {
            if entry.status == PendingStatus::Registered {
                if let Some(pkg) = lockfile.packages.get_mut(&entry.package_id) {
                    pkg.activation = Some(PluginActivation {
                        plugin_registry_path: nu_paths.plugin_registry_path.clone(),
                        nu_executable_sha256: nu_paths.nu_executable_hash.clone(),
                        nu_version: nu_paths.nu_version.clone(),
                        activated_at: format_timestamp(),
                    });
                }
            }
        }
        lockfile.save(root)?;
        PendingActivation::delete(root)?;
        eprintln!("   Reconciliation complete.");
    }

    // 5. Resolve targets
    let targets = resolve_targets(args, &lockfile, &nu_paths, root)?;

    if targets.is_empty() {
        println!("Nothing to activate.");
        return Ok(());
    }

    // 6. Consent table + confirmation
    print_consent_table(&targets, &nu_paths.plugin_registry_path);

    if !std::io::stdin().is_terminal() && !args.yes {
        bail!(
            "Interactive confirmation required for non-TTY sessions. \
             Pass --yes to activate without prompting."
        );
    }

    if !args.yes {
        print!("Proceed? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            bail!("Activation cancelled.");
        }
    }

    // 7. Snapshot lockfile once before first mutation
    if !lockfile.is_empty() {
        lockfile.snapshot(root)?;
    }

    // 8. Write pending journal — all entries start at `prepared`
    let mut journal = PendingActivation {
        nu_executable_sha256: nu_paths.nu_executable_hash.clone(),
        nu_version: nu_paths.nu_version.clone(),
        plugin_registry_path: nu_paths.plugin_registry_path.clone(),
        created_at: format_timestamp(),
        entries: targets
            .iter()
            .map(|t| PendingActivationEntry {
                package_id: t.package_id.clone(),
                payload_path: t.payload_path.clone(),
                executable_path: t.executable_path.clone(),
                absolute_binary_path: t.absolute_binary_path.to_string_lossy().into_owned(),
                status: PendingStatus::Prepared,
                error: None,
            })
            .collect(),
    };
    journal.save(root)?;

    // 9. Register each plugin
    let mut any_failed = false;
    for (i, target) in targets.iter().enumerate() {
        if args.verbose {
            println!(
                "  Registering {} ({})",
                target.package_id,
                target.absolute_binary_path.display()
            );
        }

        let binary_str = target.absolute_binary_path.to_string_lossy();
        match registrar(
            &nu_paths.nu_executable,
            &binary_str,
            &nu_paths.plugin_registry_path,
        ) {
            Ok(()) => {
                // a. Advance journal entry to `registered`
                journal.entries[i].status = PendingStatus::Registered;
                journal.save(root)?;

                // b. Persist activation record to lockfile atomically
                if let Some(pkg) = lockfile.packages.get_mut(&target.package_id) {
                    pkg.activation = Some(PluginActivation {
                        plugin_registry_path: nu_paths.plugin_registry_path.clone(),
                        nu_executable_sha256: nu_paths.nu_executable_hash.clone(),
                        nu_version: nu_paths.nu_version.clone(),
                        activated_at: format_timestamp(),
                    });
                }
                lockfile.save(root)?;

                println!(
                    "{} Registered {} — takes effect in future Nu sessions (not the current shell)",
                    console::style("✓").green(),
                    target.package_id,
                );
            }
            Err(e) => {
                journal.entries[i].status = PendingStatus::Failed;
                journal.entries[i].error = Some(e.to_string());
                journal.save(root)?;

                eprintln!(
                    "{} Failed to register {}: {}",
                    console::style("✗").red(),
                    target.package_id,
                    e
                );
                any_failed = true;
            }
        }
    }

    // 10. Cleanup
    PendingActivation::delete(root)?;

    if any_failed {
        bail!(
            "One or more plugins failed to activate. Successful activations have been persisted."
        );
    }

    Ok(())
}

/// Build the list of plugins to activate.
fn resolve_targets(
    args: &ActivateArgs,
    lockfile: &Lockfile,
    nu_paths: &NuPaths,
    root: &Path,
) -> Result<Vec<ActivateTarget>> {
    if args.packages.is_empty() {
        // All installed plugins not yet active for this Nu identity
        let mut targets = Vec::new();
        for (id, entry) in &lockfile.packages {
            if entry.package_type != "plugin" {
                continue;
            }
            if entry.is_active_for(
                &nu_paths.nu_executable_hash,
                &nu_paths.nu_version,
                &nu_paths.plugin_registry_path,
            ) {
                continue;
            }
            let exe = entry
                .executable_path
                .as_deref()
                .with_context(|| format!("{id}: missing executable_path in lockfile"))?;
            let abs = validate_payload_path(root, &entry.payload_path, exe)?;
            targets.push(ActivateTarget {
                package_id: id.clone(),
                payload_path: entry.payload_path.clone(),
                executable_path: exe.to_string(),
                absolute_binary_path: abs,
            });
        }
        Ok(targets)
    } else {
        let mut targets = Vec::new();
        for pkg_id in &args.packages {
            let entry = lockfile.packages.get(pkg_id).with_context(|| {
                format!("Package '{pkg_id}' not found in lockfile (not installed)")
            })?;

            if entry.package_type != "plugin" {
                bail!(
                    "Package '{pkg_id}' is a {} — only plugins can be activated in Phase 3 \
                     (modules/scripts/completions: Phase 4).",
                    entry.package_type
                );
            }

            if entry.is_active_for(
                &nu_paths.nu_executable_hash,
                &nu_paths.nu_version,
                &nu_paths.plugin_registry_path,
            ) {
                println!(
                    "{} {} is already active for Nu {} (skipped)",
                    console::style("✓").green(),
                    pkg_id,
                    nu_paths.nu_version,
                );
                continue;
            }

            let exe = entry
                .executable_path
                .as_deref()
                .with_context(|| format!("{pkg_id}: missing executable_path in lockfile"))?;
            let abs = validate_payload_path(root, &entry.payload_path, exe)?;
            targets.push(ActivateTarget {
                package_id: pkg_id.clone(),
                payload_path: entry.payload_path.clone(),
                executable_path: exe.to_string(),
                absolute_binary_path: abs,
            });
        }
        Ok(targets)
    }
}

/// Validate that `payload_path` and `executable_path` are safe, then resolve
/// the absolute binary path and verify it exists under `root`.
///
/// All validation happens in Rust before any shell invocation. Paths are never
/// interpolated into command strings — they are passed via environment variables.
pub fn validate_payload_path(
    root: &Path,
    payload_path: &str,
    executable_path: &str,
) -> Result<PathBuf> {
    // payload_path: relative, no traversal
    let pp = Path::new(payload_path);
    if pp.is_absolute() {
        bail!("payload_path is absolute (must be relative): '{payload_path}'");
    }
    for component in pp.components() {
        if component == Component::ParentDir {
            bail!("payload_path contains '..' traversal: '{payload_path}'");
        }
    }

    // executable_path: relative, no traversal
    let ep = Path::new(executable_path);
    if ep.is_absolute() {
        bail!("executable_path is absolute (must be relative): '{executable_path}'");
    }
    for component in ep.components() {
        if component == Component::ParentDir {
            bail!("executable_path contains '..' traversal: '{executable_path}'");
        }
    }

    // Resolve and canonicalize (follows symlinks — catches escape via symlink)
    let candidate = root.join(payload_path).join(executable_path);
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("Binary not found at '{}'", candidate.display()))?;

    // Must stay under root after canonicalization
    let canonical_root = root
        .canonicalize()
        .context("Failed to canonicalize numan root")?;
    if !canonical.starts_with(&canonical_root) {
        bail!(
            "Binary path escapes numan root after canonicalization: '{}'",
            canonical.display()
        );
    }

    // Must be a regular file
    let meta = std::fs::metadata(&canonical)
        .with_context(|| format!("Failed to stat '{}'", canonical.display()))?;
    if !meta.is_file() {
        bail!(
            "Binary path is not a regular file: '{}'",
            canonical.display()
        );
    }

    // Basename must start with nu_plugin_
    let basename = canonical.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let base_stem = basename.trim_end_matches(".exe");
    if !base_stem.starts_with("nu_plugin_") {
        bail!(
            "Binary name must start with 'nu_plugin_' (got '{basename}'). \
             Only Nu plugins can be activated with this command."
        );
    }

    Ok(canonical)
}

/// Print the consent table showing packages, binaries, and target registry.
fn print_consent_table(targets: &[ActivateTarget], registry_path: &str) {
    println!("\nPlugins to activate:");
    println!("  {:<32} {:<52} Registry", "Package", "Binary");
    println!("  {}", "-".repeat(110));
    for t in targets {
        println!(
            "  {:<32} {:<52} {}",
            t.package_id,
            t.absolute_binary_path.display(),
            registry_path,
        );
    }
    println!();
}

/// Invoke Nu with `plugin add` — paths via environment variables only.
fn run_plugin_add(nu_executable: &str, plugin_binary: &str, plugin_config: &str) -> Result<()> {
    let output = std::process::Command::new(nu_executable)
        .args(["-c", ADD_PLUGIN])
        .env("NUMAN_PLUGIN_BINARY", plugin_binary)
        .env("NUMAN_PLUGIN_CONFIG", plugin_config)
        .output()
        .with_context(|| format!("Failed to invoke Nu at '{nu_executable}'"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("plugin add failed:\n{stderr}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_payload_path_rejects_absolute_payload() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let err = validate_payload_path(&root, "/absolute/path", "nu_plugin_x").unwrap_err();
        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn validate_payload_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let err = validate_payload_path(&root, "packages/../../../etc", "nu_plugin_x").unwrap_err();
        assert!(err.to_string().contains("..") || err.to_string().contains("traversal"));
    }

    #[test]
    fn validate_executable_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("packages")).unwrap();
        let err = validate_payload_path(&root, "packages", "../../../etc/shadow").unwrap_err();
        assert!(err.to_string().contains("..") || err.to_string().contains("traversal"));
    }

    #[test]
    fn validate_payload_path_rejects_absolute_executable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let err = validate_payload_path(
            &root,
            "packages/plugins/x/y/1.0.0-abc",
            "/absolute/nu_plugin_x",
        )
        .unwrap_err();
        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn validate_executable_name_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let payload = "packages/plugins/owner/name/1.0.0-abc";
        std::fs::create_dir_all(root.join(payload)).unwrap();
        // File exists but name doesn't start with nu_plugin_
        std::fs::write(root.join(payload).join("wrong_name"), b"fake").unwrap();
        let err = validate_payload_path(&root, payload, "wrong_name").unwrap_err();
        assert!(err.to_string().contains("nu_plugin_"));
    }

    #[test]
    fn validate_binary_path_under_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        // Binary exists but outside root
        let outside = tempfile::tempdir().unwrap();
        let ext_binary = outside.path().join("nu_plugin_evil");
        std::fs::write(&ext_binary, b"evil").unwrap();

        // Create a symlink inside root pointing outside
        #[cfg(unix)]
        {
            let payload = "packages/plugins/owner/evil/1.0.0-abc";
            std::fs::create_dir_all(root.join(payload)).unwrap();
            std::os::unix::fs::symlink(&ext_binary, root.join(payload).join("nu_plugin_evil"))
                .unwrap();
            let err = validate_payload_path(&root, payload, "nu_plugin_evil").unwrap_err();
            assert!(
                err.to_string().contains("escapes") || err.to_string().contains("root"),
                "Expected root-escape error, got: {err}"
            );
        }
        #[cfg(not(unix))]
        {
            // On Windows, symlink creation requires elevation — skip symlink test,
            // but verify the binary-not-found path
            let err = validate_payload_path(&root, "packages/plugins/x/y/1.0.0-abc", "nu_plugin_x")
                .unwrap_err();
            assert!(err.to_string().contains("not found") || err.to_string().contains("Binary"));
        }
    }

    #[test]
    fn validate_payload_path_succeeds_for_valid_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let payload = "packages/plugins/owner/test/1.0.0-abc";
        std::fs::create_dir_all(root.join(payload)).unwrap();
        std::fs::write(root.join(payload).join("nu_plugin_test"), b"fake").unwrap();
        let result = validate_payload_path(&root, payload, "nu_plugin_test");
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
    }

    #[test]
    fn add_plugin_command_string_contains_env_vars_only() {
        assert!(ADD_PLUGIN.contains("$env.NUMAN_PLUGIN_BINARY"));
        assert!(ADD_PLUGIN.contains("$env.NUMAN_PLUGIN_CONFIG"));
        assert!(
            !ADD_PLUGIN.contains('/'),
            "No literal paths in ADD_PLUGIN command string"
        );
    }
}
