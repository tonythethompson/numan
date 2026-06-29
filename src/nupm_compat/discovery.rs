use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::classify::classify_source_root;
use super::report::{InstalledOnlyEntry, SourceRootEntry};
use super::schema::MAX_DISCOVERY_ENTRIES;
use super::walk::{check_path_chain_safe, check_path_chain_safe_within, is_safe_package_name};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NupmHomeResolution {
    Found(PathBuf),
    NotConfigured,
}

pub fn resolve_nupm_home(flag: Option<&Path>) -> Result<NupmHomeResolution> {
    if let Some(p) = flag {
        if !p.exists() {
            bail!(
                "nupm home '{}' does not exist.\n\
                 Pass --nupm-home <path> or set NUPM_HOME.",
                p.display()
            );
        }
        return Ok(NupmHomeResolution::Found(p.to_path_buf()));
    }

    if let Some(env) = std::env::var_os("NUPM_HOME") {
        let p = PathBuf::from(env);
        if !p.exists() {
            bail!(
                "NUPM_HOME '{}' does not exist.\n\
                 Fix the environment variable or pass --nupm-home <path>.",
                p.display()
            );
        }
        return Ok(NupmHomeResolution::Found(p));
    }

    Ok(NupmHomeResolution::NotConfigured)
}

pub struct ScanResult {
    pub source_roots: Vec<SourceRootEntry>,
    pub installed_only: Vec<InstalledOnlyEntry>,
    pub script_entries: usize,
    pub unsafe_entries: usize,
}

pub fn scan_nupm_home(nupm_home: &Path) -> Result<ScanResult> {
    check_path_chain_safe(nupm_home)?;

    let mut source_roots = Vec::new();
    let mut installed_only = Vec::new();
    let mut script_entries = 0usize;
    let mut unsafe_entries = 0usize;
    let mut seen = HashSet::new();
    let mut total = 0usize;

    let modules_dir = nupm_home.join("modules");
    if modules_dir.is_dir() {
        check_path_chain_safe_within(nupm_home, &modules_dir)?;
        for entry in std::fs::read_dir(&modules_dir)
            .with_context(|| format!("Failed to read '{}'", modules_dir.display()))?
        {
            let entry = entry?;
            total += 1;
            if total > MAX_DISCOVERY_ENTRIES {
                bail!("nupm home exceeds maximum discovery entry count ({MAX_DISCOVERY_ENTRIES})");
            }

            let path = entry.path();
            let key = normalize_key(&path);
            if !seen.insert(key) {
                continue;
            }

            if check_path_chain_safe_within(nupm_home, &path).is_err() {
                unsafe_entries += 1;
                continue;
            }

            let metadata = path.join(super::schema::METADATA_FILENAME);
            if metadata.is_file() {
                match classify_source_root(&path) {
                    Ok((compat, meta)) => {
                        source_roots.push(SourceRootEntry {
                            source_path: path.clone(),
                            compatibility: compat,
                            metadata: meta,
                        });
                    }
                    Err(_) => unsafe_entries += 1,
                }
            } else if path.is_dir() {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                if is_safe_package_name(&name) {
                    installed_only.push(InstalledOnlyEntry { name, path });
                } else {
                    unsafe_entries += 1;
                }
            }
        }
    }

    let scripts_dir = nupm_home.join("scripts");
    if scripts_dir.is_dir() {
        check_path_chain_safe_within(nupm_home, &scripts_dir)?;
        for entry in std::fs::read_dir(&scripts_dir)
            .with_context(|| format!("Failed to read '{}'", scripts_dir.display()))?
        {
            let entry = entry?;
            total += 1;
            if total > MAX_DISCOVERY_ENTRIES {
                bail!("nupm home exceeds maximum discovery entry count ({MAX_DISCOVERY_ENTRIES})");
            }
            let path = entry.path();
            if path.is_file() && check_path_chain_safe_within(nupm_home, &path).is_ok() {
                script_entries += 1;
            } else {
                unsafe_entries += 1;
            }
        }
    }

    Ok(ScanResult {
        source_roots,
        installed_only,
        script_entries,
        unsafe_entries,
    })
}

fn normalize_key(path: &Path) -> OsString {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .as_os_str()
        .to_os_string()
}

pub fn inspect_path(path: &Path) -> Result<SourceRootEntry> {
    let root = super::walk::find_package_root(path)?
        .with_context(|| format!("No nupm.nuon found for path '{}'", path.display()))?;
    let (compat, meta) = classify_source_root(&root)?;
    Ok(SourceRootEntry {
        source_path: root,
        compatibility: compat,
        metadata: meta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nupm_compat::NupmCompatibility;

    fn layout_home() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nupm/nupm-home-layout")
    }

    #[test]
    fn t13_scan_finds_installed_only() {
        let scan = scan_nupm_home(&layout_home()).unwrap();
        assert_eq!(scan.installed_only.len(), 1);
        assert!(scan
            .source_roots
            .iter()
            .all(|e| e.compatibility != NupmCompatibility::ImportableModule));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_modules_dir_rejects_scan() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let real_modules = home.join("real_modules");
        std::fs::create_dir_all(&real_modules).unwrap();
        let modules = home.join("modules");
        std::os::unix::fs::symlink(&real_modules, &modules).unwrap();
        assert!(scan_nupm_home(home).is_err());
    }
}
