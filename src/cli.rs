//! CLI helpers — embedding orchestration and path resolution.
//!
//! These functions are only compiled into the `cogz` binary, not the
//! library crate. They bridge CLI commands to library functionality.

use cogz::config::Config;
use cogz::storage::Storage;

/// Embed synced entities, selecting the model per entity type.
/// Code entities use CodeRankEmbed; knowledge entities use bge-base.
/// Returns the count of entities successfully embedded. If a model
/// is unavailable, those entities are skipped (graceful degradation).
///
/// Entity data is fetched under the DB lock, then the lock is dropped
/// during model inference, then re-acquired to store vectors. This
/// prevents blocking other DB callers during potentially slow ONNX
/// inference.
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

    // Phase 2: embed without holding the DB lock
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

    if !code_entities.is_empty() {
        let model = OnnxEmbeddingModel::with_model_id(
            ModelType::Code,
            &models_dir,
            config.embedding.dimension,
            &config.embedding.code_model,
        );
        if model.model_files_exist() {
            all_embeddings.extend(cogz::files::embed_sync::embed_entities(
                &model,
                &cache,
                &code_entities,
            ));
        }
    }

    // Phase 3: store vectors under the lock
    if all_embeddings.is_empty() {
        return 0;
    }
    let mut conn = storage.conn();
    cogz::files::embed_sync::store_embeddings(&mut conn, &all_embeddings)
}

/// Get the models directory: `~/.local/share/cogz/models/`
pub fn models_dir() -> std::path::PathBuf {
    cogz::embed::models_dir()
}

/// Embed a search query using the knowledge model (bge-base).
/// Returns None if the model is unavailable (graceful degradation
/// to FTS-only search).
pub fn embed_query(config: &Config, query: &str) -> Option<Vec<f32>> {
    use cogz::embed::{EmbeddingModel, ModelType, OnnxEmbeddingModel};

    let models_dir = models_dir();
    let model = OnnxEmbeddingModel::new(
        ModelType::Knowledge,
        &models_dir,
        config.embedding.dimension,
    );

    if !model.model_files_exist() {
        return None;
    }

    match model.embed(&[query]) {
        Ok(embeddings) => embeddings.into_iter().next(),
        Err(e) => {
            tracing::warn!("query embedding failed: {}", e);
            None
        }
    }
}

/// Run the `cogz search` command.
#[allow(clippy::too_many_arguments)]
pub fn run_search(
    query: &str,
    repo: &std::path::Path,
    entity_type: Option<String>,
    status: Option<String>,
    limit: Option<u32>,
    no_expand: bool,
) -> anyhow::Result<()> {
    let cogz_dir = repo.join(".cogz");
    let config_path = cogz_dir.join("config.toml");

    if !config_path.exists() {
        anyhow::bail!(
            "No .cogz/ directory found in {}. Run `cogz init` first.",
            repo.display()
        );
    }

    let config = cogz::config::load(&config_path)?;
    let db_path = repo.join(&config.storage.db_path);

    if !db_path.exists() {
        anyhow::bail!(
            "Database not found at {}. Run `cogz index` first.",
            db_path.display()
        );
    }

    let storage = Storage::open(&db_path)?;

    let query_embedding = embed_query(&config, query);

    let params = cogz::search::SearchParams {
        entity_type,
        status,
        limit: limit.unwrap_or(config.search.max_results),
        expand: !no_expand,
        max_hops: if no_expand {
            0
        } else {
            config.context.task_max_hops
        },
    };

    let results = {
        let conn = storage.conn();
        cogz::search::search(
            &conn,
            query,
            query_embedding.as_deref(),
            &params,
            &config.search,
        )?
    };

    println!("Search mode: {}", results.search_mode.as_str());
    println!("Results: {}\n", results.results.len());

    for (i, result) in results.results.iter().enumerate() {
        let title = result.entity.title.as_deref().unwrap_or("(untitled)");
        let relevance = if result.relevance > 0.0 {
            format!("{:.4}", result.relevance)
        } else {
            "expanded".to_string()
        };

        println!(
            "  {}. [{}] {} (relevance: {})",
            i + 1,
            result.entity.r#type,
            title,
            relevance
        );

        if !result.graph_path_description.is_empty() {
            println!("     path: {}", result.graph_path_description);
        }

        let preview: String = result.entity.content.chars().take(120).collect();
        if !preview.is_empty() {
            println!("     {}", preview);
        }
        println!();
    }

    Ok(())
}

