//! CLI entrypoint for the `baez` command.
//!
//! Handles error exit codes and command dispatch.

use baez::{
    api::ApiClient,
    auth::resolve_api_key,
    cli::Cli,
    storage::Paths,
    sync::{fix_dates, sync_all},
    Result,
};
use clap::Parser;

fn main() {
    if let Err(e) = run() {
        eprintln!("baez: [E{}] {}", e.exit_code(), e);
        std::process::exit(e.exit_code());
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    baez::storage::warn_config_permissions();

    match cli.command {
        None => {
            use clap::CommandFactory;
            Cli::command().print_help().expect("Failed to print help");
            println!();
        }
        Some(baez::cli::Commands::Sync {
            force,
            no_summarize,
            dry_run,
        }) => {
            let client = create_client(&cli)?;
            let paths = Paths::new(cli.vault)?;
            sync_all(&client, &paths, force, !no_summarize, cli.verbose, dry_run)?;
        }
        Some(baez::cli::Commands::List) => {
            let client = create_client(&cli)?;
            let notes = client.list_notes(None)?;

            for note in notes {
                let date = note.created_at.format("%Y-%m-%d");
                let title = note.title.as_deref().unwrap_or("Untitled");
                println!("{}\t{}\t{}", note.id, date, title);
            }
        }
        Some(baez::cli::Commands::Fetch { ref id }) => {
            let client = create_client(&cli)?;
            let paths = Paths::new(cli.vault)?;
            paths.ensure_dirs()?;

            let note_resp = client.get_note_with_raw(id)?;
            let note = note_resp.parsed;

            let slug = baez::util::doc_slug(note.title.as_deref(), id);
            let doc_path = paths.doc_path(&note.created_at, &slug);
            let date = note.created_at.format("%Y-%m-%d").to_string();
            let base_filename = format!("{}_{}", date, slug);

            let md = baez::convert::to_markdown(&note, None, vec![], None)?;
            let full_md = format!("---\n{}---\n\n{}", md.frontmatter_yaml, md.body);

            let note_json_path = paths.raw_dir.join(format!("{}_note.json", base_filename));

            baez::storage::write_atomic(&note_json_path, note_resp.raw.as_bytes(), &paths.tmp_dir)?;
            baez::storage::write_atomic(&doc_path, full_md.as_bytes(), &paths.tmp_dir)?;

            baez::storage::set_file_time(&note_json_path, &note.created_at)?;
            baez::storage::set_file_time(&doc_path, &note.created_at)?;

            println!("wrote {}", note_json_path.display());
            println!("wrote {}", doc_path.display());
        }
        Some(baez::cli::Commands::Open) => {
            let paths = Paths::new(cli.vault)?;
            paths.ensure_dirs()?;

            if let Err(e) = open::that(&paths.granola_dir) {
                eprintln!("Failed to open vault directory: {}", e);
                std::process::exit(1);
            }
            println!("Opened vault directory: {}", paths.granola_dir.display());
        }
        Some(baez::cli::Commands::FixDates) => {
            let paths = Paths::new(cli.vault)?;
            fix_dates(&paths)?;
        }
        Some(baez::cli::Commands::SetGranolaApiKey { api_key }) => {
            baez::auth::set_api_key_in_keychain(&api_key)?;
            println!("Granola API key stored in macOS keychain");
        }
        #[cfg(feature = "summaries")]
        Some(baez::cli::Commands::SetApiKey { api_key }) => {
            baez::summary::set_api_key_in_keychain(&api_key)?;
        }
        #[cfg(feature = "summaries")]
        Some(baez::cli::Commands::SetConfig {
            model,
            context_window,
            prompt_file,
            show,
        }) => {
            let paths = Paths::new(cli.vault)?;
            let config_path = paths.baez_dir.join("summary_config.json");

            if show {
                let config = baez::summary::SummaryConfig::load(&config_path)?;
                println!("Current summarization configuration:");
                println!("  Model: {}", config.model);
                println!("  Max input: {} characters", config.max_input_chars);
                println!("  Max tokens: {}", config.max_tokens);
                println!(
                    "  Custom prompt: {}",
                    if config.custom_prompt.is_some() {
                        "Yes"
                    } else {
                        "No (using default)"
                    }
                );
                if let Some(prompt) = &config.custom_prompt {
                    println!("\nCustom prompt:");
                    println!("{}", prompt);
                }
                return Ok(());
            }

            let mut config = baez::summary::SummaryConfig::load(&config_path)?;

            if let Some(m) = model {
                config.model = m;
            }
            if let Some(cw) = context_window {
                config.max_input_chars = cw;
            }
            if let Some(pf) = prompt_file {
                let prompt = std::fs::read_to_string(&pf)?;
                config.custom_prompt = Some(prompt);
            }

            config.save(&config_path, &paths.tmp_dir)?;
            println!("Configuration saved");
            println!("  Model: {}", config.model);
            println!("  Max input: {} characters", config.max_input_chars);
        }
        #[cfg(feature = "summaries")]
        Some(baez::cli::Commands::SummarizeAll { force, dry_run }) => {
            let paths = Paths::new(cli.vault)?;
            paths.ensure_dirs()?;
            baez::sync::summarize_all_docs(&paths, force, cli.verbose, dry_run)?;
        }
        #[cfg(feature = "summaries")]
        Some(baez::cli::Commands::Summarize { doc_id, save }) => {
            let paths = Paths::new(cli.vault)?;

            let config_path = paths.baez_dir.join("summary_config.json");
            let config = baez::summary::SummaryConfig::load(&config_path)?;

            let md_path = find_transcript_by_id(&paths, &doc_id)?;

            let md_stem = md_path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| {
                    baez::Error::Filesystem(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Invalid filename",
                    ))
                })?;
            let note_json_path = paths.raw_dir.join(format!("{}_note.json", md_stem));

            if !note_json_path.exists() {
                return Err(baez::Error::Filesystem(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "Raw note JSON not found: {}. Run `baez sync` first.",
                        note_json_path.display()
                    ),
                )));
            }

            let raw_json = std::fs::read_to_string(&note_json_path)?;
            let note: baez::Note = serde_json::from_str(&raw_json)?;

            let api_key = baez::summary::get_api_key_verbose(cli.verbose).ok_or_else(|| {
                baez::Error::Auth(
                    "No Anthropic API key found. Set BAEZ_ANTHROPIC_API_KEY, ANTHROPIC_API_KEY, add anthropic_api_key to ~/.config/baez/config.json, or run `baez set-api-key`."
                        .into(),
                )
            })?;

            let claude_client = baez::summary::build_claude_client()?;
            let input = baez::summary::format_transcript_for_llm(&note);
            println!(
                "Summarizing with {} (max input: {} chars)...",
                config.model, config.max_input_chars
            );
            let raw_summary =
                baez::summary::summarize_transcript(&input, &api_key, &config, &claude_client, "")?;
            let (summary, _entities) = baez::summary::parse_summary_output(&raw_summary);

            if save {
                let content = std::fs::read_to_string(&md_path)?;
                let updated = baez::summary::update_summary_in_markdown(&content, &summary);
                baez::storage::write_atomic(&md_path, updated.as_bytes(), &paths.tmp_dir)?;
                println!("Summary updated in: {}", md_path.display());
            } else {
                println!("\n{}\n", summary);
            }
        }
    }

    Ok(())
}

