//! ONNX Runtime library discovery and auto-download.
//!
//! The `ort` crate with `load-dynamic` requires `libonnxruntime.so`
//! at runtime. This module handles:
//!
//! 1. Checking if the library is already available (system install,
//!    `ORT_DYLIB_PATH`, or previously downloaded to cogz's lib dir).
//! 2. Downloading the CPU-only ONNX Runtime (~11MB) from GitHub
//!    releases to `~/.local/share/cogz/lib/` if not found.
//! 3. Initializing `ort` with the discovered path via `ort::init_from()`.
//!
//! This makes the out-of-box experience seamless: no system package
//! installation, no environment variables, no manual symlinks.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tracing::{info, warn};

/// ONNX Runtime version to download. ort 2.0.0-rc.13 supports ORT 1.28;
/// we use 1.27.0 to match the version FastEmbed ships, ensuring
/// comparable inference performance.
const ORT_VERSION: &str = "1.27.0";

/// GitHub release asset name for Linux x86_64 CPU-only.
#[cfg(target_os = "linux")]
#[cfg(target_arch = "x86_64")]
const ORT_ASSET: &str = "onnxruntime-linux-x64-1.27.0.tgz";

/// The shared library name inside the extracted archive.
#[cfg(target_os = "linux")]
const ORT_LIB_NAME: &str = "libonnxruntime.so";

/// Static init guard — ensures `ort::init_from` is called exactly once.
static ORT_INIT: OnceLock<bool> = OnceLock::new();

/// Get the cogz lib directory: `~/.local/share/cogz/lib/`.
fn cogz_lib_dir() -> PathBuf {
    super::models_dir()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("lib")
}

/// The expected path for the downloaded ONNX Runtime library.
pub fn ort_lib_path() -> PathBuf {
    cogz_lib_dir().join(ORT_LIB_NAME)
}

/// Check if the ONNX Runtime library is available at a given path.
fn lib_exists(path: &Path) -> bool {
    path.exists() && path.is_file()
}

/// Try to find an existing ONNX Runtime library. Checks:
/// 1. `ORT_DYLIB_PATH` env var (user override)
/// 2. cogz's lib dir (previously downloaded)
/// 3. System library paths (via dlopen name resolution)
fn find_existing_lib() -> Option<PathBuf> {
    // 1. User override via env var
    if let Ok(path) = std::env::var("ORT_DYLIB_PATH")
        && !path.is_empty()
        && lib_exists(Path::new(&path))
    {
        return Some(PathBuf::from(path));
    }

    // 2. Previously downloaded to cogz lib dir
    let cogz_path = ort_lib_path();
    if lib_exists(&cogz_path) {
        return Some(cogz_path);
    }

    // 3. System library — check common paths and versioned names.
    // ort 2.0.0-rc.13 supports ORT 1.28; accepts older versions with
    // a warning. We prefer the cogz-managed copy (checked above) but
    // fall back to any system installation.
    for candidate in [
        "/usr/lib/x86_64-linux-gnu/libonnxruntime.so",
        "/usr/lib/x86_64-linux-gnu/libonnxruntime.so.1",
        "/usr/lib/x86_64-linux-gnu/libonnxruntime.so.1.23",
        "/usr/local/lib/libonnxruntime.so",
        "/usr/lib/libonnxruntime.so",
    ] {
        if lib_exists(Path::new(candidate)) {
            return Some(PathBuf::from(candidate));
        }
    }

    None
}

