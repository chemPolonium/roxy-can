//! The ASC recorder: the open file's lifetime plus the Record checkbox's
//! intent state.

use crate::can::frame::CanFrame;
use crate::log::AscWriter;

/// Owns the open ASC file while recording. The checkbox intent
/// (`recording`) is deliberately separate from the open file (`writer`):
/// ticking Record while stopped only arms the intent -- the file itself is
/// created by the next measurement start, so an armed checkbox never leaves
/// an empty record behind.
pub struct Recorder {
    writer: Option<AscWriter>,
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
        if let Some(w) = &mut self.writer {
            w.write(f).ok();
        }
    }

    /// Closes the file, if any. Recorded data stays; only the handle goes.
    pub fn close(&mut self) {
        if let Some(w) = self.writer.take() {
            w.finish().ok();
        }
    }

    /// Opens the dated ASC file derived from `record_path`. Returns the
    /// opened path, or the error text for the status line.
    pub fn open(&mut self) -> Result<String, String> {
        let b = self.record_path.trim();
        let b = b
            .strip_suffix(".asc")
            .or_else(|| b.strip_suffix(".ASC"))
            .unwrap_or(b);
        let base = if b.is_empty() { "record" } else { b };
        let path = format!(
            "{}_{}.asc",
            base,
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        match AscWriter::new(&path) {
            Ok(w) => {
                self.writer = Some(w);
                let opened = path.clone();
                self.last_record = opened.clone();
                Ok(opened)
            }
            Err(e) => Err(format!("{e}")),
        }
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}
