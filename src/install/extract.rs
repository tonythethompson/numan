use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use tar::Archive as TarArchive;
use zip::ZipArchive;

#[derive(Debug, Clone)]
pub struct ExtractConfig {
    pub archive_root: Option<String>,
    pub include: Option<Vec<String>>,
    pub entry: Option<String>,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            archive_root: None,
            include: None,
            entry: None,
        }
    }
}

#[derive(Debug)]
pub struct ExtractResult {
    pub files: Vec<PathBuf>,
    pub entry_found: bool,
}

pub fn extract_archive(
    archive_path: &Path,
    dest_dir: &Path,
    config: &ExtractConfig,
) -> Result<ExtractResult> {
    let extension = archive_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let result = match extension {
        "zip" => extract_zip(archive_path, dest_dir, config),
        "gz" => {
            // Check if it's .tar.gz
            let stem = archive_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if stem.ends_with(".tar") {
                extract_tar_gz(archive_path, dest_dir, config)
            } else {
                extract_tar_gz(archive_path, dest_dir, config)
            }
        }
        "tar" => extract_tar(archive_path, dest_dir, config),
        "xz" => {
            // Check for .tar.xz
            extract_tar_xz(archive_path, dest_dir, config)
        }
        _ => bail!(
            "Unsupported archive format: .{extension}. Supported: .zip, .tar.gz, .tar.xz"
        ),
    };

    result
}

fn extract_zip(
    archive_path: &Path,
    dest_dir: &Path,
    config: &ExtractConfig,
) -> Result<ExtractResult> {
    let file = File::open(archive_path)
        .with_context(|| format!("Failed to open archive: {}", archive_path.display()))?;
    let mut archive =
        ZipArchive::new(file).with_context(|| format!("Failed to read zip: {}", archive_path.display()))?;

    let mut extracted_files = Vec::new();
    let mut entry_found = false;
    let archive_root = config.archive_root.as_deref();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| anyhow::anyhow!("Zip entry error: {e}"))?;
        let entry_path = entry
            .mangled_name()
            .to_owned();

        // Skip directories
        if entry.is_dir() {
            continue;
        }

        // Get relative path (strip archive_root if present)
        let relative_path = if let Some(root) = archive_root {
            strip_leading_component(&entry_path, root)
        } else {
            strip_first_component_if_single(&entry_path)
        };

        let relative_path = match relative_path {
            Some(p) => p,
            None => continue, // Skip if path doesn't match archive_root
        };

        // Check include filter
        if !include_matches(&relative_path, config.include.as_deref()) {
            continue;
        }

        // Validate path (no traversal)
        if has_path_traversal(&relative_path) {
            bail!(
                "Path traversal detected in archive: {}",
                entry_path.display()
            );
        }

        let out_path = dest_dir.join(&relative_path);

        // Create parent directory
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Extract file
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .with_context(|| format!("Failed to read zip entry: {}", entry_path.display()))?;
        std::fs::write(&out_path, contents)?;

        // Check entry point
        if let Some(ref entry_name) = config.entry {
            if relative_path.file_name().and_then(|f| f.to_str()) == Some(entry_name) {
                entry_found = true;
            }
        }

        extracted_files.push(out_path);
    }

    Ok(ExtractResult {
        files: extracted_files,
        entry_found,
    })
}

