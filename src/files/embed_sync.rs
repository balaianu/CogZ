//! Embedding integration for the file sync pipeline.
//!
//! After syncing a file to DB, embeds its content and stores the
//! vector in vec0. Uses the content-hash cache to skip re-embedding
//! unchanged text. Degrades gracefully when the model is unavailable.

use rusqlite::Connection;

use crate::embed::{EmbeddingCache, EmbeddingModel};
use crate::storage::{self, crud::Entity};

/// Compute embeddings for a list of entities. Pure computation —
/// no DB access. Returns `(entity_id, embedding)` pairs for entities
/// that were successfully embedded.
///
/// If the model is unavailable, `embed()` returns
/// `EmbeddingError::ModelUnavailable` on the first call, which is
/// logged and skipped. The caller is responsible for DB storage via
/// `store_embeddings`.
pub fn embed_entities(
    model: &dyn EmbeddingModel,
    cache: &EmbeddingCache,
    entities: &[Entity],
) -> Vec<(String, Vec<f32>)> {
    let mut results = Vec::new();
    for entity in entities {
        let text = match &entity.title {
            Some(t) => format!("{}\n\n{}", t, entity.content),
            None => entity.content.clone(),
        };

        match cache.embed_cached(model, &text) {
            Ok(embedding) => results.push((entity.id.clone(), embedding)),
            Err(e) => tracing::warn!("embedding failed for {}: {}", entity.id, e),
        }
    }
    results
}

/// Store embeddings in the vec0 table. Replaces any existing
/// embedding for each entity. Requires a DB connection — caller
/// is responsible for lock acquisition.
pub fn store_embeddings(conn: &Connection, embeddings: &[(String, Vec<f32>)]) -> usize {
    let mut stored = 0;
    for (entity_id, embedding) in embeddings {
        let _ = storage::embeddings::delete_embedding(conn, entity_id);
        if let Err(e) = storage::embeddings::insert_embedding(conn, entity_id, embedding) {
            tracing::warn!("failed to store embedding for {}: {}", entity_id, e);
        } else {
            stored += 1;
        }
    }
    stored
}
