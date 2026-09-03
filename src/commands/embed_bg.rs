//! Background code embedding — spawned by `cogz index` to embed code
//! entities after the foreground indexing completes.

use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::cli;

/// Embed code entities in the background after indexing.
///
/// Reads entity IDs from `ids_file`, opens the DB at `db_path`, loads
/// the code model, and embeds each entity in batches. The ID file is
/// removed on completion (including early returns when there's nothing
/// to embed or the model is unavailable).
///
/// On startup, sweeps the temp directory for stale `cogz-embed-bg-*.txt`
/// files older than 1 hour — these accumulate when background embed
/// processes crash before cleaning up their ID files.
pub fn run_embed_bg(
    db_path: &Path,
    ids_file: &Path,
    code_model: &str,
    dimension: usize,
    idle_ttl: u64,
    min_free_mb: u64,
) -> anyhow::Result<()> {
    use cogz::embed::{EmbeddingCache, ModelType, OnnxEmbeddingModel};
    use cogz::storage::crud::get_entities_batch;

    // Clean up stale ID files from crashed background embed processes.
    cleanup_stale_id_files();

    // Read entity IDs from the file.
    let ids_content = std::fs::read_to_string(ids_file)?;
    let entity_ids: Vec<String> = ids_content
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();

    if entity_ids.is_empty() {
        let _ = std::fs::remove_file(ids_file);
        return Ok(());
    }

    // Open the DB. The dimension parameter must match the vec0 table
    // created during the initial index. It's passed from the parent.
    let storage = cogz::storage::Storage::open(db_path, dimension)?;

    // Fetch entity data under the lock, then drop it.
    let entities: Vec<_> = {
        let conn = storage.conn();
        get_entities_batch(&conn, &entity_ids).unwrap_or_default()
    };

    if entities.is_empty() {
        let _ = std::fs::remove_file(ids_file);
        return Ok(());
    }

    // Load the code model.
    let models_dir = cli::models_dir();
    let model = OnnxEmbeddingModel::with_resource_config(
        ModelType::Code,
        &models_dir,
        dimension,
        code_model,
        idle_ttl,
        min_free_mb,
    );

    if !model.model_files_exist() {
        tracing::warn!("background embed: code model not available");
        let _ = std::fs::remove_file(ids_file);
        return Ok(());
    }

    // Embed in chunks, storing after each chunk to show progress.
    let cache = EmbeddingCache::new();
    let chunk_size = 32;
    let mut total_embedded = 0;

    for chunk in entities.chunks(chunk_size) {
        let embeddings = cogz::files::embed_sync::embed_entities(&model, &cache, chunk);
        if !embeddings.is_empty() {
            let mut conn = storage.conn();
            total_embedded += cogz::files::embed_sync::store_embeddings(&mut conn, &embeddings);
        }
    }

    tracing::info!("background embed: {} entities embedded", total_embedded);

    // Clean up the ID file.
    let _ = std::fs::remove_file(ids_file);

    Ok(())
}

/// Remove stale `cogz-embed-bg-*.txt` files from the temp directory.
/// These accumulate when background embed processes crash before
/// cleaning up their ID files. Files older than 1 hour are removed —
/// this preserves ID files from processes that are still running.
fn cleanup_stale_id_files() {
    let one_hour_ago = SystemTime::now() - Duration::from_secs(3600);
    let temp_dir = std::env::temp_dir();

    let Ok(entries) = std::fs::read_dir(&temp_dir) else {
        return;
    };

    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("cogz-embed-bg-") || !name.ends_with(".txt") {
            continue;
        }
        if let Ok(metadata) = std::fs::metadata(&path)
            && let Ok(modified) = metadata.modified()
            && modified < one_hour_ago
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }

    if removed > 0 {
        tracing::debug!("cleaned {} stale background embed ID file(s)", removed);
    }
}
