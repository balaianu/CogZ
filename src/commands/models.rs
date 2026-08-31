//! `cogz models` subcommand handlers — download, list, clean.

use std::io::Write;
use std::path::Path;

use cogz::config::Config;

use super::load_config;

/// `cogz models list` — show configured models and download status.
pub fn run_models_list(repo: &Path) -> anyhow::Result<()> {
    let (config, _) = load_config(repo)?;
    let models_dir = crate::cli::models_dir();

    println!("Configured models:\n");
    print_model_status("code", &config.embedding.code_model, &models_dir);
    print_model_status("knowledge", &config.embedding.knowledge_model, &models_dir);
    if !config.embedding.nli_model.is_empty() {
        print_model_status("nli", &config.embedding.nli_model, &models_dir);
    }

    println!("\n  Models directory: {}", models_dir.display());
    Ok(())
}

fn print_model_status(label: &str, model_id: &str, models_dir: &Path) {
    let cached = cogz::embed::is_model_cached(model_id, models_dir);
    println!(
        "  {:10} {} — {}",
        label,
        model_id,
        if cached {
            "downloaded"
        } else {
            "not downloaded"
        }
    );
}

/// `cogz models download` — download configured models.
pub fn run_models_download(
    repo: &Path,
    code: bool,
    knowledge: bool,
    nli: bool,
) -> anyhow::Result<()> {
    let (config, _) = load_config(repo)?;
    let models_dir = crate::cli::models_dir();

    let download_any = code || knowledge || nli;
    let download_all = !download_any;

    if code || download_all {
        download_one("code", &config.embedding.code_model, &models_dir)?;
    }
    if knowledge || download_all {
        download_one("knowledge", &config.embedding.knowledge_model, &models_dir)?;
    }
    if (nli || download_all) && !config.embedding.nli_model.is_empty() {
        download_one("nli", &config.embedding.nli_model, &models_dir)?;
    }

    Ok(())
}

fn download_one(label: &str, model_id: &str, models_dir: &Path) -> anyhow::Result<()> {
    if cogz::embed::is_model_cached(model_id, models_dir) {
        println!("  {} ({}) — already cached", label, model_id);
        return Ok(());
    }
    print!("  {} ({}) — downloading... ", label, model_id);
    std::io::stdout().flush()?;

    match cogz::embed::download_model(model_id, models_dir) {
        Ok(result) => {
            println!("done ({})", result.model_dir.display());
            Ok(())
        }
        Err(e) => {
            println!("failed");
            anyhow::bail!("failed to download {}: {}", model_id, e);
        }
    }
}

/// `cogz models clean` — run clean_broken_cache manually.
pub fn run_models_clean() -> anyhow::Result<()> {
    let models_dir = crate::cli::models_dir();
    println!("Cleaning model cache at {}...", models_dir.display());
    let removed = cogz::embed::clean_broken_cache(&models_dir);
    println!("  Removed {} broken cache file(s)", removed);
    Ok(())
}

/// Download models that are not yet cached. Called from `run_index`
/// when auto_download is enabled. Failures are logged but do not
/// stop indexing — the system degrades to FTS-only.
pub(crate) fn auto_download_models(config: &Config) {
    let models_dir = crate::cli::models_dir();

    let mut model_ids: Vec<&str> = vec![
        config.embedding.code_model.as_str(),
        config.embedding.knowledge_model.as_str(),
    ];
    if !config.embedding.nli_model.is_empty() {
        model_ids.push(config.embedding.nli_model.as_str());
    }

    for model_id in &model_ids {
        if cogz::embed::is_model_cached(model_id, &models_dir) {
            continue;
        }
        tracing::info!("auto-downloading model: {}", model_id);
        if let Err(e) = cogz::embed::download_model(model_id, &models_dir) {
            tracing::warn!(
                "auto-download failed for {} — continuing in FTS-only mode: {}",
                model_id,
                e
            );
            return;
        }
    }
}
