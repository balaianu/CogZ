//! Schema definitions, migrations, and version tracking.

use rusqlite::Connection;

use super::StorageError;

/// Current schema version. Increment when migrations are added.
/// Stored in `PRAGMA user_version`.
pub const SCHEMA_VERSION: u32 = 4;

/// Run all migrations to bring the database up to `SCHEMA_VERSION`.
///
/// Migrations are forward-only. On a fresh database, all tables are
/// created. On an existing database, only new migrations run.
///
/// `embedding_dim` is used for the vec0 virtual table dimension.
/// It must match the configured model dimension.
pub fn run_migrations(conn: &Connection, embedding_dim: usize) -> Result<(), StorageError> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    if current >= SCHEMA_VERSION {
        return Ok(());
    }

    if current < 1 {
        migrate_v1(conn, embedding_dim)?;
    }

    if current < 2 {
        migrate_v2(conn)?;
    }

    if current < 3 {
        migrate_v3(conn, embedding_dim)?;
    }

    if current < 4 {
        migrate_v4(conn)?;
    }

    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// Verify the database schema version matches what the binary expects.
pub fn check_version(conn: &Connection) -> Result<(), StorageError> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current != SCHEMA_VERSION {
        return Err(StorageError::SchemaVersionMismatch {
            db: current,
            expected: SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Migration v1: initial schema — all tables, indexes, FTS5 triggers.
fn migrate_v1(conn: &Connection, embedding_dim: usize) -> Result<(), StorageError> {
    // --- Entities table ---
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS entities (
            id           TEXT PRIMARY KEY,
            type         TEXT NOT NULL,
            title        TEXT,
            content      TEXT NOT NULL,
            properties   TEXT DEFAULT '{}',
            file_path    TEXT,
            status       TEXT DEFAULT 'active',
            content_hash TEXT,
            created_at   TEXT NOT NULL,
            updated_at   TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_entities_type      ON entities(type);
        CREATE INDEX IF NOT EXISTS idx_entities_status    ON entities(status);
        CREATE INDEX IF NOT EXISTS idx_entities_file_path ON entities(file_path);
        "#,
    )?;

    // --- FTS5 external content table ---
    conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS entities_fts USING fts5(
            title,
            content,
            content='entities',
            content_rowid='rowid',
            tokenize='porter unicode61'
        );
        "#,
    )?;

    // --- FTS5 sync triggers ---
    conn.execute_batch(
        r#"
        CREATE TRIGGER IF NOT EXISTS entities_fts_ai AFTER INSERT ON entities BEGIN
            INSERT INTO entities_fts(rowid, title, content)
            VALUES (new.rowid, new.title, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS entities_fts_ad AFTER DELETE ON entities BEGIN
            INSERT INTO entities_fts(entities_fts, rowid, title, content)
            VALUES('delete', old.rowid, old.title, old.content);
        END;

        CREATE TRIGGER IF NOT EXISTS entities_fts_au AFTER UPDATE ON entities BEGIN
            INSERT INTO entities_fts(entities_fts, rowid, title, content)
            VALUES('delete', old.rowid, old.title, old.content);
            INSERT INTO entities_fts(rowid, title, content)
            VALUES (new.rowid, new.title, new.content);
        END;
        "#,
    )?;

    // --- Edges table ---
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS edges (
            source_id   TEXT NOT NULL REFERENCES entities(id),
            target_id   TEXT NOT NULL REFERENCES entities(id),
            edge_type   TEXT NOT NULL,
            weight      REAL DEFAULT 1.0,
            created_at  TEXT NOT NULL,
            PRIMARY KEY (source_id, target_id, edge_type)
        );

        CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source_id, edge_type);
        CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_id, edge_type);
        "#,
    )?;

    // --- Embeddings (separate vec0 tables for code and knowledge) ---
    // Code and knowledge entities use different embedding models with
    // incompatible vector spaces even at the same dimensionality. Separate
    // tables ensure KNN only compares vectors within the same space.
    let code_vec0 = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS code_embeddings USING vec0(\n            embedding FLOAT[{}],\n            entity_id TEXT\n        );",
        embedding_dim
    );
    conn.execute_batch(&code_vec0)?;

    let knowledge_vec0 = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_embeddings USING vec0(\n            embedding FLOAT[{}],\n            entity_id TEXT\n        );",
        embedding_dim
    );
    conn.execute_batch(&knowledge_vec0)?;

    // --- Events table ---
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS events (
            id          INTEGER PRIMARY KEY,
            event_type  TEXT NOT NULL,
            entity_id   TEXT REFERENCES entities(id),
            payload     TEXT DEFAULT '{}',
            created_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_events_type   ON events(event_type);
        CREATE INDEX IF NOT EXISTS idx_events_entity ON events(entity_id);
        "#,
    )?;

    // --- Entity access tracking (derived, not in canonical files) ---
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS entity_access (
            entity_id    TEXT PRIMARY KEY REFERENCES entities(id),
            access_count INTEGER DEFAULT 0,
            last_accessed TEXT
        );
        "#,
    )?;

    Ok(())
}

