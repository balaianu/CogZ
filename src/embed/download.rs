//! Model download via `hf-hub` and broken cache cleanup.
//!
//! Downloads ONNX models from HuggingFace on first use. Before every
//! model load, `clean_broken_cache()` removes stale `.incomplete` files
//! and empty `refs/main` files that accumulate from interrupted
//! downloads — preventing the disk-filling retry loop that affected
//! Mnemos.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use hf_hub::{HFClientBuilder, HFError, split_id};

/// Error during model download or cache cleanup.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("hf-hub error: {0}")]
    HfHub(#[from] HFError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("model not found in cache after download: {0}")]
    ModelNotFound(String),
}

/// Result of a model download.
#[derive(Debug)]
pub struct DownloadedModel {
    pub model_dir: PathBuf,
    pub model_id: String,
}

/// Download a model from HuggingFace Hub. Returns the local directory
/// where the model files are cached.
///
/// Uses the model registry to resolve the actual HF download source.
/// For example, `nomic-ai/CodeRankEmbed-int8` maps to
/// `mrsladoje/CodeRankEmbed-onnx-int8` (an ungated community ONNX export).
///
/// Uses `hf-hub` with content-addressed caching. If the model is already
/// cached, it is not re-downloaded. Downloads the ONNX model file and
/// tokenizer.
///
/// After download, creates a symlink from `cache_dir/{model_id}` to the
/// hf-hub snapshot directory so the loader can find files at a
/// predictable flat path.
pub fn download_model(model_id: &str, cache_dir: &Path) -> Result<DownloadedModel, DownloadError> {
    clean_broken_cache(cache_dir);

    // Resolve the actual HF source repo via the registry.
    let (hf_source, onnx_layout) = super::registry::resolve_source(model_id);
    let onnx_file = super::registry::onnx_filename(onnx_layout);

    tracing::info!(
        "downloading model {} from {} ({})",
        model_id,
        hf_source,
        onnx_file
    );

    let client = HFClientBuilder::new()
        .cache_dir(cache_dir.to_path_buf())
        .build_sync()?;

    let (owner, name) = split_id(hf_source);
    let repo = client.model(owner, name);

    // Download the ONNX model file at the layout-specific path.
    let model_path = repo
        .download_file()
        .filename(onnx_file)
        .send()
        .map_err(|e| {
            tracing::warn!("failed to download {} from {}: {}", onnx_file, hf_source, e);
            DownloadError::HfHub(e)
        })?;

    // tokenizer.json is always at the root.
    let _tokenizer_path = repo.download_file().filename("tokenizer.json").send()?;

    // config.json contains the id2label mapping for NLI models.
    // Download it if available; non-NLI models also have it but we
    // only use it for NLI label detection. Ignore errors — not all
    // repos have config.json, and the loader falls back to defaults.
    let _ = repo.download_file().filename("config.json").send();

    // The snapshot root is the parent of tokenizer.json.
    // For OnnxSubdir layouts, the model is at snapshot/onnx/model.onnx,
    // so we need the grandparent. For Root layouts, the model is at
    // snapshot/model.onnx, so we need the parent.
    let snapshot_dir = if onnx_file.contains('/') {
        // onnx/model.onnx or onnx/model_quantized.onnx
        model_path
            .parent()
            .and_then(|p| p.parent())
            .ok_or_else(|| DownloadError::ModelNotFound(model_id.to_string()))?
            .to_path_buf()
    } else {
        // model.onnx or model_optimized.onnx at root
        model_path
            .parent()
            .ok_or_else(|| DownloadError::ModelNotFound(model_id.to_string()))?
            .to_path_buf()
    };

    // Create a link from cache_dir/{model_id} to the hf-hub snapshot
    // directory so the loader finds files at a predictable flat path.
    // On Unix, this is a symlink. On Windows, symlinks require admin
    // privileges or Developer Mode, so we fall back to using the
    // snapshot directory directly.
    let flat_dir = cache_dir.join(model_id);
    if flat_dir.exists() || flat_dir.is_symlink() {
        let _ = std::fs::remove_file(&flat_dir);
    }
    if let Some(parent) = flat_dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&snapshot_dir, &flat_dir)?;
    }
    #[cfg(windows)]
    {
        // Try symlink first (works with Developer Mode), fall back to
        // using the snapshot directory directly.
        match std::os::windows::fs::symlink_dir(&snapshot_dir, &flat_dir) {
            Ok(()) => {}
            Err(_) => {
                // Can't create symlink — use the snapshot directory directly.
                tracing::debug!(
                    "could not create symlink on Windows (needs Developer Mode), \
                     using snapshot path directly"
                );
                return Ok(DownloadedModel {
                    model_dir: snapshot_dir,
                    model_id: model_id.to_string(),
                });
            }
        }
    }

    tracing::info!(
        "downloaded model {} to {} (symlink: {})",
        model_id,
        snapshot_dir.display(),
        flat_dir.display()
    );

    Ok(DownloadedModel {
        model_dir: flat_dir,
        model_id: model_id.to_string(),
    })
}

