//! NLI-based contradiction detection.
//!
//! Runs on insert when `contradiction_check` is enabled and an NLI
//! model is available. Compares a new observation or rule against
//! existing entities of the same type and flags contradictions.

use rusqlite::Connection;

use crate::config::ConsolidationConfig;
use crate::embed::{EmbeddingModel, NliModel, similarity::cosine_similarity};
use crate::storage::crud::Entity;
use crate::storage::query::get_entities_by_type;

/// Result of a contradiction check on a newly inserted entity.
#[derive(Debug, Clone)]
pub struct ContradictionResult {
    /// True if the NLI model detected at least one contradiction.
    pub contradiction_flagged: bool,
    /// IDs of entities that contradict the new entity.
    pub contradicts_ids: Vec<String>,
}

impl ContradictionResult {
    pub fn empty() -> Self {
        Self {
            contradiction_flagged: false,
            contradicts_ids: Vec::new(),
        }
    }
}

/// Run contradiction detection against existing entities of the same
/// type. Returns an empty result if the NLI model is unavailable or
/// contradiction checking is disabled in config.
///
/// Uses bidirectional NLI scoring (both premise→hypothesis and
/// hypothesis→premise), taking the max P(contradiction) across
/// directions. Real contradictions can score asymmetrically.
///
/// Pre-filters reduce false positives:
/// - Length ratio: texts differing by >5:1 are different content types.
/// - Cosine similarity (when an embedding model is provided): genuine
///   contradictions share the same topic, so embeddings should be
///   highly similar (≥ 0.85).
///
/// The caller is responsible for recording `contradicts` edges
/// and the `contradiction_found` event.
pub fn check_contradiction(
    conn: &Connection,
    new_id: &str,
    new_content: &str,
    entity_type: &str,
    model: Option<&dyn NliModel>,
    embed_model: Option<&dyn EmbeddingModel>,
    config: &ConsolidationConfig,
) -> ContradictionResult {
    let candidates = fetch_contradiction_candidates(conn, new_id, new_content, entity_type, config);
    let contradicts_ids = classify_candidates(candidates, new_content, model, embed_model, config);
    let contradiction_flagged = !contradicts_ids.is_empty();
    ContradictionResult {
        contradiction_flagged,
        contradicts_ids,
    }
}

/// Fetch candidate entities for contradiction checking. Returns the
/// list of existing active entities of the same type (excluding self).
/// This is the DB-only portion of contradiction detection — it should
/// be called under the storage lock, and the results passed to
/// `classify_candidates` outside the lock.
pub fn fetch_contradiction_candidates(
    conn: &Connection,
    new_id: &str,
    _new_content: &str,
    entity_type: &str,
    config: &ConsolidationConfig,
) -> Vec<Entity> {
    if !config.contradiction_check {
        return Vec::new();
    }

    match get_entities_by_type(conn, entity_type, Some("active"), 50) {
        Ok(e) => e.into_iter().filter(|e| e.id != new_id).collect(),
        Err(_) => Vec::new(),
    }
}

