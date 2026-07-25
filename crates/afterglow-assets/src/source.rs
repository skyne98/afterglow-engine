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

/// Opens a confined [`AssetSource`] for a URL path.
///
/// Implemented by [`crate::AssetRoot`] (uncached: `canonicalize` + `File::open`
/// + `metadata` per call) and [`AssetSourceCache`] (cached: open once, reuse
/// the `pread`-safe handle). Both serving backends (the CEF scheme handler and
/// the web dev server) take this so the cache is a drop-in with no policy
/// duplication.
pub trait AssetSourceProvider {
    /// Resolve, confine, and open `url_path`. `None` for any miss/escape/
    /// unreadable path — callers answer a uniform 404.
    fn open_source(&self, url_path: &str) -> Option<Box<dyn AssetSource + Send + Sync>>;
}

/// A shared, `pread`-safe file source. Cheap to clone (one `Arc` refcount);
/// `read_at` is positional so concurrent reads on clones need no cursor or lock.
#[cfg(not(target_arch = "wasm32"))]
struct SharedFsSource(std::sync::Arc<FsSource>);

#[cfg(not(target_arch = "wasm32"))]
impl AssetSource for SharedFsSource {
    fn len(&self) -> u64 {
        self.0.len()
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read_at(offset, buf)
    }
    fn etag(&self) -> Option<String> {
        self.0.etag()
    }
}

/// Default maximum number of concurrently retained open asset files.
#[cfg(not(target_arch = "wasm32"))]
pub const DEFAULT_SOURCE_CACHE_CAPACITY: usize = 16;

