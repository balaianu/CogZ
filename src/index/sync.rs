//! Code entity synchronization — insert/update/stale-mark code entities.
//!
//! Code entities (function, class, file, module) are not file-backed.
//! They are extracted from source code by tree-sitter and exist only
//! in the DB. Their UUIDs are deterministic (UUID v5) so that
//! `cogz reset` + `cogz index` produces identical IDs, preserving
//! `references` edges from observations.
//!
//! Sync rules mirror file sync:
//! - New entity → insert into DB, index in FTS
//! - Changed entity (content hash differs) → update DB entity
//! - Deleted source file → mark all its code entities as stale

use std::collections::HashMap;
use std::path::Path;

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::index::tree_sitter::Language;
use crate::storage;
use crate::storage::crud::{Entity, EntityType};

/// UUID v5 namespace for CogZ code entities. Deterministic across
/// rebuilds — the same source file + entity name always maps to the
/// same UUID.
const CODE_ENTITY_NAMESPACE: Uuid = Uuid::from_bytes([
    0xc0, 0x9e, 0x4a, 0x7b, 0x1f, 0x2d, 0x4b, 0x8c,
    0xa3, 0x56, 0xd1, 0xe4, 0xf2, 0x8a, 0x9c, 0x6b,
]);

/// Result of a code entity sync operation.
#[derive(Debug, Default)]
pub struct CodeSyncResult {
    pub created: usize,
    pub updated: usize,
    pub marked_stale: usize,
    pub skipped: usize,
    /// IDs of entities that were created or updated (for embedding).
    pub synced_entity_ids: Vec<String>,
}

/// Generate a deterministic UUID v5 for a code entity.
///
/// The name is `{file_path}:{entity_type}:{qualified_name}`. This
/// ensures the same code entity always gets the same UUID, even after
/// a full DB rebuild.
pub fn code_entity_uuid(file_path: &str, entity_type: &str, qualified_name: &str) -> String {
    let name = format!("{file_path}:{entity_type}:{qualified_name}");
    Uuid::new_v5(&CODE_ENTITY_NAMESPACE, name.as_bytes()).to_string()
}

