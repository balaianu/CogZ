//! Entity CRUD operations and the `Entity` struct.

use std::str::FromStr;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[path = "crud_batch.rs"]
mod crud_batch;

pub use crud_batch::{
    delete_entities_cascade_batch, delete_entity, delete_entity_cascade,
    mark_code_entities_stale_by_file_paths, mark_entities_stale_by_ids, tombstone_entity,
    tombstone_entity_with_timestamp,
};

use super::StorageError;

/// Column list for SELECT queries that return full Entity rows.
/// Used by `get_entity`, `query_entities`, `fts_search`, and
/// `get_file_backed_entities` to avoid column-list drift.
pub const ENTITY_COLUMNS: &str =
    "id, type, title, content, properties, file_path, status, content_hash, created_at, updated_at";

/// Valid entity types. Code entities (function, class, file, module)
/// are extracted from source; knowledge entities (observation, rule,
/// knowledge) are file-backed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Observation,
    Rule,
    Knowledge,
    Function,
    Class,
    File,
    Module,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Rule => "rule",
            Self::Knowledge => "knowledge",
            Self::Function => "function",
            Self::Class => "class",
            Self::File => "file",
            Self::Module => "module",
        }
    }

    pub fn parse(s: &str) -> Result<Self, StorageError> {
        Self::from_str(s).map_err(|_| StorageError::InvalidEntityType(s.to_string()))
    }

    /// Whether this entity type is extracted from source code rather
    /// than file-backed markdown. Determines which embedding model to use.
    pub fn is_code(&self) -> bool {
        matches!(
            self,
            Self::Function | Self::Class | Self::File | Self::Module
        )
    }
}

impl std::fmt::Display for EntityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EntityType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "observation" => Ok(Self::Observation),
            "rule" => Ok(Self::Rule),
            "knowledge" => Ok(Self::Knowledge),
            "function" => Ok(Self::Function),
            "class" => Ok(Self::Class),
            "file" => Ok(Self::File),
            "module" => Ok(Self::Module),
            _ => Err(format!("invalid entity type: {s}")),
        }
    }
}

/// A row in the `entities` table. All entity types share this struct.
/// Type-specific fields live in `properties` as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub r#type: String,
    pub title: Option<String>,
    pub content: String,
    pub properties: serde_json::Value,
    pub file_path: Option<String>,
    pub status: String,
    pub content_hash: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Entity {
    /// Create a new active entity with empty properties and default status.
    pub fn new(id: &str, entity_type: &str, title: &str, content: &str) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: id.to_string(),
            r#type: entity_type.to_string(),
            title: Some(title.to_string()),
            content: content.to_string(),
            properties: serde_json::json!({}),
            file_path: None,
            status: "active".to_string(),
            content_hash: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Get the entity type as a typed enum.
    pub fn entity_type(&self) -> Result<EntityType, StorageError> {
        EntityType::parse(&self.r#type)
    }
}

// --- Entity CRUD ---

/// Insert a new entity. FTS5 trigger fires automatically.
pub fn insert_entity(conn: &Connection, entity: &Entity) -> Result<(), StorageError> {
    conn.execute(
        r#"INSERT INTO entities
           (id, type, title, content, properties, file_path, status, content_hash, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
        params![
            entity.id,
            entity.r#type,
            entity.title,
            entity.content,
            entity.properties.to_string(),
            entity.file_path,
            entity.status,
            entity.content_hash,
            entity.created_at,
            entity.updated_at,
        ],
    )?;
    Ok(())
}

/// Get an entity by ID.
pub fn get_entity(conn: &Connection, id: &str) -> Result<Entity, StorageError> {
    conn.query_row(
        &format!("SELECT {ENTITY_COLUMNS} FROM entities WHERE id = ?1"),
        params![id],
        row_to_entity,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => StorageError::EntityNotFound(id.to_string()),
        other => StorageError::Sqlite(other),
    })
}

/// Get multiple entities by ID in a single query. Returns only the
/// entities that exist — missing IDs are silently skipped. Use this
/// instead of calling `get_entity` in a loop.
pub fn get_entities_batch(conn: &Connection, ids: &[String]) -> Result<Vec<Entity>, StorageError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // Chunk to respect SQLITE_MAX_VARIABLE_NUMBER (999 default).
    const CHUNK_SIZE: usize = 999;

    let mut entities = Vec::new();
    for chunk in ids.chunks(CHUNK_SIZE) {
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let sql = format!("SELECT {ENTITY_COLUMNS} FROM entities WHERE id IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), row_to_entity)?;
        for row in rows {
            entities.push(row?);
        }
    }
    Ok(entities)
}

/// Update an entity's content, title, properties, status, content_hash,
/// and updated_at. FTS5 trigger fires automatically on UPDATE.
pub fn update_entity(conn: &Connection, entity: &Entity) -> Result<(), StorageError> {
    let affected = conn.execute(
        r#"UPDATE entities SET
           title = ?2, content = ?3, properties = ?4, file_path = ?5,
           status = ?6, content_hash = ?7, updated_at = ?8
           WHERE id = ?1"#,
        params![
            entity.id,
            entity.title,
            entity.content,
            entity.properties.to_string(),
            entity.file_path,
            entity.status,
            entity.content_hash,
            entity.updated_at,
        ],
    )?;
    if affected == 0 {
        return Err(StorageError::EntityNotFound(entity.id.clone()));
    }
    Ok(())
}

/// Update only the status field (frontmatter-only change). Validates
/// the transition through the state machine.
pub fn update_status(conn: &Connection, id: &str, new_status: &str) -> Result<(), StorageError> {
    let current: String = conn
        .query_row(
            "SELECT status FROM entities WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::EntityNotFound(id.to_string()),
            other => StorageError::Sqlite(other),
        })?;

    super::status::transition_status(&current, new_status)?;

    let now = chrono::Utc::now().to_rfc3339();
    let affected = conn.execute(
        "UPDATE entities SET status = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, new_status, now],
    )?;
    if affected == 0 {
        return Err(StorageError::EntityNotFound(id.to_string()));
    }
    Ok(())
}

/// Count entities by type.
pub fn count_by_type(conn: &Connection, entity_type: &str) -> Result<i64, StorageError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE type = ?1",
        params![entity_type],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// Count all entities.
pub fn count_all(conn: &Connection) -> Result<i64, StorageError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))?;
    Ok(count)
}

// --- Helper ---

pub fn row_to_entity(row: &rusqlite::Row<'_>) -> Result<Entity, rusqlite::Error> {
    let props_str: String = row.get(4)?;
    let properties = serde_json::from_str(&props_str).unwrap_or_else(|e| {
        tracing::warn!("malformed entity properties JSON: {}", e);
        serde_json::json!({ "_corrupt_properties": props_str })
    });
    Ok(Entity {
        id: row.get(0)?,
        r#type: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        properties,
        file_path: row.get(5)?,
        status: row.get(6)?,
        content_hash: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests;
