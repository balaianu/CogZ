//! CogZ CLI entry point.

mod cli;
mod cli_embed;
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
        /// Gitignore all of .cogz/ — nothing is committed to git.
        /// Use this for solo projects where knowledge, rules, and
        /// observations should stay local. Default is team-sharing
        /// mode (knowledge, rules, and observations are committed;
        /// DB is gitignored).
        #[arg(long)]
        local_only: bool,
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

        /// Skip model download. Operates in FTS-only mode.
        #[arg(long)]
        no_download: bool,
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

        /// Use the code model (CodeRankEmbed) for query embedding instead of
        /// the knowledge model. Applies the CodeRankEmbed query prefix.
        #[arg(long)]
        code: bool,
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
    /// The server starts empty; every tool call must specify `repo`.
    McpStdio,

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

    /// Capture a lifecycle event. Called by agent hook systems or
    /// manually. For session_start and prompt_submit, prints a context
    /// pack to stdout for agent injection. For file_save, triggers an
    /// incremental code reindex. For session_end, runs consolidation.
    /// For stop, records the event (no side effects).
    CaptureEvent {
        /// Event type: session_start, prompt_submit, pre_tool_use,
        /// post_tool_use, file_save, session_end, stop.
        event_type: String,

        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// Prompt text (for prompt_submit).
        #[arg(long)]
        prompt: Option<String>,

        /// Read prompt from a file (for prompt_submit).
        #[arg(long)]
        prompt_file: Option<String>,

        /// Tool name (for pre_tool_use, post_tool_use).
        #[arg(long)]
        tool_name: Option<String>,

        /// Tool result summary (for post_tool_use).
        #[arg(long)]
        tool_result: Option<String>,

        /// Saved file path, relative to repo root (for file_save).
        #[arg(long)]
        file_path: Option<String>,

        /// Wrap output as JSON for agent hook systems
        /// ({"hookSpecificOutput": {...}}). Silent skip if no .cogz/
        /// found. Use this when calling from agent hook configs.
        #[arg(long)]
        hook_json: bool,

        /// Skip model loading — use FTS-only search for context
        /// assembly. Much faster (~2s vs ~40s) but lower quality
        /// ranking. Recommended for hook calls where speed matters.
        #[arg(long)]
        fts_only: bool,
    },

    /// Model management — download, list, clean.
    Models {
        #[command(subcommand)]
        subcommand: ModelsSub,
    },

    /// Health check — DB integrity, model availability, policy violations.
    Doctor {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// Report prunable observations (dry-run).
        #[arg(long)]
        prune_observations: bool,

        /// Confirm pruning (deletes files, creates tombstones).
        #[arg(long)]
        confirm: bool,
    },

    /// Self-update — download and install the latest release from GitHub.
    Update {},

    /// Background code embedding — spawned by `cogz index` for large
    /// codebases. Not intended for direct use.
    EmbedBg {
        /// Database path.
        #[arg(long)]
        db: PathBuf,

        /// File containing entity IDs (one per line).
        #[arg(long)]
        ids_file: PathBuf,

        /// Code model ID.
        #[arg(long)]
        code_model: String,

        /// Embedding dimension.
        #[arg(long)]
        dimension: usize,

        /// Model idle TTL in seconds (0 = never unload).
        #[arg(long, default_value_t = 300)]
        idle_ttl: u64,

        /// Minimum free memory in MB to load a model (0 = no check).
        #[arg(long, default_value_t = 512)]
        min_free_mb: u64,
    },
}

#[derive(Subcommand)]
enum ModelsSub {
    /// Download configured models from HuggingFace.
    Download {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// Download only the code embedding model.
        #[arg(long)]
        code: bool,

        /// Download only the knowledge embedding model.
        #[arg(long)]
        knowledge: bool,

        /// Download only the NLI model.
        #[arg(long)]
        nli: bool,
    },

    /// Show configured models and download status.
    List {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },

    /// Remove broken cache files (.incomplete >1h old, empty refs/main).
    Clean {},
}

fn main() -> anyhow::Result<()> {
    // MCP stdio uses stdout as the transport — tracing must go to
    // stderr to avoid corrupting the protocol.
    let is_mcp_stdio = std::env::args().any(|a| a == "mcp-stdio");

    let subscriber = tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
    );

    if is_mcp_stdio {
        subscriber.with_writer(std::io::stderr).init();
    } else {
        subscriber.init();
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { repo, local_only } => {
            let msg = cogz::init::run(&repo, local_only)?;
            println!("{}", msg);
            Ok(())
        }
        Commands::Status { repo } => commands::run_status(&repo),
        Commands::Index { repo, no_download } => commands::run_index(&repo, no_download),
        Commands::Reindex { repo } => commands::run_reindex(&repo),
        Commands::Reset { repo, purge } => commands::run_reset(&repo, purge),
        Commands::Search {
            query,
            repo,
            entity_type,
            status,
            limit,
            no_expand,
            code,
        } => cli::run_search(&query, &repo, entity_type, status, limit, no_expand, code),
        Commands::Context {
            mode,
            query,
            repo,
            include_stale,
            max_tokens,
        } => cli::run_context(&mode, query.as_deref(), &repo, include_stale, max_tokens),
        Commands::McpStdio => cli::run_mcp_stdio(),
        Commands::Consolidate { repo, dry_run } => commands::run_consolidate(&repo, dry_run),
        Commands::CaptureEvent {
            event_type,
            repo,
            prompt,
            prompt_file,
            tool_name,
            tool_result,
            file_path,
            hook_json,
            fts_only,
        } => cli::run_capture_event(&cogz::hooks::CaptureInput {
            repo: &repo,
            event_str: &event_type,
            prompt: prompt.as_deref(),
            prompt_file: prompt_file.as_deref(),
            tool_name: tool_name.as_deref(),
            tool_result: tool_result.as_deref(),
            file_path: file_path.as_deref(),
            hook_json,
            fts_only,
        }),
        Commands::Models { subcommand } => match subcommand {
            ModelsSub::Download {
                repo,
                code,
                knowledge,
                nli,
            } => commands::run_models_download(&repo, code, knowledge, nli),
            ModelsSub::List { repo } => commands::run_models_list(&repo),
            ModelsSub::Clean {} => commands::run_models_clean(),
        },
        Commands::Doctor {
            repo,
            prune_observations,
            confirm,
        } => commands::run_doctor(&repo, prune_observations, confirm),
        Commands::Update {} => match cogz::update::run_update() {
            Ok(msg) => {
                println!("{}", msg);
                Ok(())
            }
            Err(e) => anyhow::bail!("update failed: {}", e),
        },
        Commands::EmbedBg {
            db,
            ids_file,
            code_model,
            dimension,
            idle_ttl,
            min_free_mb,
        } => commands::run_embed_bg(
            &db,
            &ids_file,
            &code_model,
            dimension,
            idle_ttl,
            min_free_mb,
        ),
    }
}
