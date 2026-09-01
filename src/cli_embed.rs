//! CLI embedding helpers — orchestrate model selection and the
//! lock-drop-reacquire pattern for embedding synced entities and
//! search queries.

use cogz::config::Config;
use cogz::storage::Storage;

/// Embed synced entities, selecting the model per entity type.
/// Code entities use CodeRankEmbed; knowledge entities use bge-base.
/// Returns the count of entities successfully embedded. If a model
/// is unavailable, those entities are skipped (graceful degradation).
///
/// By default, only knowledge entities are embedded inline (fast —
/// usually <50 entities). Code entities are deferred to a background
/// process via `spawn_code_embed_background` to avoid blocking the
/// index command for minutes on large codebases.
///
/// Entity data is fetched under the DB lock, then the lock is dropped
/// during model inference, then re-acquired to store vectors.
pub fn embed_synced(storage: &Storage, config: &Config, entity_ids: &[String]) -> usize {
    use cogz::embed::{EmbeddingCache, ModelType, OnnxEmbeddingModel};
    use cogz::storage::crud::EntityType;

    let models_dir = models_dir();
    let cache = EmbeddingCache::new();

    // Phase 1: fetch entity data under the lock, then drop it
    let entities: Vec<_> = {
        let conn = storage.conn();
        cogz::storage::crud::get_entities_batch(&conn, entity_ids).unwrap_or_default()
    };

    let (code_entities, knowledge_entities): (Vec<_>, Vec<_>) =
        entities.iter().cloned().partition(|e| {
            EntityType::parse(&e.r#type)
                .map(|t| t.is_code())
                .unwrap_or(false)
        });

    // Phase 2: embed knowledge entities inline (fast — few entities)
    let mut all_embeddings = Vec::new();

    if !knowledge_entities.is_empty() {
        let model = OnnxEmbeddingModel::with_model_id(
            ModelType::Knowledge,
            &models_dir,
            config.embedding.dimension,
            &config.embedding.knowledge_model,
        );
        if model.model_files_exist() {
            all_embeddings.extend(cogz::files::embed_sync::embed_entities(
                &model,
                &cache,
                &knowledge_entities,
            ));
        }
    }

    // Phase 3: store knowledge vectors under the lock
    if !all_embeddings.is_empty() {
        let mut conn = storage.conn();
        cogz::files::embed_sync::store_embeddings(&mut conn, &all_embeddings);
    }

    // Phase 4: embed code entities inline if there are few enough,
    // otherwise defer to background. The threshold is 50 — below that,
    // inline embedding takes seconds. Above it, background is better.
    const INLINE_CODE_THRESHOLD: usize = 50;

    let code_count = code_entities.len();
    if code_count <= INLINE_CODE_THRESHOLD {
        return embed_code_inline(&code_entities, config, &cache, storage) + all_embeddings.len();
    }

    // Defer code embedding to a background process.
    spawn_code_embed_background(storage, config, &code_entities);
    all_embeddings.len()
}

/// Embed code entities inline (synchronous). Used when the count is
/// small enough that blocking is acceptable.
fn embed_code_inline(
    code_entities: &[cogz::storage::crud::Entity],
    config: &Config,
    cache: &cogz::embed::EmbeddingCache,
    storage: &Storage,
) -> usize {
    use cogz::embed::{ModelType, OnnxEmbeddingModel};

    let models_dir = models_dir();
    let model = OnnxEmbeddingModel::with_model_id(
        ModelType::Code,
        &models_dir,
        config.embedding.dimension,
        &config.embedding.code_model,
    );

    if !model.model_files_exist() {
        return 0;
    }

    let embeddings = cogz::files::embed_sync::embed_entities(&model, cache, code_entities);
    if embeddings.is_empty() {
        return 0;
    }
    let mut conn = storage.conn();
    cogz::files::embed_sync::store_embeddings(&mut conn, &embeddings)
}

/// Spawn a background process to embed code entities. The index
/// command returns immediately; embeddings fill in over time.
///
/// The background process is a `cogz embed-bg` invocation that
/// re-opens the DB, embeds the entities, and stores vectors. This
/// avoids holding the DB lock in the foreground process.
fn spawn_code_embed_background(
    storage: &Storage,
    config: &Config,
    code_entities: &[cogz::storage::crud::Entity],
) {
    // Write the entity IDs to a temp file for the background process to read.
    let id_file = std::env::temp_dir().join(format!(
        "cogz-embed-bg-{}-{}.txt",
        std::process::id(),
        chrono::Utc::now().timestamp()
    ));

    let ids: Vec<String> = code_entities.iter().map(|e| e.id.clone()).collect();
    if std::fs::write(&id_file, ids.join("\n")).is_err() {
        tracing::warn!("failed to write background embed ID file");
        return;
    }

    // Get the DB path for the background process to re-open.
    let db_path: Option<String> = {
        let conn = storage.conn();
        conn.query_row(
            "SELECT file FROM pragma_database_list() WHERE name='main'",
            [],
            |row| row.get(0),
        )
        .ok()
    };

    let Some(db_path) = db_path else {
        tracing::warn!("failed to get DB path for background embedding");
        let _ = std::fs::remove_file(&id_file);
        return;
    };

    // Find the cogz binary.
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("cogz"));

    // Spawn the background process.
    let result = std::process::Command::new(&exe)
        .arg("embed-bg")
        .arg("--db")
        .arg(&db_path)
        .arg("--ids-file")
        .arg(&id_file)
        .arg("--code-model")
        .arg(&config.embedding.code_model)
        .arg("--dimension")
        .arg(config.embedding.dimension.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match result {
        Ok(child) => {
            println!(
                "  Code embedding deferred to background ({} entities, pid={})",
                code_entities.len(),
                child.id()
            );
            // Detach the child so it survives the parent's exit.
            drop(child);
        }
        Err(e) => {
            tracing::warn!("failed to spawn background embedding: {}", e);
            println!(
                "  Code embedding skipped ({} entities — run `cogz embed-bg` manually)",
                code_entities.len()
            );
        }
    }

    // Clean up the ID file eventually (the background process reads it
    // and deletes it when done, but if spawn fails we clean up here).
    // We don't delete it here because the child may still be starting.
}

/// Get the models directory: `~/.local/share/cogz/models/`
pub fn models_dir() -> std::path::PathBuf {
    cogz::embed::models_dir()
}

/// Embed a search query using the configured knowledge model.
/// Returns None if the model is unavailable (graceful degradation
/// to FTS-only search).
pub fn embed_query(config: &Config, query: &str) -> Option<Vec<f32>> {
    use cogz::embed::{EmbeddingModel, ModelType, OnnxEmbeddingModel};

    let models_dir = models_dir();
    let model = OnnxEmbeddingModel::with_model_id(
        ModelType::Knowledge,
        &models_dir,
        config.embedding.dimension,
        &config.embedding.knowledge_model,
    );

    if !model.model_files_exist() {
        return None;
    }

    match model.embed_query(&[query]) {
        Ok(embeddings) => embeddings.into_iter().next(),
        Err(e) => {
            tracing::warn!("query embedding failed: {}", e);
            None
        }
    }
}
