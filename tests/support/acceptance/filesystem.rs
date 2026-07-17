use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::model::{
    utc_unix_ms, InventoryEntry, InventoryKind, PackageDirectoryEvidence, PayloadReference,
    StateEvidence, StateFileEvidence, EVIDENCE_SCHEMA_VERSION,
};
use super::process::streaming_sha256;

const INLINE_LIMIT: u64 = 1024 * 1024;

pub fn inventory_root(root: &Path) -> Result<Vec<InventoryEntry>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut inventory = Vec::new();
    walk_inventory(root, root, &mut inventory)?;
    inventory.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(inventory)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("failed to open {} for hashing", path.display()))?;
    streaming_sha256(file).map(|(hash, _)| hash)
}

pub fn discover_journals(root: &Path) -> Result<Vec<PathBuf>> {
    let state = root.join("state");
    if !state.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    discover_matching_files(&state, &mut paths, &|name| {
        (name.starts_with("pending-") && name.ends_with(".json"))
            || name.to_ascii_lowercase().contains("journal")
    })?;
    paths.sort_by_key(|path| normalize_relative(root, path));
    Ok(paths)
}

pub fn capture_state(root: &Path, run_dir: &Path, inventory: &[InventoryEntry]) -> StateEvidence {
    capture_state_result(root, run_dir, inventory).unwrap_or_else(|error| StateEvidence {
        schema_version: EVIDENCE_SCHEMA_VERSION,
        captured_utc_ms: utc_unix_ms(),
        files: vec![StateFileEvidence {
            path: "<capture-error>".to_string(),
            exists: false,
            size: None,
            sha256: None,
            parsed: None,
            text: Some(error.to_string()),
        }],
        journals: Vec::new(),
    })
}

pub fn path_is_contained(path: &Path, boundary: &Path) -> Result<bool> {
    let boundary = canonicalize_allow_missing(boundary)?;
    let candidate = canonicalize_allow_missing(path)?;
    Ok(path_starts_with(&candidate, &boundary))
}

pub fn classify_package_dirs(root: &Path) -> Result<Vec<PackageDirectoryEvidence>> {
    let mut references: BTreeMap<String, Vec<PayloadReference>> = BTreeMap::new();
    collect_lockfile_references(
        root,
        &root.join("lockfile"),
        "current_lockfile",
        &mut references,
    )?;

    let snapshots = root.join("state/snapshots");
    if snapshots.exists() {
        let mut lockfiles = Vec::new();
        discover_matching_files(&snapshots, &mut lockfiles, &|name| name == "lockfile.json")?;
        lockfiles.sort();
        for lockfile in lockfiles {
            let snapshot_id = lockfile
                .parent()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".to_string());
            collect_lockfile_references(
                root,
                &lockfile,
                &format!("snapshot:{snapshot_id}"),
                &mut references,
            )?;
        }
    }

    for journal in discover_journals(root)? {
        let content = std::fs::read_to_string(&journal)?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
            let label = format!("journal:{}", normalize_relative(root, &journal));
            let mut payloads = Vec::new();
            collect_payload_fields(&value, None, &mut payloads);
            for (package_id, payload) in payloads {
                references
                    .entry(normalize_payload(&payload))
                    .or_default()
                    .push(PayloadReference {
                        source: label.clone(),
                        package_id,
                    });
            }
        }
    }

    let mut directories = Vec::new();
    for directory in package_version_dirs(&root.join("packages")) {
        let relative = normalize_relative(root, &directory);
        let mut labels = references.remove(&relative).unwrap_or_default();
        labels.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then(left.package_id.cmp(&right.package_id))
        });
        labels.dedup();
        directories.push(PackageDirectoryEvidence {
            path: relative,
            orphan: labels.is_empty(),
            references: labels,
        });
    }
    directories.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(directories)
}

fn walk_inventory(root: &Path, directory: &Path, result: &mut Vec<InventoryEntry>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(directory)
        .with_context(|| format!("failed to read directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        let (kind, sha256, target) = if file_type.is_symlink() {
            (
                InventoryKind::Symlink,
                None,
                std::fs::read_link(&path)
                    .ok()
                    .map(|target| normalize_path(&target)),
            )
        } else if file_type.is_file() {
            (InventoryKind::File, Some(sha256_file(&path)?), None)
        } else if file_type.is_dir() {
            (InventoryKind::Directory, None, None)
        } else {
            (InventoryKind::Other, None, None)
        };
        result.push(InventoryEntry {
            path: normalize_relative(root, &path),
            kind,
            size: metadata.len(),
            sha256,
            symlink_target: target,
        });
        if file_type.is_dir() && !file_type.is_symlink() {
            walk_inventory(root, &path, result)?;
        }
    }
    Ok(())
}

