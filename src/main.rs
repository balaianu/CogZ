//! CogZ CLI entry point.

mod cli;
mod commands;

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

    /// Run background consolidation: promote supported observations
    /// to rules, merge confirmed duplicates. Dedup and contradiction
    /// detection happen automatically on every insert; this runs the
    /// deferred phases.
    Consolidate {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// Report what would be consolidated without making changes.
        #[arg(long)]
        dry_run: bool,
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
        Commands::Status { repo } => commands::run_status(&repo),
        Commands::Index { repo } => commands::run_index(&repo),
        Commands::Reindex { repo } => commands::run_reindex(&repo),
        Commands::Reset { repo, purge } => commands::run_reset(&repo, purge),
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
        Commands::Consolidate { repo, dry_run } => commands::run_consolidate(&repo, dry_run),
    }
}
