//! CLI command handlers — status, index, reindex, reset, consolidate.
//!
//! These functions are the dispatch targets for the CLI subcommands
//! defined in `main.rs`. They handle config loading, storage setup,
//! and user-facing output via `println!`.

pub mod doctor;
pub mod models;

use std::path::Path;

use cogz::config::Config;

use crate::cli;

pub use doctor::run_doctor;
pub use models::{run_models_clean, run_models_download, run_models_list};

pub(crate) fn load_config(repo: &Path) -> anyhow::Result<(Config, std::path::PathBuf)> {
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
    Ok((config, db_path))
}

pub fn run_status(repo: &Path) -> anyhow::Result<()> {
    let (config, db_path) = load_config(repo)?;

    if !db_path.exists() {
        println!("CogZ status for project: {}\n", config.project.name);
        println!(
            "  DB not found at {}. Run `cogz index` to build it.",
            db_path.display()
        );
        return Ok(());
    }

    let storage = cogz::storage::Storage::open(&db_path, config.embedding.dimension)?;

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

    let models_dir = cli::models_dir();
    let code_model = cogz::embed::OnnxEmbeddingModel::with_model_id(
        cogz::embed::ModelType::Code,
        &models_dir,
        config.embedding.dimension,
        &config.embedding.code_model,
    );
    let knowledge_model = cogz::embed::OnnxEmbeddingModel::with_model_id(
        cogz::embed::ModelType::Knowledge,
        &models_dir,
        config.embedding.dimension,
        &config.embedding.knowledge_model,
    );
    println!("\n  Models:");
    println!(
        "    Code ({}): {}",
        config.embedding.code_model,
        if code_model.model_files_exist() {
            "available"
        } else {
            "not found"
        }
    );
    println!(
        "    Knowledge ({}): {}",
        config.embedding.knowledge_model,
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

pub fn run_index(repo: &Path, no_download: bool) -> anyhow::Result<()> {
    let (config, db_path) = load_config(repo)?;
    let cogz_dir = repo.join(".cogz");
    let storage = cogz::storage::Storage::open(&db_path, config.embedding.dimension)?;

    if config.embedding.auto_download && !no_download {
        models::auto_download_models(&config);
    }

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

    if !result.synced_entity_ids.is_empty() {
        let embedded = cli::embed_synced(&storage, &config, &result.synced_entity_ids);
        if embedded > 0 {
            println!("  Embedded: {}", embedded);
        }
    }

    println!("\nIndexing source code...");
    let code_result = cogz::index::index_code(&storage, repo, &config);
    println!(
        "  Code entities: {} created, {} updated, {} stale, {} skipped",
        code_result.created, code_result.updated, code_result.marked_stale, code_result.skipped
    );

    if !code_result.synced_entity_ids.is_empty() {
        let embedded = cli::embed_synced(&storage, &config, &code_result.synced_entity_ids);
        if embedded > 0 {
            println!("  Code embedded: {}", embedded);
        }
    }

    let conn = storage.conn();
    let total = cogz::storage::crud::count_all(&conn)?;
    println!("\n  Total entities: {}", total);

    if let Some(sha) = cogz::index::git_diff::head_sha(repo)
        && let Err(e) = cogz::storage::set_meta(&conn, "last_indexed_commit", &sha)
    {
        tracing::warn!("failed to record last_indexed_commit: {}", e);
    }

    Ok(())
}

pub fn run_reindex(repo: &Path) -> anyhow::Result<()> {
    let (config, db_path) = load_config(repo)?;
    let cogz_dir = repo.join(".cogz");

    if !db_path.exists() {
        anyhow::bail!(
            "Database not found at {}. Run `cogz index` first.",
            db_path.display()
        );
    }

    let storage = cogz::storage::Storage::open(&db_path, config.embedding.dimension)?;

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

    if !result.synced_entity_ids.is_empty() {
        let embedded = cli::embed_synced(&storage, &config, &result.synced_entity_ids);
        if embedded > 0 {
            println!("  Embedded: {}", embedded);
        }
    }

    println!("\nReindexing source code...");
    let code_result = cogz::index::reindex_code(&storage, repo, &config);

    if code_result.incremental {
        println!("  (incremental — git diff)");
    } else {
        println!("  (full scan — no git baseline)");
    }
    println!(
        "  Code entities: {} created, {} updated, {} stale, {} skipped",
        code_result.created, code_result.updated, code_result.marked_stale, code_result.skipped
    );

    if !code_result.synced_entity_ids.is_empty() {
        let embedded = cli::embed_synced(&storage, &config, &code_result.synced_entity_ids);
        if embedded > 0 {
            println!("  Code embedded: {}", embedded);
        }
    }

    let mut all_changed = code_result.changed_code_ids;
    all_changed.extend(code_result.deleted_code_ids.iter().cloned());
    if !all_changed.is_empty() {
        let flagged =
            cogz::index::stale_flagging::flag_stale_knowledge(&storage, &cogz_dir, &all_changed);
        if flagged > 0 {
            println!("  Stale knowledge flagged: {}", flagged);
        }
    }

    Ok(())
}

pub fn run_reset(repo: &Path, purge: bool) -> anyhow::Result<()> {
    let (_config, db_path) = load_config(repo)?;
    let cogz_dir = repo.join(".cogz");

    for suffix in &["", "-wal", "-shm"] {
        let path = format!("{}{}", db_path.display(), suffix);
        let path = std::path::Path::new(&path);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    println!("Dropped database: {}", db_path.display());

    if purge {
        let obs_dir = cogz_dir.join("observations");
        if obs_dir.exists() {
            std::fs::remove_dir_all(&obs_dir)?;
            std::fs::create_dir_all(&obs_dir)?;
            println!("Purged observations: {}", obs_dir.display());
        }

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

pub fn run_consolidate(repo: &Path, dry_run: bool) -> anyhow::Result<()> {
    let (config, db_path) = load_config(repo)?;
    let cogz_dir = repo.join(".cogz");

    if !db_path.exists() {
        anyhow::bail!(
            "Database not found at {}. Run `cogz index` first.",
            db_path.display()
        );
    }

    let storage = cogz::storage::Storage::open(&db_path, config.embedding.dimension)?;

    println!(
        "Consolidating{}...",
        if dry_run { " (dry run)" } else { "" }
    );

    let promoted = cogz::consolidate::promote::run_promotion(
        &storage,
        &cogz_dir,
        &config.consolidation,
        dry_run,
    )?;
    println!("\n  Promoted: {}", promoted.len());
    for p in &promoted {
        if dry_run {
            println!(
                "    [dry-run] observation {} → would create rule ({})",
                p.observation_id, p.reason
            );
        } else {
            println!(
                "    observation {} → rule {} ({})",
                p.observation_id, p.new_rule_id, p.reason
            );
        }
    }

    let merged =
        cogz::consolidate::merge::run_merge(&storage, &cogz_dir, &config.consolidation, dry_run)?;
    println!("\n  Merged: {}", merged.len());
    for m in &merged {
        println!(
            "    {} superseded by {} ({}){}",
            m.superseded_id,
            m.survivor_id,
            m.reason,
            if dry_run { " [dry-run]" } else { "" }
        );
    }

    if dry_run {
        println!("\n  (dry run — no changes made)");
    }

    Ok(())
}

/// Background code embedding — spawned by `cogz index` for large
/// codebases. Reads entity IDs from a file, embeds them with the code
/// model, and stores vectors in the DB. Deletes the ID file when done.
pub fn run_embed_bg(
    db_path: &Path,
    ids_file: &Path,
    code_model: &str,
    dimension: usize,
) -> anyhow::Result<()> {
    use cogz::embed::{EmbeddingCache, ModelType, OnnxEmbeddingModel};
    use cogz::storage::crud::get_entities_batch;

    // Read entity IDs from the file.
    let ids_content = std::fs::read_to_string(ids_file)?;
    let entity_ids: Vec<String> = ids_content
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();

    if entity_ids.is_empty() {
        let _ = std::fs::remove_file(ids_file);
        return Ok(());
    }

    // Open the DB. The dimension parameter must match the vec0 table
    // created during the initial index. It's passed from the parent.
    let storage = cogz::storage::Storage::open(db_path, dimension)?;

    // Fetch entity data under the lock, then drop it.
    let entities: Vec<_> = {
        let conn = storage.conn();
        get_entities_batch(&conn, &entity_ids).unwrap_or_default()
    };

    if entities.is_empty() {
        let _ = std::fs::remove_file(ids_file);
        return Ok(());
    }

    // Load the code model.
    let models_dir = cli::models_dir();
    let model =
        OnnxEmbeddingModel::with_model_id(ModelType::Code, &models_dir, dimension, code_model);

    if !model.model_files_exist() {
        tracing::warn!("background embed: code model not available");
        let _ = std::fs::remove_file(ids_file);
        return Ok(());
    }

    // Embed in chunks, storing after each chunk to show progress.
    let cache = EmbeddingCache::new();
    let chunk_size = 32;
    let mut total_embedded = 0;

    for chunk in entities.chunks(chunk_size) {
        let embeddings = cogz::files::embed_sync::embed_entities(&model, &cache, chunk);
        if !embeddings.is_empty() {
            let mut conn = storage.conn();
            total_embedded += cogz::files::embed_sync::store_embeddings(&mut conn, &embeddings);
        }
    }

    tracing::info!("background embed: {} entities embedded", total_embedded);

    // Clean up the ID file.
    let _ = std::fs::remove_file(ids_file);

    Ok(())
}
