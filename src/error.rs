use thiserror::Error;

#[derive(Error, Debug)]
pub enum BandcampError {
    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Not logged in")]
    NotLoggedIn,

    #[error("Session expired")]
    SessionExpired,

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Download failed: {0}")]
    DownloadError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    ParseError(String),
}

pub type Result<T> = std::result::Result<T, BandcampError>;
