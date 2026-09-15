//! Native transactional bytes with caller-selected capacities.
//!
//! One owner uses this store on a native worker thread. Keys and values contain
//! no application policy. Optional file admission bounds the database and journal together.

mod vfs;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::time::Duration;

const PAGE_BYTES: u64 = 4096;
const APPLICATION_ID: i64 = 0x4147_4253;
const VERSION: i64 = 1;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub database_bytes: u64,
    pub value_bytes: usize,
    pub transaction_operations: usize,
    pub transaction_bytes: usize,
    /// SQLite page-cache target, not a total process-memory limit.
    pub cache_bytes: u32,
}

#[derive(Debug)]
pub enum Error {
    Limit(&'static str),
    Format,
    Database(rusqlite::Error),
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit(name) => write!(output, "byte-store capacity exceeded: {name}"),
            Self::Format => output.write_str("incorrect byte-store format"),
            Self::Database(error) => error.fmt(output),
        }
    }
}
impl std::error::Error for Error {}

pub enum Mutation<'a> {
    Put { key: u64, value: &'a [u8] },
    Remove { key: u64 },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Stats {
    pub entries: u64,
    pub value_bytes: u64,
    pub database_bytes: u64,
}

pub struct ByteStore {
    // Drop the connection before its registered VFS.
    connection: Connection,
    vfs: Option<Box<vfs::BoundedVfs>>,
    limits: Limits,
}

impl ByteStore {
    /// Open a database in a caller-owned directory. Do not pass an unconfined path.
    /// An existing store must use the same format and fit the requested capacity.
    pub fn open(path: &Path, limits: Limits) -> Result<Self, Error> {
        Self::open_inner(path, limits, None)
    }

    /// Bound database plus rollback-journal extents before file growth.
    pub fn open_bounded(path: &Path, limits: Limits, file_bytes: u64) -> Result<Self, Error> {
        if file_bytes < limits.database_bytes { return Err(Error::Limit("file capacity")); }
        Self::open_inner(path, limits, Some(vfs::BoundedVfs::new(file_bytes)?))
    }

    pub fn peak_file_bytes(&self) -> Option<u64> { self.vfs.as_ref().map(|vfs| vfs.peak()) }

    fn open_inner(path: &Path, limits: Limits, vfs: Option<Box<vfs::BoundedVfs>>) -> Result<Self, Error> {
        let pages = limits.database_bytes / PAGE_BYTES;
        if !(4..=u64::from(u32::MAX) - 1).contains(&pages)
            || limits.value_bytes == 0
            || limits.value_bytes > (i32::MAX as usize - 4096)
            || !(1..=1024).contains(&limits.transaction_operations)
            || limits.transaction_bytes < limits.value_bytes
            || limits.cache_bytes < 1024
        {
            return Err(Error::Limit("configuration"));
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = if let Some(vfs) = &vfs {
            Connection::open_with_flags_and_vfs(path, flags, vfs.name())?
        } else { Connection::open_with_flags(path, flags)? };
        connection.busy_timeout(Duration::ZERO)?;
        let application: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let empty = connection.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema)",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !(application == APPLICATION_ID && version == VERSION)
            && !(application == 0 && version == 0 && empty)
        {
            return Err(Error::Format);
        }
        connection.pragma_update(None, "page_size", PAGE_BYTES)?;
        let page_size: u64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
        if page_size != PAGE_BYTES {
            return Err(Error::Format);
        }
        let admitted_pages: u64 =
            connection.query_row(&format!("PRAGMA max_page_count={pages}"), [], |row| {
                row.get(0)
            })?;
        if admitted_pages > pages {
            return Err(Error::Limit("existing database"));
        }
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        let journal: String = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        if !journal.eq_ignore_ascii_case("delete") { return Err(Error::Format); }
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "temp_store", "MEMORY")?;
        connection.pragma_update(None, "mmap_size", 0)?;
        connection.pragma_update(None, "cache_size", -(i64::from(limits.cache_bytes) / 1024))?;
        connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
        connection.set_prepared_statement_cache_capacity(8);
        if empty {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(
                "CREATE TABLE items(key INTEGER PRIMARY KEY, value BLOB NOT NULL);
                 CREATE TABLE counts(id INTEGER PRIMARY KEY CHECK(id=1), entries INTEGER NOT NULL, bytes INTEGER NOT NULL);
                 INSERT INTO counts VALUES(1,0,0);")?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", VERSION)?;
            transaction.commit()?;
        }
        connection.set_limit(
            rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
            (limits.value_bytes + 4096) as i32,
        )?;
        let store = Self { connection, vfs, limits };
        store.stats()?;
        Ok(store)
    }

    /// Copy a value into caller storage. Missing keys and empty values differ.
    /// A capacity error leaves the output unchanged.
    pub fn get(&self, key: u64, output: &mut [u8]) -> Result<Option<usize>, Error> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT value FROM items WHERE key=?1")?;
        let mut rows = statement.query([key as i64])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let value = row.get_ref(0)?.as_blob().map_err(|_| Error::Format)?;
        if value.len() > self.limits.value_bytes {
            return Err(Error::Limit("stored value"));
        }
        if value.len() > output.len() {
            return Err(Error::Limit("output"));
        }
        output[..value.len()].copy_from_slice(value);
        Ok(Some(value.len()))
    }

    /// Read at most 1,024 keys after `after`, in unsigned numeric order.
    /// The caller owns the output and uses its last key for the next batch.
    pub fn keys_after(&self, after: Option<u64>, output: &mut [u64]) -> Result<usize, Error> {
        if output.len() > 1024 {
            return Err(Error::Limit("key batch"));
        }
        if after == Some(u64::MAX) || output.is_empty() {
            return Ok(0);
        }
        let start = after.map_or(0, |key| key + 1);
        let mut count = 0;
        // Two indexed ranges retain unsigned order across SQLite's signed keys.
        for (low, high) in [(0, i64::MAX as u64), (1u64 << 63, u64::MAX)] {
            if start > high || count == output.len() {
                continue;
            }
            let mut statement = self.connection.prepare_cached(
                "SELECT key FROM items WHERE key>=?1 AND key<=?2 ORDER BY key LIMIT ?3",
            )?;
            let mut rows = statement.query(params![start.max(low) as i64, high as i64, (output.len() - count) as i64])?;
            while let Some(row) = rows.next()? {
                output[count] = row.get::<_, i64>(0)? as u64;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Commit all mutations together. Repeated keys follow slice order.
    /// Invalid input or a database error leaves the previous transaction intact.
    pub fn apply(&mut self, mutations: &[Mutation<'_>]) -> Result<(), Error> {
        if mutations.len() > self.limits.transaction_operations {
            return Err(Error::Limit("transaction operations"));
        }
        let mut bytes = 0usize;
        for mutation in mutations {
            if let Mutation::Put { value, .. } = mutation {
                if value.len() > self.limits.value_bytes {
                    return Err(Error::Limit("value"));
                }
                bytes = bytes
                    .checked_add(value.len())
                    .ok_or(Error::Limit("transaction bytes"))?;
                if bytes > self.limits.transaction_bytes {
                    return Err(Error::Limit("transaction bytes"));
                }
            }
        }
        if mutations.is_empty() {
            return Ok(());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut entries, mut total): (i64, i64) =
            transaction.query_row("SELECT entries,bytes FROM counts WHERE id=1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        for mutation in mutations {
            let key = match mutation {
                Mutation::Put { key, .. } | Mutation::Remove { key } => *key as i64,
            };
            let old: Option<i64> = transaction
                .prepare_cached("SELECT length(value) FROM items WHERE key=?1")?
                .query_row([key], |row| row.get(0))
                .optional()?;
            if let Some(old) = old {
                entries -= 1;
                total -= old;
            }
            match mutation {
                Mutation::Put { value, .. } => {
                    transaction.prepare_cached("INSERT INTO items(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value")?
                        .execute(params![key, value])?;
                    entries += 1;
                    total = total
                        .checked_add(value.len() as i64)
                        .ok_or(Error::Limit("stored bytes"))?;
                }
                Mutation::Remove { .. } => {
                    transaction
                        .prepare_cached("DELETE FROM items WHERE key=?1")?
                        .execute([key])?;
                }
            }
        }
        if entries < 0 || total < 0 {
            return Err(Error::Format);
        }
        transaction.execute(
            "UPDATE counts SET entries=?1,bytes=?2 WHERE id=1",
            params![entries, total],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn stats(&self) -> Result<Stats, Error> {
        let (entries, bytes): (i64, i64) = self.connection.query_row(
            "SELECT entries,bytes FROM counts WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let pages: u64 = self
            .connection
            .pragma_query_value(None, "page_count", |row| row.get(0))?;
        if entries < 0 || bytes < 0 || pages * PAGE_BYTES > self.limits.database_bytes {
            return Err(Error::Format);
        }
        Ok(Stats {
            entries: entries as u64,
            value_bytes: bytes as u64,
            database_bytes: pages * PAGE_BYTES,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::ffi;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "afterglow-byte-store-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn file(&self) -> std::path::PathBuf {
            self.0.join("bytes.sqlite")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn limits() -> Limits {
        Limits {
            database_bytes: 8 * 1024 * 1024,
            value_bytes: 65536,
            transaction_operations: 4,
            transaction_bytes: 131072,
            cache_bytes: 65536,
        }
    }
    fn read(store: &ByteStore, key: u64) -> Option<Vec<u8>> {
        let mut output = vec![0; 65536];
        store.get(key, &mut output).unwrap().map(|len| {
            output.truncate(len);
            output
        })
    }

    #[test]
    fn commits_reopens_and_distinguishes_empty_values() {
        let directory = Directory::new();
        let mut store = ByteStore::open(&directory.file(), limits()).unwrap();
        store
            .apply(&[
                Mutation::Put {
                    key: 0,
                    value: b"first",
                },
                Mutation::Put {
                    key: u64::MAX,
                    value: b"",
                },
            ])
            .unwrap();
        assert_eq!(read(&store, 0), Some(b"first".to_vec()));
        assert_eq!(read(&store, u64::MAX), Some(vec![]));
        assert_eq!(read(&store, 4), None);
        let mut short = [0x55; 2];
        assert!(store.get(0, &mut short).is_err());
        assert_eq!(short, [0x55; 2]);
        store
            .apply(&[
                Mutation::Put {
                    key: 0,
                    value: b"next",
                },
                Mutation::Remove { key: 0 },
                Mutation::Put {
                    key: 0,
                    value: b"final",
                },
                Mutation::Remove { key: u64::MAX },
            ])
            .unwrap();
        let stats = store.stats().unwrap();
        assert_eq!((stats.entries, stats.value_bytes), (1, 5));
        drop(store);
        let store = ByteStore::open(&directory.file(), limits()).unwrap();
        assert_eq!(read(&store, 0), Some(b"final".to_vec()));
        assert_eq!(store.stats().unwrap(), stats);
    }

    #[test]
    fn key_batches_keep_unsigned_order_and_allow_removal_between_batches() {
        let directory = Directory::new();
        let mut store = ByteStore::open(&directory.file(), limits()).unwrap();
        let keys = [0, 1, i64::MAX as u64, 1u64 << 63, u64::MAX];
        for key in keys {
            store.apply(&[Mutation::Put { key, value: b"value" }]).unwrap();
        }
        let mut output = [0; 2];
        let mut after = None;
        let mut found = Vec::new();
        loop {
            let count = store.keys_after(after, &mut output).unwrap();
            if count == 0 { break; }
            found.extend_from_slice(&output[..count]);
            after = Some(output[count - 1]);
            for &key in &output[..count] {
                store.apply(&[Mutation::Remove { key }]).unwrap();
            }
        }
        assert_eq!(found, keys);
        assert_eq!(store.keys_after(None, &mut []).unwrap(), 0);
        assert!(store.keys_after(None, &mut [0; 1025]).is_err());
        assert_eq!(store.stats().unwrap().entries, 0);
    }

    #[test]
    fn validates_the_entire_transaction_before_writes() {
        let directory = Directory::new();
        let mut store = ByteStore::open(&directory.file(), limits()).unwrap();
        store
            .apply(&[Mutation::Put {
                key: 1,
                value: b"keep",
            }])
            .unwrap();
        let oversized = vec![1; 65537];
        assert!(
            store
                .apply(&[
                    Mutation::Remove { key: 1 },
                    Mutation::Put {
                        key: 2,
                        value: &oversized
                    }
                ])
                .is_err()
        );
        let block = vec![1; 65536];
        assert!(
            store
                .apply(&[
                    Mutation::Put {
                        key: 1,
                        value: &block
                    },
                    Mutation::Put {
                        key: 2,
                        value: &block
                    },
                    Mutation::Put {
                        key: 3,
                        value: &[1]
                    }
                ])
                .is_err()
        );
        assert!(
            store
                .apply(&[
                    Mutation::Remove { key: 1 },
                    Mutation::Remove { key: 2 },
                    Mutation::Remove { key: 3 },
                    Mutation::Remove { key: 4 },
                    Mutation::Remove { key: 5 }
                ])
                .is_err()
        );
        assert_eq!(read(&store, 1), Some(b"keep".to_vec()));
        assert_eq!(store.stats().unwrap().entries, 1);
    }

    #[test]
    fn full_database_rolls_back_prior_writes_and_accepts_later_commands() {
        let directory = Directory::new();
        let mut bounds = limits();
        bounds.database_bytes = 4 * PAGE_BYTES;
        let mut store = ByteStore::open(&directory.file(), bounds).unwrap();
        store
            .apply(&[Mutation::Put {
                key: 1,
                value: b"keep",
            }])
            .unwrap();
        let error = store
            .apply(&[
                Mutation::Put {
                    key: 1,
                    value: b"lost",
                },
                Mutation::Put {
                    key: 2,
                    value: &vec![5; 65536],
                },
            ])
            .unwrap_err();
        assert!(matches!(
            error,
            Error::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ErrorCode::DiskFull,
                    ..
                },
                _
            ))
        ));
        assert_eq!(read(&store, 1), Some(b"keep".to_vec()));
        assert_eq!(read(&store, 2), None);
        assert!(store.stats().unwrap().database_bytes <= bounds.database_bytes);
        store
            .apply(&[Mutation::Put {
                key: 1,
                value: b"next",
            }])
            .unwrap();
        assert_eq!(read(&store, 1), Some(b"next".to_vec()));
    }

    #[test]
    fn combined_file_limit_preserves_data_after_journal_exhaustion() {
        let directory = Directory::new();
        let capacity = 256 * 1024;
        let bounds = Limits { database_bytes: capacity, ..limits() };
        let mut store = ByteStore::open_bounded(&directory.file(), bounds, capacity).unwrap();
        let original = vec![7; 65536];
        store.apply(&[Mutation::Put { key: 1, value: &original }, Mutation::Put { key: 2, value: &original }]).unwrap();
        let changed = vec![9; 65536];
        let error = store.apply(&[Mutation::Put { key: 1, value: &changed }, Mutation::Put { key: 2, value: &changed }]).unwrap_err();
        assert!(matches!(error, Error::Database(rusqlite::Error::SqliteFailure(ffi::Error { code: rusqlite::ErrorCode::DiskFull, .. }, _))));
        assert_eq!(read(&store, 1), Some(original.clone()));
        assert_eq!(read(&store, 2), Some(original.clone()));
        assert!(store.peak_file_bytes().unwrap() <= capacity);
        assert!(store.peak_file_bytes().unwrap() > store.stats().unwrap().database_bytes);
        // Another path or connection cannot share this connection's quota slots.
        let foreign = directory.file().with_extension("other-journal");
        std::fs::write(&foreign, b"keep").unwrap();
        let foreign_name = std::ffi::CString::new(foreign.to_str().unwrap()).unwrap();
        let vfs_name = std::ffi::CString::new(store.vfs.as_ref().unwrap().name()).unwrap();
        unsafe {
            let vfs = ffi::sqlite3_vfs_find(vfs_name.as_ptr());
            assert_eq!(((*vfs).xDelete.unwrap())(vfs, foreign_name.as_ptr(), 0), ffi::SQLITE_IOERR_DELETE);
        }
        assert_eq!(std::fs::read(&foreign).unwrap(), b"keep");
        assert!(Connection::open_with_flags_and_vfs(directory.file(), OpenFlags::SQLITE_OPEN_READ_WRITE,
            store.vfs.as_ref().unwrap().name()).is_err());
        // A file-size hint must not bypass the file-growth guard.
        let before = std::fs::metadata(directory.file()).unwrap().len();
        let mut hint = capacity as i64 * 4;
        unsafe {
            assert_eq!(ffi::sqlite3_file_control(store.connection.handle(), c"main".as_ptr(),
                ffi::SQLITE_FCNTL_SIZE_HINT, (&mut hint as *mut i64).cast()), ffi::SQLITE_OK);
        }
        assert_eq!(std::fs::metadata(directory.file()).unwrap().len(), before);
        store.apply(&[Mutation::Put { key: 3, value: b"next" }]).unwrap();
        drop(store);
        let store = ByteStore::open_bounded(&directory.file(), bounds, capacity).unwrap();
        assert_eq!(read(&store, 1), Some(original.clone()));
        assert_eq!(read(&store, 2), Some(original));
        assert_eq!(read(&store, 3), Some(b"next".to_vec()));
        assert!(store.peak_file_bytes().unwrap() <= capacity);
    }

    #[test]
    fn rejects_foreign_databases_without_erasing_them() {
        let directory = Directory::new();
        let connection = Connection::open(directory.file()).unwrap();
        connection
            .execute_batch("CREATE TABLE other(value TEXT); INSERT INTO other VALUES('keep');")
            .unwrap();
        drop(connection);
        assert!(matches!(
            ByteStore::open(&directory.file(), limits()),
            Err(Error::Format)
        ));
        let connection = Connection::open(directory.file()).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT value FROM other", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "keep"
        );
    }

    #[test]
    fn crash_child() {
        let Some(path) = std::env::var_os("AFTERGLOW_BYTE_STORE_CRASH_TEST") else {
            return;
        };
        let mut store = ByteStore::open_bounded(Path::new(&path), limits(), 16 * 1024 * 1024).unwrap();
        let transaction = store.connection.transaction().unwrap();
        transaction
            .execute("UPDATE items SET value=?1 WHERE key=1", [b"new".as_slice()])
            .unwrap();
        transaction
            .execute("UPDATE items SET value=?1 WHERE key=2", [b"NEW".as_slice()])
            .unwrap();
        // The connection stays live. Flush dirty pages without a commit.
        let result = unsafe { rusqlite::ffi::sqlite3_db_cacheflush(transaction.handle()) };
        assert_eq!(result, rusqlite::ffi::SQLITE_OK);
        if std::env::var_os("AFTERGLOW_BYTE_STORE_COMMIT_TEST").is_some() {
            transaction.commit().unwrap();
        }
        // No Rust or SQLite destructors run after this exit.
        std::process::exit(77);
    }

    #[test]
    fn process_exit_preserves_the_last_completed_transaction() {
        let directory = Directory::new();
        let mut store = ByteStore::open(&directory.file(), limits()).unwrap();
        store
            .apply(&[
                Mutation::Put {
                    key: 1,
                    value: b"old",
                },
                Mutation::Put {
                    key: 2,
                    value: b"OLD",
                },
            ])
            .unwrap();
        drop(store);
        for commit in [false, true] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "byte_store::tests::crash_child", "--nocapture"])
                .env("AFTERGLOW_BYTE_STORE_CRASH_TEST", directory.file());
            if commit {
                command.env("AFTERGLOW_BYTE_STORE_COMMIT_TEST", "1");
            }
            let status = command.status().unwrap();
            assert_eq!(status.code(), Some(77));
            let store = ByteStore::open_bounded(&directory.file(), limits(), 16 * 1024 * 1024).unwrap();
            assert_eq!(
                read(&store, 1),
                Some(if commit { b"new" } else { b"old" }.to_vec())
            );
            assert_eq!(
                read(&store, 2),
                Some(if commit { b"NEW" } else { b"OLD" }.to_vec())
            );
            assert_eq!(store.stats().unwrap().value_bytes, 6);
        }
    }
}
