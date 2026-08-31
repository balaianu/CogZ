//! MCP response builders — serialize tool outputs to JSON.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::json;

use crate::search::SearchResults;
use crate::storage::crud;

/// Build a successful tool result from a JSON value.
pub fn tool_success(value: serde_json::Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}

/// Build a query response JSON from a list of entities.
///
/// Includes `references` (from the `references` graph edges) and
/// type-specific properties (`category`, `tags` for knowledge;
/// `confidence` for rules) per the MCP contract.
pub fn query_response(
    entities: Vec<crud::Entity>,
    key: &str,
    references_map: &std::collections::HashMap<String, Vec<String>>,
) -> serde_json::Value {
    let items: Vec<_> = entities
        .iter()
        .map(|e| {
            let mut item = json!({
                "id": e.id,
                "title": e.title,
                "content": e.content,
                "status": e.status,
                "references": references_map.get(&e.id).cloned().unwrap_or_default(),
                "created_at": e.created_at,
                "updated_at": e.updated_at,
                "file_path": e.file_path,
            });
            if e.r#type == "knowledge" {
                if let Some(cat) = e.properties.get("category").and_then(|v| v.as_str()) {
                    item["category"] = json!(cat);
                }
                if let Some(tags) = e.properties.get("tags").and_then(|v| v.as_array()) {
                    item["tags"] = json!(tags);
                }
            } else if e.r#type == "rule" {
                if let Some(conf) = e.properties.get("confidence").and_then(|v| v.as_f64()) {
                    item["confidence"] = json!(conf);
                }
            } else if e.r#type == "observation" {
                if let Some(src) = e.properties.get("source").and_then(|v| v.as_str()) {
                    item["source"] = json!(src);
                } else {
                    item["source"] = json!("agent");
                }
            }
            item
        })
        .collect();
    json!({ key: items, "count": items.len() })
}

/// Build a search response from SearchResults.
pub fn search_response(results: SearchResults) -> serde_json::Value {
    let items: Vec<_> = results
        .results
        .iter()
        .map(|r| {
            json!({
                "id": r.entity.id,
                "type": r.entity.r#type,
                "title": r.entity.title,
                "content": r.entity.content,
                "relevance": r.relevance,
                "graph_path": r.graph_path,
                "graph_path_description": r.graph_path_description,
            })
        })
        .collect();
    json!({
        "results": items,
        "count": items.len(),
        "search_mode": results.search_mode.as_str(),
    })
}

/// Build a context pack response.
pub fn context_response(pack: crate::context::ContextPack) -> serde_json::Value {
    context_response_ref(&pack)
}

/// Same as `context_response` but takes a reference, for embedding
/// a context pack inside a larger response (e.g. `capture_event`).
pub fn context_response_ref(pack: &crate::context::ContextPack) -> serde_json::Value {
    let sections: Vec<_> = pack
        .sections
        .iter()
        .map(|s| {
            json!({
                "source": s.source,
                "entity_id": s.entity_id,
                "title": s.title,
                "content": s.content,
                "relevance": s.relevance,
                "graph_path": s.graph_path,
            })
        })
        .collect();
    json!({
        "query": pack.query,
        "mode": pack.mode.as_str(),
        "sections": sections,
        "metadata": {
            "size_tokens": pack.metadata.size_tokens,
            "selected_sources": pack.metadata.selected_sources,
            "dropped_sources": pack.metadata.dropped_sources,
            "search_mode": pack.metadata.search_mode,
        },
    })
}
