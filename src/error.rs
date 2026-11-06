#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: String,
    pub message: String,
    pub retry_after: std::time::Duration,
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retry_after: std::time::Duration::ZERO,
        }
    }
    pub fn integrity(&self) -> bool {
        matches!(
            self.code.as_str(),
            "remote_changed"
                | "bad_content_range"
                | "hash_mismatch"
                | "seam_mismatch"
                | "not_same_file"
                | "part_too_large"
        )
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        let code = if matches!(e.raw_os_error(), Some(112 | 39)) {
            "no_space"
        } else {
            "other"
        };
        Self::new(code, e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("other", e.to_string())
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("database", e.to_string())
    }
}
impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::new("network", e.without_url().to_string())
    }
}