/// Compute SHA-256 hash of entity content.
fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Sync code entities from a set of source files to the database.
///
/// `source_files` is a list of (relative_path, source_code, language)
/// tuples. The function parses each file, extracts entities, and
/// syncs them to the DB. Entities whose source file is no longer in
/// the list are marked stale.
///
/// File reads and parsing happen without holding the storage mutex;
/// DB operations acquire it in a second phase.
pub fn sync_code_entities(
    storage: &storage::Storage,
    repo_root: &Path,
    source_files: &[(std::path::PathBuf, String, Language)],
) -> CodeSyncResult {
    let mut result = CodeSyncResult::default();

    // Phase 1: parse all source files (no lock held).
    let mut all_entities: Vec<(String, Entity)> = Vec::new();
    let mut file_to_entity_ids: HashMap<String, Vec<String>> = HashMap::new();

    for (rel_path, source, language) in source_files {
        let abs_path = repo_root.join(rel_path);
        let entities = crate::index::tree_sitter::extract_entities(
            &abs_path,
            source,
            *language,
        );

        for ce in entities {
            let qualified_name = ce
                .properties
                .get("qualified_name")
                .and_then(|v| v.as_str())
                .or_else(|| ce.properties.get("module_path").and_then(|v| v.as_str()))
                .unwrap_or(&ce.title)
                .to_string();

            let file_path_str = rel_path.to_string_lossy().to_string();
            let id = code_entity_uuid(&file_path_str, ce.entity_type, &qualified_name);
            let hash = content_hash(&ce.content);

            let entity_type = match ce.entity_type {
                "function" => EntityType::Function,
                "class" => EntityType::Class,
                "file" => EntityType::File,
                "module" => EntityType::Module,
                _ => continue,
            };

            let now = chrono::Utc::now().to_rfc3339();
            let entity = Entity {
                id: id.clone(),
                r#type: entity_type.as_str().to_string(),
                title: Some(ce.title.clone()),
                content: ce.content.clone(),
                properties: ce.properties.clone(),
                file_path: Some(file_path_str.clone()),
                status: "active".to_string(),
                content_hash: Some(hash),
                created_at: now.clone(),
                updated_at: now,
            };

            file_to_entity_ids
                .entry(file_path_str)
                .or_default()
                .push(id.clone());
            all_entities.push((id.clone(), entity));
        }
    }

    // Phase 2: DB operations (lock held).
    let conn = storage.conn();

    // Sync each entity: insert or update.
    for (id, entity) in &all_entities {
        match storage::crud::get_entity(&conn, id) {
            Ok(existing) => {
                if existing.status == "stale" {
                    // Reactivate stale entity even if content is unchanged.
                    let mut updated = entity.clone();
                    updated.status = "active".to_string();
                    updated.created_at = existing.created_at.clone();
                    if let Err(e) = storage::crud::update_entity(&conn, &updated) {
                        tracing::warn!("failed to reactivate code entity {}: {}", id, e);
                        continue;
                    }
                    result.updated += 1;
                    result.synced_entity_ids.push(id.clone());
                    continue;
                }
                if existing.content_hash == entity.content_hash {
                    result.skipped += 1;
                    continue;
                }
                // Update content, properties, hash. Preserve status.
                let mut updated = entity.clone();
                updated.status = existing.status.clone();
                updated.created_at = existing.created_at.clone();
                if let Err(e) = storage::crud::update_entity(&conn, &updated) {
                    tracing::warn!("failed to update code entity {}: {}", id, e);
                    continue;
                }
                result.updated += 1;
                result.synced_entity_ids.push(id.clone());
            }
            Err(storage::StorageError::EntityNotFound(_)) => {
                if let Err(e) = storage::crud::insert_entity(&conn, entity) {
                    tracing::warn!("failed to insert code entity {}: {}", id, e);
                    continue;
                }
                result.created += 1;
                result.synced_entity_ids.push(id.clone());
            }
            Err(e) => {
                tracing::warn!("failed to query code entity {}: {}", id, e);
            }
        }
    }

    // Mark stale: code entities whose file_path is not in the current
    // source file set. We query all code entities and check.
    result.marked_stale = mark_stale_code_entities(&conn, &file_to_entity_ids);

    // Record last code index timestamp.
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(e) = storage::set_meta(&conn, "last_code_index", &now) {
        tracing::warn!("failed to record last_code_index: {}", e);
    }

    result
}

