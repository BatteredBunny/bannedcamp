use reqwest::header::{COOKIE, HeaderMap, HeaderValue};
use serde::Deserialize;
use tracing::{debug, info};

use crate::core::auth::Credentials;
use crate::core::library::{AudioFormat, ItemType, LibraryItem};
use crate::error::{BandcampError, Result};

const BANDCAMP_BASE: &str = "https://bandcamp.com";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

/// Response from the collection_items API endpoint
#[derive(Debug, Deserialize)]
struct CollectionResponse {
    items: Vec<CollectionItem>,
    more_available: bool,
    last_token: Option<String>,
    /// Map of "{sale_item_type}{sale_item_id}" -> redownload URL
    #[serde(default)]
    redownload_urls: std::collections::HashMap<String, String>,
}

/// URL hints from collection item
#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct UrlHints {
    subdomain: Option<String>,
    #[serde(default)]
    custom_domain: Option<String>,
    slug: Option<String>,
    item_type: Option<String>,
}

/// Individual item from collection response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CollectionItem {
    sale_item_id: u64,
    sale_item_type: String,
    band_name: String,
    item_title: String,
    item_id: u64,
    band_id: u64,
    #[serde(default)]
    item_art_id: Option<u64>,
    #[serde(default)]
    is_preorder: bool,
    #[serde(default)]
    hidden: Option<bool>,
    token: String,
    tralbum_type: String,
    #[serde(default)]
    url_hints: Option<UrlHints>,
    #[serde(default)]
    item_url: Option<String>,
}

/// Page data embedded in HTML containing fan_id and other info
#[derive(Debug, Deserialize)]
struct PageData {
    fan_data: FanData,
}

#[derive(Debug, Deserialize)]
struct FanData {
    fan_id: u64,
}

