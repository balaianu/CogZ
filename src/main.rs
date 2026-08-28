//! CogZ CLI entry point.

mod cli;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// CogZ — local-first, code-aware engineering cognition runtime.
#[derive(Parser)]
#[command(name = "cogz", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize .cogz/ in a repository.
    Init {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },

    /// Show system status — DB stats, entity counts, stale count.
    Status {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },

    /// Index the repository — scan .cogz/ files and sync to DB.
    Index {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },

    /// Re-index changed files only (incremental sync).
    Reindex {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },

    /// Drop the database (rebuildable from files via `cogz index`).
    Reset {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// Also remove observations and the generated .gitignore.
        /// Keeps knowledge, rules, and config.
        #[arg(long)]
        purge: bool,
    },

    /// Search entities with hybrid FTS + vector search.
    Search {
        /// Search query.
        query: String,

        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// Filter by entity type (observation, rule, knowledge, function, class, file, module).
        #[arg(long)]
        entity_type: Option<String>,

        /// Filter by status. Default: active. Use "all" for everything.
        #[arg(long)]
        status: Option<String>,

        /// Max results before graph expansion. Defaults to search.max_results from config.
        #[arg(long)]
        limit: Option<u32>,

        /// Disable graph expansion.
        #[arg(long)]
        no_expand: bool,
    },

    /// Assemble a context pack for agent consumption.
    Context {
        /// Context mode: cold_start, task, or escalation.
        /// Default: task. cold_start doesn't require a query.
        #[arg(long, default_value = "task")]
        mode: String,

        /// Search query (required for task and escalation modes).
        query: Option<String>,

        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// Include stale entities in the context pack.
        #[arg(long)]
        include_stale: bool,

        /// Override the token budget from config.
        #[arg(long)]
        max_tokens: Option<usize>,
    },

    /// Run the MCP server over stdio (for AI agent integration).
    McpStdio {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { repo } => {
            let msg = cogz::init::run(&repo)?;
            println!("{}", msg);
            Ok(())
        }
        Commands::Status { repo } => run_status(&repo),
        Commands::Index { repo } => run_index(&repo),
        Commands::Reindex { repo } => run_reindex(&repo),
        Commands::Reset { repo, purge } => run_reset(&repo, purge),
        Commands::Search {
            query,
            repo,
            entity_type,
            status,
            limit,
            no_expand,
        } => cli::run_search(&query, &repo, entity_type, status, limit, no_expand),
        Commands::Context {
            mode,
            query,
            repo,
            include_stale,
            max_tokens,
        } => cli::run_context(&mode, query.as_deref(), &repo, include_stale, max_tokens),
        Commands::McpStdio { repo } => cli::run_mcp_stdio(&repo),
    }
}

fn run_status(repo: &std::path::Path) -> anyhow::Result<()> {
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
        println!("CogZ status for project: {}\n", config.project.name);
        println!(
            "  DB not found at {}. Run `cogz index` to build it.",
            db_path.display()
        );
        return Ok(());
    }

    let storage = cogz::storage::Storage::open(&db_path)?;

    println!("CogZ status for project: {}\n", config.project.name);
    println!("  DB: {}", db_path.display());
    println!("  DB size: {} bytes", storage.db_size_bytes());

    let conn = storage.conn();
    let schema_version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    println!("  Schema version: {}", schema_version);

    let counts = cogz::storage::query::entity_counts_by_type(&conn)?;
    println!("\n  Entities by type:");
    if counts.is_empty() {
        println!("    (none)");
    } else {
        for (etype, count) in &counts {
            println!("    {:12} {}", etype, count);
        }
    }

    let total = cogz::storage::crud::count_all(&conn)?;
    println!("\n  Total entities: {}", total);

    let stale = cogz::storage::query::count_stale(&conn)?;
    println!("  Stale entities: {}", stale);

    let edges = cogz::storage::edges::count_edges(&conn)?;
    println!("  Edges: {}", edges);

    let events = cogz::storage::events::count_events(&conn)?;
    println!("  Events: {}", events);

    // Model availability
    let models_dir = cli::models_dir();
    let code_model = cogz::embed::OnnxEmbeddingModel::new(
        cogz::embed::ModelType::Code,
        &models_dir,
        config.embedding.dimension,
    );
    let knowledge_model = cogz::embed::OnnxEmbeddingModel::new(
        cogz::embed::ModelType::Knowledge,
        &models_dir,
        config.embedding.dimension,
    );
    println!("\n  Models:");
    println!(
        "    Code (CodeRankEmbed): {}",
        if code_model.model_files_exist() {
            "available"
        } else {
            "not found"
        }
    );
    println!(
        "    Knowledge (bge-base): {}",
        if knowledge_model.model_files_exist() {
            "available"
        } else {
            "not found"
        }
    );
    let embedding_count = cogz::storage::embeddings::count_embeddings(&conn)?;
    println!("  Embeddings stored: {}", embedding_count);

    Ok(())
}

