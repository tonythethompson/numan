use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::util::fs_safety::is_symlink_or_reparse;

use super::schema::{MAX_MODULE_TREE_ENTRIES, MAX_PARENT_WALK_HOPS, METADATA_FILENAME};

/// Walk parents from `start` looking for `nupm.nuon` (bounded).
pub fn find_package_root(start: &Path) -> Result<Option<PathBuf>> {
    let start = absolute_path(start)
        .with_context(|| format!("Failed to resolve path '{}'", start.display()))?;
    check_path_prefixes_for_symlinks(&start)?;

    let mut current = start;
    for _ in 0..MAX_PARENT_WALK_HOPS {
        let candidate = current.join(METADATA_FILENAME);
        if is_symlink_or_reparse(&candidate)? {
            anyhow::bail!(
                "Unsafe filesystem layout: metadata file '{}' is a symlink or reparse point",
                candidate.display()
            );
        }
        if is_regular_file(&candidate)? {
            return Ok(Some(current));
        }
        match current.parent() {
            Some(p) => current = p.to_path_buf(),
            None => break,
        }
    }
    Ok(None)
}

/// Reject when `path` itself is a symlink or reparse point.
pub fn check_path_chain_safe(path: &Path) -> Result<()> {
    if is_symlink_or_reparse(path)? {
        anyhow::bail!(
            "Unsafe filesystem layout: path '{}' is a symlink or reparse point",
            path.display()
        );
    }
    Ok(())
}

/// Reject when any component on the path from `base` to `path` is a symlink or reparse point.
///
/// Only suffix components under `base` are checked so OS-level symlinks such as macOS
/// `/var` → `/private/var` do not false-positive on temp directories outside nupm home.
pub fn check_path_chain_safe_within(base: &Path, path: &Path) -> Result<()> {
    check_path_chain_safe(base)?;
    if path == base {
        return Ok(());
    }

    let relative = path.strip_prefix(base).with_context(|| {
        format!(
            "Path '{}' is not under base '{}'",
            path.display(),
            base.display()
        )
    })?;

    let mut suffix = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::bail!(
                    "Unsafe filesystem layout: path '{}' contains parent directory components",
                    path.display()
                );
            }
            Component::Normal(name) => {
                suffix.push(name);
                let prefix = base.join(&suffix);
                if is_symlink_or_reparse(&prefix)? {
                    anyhow::bail!(
                        "Unsafe filesystem layout: path '{}' traverses a symlink or reparse point at '{}'",
                        path.display(),
                        prefix.display()
                    );
                }
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn is_regular_file(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.is_file()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => {
            Err(e).with_context(|| format!("Failed to read metadata for '{}'", path.display()))
        }
    }
}

/// Reject symlink/reparse components in `path`, skipping the first normal segment after the
/// volume root so OS-level links such as macOS `/var` → `/private/var` do not false-positive.
fn check_path_prefixes_for_symlinks(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    let mut normal_depth = 0usize;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::bail!(
                    "Unsafe filesystem layout: path '{}' contains parent directory components",
                    path.display()
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                prefix.push(component);
            }
            Component::Normal(name) => {
                normal_depth += 1;
                prefix.push(name);
                if normal_depth > 1 && is_symlink_or_reparse(&prefix)? {
                    anyhow::bail!(
                        "Unsafe filesystem layout: path '{}' traverses a symlink or reparse point at '{}'",
                        path.display(),
                        prefix.display()
                    );
                }
            }
        }
    }
    Ok(())
}

/// Reject symlink/reparse points anywhere under a module payload directory (bounded walk).
pub fn check_module_tree_safe(module_dir: &Path) -> Result<()> {
    check_path_chain_safe(module_dir)?;

    let mut stack = vec![module_dir.to_path_buf()];
    let mut entries_seen = 0usize;

    while let Some(current) = stack.pop() {
        let read_dir = std::fs::read_dir(&current).with_context(|| {
            format!(
                "Failed to read module tree directory '{}'",
                current.display()
            )
        })?;

        for entry in read_dir {
            let entry = entry.with_context(|| {
                format!(
                    "Failed to read module tree entry under '{}'",
                    module_dir.display()
                )
            })?;
            entries_seen += 1;
            if entries_seen > MAX_MODULE_TREE_ENTRIES {
                anyhow::bail!(
                    "Unsafe filesystem layout: module tree '{}' exceeds maximum entry count ({MAX_MODULE_TREE_ENTRIES})",
                    module_dir.display()
                );
            }

            let path = entry.path();
            check_path_chain_safe_within(module_dir, &path)?;

            let file_type = entry.file_type().with_context(|| {
                format!(
                    "Failed to read module tree entry type for '{}'",
                    path.display()
                )
            })?;
            if file_type.is_dir() {
                stack.push(path);
            }
        }
    }

    Ok(())
}

/// Returns true when `name` is one safe path component.
pub fn is_safe_package_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    let path = Path::new(name);
    if path.components().count() != 1 {
        return false;
    }
    matches!(path.components().next(), Some(Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn safe_names() {
        assert!(is_safe_package_name("foo"));
        assert!(!is_safe_package_name("../x"));
        assert!(!is_safe_package_name(""));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_ancestor_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let real_modules = dir.path().join("real_modules");
        fs::create_dir_all(&real_modules).unwrap();
        let modules = dir.path().join("modules");
        std::os::unix::fs::symlink(&real_modules, &modules).unwrap();
        let pkg = modules.join("pkg");
        fs::create_dir_all(&pkg).unwrap();

        let home = dir.path();
        assert!(check_path_chain_safe_within(home, &modules).is_err());
        assert!(check_path_chain_safe_within(home, &pkg).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn find_package_root_rejects_symlink_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let module_dir = real.join("minimal-module");
        fs::create_dir_all(&module_dir).unwrap();
        fs::write(
            real.join("nupm.nuon"),
            br#"{ name: minimal-module, version: "0.1.0", type: module }"#,
        )
        .unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let start = link.join("minimal-module/mod.nu");
        assert!(find_package_root(&start).is_err());
    }
}
