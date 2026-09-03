//! Self-update from GitHub releases.
//!
//! Checks the latest release version from the GitHub API, downloads
//! the new binary if newer, verifies the checksum, and atomically
//! replaces the current binary.

use std::path::{Path, PathBuf};

use serde::Deserialize;

const GITHUB_API: &str = "https://api.github.com/repos/balaianu/CogZ/releases/latest";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Error during self-update.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("network error: {0}")]
    Network(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("checksum file downloaded but no entry found for asset {0}")]
    ChecksumEntryNotFound(String),
    #[error("no suitable asset found in release")]
    NoAsset,
}

/// GitHub release response (partial).
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Result of a version check.
#[derive(Debug, PartialEq)]
pub enum VersionStatus {
    UpToDate(String),
    UpdateAvailable { current: String, latest: String },
}

/// Check the latest release version from GitHub.
pub fn check_latest_version() -> Result<VersionStatus, UpdateError> {
    let resp = ureq::get(GITHUB_API)
        .header("User-Agent", "cogz-self-update")
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;

    let release: Release = resp
        .into_body()
        .read_json()
        .map_err(|e| UpdateError::Network(e.to_string()))?;

    // Strip leading 'v' from tag name.
    let latest = release.tag_name.trim_start_matches('v').to_string();

    if latest == CURRENT_VERSION {
        Ok(VersionStatus::UpToDate(latest))
    } else {
        Ok(VersionStatus::UpdateAvailable {
            current: CURRENT_VERSION.to_string(),
            latest,
        })
    }
}

/// Detect the current platform's asset name.
fn platform_asset_name() -> &'static str {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    match (os, arch) {
        ("linux", "x86_64") => "cogz-x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "cogz-aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "cogz-x86_64-apple-darwin",
        ("macos", "aarch64") => "cogz-aarch64-apple-darwin",
        _ => "cogz-x86_64-unknown-linux-gnu",
    }
}

/// Find the download URL for the current platform's binary.
fn find_asset_url(release: &Release) -> Option<&str> {
    let target = platform_asset_name();
    release
        .assets
        .iter()
        .find(|a| a.name == target)
        .map(|a| a.browser_download_url.as_str())
}

/// Find the checksum URL.
fn find_checksum_url(release: &Release) -> Option<&str> {
    release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .map(|a| a.browser_download_url.as_str())
}

/// Download a file to a temporary path.
fn download_file(url: &str, dest: &Path) -> Result<(), UpdateError> {
    let resp = ureq::get(url)
        .header("User-Agent", "cogz-self-update")
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;

    let mut file = std::fs::File::create(dest)?;
    let mut reader = resp.into_body().into_reader();
    std::io::copy(&mut reader, &mut file)?;
    file.sync_all()?;
    Ok(())
}

/// Compute SHA-256 of a file.
fn sha256_file(path: &Path) -> Result<String, UpdateError> {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect())
}

/// Parse a SHA256SUMS file and find the checksum for the target asset.
fn find_checksum(sums_content: &str, asset_name: &str) -> Option<String> {
    for line in sums_content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2 && parts[1] == asset_name {
            return Some(parts[0].to_string());
        }
    }
    None
}

/// Get the path to the current binary.
fn current_binary_path() -> Result<PathBuf, UpdateError> {
    std::env::current_exe().map_err(Into::into)
}

/// Run the self-update process.
///
/// 1. Check latest version from GitHub
/// 2. If newer, download the binary and checksum
/// 3. Verify checksum
/// 4. Atomically replace the current binary
pub fn run_update() -> Result<String, UpdateError> {
    let status = check_latest_version()?;

    match status {
        VersionStatus::UpToDate(v) => Ok(format!("CogZ is up to date (v{})", v)),
        VersionStatus::UpdateAvailable { current, latest } => {
            // Fetch release info again to get asset URLs.
            let resp = ureq::get(GITHUB_API)
                .header("User-Agent", "cogz-self-update")
                .call()
                .map_err(|e| UpdateError::Network(e.to_string()))?;
            let release: Release = resp
                .into_body()
                .read_json()
                .map_err(|e| UpdateError::Network(e.to_string()))?;

            let asset_name = platform_asset_name();
            let asset_url = find_asset_url(&release).ok_or(UpdateError::NoAsset)?;

            // Download to a temp file. Include PID to avoid collisions
            // between concurrent update processes.
            let temp_dir = std::env::temp_dir();
            let pid = std::process::id();
            let temp_binary = temp_dir.join(format!("cogz-update-{latest}-{pid}"));
            let temp_checksum = temp_dir.join(format!("cogz-update-{latest}-{pid}.sha256"));

            download_file(asset_url, &temp_binary)?;

            // Download and verify checksum if available.
            if let Some(checksum_url) = find_checksum_url(&release) {
                download_file(checksum_url, &temp_checksum)?;
                let sums_content = std::fs::read_to_string(&temp_checksum)?;
                let expected_hash = find_checksum(&sums_content, asset_name).ok_or_else(|| {
                    let _ = std::fs::remove_file(&temp_binary);
                    let _ = std::fs::remove_file(&temp_checksum);
                    UpdateError::ChecksumEntryNotFound(asset_name.to_string())
                })?;
                let actual_hash = sha256_file(&temp_binary)?;
                if expected_hash != actual_hash {
                    let _ = std::fs::remove_file(&temp_binary);
                    let _ = std::fs::remove_file(&temp_checksum);
                    return Err(UpdateError::ChecksumMismatch {
                        expected: expected_hash,
                        actual: actual_hash,
                    });
                }
            }

            // Make the new binary executable.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&temp_binary)?.permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&temp_binary, perms)?;
            }

            // Atomically replace the current binary.
            let bin_path = current_binary_path()?;
            let backup = bin_path.with_extension("bak");

            // Backup current binary.
            if bin_path.exists() {
                let _ = std::fs::copy(&bin_path, &backup);
            }

            // Replace with the new binary.
            if let Err(e) = std::fs::rename(&temp_binary, &bin_path) {
                // rename can fail across filesystems — fall back to copy + remove.
                std::fs::copy(&temp_binary, &bin_path)?;
                let _ = std::fs::remove_file(&temp_binary);
                tracing::debug!("rename failed, used copy: {}", e);
            }

            // Clean up.
            let _ = std::fs::remove_file(&temp_checksum);
            let _ = std::fs::remove_file(&backup);

            Ok(format!("Updated CogZ from v{} to v{}", current, latest))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_checksum_matches_correct_asset() {
        let sums =
            "abc123  cogz-x86_64-unknown-linux-gnu\ndef456  cogz-aarch64-unknown-linux-gnu\n";
        assert_eq!(
            find_checksum(sums, "cogz-x86_64-unknown-linux-gnu"),
            Some("abc123".to_string())
        );
        assert_eq!(
            find_checksum(sums, "cogz-aarch64-unknown-linux-gnu"),
            Some("def456".to_string())
        );
    }

    #[test]
    fn find_checksum_returns_none_for_missing_asset() {
        let sums = "abc123  cogz-x86_64-unknown-linux-gnu\n";
        assert_eq!(find_checksum(sums, "cogz-windows-x86_64.exe"), None);
    }

    #[test]
    fn find_checksum_handles_empty_file() {
        assert_eq!(find_checksum("", "any-asset"), None);
    }

    #[test]
    fn find_checksum_handles_malformed_lines() {
        let sums = "not-a-valid-line\nabc123  cogz-x86_64-unknown-linux-gnu\n";
        assert_eq!(
            find_checksum(sums, "cogz-x86_64-unknown-linux-gnu"),
            Some("abc123".to_string())
        );
    }
}
