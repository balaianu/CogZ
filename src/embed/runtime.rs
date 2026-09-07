//! ONNX Runtime library discovery and auto-download.
//!
//! The `ort` crate with `load-dynamic` requires the ONNX Runtime
//! shared library at runtime. This module handles:
//!
//! 1. Checking if the library is already available (system install,
//!    `ORT_DYLIB_PATH`, or previously downloaded to cogz's lib dir).
//! 2. Downloading the CPU-only ONNX Runtime from GitHub releases to
//!    `~/.local/share/cogz/lib/` if not found.
//! 3. Initializing `ort` with the discovered path via `ort::init_from()`.
//!
//! Supported platforms for auto-download:
//! - Linux x86_64, Linux aarch64
//! - macOS arm64 (Apple Silicon)
//! - Windows x86_64
//!
//! macOS x86_64 (Intel) is not supported — Microsoft dropped macOS
//! Intel binaries after ORT 1.22. Intel Mac users can use Rosetta 2
//! or operate in FTS-only mode.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[path = "checksum.rs"]
mod checksum;

use super::suppress::suppress_stderr_during;
pub use checksum::ort_lib_path;
use checksum::{cogz_lib_dir, find_ort_checksum, lib_exists, sha256_file};
use tracing::{info, warn};

/// ONNX Runtime version to download. ort 2.0.0-rc.13 supports ORT 1.28;
/// we use 1.27.0 to match the version FastEmbed ships, ensuring
/// comparable inference performance.
const ORT_VERSION: &str = "1.27.0";

// ── Platform-specific constants ─────────────────────────────────────

/// GitHub release asset name for the current platform.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const ORT_ASSET: &str = "onnxruntime-linux-x64-1.27.0.tgz";

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const ORT_ASSET: &str = "onnxruntime-linux-aarch64-1.27.0.tgz";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const ORT_ASSET: &str = "onnxruntime-osx-arm64-1.27.0.tgz";

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const ORT_ASSET: &str = "onnxruntime-win-x64-1.27.0.zip";

/// Fallback for unsupported platforms (e.g. macOS Intel). The
/// download function checks this and returns an error.
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64"),
)))]
const ORT_ASSET: &str = "";

/// The shared library file name for the current platform.
#[cfg(target_os = "linux")]
pub(crate) const ORT_LIB_NAME: &str = "libonnxruntime.so";

#[cfg(target_os = "macos")]
pub(crate) const ORT_LIB_NAME: &str = "libonnxruntime.dylib";

#[cfg(target_os = "windows")]
pub(crate) const ORT_LIB_NAME: &str = "onnxruntime.dll";

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) const ORT_LIB_NAME: &str = "";

/// Archive extension for the current platform's ORT download.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const ORT_ARCHIVE_EXT: &str = "tgz";

#[cfg(target_os = "windows")]
const ORT_ARCHIVE_EXT: &str = "zip";

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
const ORT_ARCHIVE_EXT: &str = "";

/// Whether this platform supports ORT auto-download.
const fn ort_download_supported() -> bool {
    !ORT_ASSET.is_empty()
}

/// Static init guard — ensures `ort::init_from` is called exactly once.
static ORT_INIT: OnceLock<bool> = OnceLock::new();

/// System library paths to check for an existing ONNX Runtime install.
/// Returns platform-appropriate candidate paths.
fn system_lib_candidates() -> Vec<&'static str> {
    let mut candidates = Vec::new();

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        candidates.extend_from_slice(&[
            "/usr/lib/x86_64-linux-gnu/libonnxruntime.so",
            "/usr/lib/x86_64-linux-gnu/libonnxruntime.so.1",
            "/usr/lib/x86_64-linux-gnu/libonnxruntime.so.1.23",
            "/usr/local/lib/libonnxruntime.so",
            "/usr/lib/libonnxruntime.so",
        ]);
    }

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        candidates.extend_from_slice(&[
            "/usr/lib/aarch64-linux-gnu/libonnxruntime.so",
            "/usr/lib/aarch64-linux-gnu/libonnxruntime.so.1",
            "/usr/local/lib/libonnxruntime.so",
            "/usr/lib/libonnxruntime.so",
        ]);
    }

    #[cfg(target_os = "macos")]
    {
        candidates.extend_from_slice(&[
            "/opt/homebrew/lib/libonnxruntime.dylib",
            "/usr/local/lib/libonnxruntime.dylib",
        ]);
    }

    #[cfg(target_os = "windows")]
    {
        candidates.extend_from_slice(&[
            "C:\\Program Files\\onnxruntime\\lib\\onnxruntime.dll",
            "C:\\onnxruntime\\lib\\onnxruntime.dll",
        ]);
    }

    candidates
}

/// Try to find an existing ONNX Runtime library. Checks:
/// 1. `ORT_DYLIB_PATH` env var (user override)
/// 2. cogz's lib dir (previously downloaded)
/// 3. System library paths
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

    // 3. System library paths
    for candidate in system_lib_candidates() {
        if lib_exists(Path::new(candidate)) {
            return Some(PathBuf::from(candidate));
        }
    }

    None
}

