use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{Semaphore, mpsc};
use tracing::debug;

use crate::core::auth::Credentials;
use crate::core::client::BandcampClient;
use crate::core::download::{DownloadProgressReporter, download_item};
use crate::core::library::{AudioFormat, LibraryItem};
use crate::tui::app::MAX_CONCURRENT_DOWNLOADS;

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
                tui_download(client, response_tx, item, item_index, format, output_dir).await;
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

pub struct TuiProgressReporter {
    item_id: String,
    response_tx: mpsc::Sender<AsyncResponse>,
    last_progress: std::sync::Mutex<Instant>,
}

impl TuiProgressReporter {
    pub fn new(item_id: String, response_tx: mpsc::Sender<AsyncResponse>) -> Self {
        Self {
            item_id,
            response_tx,
            last_progress: std::sync::Mutex::new(Instant::now()),
        }
    }
}

impl DownloadProgressReporter for TuiProgressReporter {
    fn on_start(&self, _total_size: Option<u64>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {})
    }

    fn on_fetching_url(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {})
    }

    fn on_progress(
        &self,
        downloaded: u64,
        total: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // Throttle updates to every 100ms
            let should_send = {
                let mut last = self.last_progress.lock().unwrap();
                if last.elapsed().as_millis() >= 100 {
                    *last = Instant::now();
                    true
                } else {
                    false
                }
            };

            if should_send {
                let _ = self
                    .response_tx
                    .send(AsyncResponse::DownloadProgress {
                        item_id: self.item_id.clone(),
                        downloaded,
                        total,
                    })
                    .await;
            }
        })
    }

    fn on_extracting(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {})
    }

    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {})
    }

    fn on_error(&self, _error: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {})
    }
}

// Usage in TUI:
async fn tui_download(
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

    let reporter = TuiProgressReporter::new(item_id.to_string(), response_tx.clone());

    let result = download_item(&client, &item, &output_dir, format, reporter)
        .await
        .map_err(|e| e.to_string());

    let _ = response_tx
        .send(AsyncResponse::ItemDownloadComplete {
            item_id,
            item_index,
            result,
        })
        .await;
}
