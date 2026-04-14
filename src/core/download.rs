use std::io::Write;
use std::path::{Path, PathBuf};

use futures::StreamExt;
use std::future::Future;
use std::pin::Pin;
use tracing::{debug, info};

use crate::core::client::BandcampClient;
use crate::core::library::{AudioFormat, ItemType, LibraryItem};
use crate::error::{BandcampError, Result};

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

/// Trait for reporting download progress
pub trait DownloadProgressReporter: Send + Sync {
    /// Called when download starts (returns total size if known)
    fn on_start(&self, total_size: Option<u64>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Called when fetching download URL
    fn on_fetching_url(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Called during download with current progress
    fn on_progress(
        &self,
        downloaded: u64,
        total: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Called when extracting (for albums/packages)
    fn on_extracting(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Called when download completes successfully
    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Called when download fails
    fn on_error(&self, error: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Generic download function that works for both CLI and TUI
pub async fn download_item<P: DownloadProgressReporter>(
    client: &BandcampClient,
    item: &LibraryItem,
    output_dir: &Path,
    format: AudioFormat,
    name_format: Option<&str>,
    reporter: P,
) -> Result<PathBuf> {
    info!("Downloading: {} - {}", item.artist, item.title);

    // Fetch download URL
    reporter.on_fetching_url().await;
    let download_url = client.get_download_url_with_retry(item, format, 30).await?;
    debug!("Download URL: {download_url}");

    // Start download
    let response = client.download(&download_url).await?;

    if !response.status().is_success() {
        let error_msg = format!(
            "HTTP {}: {}",
            response.status(),
            response.status().canonical_reason().unwrap_or("Unknown")
        );
        reporter.on_error(&error_msg).await;
        return Err(BandcampError::DownloadError(error_msg));
    }

    let total_size = response.content_length();
    reporter.on_start(total_size).await;

    // Create temporary file
    let temp_path = output_dir.join(format!(".{}.tmp", item.id));
    std::fs::create_dir_all(output_dir)?;
    let mut file = std::fs::File::create(&temp_path)?;

    // Download with progress reporting
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| BandcampError::DownloadError(e.to_string()))?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        reporter.on_progress(downloaded, total_size).await;
    }

    file.flush()?;
    drop(file);

    let filename = item.construct_filename(format, name_format);

    let output_path = if item.item_type == ItemType::Track {
        // For tracks, rename the temp file
        let final_path = output_dir.join(&filename);
        let tp = temp_path.clone();
        let fp = final_path.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(parent) = fp.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&tp, &fp)?;
            Ok(())
        })
        .await
        .map_err(|e| BandcampError::DownloadError(format!("Task join error: {e}")))??;

        final_path
    } else {
        // For albums and packages, extract the zip archive
        reporter.on_extracting().await;
        let extract_path = output_dir.join(&filename);
        let tp = temp_path.clone();
        let ep = extract_path.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            std::fs::create_dir_all(&ep)?;
            extract_zip(&tp, &ep)?;
            std::fs::remove_file(&tp)?;
            Ok(())
        })
        .await
        .map_err(|e| BandcampError::DownloadError(format!("Task join error: {e}")))??;

        extract_path
    };

    reporter.on_complete().await;
    info!("Completed: {filename}");

    Ok(output_path)
}

/// Extracts a ZIP archive to the specified directory.
///
/// Returns the `output_dir` path on success.
pub fn extract_zip(zip_path: &Path, output_dir: &Path) -> Result<()> {
    debug!("Extracting {zip_path:?} to {output_dir:?}");

    let file = std::fs::File::open(zip_path)?;

    zip::ZipArchive::new(file)
        .map_err(|e| BandcampError::DownloadError(format!("Invalid ZIP file: {e}")))?
        .extract(output_dir)
        .map_err(|e| BandcampError::DownloadError(format!("ZIP extraction failed: {e}")))?;

    fix_permissions(output_dir)?;

    Ok(())
}

/// Recursively resets permissions to 0755 for directories and 0644 for files.
fn fix_permissions(path: &Path) -> Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            std::fs::set_permissions(entry.path(), Permissions::from_mode(0o755))?;
            fix_permissions(&entry.path())?;
        } else {
            std::fs::set_permissions(entry.path(), Permissions::from_mode(0o644))?;
        }
    }

    Ok(())
}
