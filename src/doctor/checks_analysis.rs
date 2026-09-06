use rusqlite::Connection;

use crate::config::Config;
use crate::doctor::checks::{DoctorReport, Issue, IssueKind};

pub(crate) fn check_near_duplicates(conn: &Connection, report: &mut DoctorReport) {
    // Check for knowledge entries with cosine similarity > 0.80.
    // This requires embeddings. If no embeddings are stored, skip.
    let embedding_count = crate::storage::embeddings::count_embeddings(conn).unwrap_or(0);
    if embedding_count == 0 {
        return;
    }

    // Fetch knowledge entity IDs in a single query (avoids N+1).
    let knowledge_ids: Vec<String> = {
        let sql = "SELECT id FROM entities WHERE type = 'knowledge' AND status != 'pruned'";
        if let Ok(mut stmt) = conn.prepare(sql)
            && let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0))
        {
            rows.flatten().collect()
        } else {
            return;
        }
    };

    if knowledge_ids.len() < 2 {
        return;
    }

    // Batch-fetch all embeddings for knowledge entities in one query.
    // vec0 supports WHERE entity_id IN (...) filtering.
    let (embeddings, failed_count) = fetch_embeddings_batch(conn, &knowledge_ids);
    if failed_count > 0 {
        report.issues.push(Issue {
            kind: IssueKind::CorruptEmbedding,
            entity_id: None,
            message: format!(
                "{failed_count} knowledge embedding{} could not be read and were skipped",
                if failed_count == 1 { "" } else { "s" }
            ),
        });
    }
    if embeddings.len() < 2 {
        return;
    }

    // Pairwise comparison. O(n²) but knowledge entries are typically
    // small in number (tens, not thousands).
    for i in 0..embeddings.len() {
        for j in (i + 1)..embeddings.len() {
            let sim = cosine_similarity(&embeddings[i].1, &embeddings[j].1);
            if sim > 0.80 {
                report.issues.push(Issue {
                    kind: IssueKind::NearDuplicate,
                    entity_id: Some(embeddings[i].0.clone()),
                    message: format!(
                        "knowledge {} and {} are near-duplicates (similarity={:.3})",
                        embeddings[i].0, embeddings[j].0, sim
                    ),
                });
            }
        }
    }
}

/// Fetch embeddings for multiple entity IDs from the knowledge
/// embedding table. Returns `(embeddings, failed_count)` where
/// `failed_count` is the number of rows that could not be parsed
/// (corrupt blobs). Chunked to respect SQLite variable limits.
///
/// Only queries `knowledge_embeddings` — the caller (`check_near_duplicates`)
/// only passes knowledge entity IDs. The old `entity_embeddings` table
/// was split into `code_embeddings` and `knowledge_embeddings` in
/// schema migration v3 and no longer exists.
pub(crate) fn fetch_embeddings_batch(
    conn: &Connection,
    ids: &[String],
) -> (Vec<(String, Vec<f32>)>, usize) {
    let chunk_size = 998; // SQLITE_MAX_VARIABLE_NUMBER / 1 param per row
    let mut result = Vec::new();
    let mut failed = 0usize;

    for chunk in ids.chunks(chunk_size) {
        let placeholders: Vec<&str> = (0..chunk.len()).map(|_| "?").collect();
        let sql = format!(
            "SELECT entity_id, embedding FROM knowledge_embeddings WHERE entity_id IN ({})",
            placeholders.join(",")
        );
        let params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

        if let Ok(mut stmt) = conn.prepare(&sql)
            && let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
                let entity_id: String = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                let floats: Vec<f32> = blob
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|chunk| f32::from_le_bytes(*chunk))
                    .collect();
                Ok((entity_id, floats))
            })
        {
            for row in rows {
                match row {
                    Ok(entry) => result.push(entry),
                    Err(e) => {
                        tracing::warn!("failed to read embedding row: {}", e);
                        failed += 1;
                    }
                }
            }
        }
    }

    (result, failed)
}

pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Check that vec0 tables' declared dimensions match the configured
/// embedding dimension. Reads the stored DDL from sqlite_master since
/// vec0 doesn't expose introspection via PRAGMA. Detects silent
/// mismatches from config changes after DB creation.
pub(crate) fn check_vec_dimensions(conn: &Connection, config: &Config, report: &mut DoctorReport) {
    for table in ["code_embeddings", "knowledge_embeddings"] {
        let ddl: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name = ?1",
                rusqlite::params![table],
                |r| r.get(0),
            )
            .ok();

        let Some(ddl) = ddl else { continue };

        // Extract the dimension from the DDL: "embedding FLOAT[N]"
        let dim = extract_vec0_dim(&ddl);
        if let Some(stored_dim) = dim
            && stored_dim != config.embedding.dimension
        {
            report.issues.push(Issue {
                kind: IssueKind::DimensionMismatch,
                entity_id: None,
                message: format!(
                    "{} vec0 table is {}-dim but config dimension is {}. \
                     Run `cogz reset` + `cogz index` to rebuild at the new dimension.",
                    table, stored_dim, config.embedding.dimension
                ),
            });
        }
    }
}

/// Extract the dimension from a vec0 DDL string like
/// "CREATE VIRTUAL TABLE ... embedding FLOAT[768] ...".
pub(crate) fn extract_vec0_dim(ddl: &str) -> Option<usize> {
    let open = ddl.find('[')?;
    let close = ddl[open..].find(']')?;
    let num_str = &ddl[open + 1..open + close];
    num_str.parse().ok()
}

/// Check for entities with corrupt properties JSON and events with
/// corrupt payload JSON. Uses `json_valid()` to detect malformed JSON
/// directly in the persisted columns — the in-memory `_corrupt_*`
/// markers set by `row_to_entity`/`row_to_event` are never written
/// back to the DB, so searching for them would never match.
pub(crate) fn check_corrupt_json(conn: &Connection, report: &mut DoctorReport) {
    // Entities: properties column must be valid JSON (or empty/null).
    // A well-formed entity has properties = '{}' or a JSON object.
    let corrupt_entities: Vec<String> = {
        let mut stmt = match conn.prepare(
            "SELECT id FROM entities \
             WHERE properties IS NOT NULL \
             AND properties != '' \
             AND json_valid(properties) = 0",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("failed to query corrupt entity JSON: {}", e);
                return;
            }
        };
        let rows = match stmt.query_map([], |r| r.get::<_, String>(0)) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("failed to query corrupt entity JSON: {}", e);
                return;
            }
        };
        rows.flatten().collect()
    };
    for id in &corrupt_entities {
        report.issues.push(Issue {
            kind: IssueKind::CorruptJson,
            entity_id: Some(id.clone()),
            message: "entity has corrupt properties JSON".to_string(),
        });
    }

    // Events: payload column must be valid JSON (or empty/null).
    let corrupt_events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events \
             WHERE payload IS NOT NULL \
             AND payload != '' \
             AND json_valid(payload) = 0",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if corrupt_events > 0 {
        report.issues.push(Issue {
            kind: IssueKind::CorruptJson,
            entity_id: None,
            message: format!(
                "{corrupt_events} event{} with corrupt payload JSON",
                if corrupt_events == 1 { "" } else { "s" }
            ),
        });
    }
}
