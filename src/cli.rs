//! Command-line interface definitions using clap.
//!
//! Defines all subcommands and global flags.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "baez")]
#[command(about = "Sync Granola meeting transcripts into an Obsidian vault", long_about = None)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Granola token (overrides BAEZ_GRANOLA_TOKEN env var, config file, and session)
    #[arg(long, global = true)]
    pub token: Option<String>,

    /// API base URL
    #[arg(long, global = true, default_value = "https://api.granola.ai")]
    pub api_base: String,

    /// Path to Obsidian vault root (overrides BAEZ_VAULT env var and config file)
    #[arg(long, global = true)]
    pub vault: Option<PathBuf>,

    /// Print diagnostic details (token source, API key source, full errors)
    #[arg(long, short, global = true)]
    pub verbose: bool,

    /// Disable throttling (not recommended)
    #[arg(long, global = true)]
    pub no_throttle: bool,

    /// Throttle range in ms (min:max)
    #[arg(long, global = true, value_parser = parse_throttle_range)]
    pub throttle_ms: Option<(u64, u64)>,
}

fn parse_throttle_range(s: &str) -> Result<(u64, u64), String> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return Err("Expected format: min:max".into());
    }

    let min = parts[0].parse().map_err(|_| "Invalid min value")?;
    let max = parts[1].parse().map_err(|_| "Invalid max value")?;

    if min > max {
        return Err("min must be <= max".into());
    }

    Ok((min, max))
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Sync all documents
    Sync {
        /// Force re-sync of all documents, ignoring cache timestamps
        #[arg(long)]
        force: bool,
        /// Skip AI summarization even if API key is configured
        #[arg(long)]
        no_summarize: bool,
        /// Preview what would be synced without writing any files
        #[arg(long)]
        dry_run: bool,
    },

    /// List all documents
    List,

    /// Fetch a specific document by ID
    Fetch {
        /// Document ID to fetch
        id: String,
    },

    /// Open the vault directory in the system file browser
    Open,

    /// Fix file modification dates to match meeting creation dates
    FixDates,

    /// Store Anthropic API key in system keychain (macOS only)
    #[cfg(feature = "summaries")]
    SetApiKey {
        /// Anthropic API key
        api_key: String,
    },

    /// Configure summarization settings (model, max input size, prompt)
    #[cfg(feature = "summaries")]
    SetConfig {
        /// Claude model to use (e.g., claude-opus-4-6, claude-sonnet-4-20250514)
        #[arg(long)]
        model: Option<String>,

        /// Max input size in characters
        #[arg(long)]
        context_window: Option<usize>,

        /// Path to custom prompt file
        #[arg(long)]
        prompt_file: Option<std::path::PathBuf>,

        /// Show current configuration
        #[arg(long)]
        show: bool,
    },

    /// Summarize a transcript using Claude
    #[cfg(feature = "summaries")]
    Summarize {
        /// Document ID to summarize
        doc_id: String,

        /// Save summary to file (default: print to stdout)
        #[arg(long)]
        save: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_parse_throttle_range_valid() {
        let result = parse_throttle_range("100:300").unwrap();
        assert_eq!(result, (100, 300));
    }

    #[test]
    fn test_parse_throttle_range_invalid() {
        assert!(parse_throttle_range("300:100").is_err());
        assert!(parse_throttle_range("abc:def").is_err());
        assert!(parse_throttle_range("100").is_err());
    }

    #[test]
    fn test_no_args_yields_none_command() {
        let cli = Cli::try_parse_from(["baez"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_sync_subcommand_yields_some() {
        let cli = Cli::try_parse_from(["baez", "sync"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Sync { .. })));
    }
}
