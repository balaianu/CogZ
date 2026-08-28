//! Unit tests for entity file I/O — slug generation, file reading/writing,
//! and file path computation.

use std::path::Path;

use cogz::files::entities::{
    EntityFile, FileEntityType, read_entity_file, slug_with_hash, slugify, write_entity_file,
};
use cogz::files::frontmatter::FmValue;

#[test]
fn slugify_basic() {
    assert_eq!(slugify("Search Architecture"), "search-architecture");
    assert_eq!(
        slugify("Use SQLite over Postgres"),
        "use-sqlite-over-postgres"
    );
}

#[test]
fn slugify_strips_special_chars() {
    assert_eq!(slugify("FTS5: Ranking Bug!"), "fts5-ranking-bug");
}

#[test]
fn slugify_truncates_long_titles() {
    let long_title =
        "This is a very long title that exceeds the sixty character limit and should be truncated";
    let slug = slugify(long_title);
    assert!(slug.len() <= 60);
}

#[test]
fn slug_with_hash_adds_suffix() {
    let s = slug_with_hash("search-architecture", "extra seed");
    assert!(s.starts_with("search-architecture-"));
    assert!(s.len() > "search-architecture".len());
}

#[test]
fn entity_file_roundtrip() {
    let mut entity = EntityFile::new("Test Knowledge", FileEntityType::Knowledge, "Body text");
    entity
        .frontmatter
        .insert("category", FmValue::String("architecture".to_string()));
    entity
        .frontmatter
        .insert("tags", FmValue::Array(vec!["search".to_string()]));

    let content = entity.to_file_content();
    let parsed = EntityFile::from_content(&content).unwrap();

    assert_eq!(parsed.id, entity.id);
    assert_eq!(parsed.title, "Test Knowledge");
    assert_eq!(parsed.entity_type, FileEntityType::Knowledge);
    assert_eq!(parsed.status, "active");
    assert_eq!(parsed.body, "Body text");
    assert_eq!(
        parsed.frontmatter.get("category").unwrap().as_str(),
        Some("architecture")
    );
}

#[test]
fn entity_file_with_references() {
    let mut entity = EntityFile::new("Test Obs", FileEntityType::Observation, "Content");
    entity.references = vec!["abc-123".to_string(), "def-456".to_string()];

    let content = entity.to_file_content();
    let parsed = EntityFile::from_content(&content).unwrap();

    assert_eq!(parsed.references, entity.references);
}

#[test]
fn entity_file_missing_required_field_errors() {
    let bad_content = "---\nid: abc\ntitle: Test\n---\n\nBody";
    assert!(EntityFile::from_content(bad_content).is_err());
}

#[test]
fn entity_file_invalid_type_errors() {
    let bad_content = "---\nid: abc\ntitle: Test\ntype: invalid_type\nstatus: active\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\n---\n\nBody";
    assert!(EntityFile::from_content(bad_content).is_err());
}

#[test]
fn knowledge_file_path() {
    let mut entity = EntityFile::new("Search Design", FileEntityType::Knowledge, "body");
    entity
        .frontmatter
        .insert("category", FmValue::String("architecture".to_string()));
    entity.created_at = "2026-08-27T14:30:00Z".to_string();

    let cogz = Path::new(".cogz");
    let path = entity.file_path(cogz);
    assert_eq!(
        path,
        Path::new(".cogz/knowledge/architecture/search-design.md")
    );
}

#[test]
fn rule_file_path() {
    let entity = EntityFile::new("Use Parameterized Queries", FileEntityType::Rule, "body");
    let cogz = Path::new(".cogz");
    let path = entity.file_path(cogz);
    assert_eq!(path, Path::new(".cogz/rules/use-parameterized-queries.md"));
}

#[test]
fn observation_file_path() {
    let entity = EntityFile::new("Some Bug", FileEntityType::Observation, "body");
    let cogz = Path::new(".cogz");
    let path = entity.file_path(cogz);
    assert!(path.starts_with(".cogz/observations/"));
    assert!(path.to_string_lossy().ends_with(".md"));
}

#[test]
fn write_and_read_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.md");

    let entity = EntityFile::new("Test", FileEntityType::Rule, "Rule body");
    write_entity_file(&path, &entity).unwrap();

    let read = read_entity_file(&path).unwrap();
    assert_eq!(read.id, entity.id);
    assert_eq!(read.title, "Test");
    assert_eq!(read.body, "Rule body");
}