/// Download page data (for future use)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DownloadPageData {
    download_items: Vec<DownloadItem>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DownloadItem {
    downloads: std::collections::HashMap<String, DownloadFormat>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DownloadFormat {
    url: String,
}

pub struct BandcampClient {
    http: reqwest::Client,
    credentials: Option<Credentials>,
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

    /// Validate a session cookie by attempting to fetch the user's fan ID
    pub async fn validate_cookie(&mut self, identity_cookie: &str) -> Result<Credentials> {
        info!("Validating session cookie...");

        let mut headers = HeaderMap::new();
        let cookie_value = format!("identity={identity_cookie}");
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&cookie_value)
                .map_err(|e| BandcampError::AuthError(e.to_string()))?,
        );

        // Try fetching the fan page / collection summary which should have fan_id
        let response = self
            .http
            .get(format!("{BANDCAMP_BASE}/api/fan/2/collection_summary"))
            .headers(headers.clone())
            .send()
            .await?;

        if response.status().is_success() {
            // Try to extract fan_id from JSON API response
            let text = response.text().await?;
            if let Some(fan_id) = self.extract_fan_id_from_json(&text) {
                info!("Cookie validated via API, fan_id: {fan_id}");
                let mut credentials = Credentials::new(identity_cookie.to_string());
                credentials.fan_id = Some(fan_id.to_string());
                self.credentials = Some(credentials.clone());
                return Ok(credentials);
            }
        }

        // Fallback: try the settings page
        let response = self
            .http
            .get(format!("{BANDCAMP_BASE}/settings"))
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(BandcampError::InvalidCredentials);
        }

        let html = response.text().await?;

        // Extract fan_id from the page data
        let fan_id = self.extract_fan_id(&html)?;

        info!("Cookie validated, fan_id: {fan_id}");

        let mut credentials = Credentials::new(identity_cookie.to_string());
        credentials.fan_id = Some(fan_id.to_string());

        self.credentials = Some(credentials.clone());
        Ok(credentials)
    }

    /// Extract fan_id from JSON response
    fn extract_fan_id_from_json(&self, json: &str) -> Option<u64> {
        // Look for "fan_id":NUMBER pattern
        if let Some(pos) = json.find("\"fan_id\":") {
            let start = pos + 9;
            let end = json[start..]
                .find(|c: char| !c.is_ascii_digit())
                .map(|p| start + p)
                .unwrap_or(start);
            if end > start
                && let Ok(fan_id) = json[start..end].parse::<u64>()
            {
                return Some(fan_id);
            }
        }
        None
    }

    /// Extract fan_id from HTML page data
    fn extract_fan_id(&self, html: &str) -> Result<u64> {
        // Try multiple patterns to find fan_id

        // Pattern 1: Look for "fan_id":NUMBER directly in HTML
        if let Some(pos) = html.find("\"fan_id\":") {
            let start = pos + 9; // len of "\"fan_id\":"
            let end = html[start..]
                .find(|c: char| !c.is_ascii_digit())
                .map(|p| start + p)
                .unwrap_or(start);
            if end > start
                && let Ok(fan_id) = html[start..end].parse::<u64>()
            {
                debug!("Found fan_id via direct pattern: {fan_id}");
                return Ok(fan_id);
            }
        }

        // Pattern 2: Look for data-blob attribute
        if let Some(data_blob_start) = html
            .find("data-blob=\"")
            .or_else(|| html.find("data-blob='"))
        {
            let quote_char = if html[data_blob_start..].starts_with("data-blob=\"") {
                '"'
            } else {
                '\''
            };

            let start = data_blob_start + 11; // len of "data-blob=\""
            if let Some(end_offset) = html[start..].find(quote_char) {
                let end = start + end_offset;
                let data_blob = &html[start..end];

                // Unescape HTML entities
                let unescaped = data_blob
                    .replace("&quot;", "\"")
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&#39;", "'");

                // Try to parse as PageData
                if let Ok(page_data) = serde_json::from_str::<PageData>(&unescaped) {
                    debug!(
                        "Found fan_id via data-blob PageData: {}",
                        page_data.fan_data.fan_id
                    );
                    return Ok(page_data.fan_data.fan_id);
                }

                // Try to extract fan_id from the unescaped JSON directly
                if let Some(pos) = unescaped.find("\"fan_id\":") {
                    let id_start = pos + 9;
                    let id_end = unescaped[id_start..]
                        .find(|c: char| !c.is_ascii_digit())
                        .map(|p| id_start + p)
                        .unwrap_or(id_start);
                    if id_end > id_start
                        && let Ok(fan_id) = unescaped[id_start..id_end].parse::<u64>()
                    {
                        debug!("Found fan_id via data-blob direct: {}", fan_id);
                        return Ok(fan_id);
                    }
                }
            }
        }

        Err(BandcampError::ParseError(
            "Could not find fan_id in page".into(),
        ))
    }

    /// Fetch the user's library collection
    pub async fn get_collection(&self) -> Result<Vec<LibraryItem>> {
        let creds = self
            .credentials
            .as_ref()
            .ok_or(BandcampError::NotLoggedIn)?;

        let fan_id = creds
            .fan_id
            .as_ref()
            .ok_or_else(|| BandcampError::AuthError("No fan_id in credentials".into()))?;

        info!("Fetching collection for fan_id: {fan_id}");

        let mut all_items = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        // Use a far-future timestamp for the first request (this gets items before this date)
        let mut older_than_token: String =
            format!("{}::a::", chrono::Utc::now().timestamp() + 86400);

        loop {
            let url = format!("{BANDCAMP_BASE}/api/fancollection/1/collection_items");

            let body = serde_json::json!({
                "fan_id": fan_id.parse::<u64>().unwrap_or(0),
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
                return Err(BandcampError::SessionExpired);
            }

            if !response.status().is_success() {
                return Err(BandcampError::NetworkError(
                    response.error_for_status().unwrap_err(),
                ));
            }

            let text = response.text().await?;
            debug!("Collection API response: {}", &text[..text.len().min(2000)]);

            let collection: CollectionResponse = serde_json::from_str(&text).map_err(|e| {
                BandcampError::ParseError(format!(
                    "Failed to parse collection response: {e} - Response: {}",
                    &text[..text.len().min(500)]
                ))
            })?;

            debug!(
                "Fetched {} items, more_available: {}",
                collection.items.len(),
                collection.more_available
            );

            let mut duplicates_skipped = 0;
            for item in collection.items {
                // Deduplicate by sale_item_id to prevent pagination overlap issues
                if seen_ids.insert(item.sale_item_id) {
                    all_items.push(self.convert_collection_item(item, &collection.redownload_urls));
                } else {
                    duplicates_skipped += 1;
                }
            }

            if duplicates_skipped > 0 {
                debug!("Skipped {duplicates_skipped} duplicate items in this page",);
            }

            if !collection.more_available {
                break;
            }

            if let Some(token) = collection.last_token {
                older_than_token = token;
            } else {
                break;
            }
        }

        info!("Fetched {} total items from collection", all_items.len());
        Ok(all_items)
    }

    /// Convert API collection item to our LibraryItem type
    fn convert_collection_item(
        &self,
        item: CollectionItem,
        redownload_urls: &std::collections::HashMap<String, String>,
    ) -> LibraryItem {
        let item_type = match item.tralbum_type.as_str() {
            "a" => ItemType::Album,
            "t" => ItemType::Track,
            "p" => ItemType::Package,
            _ => ItemType::Album, // Default to album
        };

        let artwork_url = item
            .item_art_id
            .map(|id| format!("https://f4.bcbits.com/img/a{}_10.jpg", id));

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
            purchase_date: chrono::Utc::now(), // API doesn't return this, would need to parse token
            artwork_url,
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

        // Debug: log what we found in the page
        self.log_page_debug_info(html);

        Err(BandcampError::ParseError(format!(
            "Could not find download URL for format '{}' in page",
            format_str
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
        let search_pattern = format!("\"{}\":", format);

        if let Some(format_pos) = html.find(&search_pattern) {
            debug!(
                "Found format pattern '{}' at position {}",
                search_pattern, format_pos
            );

            // Look for "url" field after this
            let search_area = &html[format_pos..html.len().min(format_pos + 1000)];

            for url_pattern in ["\"url\":\"", "\"url\": \"", "url\":\""] {
                if let Some(url_start) = search_area.find(url_pattern) {
                    let actual_start = url_start + url_pattern.len();
                    if let Some(url_end) = search_area[actual_start..].find('"') {
                        let url = &search_area[actual_start..actual_start + url_end];
                        let url = url.replace("\\u0026", "&").replace("\\/", "/");
                        debug!("Found URL via direct pattern: {}", url);
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
                return Some(url.replace("\\u0026", "&").replace("\\/", "/"));
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

    /// Log debug information about the page for troubleshooting
    fn log_page_debug_info(&self, html: &str) {
        // Log page title
        if let Some(title_pos) = html.find("<title>")
            && let Some(title_end) = html[title_pos..].find("</title>")
        {
            debug!(
                "Page title: {}",
                &html[title_pos + 7..title_pos + title_end]
            );
        }

        // Check for common indicators
        let indicators = [
            ("pagedata", html.contains("pagedata")),
            ("data-blob", html.contains("data-blob")),
            ("digital_items", html.contains("digital_items")),
            ("download_items", html.contains("download_items")),
            ("downloads", html.contains("\"downloads\"")),
            ("TralbumData", html.contains("TralbumData")),
        ];

        for (name, found) in indicators {
            if found {
                debug!("Page contains: {name}");
            }
        }

        // Check for error messages
        if (html.contains("error") || html.contains("Error"))
            && let Some(pos) = html.to_lowercase().find("error")
        {
            let snippet = &html[pos.saturating_sub(50)..html.len().min(pos + 200)];
            debug!("Possible error in page: {}", snippet);
        }

        // List available formats if downloads section found
        for fmt in [
            "flac",
            "mp3-320",
            "mp3-v0",
            "aac-hi",
            "vorbis",
            "alac",
            "wav",
            "aiff-lossless",
        ] {
            if html.contains(&format!("\"{}\"", fmt)) {
                debug!("Format '{}' found in page", fmt);
            }
        }
    }
}

impl Default for BandcampClient {
    fn default() -> Self {
        Self::new()
    }
}
