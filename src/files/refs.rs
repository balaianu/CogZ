//! Edge synchronization — manages file-backed edges derived from
//! entity file frontmatter.
//!
//! Synced edge types and their frontmatter sources:
//! - `references` ← `references` field (array of UUIDs)
//! - `supports` ← `supporting_ids` field (array of UUIDs)
//! - `contradicts` ← `contradicts` field (array of UUIDs)
//! - `derived_from` ← `derived_from` field (single UUID string)

use rusqlite::Connection;

use crate::storage::StorageError;
use crate::storage::edges::{Edge, delete_edges_by_source_and_type, insert_edge_skip_fk_violation};

use super::entities::EntityFile;

/// Sync file-backed edges from the entity file to the DB.
/// Replaces all existing edges of these types for this entity.
///
/// Forward references to not-yet-synced entities are silently
/// skipped — FK constraints reject them, and they'll be created
/// when the target entity is synced.
pub fn sync_references(conn: &Connection, entity_file: &EntityFile) -> Result<(), StorageError> {
    delete_edges_by_source_and_type(conn, &entity_file.id, "references")?;
    delete_edges_by_source_and_type(conn, &entity_file.id, "supports")?;
    delete_edges_by_source_and_type(conn, &entity_file.id, "contradicts")?;
    delete_edges_by_source_and_type(conn, &entity_file.id, "derived_from")?;

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

    // Sync supports edges from supporting_ids frontmatter field.
    if let Some(supporting) = entity_file
        .frontmatter
        .get("supporting_ids")
        .and_then(|v| v.as_array())
    {
        for target_id in supporting {
            let edge = Edge {
                source_id: entity_file.id.clone(),
                target_id: target_id.clone(),
                edge_type: "supports".to_string(),
                weight: 1.0,
                created_at: now.clone(),
            };
            insert_edge_skip_fk_violation(conn, &edge)?;
        }
    }

    // Sync contradicts edges from contradicts frontmatter field.
    if let Some(contradicts) = entity_file
        .frontmatter
        .get("contradicts")
        .and_then(|v| v.as_array())
    {
        for target_id in contradicts {
            let edge = Edge {
                source_id: entity_file.id.clone(),
                target_id: target_id.clone(),
                edge_type: "contradicts".to_string(),
                weight: 1.0,
                created_at: now.clone(),
            };
            insert_edge_skip_fk_violation(conn, &edge)?;
        }
    }

    // Sync derived_from edge from derived_from frontmatter field.
    if let Some(derived_from) = entity_file
        .frontmatter
        .get("derived_from")
        .and_then(|v| v.as_str())
    {
        let edge = Edge {
            source_id: entity_file.id.clone(),
            target_id: derived_from.to_string(),
            edge_type: "derived_from".to_string(),
            weight: 1.0,
            created_at: now.clone(),
        };
        insert_edge_skip_fk_violation(conn, &edge)?;
    }

    Ok(())
}
