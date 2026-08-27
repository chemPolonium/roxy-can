//! Byte source shared by the streaming log readers.
//!
//! A reader needs a `&[u8]` view it can cursor over without materialising the
//! whole log; where those bytes come from differs between production (a memory
//! map, so RSS tracks page cache instead of the heap) and tests (an owned
//! `Vec`, so no filesystem is involved). Wrapping both in one enum keeps the
//! parsing code in `asc`/`blf` unaware of which it holds.

use std::fs::File;
use std::path::Path;

use memmap2::Mmap;

use crate::log::error::LogError;

pub(crate) enum Backing {
    Mapped {
        // Keeping the File alive is what pins the mapping on Windows; if it
        // drops first the map is torn down mid-iteration.
        #[allow(dead_code)]
        file: File,
        map: Mmap,
    },
    // Only the `from_bytes` test constructors build this variant; the arm has
    // to exist for the shared readers to compile in a normal build.
    #[allow(dead_code)]
    Owned(Vec<u8>),
}

impl Backing {
    /// Memory-map `path`. SAFETY: every caller treats the log as immutable once
    /// written. Vector either closes the file or appends whole records, so a
    /// concurrently-grown tail at worst fails a bounds or signature check and
    /// the reader stops cleanly -- it cannot alias a write mid-parse.
    pub(crate) fn map_path(path: &Path) -> Result<Self, LogError> {
        let file = File::open(path)?;
        let map = unsafe { Mmap::map(&file) }?;
        Ok(Backing::Mapped { file, map })
    }

    #[cfg(test)]
    pub(crate) fn owned(bytes: &[u8]) -> Self {
        Backing::Owned(bytes.to_vec())
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Backing::Mapped { map, .. } => &map[..],
            Backing::Owned(v) => v.as_slice(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// One-word provenance tag for `describe()`, so the status bar makes it
    /// obvious when a test fixture is not going through the mmap path.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Backing::Mapped { .. } => "mmap",
            Backing::Owned(_) => "mem",
        }
    }
}
