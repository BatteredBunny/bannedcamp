use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::core::auth::Credentials;
use crate::core::library::{AudioFormat, LibraryItem};
use crate::tui::async_bridge::{AsyncRequest, AsyncResponse};
use crate::tui::widgets::spinner::Spinner;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Login,
    Library,
    Download,
}

/// Login screen state
pub struct LoginState {
    pub cookie_input: String,
    pub cursor_position: usize,
    pub loading: bool,
    pub spinner: Spinner,
    pub error: Option<String>,
}

impl Default for LoginState {
    fn default() -> Self {
        // Pre-fill from BANDCAMP_COOKIE env var if available
        let cookie_input = std::env::var("BANDCAMP_COOKIE").unwrap_or_default();
        let cursor_position = cookie_input.len();
        Self {
            cookie_input,
            cursor_position,
            loading: false,
            spinner: Spinner::default(),
            error: None,
        }
    }
}

/// Library screen mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryMode {
    #[default]
    Browse,
    FormatSelection,
}

/// Which element has focus in the library screen
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryFocus {
    #[default]
    List,
    SearchBar,
}

/// Library browser state
pub struct LibraryState {
    pub items: Vec<LibraryItem>,
    /// Indices of items matching current search (empty = show all)
    pub filtered_indices: Vec<usize>,
    /// Current position in filtered list
    pub selected: usize,
    pub scroll_offset: usize,
    pub loading: bool,
    pub spinner: Spinner,
    pub mode: LibraryMode,
    /// Which element currently has keyboard focus
    pub focus: LibraryFocus,
    /// IDs of selected items for download
    pub selected_items: HashSet<String>,
    /// Selected format in format selection menu
    pub selected_format: usize,
    /// Current search query
    pub search_query: String,
    /// Error message to display
    pub error: Option<String>,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            filtered_indices: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            loading: false,
            spinner: Spinner::new(),
            mode: LibraryMode::Browse,
            focus: LibraryFocus::List,
            selected_items: HashSet::new(),
            selected_format: 0, // FLAC by default
            search_query: String::new(),
            error: None,
        }
    }
}

impl LibraryState {
    /// Get the currently visible items (filtered or all)
    pub fn visible_items(&self) -> Vec<(usize, &LibraryItem)> {
        if self.search_query.is_empty() {
            self.items.iter().enumerate().collect()
        } else {
            self.filtered_indices
                .iter()
                .filter_map(|&i| self.items.get(i).map(|item| (i, item)))
                .collect()
        }
    }

    /// Get the number of visible items
    pub fn visible_count(&self) -> usize {
        if self.search_query.is_empty() {
            self.items.len()
        } else {
            self.filtered_indices.len()
        }
    }

    /// Get the item at the current selection
    pub fn selected_item(&self) -> Option<&LibraryItem> {
        let visible = self.visible_items();
        visible.get(self.selected).map(|(_, item)| *item)
    }

    /// Update filtered indices based on search query
    pub fn update_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_indices.clear();
        } else {
            let query = self.search_query.to_lowercase();
            self.filtered_indices = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    item.artist.to_lowercase().contains(&query)
                        || item.title.to_lowercase().contains(&query)
                })
                .map(|(i, _)| i)
                .collect();
        }
        // Reset selection if out of bounds
        let count = self.visible_count();
        if count == 0 {
            self.selected = 0;
            self.scroll_offset = 0;
        } else if self.selected >= count {
            self.selected = count.saturating_sub(1);
            self.scroll_offset = self.scroll_offset.min(self.selected);
        }
    }
}

impl AudioFormat {
    pub const ALL: [AudioFormat; 8] = [
        AudioFormat::Flac,
        AudioFormat::Mp3320,
        AudioFormat::Mp3V0,
        AudioFormat::Aac,
        AudioFormat::OggVorbis,
        AudioFormat::Alac,
        AudioFormat::Wav,
        AudioFormat::Aiff,
    ];

    pub fn display_name(&self) -> &'static str {
        match self {
            AudioFormat::Flac => "FLAC (lossless)",
            AudioFormat::Mp3320 => "MP3 320kbps",
            AudioFormat::Mp3V0 => "MP3 V0 (variable bitrate)",
            AudioFormat::Aac => "AAC (high quality)",
            AudioFormat::OggVorbis => "Ogg Vorbis",
            AudioFormat::Alac => "ALAC (Apple lossless)",
            AudioFormat::Wav => "WAV (uncompressed)",
            AudioFormat::Aiff => "AIFF (uncompressed)",
        }
    }
}

