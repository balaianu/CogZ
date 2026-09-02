//! Domain event recording.
//!
//! Only domain events are recorded — never indexing operations.
//! This keeps the events table small and meaningful.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::StorageError;

/// Domain event types (from `docs/architecture.md` schema).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    ObservationCreated,
    ObservationEdited,
    ObservationRejected,
    RuleCreated,
    RuleEdited,
    RulePromoted,
    KnowledgeCreated,
    KnowledgeUpdated,
    KnowledgeMerged,
    ContradictionFound,
    CodeChanged,
    SessionStart,
    PromptSubmit,
    PreToolUse,
    PostToolUse,
    FileSave,
    SessionEnd,
    Stop,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ObservationCreated => "observation_created",
            Self::ObservationEdited => "observation_edited",
            Self::ObservationRejected => "observation_rejected",
            Self::RuleCreated => "rule_created",
            Self::RuleEdited => "rule_edited",
            Self::RulePromoted => "rule_promoted",
            Self::KnowledgeCreated => "knowledge_created",
            Self::KnowledgeUpdated => "knowledge_updated",
            Self::KnowledgeMerged => "knowledge_merged",
            Self::ContradictionFound => "contradiction_found",
            Self::CodeChanged => "code_changed",
            Self::SessionStart => "session_start",
            Self::PromptSubmit => "prompt_submit",
            Self::PreToolUse => "pre_tool_use",
            Self::PostToolUse => "post_tool_use",
            Self::FileSave => "file_save",
            Self::SessionEnd => "session_end",
            Self::Stop => "stop",
        }
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A row in the `events` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEvent {
    pub id: i64,
    pub event_type: String,
    pub entity_id: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: String,
}

/// Record a domain event. Returns the inserted event ID.
pub fn record_event(
    conn: &Connection,
    event_type: EventType,
    entity_id: Option<&str>,
    payload: &serde_json::Value,
) -> Result<i64, StorageError> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO events (event_type, entity_id, payload, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![event_type.as_str(), entity_id, payload.to_string(), now,],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Get events for a specific entity, ordered by creation time.
pub fn get_events_for_entity(
    conn: &Connection,
    entity_id: &str,
) -> Result<Vec<DomainEvent>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, entity_id, payload, created_at
         FROM events WHERE entity_id = ?1 ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map(params![entity_id], row_to_event)?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}

/// Get recent events of a specific type.
pub fn get_recent_events(
    conn: &Connection,
    event_type: &str,
    limit: i64,
) -> Result<Vec<DomainEvent>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, entity_id, payload, created_at
         FROM events WHERE event_type = ?1 ORDER BY created_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![event_type, limit], row_to_event)?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}

/// Count all events.
pub fn count_events(conn: &Connection) -> Result<i64, StorageError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
    Ok(count)
}

fn row_to_event(row: &rusqlite::Row<'_>) -> Result<DomainEvent, rusqlite::Error> {
    let payload_str: String = row.get(3)?;
    let payload = serde_json::from_str(&payload_str).unwrap_or_else(|e| {
        tracing::warn!("malformed event payload JSON: {}", e);
        serde_json::json!({ "_corrupt_payload": payload_str })
    });
    Ok(DomainEvent {
        id: row.get(0)?,
        event_type: row.get(1)?,
        entity_id: row.get(2)?,
        payload,
        created_at: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::super::crud::Entity;
    use super::super::crud::insert_entity;
    use super::super::ensure_vec_extension;
    use super::super::schema::run_migrations;
    use super::*;

    fn setup() -> Connection {
        ensure_vec_extension();
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, 768).unwrap();
        conn
    }

    #[test]
    fn record_and_get_event() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "Test", "content")).unwrap();

        let payload = serde_json::json!({"reason": "duplicate detected"});
        let id = record_event(&conn, EventType::ObservationCreated, Some("u1"), &payload).unwrap();
        assert!(id > 0);

        let events = get_events_for_entity(&conn, "u1").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "observation_created");
        assert_eq!(events[0].entity_id, Some("u1".to_string()));
        assert_eq!(events[0].payload["reason"], "duplicate detected");
    }

    #[test]
    fn record_event_without_entity() {
        let conn = setup();
        let payload = serde_json::json!({"detail": "system startup"});
        record_event(&conn, EventType::CodeChanged, None, &payload).unwrap();

        let events = get_recent_events(&conn, "code_changed", 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].entity_id.is_none());
    }

    #[test]
    fn count_all_events() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "T", "c")).unwrap();

        record_event(
            &conn,
            EventType::ObservationCreated,
            Some("u1"),
            &serde_json::json!({}),
        )
        .unwrap();
        record_event(
            &conn,
            EventType::ObservationRejected,
            Some("u1"),
            &serde_json::json!({}),
        )
        .unwrap();

        assert_eq!(count_events(&conn).unwrap(), 2);
    }

    #[test]
    fn get_recent_events_filtered_by_type() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "T", "c")).unwrap();

        record_event(
            &conn,
            EventType::ObservationCreated,
            Some("u1"),
            &serde_json::json!({}),
        )
        .unwrap();
        record_event(
            &conn,
            EventType::RuleCreated,
            Some("u1"),
            &serde_json::json!({}),
        )
        .unwrap();
        record_event(
            &conn,
            EventType::ObservationCreated,
            Some("u1"),
            &serde_json::json!({}),
        )
        .unwrap();

        let obs_events = get_recent_events(&conn, "observation_created", 10).unwrap();
        assert_eq!(obs_events.len(), 2);

        let rule_events = get_recent_events(&conn, "rule_created", 10).unwrap();
        assert_eq!(rule_events.len(), 1);
    }
}
