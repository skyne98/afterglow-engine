//! Streaming asset sources — positional reads with no whole-file buffering.
//!
//! [`AssetSource`] is the single abstraction both serving backends (the CEF
//! scheme handler and the web HTTP dev server) and the asset loader worker
//! build on. A source reports its length and serves `read_at` at arbitrary
//! offsets — the primitive both `Range` requests (web) and `skip`+`read`
//! (CEF) need.
//!
//! Implementations:
//! - [`FsSource`] — a file on disk, read via positional `pread` (no mutex,
//!   no whole-file load). Native only.
//! - [`BytesSource`] — an in-memory `&'static [u8]` (the one embedded asset,
//!   `index.html`). Works on all targets.
//!
//! The crate previously documented itself as "no file-content reads." Adding
//! [`FsSource`] (which opens + reads `File`s) inverts that stance — the
//! user-facing asset system now owns streaming. Path/MIME/confinement logic
//! stays in the parent module, clearly separated from this source module.

use std::io;
use std::path::Path;

/// A positional, streaming byte source.
///
/// One producer (the asset store), one or more consumers (concurrent range
/// requests share a `File` via `pread` — no shared cursor, so `read_at` is
/// trivially safe to call concurrently). Implementations need not be `Send`/
/// `Sync` themselves unless used across threads; [`FsSource`] is.
pub trait AssetSource {
    /// Total length in bytes.
    fn len(&self) -> u64;
    /// `true` if the source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Read up to `buf.len()` bytes at `offset`, returning the number read
    /// (`0` at EOF). Never reads more than `buf.len()`; may read less.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;
    /// An optional weak ETag for caching (typically mtime-based). `None` if
    /// unsupported or unset.
    fn etag(&self) -> Option<String> {
        None
    }
}

/// A file-backed source. Reads via positional `pread` (`FileExt::read_at` on
/// Unix, `seek_read` on Windows) — no mutex, no whole-file load, streams in
/// whatever chunk the caller asks for.
///
/// Native only (`cfg(not(target_arch = "wasm32"))`); on wasm the page fetches
/// via HTTP `Range` instead.
#[cfg(not(target_arch = "wasm32"))]
pub struct FsSource {
    file: std::fs::File,
    size: u64,
    etag: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl FsSource {
    /// Open `path` for positional reads. Returns `None` if the file can't be
    /// opened or its size can't be determined. `etag` is derived from the
    /// modified-time for weak caching.
    pub fn open(path: impl AsRef<Path>) -> Option<Self> {
        let path = path.as_ref();
        let file = std::fs::File::open(path).ok()?;
        let meta = file.metadata().ok()?;
        let size = meta.len();
        let etag = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| format!("\"{:x}-{:x}\"", d.as_secs(), d.subsec_nanos()));
        Some(Self { file, size, etag })
    }

    /// The underlying file path is not retained; this is a convenience for
    /// callers that opened the `File` themselves.
    pub fn from_file(file: std::fs::File, size: u64, etag: Option<String>) -> Self {
        Self { file, size, etag }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AssetSource for FsSource {
    fn len(&self) -> u64 {
        self.size
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.size {
            return Ok(0);
        }
        use std::os::unix::fs::FileExt;
        // `read_at` reads at the given offset without mutating any cursor —
        // safe to call concurrently across threads on the same `File`.
        self.file.read_at(buf, offset)
    }
    fn etag(&self) -> Option<String> {
        self.etag.clone()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl std::fmt::Debug for FsSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FsSource")
            .field("size", &self.size)
            .field("etag", &self.etag)
            .finish()
    }
}

/// An in-memory byte source — the one embedded asset (`index.html`).
/// Works on all targets (wasm included).
pub struct BytesSource(pub &'static [u8]);

impl AssetSource for BytesSource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let off = usize::try_from(offset).unwrap_or(usize::MAX);
        if off >= self.0.len() {
            return Ok(0);
        }
        let end = (off + buf.len()).min(self.0.len());
        let n = end - off;
        buf[..n].copy_from_slice(&self.0[off..end]);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_source_read_full() {
        let s = BytesSource(b"hello world");
        let mut buf = [0u8; 11];
        let n = s.read_at(0, &mut buf).unwrap();
        assert_eq!(n, 11);
        assert_eq!(&buf, b"hello world");
        assert_eq!(s.len(), 11);
    }

    #[test]
    fn bytes_source_read_at_offset() {
        let s = BytesSource(b"hello world");
        let mut buf = [0u8; 5];
        let n = s.read_at(6, &mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf, b"world");
    }

    #[test]
    fn bytes_source_read_past_eof() {
        let s = BytesSource(b"hi");
        let mut buf = [0u8; 10];
        assert_eq!(s.read_at(0, &mut buf).unwrap(), 2);
        assert_eq!(s.read_at(2, &mut buf).unwrap(), 0);
        assert_eq!(s.read_at(100, &mut buf).unwrap(), 0);
    }

    #[test]
    fn bytes_source_partial_read_at_eof() {
        let s = BytesSource(b"hello");
        let mut buf = [0u8; 10];
        let n = s.read_at(3, &mut buf).unwrap();
        assert_eq!(n, 2);
        assert_eq!(&buf[..n], b"lo");
    }

    #[test]
    fn bytes_source_empty() {
        let s = BytesSource(b"");
        assert!(s.is_empty());
        let mut buf = [0u8; 1];
        assert_eq!(s.read_at(0, &mut buf).unwrap(), 0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fs_source_reads_a_file() {
        use std::io::Write;
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("ag-fs-source-{}", std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp).unwrap();
            f.write_all(b"hello from disk").unwrap();
        }
        let s = FsSource::open(&tmp).unwrap();
        assert_eq!(s.len(), 15);
        let mut buf = [0u8; 15];
        assert_eq!(s.read_at(0, &mut buf).unwrap(), 15);
        assert_eq!(&buf, b"hello from disk");
        // partial read at offset
        let mut small = [0u8; 5];
        assert_eq!(s.read_at(6, &mut small).unwrap(), 5);
        assert_eq!(&small, b"from ");
        assert!(s.etag().is_some());
        let _ = std::fs::remove_file(&tmp);
    }
}