/// Individual item download result
#[derive(Debug, Clone)]
pub struct ItemDownloadResult {
    pub item_id: String,
    pub result: Result<PathBuf, String>,
}

/// Progress state for a single concurrent download slot
#[derive(Debug, Clone, Default)]
pub struct DownloadSlot {
    /// Item being downloaded in this slot
    pub item: Option<LibraryItem>,
    /// Item ID (for matching progress updates)
    pub item_id: Option<String>,
    /// Bytes downloaded
    pub downloaded: u64,
    /// Total bytes (if known)
    pub total: Option<u64>,
    /// Download speed in bytes/sec
    pub speed_bytes_per_sec: f64,
    /// Last progress update time
    pub last_update: Option<Instant>,
    /// Last bytes for speed calculation
    pub last_bytes: u64,
}

impl DownloadSlot {
    pub fn progress_percent(&self) -> f64 {
        match self.total {
            Some(total) if total > 0 => (self.downloaded as f64 / total as f64) * 100.0,
            _ => 0.0,
        }
    }

    pub fn update_speed(&mut self) {
        let now = Instant::now();
        if let Some(last_time) = self.last_update {
            let elapsed = now.duration_since(last_time).as_secs_f64();
            if elapsed >= 0.3 {
                let bytes_diff = self.downloaded.saturating_sub(self.last_bytes);
                self.speed_bytes_per_sec = bytes_diff as f64 / elapsed;
                self.last_bytes = self.downloaded;
                self.last_update = Some(now);
            }
        } else {
            self.last_update = Some(now);
            self.last_bytes = self.downloaded;
        }
    }
}

/// Maximum concurrent downloads
pub const MAX_CONCURRENT_DOWNLOADS: usize = 3;

/// Download progress state for batch downloads
#[derive(Default)]
pub struct DownloadState {
    /// Items queued for download
    pub items: Vec<LibraryItem>,
    /// Total number of items
    pub total_items: usize,
    /// Number of items started (queued to slots)
    pub started_count: usize,
    /// Whether download is active
    pub is_active: bool,
    /// Results for completed items
    pub results: Vec<ItemDownloadResult>,
    /// When the download started
    pub start_time: Option<Instant>,
    /// Concurrent download slots
    pub slots: [DownloadSlot; MAX_CONCURRENT_DOWNLOADS],
}

impl DownloadState {
    pub fn success_count(&self) -> usize {
        self.results.iter().filter(|r| r.result.is_ok()).count()
    }

    pub fn failure_count(&self) -> usize {
        self.results.iter().filter(|r| r.result.is_err()).count()
    }

    /// Get total download speed across all slots
    pub fn total_speed(&self) -> f64 {
        self.slots.iter().map(|s| s.speed_bytes_per_sec).sum()
    }

    /// Get total bytes downloaded across all active slots
    pub fn total_downloaded(&self) -> u64 {
        self.slots.iter().map(|s| s.downloaded).sum()
    }

    /// Get number of active slots
    pub fn active_slot_count(&self) -> usize {
        self.slots.iter().filter(|s| s.item.is_some()).count()
    }

    /// Find slot by item_id
    pub fn find_slot_mut(&mut self, item_id: &str) -> Option<&mut DownloadSlot> {
        self.slots
            .iter_mut()
            .find(|s| s.item_id.as_deref() == Some(item_id))
    }

    /// Find first empty slot
    pub fn find_empty_slot(&mut self) -> Option<&mut DownloadSlot> {
        self.slots.iter_mut().find(|s| s.item.is_none())
    }

    /// Clear a slot by item_id
    pub fn clear_slot(&mut self, item_id: &str) {
        if let Some(slot) = self.find_slot_mut(item_id) {
            *slot = DownloadSlot::default();
        }
    }
}

pub struct App {
    pub screen: Screen,
    pub should_quit: bool,

    // Authentication
    pub credentials: Option<Credentials>,

    // Screen states
    pub login_state: LoginState,
    pub library_state: LibraryState,
    pub download_state: DownloadState,

