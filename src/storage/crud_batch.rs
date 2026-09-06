use rusqlite::{Connection, params};

use super::StorageError;

/// Mark all active code entities (function, class, file, module)
/// whose `file_path` is in `file_paths` as stale. Returns the number
/// of entities updated. This is a single batched UPDATE — far more
/// efficient than fetching all entities and updating them in a loop.
pub fn mark_code_entities_stale_by_file_paths(
    conn: &Connection,
    file_paths: &[String],
) -> Result<usize, StorageError> {
    if file_paths.is_empty() {
        return Ok(0);
    }

    // SQLITE_MAX_VARIABLE_NUMBER is 999 by default. This query uses
    // 1 (timestamp) + 4 (entity types) + N (file paths) parameters,
    // so the chunk size must be 999 - 5 = 994 to stay within the limit.
    const CHUNK_SIZE: usize = 994;
    let code_types = ["function", "class", "file", "module"];
    let type_placeholders = (0..code_types.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let now = chrono::Utc::now().to_rfc3339();
    let mut total = 0;

    for chunk in file_paths.chunks(CHUNK_SIZE) {
        let path_placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE entities SET status = 'stale', updated_at = ? \
             WHERE status = 'active' \
             AND type IN ({type_placeholders}) \
             AND file_path IN ({path_placeholders})"
        );

        let mut params: Vec<&dyn rusqlite::ToSql> =
            Vec::with_capacity(1 + code_types.len() + chunk.len());
        params.push(&now);
        for t in &code_types {
            params.push(t);
        }
        for p in chunk {
            params.push(p);
        }

        total += conn.execute(&sql, params.as_slice())?;
    }

    Ok(total)
}

/// Mark specific entities as stale by ID. Only active entities are
/// updated — the WHERE clause enforces the `active → stale` transition,
/// which is legal per the status state machine. Returns the count of
/// entities actually marked stale.
///
/// Used by incremental code sync when entities are removed from a
/// changed file (renamed or deleted within a file that still exists).
pub fn mark_entities_stale_by_ids(
    conn: &Connection,
    entity_ids: &[String],
) -> Result<usize, StorageError> {
    if entity_ids.is_empty() {
        return Ok(0);
    }

    const CHUNK_SIZE: usize = 998;
    let now = chrono::Utc::now().to_rfc3339();
    let mut total = 0;

    for chunk in entity_ids.chunks(CHUNK_SIZE) {
        if chunk.is_empty() {
            continue;
        }
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE entities SET status = 'stale', updated_at = ? \
             WHERE id IN ({placeholders}) AND status = 'active'"
        );
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(1 + chunk.len());
        params.push(&now);
        for id in chunk {
            params.push(id);
        }
        total += conn.execute(&sql, params.as_slice())?;
    }

    Ok(total)
}

/// Delete an entity by ID. FTS5 trigger fires automatically.
pub fn delete_entity(conn: &Connection, id: &str) -> Result<(), StorageError> {
    conn.execute("DELETE FROM entities WHERE id = ?1", params![id])?;
    Ok(())
}

/// Convert an entity to a tombstone: clear content, title, and hash,
/// set status to `pruned`. FTS5 UPDATE trigger removes the old text
/// from the search index and inserts an empty row (effectively
/// removing it from search results).
///
/// This is the only write operation that bypasses the file-first
/// invariant — the canonical file has already been deleted by the
/// caller (doctor pruning). The DB record is intentionally reduced to
/// a minimal tombstone to preserve graph edges and event history.
pub fn tombstone_entity(conn: &Connection, id: &str) -> Result<(), StorageError> {
    tombstone_entity_with_timestamp(conn, id, &chrono::Utc::now().to_rfc3339())
}

/// Tombstone an entity using a specific timestamp. Used by prune to
/// match the canonical file's `updated_at` so that `reset + index`
/// reproduces the same DB row.
pub fn tombstone_entity_with_timestamp(
    conn: &Connection,
    id: &str,
    updated_at: &str,
) -> Result<(), StorageError> {
    let affected = conn.execute(
        "UPDATE entities SET content = '', title = NULL, content_hash = NULL, \
         status = 'pruned', updated_at = ?1 \
         WHERE id = ?2 AND status IN ('rejected', 'superseded')",
        params![updated_at, id],
    )?;
    if affected == 0 {
        // Either the entity doesn't exist or it's not in a prunable status.
        // Check which one for a precise error.
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM entities WHERE id = ?1)",
                params![id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !exists {
            return Err(StorageError::EntityNotFound(id.to_string()));
        }
        return Err(StorageError::IllegalTransition {
            from: "current".to_string(),
            to: "pruned".to_string(),
        });
    }
    Ok(())
}

/// Delete an entity along with all edges referencing it and nullify
/// event references. Used by tombstone cleanup to remove old pruned
/// entities that exceed the retention limit.
///
/// Handles FK constraints properly: edges are deleted first, event
/// references are nullified, then the entity is deleted. The FTS5
/// DELETE trigger fires on entity deletion.
pub fn delete_entity_cascade(conn: &Connection, id: &str) -> Result<(), StorageError> {
    crate::storage::edges::delete_edges_for_entity(conn, id)?;
    // Nullify event references to avoid FK constraint violations.
    conn.execute(
        "UPDATE events SET entity_id = NULL WHERE entity_id = ?1",
        params![id],
    )?;
    conn.execute("DELETE FROM entities WHERE id = ?1", params![id])?;
    Ok(())
}

/// Batch-delete multiple entities by ID, cascading edge and event
/// cleanup. More efficient than calling `delete_entity_cascade` per
/// entity when removing a batch (e.g. excess tombstones).
pub fn delete_entities_cascade_batch(
    conn: &Connection,
    ids: &[String],
) -> Result<usize, StorageError> {
    if ids.is_empty() {
        return Ok(0);
    }

    // 499 because the edge DELETE uses placeholders twice (2x params).
    const CHUNK_SIZE: usize = 499;
    let mut total = 0;

    for chunk in ids.chunks(CHUNK_SIZE) {
        if chunk.is_empty() {
            continue;
        }
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");

        // Delete edges where the entity is source or target.
        // Placeholders appear twice (source_id IN + target_id IN),
        // so params must be duplicated.
        let edge_params: Vec<&dyn rusqlite::ToSql> = chunk
            .iter()
            .chain(chunk.iter())
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        conn.execute(
            &format!("DELETE FROM edges WHERE source_id IN ({placeholders}) OR target_id IN ({placeholders})"),
            edge_params.as_slice(),
        )?;

        // Nullify event references.
        let params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        conn.execute(
            &format!("UPDATE events SET entity_id = NULL WHERE entity_id IN ({placeholders})"),
            params.as_slice(),
        )?;

        // Delete entities.
        let n = conn.execute(
            &format!("DELETE FROM entities WHERE id IN ({placeholders})"),
            params.as_slice(),
        )?;
        total += n;
    }

    Ok(total)
}
