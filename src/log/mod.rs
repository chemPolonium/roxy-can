pub mod asc;
pub mod error;
pub mod vec_stream;

use std::path::Path;

use crate::source::FrameStream;
use asc::{AscStream, parse_asc};
use error::LogError;
use vec_stream::VecStream;

pub use asc::AscWriter;

/// Log containers roxy-can can distinguish by extension. BLF ships in 0.3.0;
/// MF4 is recognised but not yet parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Asc,
    Blf,
    Mf4,
}

impl LogFormat {
    pub fn detect(path: &Path) -> Option<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())?;
        match ext.as_str() {
            "asc" => Some(LogFormat::Asc),
            "blf" => Some(LogFormat::Blf),
            "mf4" => Some(LogFormat::Mf4),
            _ => None,
        }
    }
}

/// Below this size we keep the classic `read_to_string → parse_asc` path:
/// a single syscall is cheaper than an mmap, and the existing tests already
/// exercise that branch. Above it, [`AscStream`] pages the file lazily.
const ASC_MMAP_THRESHOLD: u64 = 100 * 1024 * 1024;

/// Opens a log file and returns a stream over its frames. Dispatch is by
/// extension so adding MF4 later means one match arm, nothing at call sites.
pub fn open_stream(path: &Path) -> Result<Box<dyn FrameStream>, LogError> {
    match LogFormat::detect(path) {
        Some(LogFormat::Blf) => blf_arm(path),
        Some(LogFormat::Mf4) => Err(LogError::UnsupportedFormat("MF4")),
        Some(LogFormat::Asc) => {
            let size = std::fs::metadata(path)?.len();
            if size < ASC_MMAP_THRESHOLD {
                let s = std::fs::read_to_string(path)?;
                Ok(Box::new(VecStream::new(parse_asc(&s))))
            } else {
                Ok(Box::new(AscStream::open(path)?))
            }
        }
        None => Err(LogError::UnsupportedFormat("unknown extension")),
    }
}

/// Placeholder until `src/log/blf.rs` lands in the same release cycle.
fn blf_arm(_path: &Path) -> Result<Box<dyn FrameStream>, LogError> {
    Err(LogError::UnsupportedFormat("BLF"))
}
