use std::path::{Path, PathBuf};

use crate::files::entities::{EntityFile, FileEntityType, fm_value_to_json};
use crate::files::events;
use crate::files::sync::{SyncAction, SyncError, SyncFailure, SyncResult, content_hash};
use crate::storage;
use crate::storage::crud::{ENTITY_COLUMNS, Entity, EntityType};

pub(crate) fn read_and_parse(
    file_path: &Path,
    cogz_dir: &Path,
) -> Result<(EntityFile, String, String), SyncError> {
    let raw_content = std::fs::read_to_string(file_path)?;
    let entity_file = EntityFile::from_content(&raw_content)?;
    let hash = content_hash(&raw_content);
    let relative_path = file_path
        .strip_prefix(cogz_dir)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();
    Ok((entity_file, hash, relative_path))
}

/// Sync a parsed entity file to the DB. Requires the storage lock.
pub(crate) fn sync_parsed_file(
    conn: &rusqlite::Connection,
    entity_file: &EntityFile,
    hash: &str,
    relative_path: &str,
    incremental: bool,
) -> Result<SyncAction, SyncError> {
    match storage::crud::get_entity(conn, &entity_file.id) {
        Ok(existing) => {
            if incremental && existing.content_hash.as_deref() == Some(hash) {
                return Ok(SyncAction::Skipped);
            }
            let content_changed = existing.content != entity_file.body;
            let entity = build_entity(entity_file, hash, relative_path);
            update_entity_preserving_status(conn, &existing, &entity)?;
            if content_changed {
                events::record_edit_event(conn, entity_file, &existing.content);
            }
            Ok(SyncAction::Updated)
        }
        Err(storage::StorageError::EntityNotFound(_)) => {
            let entity = build_entity(entity_file, hash, relative_path);
            storage::crud::insert_entity(conn, &entity)?;
            events::record_create_event(conn, entity_file);
            Ok(SyncAction::Created)
        }
        Err(e) => Err(SyncError::Storage(e)),
    }
}

/// Update an entity. If the file's status would require an illegal
/// transition, return an error — the caller must perform sanctioned
/// status transitions before rewriting canonical frontmatter. This
/// prevents silent divergence between the canonical file and DB.
pub(crate) fn update_entity_preserving_status(
    conn: &rusqlite::Connection,
    existing: &Entity,
    new_entity: &Entity,
) -> Result<(), SyncError> {
    if existing.status != new_entity.status
        && storage::status::transition_status(&existing.status, &new_entity.status).is_err()
    {
        return Err(SyncError::Storage(
            storage::StorageError::IllegalTransition {
                from: existing.status.clone(),
                to: new_entity.status.clone(),
            },
        ));
    }
    storage::crud::update_entity(conn, new_entity)?;
    Ok(())
}

/// Build a storage Entity from an EntityFile.
pub(crate) fn build_entity(entity_file: &EntityFile, hash: &str, relative_path: &str) -> Entity {
    let entity_type = match entity_file.entity_type {
        FileEntityType::Observation => EntityType::Observation,
        FileEntityType::Rule => EntityType::Rule,
        FileEntityType::Knowledge => EntityType::Knowledge,
    };

    let mut properties = serde_json::Map::new();
    const COMMON_FIELDS: &[&str] = &[
        "id",
        "title",
        "type",
        "status",
        "created_at",
        "updated_at",
        "references",
    ];
    for (key, value) in &entity_file.frontmatter.entries {
        if !COMMON_FIELDS.contains(&key.as_str()) {
            properties.insert(key.clone(), fm_value_to_json(value));
        }
    }

    Entity {
        id: entity_file.id.clone(),
        r#type: entity_type.as_str().to_string(),
        title: if entity_file.title.is_empty() {
            None
        } else {
            Some(entity_file.title.clone())
        },
        content: entity_file.body.clone(),
        properties: serde_json::Value::Object(properties),
        file_path: Some(relative_path.to_string()),
        status: entity_file.status.clone(),
        // Pruned tombstones have no content hash — matches
        // tombstone_entity which sets content_hash = NULL.
        content_hash: if entity_file.status == "pruned" {
            None
        } else {
            Some(hash.to_string())
        },
        created_at: entity_file.created_at.clone(),
        updated_at: entity_file.updated_at.clone(),
    }
}