/// Download and extract the ONNX Runtime library to cogz's lib dir.
/// Downloads the CPU-only build from GitHub releases.
/// Verifies the SHA-256 checksum from the release's SHA256SUMS file
/// before extracting.
fn download_ort() -> Result<PathBuf, std::io::Error> {
    if !ort_download_supported() {
        return Err(std::io::Error::other(
            "ONNX Runtime auto-download is not supported on this platform. \
             Install onnxruntime manually and set ORT_DYLIB_PATH, or use FTS-only mode.",
        ));
    }

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

    let archive_name = format!("onnxruntime.{}", ORT_ARCHIVE_EXT);
    let archive_path = temp_dir.join(&archive_name);

    // Download the archive using ureq (already a dependency).
    let response = ureq::get(&url)
        .call()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NetworkUnreachable, e.to_string()))?;
    let mut body = response.into_body().into_reader();
    let mut file = std::fs::File::create(&archive_path)?;
    std::io::copy(&mut body, &mut file)?;
    drop(file);

    // Verify checksum from the release's SHA256SUMS file if available.
    // If the manifest exists but lacks our asset, fail — that's a
    // supply-chain red flag. If the manifest itself is unavailable
    // (404), proceed with the HTTPS-only download as degraded mode.
    let checksum_url = format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{}/SHA256SUMS",
        ORT_VERSION
    );
    match ureq::get(&checksum_url).call() {
        Ok(checksum_resp) => {
            let sums_content = checksum_resp
                .into_body()
                .read_to_string()
                .map_err(|e| std::io::Error::other(format!("failed to read SHA256SUMS: {e}")))?;
            match find_ort_checksum(&sums_content, ORT_ASSET) {
                Some(expected_hash) => {
                    let actual_hash = sha256_file(&archive_path)?;
                    if expected_hash != actual_hash {
                        let _ = std::fs::remove_dir_all(&temp_dir);
                        return Err(std::io::Error::other(format!(
                            "ONNX Runtime checksum mismatch: expected {expected_hash}, got {actual_hash}"
                        )));
                    }
                    info!("ONNX Runtime checksum verified");
                }
                None => {
                    let _ = std::fs::remove_dir_all(&temp_dir);
                    return Err(std::io::Error::other(format!(
                        "ONNX Runtime SHA256SUMS downloaded but no entry for {ORT_ASSET} — \
                         refusing to install unverified binary"
                    )));
                }
            }
        }
        Err(e) => {
            // Manifest unavailable (404, network error) — degraded mode.
            // The archive was downloaded over HTTPS from GitHub's release
            // CDN. Microsoft ORT releases don't always ship SHA256SUMS.
            warn!(
                "ONNX Runtime SHA256SUMS not available ({}): \
                   proceeding with unverified HTTPS download as degraded mode",
                e
            );
        }
    }

    // Extract the archive. `tar` is available on all supported platforms:
    // - Linux/macOS: GNU tar (always present)
    // - Windows 10 1803+: bsdtar (included with the OS)
    // bsdtar on Windows handles .zip; GNU tar on Unix handles .tgz.
    //
    // --no-absolute-names (GNU tar) / --no-same-owner prevents
    // path-traversal attacks from malicious archives (tar slip).
    // On bsdtar (Windows), --no-absolute-names is also supported.
    let extract_dir = temp_dir.join("extracted");
    std::fs::create_dir_all(&extract_dir)?;

    let output = std::process::Command::new("tar")
        .arg("xf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&extract_dir)
        .arg("--no-absolute-names")
        .output()?;

    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(std::io::Error::other(format!(
            "archive extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    // Find the shared library in the extracted directory.
    // Verify the found path is inside extract_dir (defense-in-depth
    // against tar slip even after --no-absolute-names).
    let mut found_lib: Option<PathBuf> = None;
    for entry in walkdir::WalkDir::new(&extract_dir) {
        let entry = entry?;
        if entry.file_name() == ORT_LIB_NAME {
            let path = entry.path().to_path_buf();
            // Canonicalize both paths to compare them reliably.
            let canonical_extract = extract_dir
                .canonicalize()
                .unwrap_or_else(|_| extract_dir.clone());
            let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !canonical_path.starts_with(&canonical_extract) {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(std::io::Error::other(
                    "archive contains path outside extraction directory — refusing to load",
                ));
            }
            found_lib = Some(path);
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

    // On Windows, the DLL may have companion libraries that need to be
    // in the same directory. Copy all .dll files from the lib directory
    // in the archive.
    #[cfg(target_os = "windows")]
    {
        if let Some(src_lib_dir) = src_lib.parent()
            && let Ok(entries) = std::fs::read_dir(src_lib_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("dll")
                    && let Some(name) = path.file_name()
                {
                    let dest = lib_dir.join(name);
                    if !dest.exists() {
                        let _ = std::fs::copy(&path, &dest);
                    }
                }
            }
        }
    }

    // Clean up temp dir
    let _ = std::fs::remove_dir_all(&temp_dir);

    info!(
        "ONNX Runtime installed to {} ({}KB)",
        lib_path.display(),
        lib_path.metadata().map(|m| m.len() / 1024).unwrap_or(0)
    );

    Ok(lib_path)
}

/// Ensure the ONNX Runtime is available and initialize `ort` with the
/// correct library path. Must be called before any `ort` API usage.
///
/// If the library is already available (system, env var, or previously
/// downloaded), uses it directly. If not, downloads the CPU-only
/// runtime to `~/.local/share/cogz/lib/`.
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
        //
        // The ONNX Runtime C++ library emits ~1258 duplicate schema
        // registration warnings directly to stderr (fd 2) during init.
        // These are harmless (idempotent re-registration) but flood
        // terminal output. Suppress stderr during init, then restore.
        match suppress_stderr_during(|| ort::init_from(lib_path.to_string_lossy().to_string())) {
            Ok(builder) => {
                // commit() may also emit warnings — suppress here too.
                let _ = suppress_stderr_during(|| builder.commit());
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
#[path = "runtime_tests.rs"]
mod tests;
