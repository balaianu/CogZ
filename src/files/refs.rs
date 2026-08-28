//! Reference edge synchronization — manages `references` edges
//! derived from entity file frontmatter.

use rusqlite::Connection;

use crate::storage::StorageError;
use crate::storage::edges::{Edge, delete_edges_by_source_and_type, insert_edge_skip_fk_violation};

use super::entities::EntityFile;

/// Sync `references` edges from the entity file to the DB.
/// Replaces all existing `references` edges for this entity.
///
/// Forward references to not-yet-synced entities are silently
/// skipped — FK constraints reject them, and they'll be created
/// when the target entity is synced.
pub fn sync_references(conn: &Connection, entity_file: &EntityFile) -> Result<(), StorageError> {
    delete_edges_by_source_and_type(conn, &entity_file.id, "references")?;

    let now = chrono::Utc::now().to_rfc3339();
    for ref_id in &entity_file.references {
        let edge = Edge {
            source_id: entity_file.id.clone(),
            target_id: ref_id.clone(),
            edge_type: "references".to_string(),
            weight: 1.0,
            created_at: now.clone(),
        };
        insert_edge_skip_fk_violation(conn, &edge)?;
    }

    Ok(())
}
