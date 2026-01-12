use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use url::Url;

pub use crate::core::library::AudioFormat;

#[derive(Parser, Debug)]
#[command(name = "bannedcamp")]
#[command(author, version, about = "Bandcamp library downloader", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress output
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Launch interactive library downloader
    Library {
        /// Output directory for downloads
        #[arg(short, long, default_value = ".")]
        output: std::path::PathBuf,
    },

    /// Download items from library
    Download {
        #[command(flatten)]
        args: DownloadArgs,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Debug, Args)]
pub struct DownloadArgs {
    /// Bandcamp identity cookie (can also be set via BANDCAMP_COOKIE env vars)
    #[arg(long, global = true)]
    pub cookie: Option<String>,

    /// Audio format
    #[arg(short, long, value_enum, default_value = "flac", global = true)]
    pub format: AudioFormat,
    /// Output directory
    #[arg(short, long, default_value = ".", global = true)]
    pub output: PathBuf,

    /// Concurrent downloads
    #[arg(long, default_value = "3", global = true)]
    pub parallel: u8,

    /// Show what would be downloaded without downloading
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Skip downloads that already exist
    #[arg(long, global = true)]
    pub skip_existing: bool,

    /**
      Custom name format for download outputs
      Note! This does not modify the files inside album packages, only the top-level folder/file name.

      Variables:
      - {artist}: Artist name
      - {title}: Item title (track or album name)
      - {ext}: File extension (e.g., .flac, .mp3), only used for single track downloads, will be empty otherwise. Note that the extension includes the dot!
      - {id}: Item ID

      Examples:
      - "{artist} - {title}{ext}" -> "Clark Rainbow - Chainsaw.flac" (when downloading a single track URL https://clarkrainbow.bandcamp.com/track/chainsaw)
      - "{artist} - {title}" -> "Bad Math - Missing Narrative"
      - "{artist}/{title}" -> "Bad Math/Missing Narrative"

      Default:
        "{artist} - {title}" for albums,
        "{artist} - {title}{ext}" for tracks

    */
    #[arg(long, global = true, verbatim_doc_comment)]
    pub custom_format: Option<String>,

    #[command(subcommand)]
    pub target: DownloadTarget,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DownloadTarget {
    /// Download all items from your library
    All,

    /// Download items from urls
    Url {
        /**
            One or more Bandcamp URLs (artist, album, tracks)
            Examples:
            https://badmathhk.bandcamp.com                          (all from artist)
            https://badmathhk.bandcamp.com/album/missing-narrative  (specific album)
            https://clarkrainbow.bandcamp.com/track/chainsaw        (specific track)
        */
        #[arg(required = true, num_args = 1.., verbatim_doc_comment)]
        urls: Vec<String>,
    },
}

/// Parsed Bandcamp URL with extracted components
#[derive(Debug)]
pub struct BandcampUrl {
    /// Artist subdomain (e.g., example from example.bandcamp.com)
    pub artist: String,
    /// Album/track slug (e.g /album/example)
    pub slug: Option<String>,
}

impl BandcampUrl {
    /// Parse a Bandcamp URL string.
    /// Returns None if the URL is invalid or not a bandcamp.com URL.
    pub fn parse(input: &str) -> Option<Self> {
        let url = Url::parse(input).ok()?;
        let host = url.host_str()?;

        if !host.ends_with(".bandcamp.com") {
            return None;
        }

        let artist = host.strip_suffix(".bandcamp.com")?.to_string();
        let slug = url.path_segments()?.nth(1).map(String::from);

        Some(Self { artist, slug })
    }

    /// Returns true if this URL points to an artist page (no specific album/track)
    pub fn is_artist_url(&self) -> bool {
        self.slug.is_none()
    }
}

#[derive(ValueEnum, Clone, Debug)]
pub enum Shell {
    Bash,
    Fish,
    Zsh,
}

impl From<Shell> for clap_complete::Shell {
    fn from(shell: Shell) -> Self {
        match shell {
            Shell::Bash => clap_complete::Shell::Bash,
            Shell::Fish => clap_complete::Shell::Fish,
            Shell::Zsh => clap_complete::Shell::Zsh,
        }
    }
}
