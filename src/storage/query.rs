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
pub fn fts_search(
    conn: &Connection,
    query: &str,
    type_filter: Option<&str>,
    status_filter: Option<&str>,
    limit: i64,
) -> Result<Vec<Entity>, StorageError> {
    // Mirrors ENTITY_COLUMNS with `e.` prefix for the JOIN.
    let mut sql = "SELECT e.id, e.type, e.title, e.content, e.properties, e.file_path, e.status, e.content_hash, e.created_at, e.updated_at
         FROM entities_fts fts
         JOIN entities e ON e.rowid = fts.rowid
         WHERE entities_fts MATCH ?1"
        .to_string();
    let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(query.to_string())];

    if let Some(et) = type_filter {
        sql.push_str(" AND e.type = ?");
        param_values.push(Box::new(et.to_string()));
    }
    if let Some(st) = status_filter {
        sql.push_str(" AND e.status = ?");
        param_values.push(Box::new(st.to_string()));
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

#[cfg(test)]
mod tests {
    use super::super::crud::insert_entity;
    use super::super::ensure_vec_extension;
    use super::super::schema::run_migrations;
    use super::*;

    fn setup() -> Connection {
        ensure_vec_extension();
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn query_entities_by_type() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "content a")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "rule", "B", "content b")).unwrap();

        let result = get_entities_by_type(&conn, "observation", Some("active"), 20).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "u1");
    }

    #[test]
    fn query_entities_by_status() {
        let conn = setup();
        let mut e1 = Entity::new("u1", "observation", "A", "c");
        e1.status = "stale".to_string();
        insert_entity(&conn, &e1).unwrap();
        insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

        let active = get_entities_by_type(&conn, "observation", Some("active"), 20).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "u2");

        let stale = get_entities_by_type(&conn, "observation", Some("stale"), 20).unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, "u1");
    }

    #[test]
    fn fts_search_finds_content() {
        let conn = setup();
        let e1 = Entity::new(
            "u1",
            "observation",
            "FTS5 ranking bug",
            "The RRF fusion produces incorrect rankings",
        );
        insert_entity(&conn, &e1).unwrap();
        insert_entity(
            &conn,
            &Entity::new("u2", "rule", "Unrelated", "completely different content"),
        )
        .unwrap();

        let results = fts_search(&conn, "ranking", None, Some("active"), 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "u1");
    }

    #[test]
    fn fts_search_with_type_filter() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "observation", "ranking", "ranking content"),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new("u2", "rule", "ranking", "ranking content"),
        )
        .unwrap();

        let obs = fts_search(&conn, "ranking", Some("observation"), Some("active"), 20).unwrap();
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].r#type, "observation");
    }

    #[test]
    fn count_stale_entities() {
        let conn = setup();
        let mut e1 = Entity::new("u1", "observation", "A", "c");
        e1.status = "stale".to_string();
        insert_entity(&conn, &e1).unwrap();
        insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

        assert_eq!(count_stale(&conn).unwrap(), 1);
    }

    #[test]
    fn get_entity_counts_by_type() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u3", "rule", "R", "c")).unwrap();

        let counts = entity_counts_by_type(&conn).unwrap();
        let obs = counts.iter().find(|(t, _)| t == "observation").unwrap();
        assert_eq!(obs.1, 2);
    }

    #[test]
    fn rules_sorted_by_confidence_then_recency() {
        let conn = setup();

        // Low confidence rule (0.3)
        let mut r1 = Entity::new("r1", "rule", "Low confidence", "c");
        r1.properties = serde_json::json!({"confidence": 0.3});
        insert_entity(&conn, &r1).unwrap();

        // High confidence rule (0.9)
        let mut r2 = Entity::new("r2", "rule", "High confidence", "c");
        r2.properties = serde_json::json!({"confidence": 0.9});
        insert_entity(&conn, &r2).unwrap();

        // No confidence property — defaults to 1.0
        let r3 = Entity::new("r3", "rule", "Default confidence", "c");
        insert_entity(&conn, &r3).unwrap();

        let rules = get_rules_by_confidence(&conn, Some("active"), 20).unwrap();
        // r3 (1.0) > r2 (0.9) > r1 (0.3)
        assert_eq!(rules[0].id, "r3");
        assert_eq!(rules[1].id, "r2");
        assert_eq!(rules[2].id, "r1");
    }
}
