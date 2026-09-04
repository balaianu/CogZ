//! Read queries — by type, status, file_path, and FTS5 search.

use rusqlite::{Connection, params};

use super::StorageError;
use super::crud::{ENTITY_COLUMNS, Entity, row_to_entity};

/// Filter criteria for querying entities.
pub struct EntityFilter<'a> {
    pub entity_type: Option<&'a str>,
    pub status: Option<&'a str>,
    pub file_path: Option<&'a str>,
    pub references: Option<&'a str>,
    pub limit: i64,
}

impl Default for EntityFilter<'_> {
    fn default() -> Self {
        Self {
            entity_type: None,
            status: Some("active"),
            file_path: None,
            references: None,
            limit: 20,
        }
    }
}

/// Query entities with optional filters. Returns matching entities
/// sorted by `updated_at` descending (most recent first).
pub fn query_entities(
    conn: &Connection,
    filter: &EntityFilter<'_>,
) -> Result<Vec<Entity>, StorageError> {
    let mut sql = format!("SELECT {ENTITY_COLUMNS} FROM entities WHERE 1=1");
    let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(et) = filter.entity_type {
        sql.push_str(" AND type = ?");
        param_values.push(Box::new(et.to_string()));
    }
    if let Some(st) = filter.status {
        sql.push_str(" AND status = ?");
        param_values.push(Box::new(st.to_string()));
    }
    if let Some(fp) = filter.file_path {
        sql.push_str(" AND file_path = ?");
        param_values.push(Box::new(fp.to_string()));
    }
    if let Some(ref_id) = filter.references {
        sql.push_str(
            " AND id IN (SELECT source_id FROM edges WHERE target_id = ? AND edge_type = 'references')",
        );
        param_values.push(Box::new(ref_id.to_string()));
    }

    sql.push_str(" ORDER BY updated_at DESC LIMIT ?");
    param_values.push(Box::new(filter.limit));

    let params_refs: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_refs.as_slice(), row_to_entity)?;
    let mut entities = Vec::new();
    for row in rows {
        entities.push(row?);
    }
    Ok(entities)
}

/// Get entities by type, optionally filtered by status.
pub fn get_entities_by_type(
    conn: &Connection,
    entity_type: &str,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<Entity>, StorageError> {
    query_entities(
        conn,
        &EntityFilter {
            entity_type: Some(entity_type),
            status,
            file_path: None,
            references: None,
            limit,
        },
    )
}

/// Get rules sorted by confidence (descending) then recency.
/// Confidence is stored in the JSON `properties` column; rules
/// without an explicit confidence default to 1.0.
pub fn get_rules_by_confidence(
    conn: &Connection,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<Entity>, StorageError> {
    let mut sql = format!("SELECT {ENTITY_COLUMNS} FROM entities WHERE type = 'rule'");
    let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(st) = status {
        sql.push_str(" AND status = ?");
        param_values.push(Box::new(st.to_string()));
    }

    sql.push_str(
        " ORDER BY COALESCE(json_extract(properties, '$.confidence'), 1.0) DESC, updated_at DESC LIMIT ?",
    );
    param_values.push(Box::new(limit));

    let params_refs: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_refs.as_slice(), row_to_entity)?;
    let mut entities = Vec::new();
    for row in rows {
        entities.push(row?);
    }
    Ok(entities)
}

/// Get knowledge entities with optional category and tags filters
/// pushed into SQL. Tags uses "any match" semantics — an entity is
/// included if any of its tags matches any of the provided tags.
pub fn get_knowledge_filtered(
    conn: &Connection,
    status: Option<&str>,
    category: Option<&str>,
    tags: Option<&[String]>,
    limit: i64,
) -> Result<Vec<Entity>, StorageError> {
    let mut sql = format!("SELECT {ENTITY_COLUMNS} FROM entities WHERE type = 'knowledge'");
    let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(st) = status {
        sql.push_str(" AND status = ?");
        param_values.push(Box::new(st.to_string()));
    }
    if let Some(cat) = category {
        sql.push_str(" AND json_extract(properties, '$.category') = ?");
        param_values.push(Box::new(cat.to_string()));
    }
    if let Some(tags) = tags
        && !tags.is_empty()
    {
        let placeholders: String = tags.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM json_each(json_extract(properties, '$.tags')) WHERE value IN ({placeholders}))"
        ));
        for t in tags {
            param_values.push(Box::new(t.clone()));
        }
    }

    sql.push_str(" ORDER BY updated_at DESC LIMIT ?");
    param_values.push(Box::new(limit));

    let params_refs: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_refs.as_slice(), row_to_entity)?;
    let mut entities = Vec::new();
    for row in rows {
        entities.push(row?);
    }
    Ok(entities)
}

