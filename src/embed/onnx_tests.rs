//! Tests for ONNX embedding model configuration.

use super::*;

#[test]
fn coderankembed_query_prefix_detected() {
    let model = OnnxEmbeddingModel::with_model_id(
        ModelType::Code,
        std::path::Path::new("/tmp"),
        768,
        "nomic-ai/CodeRankEmbed-int8",
    );
    assert_eq!(
        model.query_prefix(),
        "Represent this query for searching relevant code: "
    );
}

#[test]
fn bge_base_no_query_prefix() {
    let model = OnnxEmbeddingModel::with_model_id(
        ModelType::Knowledge,
        std::path::Path::new("/tmp"),
        768,
        "BAAI/bge-base-en-v1.5",
    );
    assert_eq!(model.query_prefix(), "");
}

#[test]
fn default_code_model_has_prefix() {
    let model = OnnxEmbeddingModel::new(ModelType::Code, std::path::Path::new("/tmp"), 768);
    assert!(!model.query_prefix().is_empty());
}

#[test]
fn default_knowledge_model_no_prefix() {
    let model = OnnxEmbeddingModel::new(ModelType::Knowledge, std::path::Path::new("/tmp"), 768);
    assert!(model.query_prefix().is_empty());
}
