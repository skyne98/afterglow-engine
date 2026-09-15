//! Native tile versions and completed-document roots in the generic byte store.
//! Only the root publication changes the recovery document. Tile writeback does not.

use afterglow_storage_worker::byte_store::{ByteStore, Limits, Mutation, Stats};
use maipointo::tile_cache::{Backing, Pixels};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const GIB: u64 = 1024 * 1024 * 1024;
pub const SCRATCH_BYTES: u64 = 64 * GIB;
pub const FREE_RESERVE_BYTES: u64 = 16 * GIB;
const JOURNAL_BYTES: u64 = 2 * GIB;
const MAX_TILES: usize = 8 * 256 * 256;
const MAX_METADATA: usize = 65536;
const MAX_ROOTS: usize = maipointo::app::WEB_HISTORY_RECORDS + 1;
const ROOT_BYTES: usize = 16 + MAX_METADATA + MAX_TILES * 16;
const TILE_BYTES: usize = 32768 + 8;
const STAGED_TILES: usize = 32;
const COUNTER_KEY: u64 = 0;
const HEAD_KEY: u64 = 1;

type Tiles = HashMap<u64, u64>;
type References = HashMap<u64, u32>;

#[derive(Clone, Copy)]
pub struct DiskLimits {
    pub scratch_bytes: u64,
    pub free_reserve_bytes: u64,
}
impl Default for DiskLimits {
    fn default() -> Self {
        Self { scratch_bytes: SCRATCH_BYTES, free_reserve_bytes: FREE_RESERVE_BYTES }
    }
}
impl DiskLimits {
    fn journal_bytes(self) -> u64 { JOURNAL_BYTES.min(self.scratch_bytes / 4) }
    fn database_bytes(self) -> u64 { self.scratch_bytes - self.journal_bytes() }
    fn store_limits(self) -> Limits {
        Limits {
            database_bytes: self.database_bytes(), value_bytes: ROOT_BYTES,
            transaction_operations: STAGED_TILES * 2 + 1,
            transaction_bytes: ROOT_BYTES + 4096, cache_bytes: 8 * 1024 * 1024,
        }
    }
}

struct Snapshot { metadata: Vec<u8>, tiles: Tiles }

/// One owner keeps current tile keys, reference counts, and bounded write staging.
/// Undo roots stay on disk rather than retaining complete pixel copies in RAM.
pub struct PaintStore {
    database: ByteStore,
    path: PathBuf,
    limits: DiskLimits,
    next: u64,
    roots: Vec<u64>,
    cursor: usize,
    working: Tiles,
    completed: Tiles,
    metadata: Vec<u8>,
    references: References,
    root_buffer: Vec<u8>,
    tile_buffer: [u8; TILE_BYTES],
    staging: Vec<u8>,
    staged_keys: [u64; STAGED_TILES],
    staged_count: usize,
    garbage_pending: bool,
    #[cfg(test)]
    fail_read: bool,
    #[cfg(test)]
    pub(super) fail_write: bool,
    // Keep the shared reservation until every map and buffer above is released.
    _memory: afterglow_memory::Reservation,
}

/// Surface access stays on the same owner thread as the SQLite connection.
pub struct PaintBacking(pub Rc<RefCell<PaintStore>>);
impl Backing for PaintBacking {
    fn load(&mut self, key: u64, output: &mut Pixels) -> Result<bool, String> {
        self.0.borrow_mut().load(key, output)
    }
    fn save(&mut self, key: u64, pixels: &Pixels) -> Result<(), String> {
        self.0.borrow_mut().save(key, pixels)
    }
    fn remove(&mut self, key: u64) -> Result<(), String> {
        self.0.borrow_mut().remove(key);
        Ok(())
    }
}

