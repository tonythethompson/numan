use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::util::fs_safety::is_symlink_or_reparse;

use super::schema::{MAX_PARENT_WALK_HOPS, METADATA_FILENAME};

/// Walk parents from `start` looking for `nupm.nuon` (bounded).
pub fn find_package_root(start: &Path) -> Result<Option<PathBuf>> {
    let start = start
        .canonicalize()
        .with_context(|| format!("Failed to resolve path '{}'", start.display()))?;

    let mut current = start.clone();
    for _ in 0..MAX_PARENT_WALK_HOPS {
        check_path_chain_safe(&current)?;
        let candidate = current.join(METADATA_FILENAME);
        if candidate.is_file() {
            if is_symlink_or_reparse(&candidate)? {
                anyhow::bail!(
                    "Unsafe filesystem layout: metadata file '{}' is a symlink or reparse point",
                    candidate.display()
                );
            }
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
}