/// Migration v2: meta table for key-value metadata (last_index, etc.).
fn migrate_v2(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

/// Migration v3: split entity_embeddings into code_embeddings and
/// knowledge_embeddings. Existing embeddings are migrated to the
/// correct table based on entity type, then the old table is dropped.
///
/// On fresh databases (v1 already creates the two new tables), the
/// old table doesn't exist so the migration is a no-op.
fn migrate_v3(conn: &Connection, embedding_dim: usize) -> Result<(), StorageError> {
    // Ensure the new tables exist (idempotent — v1 already creates them
    // on fresh databases, but this covers databases at v2 that predate
    // the split).
    let code_vec0 = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS code_embeddings USING vec0(\n            embedding FLOAT[{}],\n            entity_id TEXT\n        );",
        embedding_dim
    );
    conn.execute_batch(&code_vec0)?;

    let knowledge_vec0 = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_embeddings USING vec0(\n            embedding FLOAT[{}],\n            entity_id TEXT\n        );",
        embedding_dim
    );
    conn.execute_batch(&knowledge_vec0)?;

    // Migrate existing embeddings from the old single table if it exists.
    // vec0 tables don't support INSERT INTO ... SELECT across virtual
    // tables, so we read all embeddings in Rust and re-insert them.
    let has_old_table: bool = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='entity_embeddings'")
        .map_err(StorageError::from)?
        .exists([])?;

    if has_old_table {
        use crate::storage::crud::EntityType;
        use zerocopy::IntoBytes;

        // Read all (entity_id, embedding) pairs from the old table.
        let rows: Vec<(String, Vec<u8>, String)> = {
            let mut stmt = conn.prepare(
                "SELECT e.entity_id, e.embedding, ent.type
                 FROM entity_embeddings e
                 JOIN entities ent ON ent.id = e.entity_id",
            )?;
            let row_iter = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            row_iter.filter_map(|r| r.ok()).collect()
        };

        for (entity_id, blob, entity_type) in rows {
            let is_code = EntityType::parse(&entity_type)
                .map(|t| t.is_code())
                .unwrap_or(false);
            let table = if is_code {
                "code_embeddings"
            } else {
                "knowledge_embeddings"
            };
            let sql = format!(
                "INSERT INTO {}(entity_id, embedding) VALUES (?1, ?2)",
                table
            );
            // blob is already raw f32 bytes — re-insert as-is.
            let _: usize = conn.execute(&sql, rusqlite::params![entity_id, blob.as_bytes()])?;
        }

        // Drop the old table.
        conn.execute_batch("DROP TABLE IF EXISTS entity_embeddings")?;
    }

    Ok(())
}

/// Migration v4: entity_access table for tracking retrieval frequency.
/// Derived state — not in canonical files. Reset to 0 on DB rebuild.
fn migrate_v4(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS entity_access (
            entity_id    TEXT PRIMARY KEY REFERENCES entities(id),
            access_count INTEGER DEFAULT 0,
            last_accessed TEXT
        );
        "#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_db_has_all_tables() {
        let conn = Connection::open_in_memory().unwrap();
        crate::storage::ensure_vec_extension();
        run_migrations(&conn, 768).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"entities".to_string()));
        assert!(tables.contains(&"edges".to_string()));
        assert!(tables.contains(&"events".to_string()));
        assert!(tables.contains(&"meta".to_string()));
        // FTS5 and vec0 tables appear as virtual tables
        assert!(tables.iter().any(|t| t.contains("entities_fts")));
        assert!(tables.iter().any(|t| t.contains("code_embeddings")));
        assert!(tables.iter().any(|t| t.contains("knowledge_embeddings")));
    }

    #[test]
    fn fts_triggers_sync_on_insert() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, 768).unwrap();

        conn.execute(
            "INSERT INTO entities (id, type, title, content, properties, status, created_at, updated_at)
             VALUES ('test-1', 'observation', 'Test Title', ' searchable content here ', '{}', 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 1);
    }

    #[test]
    fn fts_triggers_sync_on_delete() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, 768).unwrap();

        conn.execute(
            "INSERT INTO entities (id, type, title, content, properties, status, created_at, updated_at)
             VALUES ('test-1', 'observation', 'Test', 'content here', '{}', 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM entities WHERE id = 'test-1'", [])
            .unwrap();

        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 0);
    }

    #[test]
    fn fts_triggers_sync_on_update() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, 768).unwrap();

        conn.execute(
            "INSERT INTO entities (id, type, title, content, properties, status, created_at, updated_at)
             VALUES ('test-1', 'observation', 'Old Title', 'old content', '{}', 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        conn.execute(
            "UPDATE entities SET title = 'New Title', content = 'new content' WHERE id = 'test-1'",
            [],
        )
        .unwrap();

        let title: String = conn
            .query_row(
                "SELECT title FROM entities_fts WHERE entities_fts MATCH 'new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "New Title");
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, 768).unwrap();
        // Running again should not error
        run_migrations(&conn, 768).unwrap();
    }
}
