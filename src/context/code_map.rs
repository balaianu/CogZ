//! Code map generation for cold-start context packs.
//!
//! Produces a bounded summary of the project's structural modules and
//! key files, filtering out inline test modules and ranking files by
//! incoming edge count rather than symbol count.

use rusqlite::Connection;

use crate::storage::query::get_entities_by_type;

use super::ContextSection;

/// Build a bounded code map summary: top modules and key files.
///
/// Filters out `#[cfg(test)] mod tests` blocks from the module list
/// (they're not structural modules). Ranks key files by incoming edge
/// count (how many other files import/reference them) rather than
/// symbol count, since test files have many symbols but low structural
/// importance.
pub fn code_map_sections(conn: &Connection) -> Vec<ContextSection> {
    let mut sections = Vec::new();

    // Top modules — filter out inline test modules (title == "tests")
    let modules = get_entities_by_type(conn, "module", Some("active"), 100).unwrap_or_default();
    let real_modules: Vec<_> = modules
        .iter()
        .filter(|m| {
            let title = m.title.as_deref().unwrap_or("");
            title != "tests" && title != "test"
        })
        .take(20)
        .collect();
    if !real_modules.is_empty() {
        let module_list: Vec<String> = real_modules
            .iter()
            .map(|m| {
                format!(
                    "- {} ({})",
                    m.title.as_deref().unwrap_or("(unnamed)"),
                    m.file_path.as_deref().unwrap_or("?")
                )
            })
            .collect();
        sections.push(ContextSection {
            source: "code_map".to_string(),
            entity_id: "modules".to_string(),
            title: "Code Map — Modules".to_string(),
            content: module_list.join("\n"),
            relevance: 0.0,
            graph_path: vec![],
            graph_path_description: String::new(),
        });
    }

    // Key files — ranked by incoming edge count (structural importance:
    // how many other entities reference or import this file).
    let key_files: Vec<(String, String, i64)> = {
        let mut stmt = match conn.prepare(
            "SELECT e.title, e.file_path, COUNT(*) as incoming_count
             FROM edges ed
             JOIN entities e ON ed.target_id = e.id
             WHERE e.type = 'file'
             GROUP BY e.id
             ORDER BY incoming_count DESC
             LIMIT 15",
        ) {
            Ok(s) => s,
            Err(_) => return sections,
        };
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .ok();
        match rows {
            Some(rows) => rows.filter_map(|r| r.ok()).collect(),
            None => return sections,
        }
    };

    if !key_files.is_empty() {
        let file_list: Vec<String> = key_files
            .iter()
            .map(|(title, path, count)| format!("- {title} ({path}) — {count} refs"))
            .collect();
        sections.push(ContextSection {
            source: "code_map".to_string(),
            entity_id: "key_files".to_string(),
            title: "Code Map — Key Files".to_string(),
            content: file_list.join("\n"),
            relevance: 0.0,
            graph_path: vec![],
            graph_path_description: String::new(),
        });
    }

    sections
}
