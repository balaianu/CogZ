//! CogZ CLI entry point.

mod cli;
mod cli_embed;
mod cli_events;
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

        /// Wire host-agent MCP + hook config after init.
        /// Comma-separated harness ids (claude-code, cursor, codex,
        /// gemini, copilot, devin) or `auto` to configure every
        /// detected agent.
        #[arg(long)]
        configure: Option<String>,

        /// With --configure: write user-level (global) agent config
        /// instead of project files.
        #[arg(long, requires = "configure")]
        global: bool,
    },

    /// Write host-agent MCP + hook configuration for this repo.
    /// Merges into existing files (with a .cogz.bak backup) and never
    /// clobbers other servers or hooks.
    Configure {
        /// Harness(es) to configure: claude-code, cursor, codex,
        /// gemini, copilot, devin — comma-separated, or `auto` to
        /// configure every detected agent.
        harnesses: String,

        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// Write user-level (global) agent config instead of project
        /// files.
        #[arg(long)]
        global: bool,
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

        /// Also remove observations. Keeps knowledge, rules, config,
        /// and the generated .gitignore. Prints a dry run unless
        /// --confirm is passed.
        #[arg(long)]
        purge: bool,

        /// Apply a --purge reset (without it, --purge only previews).
        #[arg(long)]
        confirm: bool,
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

        /// Only use git history committed before this unix timestamp
        /// for the co-change channel.
        #[arg(long)]
        before: Option<i64>,

        /// Inspection mode: read results without recording usage
        /// telemetry (access counts, delivery rows). Use for humans
        /// or tooling that look at results without consuming them.
        #[arg(long)]
        no_record: bool,
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

        /// Inspection mode: assemble the pack without recording usage
        /// telemetry or closing open deliveries. Use for humans or
        /// tooling that inspect packs without injecting them.
        #[arg(long)]
        no_record: bool,
    },

    /// List mined observation candidates — the review surface the
    /// write-back nudges point at. Nothing is written; confirming a
    /// candidate goes through `create_entity` or a manual file.
    Suggest {
        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// How far back to mine events and usage, in days.
        #[arg(long, default_value = "7")]
        days: u32,

        /// Max candidates to return.
        #[arg(long, default_value = "10")]
        limit: u32,
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
    /// manually — see cli-reference for per-event behavior.
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
        /// found.
        #[arg(long)]
        hook_json: bool,

        /// Skip model loading — FTS-only search. Much faster but
        /// lower quality ranking. Recommended for hook calls.
        #[arg(long)]
        fts_only: bool,

        /// Hook response envelope for agents whose stdout contract
        /// isn't Claude-shaped: `cursor` emits {"additional_context"},
        /// `copilot` emits {"additionalContext"}. Requires --hook-json.
        #[arg(long, value_parser = ["claude", "cursor", "copilot"], default_value = "claude", requires = "hook_json")]
        hook_format: String,

        /// Record the event and run the lifecycle; emit nothing on stdout.
        #[arg(long)]
        record_only: bool,
    },

    /// Verify a knowledge entity against its referenced code —
    /// re-stamps `verified_against` provenance, clears drift rows,
    /// and reactivates the entity when all references resolve active.
    /// Use after confirming drifted or orphaned knowledge is still
    /// accurate.
    Verify {
        /// Entity UUID to verify. Omit when a selection flag is used.
        entity_id: Option<String>,
        /// Reactivate even when some references are dead.
        #[arg(long)]
        allow_dead_refs: bool,
        /// Batch: verify every entity with drift rows.
        #[arg(long, conflicts_with_all = ["entity_id", "stale", "unverified", "all"])]
        drifted: bool,
        /// Batch: verify every stale file-backed entity.
        #[arg(long, conflicts_with_all = ["entity_id", "unverified", "all"])]
        stale: bool,
        /// Batch: verify living entities with no provenance baseline.
        #[arg(long, conflicts_with_all = ["entity_id", "all"])]
        unverified: bool,
        /// Batch: verify every living file-backed entity.
        #[arg(long, conflicts_with = "entity_id")]
        all: bool,

        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },

    /// Reject a knowledge-layer entity — writes `status: rejected`
    /// into its file, validated against the status lattice. Only
    /// `active` entities can be rejected.
    Reject {
        /// Entity UUID to reject.
        entity_id: String,

        /// Why the entity is rejected — stored as `rejected_reason`
        /// in the file's frontmatter.
        #[arg(long)]
        reason: Option<String>,

        /// Repository root directory. Defaults to current directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
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

        /// Purge stored telemetry (events, deliveries, entity_usage).
        /// Dry-run lists row counts; --confirm deletes them.
        #[arg(long)]
        purge_telemetry: bool,

        /// Confirm pruning (deletes files, creates tombstones).
        #[arg(long)]
        confirm: bool,

        /// Batch-verify knowledge claims: drifted | stale | unverified | all.
        /// Re-stamps provenance and reports dead references over the
        /// selected queue.
        #[arg(long, value_name = "MODE")]
        verify: Option<String>,
    },

    /// Self-update — download and install the latest release from GitHub.
    /// Use --check to see if an update is available without installing.
    Update {
        /// Check for updates without downloading or installing.
        #[arg(long)]
        check: bool,
    },

    /// Background code reindex — spawned by hooks to catch changes
    /// from non-hook events (branch switches, pulls, merges, human
    /// edits). Not intended for direct use.
    ReindexBg {
        /// Repository root directory.
        #[arg(long)]
        repo: PathBuf,

        /// Database path.
        #[arg(long)]
        db: PathBuf,
    },

    /// Background code embedding — spawned by `cogz index` for large
    /// codebases. Not intended for direct use.
    EmbedBg {
        /// Database path.
        #[arg(long)]
        db: PathBuf,

        /// File containing entity IDs (one per line).
        #[arg(long)]
        ids_file: PathBuf,

        /// Repo .cogz directory — holds .bg.lock for the
        /// single-background-worker gate.
        #[arg(long)]
        cogz_dir: PathBuf,

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

        /// Parent already holds .bg.lock — run as the designated worker.
        #[arg(long)]
        lock_held: bool,
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

/// Shared `configure` invocation: resolve home, scope, and the cogz
/// binary path, then write the harness config.
fn run_configure(repo: &std::path::Path, arg: &str, global: bool) -> anyhow::Result<()> {
    let home =
        std::env::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    let scope = if global {
        cogz::configure::Scope::User
    } else {
        cogz::configure::Scope::Project
    };
    print!(
        "{}",
        cogz::configure::run(repo, arg, scope, &home, &cogz::configure::self_bin())?
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    use std::io::IsTerminal;

    // Diagnostics never go to stdout: MCP stdio uses stdout as the
    // transport, and hook commands emit machine-readable JSON there.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            repo,
            local_only,
            configure,
            global,
        } => {
            let msg = cogz::init::run(&repo, local_only)?;
            println!("{}", msg);
            match configure {
                Some(arg) => run_configure(&repo, &arg, global)?,
                None => {
                    let home = std::env::home_dir()
                        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
                    let detected = cogz::configure::detect(&repo, &home);
                    if !detected.is_empty() {
                        println!(
                            "  agents detected: {}\n  wire MCP + hooks: cogz configure auto",
                            detected.join(", ")
                        );
                    }
                }
            }
            Ok(())
        }
        Commands::Configure {
            harnesses,
            repo,
            global,
        } => run_configure(&repo, &harnesses, global),
        Commands::Status { repo } => commands::run_status(&repo),
        Commands::Index { repo, no_download } => commands::run_index(&repo, no_download),
        Commands::Reindex { repo } => commands::run_reindex(&repo),
        Commands::Reset {
            repo,
            purge,
            confirm,
        } => commands::run_reset(&repo, purge, confirm),
        Commands::Search {
            query,
            repo,
            entity_type,
            status,
            limit,
            no_expand,
            code,
            before,
            no_record,
        } => cli::run_search(
            &query,
            &repo,
            entity_type,
            status,
            limit,
            no_expand,
            code,
            before,
            no_record,
        ),
        Commands::Context {
            mode,
            query,
            repo,
            include_stale,
            max_tokens,
            no_record,
        } => cli::run_context(
            &mode,
            query.as_deref(),
            &repo,
            include_stale,
            max_tokens,
            no_record,
        ),
        Commands::Suggest { repo, days, limit } => cli_events::run_suggest(&repo, days, limit),
        Commands::McpStdio => cli_events::run_mcp_stdio(),
        Commands::Consolidate { repo, dry_run } => commands::run_consolidate(&repo, dry_run),
        Commands::Verify {
            entity_id,
            allow_dead_refs,
            drifted,
            stale,
            unverified,
            all,
            repo,
        } => {
            let selection = if drifted {
                Some(cogz::index::drift::VerifySelection::Drifted)
            } else if stale {
                Some(cogz::index::drift::VerifySelection::Stale)
            } else if unverified {
                Some(cogz::index::drift::VerifySelection::Unverified)
            } else if all {
                Some(cogz::index::drift::VerifySelection::All)
            } else {
                None
            };
            commands::run_verify(&repo, entity_id.as_deref(), selection, allow_dead_refs)
        }
        Commands::Reject {
            entity_id,
            reason,
            repo,
        } => commands::run_reject(&repo, &entity_id, reason.as_deref()),
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
            hook_format,
            record_only,
        } => cli_events::run_capture_event(&cogz::hooks::CaptureInput {
            repo: &repo,
            event_str: &event_type,
            prompt: prompt.as_deref(),
            prompt_file: prompt_file.as_deref(),
            tool_name: tool_name.as_deref(),
            tool_result: tool_result.as_deref(),
            file_path: file_path.as_deref(),
            hook_json,
            fts_only,
            hook_format: cogz::hooks::format::HookFormat::parse(&hook_format).unwrap_or_default(),
            record_only,
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
            purge_telemetry,
            confirm,
            verify,
        } => commands::run_doctor(&repo, prune_observations, purge_telemetry, confirm, verify),
        Commands::Update { check } => {
            if check {
                match cogz::update::check_latest_version() {
                    Ok(cogz::update::VersionStatus::UpToDate(v)) => {
                        println!("CogZ is up to date (v{})", v)
                    }
                    Ok(cogz::update::VersionStatus::UpdateAvailable { current, latest }) => {
                        println!("Update available: v{} → v{}", current, latest)
                    }
                    Err(e) => anyhow::bail!("update check failed: {}", e),
                }
            } else {
                match cogz::update::run_update() {
                    Ok(msg) => println!("{}", msg),
                    Err(e) => anyhow::bail!("update failed: {}", e),
                }
            }
            Ok(())
        }
        Commands::EmbedBg {
            db,
            ids_file,
            cogz_dir,
            code_model,
            dimension,
            idle_ttl,
            min_free_mb,
            lock_held,
        } => commands::run_embed_bg(&commands::embed_bg::EmbedBgArgs {
            db_path: db,
            ids_file,
            cogz_dir,
            code_model,
            dimension,
            idle_ttl,
            min_free_mb,
            lock_held,
        }),
        Commands::ReindexBg { repo, db } => {
            if let Err(e) = commands::run_reindex_bg(&repo, &db) {
                eprintln!("reindex-bg: error: {}", e);
                std::process::exit(1);
            }
            Ok(())
        }
    }
}