/// Download and extract the ONNX Runtime library to cogz's lib dir.
/// Downloads the CPU-only Linux x86_64 build (~11MB) from GitHub releases.
/// Verifies the SHA-256 checksum from the release's SHA256SUMS file before
/// extracting.
fn download_ort() -> Result<PathBuf, std::io::Error> {
    let lib_dir = cogz_lib_dir();
    std::fs::create_dir_all(&lib_dir)?;

    let lib_path = ort_lib_path();
    if lib_exists(&lib_path) {
        return Ok(lib_path);
    }

    let url = format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{}/{}",
        ORT_VERSION, ORT_ASSET
    );

    info!("downloading ONNX Runtime {} from GitHub", ORT_VERSION);

    // Use a temp directory under the system temp dir (no tempfile dependency).
    let temp_dir = std::env::temp_dir().join(format!("cogz-ort-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir)?;

    let archive_path = temp_dir.join("onnxruntime.tgz");

    // Download the .tgz archive using ureq (already a dependency).
    let response = ureq::get(&url)
        .call()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NetworkUnreachable, e.to_string()))?;
    let mut body = response.into_body().into_reader();
    let mut file = std::fs::File::create(&archive_path)?;
    std::io::copy(&mut body, &mut file)?;
    drop(file);

    // Verify checksum from the release's SHA256SUMS file.
    let checksum_url = format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{}/SHA256SUMS",
        ORT_VERSION
    );
    if let Ok(checksum_resp) = ureq::get(&checksum_url).call() {
        if let Ok(sums_content) = checksum_resp.into_body().read_to_string() {
            if let Some(expected_hash) = find_ort_checksum(&sums_content, ORT_ASSET) {
                let actual_hash = sha256_file(&archive_path)?;
                if expected_hash != actual_hash {
                    let _ = std::fs::remove_dir_all(&temp_dir);
                    return Err(std::io::Error::other(format!(
                        "ONNX Runtime checksum mismatch: expected {expected_hash}, got {actual_hash}"
                    )));
                }
                info!("ONNX Runtime checksum verified");
            } else {
                warn!(
                    "ONNX Runtime SHA256SUMS downloaded but no entry for {} — \
                     proceeding without checksum verification",
                    ORT_ASSET
                );
            }
        }
    } else {
        warn!("ONNX Runtime SHA256SUMS not available — proceeding without checksum verification");
    }

    // Extract the .tgz archive
    let extract_dir = temp_dir.join("extracted");
    std::fs::create_dir_all(&extract_dir)?;

    let output = std::process::Command::new("tar")
        .arg("xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&extract_dir)
        .output()?;

    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(std::io::Error::other(format!(
            "tar extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    // Find the shared library in the extracted directory
    let mut found_lib: Option<PathBuf> = None;
    for entry in walkdir::WalkDir::new(&extract_dir) {
        let entry = entry?;
        if entry.file_name() == ORT_LIB_NAME {
            found_lib = Some(entry.path().to_path_buf());
            break;
        }
    }

    let Some(src_lib) = found_lib else {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} not found in archive", ORT_LIB_NAME),
        ));
    };

    // Copy the library to cogz's lib dir
    std::fs::copy(&src_lib, &lib_path)?;

    // Clean up temp dir
    let _ = std::fs::remove_dir_all(&temp_dir);

    info!(
        "ONNX Runtime installed to {} ({}KB)",
        lib_path.display(),
        lib_path.metadata().map(|m| m.len() / 1024).unwrap_or(0)
    );

    Ok(lib_path)
}

/// Parse a SHA256SUMS file and find the checksum for the target asset.
fn find_ort_checksum(sums_content: &str, asset_name: &str) -> Option<String> {
    for line in sums_content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2 && parts[1] == asset_name {
            return Some(parts[0].to_string());
        }
    }
    None
}

/// Compute SHA-256 of a file.
fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
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

/// Ensure the ONNX Runtime is available and initialize `ort` with the
/// correct library path. Must be called before any `ort` API usage.
///
/// If the library is already available (system, env var, or previously
/// downloaded), uses it directly. If not, downloads the CPU-only
/// runtime (~11MB) to `~/.local/share/cogz/lib/`.
///
/// If the library cannot be found or downloaded, returns false — the
/// caller should operate in FTS-only mode.
pub fn ensure_ort() -> bool {
    *ORT_INIT.get_or_init(|| {
        // Try to find an existing library first.
        let lib_path = match find_existing_lib() {
            Some(path) => path,
            None => {
                // Try to download it.
                match download_ort() {
                    Ok(path) => path,
                    Err(e) => {
                        warn!("ONNX Runtime not found and download failed: {}", e);
                        warn!("system will operate in FTS-only mode");
                        warn!("to enable embeddings, install onnxruntime or set ORT_DYLIB_PATH");
                        return false;
                    }
                }
            }
        };

        // Initialize ort with the discovered library path.
        // This must happen before any Session::builder() call.
        // In ort 2.0.0-rc.13, init_from returns Result and commit returns bool.
        match ort::init_from(lib_path.to_string_lossy().to_string()) {
            Ok(builder) => {
                builder.commit();
                info!("ONNX Runtime initialized from {}", lib_path.display());
                true
            }
            Err(e) => {
                warn!(
                    "ONNX Runtime init failed from {}: {}",
                    lib_path.display(),
                    e
                );
                false
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ort_lib_path_is_under_cogz_lib() {
        let path = ort_lib_path();
        assert!(path.ends_with(ORT_LIB_NAME));
    }

    #[test]
    fn find_existing_lib_returns_none_when_nothing_installed() {
        // SAFETY: this test runs single-threaded; removing an env var
        // here is safe because no other thread is reading the environment.
        unsafe {
            std::env::remove_var("ORT_DYLIB_PATH");
        }
        let _ = find_existing_lib();
    }

    #[test]
    fn find_ort_checksum_matches_correct_asset() {
        let sums =
            "abc123  onnxruntime-linux-x64-1.27.0.tgz\ndef456  onnxruntime-win-x64-1.27.0.zip\n";
        assert_eq!(
            find_ort_checksum(sums, "onnxruntime-linux-x64-1.27.0.tgz"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn find_ort_checksum_returns_none_for_missing_asset() {
        let sums = "abc123  onnxruntime-linux-x64-1.27.0.tgz\n";
        assert_eq!(find_ort_checksum(sums, "onnxruntime-windows.zip"), None);
    }
}