impl PaintStore {
    /// The caller must confine the path to an owned native storage directory.
    pub fn open(path: &Path, limits: DiskLimits) -> Result<Self, String> {
        if limits.scratch_bytes < 65536 || limits.scratch_bytes > SCRATCH_BYTES {
            return Err("incorrect paint scratch capacity".into());
        }
        // Reserve bounded maps, clone/growth overlap, root buffers, staging, and SQLite cache headroom.
        let versions = limits.database_bytes() / TILE_BYTES as u64;
        let bytes = versions * 256 + MAX_TILES as u64 * 512 + 64 * 1024 * 1024;
        let memory = afterglow_memory::process()?.reserve(bytes).ok_or("native paint store memory capacity reached")?;
        let database = ByteStore::open_bounded(path, limits.store_limits(), limits.scratch_bytes).map_err(storage_error)?;
        let mut store = Self {
            database, path: path.to_owned(), limits, next: 2, roots: Vec::new(), cursor: 0,
            working: Tiles::new(), completed: Tiles::new(), metadata: Vec::new(), references: References::new(),
            root_buffer: zeroed(ROOT_BYTES)?, tile_buffer: [0; TILE_BYTES],
            staging: zeroed(STAGED_TILES * TILE_BYTES)?, staged_keys: [0; STAGED_TILES],
            staged_count: 0, garbage_pending: true,
            #[cfg(test)] fail_read: false,
            #[cfg(test)] fail_write: false,
            _memory: memory,
        };
        let mut counter = [0; 16];
        match store.database.get(COUNTER_KEY, &mut counter).map_err(storage_error)? {
            None => {
                if store.database.stats().map_err(storage_error)?.entries != 0 {
                    return Err("missing paint version counter".into());
                }
                store.admit_write()?;
                store.database.apply(&[Mutation::Put { key: COUNTER_KEY, value: &counter_record(2) }]).map_err(storage_error)?;
            }
            Some(16) => {
                let data = payload(&counter, b"APN1")?;
                store.next = word(data, 4)?;
                if !(2..=i64::MAX as u64).contains(&store.next) { return Err("incorrect paint version counter".into()); }
            }
            Some(_) => return Err("incorrect paint version counter size".into()),
        }
        let mut head = [0; 12 + MAX_ROOTS * 8];
        if let Some(size) = store.database.get(HEAD_KEY, &mut head).map_err(storage_error)? {
            let data = payload(&head[..size], b"APH1")?;
            if data.len() < 8 { return Err("short paint recovery head".into()); }
            let count = u16::from_le_bytes(data[4..6].try_into().unwrap()) as usize;
            store.cursor = u16::from_le_bytes(data[6..8].try_into().unwrap()) as usize;
            if count == 0 || count > MAX_ROOTS || store.cursor >= count || data.len() != 8 + count * 8 {
                return Err("incorrect paint recovery head".into());
            }
            store.roots.try_reserve_exact(MAX_ROOTS).map_err(allocation_error)?;
            for index in 0..count {
                let key = word(data, 8 + index * 8)?;
                if key < 2 || key >= store.next || store.roots.contains(&key) {
                    return Err("incorrect paint root key".into());
                }
                store.roots.push(key);
            }
            let maximum_versions = store.maximum_versions();
            for index in 0..count {
                let snapshot = store.read_root(store.roots[index])?;
                add_references(&mut store.references, &snapshot.tiles, maximum_versions)?;
                if index == store.cursor {
                    store.completed = snapshot.tiles;
                    store.metadata = snapshot.metadata;
                }
            }
            store.working = copy_map(&store.completed)?;
            add_references(&mut store.references, &store.working, maximum_versions)?;
        }
        // Orphan removal can wait when disk space is below the reserve.
        // Read-only recovery does not need a successful cleanup transaction.
        Ok(store)
    }

    pub fn has_recovery(&self) -> bool { !self.roots.is_empty() }
    pub fn metadata(&self) -> &[u8] { &self.metadata }
    pub fn tile_keys(&self) -> impl Iterator<Item = u64> + '_ { self.working.keys().copied() }
    pub fn completed_tiles(&self) -> &HashMap<u64, u64> { &self.completed }
    pub fn can_undo(&self) -> bool { self.has_recovery() && self.cursor != 0 }
    pub fn can_redo(&self) -> bool { self.cursor + 1 < self.roots.len() }
    pub fn stats(&self) -> Result<Stats, String> { self.database.stats().map_err(storage_error) }
    pub fn peak_file_bytes(&self) -> u64 { self.database.peak_file_bytes().unwrap_or(0) }

    fn maximum_versions(&self) -> usize {
        (self.limits.database_bytes() / TILE_BYTES as u64) as usize
    }

    fn admit_write(&self) -> Result<(), String> {
        #[cfg(test)]
        if self.fail_write { return Err("injected paint write failure".into()); }
        let parent = self.path.parent().ok_or("paint storage has no directory")?;
        let necessary = self.limits.free_reserve_bytes.checked_add(self.limits.journal_bytes())
            .ok_or("paint disk reserve overflow")?;
        if free_bytes(parent)? < necessary { return Err("paint scratch free-space reserve reached".into()); }
        Ok(())
    }

    pub fn load(&mut self, key: u64, output: &mut Pixels) -> Result<bool, String> {
        #[cfg(test)]
        if self.fail_read { return Err("injected paint read failure".into()); }
        if let Some(index) = self.staged_keys[..self.staged_count].iter().position(|&item| item == key) {
            decode_tile(&self.staging[index * TILE_BYTES..(index + 1) * TILE_BYTES], output)?;
            return Ok(true);
        }
        let Some(&version) = self.working.get(&key) else { return Ok(false); };
        let size = self.database.get(version, &mut self.tile_buffer).map_err(storage_error)?
            .ok_or("missing stored paint tile")?;
        if size != TILE_BYTES { return Err("incorrect stored paint tile size".into()); }
        decode_tile(&self.tile_buffer, output)?;
        Ok(true)
    }