/// Classify candidate entities against the new content using the NLI
/// model. Returns the IDs of entities that contradict the new content.
/// This is the I/O portion — it must NOT be called while holding the
/// storage lock, since ONNX inference is blocking.
///
/// Uses bidirectional scoring: runs NLI in both directions (new→existing
/// and existing→new), takes the max P(contradiction). Real contradictions
/// can score asymmetrically (benched 0.44 one direction, 0.99 the other).
///
/// Pre-filters reduce false positives before NLI inference:
/// - Identical text fast-path: skip NLI entirely.
/// - Length ratio: texts differing by > `contradiction_length_ratio`
///   are different content types, not contradictions.
/// - Cosine similarity (when an embedding model is provided): genuine
///   contradictions share the same topic, so embeddings should be
///   highly similar (≥ `contradiction_cosine_threshold`).
///
/// A pair is flagged as contradicting only when:
/// 1. Texts are not identical.
/// 2. Length ratio is within bounds.
/// 3. Cosine similarity ≥ threshold (when embedding model is available).
/// 4. Max-direction P(contradiction) ≥ `contradiction_threshold`.
pub fn classify_candidates(
    candidates: Vec<Entity>,
    new_content: &str,
    model: Option<&dyn NliModel>,
    embed_model: Option<&dyn EmbeddingModel>,
    config: &ConsolidationConfig,
) -> Vec<String> {
    let Some(model) = model else {
        return Vec::new();
    };

    let new_lower = new_content.to_lowercase();
    let new_len = new_content.len();

    // Pre-compute the new content's embedding if an embedding model is
    // available, for cosine similarity pre-filtering.
    let new_embedding = if let Some(em) = embed_model {
        em.embed(&[new_content])
            .ok()
            .and_then(|v| v.into_iter().next())
    } else {
        None
    };

    // Phase 1: apply cheap pre-filters (identical text, length ratio)
    // to determine which candidates need embedding + NLI.
    let mut need_nli: Vec<&Entity> = Vec::new();
    for entity in &candidates {
        if new_lower == entity.content.to_lowercase() {
            continue;
        }
        let entity_len = entity.content.len();
        if new_len > 0 && entity_len > 0 {
            let ratio = (new_len.max(entity_len) as f64) / (new_len.min(entity_len) as f64);
            if ratio > config.contradiction_length_ratio {
                continue;
            }
        }
        need_nli.push(entity);
    }

    if need_nli.is_empty() {
        return Vec::new();
    }

    // Phase 2: batch-embed all candidates that passed the cheap filters,
    // instead of one embed call per candidate inside the loop.
    let candidate_embeddings: Vec<Option<Vec<f32>>> =
        if let (Some(em), Some(_)) = (embed_model, &new_embedding) {
            let texts: Vec<&str> = need_nli.iter().map(|e| e.content.as_str()).collect();
            match em.embed(&texts) {
                Ok(embs) => embs.into_iter().map(Some).collect(),
                Err(_) => need_nli.iter().map(|_| None).collect(),
            }
        } else {
            need_nli.iter().map(|_| None).collect()
        };

    // Phase 3: cosine pre-filter + NLI scoring.
    let mut contradicts_ids = Vec::new();
    for (entity, entity_emb) in need_nli.iter().zip(candidate_embeddings) {
        // Cosine similarity pre-filter (when embedding is available).
        if let (Some(new_emb), Some(ref ent_emb)) = (&new_embedding, entity_emb) {
            let cos = cosine_similarity(new_emb, ent_emb);
            if cos < config.contradiction_cosine_threshold {
                continue;
            }
        }

        // Bidirectional NLI scoring: max P(contradiction) across both
        // directions. Real contradictions can score asymmetrically.
        let forward = model.classify(new_content, &entity.content);
        let reverse = model.classify(&entity.content, new_content);

        let max_contra = match (forward, reverse) {
            (Ok(f), Ok(r)) => f.contradiction.max(r.contradiction),
            (Ok(f), Err(_)) => f.contradiction,
            (Err(_), Ok(r)) => r.contradiction,
            (Err(e), _) => {
                tracing::warn!(
                    "NLI classification failed for {} vs {}: {}",
                    new_content,
                    entity.id,
                    e
                );
                continue;
            }
        };

        if max_contra >= config.contradiction_threshold as f32 {
            contradicts_ids.push(entity.id.clone());
        }
    }
    contradicts_ids
}

/// Record `contradicts` edges from the new entity to each
/// contradicted existing entity, write them to the entity's file
/// frontmatter (file-first invariant), and record a
/// `contradiction_found` domain event.
///
/// File I/O happens outside the storage lock. The lock is acquired
/// only for DB sync (which updates content_hash and creates edges
/// via sync_references) and event recording.
pub fn record_contradictions(
    storage: &crate::storage::Storage,
    cogz_dir: &std::path::Path,
    new_id: &str,
    contradicts_ids: &[String],
    file_path: &std::path::Path,
) -> Result<(), crate::storage::StorageError> {
    use crate::files::frontmatter::FmValue;
    use crate::files::{read_entity_file, write_entity_file};
    use crate::storage::events::{EventType, record_event};

    // 1. Update the file frontmatter with contradicts field (file-first).
    // Hold the canonical-file write lock across the read-modify-write
    // sequence to prevent concurrent writers from clobbering the
    // contradicts frontmatter.
    let _file_lock = storage.file_lock();

    let mut entity_file = read_entity_file(file_path).map_err(|e| {
        crate::storage::StorageError::File(format!(
            "failed to read entity file {}: {}",
            file_path.display(),
            e
        ))
    })?;
    entity_file
        .frontmatter
        .insert("contradicts", FmValue::Array(contradicts_ids.to_vec()));
    entity_file.updated_at = chrono::Utc::now().to_rfc3339();
    write_entity_file(file_path, &entity_file).map_err(|e| {
        crate::storage::StorageError::File(format!(
            "failed to write entity file {}: {}",
            file_path.display(),
            e
        ))
    })?;

    // 2. Sync the updated file to the DB. This updates the entity's
    // content_hash and properties, and sync_references creates the
    // contradicts edges from the frontmatter. The lock is acquired
    // inside sync_single_file.
    let relative_path = file_path
        .strip_prefix(cogz_dir)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();
    let sync_result = crate::files::sync_single_file(storage, cogz_dir, &relative_path);
    if let Some(err) = sync_result.errors.first() {
        return Err(crate::storage::StorageError::File(format!(
            "sync failed for {}: {}",
            file_path.display(),
            err.error
        )));
    }

    // 3. Record the contradiction_found event (under lock).
    let conn = storage.conn();
    let payload = serde_json::json!({
        "contradicts_ids": contradicts_ids,
        "model": "nli",
    });
    record_event(&conn, EventType::ContradictionFound, Some(new_id), &payload)?;
    Ok(())
}

#[cfg(test)]
#[path = "contradict_tests.rs"]
mod tests;
