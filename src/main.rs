//! CLI entrypoint for the `baez` command.
//!
//! Handles error exit codes and command dispatch.

use baez::{
    api::ApiClient,
    auth::resolve_token,
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
            let docs = client.list_documents()?;

            for doc in docs {
                let date = doc.created_at.format("%Y-%m-%d");
                let title = doc.title.as_deref().unwrap_or("Untitled");
                println!("{}\t{}\t{}", doc.id, date, title);
            }
        }
        Some(baez::cli::Commands::Fetch { ref id }) => {
            let client = create_client(&cli)?;
            let paths = Paths::new(cli.vault)?;
            paths.ensure_dirs()?;

            // Fetch metadata and transcript, keeping raw responses
            let meta_resp = client.get_metadata_with_raw(id)?;
            let transcript_resp = client.get_transcript_with_raw(id)?;
            let meta = meta_resp.parsed;
            let transcript = transcript_resp.parsed;

            // Compute filename
            let slug = baez::util::slugify(meta.title.as_deref().unwrap_or("untitled"));
            let doc_path = paths.doc_path(&meta.created_at, &slug);
            let date = meta.created_at.format("%Y-%m-%d").to_string();
            let base_filename = format!("{}_{}", date, slug);

            // Convert to markdown (notes/summary fetched only during sync)
            let md = baez::convert::to_markdown(&transcript, &meta, id, None, None)?;
            let full_md = format!("---\n{}---\n\n{}", md.frontmatter_yaml, md.body);

            // Write files: save verbatim API responses as raw JSON
            let transcript_json_path = paths
                .raw_dir
                .join(format!("{}_transcript.json", base_filename));
            let metadata_json_path = paths
                .raw_dir
                .join(format!("{}_metadata.json", base_filename));

            baez::storage::write_atomic(
                &transcript_json_path,
                transcript_resp.raw.as_bytes(),
                &paths.tmp_dir,
            )?;
            baez::storage::write_atomic(
                &metadata_json_path,
                meta_resp.raw.as_bytes(),
                &paths.tmp_dir,
            )?;
            baez::storage::write_atomic(&doc_path, full_md.as_bytes(), &paths.tmp_dir)?;

            // Set file modification time to meeting creation date
            baez::storage::set_file_time(&transcript_json_path, &meta.created_at)?;
            baez::storage::set_file_time(&metadata_json_path, &meta.created_at)?;
            baez::storage::set_file_time(&doc_path, &meta.created_at)?;

            println!("wrote {}", transcript_json_path.display());
            println!("wrote {}", metadata_json_path.display());
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
        Some(baez::cli::Commands::Summarize { doc_id, save }) => {
            let paths = Paths::new(cli.vault)?;

            let config_path = paths.baez_dir.join("summary_config.json");
            let config = baez::summary::SummaryConfig::load(&config_path)?;

            // Find the markdown file for this doc_id (walk date-based tree)
            let md_path = find_transcript_by_id(&paths, &doc_id)?;

            // Find the corresponding raw transcript JSON
            let md_stem = md_path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| {
                    baez::Error::Filesystem(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Invalid filename",
                    ))
                })?;
            let transcript_json_path = paths.raw_dir.join(format!("{}_transcript.json", md_stem));

            if !transcript_json_path.exists() {
                return Err(baez::Error::Filesystem(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "Raw transcript not found: {}. Run `baez sync` first.",
                        transcript_json_path.display()
                    ),
                )));
            }

            // Parse raw transcript and build metadata from frontmatter
            let raw_json = std::fs::read_to_string(&transcript_json_path)?;
            let transcript: baez::model::RawTranscript = serde_json::from_str(&raw_json)?;

            let fm = baez::storage::read_frontmatter(&md_path)?.ok_or_else(|| {
                baez::Error::Filesystem(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "No frontmatter found in markdown file",
                ))
            })?;

            let meta = baez::model::DocumentMetadata {
                id: Some(fm.doc_id.clone()),
                title: fm.title.clone(),
                created_at: fm.created,
                updated_at: fm.updated,
                participants: fm.attendees.clone(),
                duration_seconds: fm.duration_minutes.map(|m| m * 60),
                labels: fm.tags.clone(),
                creator: None,
                attendees: None,
            };

            let api_key = baez::summary::get_api_key_verbose(cli.verbose).ok_or_else(|| {
                baez::Error::Auth(
                    "No Anthropic API key found. Set BAEZ_ANTHROPIC_API_KEY, ANTHROPIC_API_KEY, add anthropic_api_key to ~/.config/baez/config.json, or run `baez set-api-key`."
                        .into(),
                )
            })?;

            let claude_client = baez::summary::build_claude_client()?;
            let input = baez::summary::format_transcript_for_llm(&transcript, &meta);
            println!(
                "Summarizing with {} (max input: {} chars)...",
                config.model, config.max_input_chars
            );
            let summary =
                baez::summary::summarize_transcript(&input, &api_key, &config, &claude_client)?;

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

/// Find a transcript file by document ID, walking the date-based directory tree recursively
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
    let token = resolve_token(cli.token.clone(), cli.verbose)?;
    let mut client = ApiClient::new(token, Some(cli.api_base.clone()))?;

    if cli.no_throttle {
        client = client.disable_throttle();
    } else if let Some((min, max)) = cli.throttle_ms {
        client = client.with_throttle(min, max);
    }

    Ok(client)
}
