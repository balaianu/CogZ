//! Unit tests for the frontmatter parser and serializer.

use cogz::files::frontmatter::{FmValue, Frontmatter, parse, serialize, split_frontmatter};

#[test]
fn split_simple_frontmatter() {
    let input = "---\nid: abc\ntitle: Test\n---\n\nBody text here.";
    let (fm, body) = split_frontmatter(input).unwrap();
    assert_eq!(fm, "id: abc\ntitle: Test");
    assert_eq!(body, "Body text here.");
}

#[test]
fn split_missing_delimiters() {
    assert!(split_frontmatter("no frontmatter here").is_err());
}

#[test]
fn parse_basic_fields() {
    let fm = parse("id: abc-123\ntitle: Test Title\nstatus: active").unwrap();
    assert_eq!(fm.get("id").unwrap().as_str(), Some("abc-123"));
    assert_eq!(fm.get("title").unwrap().as_str(), Some("Test Title"));
    assert_eq!(fm.get("status").unwrap().as_str(), Some("active"));
}

#[test]
fn parse_quoted_string() {
    let fm = parse("title: \"Hello: World\"").unwrap();
    assert_eq!(fm.get("title").unwrap().as_str(), Some("Hello: World"));
}

#[test]
fn parse_single_quoted_string() {
    let fm = parse("title: 'Hello World'").unwrap();
    assert_eq!(fm.get("title").unwrap().as_str(), Some("Hello World"));
}

#[test]
fn parse_float() {
    let fm = parse("confidence: 0.7").unwrap();
    assert_eq!(fm.get("confidence").unwrap().as_float(), Some(0.7));
}

#[test]
fn parse_int() {
    let fm = parse("validation_count: 3").unwrap();
    assert_eq!(fm.get("validation_count").unwrap().as_int(), Some(3));
}

#[test]
fn parse_bool() {
    let fm = parse("contradiction_check: true").unwrap();
    assert_eq!(fm.get("contradiction_check").unwrap().as_bool(), Some(true));
}

#[test]
fn parse_inline_array() {
    let fm = parse("references: [\"a1b2\", \"c3d4\"]").unwrap();
    let arr = fm.get("references").unwrap().as_array().unwrap();
    assert_eq!(arr, &["a1b2".to_string(), "c3d4".to_string()]);
}

#[test]
fn parse_empty_array() {
    let fm = parse("references: []").unwrap();
    assert!(fm.get("references").unwrap().as_array().unwrap().is_empty());
}

#[test]
fn parse_unquoted_array_items() {
    let fm = parse("tags: [search, fts5, vector]").unwrap();
    let arr = fm.get("tags").unwrap().as_array().unwrap();
    assert_eq!(arr, &["search", "fts5", "vector"]);
}

#[test]
fn parse_skips_comments_and_blanks() {
    let fm = parse("# comment\nid: abc\n\ntitle: Test").unwrap();
    assert_eq!(fm.entries.len(), 2);
    assert_eq!(fm.get("id").unwrap().as_str(), Some("abc"));
}

#[test]
fn parse_missing_colon_errors() {
    let result = parse("no colon here");
    assert!(result.is_err());
}

#[test]
fn serialize_roundtrip() {
    let mut fm = Frontmatter::new();
    fm.insert("id", FmValue::String("abc-123".to_string()));
    fm.insert("title", FmValue::String("Test Title".to_string()));
    fm.insert("confidence", FmValue::Float(0.7));
    fm.insert(
        "references",
        FmValue::Array(vec!["a1".to_string(), "b2".to_string()]),
    );

    let serialized = serialize(&fm);
    let parsed = parse(&serialized).unwrap();

    assert_eq!(fm, parsed);
}

#[test]
fn serialize_quotes_special_chars() {
    let mut fm = Frontmatter::new();
    fm.insert("title", FmValue::String("Bug: FTS5 issue".to_string()));
    let out = serialize(&fm);
    assert!(out.contains("title: \"Bug: FTS5 issue\""));
}

#[test]
fn serialize_empty_array() {
    let mut fm = Frontmatter::new();
    fm.insert("references", FmValue::Array(vec![]));
    let out = serialize(&fm);
    assert!(out.contains("references: []"));
}

#[test]
fn serialize_preserves_order() {
    let mut fm = Frontmatter::new();
    fm.insert("z", FmValue::String("alpha".to_string()));
    fm.insert("a", FmValue::String("beta".to_string()));
    fm.insert("m", FmValue::String("gamma".to_string()));
    let out = serialize(&fm);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "z: alpha");
    assert_eq!(lines[1], "a: beta");
    assert_eq!(lines[2], "m: gamma");
}

#[test]
fn full_file_roundtrip() {
    let input = "---\nid: 550e8400-e29b-41d4-a716-446655440000\ntitle: \"FTS5 ranking bug\"\ntype: observation\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: [\"a1b2c3d4-e5f6-4789-abcd-000000000017\"]\nsource: agent\nconfidence: 0.7\n---\n\nThe RRF fusion produces incorrect rankings.\n";
    let (fm_text, body) = split_frontmatter(input).unwrap();
    let fm = parse(&fm_text).unwrap();

    assert_eq!(
        fm.get("id").unwrap().as_str(),
        Some("550e8400-e29b-41d4-a716-446655440000")
    );
    assert_eq!(fm.get("title").unwrap().as_str(), Some("FTS5 ranking bug"));
    assert_eq!(fm.get("confidence").unwrap().as_float(), Some(0.7));
    assert_eq!(
        fm.get("references").unwrap().as_array().unwrap(),
        &["a1b2c3d4-e5f6-4789-abcd-000000000017".to_string()]
    );
    assert_eq!(body, "The RRF fusion produces incorrect rankings.\n");

    // Re-serialize and verify
    let reserialized = serialize(&fm);
    let reparsed = parse(&reserialized).unwrap();
    assert_eq!(fm, reparsed);
}
