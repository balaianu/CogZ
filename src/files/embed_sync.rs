//! Embedding integration for the file sync pipeline.
//!
//! After syncing a file to DB, embeds its content and stores the
//! vector in vec0. Uses the content-hash cache to skip re-embedding
//! unchanged text. Degrades gracefully when the model is unavailable.

use rusqlite::Connection;

use crate::embed::{EmbeddingCache, EmbeddingModel};
use crate::storage::{self, crud::Entity};

/// Embed entities that were created or updated during sync.
///
/// For each entity, checks the content-hash cache. If the content
/// hasn't been embedded before (by this model), embeds it and stores
/// the vector. If the model is unavailable, logs a warning and
/// returns Ok — sync succeeds without embeddings (graceful
/// degradation).
pub fn embed_synced_entities(
    conn: &Connection,
    model: &dyn EmbeddingModel,
    cache: &EmbeddingCache,
    entities: &[Entity],
) -> usize {
    if !model.is_available() {
        tracing::debug!("embedding model unavailable, skipping embedding");
        return 0;
    }

    let mut embedded = 0;
    for entity in entities {
        // Use title + content as the text to embed
        let text = match &entity.title {
            Some(t) => format!("{}\n\n{}", t, entity.content),
            None => entity.content.clone(),
        };

        match cache.embed_cached(model, &text) {
            Ok(embedding) => {
                // Delete old embedding first (in case of re-embed)
                let _ = storage::embeddings::delete_embedding(conn, &entity.id);
                if let Err(e) = storage::embeddings::insert_embedding(conn, &entity.id, &embedding)
                {
                    tracing::warn!("failed to store embedding for {}: {}", entity.id, e);
                } else {
                    embedded += 1;
                }
            }
            Err(e) => {
                tracing::warn!("embedding failed for {}: {}", entity.id, e);
            }
        }
    }
    embedded
}
