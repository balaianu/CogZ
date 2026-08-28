//! Domain event recording for file sync operations.
//!
//! Records creation and edit events per entity-spec.md policy:
//! - New entity → `*_created` event
//! - Content body change → `*_edited` / `*_updated` event
//!   (observation/rule body edits are policy-relevant)

use rusqlite::Connection;

use crate::storage::events::EventType;

use super::entities::{EntityFile, FileEntityType};

/// Record a domain event for entity creation.
pub fn record_create_event(conn: &Connection, entity_file: &EntityFile) {
    let event_type = match entity_file.entity_type {
        FileEntityType::Observation => EventType::ObservationCreated,
        FileEntityType::Rule => EventType::RuleCreated,
        FileEntityType::Knowledge => EventType::KnowledgeCreated,
    };
    let payload = serde_json::json!({"title": entity_file.title});
    let _ = crate::storage::events::record_event(conn, event_type, Some(&entity_file.id), &payload);
}

/// Record a domain event for entity content edit.
pub fn record_edit_event(conn: &Connection, entity_file: &EntityFile, old_content: &str) {
    let event_type = match entity_file.entity_type {
        FileEntityType::Observation => EventType::ObservationEdited,
        FileEntityType::Rule => EventType::RuleEdited,
        FileEntityType::Knowledge => EventType::KnowledgeUpdated,
    };
    let payload = serde_json::json!({"old_content": old_content});
    let _ = crate::storage::events::record_event(conn, event_type, Some(&entity_file.id), &payload);
}