fn run_index(repo: &std::path::Path) -> anyhow::Result<()> {
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
    let storage = cogz::storage::Storage::open(&db_path)?;

    println!("Indexing {}...", cogz_dir.display());
    let result = cogz::files::sync_all(&storage, &cogz_dir);

    println!(
        "  Created: {}\n  Updated: {}\n  Stale: {}\n  Skipped: {}",
        result.created, result.updated, result.marked_stale, result.skipped
    );

    if !result.errors.is_empty() {
        println!("\n  Errors ({}):", result.errors.len());
        for err in &result.errors {
            println!("    {}: {}", err.file_path.display(), err.error);
        }
    }

    // Embed synced entities (graceful degradation if model unavailable)
    if !result.synced_entity_ids.is_empty() {
        let embedded = cli::embed_synced(&storage, &config, &result.synced_entity_ids);
        if embedded > 0 {
            println!("  Embedded: {}", embedded);
        }
    }

    let conn = storage.conn();
    let total = cogz::storage::crud::count_all(&conn)?;
    println!("\n  Total entities: {}", total);

    Ok(())
}

fn run_reindex(repo: &std::path::Path) -> anyhow::Result<()> {
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

    let storage = cogz::storage::Storage::open(&db_path)?;

    println!("Reindexing (incremental) {}...", cogz_dir.display());
    let result = cogz::files::sync_incremental(&storage, &cogz_dir);

    println!(
        "  Created: {}\n  Updated: {}\n  Stale: {}\n  Skipped: {}",
        result.created, result.updated, result.marked_stale, result.skipped
    );

    if !result.errors.is_empty() {
        println!("\n  Errors ({}):", result.errors.len());
        for err in &result.errors {
            println!("    {}: {}", err.file_path.display(), err.error);
        }
    }

    // Embed synced entities (graceful degradation if model unavailable)
    if !result.synced_entity_ids.is_empty() {
        let embedded = cli::embed_synced(&storage, &config, &result.synced_entity_ids);
        if embedded > 0 {
            println!("  Embedded: {}", embedded);
        }
    }

    Ok(())
}

fn run_reset(repo: &std::path::Path, purge: bool) -> anyhow::Result<()> {
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

    // Drop the database
    for suffix in &["", "-wal", "-shm"] {
        let path = format!("{}{}", db_path.display(), suffix);
        let path = std::path::Path::new(&path);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    println!("Dropped database: {}", db_path.display());

    if purge {
        // Remove observations directory
        let obs_dir = cogz_dir.join("observations");
        if obs_dir.exists() {
            std::fs::remove_dir_all(&obs_dir)?;
            std::fs::create_dir_all(&obs_dir)?;
            println!("Purged observations: {}", obs_dir.display());
        }

        // Remove generated .gitignore
        let gitignore = cogz_dir.join(".gitignore");
        if gitignore.exists() {
            std::fs::remove_file(&gitignore)?;
            println!("Removed generated .gitignore");
        }

        println!("\n  Knowledge and rules preserved.");
    }

    println!("\nRun `cogz index` to rebuild the database from files.");

    Ok(())
}
