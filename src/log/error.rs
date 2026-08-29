use std::fmt;

/// Failure modes shared by every log reader in `src/log`. The variants map
/// 1:1 onto the checks a reader performs before it can emit a frame, so the
/// status bar can say exactly why a file was rejected.
#[derive(Debug)]
pub enum LogError {
    Io(std::io::Error),
    /// Leading magic bytes did not match what the reader expected
    /// (e.g. a `.blf` file whose header is not `LOGG`).
    #[allow(dead_code)]
    BadSignature,
    /// File ended mid-record. Only reported for fixed-layout formats; the
    /// ASC reader treats a truncated tail as EOF.
    #[allow(dead_code)]
    Truncated,
    /// The extension resolved to a format whose reader is not built yet.
    UnsupportedFormat(&'static str),
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogError::Io(e) => write!(f, "io: {e}"),
            LogError::BadSignature => write!(f, "bad signature"),
            LogError::Truncated => write!(f, "truncated"),
            LogError::UnsupportedFormat(s) => write!(f, "unsupported format: {s}"),
        }
    }
}

impl std::error::Error for LogError {}

impl From<std::io::Error> for LogError {
    fn from(e: std::io::Error) -> Self {
        LogError::Io(e)
    }
}