/// Mark DB entities as stale if their file_path no longer exists on disk.
/// Deleted-file stale entities are a transitional DB state — after
/// `cogz reset` they are gone. References in surviving files'
/// frontmatter are the historical record; graph expansion skips
/// dangling references gracefully.
pub(crate) fn mark_deleted_as_stale(
    conn: &rusqlite::Connection,
    disk_paths: &std::collections::HashSet<PathBuf>,
    cogz_dir: &Path,
    result: &mut SyncResult,
) {
    let file_backed = match get_file_backed_entities(conn) {
        Ok(e) => e,
        Err(error) => {
            result.errors.push(SyncFailure {
                file_path: cogz_dir.to_path_buf(),
                error,
            });
            return;
        }
    };

    for entity in file_backed {
        // Only mark stale if the file is genuinely missing from disk.
        // A malformed or unreadable file is NOT a deletion.
        let entity_path = match &entity.file_path {
            Some(fp) => cogz_dir.join(fp),
            None => continue,
        };
        if !disk_paths.contains(&entity_path) && entity.status == "active" {
            match storage::crud::update_status(conn, &entity.id, "stale") {
                Ok(()) => result.marked_stale += 1,
                Err(error) => result.errors.push(SyncFailure {
                    file_path: PathBuf::from(entity.file_path.unwrap_or_default()),
                    error: SyncError::Storage(error),
                }),
            }
        }
    }
}

/// Get all file-backed entities (knowledge, rules, observations —
/// not code entities, which are derived from source files and managed
/// by the index layer).
pub(crate) fn get_file_backed_entities(
    conn: &rusqlite::Connection,
) -> Result<Vec<Entity>, SyncError> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ENTITY_COLUMNS} FROM entities \
             WHERE type IN ('observation', 'rule', 'knowledge')"
        ))
        .map_err(|e| SyncError::Storage(e.into()))?;
    let rows = stmt
        .query_map([], storage::crud::row_to_entity)
        .map_err(|e| SyncError::Storage(e.into()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| SyncError::Storage(e.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::sync_single_file;
    use crate::storage::Storage;
    use tempfile::TempDir;

    fn setup_storage(dir: &Path) -> Storage {
        let db_path = dir.join("test.db");
        Storage::open(&db_path, 768).unwrap()
    }

    #[test]
    fn sync_single_file_rejects_path_traversal() {
        let dir = TempDir::new().unwrap();
        let cogz_dir = dir.path().join(".cogz");
        std::fs::create_dir_all(&cogz_dir).unwrap();

        // Create a file outside .cogz/ that we'll try to traverse to.
        let outside = dir.path().join("secret.md");
        std::fs::write(
            &outside,
            "---\nid: test\ntitle: Test\ntype: observation\n---\nbody\n",
        )
        .unwrap();

        let storage = setup_storage(dir.path());

        // Try to traverse: ../../../secret.md
        let result = sync_single_file(&storage, &cogz_dir, "../../../secret.md");
        assert!(
            result.errors.is_empty(),
            "path traversal should be silently rejected, not errored"
        );
        assert_eq!(
            result.created, 0,
            "no entity should be created from path traversal"
        );
    }

    #[test]
    fn sync_single_file_handles_nonexistent_file() {
        let dir = TempDir::new().unwrap();
        let cogz_dir = dir.path().join(".cogz");
        std::fs::create_dir_all(&cogz_dir).unwrap();
        let storage = setup_storage(dir.path());

        let result = sync_single_file(&storage, &cogz_dir, "observations/nonexistent.md");
        assert_eq!(result.created, 0);
        assert_eq!(result.errors.len(), 0);
    }
}
