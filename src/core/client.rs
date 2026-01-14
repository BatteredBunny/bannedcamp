use std::collections::HashMap;

use reqwest::header::{COOKIE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::core::auth::Credentials;
use crate::core::library::{AudioFormat, ItemType, LibraryItem};
use crate::error::{BandcampError, Result};

const BANDCAMP_BASE: &str = "https://bandcamp.com";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

/// Response from the collection_items API endpoint
#[derive(Debug, Deserialize)]
pub struct CollectionResponse {
    items: Vec<CollectionItem>,
    more_available: bool,
    last_token: Option<String>,
    /// Map of "{sale_item_type}{sale_item_id}" -> redownload URL
    #[serde(default)]
    redownload_urls: std::collections::HashMap<String, String>,
}

/// URL hints from collection item
#[derive(Debug, Deserialize, Default)]
struct UrlHints {
    subdomain: Option<String>,
    slug: Option<String>,
}

/// Individual item from collection response
#[derive(Debug, Deserialize)]
struct CollectionItem {
    band_id: u64,

    sale_item_id: u64,
    sale_item_type: String, // a, t, p

    tralbum_type: CollectionSummaryItemType,

    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    url_hints: Option<UrlHints>,
    item_title: String,
    #[serde(default)]
    item_url: Option<String>,
    #[serde(default)]
    band_name: String,
    #[serde(default)]
    is_preorder: bool,
}

pub struct BandcampClient {
    http: reqwest::Client,
    credentials: Option<Credentials>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CollectionSummary {
    pub fan_id: u64,
    pub collection_summary: CollectionSummaryInternal,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CollectionSummaryInternal {
    pub fan_id: u64,
    pub username: String,
    pub url: String,

    pub tralbum_lookup: std::collections::HashMap<String, CollectionSummaryItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CollectionSummaryItem {
    pub item_type: CollectionSummaryItemType, // a, t, p
    pub item_id: u64,
    pub band_id: u64,
    pub purchased: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum CollectionSummaryItemType {
    #[serde(rename = "a")]
    Album, // a
    #[serde(rename = "t")]
    Track, // t
    #[serde(rename = "p")]
    Package, // p
}

impl BandcampClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .cookie_store(true)
                .user_agent(USER_AGENT)
                .build()
                .expect("Failed to create HTTP client"),
            credentials: None,
        }
    }

    fn auth_headers(&self) -> Result<HeaderMap> {
        let creds = self
            .credentials
            .as_ref()
            .ok_or(BandcampError::NotLoggedIn)?;

        let mut headers = HeaderMap::new();
        let cookie_value = format!("identity={}", creds.identity_cookie);
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&cookie_value)
                .map_err(|e| BandcampError::AuthError(e.to_string()))?,
        );
        Ok(headers)
    }

    pub async fn fetch_collection_summary(
        &self,
        identity_cookie: &str,
    ) -> Result<CollectionSummary> {
        let mut headers = HeaderMap::new();
        let cookie_value = format!("identity={identity_cookie}");
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&cookie_value)
                .map_err(|e| BandcampError::AuthError(e.to_string()))?,
        );

        let response = self
            .http
            .get(format!("{BANDCAMP_BASE}/api/fan/2/collection_summary"))
            .headers(headers.clone())
            .send()
            .await?;

        if response.status().is_success() {
            Ok(response.json().await?)
        } else {
            Err(BandcampError::NetworkError(
                response.error_for_status().unwrap_err(),
            ))
        }
    }

    pub async fn collection_items(
        &self,
        fan_id: u64,
        older_than_token: &str,
    ) -> Result<CollectionResponse> {
        let url = format!("{BANDCAMP_BASE}/api/fancollection/1/collection_items");

        let body = serde_json::json!({
            "fan_id": fan_id,
            "count": 100,
            "older_than_token": older_than_token
        });

        debug!("Fetching collection page: {url} with body: {body:?}");

        let response = self
            .http
            .post(&url)
            .headers(self.auth_headers()?)
            .json(&body)
            .send()
            .await?;

        if response.status() == 401 {
            Err(BandcampError::SessionExpired)
        } else if response.status() == 503 {
            Err(BandcampError::SiteDown)
        } else if !response.status().is_success() {
            Err(BandcampError::NetworkError(
                response.error_for_status().unwrap_err(),
            ))
        } else {
            Ok(response.json().await?)
        }
    }

    /// Validate a session cookie by attempting to fetch the user's fan ID
    pub async fn validate_cookie(&mut self, identity_cookie: &str) -> Result<Credentials> {
        info!("Validating session cookie...");

        let summary = self.fetch_collection_summary(identity_cookie).await?;

        info!("Cookie validated, fan_id: {}", summary.fan_id);

        let credentials = Credentials::new(identity_cookie.to_string(), summary.fan_id);

        self.credentials = Some(credentials.clone());
        Ok(credentials)
    }

    /// Fetch the user's library collection
    pub async fn get_collection(&self) -> Result<Vec<LibraryItem>> {
        let creds = self
            .credentials
            .as_ref()
            .ok_or(BandcampError::NotLoggedIn)?;

        let fan_id = creds.fan_id;

        info!("Fetching collection for fan_id: {fan_id}");

        let mut token: String = format!("{}::a::", chrono::Utc::now().timestamp() + 86400);
        let mut items: HashMap<u64, LibraryItem> = HashMap::new();

        loop {
            let collection = self.collection_items(fan_id, &token).await?;

            debug!(
                "Fetched {} items, more_available: {}",
                collection.items.len(),
                collection.more_available
            );

            for item in collection.items {
                items.insert(
                    item.sale_item_id,
                    self.convert_collection_item(item, &collection.redownload_urls),
                );
            }

            if !collection.more_available {
                break;
            }

            if let Some(new_token) = collection.last_token {
                token = new_token;
            } else {
                break;
            }
        }

        info!("Fetched {} total items from collection", items.len());
        Ok(items.into_values().collect())
    }

    /// Convert API collection item to our LibraryItem type
    fn convert_collection_item(
        &self,
        item: CollectionItem,
        redownload_urls: &HashMap<String, String>,
    ) -> LibraryItem {
        let item_type = match item.tralbum_type {
            CollectionSummaryItemType::Album => ItemType::Album,
            CollectionSummaryItemType::Track => ItemType::Track,
            CollectionSummaryItemType::Package => ItemType::Package,
        };

        // Extract subdomain and slug from url_hints
        let (artist_subdomain, slug) = item
            .url_hints
            .as_ref()
            .map(|hints| (hints.subdomain.clone(), hints.slug.clone()))
            .unwrap_or((None, None));

        // Get redownload URL from the response map
        // Key format is "{sale_item_type}{sale_item_id}" e.g. "a12345" for album 12345
        let redownload_key = format!("{}{}", item.sale_item_type, item.sale_item_id);
        let download_url = redownload_urls
            .get(&redownload_key)
            .cloned()
            .unwrap_or_else(|| {
                // Fallback: construct URL if not in map
                format!(
                    "{BANDCAMP_BASE}/download?from=collection&payment_id={}&sitem_id={}",
                    item.sale_item_id, item.sale_item_id
                )
            });

        debug!(
            "Item {} ({redownload_key}) redownload URL: {download_url}",
            item.item_title,
        );

        LibraryItem {
            id: item.sale_item_id.to_string(),
            item_type,
            title: item.item_title,
            artist: item.band_name,
            artist_id: item.band_id.to_string(),
            artist_subdomain,
            slug,
            item_url: item.item_url,
            download_url,
            available_formats: vec![
                AudioFormat::Flac,
                AudioFormat::Mp3320,
                AudioFormat::Mp3V0,
                AudioFormat::Aac,
                AudioFormat::OggVorbis,
                AudioFormat::Alac,
                AudioFormat::Wav,
                AudioFormat::Aiff,
            ],
            is_preorder: item.is_preorder,
            is_hidden: item.hidden.unwrap_or(false),
        }
    }

    /// Make an authenticated GET request for downloading files
    pub async fn download(&self, url: &str) -> Result<reqwest::Response> {
        let response = self
            .http
            .get(url)
            .headers(self.auth_headers()?)
            .send()
            .await?;
        Ok(response)
    }

    /// Get download URL with retry logic for pending encodings
    /// Triggers encoding by requesting the download URL, then polls statdownload
    pub async fn get_download_url_with_retry(
        &self,
        item: &LibraryItem,
        format: AudioFormat,
        max_attempts: u32,
    ) -> Result<String> {
        info!(
            "Getting download URL for {} - {} ({})",
            item.artist,
            item.title,
            format.bandcamp_encoding()
        );

        // First, fetch the download page to get the format-specific URL
        debug!("Fetching download page: {}", item.download_url);

        let response = self
            .http
            .get(&item.download_url)
            .headers(self.auth_headers()?)
            .send()
            .await?;

        if response.status() == 401 {
            return Err(BandcampError::SessionExpired);
        } else if response.status() == 503 {
            return Err(BandcampError::SiteDown);
        }

        if !response.status().is_success() {
            return Err(BandcampError::DownloadError(format!(
                "Failed to fetch download page: HTTP {}",
                response.status()
            )));
        }

        let html = response.text().await?;

        // Check if already ready
        let is_ready = html.contains("\"ready\":true") || html.contains("\"ready\": true");
        if is_ready {
            debug!("Download is already ready");
            return self.extract_download_url(&html, format);
        }

        // Extract the download URL for this format
        let download_url = self.extract_download_url(&html, format)?;
        debug!("Download URL: {download_url}");

        // Trigger encoding by requesting the download URL
        // This will return HTML (preparing page) but triggers the encoding process
        debug!("Triggering encoding by requesting download URL...");
        let _ = self
            .http
            .get(&download_url)
            .headers(self.auth_headers()?)
            .send()
            .await;

        // Now poll statdownload until ready
        let stat_url_base = download_url.replace("/download/", "/statdownload/");

        for attempt in 1..=max_attempts {
            let stat_url = format!(
                "{stat_url_base}&.rand={}&.vrs=1",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );

            debug!("Polling statdownload (attempt {attempt}): {stat_url}");

            let stat_response = self
                .http
                .get(&stat_url)
                .headers(self.auth_headers()?)
                .send()
                .await?;

            let stat_text = stat_response.text().await.unwrap_or_default();
            debug!("Statdownload response: {stat_text}");

            // Check if encoding completed
            if stat_text.contains("\"result\":\"ok\"") || stat_text.contains("\"result\": \"ok\"") {
                // Extract the actual download URL from stat response
                if let Some(url_start) = stat_text.find("\"download_url\":\"") {
                    let start = url_start + 16;
                    if let Some(url_end) = stat_text[start..].find('"') {
                        let url = &stat_text[start..start + url_end];
                        let url = url.replace("\\/", "/");
                        info!("Download ready for {}", item.title);
                        return Ok(url);
                    }
                }
            }

            // Check for specific errors
            if stat_text.contains("\"errortype\":\"ExpirationError\"") {
                // Signature expired, need to get fresh URLs
                debug!("Signature expired, refreshing download page...");

                // Re-fetch download page for fresh URLs
                let response = self
                    .http
                    .get(&item.download_url)
                    .headers(self.auth_headers()?)
                    .send()
                    .await?;

                if response.status().is_success() {
                    let html = response.text().await?;
                    if html.contains("\"ready\":true") || html.contains("\"ready\": true") {
                        return self.extract_download_url(&html, format);
                    }
                }
            }

            if attempt < max_attempts {
                info!(
                    "Download for {} not ready, waiting... (attempt {attempt}/{max_attempts})",
                    item.title,
                );
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }

        Err(BandcampError::DownloadError(format!(
            "Download for {} - {} not ready after {} attempts. The encoding may take longer - try again in a few minutes.",
            item.artist, item.title, max_attempts
        )))
    }

    /// Extract download URL from download page HTML
    /// Looks for <div id="pagedata" data-blob="..."> containing JSON with digital_items
    fn extract_download_url(&self, html: &str, format: AudioFormat) -> Result<String> {
        let format_str = format.bandcamp_encoding();

        debug!(
            "Extracting download URL for format '{}' from HTML ({} chars)",
            format_str,
            html.len()
        );

        // Method 1: Look for <div id="pagedata" data-blob="...">
        // This is the primary method used by Bandcamp download pages
        if let Some(url) = self.extract_from_pagedata(html, format_str)? {
            return Ok(url);
        }

        // Method 2: Look for data-blob attribute on other elements
        if let Some(url) = self.extract_from_data_blob(html, format_str)? {
            return Ok(url);
        }

        // Method 3: Look for TralbumData in script tags (older pages)
        if let Some(url) = self.extract_from_tralbum_data(html, format_str)? {
            return Ok(url);
        }

        // Method 4: Direct pattern matching as fallback
        if let Some(url) = self.extract_direct_pattern(html, format_str)? {
            return Ok(url);
        }

        Err(BandcampError::ParseError(format!(
            "Could not find download URL for format '{format_str}' in page",
        )))
    }

    /// Extract download URL from pagedata div
    fn extract_from_pagedata(&self, html: &str, format: &str) -> Result<Option<String>> {
        // Look for <div id="pagedata" data-blob="...">
        // or <div id='pagedata' data-blob='...'>
        let pagedata_patterns = [
            r#"<div id="pagedata""#,
            r#"<div id='pagedata'"#,
            r#"id="pagedata""#,
        ];

        for pattern in pagedata_patterns {
            if let Some(div_pos) = html.find(pattern) {
                debug!("Found pagedata div at position {div_pos}");

                // Find data-blob attribute
                let search_start = div_pos;
                let search_end = html[search_start..].find('>').unwrap_or(2000) + search_start;
                let div_tag = &html[search_start..search_end];

                if let Some(blob_start) = div_tag.find("data-blob=") {
                    let quote_char = div_tag.chars().nth(blob_start + 10).unwrap_or('"');
                    let blob_content_start = blob_start + 11; // Skip 'data-blob="' or "data-blob='"

                    if let Some(blob_end) = div_tag[blob_content_start..].find(quote_char) {
                        let blob = &div_tag[blob_content_start..blob_content_start + blob_end];

                        // Unescape HTML entities
                        let unescaped = self.unescape_html(blob);
                        debug!("Pagedata blob length: {} chars", unescaped.len());

                        // Parse as JSON and extract download URL
                        if let Some(url) = self.extract_url_from_json(&unescaped, format) {
                            return Ok(Some(url));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Extract download URL from any data-blob attribute
    fn extract_from_data_blob(&self, html: &str, format: &str) -> Result<Option<String>> {
        // Look for data-blob containing digital_items or downloads
        let patterns = ["data-blob=\"", "data-blob='"];

        for pattern in patterns {
            let quote_char = if pattern.ends_with('"') { '"' } else { '\'' };
            let mut search_pos = 0;

            while let Some(blob_start) = html[search_pos..].find(pattern) {
                let actual_start = search_pos + blob_start + pattern.len();
                if let Some(blob_end) = html[actual_start..].find(quote_char) {
                    let blob = &html[actual_start..actual_start + blob_end];

                    // Check if this blob contains download info
                    if blob.contains("digital_items") || blob.contains("downloads") {
                        let unescaped = self.unescape_html(blob);
                        debug!(
                            "Found data-blob with download info ({} chars)",
                            unescaped.len()
                        );

                        if let Some(url) = self.extract_url_from_json(&unescaped, format) {
                            return Ok(Some(url));
                        }
                    }
                    search_pos = actual_start + blob_end;
                } else {
                    break;
                }
            }
        }

        Ok(None)
    }

    /// Extract download URL from TralbumData in script tags
    fn extract_from_tralbum_data(&self, html: &str, format: &str) -> Result<Option<String>> {
        // Look for TralbumData = { ... } or var TralbumData = { ... }
        let patterns = ["TralbumData = {", "TralbumData={"];

        for pattern in patterns {
            if let Some(start) = html.find(pattern) {
                let json_start = start + pattern.len() - 1; // Include the opening brace

                // Find matching closing brace (simplified - may need improvement for nested objects)
                if let Some(json_str) = self.extract_json_object(&html[json_start..]) {
                    debug!("Found TralbumData ({} chars)", json_str.len());

                    if let Some(url) = self.extract_url_from_json(&json_str, format) {
                        return Ok(Some(url));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Extract download URL using direct pattern matching
    fn extract_direct_pattern(&self, html: &str, format: &str) -> Result<Option<String>> {
        // Look for "format": { ... "url": "..." } pattern
        let search_pattern = format!("\"{format}\":");

        if let Some(format_pos) = html.find(&search_pattern) {
            debug!("Found format pattern '{search_pattern}' at position {format_pos}",);

            // Look for "url" field after this
            let search_area = &html[format_pos..html.len().min(format_pos + 1000)];

            for url_pattern in ["\"url\":\"", "\"url\": \"", "url\":\""] {
                if let Some(url_start) = search_area.find(url_pattern) {
                    let actual_start = url_start + url_pattern.len();
                    if let Some(url_end) = search_area[actual_start..].find('"') {
                        let url = &search_area[actual_start..actual_start + url_end];
                        let url = self.unescape_html(url);
                        debug!("Found URL via direct pattern: {url}");
                        return Ok(Some(url));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Extract URL from JSON string containing digital_items or downloads
    fn extract_url_from_json(&self, json_str: &str, format: &str) -> Option<String> {
        // Try parsing as full JSON first
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
            // Path 1: digital_items[0].downloads.{format}.url
            if let Some(url) = value
                .get("digital_items")
                .and_then(|items| items.get(0))
                .and_then(|item| item.get("downloads"))
                .and_then(|downloads| downloads.get(format))
                .and_then(|fmt| fmt.get("url"))
                .and_then(|url| url.as_str())
            {
                debug!("Found URL in digital_items[0].downloads.{format}.url");
                return Some(url.replace("\\u0026", "&"));
            }

            // Path 2: download_items[0].downloads.{format}.url
            if let Some(url) = value
                .get("download_items")
                .and_then(|items| items.get(0))
                .and_then(|item| item.get("downloads"))
                .and_then(|downloads| downloads.get(format))
                .and_then(|fmt| fmt.get("url"))
                .and_then(|url| url.as_str())
            {
                debug!("Found URL in download_items[0].downloads.{format}.url");
                return Some(url.replace("\\u0026", "&"));
            }

            // Path 3: downloads.{format}.url (direct)
            if let Some(url) = value
                .get("downloads")
                .and_then(|downloads| downloads.get(format))
                .and_then(|fmt| fmt.get("url"))
                .and_then(|url| url.as_str())
            {
                debug!("Found URL in downloads.{format}.url");
                return Some(url.replace("\\u0026", "&"));
            }

            // Log available formats for debugging
            if let Some(downloads) = value
                .get("digital_items")
                .and_then(|items| items.get(0))
                .and_then(|item| item.get("downloads"))
                .or_else(|| {
                    value
                        .get("download_items")
                        .and_then(|items| items.get(0))
                        .and_then(|item| item.get("downloads"))
                })
                .or_else(|| value.get("downloads"))
                && let Some(obj) = downloads.as_object()
            {
                let formats: Vec<_> = obj.keys().collect();
                debug!("Available formats in JSON: {formats:?}");
            }
        }

        // Fallback: simple pattern matching for the URL
        let format_pattern = format!("\"{format}\":");
        if let Some(pos) = json_str.find(&format_pattern) {
            let after = &json_str[pos..];
            if let Some(url_pos) = after.find("\"url\":") {
                let url_start = url_pos + 6;
                // Skip whitespace and opening quote
                let url_content_start = after[url_start..].find('"').map(|p| url_start + p + 1)?;
                let url_end = after[url_content_start..].find('"')?;
                let url = &after[url_content_start..url_content_start + url_end];
                debug!("Found URL via pattern matching in JSON string");
                return Some(self.unescape_html(url));
            }
        }

        None
    }

    /// Extract a JSON object from a string starting with '{'
    fn extract_json_object(&self, s: &str) -> Option<String> {
        if !s.starts_with('{') {
            return None;
        }

        let mut depth = 0;
        let mut in_string = false;
        let mut escape_next = false;

        for (i, c) in s.char_indices() {
            if escape_next {
                escape_next = false;
                continue;
            }

            match c {
                '\\' if in_string => escape_next = true,
                '"' => in_string = !in_string,
                '{' if !in_string => depth += 1,
                '}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(s[..=i].to_string());
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Unescape HTML entities in a string
    fn unescape_html(&self, s: &str) -> String {
        s.replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#39;", "'")
            .replace("&apos;", "'")
            .replace("&#x27;", "'")
            .replace("&#x2F;", "/")
            .replace("\\u0026", "&")
            .replace("\\/", "/")
    }
}

impl Default for BandcampClient {
    fn default() -> Self {
        Self::new()
    }
}