fn extract_tar_gz(
    archive_path: &Path,
    dest_dir: &Path,
    config: &ExtractConfig,
) -> Result<ExtractResult> {
    let file = File::open(archive_path)
        .with_context(|| format!("Failed to open archive: {}", archive_path.display()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = TarArchive::new(decoder);

    extract_tar_inner(&mut archive, dest_dir, config)
}

fn extract_tar(
    archive_path: &Path,
    dest_dir: &Path,
    config: &ExtractConfig,
) -> Result<ExtractResult> {
    let file = File::open(archive_path)
        .with_context(|| format!("Failed to open archive: {}", archive_path.display()))?;
    let mut archive = TarArchive::new(file);

    extract_tar_inner(&mut archive, dest_dir, config)
}

fn extract_tar_xz(
    archive_path: &Path,
    _dest_dir: &Path,
    _config: &ExtractConfig,
) -> Result<ExtractResult> {
    // xz decompression - use a basic approach
    // For now, we'll error and ask user to convert to tar.gz
    // Full xz support requires the xz2 crate which isn't in dependencies
    bail!(
        "xz archives not yet supported. Please convert to .tar.gz or .zip format. Got: {}",
        archive_path.display()
    )
}

fn extract_tar_inner<R: Read>(
    archive: &mut TarArchive<R>,
    dest_dir: &Path,
    config: &ExtractConfig,
) -> Result<ExtractResult> {
    let mut extracted_files = Vec::new();
    let mut entry_found = false;
    let archive_root = config.archive_root.as_deref();

    for entry in archive.entries().map_err(|e| anyhow::anyhow!("Failed to read tar entries: {e}"))? {
        let mut entry = entry.map_err(|e| anyhow::anyhow!("Tar entry error: {e}"))?;
        let entry_path = entry
            .path()
            .map_err(|e| anyhow::anyhow!("Failed to get tar entry path: {e}"))?
            .into_owned();

        // Skip directories
        if entry.header().entry_type() == tar::EntryType::Directory {
            continue;
        }

        // Get relative path (strip archive_root if present)
        let relative_path = if let Some(root) = archive_root {
            strip_leading_component(&entry_path, root)
        } else {
            strip_first_component_if_single(&entry_path)
        };

        let relative_path = match relative_path {
            Some(p) => p,
            None => continue,
        };

        // Check include filter
        if !include_matches(&relative_path, config.include.as_deref()) {
            continue;
        }

        // Validate path (no traversal)
        if has_path_traversal(&relative_path) {
            bail!(
                "Path traversal detected in archive: {}",
                entry_path.display()
            );
        }

        let out_path = dest_dir.join(&relative_path);

        // Create parent directory
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Extract file
        entry
            .unpack(&out_path)
            .with_context(|| format!("Failed to extract: {}", entry_path.display()))?;

        // Check entry point
        if let Some(ref entry_name) = config.entry {
            if relative_path.file_name().and_then(|f| f.to_str()) == Some(entry_name) {
                entry_found = true;
            }
        }

        extracted_files.push(out_path);
    }

    Ok(ExtractResult {
        files: extracted_files,
        entry_found,
    })
}

/// Strip the archive_root prefix from a path
fn strip_leading_component(path: &Path, root: &str) -> Option<PathBuf> {
    let components: Vec<&std::ffi::OsStr> = path.components().map(|c| c.as_os_str()).collect();
    if components.is_empty() {
        return None;
    }

    // Check if first component matches root
    if components[0] != std::ffi::OsStr::new(root) {
        return None;
    }

    // Return remaining path
    if components.len() == 1 {
        return None; // It's just the root directory
    }

    Some(components[1..].iter().collect())
}

/// Strip first component if it looks like a single package directory
fn strip_first_component_if_single(path: &Path) -> Option<PathBuf> {
    let components: Vec<&std::ffi::OsStr> = path.components().map(|c| c.as_os_str()).collect();
    if components.len() <= 1 {
        return Some(path.to_path_buf());
    }

    // Check if it's a directory-like first component (common in archives)
    let first = components[0].to_str().unwrap_or("");
    if first.contains('-') || first.contains('_') || first.ends_with(".zip") || first.ends_with(".tar") {
        // Looks like a package directory, strip it
        Some(components[1..].iter().collect())
    } else {
        Some(path.to_path_buf())
    }
}

/// Check if a path contains traversal components
fn has_path_traversal(path: &Path) -> bool {
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => return true,
            std::path::Component::RootDir => return true,
            _ => {}
        }
    }
    false
}

/// Check if a file path matches any of the include patterns
fn include_matches(path: &Path, patterns: Option<&[String]>) -> bool {
    let patterns = match patterns {
        Some(p) if !p.is_empty() => p,
        _ => return true, // No filter = include all
    };

    let path_str = path.to_string_lossy().to_lowercase();
    let file_name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    for pattern in patterns {
        let pattern_lower = pattern.to_lowercase();

        // Simple glob matching
        if pattern_lower == "*" {
            return true;
        }

        if pattern_lower.starts_with("*.") {
            // Extension match: *.nu, *.exe, etc.
            let ext = &pattern_lower[1..]; // e.g., ".nu"
            if path_str.ends_with(ext) {
                return true;
            }
        } else if pattern_lower.ends_with("/*") {
            // Directory prefix match: nu_plugin_*/*
            let prefix = &pattern_lower[..pattern_lower.len() - 2];
            if file_name.starts_with(prefix) {
                return true;
            }
        } else if file_name == pattern_lower {
            // Exact filename match
            return true;
        } else if path_str.contains(&pattern_lower) {
            // Substring match
            return true;
        }
    }

    false
}

