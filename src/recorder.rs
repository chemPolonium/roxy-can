//! The recorder: the open file's lifetime plus the Record checkbox's
//! intent state. The backend follows the path's extension — `.blf`
//! writes Vector's binary container format, anything else writes ASC.

use crate::can::frame::CanFrame;
use crate::log::blf::BlfWriter;
use crate::log::AscWriter;

enum Backend {
    Asc(AscWriter),
    Blf(BlfWriter),
}

/// Owns the open file while recording. The checkbox intent
/// (`recording`) is deliberately separate from the open file (`writer`):
/// ticking Record while stopped only arms the intent -- the file itself is
/// created by the next measurement start, so an armed checkbox never leaves
/// an empty record behind.
pub struct Recorder {
    writer: Option<Backend>,
    pub recording: bool,
    /// Base path as typed; the actual file gets a date-time suffix.
    pub record_path: String,
    /// The dated path of the most recent recording, kept as a replay source.
    pub last_record: String,
    /// Optional id whitelist for the file: empty records everything, a
    /// non-empty list records only those `(id, extended)` frames. The
    /// filter gates the file's contents only -- trace, aggregates, and
    /// the spec always see the whole bus.
    pub ids: Vec<(u32, bool)>,
}

impl Recorder {
    pub fn new() -> Self {
        Recorder {
            writer: None,
            recording: false,
            record_path: String::new(),
            last_record: String::new(),
            ids: Vec::new(),
        }
    }

    /// Whether a frame belongs in the file under the current filter.
    pub fn admits(&self, f: &CanFrame) -> bool {
        self.ids.is_empty() || self.ids.contains(&(f.id, f.extended))
    }

    /// Writes one frame if a recording is open and the frame passes the
    /// record filter; a no-op otherwise.
    pub fn write(&mut self, f: &CanFrame) {
        if !self.admits(f) {
            return;
        }
        match &mut self.writer {
            Some(Backend::Asc(w)) => w.write(f).ok(),
            Some(Backend::Blf(w)) => {
                w.write(f);
                None
            }
            None => None,
        };
    }

    /// Closes the file, if any. Recorded data stays; only the handle goes.
    pub fn close(&mut self) {
        if let Some(w) = self.writer.take() {
            match w {
                Backend::Asc(w) => w.finish().ok(),
                Backend::Blf(w) => w.finish().ok(),
            };
        }
    }

    /// Opens the dated file derived from `record_path`. The extension
    /// picks the backend: `.blf` writes the binary container format,
    /// anything else records ASC as before. Returns the opened path, or
    /// the error text for the status line.
    pub fn open(&mut self) -> Result<String, String> {
        let b = self.record_path.trim();
        let lower = b.to_ascii_lowercase();
        let (blf, cut) = if lower.ends_with(".blf") {
            (true, 4)
        } else if lower.ends_with(".asc") {
            (false, 4)
        } else {
            (false, 0)
        };
        let base_raw = &b[..b.len() - cut];
        let base = if base_raw.is_empty() { "record" } else { base_raw };
        let path = if blf {
            format!(
                "{}_{}.blf",
                base,
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            )
        } else {
            format!(
                "{}_{}.asc",
                base,
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            )
        };
        self.writer = Some(if blf {
            Backend::Blf(BlfWriter::create(&path).map_err(|e| e.to_string())?)
        } else {
            Backend::Asc(AscWriter::new(&path).map_err(|e| e.to_string())?)
        });
        let opened = path.clone();
        self.last_record = opened.clone();
        Ok(opened)
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}