    // Async communication
    pub async_tx: mpsc::Sender<AsyncRequest>,

    // Download settings
    pub output_dir: PathBuf,
}

impl App {
    pub fn new(async_tx: mpsc::Sender<AsyncRequest>) -> Self {
        Self {
            screen: Screen::Login,
            should_quit: false,
            credentials: None,
            login_state: LoginState::default(),
            library_state: LibraryState::default(),
            download_state: DownloadState::default(),
            async_tx,
            output_dir: PathBuf::from("."),
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    // Spinner progressing
    pub fn tick(&mut self) {
        if self.login_state.loading {
            self.login_state.spinner.tick();
        }
        if self.library_state.loading {
            self.library_state.spinner.tick();
        }
    }

    /// Handle async response from the bridge
    pub fn handle_async_response(&mut self, response: AsyncResponse) {
        match response {
            AsyncResponse::CookieValidated(result) => {
                self.login_state.loading = false;
                match result {
                    Ok(creds) => {
                        self.credentials = Some(creds);
                        self.login_state.error = None;
                        self.screen = Screen::Library;
                        self.library_state.loading = true;
                        let _ = self.async_tx.try_send(AsyncRequest::FetchCollection);
                    }
                    Err(e) => {
                        self.login_state.error = Some(e);
                    }
                }
            }
            AsyncResponse::CollectionFetched(result) => {
                self.library_state.loading = false;
                match result {
                    Ok(items) => {
                        self.library_state.items = items;
                        self.library_state.selected = 0;
                        self.library_state.scroll_offset = 0;
                        self.library_state.error = None;
                    }
                    Err(e) => {
                        self.library_state.error = Some(e);
                    }
                }
            }
            AsyncResponse::BatchDownloadStarted { total_items } => {
                self.download_state.total_items = total_items;
                self.download_state.is_active = true;
                self.download_state.start_time = Some(Instant::now());
                self.download_state.started_count = 0;
                // Clear all slots
                for slot in &mut self.download_state.slots {
                    *slot = DownloadSlot::default();
                }
            }
            AsyncResponse::ItemDownloadStarted {
                item_id,
                item_index: _,
            } => {
                self.download_state.started_count += 1;
                // Find the item first (before mutable borrow)
                let item = self
                    .download_state
                    .items
                    .iter()
                    .find(|i| i.id == item_id)
                    .cloned();
                // Find an empty slot and assign this item
                if let Some(slot) = self.download_state.find_empty_slot() {
                    slot.item = item;
                    slot.item_id = Some(item_id);
                    slot.downloaded = 0;
                    slot.total = None;
                    slot.speed_bytes_per_sec = 0.0;
                    slot.last_update = None;
                    slot.last_bytes = 0;
                }
            }
            AsyncResponse::DownloadProgress {
                item_id,
                downloaded,
                total,
            } => {
                if let Some(slot) = self.download_state.find_slot_mut(&item_id) {
                    slot.downloaded = downloaded;
                    slot.total = total;
                    slot.update_speed();
                }
            }
            AsyncResponse::ItemDownloadComplete {
                item_id,
                item_index: _,
                result,
            } => {
                // Clear the slot
                self.download_state.clear_slot(&item_id);
                // Record the result
                self.download_state
                    .results
                    .push(ItemDownloadResult { item_id, result });
            }
            AsyncResponse::BatchDownloadComplete => {
                self.download_state.is_active = false;
                // Clear all slots
                for slot in &mut self.download_state.slots {
                    *slot = DownloadSlot::default();
                }
            }
        }
    }

    // Login screen actions
    pub fn login_input_char(&mut self, c: char) {
        self.login_state
            .cookie_input
            .insert(self.login_state.cursor_position, c);
        self.login_state.cursor_position += 1;
    }

    pub fn login_delete_char(&mut self) {
        if self.login_state.cursor_position > 0 {
            self.login_state.cursor_position -= 1;
            self.login_state
                .cookie_input
                .remove(self.login_state.cursor_position);
        }
    }

    pub fn login_submit(&mut self) {
        if self.login_state.cookie_input.is_empty() {
            self.login_state.error = Some("Please enter a cookie".to_string());
            return;
        }
        self.login_state.loading = true;
        self.login_state.error = None;
        let cookie = self.login_state.cookie_input.clone();
        let _ = self.async_tx.try_send(AsyncRequest::ValidateCookie(cookie));
    }

    // Library screen actions - Browse mode
    pub fn library_move_up(&mut self) {
        if self.library_state.selected > 0 {
            self.library_state.selected -= 1;
            if self.library_state.selected < self.library_state.scroll_offset {
                self.library_state.scroll_offset = self.library_state.selected;
            }
        }
    }

    pub fn library_move_down(&mut self) {
        let max = self.library_state.visible_count().saturating_sub(1);
        if self.library_state.selected < max {
            self.library_state.selected += 1;
        }
    }

    /// Toggle selection of currently highlighted item
    pub fn library_toggle_selection(&mut self) {
        if let Some(item) = self.library_state.selected_item() {
            let id = item.id.clone();
            if self.library_state.selected_items.contains(&id) {
                self.library_state.selected_items.remove(&id);
            } else {
                self.library_state.selected_items.insert(id);
            }
        }
    }

    /// Select all visible items (respects current filter)
    pub fn library_select_all(&mut self) {
        let ids: Vec<String> = self
            .library_state
            .visible_items()
            .iter()
            .map(|(_, item)| item.id.clone())
            .collect();
        self.library_state.selected_items.extend(ids);
    }

    // Focus actions
    pub fn library_focus_search(&mut self) {
        self.library_state.focus = LibraryFocus::SearchBar;
    }

    pub fn library_focus_list(&mut self) {
        self.library_state.focus = LibraryFocus::List;
    }

    pub fn library_toggle_focus(&mut self) {
        self.library_state.focus = match self.library_state.focus {
            LibraryFocus::List => LibraryFocus::SearchBar,
            LibraryFocus::SearchBar => LibraryFocus::List,
        };
    }

    // Search actions
    pub fn library_search_input(&mut self, c: char) {
        self.library_state.search_query.push(c);
        self.library_state.update_filter();
    }

    pub fn library_search_backspace(&mut self) {
        self.library_state.search_query.pop();
        self.library_state.update_filter();
    }

    pub fn library_search_clear(&mut self) {
        self.library_state.search_query.clear();
        self.library_state.update_filter();
        self.library_state.focus = LibraryFocus::List;
    }

    /// Clear all selections
    pub fn library_clear_selection(&mut self) {
        self.library_state.selected_items.clear();
    }

    /// Show format selection dialog (called when user presses 'd' to download)
    pub fn library_show_format_selection(&mut self) {
        if self.library_state.selected_items.is_empty() {
            return;
        }
        self.library_state.mode = LibraryMode::FormatSelection;
    }

    /// Cancel format selection and go back to browse
    pub fn library_cancel_format_selection(&mut self) {
        self.library_state.mode = LibraryMode::Browse;
    }

    // Library screen actions - Format selection mode
    pub fn format_move_up(&mut self) {
        if self.library_state.selected_format > 0 {
            self.library_state.selected_format -= 1;
        }
    }

    pub fn format_move_down(&mut self) {
        if self.library_state.selected_format < AudioFormat::ALL.len() - 1 {
            self.library_state.selected_format += 1;
        }
    }

    /// Confirm format selection and start download
    pub fn format_confirm(&mut self) {
        let format = AudioFormat::ALL[self.library_state.selected_format];

        let items: Vec<LibraryItem> = self
            .library_state
            .items
            .iter()
            .filter(|i| self.library_state.selected_items.contains(&i.id))
            .cloned()
            .collect();

        if items.is_empty() {
            self.library_state.mode = LibraryMode::Browse;
            return;
        }

        // Setup download state
        self.download_state = DownloadState {
            items: items.clone(),
            total_items: items.len(),
            ..Default::default()
        };

        self.screen = Screen::Download;
        self.library_state.mode = LibraryMode::Browse;

        // Start the download
        let _ = self.async_tx.try_send(AsyncRequest::StartBatchDownload {
            items,
            format,
            output_dir: self.output_dir.clone(),
        });
    }

    // Download screen actions
    pub fn download_back_to_library(&mut self) {
        if !self.download_state.is_active {
            // Clear selections after download
            self.library_state.selected_items.clear();
            self.screen = Screen::Library;
            self.download_state = DownloadState::default();
        }
    }
}
