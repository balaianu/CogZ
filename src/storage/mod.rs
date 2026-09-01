//! Storage layer — SQLite database with entity, edge, FTS5, vec0, and
//! events tables.
//!
//! Single `Connection` behind `std::sync::Mutex`. DB calls from async
//! MCP handlers go through `tokio::task::spawn_blocking` (Phase 7).

pub mod access;
pub mod crud;
pub mod edges;
pub mod embeddings;
pub mod events;
pub mod graph;
pub mod query;
pub mod schema;
pub mod status;

pub use crud::{Entity, EntityType};
pub use edges::Edge;
pub use events::{DomainEvent, EventType};
pub use schema::SCHEMA_VERSION;

use std::path::Path;
use std::sync::{Mutex, Once};

use rusqlite::{Connection, ffi::sqlite3_auto_extension};
use sqlite_vec::sqlite3_vec_init;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("illegal status transition: {from} → {to}")]
    IllegalTransition { from: String, to: String },
    #[error("entity not found: {0}")]
    EntityNotFound(String),
    #[error("invalid entity type: {0}")]
    InvalidEntityType(String),
    #[error("schema version mismatch: db has {db}, binary expects {expected}")]
    SchemaVersionMismatch { db: u32, expected: u32 },
    #[error("file operation failed: {0}")]
    File(String),
}

/// Ensures sqlite-vec extension is registered exactly once before any
/// Connection is opened.
static VEC_INIT: Once = Once::new();

pub(crate) fn ensure_vec_extension() {
    VEC_INIT.call_once(|| {
        // SAFETY: sqlite3_vec_init is the C entrypoint for the sqlite-vec
        // extension. sqlite3_auto_extension registers it so every new
        // Connection automatically loads vec0. This is the standard
        // pattern documented in the sqlite-vec Rust guide.
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut std::os::raw::c_char,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::os::raw::c_int,
            >(sqlite3_vec_init as *const ())));
        }
    });
}

/// The storage layer. Owns a single SQLite connection behind a Mutex.
pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    /// Open or create a database at the given path.
    ///
    /// Enables WAL mode, registers sqlite-vec, runs migrations, and
    /// verifies the schema version. `embedding_dim` configures the
    /// vec0 virtual table dimension and must match the model.
    pub fn open(path: &Path, embedding_dim: usize) -> Result<Self, StorageError> {
        ensure_vec_extension();

        let conn = if path == Path::new(":memory:") {
            Connection::open_in_memory()?
        } else {
            if let Some(parent) = path.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                tracing::warn!("failed to create DB directory {}: {}", parent.display(), e);
            }
            Connection::open(path)?
        };

        // WAL mode for crash recovery and checkpoint behavior.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        schema::run_migrations(&conn, embedding_dim)?;
        schema::check_version(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory database (for tests). Uses 768-dim embeddings.
    pub fn open_memory() -> Result<Self, StorageError> {
        Self::open(Path::new(":memory:"), 768)
    }

    /// Access the underlying connection. For internal use within
    /// storage module functions. Recovers from a poisoned mutex
    /// rather than panicking — a panic in one task should not kill
    /// the entire server.
    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Database file size in bytes (0 for in-memory).
    pub fn db_size_bytes(&self) -> u64 {
        // Get the file path under the lock, then drop it before
        // doing filesystem I/O to avoid blocking other callers.
        let path = {
            let conn = self.conn();
            let path: Option<String> = conn
                .query_row("PRAGMA database_list", [], |r| r.get::<_, String>(2))
                .ok();
            path
        };
        match path {
            Some(ref p) if !p.is_empty() => std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
            _ => 0,
        }
    }
}

/// Set a metadata key-value pair in the `meta` table.
pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Get a metadata value by key. Returns None if the key doesn't exist.
pub fn get_meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_memory_creates_schema() {
        let storage = Storage::open_memory().unwrap();
        let conn = storage.conn();
        // Verify tables exist
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(count >= 5, "expected at least 5 tables, got {count}");
    }

    #[test]
    fn open_memory_schema_version() {
        let storage = Storage::open_memory().unwrap();
        let conn = storage.conn();
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn open_file_based_db() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::open(&db_path, 768).unwrap();
        let conn = storage.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn wal_mode_enabled() {
        let storage = Storage::open_memory().unwrap();
        let conn = storage.conn();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        // In-memory DBs use "memory" journal, file DBs use "wal"
        assert!(mode == "wal" || mode == "memory", "got journal_mode={mode}");
    }
}
