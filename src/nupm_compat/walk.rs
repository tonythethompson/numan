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

/// Reject paths that traverse a symlink or reparse point at any prefix component.
pub fn check_path_chain_safe(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
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
            Component::Normal(_) => {
                prefix.push(component);
                if is_symlink_or_reparse(&prefix)? {
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

        assert!(check_path_chain_safe(&modules).is_err());
        assert!(check_path_chain_safe(&pkg).is_err());
    }
}