/// Run the `cogz context` command.
pub fn run_context(
    mode_str: &str,
    query: Option<&str>,
    repo: &std::path::Path,
    include_stale: bool,
    max_tokens: Option<usize>,
) -> anyhow::Result<()> {
    use cogz::context::{AssembleError, ContextMode};

    let mode = ContextMode::parse(mode_str).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid mode '{}': expected cold_start, task, or escalation",
            mode_str
        )
    })?;

    if mode.requires_query() && query.is_none() {
        anyhow::bail!("query is required for {} mode", mode);
    }

    let cogz_dir = repo.join(".cogz");
    let config_path = cogz_dir.join("config.toml");

    if !config_path.exists() {
        anyhow::bail!(
            "No .cogz/ directory found in {}. Run `cogz init` first.",
            repo.display()
        );
    }

    let config = cogz::config::load(&config_path)?;
    let db_path = repo.join(&config.storage.db_path);

    if !db_path.exists() {
        anyhow::bail!(
            "Database not found at {}. Run `cogz index` first.",
            db_path.display()
        );
    }

    let storage = Storage::open(&db_path)?;

    let query_embedding = query.and_then(|q| embed_query(&config, q));

    let params = cogz::context::AssembleParams {
        mode,
        query,
        query_embedding: query_embedding.as_deref(),
        max_tokens,
        include_stale,
    };

    let pack = {
        let conn = storage.conn();
        cogz::context::assemble_context(&conn, &params, &config).map_err(|e| match e {
            AssembleError::QueryRequired(m) => {
                anyhow::anyhow!("query is required for {} mode", m)
            }
            other => anyhow::anyhow!("{other}"),
        })?
    };

    println!("Context pack (mode: {})", pack.mode);
    println!(
        "Query: {}",
        if pack.query.is_empty() {
            "(none)"
        } else {
            &pack.query
        }
    );
    println!("Search mode: {}", pack.metadata.search_mode);
    println!("Sections: {}", pack.sections.len());
    println!("Token estimate: {}", pack.metadata.size_tokens);

    if !pack.metadata.dropped_sources.is_empty() {
        println!(
            "\nDropped sources ({}):",
            pack.metadata.dropped_sources.len()
        );
        for src in &pack.metadata.dropped_sources {
            println!("  - {}", src);
        }
    }

    println!("\n---\n");
    for (i, section) in pack.sections.iter().enumerate() {
        let relevance = if section.relevance > 0.0 {
            format!("{:.4}", section.relevance)
        } else {
            "recent".to_string()
        };

        println!(
            "## {}. [{}] {} (relevance: {})\n",
            i + 1,
            section.source,
            section.title,
            relevance
        );

        if section.graph_path.len() > 1 {
            println!(
                "  graph path: {} -> {}\n",
                section.graph_path.first().unwrap_or(&section.entity_id),
                section.entity_id
            );
        }

        println!("{}\n", section.content);
    }

    Ok(())
}

/// Run the MCP server over stdio. Requires an initialized .cogz/
/// directory and an indexed database.
pub fn run_mcp_stdio(repo: &std::path::Path) -> anyhow::Result<()> {
    let cogz_dir = repo.join(".cogz");
    let config_path = cogz_dir.join("config.toml");

    if !config_path.exists() {
        anyhow::bail!(
            "No .cogz/ directory found in {}. Run `cogz init` first.",
            repo.display()
        );
    }

    let config = cogz::config::load(&config_path)?;
    let db_path = repo.join(&config.storage.db_path);

    if !db_path.exists() {
        anyhow::bail!(
            "Database not found at {}. Run `cogz index` first.",
            db_path.display()
        );
    }

    let storage = std::sync::Arc::new(Storage::open(&db_path)?);
    let server = cogz::mcp::CogzServer::new(storage, config, cogz_dir);

    // tracing must go to stderr, not stdout — stdout is the MCP transport
    tracing::info!("Starting CogZ MCP server over stdio");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(cogz::mcp::server::run_stdio(server))
}