    pub fn save(&mut self, key: u64, pixels: &Pixels) -> Result<(), String> {
        #[cfg(test)]
        if self.fail_write { return Err("injected paint write failure".into()); }
        let existing = self.staged_keys[..self.staged_count].iter().position(|&item| item == key);
        let index = if let Some(index) = existing { index } else {
            if self.staged_count == STAGED_TILES { self.flush()?; }
            let new_keys = self.staged_keys[..self.staged_count].iter().filter(|item| !self.working.contains_key(item)).count();
            if !self.working.contains_key(&key) && self.working.len() + new_keys >= MAX_TILES {
                return Err("paint logical tile capacity exceeded".into());
            }
            let index = self.staged_count;
            self.staged_keys[index] = key;
            self.staged_count += 1;
            index
        };
        encode_tile(pixels, &mut self.staging[index * TILE_BYTES..(index + 1) * TILE_BYTES]);
        Ok(())
    }

    pub fn remove(&mut self, key: u64) {
        if let Some(index) = self.staged_keys[..self.staged_count].iter().position(|&item| item == key) {
            self.staged_count -= 1;
            self.staged_keys[index] = self.staged_keys[self.staged_count];
            self.staging.copy_within(self.staged_count * TILE_BYTES..(self.staged_count + 1) * TILE_BYTES, index * TILE_BYTES);
        }
        if let Some(version) = self.working.remove(&key) {
            remove_reference(&mut self.references, version);
            self.garbage_pending = true;
        }
    }

    /// Save at most 32 staged tiles in one bounded transaction.
    pub fn flush(&mut self) -> Result<(), String> {
        if self.staged_count == 0 { return Ok(()); }
        self.collect_garbage()?;
        self.admit_write()?;
        self.working.try_reserve(self.staged_count).map_err(allocation_error)?;
        self.references.try_reserve(self.staged_count).map_err(allocation_error)?;
        let next = self.next.checked_add(self.staged_count as u64).filter(|&next| next <= i64::MAX as u64)
            .ok_or("paint version capacity exceeded")?;
        let counter = counter_record(next);
        let mut mutations = [const { Mutation::Remove { key: 0 } }; STAGED_TILES * 2 + 1];
        let mut count = 0;
        for index in 0..self.staged_count {
            mutations[count] = Mutation::Put { key: self.next + index as u64,
                value: &self.staging[index * TILE_BYTES..(index + 1) * TILE_BYTES] };
            count += 1;
            if let Some(&previous) = self.working.get(&self.staged_keys[index]) {
                if self.references.get(&previous) == Some(&1) {
                    mutations[count] = Mutation::Remove { key: previous };
                    count += 1;
                }
            }
        }
        mutations[count] = Mutation::Put { key: COUNTER_KEY, value: &counter };
        self.database.apply(&mutations[..count + 1]).map_err(storage_error)?;
        for index in 0..self.staged_count {
            let version = self.next + index as u64;
            if let Some(previous) = self.working.insert(self.staged_keys[index], version) {
                remove_reference(&mut self.references, previous);
            }
            self.references.insert(version, 1);
        }
        self.next = next;
        self.staged_count = 0;
        Ok(())
    }

    fn read_root(&mut self, key: u64) -> Result<Snapshot, String> {
        #[cfg(test)]
        if self.fail_read { return Err("injected paint root read failure".into()); }
        self.root_buffer.resize(ROOT_BYTES, 0);
        let size = self.database.get(key, &mut self.root_buffer).map_err(storage_error)?
            .ok_or("missing paint recovery root")?;
        decode_root(&self.root_buffer[..size], self.next)
    }

    /// Publish one completed change and retain at most 40 undo changes.
    /// All fallible preparation occurs before the head transaction.
    pub fn publish(&mut self, metadata: &[u8]) -> Result<(), String> {
        if metadata.len() > MAX_METADATA { return Err("paint document metadata capacity exceeded".into()); }
        self.flush()?;
        if self.has_recovery() && metadata == self.metadata && self.working == self.completed { return Ok(()); }
        self.collect_garbage()?;
        let completed = copy_map(&self.working)?;
        let saved_metadata = copy_bytes(metadata)?;
        let mut references = copy_map(&self.references)?;
        let mut roots = Vec::new();
        roots.try_reserve_exact(MAX_ROOTS).map_err(allocation_error)?;
        if !self.roots.is_empty() { roots.extend_from_slice(&self.roots[..=self.cursor]); }
        let mut retired = Vec::new();
        retired.try_reserve_exact(MAX_ROOTS).map_err(allocation_error)?;
        retired.extend_from_slice(self.roots.get(self.cursor + 1..).unwrap_or(&[]));
        if roots.len() == MAX_ROOTS { retired.push(roots.remove(0)); }
        for &key in &retired {
            let snapshot = self.read_root(key)?;
            for &version in snapshot.tiles.values() { remove_reference(&mut references, version); }
        }
        add_references(&mut references, &self.working, self.maximum_versions())?;
        let key = self.next;
        let next = key.checked_add(1).filter(|&next| next <= i64::MAX as u64).ok_or("paint version capacity exceeded")?;
        roots.push(key);
        let cursor = roots.len() - 1;
        let head = encode_head(&roots, cursor);
        let counter = counter_record(next);
        encode_root(&self.working, metadata, &mut self.root_buffer);
        self.admit_write()?;
        self.database.apply(&[
            Mutation::Put { key, value: &self.root_buffer },
            Mutation::Put { key: COUNTER_KEY, value: &counter },
            Mutation::Put { key: HEAD_KEY, value: &head },
        ]).map_err(storage_error)?;
        self.roots = roots;
        self.cursor = cursor;
        self.completed = completed;
        self.metadata = saved_metadata;
        self.references = references;
        self.next = next;
        self.garbage_pending |= !retired.is_empty();
        Ok(())
    }