/// An open-source cache keyed by URL path.
///
/// Resolves + confines + opens a file on the first request for a path;
/// subsequent requests reuse the open `pread`-safe handle, skipping the
/// per-request `canonicalize` + `File::open` + `metadata` that
/// [`crate::AssetRoot::open_source`] performs every call. It has a fixed
/// capacity and round-robin replacement, so it cannot retain unbounded file
/// descriptors or path strings. For a workload of many small range reads
/// against one container (the virtual-texture page-read pattern), the
/// container remains open and removes ~3-5 syscalls per read.
///
/// **Assumes assets are not replaced mid-session** (the cooked-container case):
/// a cached open file descriptor reads the inode it opened. If a file is
/// rebuilt, call [`invalidate`](Self::invalidate) or [`clear`](Self::clear).
///
/// Confinement is unchanged: the first request for a path canonically confines
/// via [`crate::AssetRoot`]; a cached entry is already confined, so a later
/// request for the same URL path cannot reach a different file.
#[cfg(not(target_arch = "wasm32"))]
pub struct AssetSourceCache {
    root: crate::AssetRoot,
    capacity: usize,
    next_evict: std::sync::atomic::AtomicUsize,
    entries: std::sync::Mutex<Vec<(String, std::sync::Arc<FsSource>)>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl AssetSourceCache {
    /// Wrap an [`crate::AssetRoot`] with a fixed 16-source cache.
    pub fn new(root: crate::AssetRoot) -> Self {
        Self::with_capacity(root, DEFAULT_SOURCE_CACHE_CAPACITY)
    }

    /// Wrap an [`crate::AssetRoot`] with a fixed-capacity open-source cache.
    pub fn with_capacity(root: crate::AssetRoot, capacity: usize) -> Self {
        assert!(capacity > 0, "source cache capacity must be positive");
        Self {
            root,
            capacity,
            next_evict: std::sync::atomic::AtomicUsize::new(0),
            entries: std::sync::Mutex::new(Vec::with_capacity(capacity)),
        }
    }

    /// Drop the cached open source for `url_path` (e.g. after the file is rebuilt).
    pub fn invalidate(&self, url_path: &str) {
        if let Ok(mut entries) = self.entries.lock()
            && let Some(index) = entries.iter().position(|(path, _)| path == url_path)
        {
            entries.swap_remove(index);
        }
    }

    /// Drop every cached open source.
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AssetSourceProvider for AssetSourceCache {
    fn open_source(&self, url_path: &str) -> Option<Box<dyn AssetSource + Send + Sync>> {
        // Capacity is fixed, so this bounded linear probe is O(1) worst case.
        // The hot hit performs no path resolution, file open, metadata, or
        // allocation; it only clones the retained Arc.
        {
            let entries = self.entries.lock().ok()?;
            if let Some((_, src)) = entries.iter().find(|(path, _)| path == url_path) {
                return Some(Box::new(SharedFsSource(std::sync::Arc::clone(src))));
            }
        }
        // Slow path: first request for this path — confine + open once.
        // Concurrent first requests may both open; one transient fd is benign.
        let src = std::sync::Arc::new(self.root.open_source(url_path)?);
        let mut entries = self.entries.lock().ok()?;
        if let Some((_, cached)) = entries.iter().find(|(path, _)| path == url_path) {
            return Some(Box::new(SharedFsSource(std::sync::Arc::clone(cached))));
        }
        if entries.len() < self.capacity {
            entries.push((url_path.to_owned(), std::sync::Arc::clone(&src)));
        } else {
            let index = self
                .next_evict
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                % self.capacity;
            entries[index] = (url_path.to_owned(), std::sync::Arc::clone(&src));
        }
        Some(Box::new(SharedFsSource(src)))
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn asset_source_cache_serves_and_invalidates() {
        // Build a root dir with one file so AssetRoot canonicalizes.
        let root_dir = std::env::temp_dir().join(format!(
            "ag-cache-{}-{}",
            std::process::id(),
            std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root_dir);
        std::fs::create_dir_all(&root_dir).unwrap();
        let file = root_dir.join("a.txt");
        std::fs::write(&file, b"hello").unwrap();
        let root = crate::AssetRoot::new(&root_dir).unwrap();
        let cache = AssetSourceCache::new(root);

        // First call resolves + confines + opens.
        let s1 = cache.open_source("/a.txt").expect("first open");
        assert_eq!(s1.len(), 5);
        let mut buf = [0u8; 5];
        assert_eq!(s1.read_at(0, &mut buf).unwrap(), 5);
        assert_eq!(&buf, b"hello");

        // Second call is a cache hit (same content, no re-open).
        let s2 = cache.open_source("/a.txt").expect("cached open");
        assert_eq!(s2.len(), 5);

        // Missing path -> None and is NOT cached.
        assert!(cache.open_source("/missing").is_none());

        // Replace the file with a NEW inode (unlink + recreate). The cached fd
        // still points at the old (now-unlinked) inode and reads old content,
        // proving the cache retained the open handle rather than re-opening.
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, b"new content!!").unwrap();
        let s3 = cache
            .open_source("/a.txt")
            .expect("cached open after replace");
        assert_eq!(s3.len(), 5, "cached fd retains old inode content");

        // invalidate() forces re-open on the next call.
        cache.invalidate("/a.txt");
        let s4 = cache
            .open_source("/a.txt")
            .expect("re-open after invalidate");
        assert_eq!(s4.len(), 13, "invalidate re-opens the replaced file");

        let _ = std::fs::remove_dir_all(&root_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn asset_source_cache_capacity_evicts_open_handle() {
        let root_dir = std::env::temp_dir().join(format!(
            "ag-cache-capacity-{}-{}",
            std::process::id(),
            std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root_dir);
        std::fs::create_dir_all(&root_dir).unwrap();
        let a = root_dir.join("a.txt");
        std::fs::write(&a, b"old").unwrap();
        std::fs::write(root_dir.join("b.txt"), b"b").unwrap();
        let cache = AssetSourceCache::with_capacity(crate::AssetRoot::new(&root_dir).unwrap(), 1);
        assert_eq!(cache.open_source("/a.txt").unwrap().len(), 3);
        std::fs::remove_file(&a).unwrap();
        std::fs::write(&a, b"newer").unwrap();

        // b replaces the sole slot, dropping a's retained descriptor.
        assert_eq!(cache.open_source("/b.txt").unwrap().len(), 1);
        assert_eq!(cache.open_source("/a.txt").unwrap().len(), 5);
        let _ = std::fs::remove_dir_all(&root_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn asset_root_provider_is_uncached() {
        let root_dir = std::env::temp_dir().join(format!(
            "ag-root-prov-{}-{}",
            std::process::id(),
            std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root_dir);
        std::fs::create_dir_all(&root_dir).unwrap();
        std::fs::write(root_dir.join("a.txt"), b"hi").unwrap();
        let root = crate::AssetRoot::new(&root_dir).unwrap();
        let s = AssetSourceProvider::open_source(&root, "/a.txt").expect("open");
        assert_eq!(s.len(), 2);
        assert!(AssetSourceProvider::open_source(&root, "/missing").is_none());
        let _ = std::fs::remove_dir_all(&root_dir);
    }
}
