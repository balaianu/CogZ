//! Helper functions extracted from hybrid.rs for file-size compliance.

use crate::storage::crud::{Entity, EntityType};

/// Split FTS results into code and knowledge entity ID lists.
/// Code entities: function, class, file, module.
/// Knowledge entities: observation, rule, knowledge.
pub(super) fn split_fts_by_type(fts_entities: &[Entity]) -> (Vec<String>, Vec<String>) {
    let mut code = Vec::new();
    let mut knowledge = Vec::new();
    for entity in fts_entities {
        let etype = match EntityType::parse(&entity.r#type) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if etype.is_code() {
            code.push(entity.id.clone());
        } else {
            knowledge.push(entity.id.clone());
        }
    }
    (code, knowledge)
}

/// Resolve the status filter: None and "active" → Some("active"),
/// "all" → None (no filter), anything else → Some(value).
pub(super) fn resolve_status_filter(status: Option<&str>) -> Option<&str> {
    match status {
        None => Some("active"),
        Some("all") => None,
        Some(s) => Some(s),
    }
}
