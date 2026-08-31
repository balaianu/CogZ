//! Health check implementations for `cogz doctor`.

use std::path::Path;

use rusqlite::Connection;

use crate::config::Config;
use crate::storage::Storage;

/// A single issue found by the doctor.
#[derive(Debug, Clone)]
pub struct Issue {
    pub kind: IssueKind,
    pub entity_id: Option<String>,
    pub message: String,
}

/// Category of issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// DB entity exists but file is missing on disk.
    MissingFile,
    /// File exists but DB entity is missing (not synced).
    MissingEntity,
    /// DB entity is not marked stale but file hash differs.
    StaleNotFlagged,
    /// Observation content was edited (append-only violation).
    ObservationEdited,
    /// Rule content was substantively changed.
    RuleEdited,
    /// Entity status transitioned illegally.
    IllegalStatusTransition,
    /// Rule is superseded but no derived_from edge to a new rule.
    OrphanedSupersede,
    /// Two knowledge entries have embedding similarity > 0.80.
    NearDuplicate,
}

impl std::fmt::Display for IssueKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFile => write!(f, "missing_file"),
            Self::MissingEntity => write!(f, "missing_entity"),
            Self::StaleNotFlagged => write!(f, "stale_not_flagged"),
            Self::ObservationEdited => write!(f, "observation_edited"),
            Self::RuleEdited => write!(f, "rule_edited"),
            Self::IllegalStatusTransition => write!(f, "illegal_status_transition"),
            Self::OrphanedSupersede => write!(f, "orphaned_supersede"),
            Self::NearDuplicate => write!(f, "near_duplicate"),
        }
    }
}

/// Full doctor report.
#[derive(Debug, Default)]
pub struct DoctorReport {
    pub issues: Vec<Issue>,
    pub db_healthy: bool,
    pub schema_version: u32,
    pub entity_count: i64,
    pub edge_count: i64,
    pub models_available: bool,
}

/// Run all health checks and return a report.
pub fn run_doctor(
    storage: &Storage,
    config: &Config,
    cogz_dir: &Path,
    repo_root: &Path,
) -> DoctorReport {
    let mut report = DoctorReport::default();
    let conn = storage.conn();

    // DB integrity checks
    report.schema_version = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);
    report.entity_count = crate::storage::crud::count_all(&conn).unwrap_or(0);
    report.edge_count = crate::storage::edges::count_edges(&conn).unwrap_or(0);
    report.db_healthy = check_integrity(&conn);

    // Model availability
    let models_dir = crate::embed::models_dir();
    report.models_available = check_models(config, &models_dir);

    // File sync consistency
    check_file_sync(&conn, cogz_dir, repo_root, &mut report);

    // Policy violations
    check_observation_edits(&conn, cogz_dir, &mut report);
    check_orphaned_supersedes(&conn, &mut report);

    // Near-duplicate knowledge (requires embeddings)
    check_near_duplicates(&conn, &mut report);

    report
}

fn check_integrity(conn: &Connection) -> bool {
    let result: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap_or_else(|_| "error".to_string());
    result == "ok"
}

fn check_models(config: &Config, models_dir: &Path) -> bool {
    let code_ok = crate::embed::is_model_cached(&config.embedding.code_model, models_dir);
    let knowledge_ok = crate::embed::is_model_cached(&config.embedding.knowledge_model, models_dir);
    code_ok && knowledge_ok
}

fn check_file_sync(
    conn: &Connection,
    cogz_dir: &Path,
    repo_root: &Path,
    report: &mut DoctorReport,
) {
    // Find DB entities with file_path that no longer exist on disk.
    // File-backed entities (observation, rule, knowledge) have paths
    // relative to cogz_dir. Code entities (function, class, file,
    // module) have paths relative to repo_root.
    let sql = "SELECT id, type, file_path FROM entities WHERE file_path IS NOT NULL AND status != 'pruned'";
    if let Ok(mut stmt) = conn.prepare(sql)
        && let Ok(rows) = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let etype: String = row.get(1)?;
            let file_path: String = row.get(2)?;
            Ok((id, etype, file_path))
        })
    {
        for row in rows.flatten() {
            let (id, etype, file_path) = row;
            let base = match etype.as_str() {
                "function" | "class" | "file" | "module" => repo_root,
                _ => cogz_dir,
            };
            let full_path = base.join(&file_path);
            if !full_path.exists() {
                report.issues.push(Issue {
                    kind: IssueKind::MissingFile,
                    entity_id: Some(id),
                    message: format!("file missing: {}", file_path),
                });
            }
        }
    }

    // Find files on disk that have no DB entity (not synced).
    // Batch-query all file paths at once instead of one query per file.
    let entity_files = crate::files::scan_entity_files(cogz_dir);
    let rel_paths: Vec<String> = entity_files
        .iter()
        .map(|p| {
            p.strip_prefix(cogz_dir)
                .unwrap_or(p)
                .to_string_lossy()
                .to_string()
        })
        .collect();

    let synced_paths: std::collections::HashSet<String> =
        batch_query_file_paths(conn, &rel_paths).unwrap_or_default();

    for rel in &rel_paths {
        if !synced_paths.contains(rel) {
            report.issues.push(Issue {
                kind: IssueKind::MissingEntity,
                entity_id: None,
                message: format!("file not synced: {}", rel),
            });
        }
    }
}

