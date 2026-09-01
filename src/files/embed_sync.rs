//! Embedding integration for the file sync pipeline.
//!
//! After syncing a file to DB, embeds its content and stores the
//! vector in vec0. Uses the content-hash cache to skip re-embedding
//! unchanged text. Degrades gracefully when the model is unavailable.

use rusqlite::Connection;

use crate::embed::{EmbeddingCache, EmbeddingModel};
use crate::storage::{self, crud::Entity};

/// Compute embeddings for a list of entities. Pure computation —
/// no DB access. Returns `(entity_id, entity_type, embedding)` triples
/// for entities that were successfully embedded. The entity type is
/// carried through so `store_embeddings` can route to the correct
/// vec0 table without a per-entity DB lookup.
///
/// If the model is unavailable, `embed()` returns
/// `EmbeddingError::ModelUnavailable` on the first call, which is
/// logged and skipped. The caller is responsible for DB storage via
/// `store_embeddings`.
pub fn embed_entities(
    model: &dyn EmbeddingModel,
    cache: &EmbeddingCache,
    entities: &[Entity],
) -> Vec<(String, String, Vec<f32>)> {
    let texts: Vec<String> = entities
        .iter()
        .map(|e| match &e.title {
            Some(t) => format!("{}\n\n{}", t, e.content),
            None => e.content.clone(),
        })
        .collect();
    let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

    match cache.embed_batch_cached(model, &text_refs) {
        Ok(embeddings) => entities
            .iter()
            .zip(embeddings)
            .map(|(entity, embedding)| (entity.id.clone(), entity.r#type.clone(), embedding))
            .collect(),
        Err(e) => {
            tracing::warn!("batch embedding failed: {}", e);
            Vec::new()
        }
    }
}

/// Store embeddings in the vec0 tables. Replaces any existing
/// embedding for each entity. Requires a DB connection — caller
/// is responsible for lock acquisition. All writes are wrapped
/// in a single transaction for atomicity and performance.
/// Each triple is (entity_id, entity_type, embedding).
pub fn store_embeddings(conn: &mut Connection, embeddings: &[(String, String, Vec<f32>)]) -> usize {
    if embeddings.is_empty() {
        return 0;
    }
    let mut stored = 0;
    let tx = match conn.transaction() {
        Ok(tx) => tx,
        Err(e) => {
            tracing::warn!("failed to begin embedding transaction: {}", e);
            return 0;
        }
    };
    for (entity_id, entity_type, embedding) in embeddings {
        let _ = storage::embeddings::delete_embedding(&tx, entity_id);
        if let Err(e) =
            storage::embeddings::insert_embedding(&tx, entity_id, entity_type, embedding)
        {
            tracing::warn!("failed to store embedding for {}: {}", entity_id, e);
        } else {
            stored += 1;
        }
    }
    if let Err(e) = tx.commit() {
        tracing::warn!("failed to commit embedding transaction: {}", e);
        return 0;
    }
    stored
}