    /// Replace the document only after an explicit New or Discard decision.
    /// Keep the previous recovery head until the empty root transaction succeeds.
    pub fn replace(&mut self, metadata: &[u8]) -> Result<(), String> {
        if metadata.len() > MAX_METADATA { return Err("paint document metadata capacity exceeded".into()); }
        let saved_metadata = copy_bytes(metadata)?;
        let key = self.next;
        let next = key.checked_add(1).filter(|&next| next <= i64::MAX as u64)
            .ok_or("paint version capacity exceeded")?;
        let mut roots = Vec::new();
        roots.try_reserve_exact(MAX_ROOTS).map_err(allocation_error)?;
        roots.push(key);
        let head = encode_head(&roots, 0);
        let counter = counter_record(next);
        encode_root(&Tiles::new(), metadata, &mut self.root_buffer);
        self.admit_write()?;
        self.database.apply(&[
            Mutation::Put { key, value: &self.root_buffer },
            Mutation::Put { key: COUNTER_KEY, value: &counter },
            Mutation::Put { key: HEAD_KEY, value: &head },
        ]).map_err(storage_error)?;
        self.roots = roots;
        self.cursor = 0;
        self.metadata = saved_metadata;
        self.next = next;
        self.working.clear();
        self.completed.clear();
        self.references.clear();
        self.staged_count = 0;
        self.garbage_pending = true;
        Ok(())
    }

    /// Select a completed root. The recovery head follows undo and redo.
    pub fn select(&mut self, delta: i32) -> Result<bool, String> {
        self.select_prepared(delta, |_, _| Ok(())).map(|result| result.is_some())
    }

    /// Prepare the consumer before the transaction changes the recovery head.
    pub fn select_prepared<T>(&mut self, delta: i32,
        prepare: impl FnOnce(&[u8], &HashMap<u64, u64>) -> Result<T, String>) -> Result<Option<T>, String> {
        if !matches!(delta, -1 | 1) { return Err("paint history direction must be -1 or 1".into()); }
        let target = self.cursor as i32 + delta;
        if target < 0 || target as usize >= self.roots.len() { return Ok(None); }
        let target = target as usize;
        let snapshot = self.read_root(self.roots[target])?;
        let prepared = prepare(&snapshot.metadata, &snapshot.tiles)?;
        let working = copy_map(&snapshot.tiles)?;
        let mut references = copy_map(&self.references)?;
        for &version in self.working.values() { remove_reference(&mut references, version); }
        add_references(&mut references, &working, self.maximum_versions())?;
        let head = encode_head(&self.roots, target);
        self.admit_write()?;
        self.database.apply(&[Mutation::Put { key: HEAD_KEY, value: &head }]).map_err(storage_error)?;
        self.completed = snapshot.tiles;
        self.metadata = snapshot.metadata;
        self.working = working;
        self.references = references;
        self.staged_count = 0;
        self.cursor = target;
        self.garbage_pending = true;
        Ok(Some(prepared))
    }

    /// Cancel working versions without an I/O operation on the recovery head.
    pub fn rollback(&mut self) -> Result<(), String> {
        let working = copy_map(&self.completed)?;
        let mut references = copy_map(&self.references)?;
        for &version in self.working.values() { remove_reference(&mut references, version); }
        add_references(&mut references, &working, self.maximum_versions())?;
        self.working = working;
        self.references = references;
        self.staged_count = 0;
        self.garbage_pending = true;
        Ok(())
    }

    /// Retire only values absent from all live roots and the working document.
    /// A failed cleanup leaves the recovery head and all referenced values intact.
    pub fn collect_garbage(&mut self) -> Result<(), String> {
        if !self.garbage_pending { return Ok(()); }
        let mut after = Some(HEAD_KEY);
        let mut keys = [0; 64];
        loop {
            let count = self.database.keys_after(after, &mut keys).map_err(storage_error)?;
            if count == 0 { break; }
            let mut mutations = [const { Mutation::Remove { key: 0 } }; 64];
            let mut removed = 0;
            for &key in &keys[..count] {
                if !self.references.contains_key(&key) && !self.roots.contains(&key) {
                    mutations[removed] = Mutation::Remove { key };
                    removed += 1;
                }
            }
            if removed != 0 {
                self.admit_write()?;
                self.database.apply(&mutations[..removed]).map_err(storage_error)?;
            }
            after = Some(keys[count - 1]);
        }
        self.garbage_pending = false;
        Ok(())
    }
}

