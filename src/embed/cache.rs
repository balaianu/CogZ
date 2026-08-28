//! Content-hash embedding cache.
//!
//! Avoids re-embedding text whose content hash hasn't changed.
//! Keyed by (model_name, content_hash) → embedding vector.
//! Lives in memory; the DB vec0 table is the persistent store.

use std::collections::HashMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::model::{EmbeddingError, EmbeddingModel, EmbeddingResult};

/// Content hash for cache keys. SHA-256 hex string.
fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// In-memory embedding cache keyed by (model_name, content_hash).
///
/// Thread-safe via internal Mutex. Used by the sync pipeline to skip
/// re-embedding text whose content hasn't changed since the last sync.
pub struct EmbeddingCache {
    entries: Mutex<HashMap<(String, String), Vec<f32>>>,
    hits: Mutex<u64>,
    misses: Mutex<u64>,
}

impl EmbeddingCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            hits: Mutex::new(0),
            misses: Mutex::new(0),
        }
    }

    /// Look up an embedding by content hash. Returns a clone of the
    /// cached vector if present.
    pub fn get(&self, model_name: &str, text: &str) -> Option<Vec<f32>> {
        let key = (model_name.to_string(), content_hash(text));
        let result = self.entries.lock().ok()?.get(&key).cloned();
        match &result {
            Some(_) => *self.hits.lock().unwrap() += 1,
            None => *self.misses.lock().unwrap() += 1,
        }
        result
    }

    /// Store an embedding in the cache.
    pub fn put(&self, model_name: &str, text: &str, embedding: Vec<f32>) {
        let key = (model_name.to_string(), content_hash(text));
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(key, embedding);
        }
    }

    /// Embed text using the model, caching the result. If the text
    /// is already cached, returns the cached embedding without calling
    /// the model.
    pub fn embed_cached(
        &self,
        model: &dyn EmbeddingModel,
        text: &str,
    ) -> EmbeddingResult<Vec<f32>> {
        if let Some(cached) = self.get(model.model_name(), text) {
            return Ok(cached);
        }
        let embeddings = model.embed(&[text])?;
        let embedding = embeddings
            .into_iter()
            .next()
            .ok_or(EmbeddingError::InferenceFailed("empty result".to_string()))?;
        self.put(model.model_name(), text, embedding.clone());
        Ok(embedding)
    }

    /// Embed a batch of texts, using the cache for any that are already
    /// present. Texts not in the cache are embedded in a single batch
    /// call to the model.
    pub fn embed_batch_cached(
        &self,
        model: &dyn EmbeddingModel,
        texts: &[&str],
    ) -> EmbeddingResult<Vec<Vec<f32>>> {
        let mut results = vec![None; texts.len()];
        let mut uncached_indices = Vec::new();
        let mut uncached_texts = Vec::new();

        for (i, text) in texts.iter().enumerate() {
            if let Some(cached) = self.get(model.model_name(), text) {
                results[i] = Some(cached);
            } else {
                uncached_indices.push(i);
                uncached_texts.push(*text);
            }
        }

        if !uncached_texts.is_empty() {
            let embeddings = model.embed(&uncached_texts)?;
            if embeddings.len() != uncached_indices.len() {
                return Err(EmbeddingError::InferenceFailed(format!(
                    "expected {} embeddings, got {}",
                    uncached_indices.len(),
                    embeddings.len()
                )));
            }
            for (idx, embedding) in uncached_indices.iter().zip(embeddings) {
                self.put(model.model_name(), texts[*idx], embedding.clone());
                results[*idx] = Some(embedding);
            }
        }

        Ok(results.into_iter().map(Option::unwrap).collect())
    }

    /// Number of cache hits since creation.
    pub fn hits(&self) -> u64 {
        *self.hits.lock().unwrap()
    }

    /// Number of cache misses since creation.
    pub fn misses(&self) -> u64 {
        *self.misses.lock().unwrap()
    }

    /// Total entries in the cache.
    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all cached embeddings.
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }
}

impl Default for EmbeddingCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::MockEmbeddingModel;

    #[test]
    fn cache_hit_skips_model_call() {
        let cache = EmbeddingCache::new();
        let model = MockEmbeddingModel::new();

        // First call — miss, embeds
        let v1 = cache.embed_cached(&model, "hello world").unwrap();
        assert_eq!(cache.misses(), 1);
        assert_eq!(cache.hits(), 0);

        // Second call — hit, returns cached
        let v2 = cache.embed_cached(&model, "hello world").unwrap();
        assert_eq!(v1, v2);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 1);
    }

    #[test]
    fn cache_miss_embeds_and_stores() {
        let cache = EmbeddingCache::new();
        let model = MockEmbeddingModel::new();

        let v1 = cache.embed_cached(&model, "text A").unwrap();
        let v2 = cache.embed_cached(&model, "text B").unwrap();

        assert_ne!(v1, v2);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.misses(), 2);
    }

    #[test]
    fn cache_batch_mixed_hits_and_misses() {
        let cache = EmbeddingCache::new();
        let model = MockEmbeddingModel::new();

        // Pre-populate one entry — 1 miss
        let _ = cache.embed_cached(&model, "cached").unwrap();

        // Batch with one cached (hit), one new (miss)
        let results = cache
            .embed_batch_cached(&model, &["cached", "fresh"])
            .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(cache.hits(), 1); // "cached" was a hit
        assert_eq!(cache.misses(), 2); // pre-populate + "fresh"
    }

    #[test]
    fn cache_clear_empties_entries() {
        let cache = EmbeddingCache::new();
        let model = MockEmbeddingModel::new();

        cache.embed_cached(&model, "text").unwrap();
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn cache_separates_models() {
        let cache = EmbeddingCache::new();

        // Same text, different model name — should be separate cache entries.
        cache.put("model_a", "text", vec![1.0; 768]);
        cache.put("model_b", "text", vec![2.0; 768]);

        let a = cache.get("model_a", "text").unwrap();
        let b = cache.get("model_b", "text").unwrap();
        assert_ne!(a, b);
    }
}