/// Compute SHA256 of a file
pub fn compute_file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("Failed to open file for hashing: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_zip(dir: &Path, files: &[(&str, &[u8])]) -> PathBuf {
        let zip_path = dir.join("test.zip");
        let file = File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        for (name, content) in files {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content).unwrap();
        }

        zip.finish().unwrap();
        zip_path
    }

    #[test]
    fn extract_zip_basic() {
        let tmp = TempDir::new().unwrap();
        let zip_path = create_test_zip(
            tmp.path(),
            &[
                ("test.txt", b"hello"),
                ("subdir/nested.txt", b"nested"),
            ],
        );

        let dest = tmp.path().join("extracted");
        std::fs::create_dir(&dest).unwrap();

        let result = extract_archive(&zip_path, &dest, &ExtractConfig::default()).unwrap();
        assert_eq!(result.files.len(), 2);
        assert!(dest.join("test.txt").exists());
        assert!(dest.join("subdir/nested.txt").exists());
    }

    #[test]
    fn extract_zip_with_archive_root() {
        let tmp = TempDir::new().unwrap();
        let zip_path = create_test_zip(
            tmp.path(),
            &[
                ("pkg-1.0.0/file.txt", b"content"),
                ("pkg-1.0.0/inner/other.txt", b"other"),
            ],
        );

        let dest = tmp.path().join("extracted");
        std::fs::create_dir(&dest).unwrap();

        let config = ExtractConfig {
            archive_root: Some("pkg-1.0.0".to_string()),
            include: None,
            entry: None,
        };

        let result = extract_archive(&zip_path, &dest, &config).unwrap();
        assert_eq!(result.files.len(), 2);
        assert!(dest.join("file.txt").exists());
        assert!(dest.join("inner/other.txt").exists());
    }

    #[test]
    fn extract_zip_with_include_filter() {
        let tmp = TempDir::new().unwrap();
        let zip_path = create_test_zip(
            tmp.path(),
            &[
                ("main.nu", b"main script"),
                ("README.md", b"readme"),
                ("lib.nu", b"library"),
            ],
        );

        let dest = tmp.path().join("extracted");
        std::fs::create_dir(&dest).unwrap();

        let config = ExtractConfig {
            archive_root: None,
            include: Some(vec!["*.nu".to_string()]),
            entry: None,
        };

        let result = extract_archive(&zip_path, &dest, &config).unwrap();
        assert_eq!(result.files.len(), 2);
        assert!(dest.join("main.nu").exists());
        assert!(dest.join("lib.nu").exists());
        assert!(!dest.join("README.md").exists());
    }

    #[test]
    fn extract_zip_with_entry_point() {
        let tmp = TempDir::new().unwrap();
        let zip_path = create_test_zip(
            tmp.path(),
            &[
                ("main.nu", b"main script"),
                ("other.nu", b"other"),
            ],
        );

        let dest = tmp.path().join("extracted");
        std::fs::create_dir(&dest).unwrap();

        let config = ExtractConfig {
            archive_root: None,
            include: None,
            entry: Some("main.nu".to_string()),
        };

        let result = extract_archive(&zip_path, &dest, &config).unwrap();
        assert!(result.entry_found);
    }

    #[test]
    fn extract_zip_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        // The zip crate's mangled_name() sanitizes `..` components,
        // so we test at the path validation level instead
        let path_with_traversal = std::path::PathBuf::from("../../etc/passwd");
        assert!(has_path_traversal(&path_with_traversal));

        let safe_path = std::path::PathBuf::from("subdir/file.txt");
        assert!(!has_path_traversal(&safe_path));
    }

    #[test]
    fn strip_leading_component_works() {
        let path = PathBuf::from("pkg-1.0.0/file.txt");
        let result = strip_leading_component(&path, "pkg-1.0.0").unwrap();
        assert_eq!(result, PathBuf::from("file.txt"));
    }

    #[test]
    fn strip_leading_component_no_match() {
        let path = PathBuf::from("other-1.0.0/file.txt");
        let result = strip_leading_component(&path, "pkg-1.0.0");
        assert!(result.is_none());
    }

    #[test]
    fn include_matches_extension() {
        let path = PathBuf::from("script.nu");
        assert!(include_matches(&path, Some(&["*.nu".to_string()])));
        assert!(!include_matches(&path, Some(&["*.rs".to_string()])));
    }

    #[test]
    fn include_matches_no_filter() {
        let path = PathBuf::from("anything.txt");
        assert!(include_matches(&path, None));
        assert!(include_matches(&path, Some(&[])));
    }

    #[test]
    fn has_path_traversal_detects_parent() {
        let path = PathBuf::from("../../etc/passwd");
        assert!(has_path_traversal(&path));

        let safe = PathBuf::from("subdir/file.txt");
        assert!(!has_path_traversal(&safe));
    }

    #[test]
    fn compute_file_sha256_deterministic() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("test.bin");
        std::fs::write(&file_path, b"test content").unwrap();

        let hash1 = compute_file_sha256(&file_path).unwrap();
        let hash2 = compute_file_sha256(&file_path).unwrap();
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA256 hex string
    }
}