fn allocation_error(_: impl std::fmt::Display) -> String { "paint storage metadata allocation failed".into() }
fn storage_error(error: impl std::fmt::Display) -> String { format!("paint scratch storage: {error}") }
fn zeroed(length: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(allocation_error)?;
    bytes.resize(length, 0);
    Ok(bytes)
}
fn copy_bytes(source: &[u8]) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(source.len()).map_err(allocation_error)?;
    bytes.extend_from_slice(source);
    Ok(bytes)
}
fn copy_map<V: Copy>(source: &HashMap<u64, V>) -> Result<HashMap<u64, V>, String> {
    let mut result = HashMap::new();
    result.try_reserve(source.len()).map_err(allocation_error)?;
    result.extend(source.iter().map(|(&key, &value)| (key, value)));
    Ok(result)
}
fn add_references(references: &mut References, tiles: &Tiles, maximum: usize) -> Result<(), String> {
    for &version in tiles.values() {
        if !references.contains_key(&version) {
            if references.len() == maximum { return Err("paint version index capacity exceeded".into()); }
            references.try_reserve(1).map_err(allocation_error)?;
        }
        let count = references.entry(version).or_default();
        *count = count.checked_add(1).ok_or("paint version reference overflow")?;
    }
    Ok(())
}
fn remove_reference(references: &mut References, version: u64) {
    if let Some(count) = references.get_mut(&version) {
        *count -= 1;
        if *count == 0 { references.remove(&version); }
    }
}
fn checksum(bytes: &mut Vec<u8>) { bytes.extend_from_slice(&crc32fast::hash(bytes).to_le_bytes()); }
fn payload<'a>(bytes: &'a [u8], magic: &[u8; 4]) -> Result<&'a [u8], String> {
    if bytes.len() < 8 || &bytes[..4] != magic { return Err("incorrect paint storage record".into()); }
    let size = bytes.len() - 4;
    if crc32fast::hash(&bytes[..size]) != u32::from_le_bytes(bytes[size..].try_into().unwrap()) {
        return Err("paint storage checksum mismatch".into());
    }
    Ok(&bytes[..size])
}
fn word(bytes: &[u8], offset: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(bytes.get(offset..offset + 8).ok_or("short paint storage record")?.try_into().unwrap()))
}
fn counter_record(next: u64) -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes[..4].copy_from_slice(b"APN1");
    bytes[4..12].copy_from_slice(&next.to_le_bytes());
    let crc = crc32fast::hash(&bytes[..12]);
    bytes[12..].copy_from_slice(&crc.to_le_bytes());
    bytes
}
fn encode_head(roots: &[u64], cursor: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(12 + MAX_ROOTS * 8);
    bytes.extend_from_slice(b"APH1");
    bytes.extend_from_slice(&(roots.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(cursor as u16).to_le_bytes());
    for key in roots { bytes.extend_from_slice(&key.to_le_bytes()); }
    checksum(&mut bytes);
    bytes
}
fn encode_tile(pixels: &Pixels, output: &mut [u8]) {
    output[..4].copy_from_slice(b"APT1");
    for (&pixel, bytes) in pixels.iter().zip(output[4..TILE_BYTES - 4].chunks_exact_mut(2)) {
        bytes.copy_from_slice(&pixel.to_le_bytes());
    }
    let crc = crc32fast::hash(&output[..TILE_BYTES - 4]);
    output[TILE_BYTES - 4..].copy_from_slice(&crc.to_le_bytes());
}
fn decode_tile(bytes: &[u8], output: &mut Pixels) -> Result<(), String> {
    if bytes.len() != TILE_BYTES { return Err("incorrect paint tile record size".into()); }
    let data = payload(bytes, b"APT1")?;
    for (pixel, bytes) in output.iter_mut().zip(data[4..].chunks_exact(2)) {
        *pixel = u16::from_le_bytes(bytes.try_into().unwrap());
    }
    Ok(())
}
fn encode_root(tiles: &Tiles, metadata: &[u8], output: &mut Vec<u8>) {
    // ponytail: at most 524,288 tile entries. Use incremental roots if the commit latency gate fails.
    output.clear();
    output.extend_from_slice(b"APR1");
    output.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    output.extend_from_slice(&(tiles.len() as u32).to_le_bytes());
    output.extend_from_slice(metadata);
    for (key, version) in tiles {
        output.extend_from_slice(&key.to_le_bytes());
        output.extend_from_slice(&version.to_le_bytes());
    }
    checksum(output);
}
fn decode_root(bytes: &[u8], next: u64) -> Result<Snapshot, String> {
    let data = payload(bytes, b"APR1")?;
    if data.len() < 12 { return Err("short paint root".into()); }
    let metadata_bytes = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let count = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    if metadata_bytes > MAX_METADATA || count > MAX_TILES || data.len() != 12 + metadata_bytes + count * 16 {
        return Err("incorrect paint root size".into());
    }
    let metadata = copy_bytes(&data[12..12 + metadata_bytes])?;
    let mut tiles = Tiles::new();
    tiles.try_reserve(count).map_err(allocation_error)?;
    for index in 0..count {
        let offset = 12 + metadata_bytes + index * 16;
        let key = word(data, offset)?;
        let version = word(data, offset + 8)?;
        if version < 2 || version >= next || tiles.insert(key, version).is_some() {
            return Err("incorrect paint root tile reference".into());
        }
    }
    Ok(Snapshot { metadata, tiles })
}

fn free_bytes(path: &Path) -> Result<u64, String> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(storage_error)?;
        let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
        // The path and output structure stay live for the system call.
        if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 { return Err(storage_error(std::io::Error::last_os_error())); }
        return (stats.f_bavail as u64).checked_mul(stats.f_frsize as u64).ok_or("paint free-space size overflow".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0u64;
        // Windows writes only to the supplied live output value.
        if unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(path.as_ptr(), &mut available,
            std::ptr::null_mut(), std::ptr::null_mut()) } == 0 { return Err(storage_error(std::io::Error::last_os_error())); }
        return Ok(available);
    }
    #[allow(unreachable_code)]
    Err("paint free-space query is unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use maipointo::tile_cache::TileCache;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("afterglow-paint-store-{}-{}",
                std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn file(&self) -> PathBuf { self.0.join("paint.sqlite") }
    }
    impl Drop for Directory {
        fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
    }
    fn limits() -> DiskLimits { DiskLimits { scratch_bytes: 128 * 1024 * 1024, free_reserve_bytes: 0 } }
    fn open(directory: &Directory) -> PaintStore { PaintStore::open(&directory.file(), limits()).unwrap() }
    fn cache(store: &Rc<RefCell<PaintStore>>) -> TileCache {
        TileCache::new(Box::new(PaintBacking(store.clone())), 1, 2, 1).unwrap()
    }
    fn read(store: &mut PaintStore, key: u64) -> Pixels {
        let mut pixels = [0; 64 * 64 * 4];
        assert!(store.load(key, &mut pixels).unwrap());
        pixels
    }
    fn save(store: &mut PaintStore, key: u64, value: u16) { store.save(key, &[value; 64 * 64 * 4]).unwrap(); }

    #[test]
    fn sqlite_paging_retains_exact_pixels_after_growth_eviction_and_reopen() {
        let directory = Directory::new();
        let store = Rc::new(RefCell::new(open(&directory)));
        let cache = cache(&store);
        for key in 0..200 {
            let mut tile = cache.write(key).unwrap();
            for (index, pixel) in tile.iter_mut().enumerate() { *pixel = (key * 131 ^ index as u64) as u16; }
        }
        assert_eq!(cache.resident_tiles(), 2);
        assert_eq!(cache.admitted_tiles(), 2);
        cache.flush().unwrap();
        store.borrow_mut().publish(b"document with layers").unwrap();
        drop(cache);
        drop(store);
        let mut store = open(&directory);
        assert_eq!(store.metadata(), b"document with layers");
        for key in 0..200 {
            for (index, pixel) in read(&mut store, key).iter().enumerate() {
                assert_eq!(*pixel, (key * 131 ^ index as u64) as u16);
            }
        }
    }

    #[test]
    fn document_metadata_and_layer_pixels_recover_as_one_completed_root() {
        use maipointo::app::PaintApp;
        use maipointo::compositor::BlendMode;
        let directory = Directory::new();
        let store = Rc::new(RefCell::new(open(&directory)));
        let resident = cache(&store);
        let mut app = PaintApp::new_with_tile_limits(256, 128, 1, 2).unwrap();
        assert!(app.set_tile_cache(resident.clone()));
        app.create_layer();
        let group = app.create_group();
        assert!(app.set_layer_group(1, group));
        app.group_pass_through[group as usize] = true;
        app.group_isolated[group as usize] = false;
        app.group_opacity[group as usize] = 0.75;
        app.set_layer_mode(0, BlendMode::Normal);
        app.set_layer_mode(1, BlendMode::Normal);
        for layer in 0..2 {
            for ty in 0..2 {
                for tx in 0..4 {
                    app.layers[layer].get_or_create_tile_mut(tx, ty).unwrap()
                        .fill((9000 + layer * 1000 + ty as usize * 100 + tx as usize * 10) as u16);
                }
            }
        }
        let mut expected = Vec::new();
        for ty in 0..2 { for tx in 0..4 { expected.push(app.render_rgba8_tile(tx, ty).to_vec()); } }
        let metadata = app.document_metadata().unwrap();
        resident.flush().unwrap();
        store.borrow_mut().publish(&serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert_eq!(resident.resident_tiles(), 2);
        drop(app);
        drop(resident);
        drop(store);

        let store = Rc::new(RefCell::new(open(&directory)));
        let resident = cache(&store);
        let metadata: serde_json::Value = serde_json::from_slice(store.borrow().metadata()).unwrap();
        let mut restored = PaintApp::from_document_metadata(&metadata, 1, 2).unwrap();
        assert!(restored.set_tile_cache(resident.clone()));
        for key in store.borrow().tile_keys() {
            let id = (key >> 32) as u32;
            let layer = restored.layers.iter_mut().find(|layer| layer.storage_id() == id).unwrap();
            assert!(layer.register_stored_tile((key & 65535) as i32, ((key >> 16) & 65535) as i32));
        }
        assert_eq!(resident.resident_tiles(), 0);
        assert_eq!(restored.document_metadata().unwrap(), metadata);
        for ty in 0..2 { for tx in 0..4 {
            assert_eq!(restored.render_rgba8_tile(tx, ty), expected[(ty * 4 + tx) as usize]);
        } }
        assert!(!restored.take_storage_error());
        assert_eq!(resident.resident_tiles(), 2);
    }

    #[test]
    fn history_exceeds_the_old_capture_budget_with_two_resident_tiles() {
        let directory = Directory::new();
        let store = Rc::new(RefCell::new(open(&directory)));
        let cache = cache(&store);
        for value in [7, 19] {
            for key in 0..1100 { cache.write(key).unwrap().fill(value); }
            cache.flush().unwrap();
            store.borrow_mut().publish(&[value as u8]).unwrap();
        }
        assert_eq!(cache.resident_tiles(), 2);
        assert!(store.borrow_mut().select(-1).unwrap());
        cache.reset().unwrap();
        for key in 0..1100 { assert!(cache.read(key).unwrap().iter().all(|&pixel| pixel == 7)); }
        assert!(store.borrow_mut().select(1).unwrap());
        cache.reset().unwrap();
        for key in 0..1100 { assert!(cache.read(key).unwrap().iter().all(|&pixel| pixel == 19)); }
        assert!(store.borrow().stats().unwrap().value_bytes > 64 * 1024 * 1024);
    }

    #[test]
    fn history_and_metadata_survive_reopen_and_retired_roots_have_bounded_storage() {
        let directory = Directory::new();
        let mut store = open(&directory);
        for value in 0..70 {
            save(&mut store, 1, value);
            store.publish(&value.to_le_bytes()).unwrap();
        }
        store.collect_garbage().unwrap();
        let stats = store.stats().unwrap();
        assert_eq!(store.roots.len(), MAX_ROOTS);
        assert_eq!(stats.entries, 2 + MAX_ROOTS as u64 * 2);
        assert!(stats.database_bytes < 4 * 1024 * 1024);
        assert!(store.select(-1).unwrap());
        drop(store);
        let mut store = open(&directory);
        assert_eq!(read(&mut store, 1)[0], 68);
        assert_eq!(store.metadata(), &68u16.to_le_bytes());
        assert!(store.can_redo());
        assert!(store.select(1).unwrap());
        assert_eq!(read(&mut store, 1)[0], 69);
        assert!(store.select(-1).unwrap());
        store.remove(1);
        save(&mut store, 2, 99);
        store.publish(b"new branch").unwrap();
        assert!(!store.can_redo());
        store.collect_garbage().unwrap();
        assert!(store.select(-1).unwrap());
        assert_eq!(read(&mut store, 1)[0], 68);
        assert!(!store.load(2, &mut [0; 16384]).unwrap());
        assert!(store.select(1).unwrap());
        assert_eq!(read(&mut store, 2)[0], 99);
        assert!(!store.load(1, &mut [0; 16384]).unwrap());
    }

    #[test]
    fn paging_and_commit_failures_allow_rollback_and_the_next_stroke() {
        let directory = Directory::new();
        let store = Rc::new(RefCell::new(open(&directory)));
        let cache = cache(&store);
        cache.write(1).unwrap().fill(17);
        cache.flush().unwrap();
        store.borrow_mut().publish(b"keep").unwrap();
        cache.write(1).unwrap().fill(42);
        cache.flush().unwrap();
        store.borrow_mut().fail_write = true;
        assert!(store.borrow_mut().publish(b"lost").is_err());
        store.borrow_mut().rollback().unwrap();
        cache.reset().unwrap();
        assert_eq!(cache.read(1).unwrap()[0], 17);
        assert_eq!(store.borrow().metadata(), b"keep");
        store.borrow_mut().fail_write = false;
        cache.write(1).unwrap().fill(23);
        cache.flush().unwrap();
        store.borrow_mut().publish(b"next").unwrap();
        assert_eq!(store.borrow().metadata(), b"next");
        assert!(store.borrow_mut().select(-1).unwrap());
        cache.reset().unwrap();
        assert_eq!(cache.read(1).unwrap()[0], 17);
    }

    #[test]
    fn full_database_and_free_space_reserve_retain_the_completed_root() {
        let directory = Directory::new();
        let limits = DiskLimits { scratch_bytes: 512 * 1024, free_reserve_bytes: 0 };
        let mut store = PaintStore::open(&directory.file(), limits).unwrap();
        save(&mut store, 1, 11);
        store.publish(b"keep").unwrap();
        for key in 2..34 { save(&mut store, key, 33); }
        assert!(store.flush().is_err());
        store.rollback().unwrap();
        assert_eq!(store.metadata(), b"keep");
        assert_eq!(read(&mut store, 1)[0], 11);
        save(&mut store, 1, 22);
        store.publish(b"after full").unwrap();
        store.limits.free_reserve_bytes = u64::MAX;
        save(&mut store, 1, 44);
        assert!(store.publish(b"below reserve").is_err());
        store.rollback().unwrap();
        assert_eq!(read(&mut store, 1)[0], 22);
        store.limits = limits;
        save(&mut store, 1, 55);
        store.publish(b"after reserve").unwrap();
        assert_eq!(read(&mut store, 1)[0], 55);
    }

    #[test]
    fn missing_and_corrupt_tiles_never_become_transparent_successes() {
        let directory = Directory::new();
        let mut store = open(&directory);
        save(&mut store, 1, 55);
        store.publish(b"keep").unwrap();
        let version = store.working[&1];
        let mut bytes = vec![0; TILE_BYTES];
        store.database.get(version, &mut bytes).unwrap();
        bytes[7] ^= 1;
        store.database.apply(&[Mutation::Put { key: version, value: &bytes }]).unwrap();
        let mut output = [77; 16384];
        assert!(store.load(1, &mut output).is_err());
        assert!(output.iter().all(|&pixel| pixel == 77));
        store.database.apply(&[Mutation::Remove { key: version }]).unwrap();
        assert!(store.load(1, &mut output).is_err());
        assert!(output.iter().all(|&pixel| pixel == 77));
        store.fail_read = true;
        assert!(store.load(2, &mut output).is_err());
    }

    #[test]
    fn root_decoder_rejects_partial_duplicate_and_future_records() {
        let mut bytes = Vec::new();
        let tiles = Tiles::from([(11, 2), (12, 3)]);
        encode_root(&tiles, b"metadata", &mut bytes);
        assert_eq!(decode_root(&bytes, 4).unwrap().tiles, tiles);
        for length in 0..bytes.len() { assert!(decode_root(&bytes[..length], 4).is_err()); }
        assert!(decode_root(&bytes, 3).is_err());
        let offset = 12 + b"metadata".len();
        let first = bytes[offset..offset + 8].to_vec();
        bytes[offset + 16..offset + 24].copy_from_slice(&first);
        bytes.truncate(bytes.len() - 4);
        checksum(&mut bytes);
        assert!(decode_root(&bytes, 4).is_err());
    }

    #[test]
    fn crash_child() {
        let Some(path) = std::env::var_os("AFTERGLOW_PAINT_STORE_CRASH_TEST") else { return; };
        let mut store = PaintStore::open(Path::new(&path), limits()).unwrap();
        save(&mut store, 1, 2);
        save(&mut store, 2, 2);
        store.flush().unwrap();
        if std::env::var_os("AFTERGLOW_PAINT_STORE_COMMIT_TEST").is_some() { store.publish(b"new").unwrap(); }
        // No destructors run. Written tile versions alone cannot publish a root.
        std::process::exit(77);
    }

    #[test]
    fn process_exit_recovers_only_the_last_published_root() {
        let directory = Directory::new();
        let mut store = open(&directory);
        save(&mut store, 1, 1);
        save(&mut store, 2, 1);
        store.publish(b"old").unwrap();
        drop(store);
        for commit in [false, true] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "paged_store::tests::crash_child", "--nocapture"])
                .env("AFTERGLOW_PAINT_STORE_CRASH_TEST", directory.file());
            if commit { command.env("AFTERGLOW_PAINT_STORE_COMMIT_TEST", "1"); }
            assert_eq!(command.status().unwrap().code(), Some(77));
            let mut store = open(&directory);
            let value = if commit { 2 } else { 1 };
            assert_eq!(read(&mut store, 1)[0], value);
            assert_eq!(read(&mut store, 2)[0], value);
            assert_eq!(store.metadata(), if commit { b"new" } else { b"old" });
            store.collect_garbage().unwrap();
            assert_eq!(store.stats().unwrap().entries, if commit { 8 } else { 5 });
        }
    }
}
