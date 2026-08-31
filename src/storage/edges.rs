//! Edge CRUD operations — graph relationships between entities.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::StorageError;

/// A row in the `edges` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub source_id: String,
    pub target_id: String,
    pub edge_type: String,
    pub weight: f64,
    pub created_at: String,
}

/// Insert an edge. If the edge already exists (same source, target,
/// type), it is replaced (upsert).
pub fn insert_edge(conn: &Connection, edge: &Edge) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO edges (source_id, target_id, edge_type, weight, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            edge.source_id,
            edge.target_id,
            edge.edge_type,
            edge.weight,
            edge.created_at,
        ],
    )?;
    Ok(())
}

/// Delete an edge.
pub fn delete_edge(
    conn: &Connection,
    source_id: &str,
    target_id: &str,
    edge_type: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "DELETE FROM edges WHERE source_id = ?1 AND target_id = ?2 AND edge_type = ?3",
        params![source_id, target_id, edge_type],
    )?;
    Ok(())
}

/// Delete all edges where the entity is either source or target.
/// Used by tombstone cleanup before deleting an entity.
pub fn delete_edges_for_entity(conn: &Connection, id: &str) -> Result<(), StorageError> {
    conn.execute(
        "DELETE FROM edges WHERE source_id = ?1 OR target_id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Delete all edges of a given type from a source entity.
pub fn delete_edges_by_source_and_type(
    conn: &Connection,
    source_id: &str,
    edge_type: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "DELETE FROM edges WHERE source_id = ?1 AND edge_type = ?2",
        params![source_id, edge_type],
    )?;
    Ok(())
}

/// Delete all edges of the given types.
///
/// Used by code indexing to clear structural edges (calls, imports,
/// extends) before re-inserting from a fresh AST pass. This prevents
/// stale edges from accumulating when source code changes.
pub fn delete_edges_by_type(conn: &Connection, edge_types: &[&str]) -> Result<(), StorageError> {
    if edge_types.is_empty() {
        return Ok(());
    }
    let placeholders = (0..edge_types.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let params: Vec<&dyn rusqlite::ToSql> = edge_types
        .iter()
        .map(|t| t as &dyn rusqlite::ToSql)
        .collect();
    conn.execute(
        &format!("DELETE FROM edges WHERE edge_type IN ({placeholders})"),
        params.as_slice(),
    )?;
    Ok(())
}

/// Delete structural edges (calls, imports, extends) where the source
/// entity is in the given set. Used by incremental reindex to clear
/// only edges from changed files before re-inserting.
pub fn delete_structural_edges_by_sources(
    conn: &Connection,
    source_ids: &[String],
) -> Result<(), StorageError> {
    if source_ids.is_empty() {
        return Ok(());
    }
    let edge_types = ["calls", "imports", "extends"];
    let id_placeholders = (0..source_ids.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let type_placeholders = (0..edge_types.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "DELETE FROM edges WHERE source_id IN ({id_placeholders}) AND edge_type IN ({type_placeholders})"
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = source_ids
        .iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();
    params.extend(edge_types.iter().map(|t| t as &dyn rusqlite::ToSql));
    conn.execute(&sql, params.as_slice())?;
    Ok(())
}

/// Insert an edge, silently skipping FK constraint violations.
///
/// Used during file sync when a reference points to an entity that
/// hasn't been synced yet. The edge will be created when the target
/// entity is synced and its references are processed.
pub fn insert_edge_skip_fk_violation(conn: &Connection, edge: &Edge) -> Result<(), StorageError> {
    let result = conn.execute(
        "INSERT OR IGNORE INTO edges (source_id, target_id, edge_type, weight, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            edge.source_id,
            edge.target_id,
            edge.edge_type,
            edge.weight,
            edge.created_at,
        ],
    );
    if let Err(e) = result
        && !matches!(
            &e,
            rusqlite::Error::SqliteFailure(ffi, _)
                if ffi.code == rusqlite::ffi::ErrorCode::ConstraintViolation
        )
    {
        return Err(e.into());
    }
    Ok(())
}

/// Get the edge type between two entities, checking both directions.
/// Returns `Some(edge_type)` if an edge exists (either a→b or b→a),
/// `None` if no edge connects them.
pub fn get_edge_type_between(
    conn: &Connection,
    a: &str,
    b: &str,
) -> Result<Option<String>, StorageError> {
    conn.query_row(
        "SELECT edge_type FROM edges
         WHERE (source_id = ?1 AND target_id = ?2)
            OR (source_id = ?2 AND target_id = ?1)
         LIMIT 1",
        params![a, b],
        |r| r.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other.into()),
    })
}

/// Count all edges.
pub fn count_edges(conn: &Connection) -> Result<i64, StorageError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
    Ok(count)
}

/// Get all edges from a source entity.
pub fn get_edges_from(conn: &Connection, source_id: &str) -> Result<Vec<Edge>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT source_id, target_id, edge_type, weight, created_at
         FROM edges WHERE source_id = ?1 ORDER BY edge_type",
    )?;
    let rows = stmt.query_map(params![source_id], |row| {
        Ok(Edge {
            source_id: row.get(0)?,
            target_id: row.get(1)?,
            edge_type: row.get(2)?,
            weight: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    let mut edges = Vec::new();
    for row in rows {
        edges.push(row?);
    }
    Ok(edges)
}

/// Get all edges to a target entity.
pub fn get_edges_to(conn: &Connection, target_id: &str) -> Result<Vec<Edge>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT source_id, target_id, edge_type, weight, created_at
         FROM edges WHERE target_id = ?1 ORDER BY edge_type",
    )?;
    let rows = stmt.query_map(params![target_id], |row| {
        Ok(Edge {
            source_id: row.get(0)?,
            target_id: row.get(1)?,
            edge_type: row.get(2)?,
            weight: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    let mut edges = Vec::new();
    for row in rows {
        edges.push(row?);
    }
    Ok(edges)
}

/// Batched neighbor lookup — get all node IDs directly connected to
/// any of the given node IDs, in a single query per direction.
///
/// Returns `(outgoing_targets, incoming_sources)` where
/// `outgoing_targets` are nodes reachable via outgoing edges from
/// the frontier, and `incoming_sources` are nodes with edges pointing
/// into the frontier. Use this instead of calling `get_edges_from` /
/// `get_edges_to` per node when traversing a frontier.
pub fn get_neighbors_batch(
    conn: &Connection,
    node_ids: &[String],
) -> Result<(Vec<String>, Vec<String>), StorageError> {
    if node_ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    // Chunk to respect SQLITE_MAX_VARIABLE_NUMBER (999 default).
    const CHUNK_SIZE: usize = 999;

    let mut outgoing = Vec::new();
    let mut incoming = Vec::new();

    for chunk in node_ids.chunks(CHUNK_SIZE) {
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

        let sql_out =
            format!("SELECT DISTINCT target_id FROM edges WHERE source_id IN ({placeholders})");
        {
            let mut stmt = conn.prepare(&sql_out)?;
            let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0))?;
            for row in rows {
                outgoing.push(row?);
            }
        }

        let sql_in =
            format!("SELECT DISTINCT source_id FROM edges WHERE target_id IN ({placeholders})");
        {
            let mut stmt = conn.prepare(&sql_in)?;
            let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0))?;
            for row in rows {
                incoming.push(row?);
            }
        }
    }

    Ok((outgoing, incoming))
}

#[cfg(test)]
mod tests;