fn capture_state_result(
    root: &Path,
    run_dir: &Path,
    inventory: &[InventoryEntry],
) -> Result<StateEvidence> {
    let inventory_by_path: BTreeMap<_, _> = inventory
        .iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect();
    let mut wanted = BTreeSet::from([
        "lockfile".to_string(),
        "config.toml".to_string(),
        "nu_state/paths.json".to_string(),
        "state/autoload-state.json".to_string(),
        "state/pending-activation.json".to_string(),
        "state/pending-autoload.json".to_string(),
        "state/pending-lifecycle.json".to_string(),
    ]);
    for entry in inventory {
        if entry.kind == InventoryKind::File
            && (entry.path.starts_with("registry/")
                || entry.path.starts_with("state/snapshots/")
                || entry.path.contains("activation")
                || entry.path.contains("autoload")
                || entry.path.contains("journal"))
        {
            wanted.insert(entry.path.clone());
        }
    }
    for journal in discover_journals(root)? {
        wanted.insert(normalize_relative(root, &journal));
    }

    let mut files = Vec::new();
    for relative in wanted {
        let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        files.push(capture_file(
            &path,
            &relative,
            inventory_by_path.get(&relative).copied(),
        )?);
    }

    let paths_file = root.join("nu_state/paths.json");
    if paths_file.exists() {
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&paths_file)?)?;
        if let Some(plugin_registry) = value
            .get("plugin_registry_path")
            .and_then(serde_json::Value::as_str)
        {
            let path = PathBuf::from(plugin_registry);
            let label = if path_is_contained(&path, run_dir)? {
                format!("run:{}", normalize_relative(run_dir, &path))
            } else {
                format!("external:{}", normalize_path(&path))
            };
            files.push(capture_file(&path, &label, None)?);
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(StateEvidence {
        schema_version: EVIDENCE_SCHEMA_VERSION,
        captured_utc_ms: utc_unix_ms(),
        files,
        journals: discover_journals(root)?
            .iter()
            .map(|path| normalize_relative(root, path))
            .collect(),
    })
}

fn capture_file(
    path: &Path,
    label: &str,
    inventory: Option<&InventoryEntry>,
) -> Result<StateFileEvidence> {
    if !path.exists() {
        return Ok(StateFileEvidence {
            path: label.to_string(),
            exists: false,
            size: None,
            sha256: None,
            parsed: None,
            text: None,
        });
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Ok(StateFileEvidence {
            path: label.to_string(),
            exists: true,
            size: Some(metadata.len()),
            sha256: None,
            parsed: None,
            text: None,
        });
    }
    let hash = inventory
        .and_then(|entry| entry.sha256.clone())
        .map(Ok)
        .unwrap_or_else(|| sha256_file(path))?;
    let mut parsed = None;
    let mut text = None;
    if metadata.len() <= INLINE_LIMIT && is_text_state(label) {
        let content = std::fs::read_to_string(path)?;
        if label.ends_with(".toml") {
            let value: toml::Value = toml::from_str(&content)?;
            parsed = Some(serde_json::to_value(value)?);
        } else if label.contains(".json") || label == "lockfile" {
            parsed = serde_json::from_str(&content).ok();
        }
        text = Some(content);
    }
    Ok(StateFileEvidence {
        path: label.to_string(),
        exists: true,
        size: Some(metadata.len()),
        sha256: Some(hash),
        parsed,
        text,
    })
}

fn is_text_state(label: &str) -> bool {
    label.ends_with(".json")
        || label.ends_with(".toml")
        || label.ends_with(".txt")
        || label.contains(".json.")
        || label == "lockfile"
}

fn discover_matching_files(
    directory: &Path,
    result: &mut Vec<PathBuf>,
    predicate: &impl Fn(&str) -> bool,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            discover_matching_files(&path, result, predicate)?;
        } else if metadata.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if predicate(&name) {
                result.push(path);
            }
        }
    }
    Ok(())
}

fn collect_lockfile_references(
    root: &Path,
    path: &Path,
    label: &str,
    result: &mut BTreeMap<String, Vec<PayloadReference>>,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    if let Some(packages) = value.get("packages").and_then(serde_json::Value::as_object) {
        for (package_id, entry) in packages {
            if let Some(payload) = entry
                .get("payload_path")
                .and_then(serde_json::Value::as_str)
            {
                let relative = normalize_payload(payload);
                if !relative.is_empty() {
                    let candidate = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
                    if path_is_contained(&candidate, root)? {
                        result.entry(relative).or_default().push(PayloadReference {
                            source: label.to_string(),
                            package_id: Some(package_id.clone()),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

fn collect_payload_fields(
    value: &serde_json::Value,
    package_id: Option<String>,
    result: &mut Vec<(Option<String>, String)>,
) {
    match value {
        serde_json::Value::Object(object) => {
            let package_id = object
                .get("package_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or(package_id);
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "payload_path" | "orphan_payload_path" | "promoted_payload_path"
                ) {
                    if let Some(path) = value.as_str() {
                        result.push((package_id.clone(), path.to_string()));
                    }
                }
                collect_payload_fields(value, package_id.clone(), result);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_payload_fields(value, package_id.clone(), result);
            }
        }
        _ => {}
    }
}

fn package_version_dirs(packages: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for type_dir in read_dirs(packages) {
        for owner_dir in read_dirs(&type_dir) {
            for name_dir in read_dirs(&owner_dir) {
                for version_dir in read_dirs(&name_dir) {
                    if std::fs::symlink_metadata(&version_dir)
                        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                        .unwrap_or(false)
                    {
                        result.push(version_dir);
                    }
                }
            }
        }
    }
    result
}

fn read_dirs(path: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths
}

fn canonicalize_allow_missing(path: &Path) -> Result<PathBuf> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("path traversal is not allowed: {}", path.display());
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut cursor = absolute.as_path();
    let mut missing = Vec::new();
    while !cursor.exists() {
        let name = cursor
            .file_name()
            .context("cannot resolve a nonexisting filesystem root")?;
        missing.push(name.to_os_string());
        cursor = cursor
            .parent()
            .context("cannot resolve a nonexisting filesystem root")?;
    }
    let mut resolved = std::fs::canonicalize(cursor)?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn path_starts_with(path: &Path, boundary: &Path) -> bool {
    if cfg!(windows) {
        normalize_path(path)
            .to_ascii_lowercase()
            .starts_with(&(normalize_path(boundary).to_ascii_lowercase() + "/"))
            || normalize_path(path).eq_ignore_ascii_case(&normalize_path(boundary))
    } else {
        path.starts_with(boundary)
    }
}

fn normalize_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(normalize_path)
        .unwrap_or_else(|_| normalize_path(path))
}

fn normalize_payload(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