/// Mark code entities as stale if their source file is no longer present.
///
/// Returns the number of entities marked stale.
fn mark_stale_code_entities(
    conn: &rusqlite::Connection,
    current_files: &HashMap<String, Vec<String>>,
) -> usize {
    // Get all active code entities grouped by file_path.
    let code_types = ["function", "class", "file", "module"];
    let mut stale_count = 0;

    for entity_type in &code_types {
        let entities = match storage::query::get_entities_by_type(
            conn,
            entity_type,
            Some("active"),
            100_000,
        ) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entity in entities {
            if let Some(ref fp) = entity.file_path {
                if !current_files.contains_key(fp) {
                    // Source file is gone — mark stale
                    let mut updated = entity.clone();
                    updated.status = "stale".to_string();
                    updated.updated_at = chrono::Utc::now().to_rfc3339();
                    if storage::crud::update_entity(conn, &updated).is_ok() {
                        stale_count += 1;
                    }
                }
            }
        }
    }

    stale_count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    #[test]
    fn uuid_is_deterministic() {
        let id1 = code_entity_uuid("src/main.rs", "function", "main");
        let id2 = code_entity_uuid("src/main.rs", "function", "main");
        assert_eq!(id1, id2);
    }

    #[test]
    fn uuid_differs_by_entity() {
        let id1 = code_entity_uuid("src/main.rs", "function", "main");
        let id2 = code_entity_uuid("src/main.rs", "function", "helper");
        assert_ne!(id1, id2);
    }

    #[test]
    fn uuid_differs_by_file() {
        let id1 = code_entity_uuid("src/main.rs", "function", "main");
        let id2 = code_entity_uuid("src/lib.rs", "function", "main");
        assert_ne!(id1, id2);
    }

    #[test]
    fn uuid_differs_by_type() {
        let id1 = code_entity_uuid("src/main.rs", "function", "foo");
        let id2 = code_entity_uuid("src/main.rs", "class", "foo");
        assert_ne!(id1, id2);
    }

    #[test]
    fn sync_inserts_code_entities() {
        let storage = Storage::open_memory().unwrap();
        let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let files = vec![(
            std::path::PathBuf::from("src/math.rs"),
            code.to_string(),
            Language::Rust,
        )];

        let result = sync_code_entities(&storage, Path::new("."), &files);
        assert_eq!(result.created, 2); // file + function
        assert_eq!(result.updated, 0);
        assert_eq!(result.marked_stale, 0);

        let conn = storage.conn();
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 2);
    }

    #[test]
    fn sync_skips_unchanged_entities() {
        let storage = Storage::open_memory().unwrap();
        let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let files = vec![(
            std::path::PathBuf::from("src/math.rs"),
            code.to_string(),
            Language::Rust,
        )];

        // First sync — creates
        let result1 = sync_code_entities(&storage, Path::new("."), &files);
        assert_eq!(result1.created, 2);

        // Second sync — skips (same content)
        let result2 = sync_code_entities(&storage, Path::new("."), &files);
        assert_eq!(result2.created, 0);
        assert_eq!(result2.skipped, 2);
    }

    #[test]
    fn sync_updates_changed_entities() {
        let storage = Storage::open_memory().unwrap();
        let code1 = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let code2 = "fn add(a: i32, b: i32) -> i64 { (a + b) as i64 }";
        let files1 = vec![(
            std::path::PathBuf::from("src/math.rs"),
            code1.to_string(),
            Language::Rust,
        )];

        // First sync
        sync_code_entities(&storage, Path::new("."), &files1);

        // Second sync with changed content
        let files2 = vec![(
            std::path::PathBuf::from("src/math.rs"),
            code2.to_string(),
            Language::Rust,
        )];
        let result2 = sync_code_entities(&storage, Path::new("."), &files2);
        assert_eq!(result2.updated, 2);
        assert_eq!(result2.skipped, 0);
    }

    #[test]
    fn sync_marks_stale_when_file_removed() {
        let storage = Storage::open_memory().unwrap();
        let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let files = vec![(
            std::path::PathBuf::from("src/math.rs"),
            code.to_string(),
            Language::Rust,
        )];

        // First sync — creates
        sync_code_entities(&storage, Path::new("."), &files);

        // Second sync — empty file list, should mark stale
        let result = sync_code_entities(&storage, Path::new("."), &[]);
        assert_eq!(result.marked_stale, 2);
    }

    #[test]
    fn sync_reactivates_stale_entities() {
        let storage = Storage::open_memory().unwrap();
        let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let files = vec![(
            std::path::PathBuf::from("src/math.rs"),
            code.to_string(),
            Language::Rust,
        )];

        // First sync — creates
        sync_code_entities(&storage, Path::new("."), &files);

        // Second sync — empty, marks stale
        sync_code_entities(&storage, Path::new("."), &[]);

        // Third sync — file is back, should reactivate
        let result = sync_code_entities(&storage, Path::new("."), &files);
        assert_eq!(result.updated, 2);
        assert_eq!(result.marked_stale, 0);

        // Verify status is active again
        let conn = storage.conn();
        let active_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entities WHERE status = 'active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active_count, 2);
    }

    #[test]
    fn rebuild_produces_same_uuids() {
        let storage = Storage::open_memory().unwrap();
        let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let files = vec![(
            std::path::PathBuf::from("src/math.rs"),
            code.to_string(),
            Language::Rust,
        )];

        // First sync
        sync_code_entities(&storage, Path::new("."), &files);
        let conn = storage.conn();
        let ids_before: Vec<String> = conn
            .prepare("SELECT id FROM entities ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        // Reset and rebuild
        drop(conn);
        let storage2 = Storage::open_memory().unwrap();
        sync_code_entities(&storage2, Path::new("."), &files);
        let conn2 = storage2.conn();
        let ids_after: Vec<String> = conn2
            .prepare("SELECT id FROM entities ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(ids_before, ids_after);
    }
}
