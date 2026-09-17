pub mod asc;
pub mod blf;
pub mod error;
pub mod vec_stream;

mod backing;

use std::path::Path;

use crate::source::FrameStream;
use asc::{AscStream, parse_asc};
use blf::BlfStream;
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
    open_stream_at(path, ASC_MMAP_THRESHOLD)
}

/// `open_stream` with the ASC mmap threshold supplied by the caller. Tests
/// pass a tiny threshold to reach [`AscStream`]; production always uses
/// [`ASC_MMAP_THRESHOLD`].
pub(crate) fn open_stream_at(
    path: &Path,
    asc_mmap_threshold: u64,
) -> Result<Box<dyn FrameStream>, LogError> {
    match LogFormat::detect(path) {
        Some(LogFormat::Blf) => Ok(Box::new(BlfStream::open(path)?)),
        Some(LogFormat::Mf4) => Err(LogError::UnsupportedFormat("MF4")),
        Some(LogFormat::Asc) => {
            let size = std::fs::metadata(path)?.len();
            if size < asc_mmap_threshold {
                let s = std::fs::read_to_string(path)?;
                Ok(Box::new(VecStream::new(parse_asc(&s))))
            } else {
                Ok(Box::new(AscStream::open(path)?))
            }
        }
        None => Err(LogError::UnsupportedFormat("unknown extension")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn temp_log(name: &str, body: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::File::create(&path)
            .unwrap()
            .write_all(body)
            .unwrap();
        path
    }

    fn sample_asc() -> String {
        let mut s = String::from("base hex  timestamps absolute\n");
        for i in 0..50u32 {
            s.push_str(&format!(
                "{:.6}   1  {:03X}  Rx  d 2  AA BB\n",
                f64::from(i) / 10.0,
                0x100 + i
            ));
        }
        s
    }

    #[test]
    fn detect_reads_extension_case_insensitively() {
        let p = |s: &str| LogFormat::detect(Path::new(s));
        assert_eq!(p("log.asc"), Some(LogFormat::Asc));
        assert_eq!(p("LOG.ASC"), Some(LogFormat::Asc));
        assert_eq!(p("capture.blf"), Some(LogFormat::Blf));
        assert_eq!(p("capture.MF4"), Some(LogFormat::Mf4));
        assert_eq!(p("notes.txt"), None);
        assert_eq!(p("no_extension"), None);
    }

    #[test]
    fn mf4_is_reported_as_unsupported_rather_than_silently_empty() {
        let path = temp_log("roxy_can_dispatch.mf4", b"not a real mf4");
        let status = match open_stream(&path) {
            Ok(_) => panic!("MF4 must not open as a stream yet"),
            Err(e) => e.to_string(),
        };
        assert!(
            status.contains("MF4"),
            "status should name the format, got {status}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn unknown_extension_is_rejected() {
        let path = temp_log("roxy_can_dispatch.log", b"0.1 1 1A4 Rx d 0\n");
        assert!(open_stream(&path).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn small_asc_uses_the_vec_stream_and_large_uses_mmap() {
        let text = sample_asc();
        let path = temp_log("roxy_can_dispatch.asc", text.as_bytes());
        let size = text.len() as u64;

        // Production threshold: a few KB stays on the read_to_string path.
        let mut small = open_stream(&path).unwrap();
        assert_eq!(small.describe(), "50 frames", "expected the Vec path");

        // Threshold at or below the file size: same log through the cursor.
        let mut mapped = open_stream_at(&path, size).unwrap();
        let describe = mapped.describe();
        assert!(
            describe.starts_with("ASC, ") && describe.ends_with(" s"),
            "expected the mmap path summary, got {describe}"
        );

        // Both branches must agree on content and length.
        let mut mapped_n = 0usize;
        while mapped.next_frame().is_some() {
            mapped_n += 1;
        }
        let mut small_n = 0usize;
        while small.next_frame().is_some() {
            small_n += 1;
        }
        assert_eq!(mapped_n, small_n);
        assert_eq!(mapped.duration_us(), small.duration_us());
        assert_eq!(mapped.duration_us(), Some(4_900_000));
        std::fs::remove_file(&path).ok();
    }
}
