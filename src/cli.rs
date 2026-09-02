//! CLI helpers — path resolution and command dispatch.
//!
//! These functions are only compiled into the `cogz` binary, not the
//! library crate. They bridge CLI commands to library functionality.

use cogz::storage::Storage;

pub use crate::cli_embed::{embed_query, embed_query_with_model, embed_synced, models_dir};

/// Run the `cogz search` command.
#[allow(clippy::too_many_arguments)]
pub fn run_search(
    query: &str,
    repo: &std::path::Path,
    entity_type: Option<String>,
    status: Option<String>,
    limit: Option<u32>,
    no_expand: bool,
    code: bool,
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

    let storage = Storage::open(&db_path, config.embedding.dimension)?;

    // Embed the query with both models for dual-channel search.
    // When --code is set, only the code model is used.
    // When --code is not set, both models are used (knowledge + code).
    let knowledge_emb = if code {
        None
    } else {
        embed_query_with_model(&config, query, false)
    };
    let code_emb = embed_query_with_model(&config, query, true);

    let embeddings = cogz::search::QueryEmbeddings {
        knowledge: knowledge_emb.as_deref(),
        code: code_emb.as_deref(),
    };

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
        cogz::search::search(&conn, query, embeddings, &params, &config.search)?
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

    let storage = Storage::open(&db_path, config.embedding.dimension)?;

    let knowledge_embedding = query.and_then(|q| embed_query(&config, q));
    let code_embedding = query.and_then(|q| embed_query_with_model(&config, q, true));

    let params = cogz::context::AssembleParams {
        mode,
        query,
        knowledge_embedding: knowledge_embedding.as_deref(),
        code_embedding: code_embedding.as_deref(),
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
            "\nDropped: {} sections over token budget",
            pack.metadata.dropped_sources.len()
        );
    }

    println!("\n---\n");
    for (i, section) in pack.sections.iter().enumerate() {
        let relevance = if section.relevance > 0.0 {
            format!("{:.4}", section.relevance)
        } else {
            "—".to_string()
        };

        println!(
            "## {}. [{}] {} (relevance: {})\n",
            i + 1,
            section.source,
            section.title,
            relevance
        );

        if section.graph_path.len() > 1 {
            if !section.graph_path_description.is_empty() {
                println!("  graph path: {}\n", section.graph_path_description);
            } else {
                println!(
                    "  graph path: {} -> {}\n",
                    section.graph_path.first().unwrap_or(&section.entity_id),
                    section.entity_id
                );
            }
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

    let storage = std::sync::Arc::new(Storage::open(&db_path, config.embedding.dimension)?);
    let server = cogz::mcp::CogzServer::new(storage, config, cogz_dir);

    // tracing must go to stderr, not stdout — stdout is the MCP transport
    tracing::info!("Starting CogZ MCP server over stdio");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(cogz::mcp::server::run_stdio(server))
}

/// Run `cogz capture-event` — capture a lifecycle event from hook
/// scripts. For session_start and prompt_submit, prints a context
/// pack to stdout for agent injection. For file_save, prints reindex
/// summary to stderr. Status info goes to stderr so stdout stays
/// clean for context pack injection.
pub fn run_capture_event(input: &cogz::hooks::CaptureInput) -> anyhow::Result<()> {
    let result = cogz::hooks::run_capture_event(input)?;

    // In hook-json mode, stdout is reserved for the JSON response.
    // Status info goes to stderr so it doesn't corrupt the JSON.
    if !input.hook_json {
        eprintln!(
            "Event {} recorded (id: {})",
            input.event_str, result.event_id
        );
    }
    if let Some(ref obs_id) = result.observation_id {
        eprintln!("Observation recorded: {}", obs_id);
    }
    if let Some(ref summary) = result.reindex_summary {
        if summary.reindexed {
            eprintln!(
                "Code reindex: {} created, {} updated, {} stale, {} knowledge flagged",
                summary.created,
                summary.updated,
                summary.marked_stale,
                summary.stale_knowledge_flagged
            );
        } else if summary.synced {
            eprintln!(
                "File sync: {} created, {} updated, {} stale, {} embedded",
                summary.created, summary.updated, summary.marked_stale, summary.embedded
            );
        }
    }
    if let Some(ref summary) = result.consolidation_summary {
        eprintln!(
            "Consolidation: {} promoted, {} merged",
            summary.promotions, summary.merges
        );
    }

    Ok(())
}