/// Batch-query which file paths exist in the entities table. Returns
/// the set of paths that have a matching entity. Chunks to respect
/// SQLite's variable number limit.
fn batch_query_file_paths(
    conn: &Connection,
    paths: &[String],
) -> Result<std::collections::HashSet<String>, rusqlite::Error> {
    if paths.is_empty() {
        return Ok(std::collections::HashSet::new());
    }

    const CHUNK_SIZE: usize = 999;
    let mut result = std::collections::HashSet::new();

    for chunk in paths.chunks(CHUNK_SIZE) {
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql =
            format!("SELECT DISTINCT file_path FROM entities WHERE file_path IN ({placeholders})");
        let params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0))?;
        for row in rows {
            result.insert(row?);
        }
    }

    Ok(result)
}

fn check_observation_edits(conn: &Connection, cogz_dir: &Path, report: &mut DoctorReport) {
    // Observations are append-only. If the file's content hash differs
    // from the DB's content_hash, the observation was edited.
    let sql = "SELECT id, file_path, content_hash FROM entities WHERE type = 'observation' AND file_path IS NOT NULL AND status != 'pruned'";
    if let Ok(mut stmt) = conn.prepare(sql)
        && let Ok(rows) = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let file_path: String = row.get(1)?;
            let db_hash: Option<String> = row.get(2)?;
            Ok((id, file_path, db_hash))
        })
    {
        for row in rows.flatten() {
            let (id, file_path, db_hash) = row;
            let full_path = cogz_dir.join(&file_path);
            if !full_path.exists() {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&full_path) {
                let file_hash = crate::files::content_hash(&content);
                if let Some(db_h) = &db_hash
                    && db_h != &file_hash
                {
                    report.issues.push(Issue {
                        kind: IssueKind::ObservationEdited,
                        entity_id: Some(id),
                        message: format!(
                            "observation content hash mismatch (file was edited): {}",
                            file_path
                        ),
                    });
                }
            }
        }
    }
}

fn check_orphaned_supersedes(conn: &Connection, report: &mut DoctorReport) {
    // A superseded entity should have a `supersedes` or `superseded_by`
    // edge pointing to the entity that replaced it. If no such edge
    // exists, it's an orphaned supersede.
    let sql = "SELECT id, type FROM entities WHERE status = 'superseded'";
    if let Ok(mut stmt) = conn.prepare(sql)
        && let Ok(rows) = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let etype: String = row.get(1)?;
            Ok((id, etype))
        })
    {
        for row in rows.flatten() {
            let (id, etype) = row;
            // Check for any edge connecting this entity to another
            // via supersedes/superseded_by/derived_from.
            let has_edge: bool = conn
                    .query_row(
                        "SELECT 1 FROM edges WHERE (source_id = ?1 OR target_id = ?1) AND edge_type IN ('supersedes', 'superseded_by', 'derived_from') LIMIT 1",
                        [&id],
                        |_| Ok(true),
                    )
                    .unwrap_or(false);
            if !has_edge {
                report.issues.push(Issue {
                    kind: IssueKind::OrphanedSupersede,
                    entity_id: Some(id),
                    message: format!(
                        "{} is superseded but has no supersede/derived_from edge",
                        etype
                    ),
                });
            }
        }
    }
}

fn check_near_duplicates(conn: &Connection, report: &mut DoctorReport) {
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
    let embeddings = fetch_embeddings_batch(conn, &knowledge_ids);
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

/// Fetch embeddings for multiple entity IDs in a single query.
/// Returns (entity_id, embedding) pairs. Chunked to respect SQLite
/// variable limits.
fn fetch_embeddings_batch(conn: &Connection, ids: &[String]) -> Vec<(String, Vec<f32>)> {
    let chunk_size = 998; // SQLITE_MAX_VARIABLE_NUMBER / 1 param per row
    let mut result = Vec::new();

    for chunk in ids.chunks(chunk_size) {
        let placeholders: Vec<&str> = (0..chunk.len()).map(|_| "?").collect();
        let sql = format!(
            "SELECT entity_id, embedding FROM entity_embeddings WHERE entity_id IN ({})",
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
            for row in rows.flatten() {
                result.push(row);
            }
        }
    }

    result
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}
