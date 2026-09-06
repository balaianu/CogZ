use super::*;

#[test]
fn sanitize_category_strips_path_traversal() {
    assert_eq!(sanitize_category("../../etc/passwd"), "etc-passwd");
    assert_eq!(sanitize_category("/absolute/path"), "absolute-path");
    assert_eq!(sanitize_category(".."), "uncategorized");
    assert_eq!(sanitize_category("normal"), "normal");
    assert_eq!(sanitize_category("with spaces"), "with-spaces");
    assert_eq!(sanitize_category(""), "uncategorized");
}

#[test]
fn knowledge_file_path_stays_within_cogz_dir() {
    let mut ef = EntityFile::new("Test Title", FileEntityType::Knowledge, "body");
    ef.frontmatter
        .insert("category", FmValue::String("../../../outside".to_string()));
    let cogz_dir = Path::new("/tmp/cogz");
    let path = ef.file_path(cogz_dir);
    // Path must stay within cogz_dir/knowledge/
    assert!(path.starts_with(cogz_dir.join("knowledge")));
    // No path traversal components
    assert!(
        !path
            .components()
            .any(|c| { matches!(c, std::path::Component::ParentDir) })
    );
}
