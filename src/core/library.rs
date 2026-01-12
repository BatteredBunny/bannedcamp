use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryItem {
    pub id: String,
    pub item_type: ItemType,
    pub title: String,
    pub artist: String,
    pub artist_id: String,

    /// Artist subdomain (e.g., "badmathhk" from badmathhk.bandcamp.com)
    pub artist_subdomain: Option<String>,

    /// URL slug for the item (e.g., "missing-narrative")
    pub slug: Option<String>,

    /// Full item URL (e.g., "https://badmathhk.bandcamp.com/album/missing-narrative")
    pub item_url: Option<String>,
    pub purchase_date: DateTime<Utc>,
    pub artwork_url: Option<String>,
    pub download_url: String,
    pub available_formats: Vec<AudioFormat>,
    pub is_preorder: bool,
    pub is_hidden: bool,
}

/// Custom name format for downloads
/// {artist}, {title}, {id}, {ext}
const DEFAULT_ALBUM_FORMAT: &str = "{artist} - {title}";
const DEFAULT_TRACK_FORMAT: &str = "{artist} - {title}{ext}";

impl LibraryItem {
    /// Constructs the folder or filename it will be downloaded as
    pub fn construct_filename(&self, format: AudioFormat, custom_format: Option<&str>) -> String {
        let extension_str = if self.item_type == ItemType::Track {
            format!(".{}", format.extension())
        } else {
            String::new()
        };

        let name_format = if let Some(fmt) = custom_format {
            fmt
        } else if self.item_type == ItemType::Track {
            DEFAULT_TRACK_FORMAT
        } else {
            DEFAULT_ALBUM_FORMAT
        };

        name_format
            .replace("{artist}", &self.artist)
            .replace("{title}", &self.title)
            .replace("{ext}", &extension_str)
            .replace("{id}", &self.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ItemType {
    Album,
    Track,
    Package,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
pub enum AudioFormat {
    Flac,
    #[value(name = "mp3-v0")]
    Mp3V0,
    #[value(name = "mp3-320")]
    Mp3320,
    Aac,
    #[value(name = "ogg")]
    OggVorbis,
    Alac,
    Wav,
    Aiff,
}

impl AudioFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Flac => "flac",
            AudioFormat::Mp3V0 | AudioFormat::Mp3320 => "mp3",
            AudioFormat::Aac => "m4a",
            AudioFormat::OggVorbis => "ogg",
            AudioFormat::Alac => "m4a",
            AudioFormat::Wav => "wav",
            AudioFormat::Aiff => "aiff",
        }
    }

    pub fn bandcamp_encoding(&self) -> &'static str {
        match self {
            AudioFormat::Flac => "flac",
            AudioFormat::Mp3V0 => "mp3-v0",
            AudioFormat::Mp3320 => "mp3-320",
            AudioFormat::Aac => "aac-hi",
            AudioFormat::OggVorbis => "vorbis",
            AudioFormat::Alac => "alac",
            AudioFormat::Wav => "wav",
            AudioFormat::Aiff => "aiff-lossless",
        }
    }
}