/// Check if a model is already cached and accessible at the flat path
/// the loader expects. The hf-hub cache may exist, but if the symlink
/// from `cache_dir/{model_id}` to the snapshot is missing, the loader
/// won't find the files.
pub fn is_model_cached(model_id: &str, cache_dir: &Path) -> bool {
    let flat_path = cache_dir.join(model_id);
    flat_path.exists()
}

/// Remove broken cache artifacts that accumulate from interrupted downloads.
///
/// - Removes `.incomplete` files older than 1 hour (preserves active downloads)
/// - Removes empty `refs/main` files (0 bytes — blocks model loading)
///
/// Returns the count of files removed.
pub fn clean_broken_cache(cache_dir: &Path) -> usize {
    let mut removed = 0;
    let one_hour_ago = SystemTime::now() - Duration::from_secs(3600);

    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return 0;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        removed += clean_model_cache_dir(&path, one_hour_ago);
    }

    if removed > 0 {
        tracing::info!("cleaned {} broken cache file(s)", removed);
    }

    removed
}

fn clean_model_cache_dir(model_dir: &Path, one_hour_ago: SystemTime) -> usize {
    let mut removed = 0;

    // Check for .incomplete files in blobs/
    let blobs_dir = model_dir.join("blobs");
    if blobs_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&blobs_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "incomplete")
                && let Ok(metadata) = std::fs::metadata(&path)
                && let Ok(modified) = metadata.modified()
                && modified < one_hour_ago
                && std::fs::remove_file(&path).is_ok()
            {
                tracing::debug!("removed stale incomplete: {}", path.display());
                removed += 1;
            }
        }
    }

    // Check for empty refs/main files
    let refs_main = model_dir.join("refs").join("main");
    if refs_main.exists()
        && let Ok(metadata) = std::fs::metadata(&refs_main)
        && metadata.len() == 0
        && std::fs::remove_file(&refs_main).is_ok()
    {
        tracing::debug!("removed empty refs/main: {}", refs_main.display());
        removed += 1;
    }

    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_cache_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn clean_removes_old_incomplete_files() {
        let dir = setup_cache_dir();
        let model_cache = dir.path().join("models--org--model");
        let blobs = model_cache.join("blobs");
        fs::create_dir_all(&blobs).unwrap();

        let incomplete = blobs.join("abc123.incomplete");
        fs::write(&incomplete, b"partial data").unwrap();

        // Set modification time to 2 hours ago
        let old_time =
            filetime::FileTime::from_system_time(SystemTime::now() - Duration::from_secs(7200));
        filetime::set_file_mtime(&incomplete, old_time).unwrap();

        let removed = clean_broken_cache(dir.path());
        assert_eq!(removed, 1);
        assert!(!incomplete.exists());
    }

    #[test]
    fn clean_preserves_recent_incomplete_files() {
        let dir = setup_cache_dir();
        let model_cache = dir.path().join("models--org--model");
        let blobs = model_cache.join("blobs");
        fs::create_dir_all(&blobs).unwrap();

        let incomplete = blobs.join("abc123.incomplete");
        fs::write(&incomplete, b"downloading").unwrap();

        // Recent file — should not be removed
        let removed = clean_broken_cache(dir.path());
        assert_eq!(removed, 0);
        assert!(incomplete.exists());
    }

    #[test]
    fn clean_removes_empty_refs_main() {
        let dir = setup_cache_dir();
        let model_cache = dir.path().join("models--org--model");
        let refs = model_cache.join("refs");
        fs::create_dir_all(&refs).unwrap();

        let refs_main = refs.join("main");
        fs::write(&refs_main, b"").unwrap();

        let removed = clean_broken_cache(dir.path());
        assert_eq!(removed, 1);
        assert!(!refs_main.exists());
    }

    #[test]
    fn clean_preserves_nonempty_refs_main() {
        let dir = setup_cache_dir();
        let model_cache = dir.path().join("models--org--model");
        let refs = model_cache.join("refs");
        fs::create_dir_all(&refs).unwrap();

        let refs_main = refs.join("main");
        fs::write(&refs_main, b"abc123commit").unwrap();

        let removed = clean_broken_cache(dir.path());
        assert_eq!(removed, 0);
        assert!(refs_main.exists());
    }

    #[test]
    fn clean_handles_nonexistent_dir() {
        let dir = setup_cache_dir();
        let removed = clean_broken_cache(dir.path().join("nonexistent").as_path());
        assert_eq!(removed, 0);
    }

    #[test]
    fn is_model_cached_false_for_missing() {
        let dir = setup_cache_dir();
        assert!(!is_model_cached("org/model", dir.path()));
    }

    #[test]
    fn is_model_cached_true_for_present() {
        let dir = setup_cache_dir();
        let flat_path = dir.path().join("org/model");
        fs::create_dir_all(&flat_path).unwrap();

        assert!(is_model_cached("org/model", dir.path()));
    }
}
