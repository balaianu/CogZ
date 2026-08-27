//! Entity CRUD operations and the `Entity` struct.

use std::str::FromStr;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::StorageError;

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
        "SELECT id, type, title, content, properties, file_path, status, content_hash, created_at, updated_at
         FROM entities WHERE id = ?1",
        params![id],
        row_to_entity,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => StorageError::EntityNotFound(id.to_string()),
        other => StorageError::Sqlite(other),
    })
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

/// Delete an entity by ID. FTS5 trigger fires automatically.
pub fn delete_entity(conn: &Connection, id: &str) -> Result<(), StorageError> {
    conn.execute("DELETE FROM entities WHERE id = ?1", params![id])?;
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

pub(super) fn row_to_entity(row: &rusqlite::Row<'_>) -> Result<Entity, rusqlite::Error> {
    let props_str: String = row.get(4)?;
    let properties = serde_json::from_str(&props_str).unwrap_or(serde_json::json!({}));
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
mod tests {
    use super::*;

    fn setup() -> Connection {
        super::super::ensure_vec_extension();
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn insert_and_get_entity() {
        let conn = setup();
        let entity = Entity::new("uuid-1", "observation", "Test", "Content here");
        insert_entity(&conn, &entity).unwrap();

        let fetched = get_entity(&conn, "uuid-1").unwrap();
        assert_eq!(fetched.id, "uuid-1");
        assert_eq!(fetched.r#type, "observation");
        assert_eq!(fetched.title, Some("Test".to_string()));
        assert_eq!(fetched.content, "Content here");
        assert_eq!(fetched.status, "active");
    }

    #[test]
    fn get_nonexistent_entity_errors() {
        let conn = setup();
        let result = get_entity(&conn, "nonexistent");
        assert!(matches!(result, Err(StorageError::EntityNotFound(_))));
    }

    #[test]
    fn update_entity_content() {
        let conn = setup();
        let mut entity = Entity::new("uuid-1", "knowledge", "Title", "Old content");
        insert_entity(&conn, &entity).unwrap();

        entity.content = "New content".to_string();
        update_entity(&conn, &entity).unwrap();

        let fetched = get_entity(&conn, "uuid-1").unwrap();
        assert_eq!(fetched.content, "New content");
    }

    #[test]
    fn update_status_legal_transition() {
        let conn = setup();
        let entity = Entity::new("uuid-1", "observation", "Test", "Content");
        insert_entity(&conn, &entity).unwrap();

        update_status(&conn, "uuid-1", "stale").unwrap();
        let fetched = get_entity(&conn, "uuid-1").unwrap();
        assert_eq!(fetched.status, "stale");
    }

    #[test]
    fn update_status_illegal_transition() {
        let conn = setup();
        let entity = Entity::new("uuid-1", "observation", "Test", "Content");
        insert_entity(&conn, &entity).unwrap();

        update_status(&conn, "uuid-1", "rejected").unwrap();

        let result = update_status(&conn, "uuid-1", "active");
        assert!(matches!(
            result,
            Err(StorageError::IllegalTransition { .. })
        ));
    }

    #[test]
    fn delete_entity_removes_from_fts() {
        let conn = setup();
        let entity = Entity::new("uuid-1", "observation", "Searchable", "unique content");
        insert_entity(&conn, &entity).unwrap();

        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 1);

        delete_entity(&conn, "uuid-1").unwrap();

        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 0);
    }

    #[test]
    fn count_entities_by_type() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u3", "rule", "R", "c")).unwrap();

        assert_eq!(count_by_type(&conn, "observation").unwrap(), 2);
        assert_eq!(count_by_type(&conn, "rule").unwrap(), 1);
        assert_eq!(count_by_type(&conn, "knowledge").unwrap(), 0);
    }

    #[test]
    fn entity_type_roundtrip() {
        assert_eq!(
            EntityType::parse("observation").unwrap(),
            EntityType::Observation
        );
        assert_eq!(EntityType::Observation.as_str(), "observation");
        assert!(EntityType::from_str("invalid").is_err());
    }
}
