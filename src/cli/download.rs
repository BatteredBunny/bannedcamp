use std::{
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::error::Result;
use crate::{
    cli::commands::AudioFormat,
    core::{
        client::BandcampClient,
        download::{DownloadProgressReporter, DownloadSummary, download_item},
        library::LibraryItem,
        utils::truncate_str,
    },
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;
use tracing::{error, info};

pub struct DownloadManager {
    client: Arc<BandcampClient>,
    output_dir: PathBuf,
    format: AudioFormat,
    name_format: Option<String>,
    parallel: usize,
    progress: MultiProgress,
}

impl DownloadManager {
    pub fn new(
        client: BandcampClient,
        output_dir: PathBuf,
        format: AudioFormat,
        name_format: Option<String>,
        parallel: usize,
    ) -> Self {
        Self {
            client: Arc::new(client),
            output_dir,
            format,
            name_format,
            parallel,
            progress: MultiProgress::new(),
        }
    }

    pub async fn download_items(&self, items: Vec<LibraryItem>) -> Result<DownloadSummary> {
        let total = items.len();
        info!("Starting download of {total} items");

        let header_pb = self.progress.add(ProgressBar::new_spinner());
        header_pb.set_style(ProgressStyle::default_spinner().template("{msg}").unwrap());
        header_pb.set_message(format!("{total} items remaining"));

        let remaining = Arc::new(AtomicUsize::new(total));
        let semaphore = Arc::new(Semaphore::new(self.parallel));
        let mut handles = Vec::new();

        let mut summary = DownloadSummary::default();

        for item in items {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let client = self.client.clone();
            let output_dir = self.output_dir.clone();
            let format = self.format;
            let progress = self.progress.clone();
            let header_pb = header_pb.clone();
            let remaining = remaining.clone();
            let name_format = self.name_format.clone();

            let handle = tokio::spawn(async move {
                let result = cli_download(&client, &item, &output_dir, format, name_format.as_deref(), &progress).await;
                let new_remaining = remaining.fetch_sub(1, Ordering::SeqCst) - 1;
                header_pb.set_message(format!("{new_remaining} items remaining"));
                drop(permit);
                (item, result)
            });

            handles.push(handle);
        }

        for handle in handles {
            match handle.await {
                Ok((item, Ok(path))) => {
                    summary.succeeded.push((item, path));
                }
                Ok((item, Err(e))) => {
                    error!("Failed to download {}: {e}", item.title);
                    summary.failed.push((item, e.to_string()));
                }
                Err(e) => {
                    error!("Task panicked: {e}");
                }
            }
        }

        header_pb.finish_and_clear();

        Ok(summary)
    }
}

pub struct CliProgressReporter {
    pb: ProgressBar,
    display_name: String,
}

impl CliProgressReporter {
    pub fn new(progress: &MultiProgress, artist: &str, title: &str) -> Self {
        let pb = progress.add(ProgressBar::new(0));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} {msg} [{bar:30.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec})")
                .unwrap()
                .progress_chars("#>-"),
        );

        let display_name = format!("{} - {}", artist, title);
        let short_name = truncate_str(&display_name, 37);

        Self {
            pb,
            display_name: short_name,
        }
    }
}

impl DownloadProgressReporter for CliProgressReporter {
    fn on_start(&self, total_size: Option<u64>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if let Some(size) = total_size {
                self.pb.set_length(size);
            }
            self.pb.set_message(self.display_name.clone());
        })
    }

    fn on_fetching_url(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.pb
                .set_message(format!("{} (fetching URL)", self.display_name));
        })
    }

    fn on_progress(
        &self,
        downloaded: u64,
        _total: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.pb.set_position(downloaded);
        })
    }

    fn on_extracting(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.pb
                .set_message(format!("{} (extracting)", self.display_name));
        })
    }

    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.pb.finish_and_clear();
        })
    }

    fn on_error(&self, _error: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.pb.finish_with_message("Failed!");
        })
    }
}

// Usage in CLI:
async fn cli_download(
    client: &BandcampClient,
    item: &LibraryItem,
    output_dir: &PathBuf,
    format: AudioFormat,
    name_format: Option<&str>,
    progress: &MultiProgress,
) -> Result<PathBuf> {
    let reporter = CliProgressReporter::new(progress, &item.artist, &item.title);
    download_item(client, item, output_dir, format, name_format, reporter).await
}
