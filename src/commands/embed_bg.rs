//! Background code embedding — spawned by `cogz index` to embed code
//! entities after the foreground indexing completes.

use std::path::Path;

use crate::cli;

/// Embed code entities in the background after indexing.
///
/// Reads entity IDs from `ids_file`, opens the DB at `db_path`, loads
/// the code model, and embeds each entity in batches. The ID file is
/// removed on completion (including early returns when there's nothing
/// to embed or the model is unavailable).
pub fn run_embed_bg(
    db_path: &Path,
    ids_file: &Path,
    code_model: &str,
    dimension: usize,
) -> anyhow::Result<()> {
    use cogz::embed::{EmbeddingCache, ModelType, OnnxEmbeddingModel};
    use cogz::storage::crud::get_entities_batch;

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
    let model =
        OnnxEmbeddingModel::with_model_id(ModelType::Code, &models_dir, dimension, code_model);

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
