use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;
use tracing::{debug, error, info};

use crate::core::client::BandcampClient;
use crate::core::library::{AudioFormat, LibraryItem};
use crate::core::utils::sanitize_filename;
use crate::error::{BandcampError, Result};

pub struct DownloadManager {
    client: Arc<BandcampClient>,
    output_dir: PathBuf,
    format: AudioFormat,
    parallel: usize,
    progress: MultiProgress,
}

impl DownloadManager {
    pub fn new(
        client: BandcampClient,
        output_dir: PathBuf,
        format: AudioFormat,
        parallel: usize,
    ) -> Self {
        Self {
            client: Arc::new(client),
            output_dir,
            format,
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

            let handle = tokio::spawn(async move {
                let result = download_item(&client, &item, &output_dir, format, &progress).await;
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

async fn download_item(
    client: &BandcampClient,
    item: &LibraryItem,
    output_dir: &PathBuf,
    format: AudioFormat,
    progress: &MultiProgress,
) -> Result<PathBuf> {
    info!("Downloading: {} - {}", item.artist, item.title);

    let pb = progress.add(ProgressBar::new(0));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} {msg} [{bar:30.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec})")
            .unwrap()
            .progress_chars("#>-"),
    );
    let display_name = format!("{} - {}", item.artist, item.title);
    let short_name = if display_name.len() > 40 {
        format!("{}...", &display_name[..37])
    } else {
        display_name.clone()
    };
    pb.set_message(short_name.clone());

    // Get download URL with retry for pending encodings (up to 60 seconds of polling)
    let download_url = client.get_download_url_with_retry(item, format, 30).await?;
    debug!("Download URL: {download_url}");

    let response = client.download(&download_url).await?;

    if !response.status().is_success() {
        pb.finish_with_message("Failed!");
        return Err(BandcampError::DownloadError(format!(
            "HTTP {}: {}",
            response.status(),
            response.status().canonical_reason().unwrap_or("Unknown")
        )));
    }

    let total_size = response.content_length().unwrap_or(0);
    pb.set_length(total_size);

    // Temporary file for the download, should add download resuming with this
    let temp_path = output_dir.join(format!(".{}.tmp", item.id));
    std::fs::create_dir_all(output_dir)?;

    let mut file: std::fs::File = std::fs::File::create(&temp_path)?;
    let mut downloaded: u64 = 0;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| BandcampError::DownloadError(e.to_string()))?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }

    file.flush()?;
    drop(file);

    pb.set_message(format!("{short_name} (extracting)"));

    let album_dir_name = sanitize_filename(&format!("{} - {}", item.artist, item.title));
    let extract_dir = output_dir.join(&album_dir_name);
    std::fs::create_dir_all(&extract_dir)?;

    extract_zip(&temp_path, &extract_dir)?;

    std::fs::remove_file(&temp_path)?;

    pb.finish_and_clear();
    info!("Completed: {} - {}", item.artist, item.title);

    Ok(extract_dir)
}

/// Extracts a ZIP archive to the specified directory.
///
/// Returns the `output_dir` path on success.
pub fn extract_zip(zip_path: &PathBuf, output_dir: &PathBuf) -> Result<()> {
    debug!("Extracting {zip_path:?} to {output_dir:?}");

    let file = std::fs::File::open(zip_path)?;

    zip::ZipArchive::new(file)
        .map_err(|e| BandcampError::DownloadError(format!("Invalid ZIP file: {e}")))?
        .extract(output_dir)
        .map_err(|e| BandcampError::DownloadError(format!("ZIP extraction failed: {e}")))?;

    Ok(())
}

/// Summary of download results
#[derive(Debug, Default)]
pub struct DownloadSummary {
    pub succeeded: Vec<(LibraryItem, PathBuf)>,
    pub failed: Vec<(LibraryItem, String)>,
}

impl DownloadSummary {
    pub fn total(&self) -> usize {
        self.succeeded.len() + self.failed.len()
    }

    pub fn success_count(&self) -> usize {
        self.succeeded.len()
    }

    pub fn failure_count(&self) -> usize {
        self.failed.len()
    }
}
