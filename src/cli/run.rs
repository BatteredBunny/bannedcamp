use std::time::Duration;

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use tracing::info;

use crate::cli::commands::{BandcampUrl, DownloadArgs, DownloadTarget};
use crate::cli::download::DownloadManager;
use crate::core::client::BandcampClient;
use crate::core::library::LibraryItem;
use crate::core::utils::sanitize_filename;

pub async fn run_download(args: DownloadArgs) -> Result<()> {
    // Fallback to looking for BANDCAMP_COOKIE env variable
    let cookie = args
        .cookie
        .or_else(|| std::env::var("BANDCAMP_COOKIE").ok())
        .ok_or_else(|| {
            anyhow::anyhow!("No cookie provided. Set --cookie flag, BANDCAMP_COOKIE env var")
        })?;

    let mut client = BandcampClient::new();

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    spinner.enable_steady_tick(Duration::from_millis(80));

    spinner.set_message("Validating session...");
    info!("Validating session cookie...");
    client.validate_cookie(&cookie).await?;

    spinner.set_message("Loading library...");
    info!("Fetching library...");
    let items = client.get_collection().await?;
    info!("Found {} items in library", items.len());

    spinner.finish_and_clear();

    let items_to_download = match &args.target {
        DownloadTarget::All => items,
        DownloadTarget::Url { urls } => {
            info!("Filtering by {} URL(s)", urls.len());
            let parsed: Vec<_> = urls.iter().filter_map(|u| BandcampUrl::parse(u)).collect();
            items
                .into_iter()
                .filter(|item| item_matches_urls(item, &parsed))
                .collect()
        }
    };

    // Filter out existing downloads if skip_existing is set
    let items_to_download = if args.skip_existing {
        let before_count = items_to_download.len();
        let filtered: Vec<_> = items_to_download
            .into_iter()
            .filter(|item| {
                let path = args.output.join(sanitize_filename(
                    &item.construct_filename(args.format, args.custom_format.as_deref()),
                ));
                !path.exists()
            })
            .collect();
        let skipped = before_count - filtered.len();
        if skipped > 0 {
            info!("Skipping {skipped} existing downloads");
        }
        filtered
    } else {
        items_to_download
    };

    if items_to_download.is_empty() {
        match &args.target {
            DownloadTarget::All => {
                if args.skip_existing {
                    println!("All items already downloaded");
                } else {
                    println!("No items found in library");
                }
            }
            DownloadTarget::Url { urls } => {
                if args.skip_existing {
                    println!("All matching items already downloaded");
                } else {
                    println!("No items found matching URL(s): {}", urls.join(", "));
                }
            }
        }
        return Ok(());
    }

    if args.dry_run {
        println!("Would download {} items.", items_to_download.len());
        for item in &items_to_download {
            let dir_name = sanitize_filename(&format!("{} - {}", item.artist, item.title));
            println!("{}", args.output.join(dir_name).display());
        }
    } else {
        let manager = DownloadManager::new(
            client,
            args.output,
            args.format,
            args.custom_format,
            args.parallel as usize,
        );

        let summary = manager.download_items(items_to_download).await?;

        println!(
            "Downloaded {} items, {} failed.",
            summary.success_count(),
            summary.failure_count()
        );

        for (_, path) in &summary.succeeded {
            println!("{}", path.display());
        }
    }

    Ok(())
}

fn item_matches_urls(item: &LibraryItem, urls: &[BandcampUrl]) -> bool {
    urls.iter().any(|url| {
        if url.is_artist_url() {
            item.artist_subdomain
                .as_ref()
                .is_some_and(|s| s.eq_ignore_ascii_case(&url.artist))
        } else {
            item.slug
                .as_ref()
                .is_some_and(|s| url.slug.as_ref().is_some_and(|u| s.eq_ignore_ascii_case(u)))
        }
    })
}
