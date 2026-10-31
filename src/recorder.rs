//! The recorder: the open file's lifetime plus the Record checkbox's
//! intent state. The backend follows the path's extension — `.blf`
//! writes Vector's binary container format, anything else writes ASC.

use crate::can::frame::CanFrame;
use crate::log::AscWriter;
use crate::log::blf::BlfWriter;

enum Backend {
    Asc(AscWriter),
    Blf(BlfWriter),
}

/// Which traffic the file is allowed to contain, in the two numbering spaces a
/// mixed session has. Empty on both sides records everything. A CAN entry is
/// `(id, extended)` -- the record filter has never distinguished channels, and
/// a FlexRay entry is `(bus, slot)`, because that is how every table in here
/// names a FlexRay arrival.
///
/// The two halves stay apart on purpose: `13` is a CAN id and a slot number at
/// once, so a filter that folded them would let a CAN entry admit a FlexRay row
/// (or drop one) with no way to tell the user which bus it decided on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordFilter {
    pub can: Vec<(u32, bool)>,
    pub fr: Vec<(u8, u16)>,
}

impl RecordFilter {
    /// A CAN-only whitelist: the shape this filter had before it could name a
    /// FlexRay slot, and what a caller that only cares about CAN still wants.
    #[cfg(test)]
    pub fn can(ids: Vec<(u32, bool)>) -> Self {
        Self {
            can: ids,
            fr: Vec::new(),
        }
    }

    pub fn admits_can(&self, id: u32, extended: bool) -> bool {
        // A filter that names only FlexRay slots says nothing about CAN frames:
        // it is a whitelist of what to record, not a switch that mutes the
        // other bus -- ticking "FR0:13" must not silently lose the CAN trace.
        self.can.is_empty() || self.can.contains(&(id, extended))
    }

    pub fn admits_fr(&self, bus: u8, slot: u16) -> bool {
        self.fr.is_empty() || self.fr.contains(&(bus, slot))
    }
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
    /// Monotonic per-session counter for `{index}` field codes.
    record_counter: u64,
    /// Optional whitelist for the file. The filter gates the file's contents
    /// only -- trace, aggregates, and the spec always see the whole bus.
    pub filter: RecordFilter,
}

impl Recorder {
    pub fn new() -> Self {
        Recorder {
            writer: None,
            recording: false,
            record_path: String::new(),
            last_record: String::new(),
            record_counter: 0,
            filter: RecordFilter::default(),
        }
    }

    /// Whether a frame belongs in the file under the current filter.
    pub fn admits(&self, f: &CanFrame) -> bool {
        self.filter.admits_can(f.id, f.extended)
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

    /// FlexRay rows for the open recording, gated by the same filter as the CAN
    /// frames -- which it now can be, since a filter entry names its bus
    /// ([`RecordFilter`]). A row that has no frame name loses it in ASC, and
    /// every row comes back on bus 0 -- the log has no place for either, exactly
    /// as in BLF.
    pub fn write_fr(&mut self, r: &crate::trace::FrRow) {
        if !self.filter.admits_fr(r.bus, r.slot) {
            return;
        }
        match &mut self.writer {
            Some(Backend::Asc(w)) => {
                w.write_fr(r).ok();
            }
            Some(Backend::Blf(w)) => w.write_fr(r),
            None => {}
        }
    }

    /// Pushes the open file's buffer to disk. Called at the end of every
    /// measurement step, so the file grows while the recording runs: a person
    /// watching it (or a process that dies before Stop) should see the frames
    /// that were already on the bus, not a 144-byte stub.
    pub fn flush(&mut self) {
        match &mut self.writer {
            Some(Backend::Asc(w)) => {
                w.flush().ok();
            }
            Some(Backend::Blf(w)) => w.flush(),
            None => {}
        }
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
        // Never abandon a live file. Nothing on today's call paths gets here
        // with a writer still open -- a start closes first, and the checkbox and
        // the trigger action are exclusive -- but "the caller remembered to
        // close" is not an assumption this type should carry: an abandoned BLF
        // loses its buffered objects and an abandoned ASC its trailer.
        self.close();
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
        let has_codes = base_raw.contains("{date}")
            || base_raw.contains("{time}")
            || base_raw.contains("{index}");
        self.record_counter = self.record_counter.saturating_add(1);
        let now = chrono::Local::now();
        let index = self.record_counter;
        let expanded = base_raw
            .replace("{date}", &now.format("%Y%m%d").to_string())
            .replace("{time}", &now.format("%H%M%S").to_string())
            .replace("{index}", &format!("{index:03}"));
        let base = if expanded.is_empty() {
            "record".to_string()
        } else {
            expanded
        };
        let ext = if blf { "blf" } else { "asc" };
        // Without a field code the recorder owns uniqueness with its own
        // timestamp; a pattern that already says when it ran keeps the name
        // the operator wrote.
        let stem = if has_codes {
            base
        } else {
            format!("{base}_{}", now.format("%Y%m%d_%H%M%S"))
        };
        let mut path = format!("{stem}.{ext}");
        // Two records in the same second must never overwrite each other:
        // every green lamp is a new file, even on a quick off/on toggle.
        let mut seq = 2;
        while std::path::Path::new(&path).exists() {
            path = format!("{stem}_{seq}.{ext}");
            seq += 1;
        }
        // The default home is the project's Record/ folder; make sure it
        // exists instead of failing the whole recording on a missing dir.
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("record folder unavailable: {e}"))?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("roxy-can-recorder-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn field_codes_expand_in_record_names() {
        let dir = temp_dir("codes");
        let mut rec = Recorder::new();
        rec.record_path = dir
            .join("drive_{date}_{time}_{index}.blf")
            .to_string_lossy()
            .into_owned();
        let path = rec.open().unwrap();
        rec.close();
        assert!(!path.contains('{'), "field codes were left literal: {path}");
        assert!(path.ends_with("_001.blf"), "index code: {path}");
        assert!(std::path::Path::new(&path).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn repeated_starts_never_reuse_a_path() {
        let dir = temp_dir("unique");
        let mut rec = Recorder::new();
        rec.record_path = dir.join("same.asc").to_string_lossy().into_owned();
        let first = rec.open().unwrap();
        rec.close();
        let second = rec.open().unwrap();
        rec.close();
        assert_ne!(first, second, "each start opens a fresh file");
        assert!(std::path::Path::new(&first).exists());
        assert!(std::path::Path::new(&second).exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