/// FTS5 search over entity titles and content.
///
/// Returns entities matching the query, filtered by type and status.
/// When `exclude_tests` is true, code entities whose `file_path`
/// matches test patterns are excluded from results.
pub fn fts_search(
    conn: &Connection,
    query: &str,
    type_filter: Option<&str>,
    status_filter: Option<&str>,
    exclude_tests: bool,
    limit: i64,
) -> Result<Vec<Entity>, StorageError> {
    // Escape FTS5 special characters by wrapping each term in double
    // quotes. FTS5 treats -, *, :, (, ), etc. as operators. Without
    // escaping, queries like "tree-sitter" or "error-handling" fail
    // with SQL errors or produce wrong results. Terms are joined with
    // OR so that documents matching any term are returned — FTS5's
    // BM25 ranking naturally prioritizes documents matching more
    // terms. This is critical for code retrieval: code entities are
    // short and dense, so a 4-word query with AND semantics would
    // exclude most relevant functions that match only 1-2 terms.
    let escaped_query = query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");

    // Mirrors ENTITY_COLUMNS with `e.` prefix for the JOIN.
    let mut sql = "SELECT e.id, e.type, e.title, e.content, e.properties, e.file_path, e.status, e.content_hash, e.created_at, e.updated_at
         FROM entities_fts fts
         JOIN entities e ON e.rowid = fts.rowid
         WHERE entities_fts MATCH ?1"
        .to_string();
    let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(escaped_query)];

    if let Some(et) = type_filter {
        sql.push_str(" AND e.type = ?");
        param_values.push(Box::new(et.to_string()));
    }
    if let Some(st) = status_filter {
        sql.push_str(" AND e.status = ?");
        param_values.push(Box::new(st.to_string()));
    }
    if exclude_tests {
        // Exclude test code using language-specific path conventions.
        // These patterns mirror `is_test_file` in `index/gitignore.rs` —
        // keep them in sync when adding new languages. Knowledge entities
        // (observations, rules, knowledge) have NULL file_path and are
        // never excluded by this filter.
        sql.push_str(
            " AND (e.file_path IS NULL \
             OR NOT (\
             e.file_path LIKE 'tests/%' \
             OR e.file_path LIKE '%/tests/%' \
             OR e.file_path LIKE '%/tests.rs' \
             OR e.file_path LIKE '%_tests.rs' \
             OR e.file_path LIKE '%_test.go' \
             OR e.file_path LIKE '%/test_%.py' \
             OR e.file_path LIKE '%/_test.py' \
             OR e.file_path LIKE '%.test.js' \
             OR e.file_path LIKE '%.spec.js' \
             OR e.file_path LIKE '%.test.mjs' \
             OR e.file_path LIKE '%.spec.mjs' \
             OR e.file_path LIKE '%.test.cjs' \
             OR e.file_path LIKE '%.spec.cjs' \
             OR e.file_path LIKE '%.test.jsx' \
             OR e.file_path LIKE '%.spec.jsx' \
             OR e.file_path LIKE '%.test.ts' \
             OR e.file_path LIKE '%.spec.ts' \
             OR e.file_path LIKE '%.test.tsx' \
             OR e.file_path LIKE '%.spec.tsx' \
             OR e.file_path LIKE '%/__tests__/%' \
             OR e.file_path LIKE '%/test_%.sh' \
             OR e.file_path LIKE '%/_test.sh' \
             OR e.file_path LIKE '%/test_%.bash' \
             OR e.file_path LIKE '%/_test.bash'))",
        );
    }

    sql.push_str(" ORDER BY rank LIMIT ?");
    param_values.push(Box::new(limit));

    let params_refs: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_refs.as_slice(), row_to_entity)?;
    let mut entities = Vec::new();
    for row in rows {
        entities.push(row?);
    }
    Ok(entities)
}

/// Count entities by status.
pub fn count_by_status(conn: &Connection, status: &str) -> Result<i64, StorageError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE status = ?1",
        params![status],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// Count stale entities.
pub fn count_stale(conn: &Connection) -> Result<i64, StorageError> {
    count_by_status(conn, "stale")
}

/// Count active entities of a specific type. Used by source balancing
/// to normalize FTS match counts by collection size.
pub fn count_active_by_type(conn: &Connection, entity_type: &str) -> Result<i64, StorageError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE type = ?1 AND status = 'active'",
        params![entity_type],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// Count active code entities (function, class, file, module).
pub fn count_active_code(conn: &Connection) -> Result<i64, StorageError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE type IN ('function','class','file','module') AND status = 'active'",
        [],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// Count active knowledge entities (knowledge, observation, rule).
pub fn count_active_knowledge(conn: &Connection) -> Result<i64, StorageError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE type IN ('knowledge','observation','rule') AND status = 'active'",
        [],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// Get entity counts grouped by type. Returns (type, count) pairs.
pub fn entity_counts_by_type(conn: &Connection) -> Result<Vec<(String, i64)>, StorageError> {
    let mut stmt =
        conn.prepare("SELECT type, COUNT(*) FROM entities GROUP BY type ORDER BY type")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut counts = Vec::new();
    for row in rows {
        counts.push(row?);
    }
    Ok(counts)
}

/// A candidate for pruning — an observation with terminal status
/// (rejected or superseded) that has a file path and is old enough
/// to prune based on the configured retention threshold.
pub struct PruneCandidateRow {
    pub entity_id: String,
    pub file_path: String,
    pub status: String,
    pub updated_at: String,
}

/// Find observations eligible for pruning. Returns entities with
/// status `rejected` or `superseded` that have a file path. The
/// caller filters by age using the `updated_at` timestamp.
pub fn find_prune_candidates(conn: &Connection) -> Result<Vec<PruneCandidateRow>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, file_path, status, updated_at FROM entities \
         WHERE type = 'observation' \
         AND status IN ('rejected', 'superseded') \
         AND file_path IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(PruneCandidateRow {
            entity_id: r.get(0)?,
            file_path: r.get(1)?,
            status: r.get(2)?,
            updated_at: r.get(3)?,
        })
    })?;
    let mut candidates = Vec::new();
    for row in rows {
        candidates.push(row?);
    }
    Ok(candidates)
}

/// Get the IDs of the oldest tombstoned (pruned) entities, ordered
/// by `updated_at` ascending. Used by tombstone limit enforcement
/// to identify which tombstones to remove first.
pub fn get_oldest_tombstone_ids(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id FROM entities WHERE status = 'pruned' \
         ORDER BY updated_at ASC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| r.get::<_, String>(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests;
