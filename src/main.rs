//! CogZ CLI entry point.

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
        Commands::Init { repo } => cogz::init::run(&repo),
        Commands::Status { repo } => run_status(&repo),
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
    let conn = storage.conn();

    println!("CogZ status for project: {}\n", config.project.name);
    println!("  DB: {}", db_path.display());
    println!("  DB size: {} bytes", storage.db_size_bytes());

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

    Ok(())
}
