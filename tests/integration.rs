use std::future::Future;
use std::pin::Pin;

use bannedcamp::core::auth::Credentials;
use bannedcamp::core::client::BandcampClient;
use bannedcamp::core::download::{DownloadProgressReporter, download_item};
use bannedcamp::core::library::AudioFormat;

fn get_cookie() -> Option<String> {
    std::env::var("BANDCAMP_COOKIE").ok()
}

async fn authenticated_client(cookie: &str) -> (BandcampClient, Credentials) {
    let mut client = BandcampClient::new();
    let creds = client
        .validate_cookie(cookie)
        .await
        .expect("failed to validate cookie");
    (client, creds)
}

struct NoopReporter;

impl DownloadProgressReporter for NoopReporter {
    fn on_start(&self, _total_size: Option<u64>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    fn on_fetching_url(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    fn on_progress(
        &self,
        _downloaded: u64,
        _total: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    fn on_extracting(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    fn on_error(&self, _error: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn test_validate_cookie() {
    let Some(cookie) = get_cookie() else {
        return;
    };

    let mut client = BandcampClient::new();
    let creds = client
        .validate_cookie(&cookie)
        .await
        .expect("validate_cookie should succeed with a valid cookie");

    assert!(creds.fan_id > 0, "fan_id should be positive");
}

#[tokio::test]
async fn test_fetch_collection_summary() {
    let Some(cookie) = get_cookie() else {
        return;
    };

    let client = BandcampClient::new();
    let summary = client
        .fetch_collection_summary(&cookie)
        .await
        .expect("fetch_collection_summary should succeed");

    assert!(summary.fan_id > 0, "fan_id should be positive");
    assert!(
        !summary.collection_summary.username.is_empty(),
        "username should be non-empty"
    );
}

#[tokio::test]
async fn test_get_collection() {
    let Some(cookie) = get_cookie() else {
        return;
    };

    let (client, _creds) = authenticated_client(&cookie).await;
    let items = client
        .get_collection()
        .await
        .expect("get_collection should succeed");

    assert!(
        !items.is_empty(),
        "collection should have at least one item"
    );

    for item in &items {
        assert!(!item.title.is_empty(), "item title should be non-empty");
        assert!(!item.artist.is_empty(), "item artist should be non-empty");
        assert!(!item.is_preorder, "preorders should be filtered out");
    }
}

#[tokio::test]
async fn test_collection_pagination() {
    let Some(cookie) = get_cookie() else {
        return;
    };

    let (client, _creds) = authenticated_client(&cookie).await;

    let summary = client
        .fetch_collection_summary(&cookie)
        .await
        .expect("fetch_collection_summary should succeed");
    let summary_count = summary.collection_summary.tralbum_lookup.len();

    let items = client
        .get_collection()
        .await
        .expect("get_collection should succeed");

    let mut seen_ids = std::collections::HashSet::new();
    for item in &items {
        assert!(seen_ids.insert(&item.id), "duplicate item id: {}", item.id);
    }

    // get_collection filters preorders, so count may be lower than summary
    assert!(
        items.len() <= summary_count,
        "get_collection returned more items ({}) than summary ({})",
        items.len(),
        summary_count,
    );

    // Large gap would indicate lost pages
    if summary_count > 0 {
        let ratio = items.len() as f64 / summary_count as f64;
        assert!(
            ratio > 0.5,
            "get_collection returned suspiciously few items: {} vs {} in summary (ratio {:.2})",
            items.len(),
            summary_count,
            ratio,
        );
    }
}

#[tokio::test]
async fn test_get_download_url() {
    let Some(cookie) = get_cookie() else {
        return;
    };

    let (client, _creds) = authenticated_client(&cookie).await;
    let items = client
        .get_collection()
        .await
        .expect("get_collection should succeed");

    let item = items
        .iter()
        .find(|i| !i.download_url.is_empty())
        .expect("need at least one downloadable item");

    let url = client
        .get_download_url_with_retry(item, AudioFormat::Mp3320, 5)
        .await
        .expect("get_download_url_with_retry should succeed");

    assert!(
        url.starts_with("http"),
        "download URL should start with http, got: {url}"
    );
}

#[tokio::test]
#[ignore] // slow -- run with `cargo test -- --include-ignored`
async fn test_download_item() {
    let Some(cookie) = get_cookie() else {
        return;
    };

    let (client, _creds) = authenticated_client(&cookie).await;
    let items = client
        .get_collection()
        .await
        .expect("get_collection should succeed");

    let item = items
        .iter()
        .find(|i| !i.download_url.is_empty())
        .expect("need at least one downloadable item");

    let tmp = tempfile::tempdir().expect("failed to create tempdir");

    let path = download_item(
        &client,
        item,
        tmp.path(),
        AudioFormat::Mp3320,
        None,
        NoopReporter,
    )
    .await
    .expect("download_item should succeed");

    assert!(path.exists(), "downloaded file should exist at {path:?}");
}
