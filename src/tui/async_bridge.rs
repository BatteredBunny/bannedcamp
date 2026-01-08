use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use tokio::sync::{Semaphore, mpsc};
use tracing::debug;

use crate::core::auth::Credentials;
use crate::core::client::BandcampClient;
use crate::core::download::extract_zip;
use crate::core::library::{AudioFormat, LibraryItem};
use crate::core::utils::sanitize_filename;
use crate::tui::app::MAX_CONCURRENT_DOWNLOADS;

type DownloadResult = Result<PathBuf, String>;

/// Messages sent from the TUI to the async runtime
#[derive(Debug)]
pub enum AsyncRequest {
    ValidateCookie(String),
    FetchCollection,
    StartBatchDownload {
        items: Vec<LibraryItem>,
        format: AudioFormat,
        output_dir: PathBuf,
    },
}

/// Messages sent from the async runtime to the TUI
#[derive(Debug)]
pub enum AsyncResponse {
    CookieValidated(Result<Credentials, String>),
    CollectionFetched(Result<Vec<LibraryItem>, String>),
    /// Batch download started with total item count
    BatchDownloadStarted {
        total_items: usize,
    },
    /// Individual item download started
    ItemDownloadStarted {
        item_id: String,
        item_index: usize,
    },
    /// Progress for current item
    DownloadProgress {
        item_id: String,
        downloaded: u64,
        total: Option<u64>,
    },
    /// Individual item completed
    ItemDownloadComplete {
        item_id: String,
        item_index: usize,
        result: Result<PathBuf, String>,
    },
    /// Entire batch completed
    BatchDownloadComplete,
}

/// Bridge between sync TUI and async operations
pub struct AsyncBridge {
    request_rx: mpsc::Receiver<AsyncRequest>,
    response_tx: mpsc::Sender<AsyncResponse>,
    client: Option<Arc<BandcampClient>>,
}

impl AsyncBridge {
    pub fn new(
        request_rx: mpsc::Receiver<AsyncRequest>,
        response_tx: mpsc::Sender<AsyncResponse>,
    ) -> Self {
        Self {
            request_rx,
            response_tx,
            client: None,
        }
    }

    pub async fn run(mut self) {
        while let Some(request) = self.request_rx.recv().await {
            debug!("Received async request: {request:?}");

            match request {
                AsyncRequest::ValidateCookie(cookie) => {
                    let result = self.validate_cookie(&cookie).await;
                    let _ = self
                        .response_tx
                        .send(AsyncResponse::CookieValidated(result))
                        .await;
                }
                AsyncRequest::FetchCollection => {
                    let result = self.fetch_collection().await;
                    let _ = self
                        .response_tx
                        .send(AsyncResponse::CollectionFetched(result))
                        .await;
                }
                AsyncRequest::StartBatchDownload {
                    items,
                    format,
                    output_dir,
                } => {
                    self.handle_batch_download(items, format, output_dir).await;
                }
            }
        }
    }

    async fn validate_cookie(&mut self, cookie: &str) -> Result<Credentials, String> {
        let mut client = BandcampClient::new();
        match client.validate_cookie(cookie).await {
            Ok(creds) => {
                self.client = Some(Arc::new(client));
                Ok(creds)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    async fn fetch_collection(&self) -> Result<Vec<LibraryItem>, String> {
        let client = self.client.as_ref().ok_or("Not logged in")?;
        client.get_collection().await.map_err(|e| e.to_string())
    }

    async fn handle_batch_download(
        &self,
        items: Vec<LibraryItem>,
        format: AudioFormat,
        output_dir: PathBuf,
    ) {
        let total_items = items.len();

        // Notify batch started
        let _ = self
            .response_tx
            .send(AsyncResponse::BatchDownloadStarted { total_items })
            .await;

        let client = match self.client.as_ref() {
            Some(c) => c.clone(),
            None => {
                // Report error for first item and complete batch
                if let Some(item) = items.first() {
                    let _ = self
                        .response_tx
                        .send(AsyncResponse::ItemDownloadComplete {
                            item_id: item.id.clone(),
                            item_index: 0,
                            result: Err("Not logged in".to_string()),
                        })
                        .await;
                }
                let _ = self
                    .response_tx
                    .send(AsyncResponse::BatchDownloadComplete)
                    .await;
                return;
            }
        };

        // Create semaphore for limiting concurrent downloads
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS));
        let mut handles = Vec::new();

        for (item_index, item) in items.into_iter().enumerate() {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let client = client.clone();
            let response_tx = self.response_tx.clone();
            let output_dir = output_dir.clone();

            let handle = tokio::spawn(async move {
                download_single_item(client, response_tx, item, item_index, format, output_dir)
                    .await;
                drop(permit);
            });

            handles.push(handle);
        }

        // Wait for all downloads to complete
        for handle in handles {
            let _ = handle.await;
        }

        let _ = self
            .response_tx
            .send(AsyncResponse::BatchDownloadComplete)
            .await;
    }
}

async fn download_single_item(
    client: Arc<BandcampClient>,
    response_tx: mpsc::Sender<AsyncResponse>,
    item: LibraryItem,
    item_index: usize,
    format: AudioFormat,
    output_dir: PathBuf,
) {
    let item_id = item.id.clone();

    let _ = response_tx
        .send(AsyncResponse::ItemDownloadStarted {
            item_id: item_id.clone(),
            item_index,
        })
        .await;

    let result = do_download(&client, &response_tx, &item, &item_id, format, &output_dir).await;

    let _ = response_tx
        .send(AsyncResponse::ItemDownloadComplete {
            item_id,
            item_index,
            result,
        })
        .await;
}

async fn do_download(
    client: &BandcampClient,
    response_tx: &mpsc::Sender<AsyncResponse>,
    item: &LibraryItem,
    item_id: &str,
    format: AudioFormat,
    output_dir: &Path,
) -> DownloadResult {
    let download_url = client
        .get_download_url_with_retry(item, format, 30)
        .await
        .map_err(|e| format!("Failed to get download URL: {e}"))?;

    let response = client
        .download(&download_url)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let total = response.content_length();

    let album_dir = sanitize_filename(&format!("{} - {}", item.artist, item.title));
    let extract_dir = output_dir.join(&album_dir);
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create directory: {e}"))?;

    let temp_path = output_dir.join(format!(".{}.tmp", item.id));
    let mut file = std::fs::File::create(&temp_path)
        .map_err(|e| format!("Failed to create temp file: {e}"))?;

    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();
    let mut last_progress = Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream error: {e}"))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Write error: {e}"))?;
        downloaded += chunk.len() as u64;

        if last_progress.elapsed().as_millis() >= 100 {
            let _ = response_tx
                .send(AsyncResponse::DownloadProgress {
                    item_id: item_id.to_string(),
                    downloaded,
                    total,
                })
                .await;
            last_progress = Instant::now();
        }
    }

    drop(file);

    extract_zip(&temp_path, &extract_dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&temp_path); // TODO: handle error

    Ok(extract_dir)
}