/// Find a markdown file by document ID, walking the date-based directory tree recursively.
#[cfg(feature = "summaries")]
fn find_transcript_by_id(paths: &Paths, doc_id: &str) -> baez::Result<std::path::PathBuf> {
    use std::fs;

    fn walk_for_id(
        dir: &std::path::Path,
        doc_id: &str,
    ) -> baez::Result<Option<std::path::PathBuf>> {
        if !dir.exists() {
            return Ok(None);
        }
        let entries = fs::read_dir(dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') {
                        continue;
                    }
                }
                if let Some(found) = walk_for_id(&path, doc_id)? {
                    return Ok(Some(found));
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
                if let Some(fm) = baez::storage::read_frontmatter(&path)? {
                    if fm.doc_id == doc_id {
                        return Ok(Some(path));
                    }
                }
            }
        }
        Ok(None)
    }

    walk_for_id(&paths.granola_dir, doc_id)?.ok_or_else(|| {
        baez::Error::Filesystem(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("No transcript found for document ID: {}", doc_id),
        ))
    })
}

/// Creates an API client with auth and throttle configuration from CLI flags.
fn create_client(cli: &Cli) -> Result<ApiClient> {
    let api_key = resolve_api_key(cli.api_key.clone(), cli.verbose)?;
    let mut client = ApiClient::new(api_key, Some(cli.api_base.clone()))?;

    if cli.no_throttle {
        client = client.disable_throttle();
    } else if let Some((min, max)) = cli.throttle_ms {
        client = client.with_throttle(min, max);
    }

    Ok(client)
}
