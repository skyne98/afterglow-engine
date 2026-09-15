//! Shared resident tiles with explicit pins and a bounded LRU.
//! Storage errors do not discard dirty data. A pin prevents eviction.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

pub type Pixels = [u16; 64 * 64 * 4];
const NONE: usize = usize::MAX;

pub trait Backing {
    /// A missing key means a transparent tile. Never return partial data on success.
    fn load(&mut self, key: u64, output: &mut Pixels) -> Result<bool, String>;
    fn save(&mut self, key: u64, pixels: &Pixels) -> Result<(), String>;
    fn remove(&mut self, key: u64) -> Result<(), String>;
}

struct Entry {
    key: u64,
    pixels: Option<Rc<Box<Pixels>>>,
    pins: u32,
    dirty: bool,
    previous: usize,
    next: usize,
    #[cfg(not(target_arch = "wasm32"))]
    _pixels_memory: Option<afterglow_memory::Reservation>,
}

struct Inner {
    backing: Box<dyn Backing>,
    entries: Vec<Entry>,
    index: HashMap<u64, usize>,
    head: usize,
    tail: usize,
    maximum: usize,
    growth: usize,
    admitted: usize,
    #[cfg(not(target_arch = "wasm32"))]
    memory: Option<std::sync::Arc<afterglow_memory::EngineMemory>>,
    #[cfg(not(target_arch = "wasm32"))]
    _metadata_memory: Option<afterglow_memory::Reservation>,
}

/// One RPC owner can share this cache across all document layers.
#[derive(Clone)]
pub struct TileCache(Rc<RefCell<Inner>>);

pub struct ReadPin {
    cache: TileCache,
    slot: usize,
    pixels: Rc<Box<Pixels>>,
}

pub struct WritePin {
    cache: TileCache,
    slot: usize,
    pixels: Option<Rc<Box<Pixels>>>,
}

impl Inner {
    fn unlink(&mut self, slot: usize) {
        let (previous, next) = (self.entries[slot].previous, self.entries[slot].next);
        if previous == NONE { self.head = next; } else { self.entries[previous].next = next; }
        if next == NONE { self.tail = previous; } else { self.entries[next].previous = previous; }
        self.entries[slot].previous = NONE;
        self.entries[slot].next = NONE;
    }

    fn link_head(&mut self, slot: usize) {
        self.entries[slot].previous = NONE;
        self.entries[slot].next = self.head;
        if self.head == NONE { self.tail = slot; } else { self.entries[self.head].previous = slot; }
        self.head = slot;
    }

    fn save(&mut self, slot: usize) -> Result<(), String> {
        let entry = &self.entries[slot];
        if entry.pins != 0 { return Err("paint tile is pinned".into()); }
        if entry.dirty {
            self.backing.save(entry.key, entry.pixels.as_ref().ok_or("missing paint tile")?)?;
            self.entries[slot].dirty = false;
        }
        Ok(())
    }

    fn ensure(&mut self, key: u64) -> Result<usize, String> {
        if let Some(&slot) = self.index.get(&key) { return Ok(slot); }
        if self.entries.len() == self.admitted && self.admitted < self.maximum {
            self.admitted = self.admitted.saturating_add(self.growth).min(self.maximum);
        }
        let grow = self.entries.len() < self.admitted;
        #[cfg(not(target_arch = "wasm32"))]
        let pixels_memory = if grow { self.memory.as_ref().and_then(|memory| memory.reserve(std::mem::size_of::<Pixels>() as u64 + 32)) } else { None };
        #[cfg(not(target_arch = "wasm32"))]
        let grow = grow && (self.memory.is_none() || pixels_memory.is_some());
        let slot = if grow {
            self.entries.try_reserve(1).map_err(|_| "paint cache metadata allocation failed")?;
            self.index.try_reserve(1).map_err(|_| "paint cache index allocation failed")?;
            let pixels = Box::try_new([0; 64 * 64 * 4]).map_err(|_| "paint cache pixel allocation failed")?;
            let slot = self.entries.len();
            let pixels = Rc::try_new(pixels).map_err(|_| "paint cache pin allocation failed")?;
            self.entries.push(Entry { key, pixels: Some(pixels), pins: 0,
                dirty: false, previous: NONE, next: NONE,
                #[cfg(not(target_arch = "wasm32"))]
                _pixels_memory: pixels_memory,
            });
            self.link_head(slot);
            slot
        } else {
            let slot = self.tail;
            if slot == NONE { return Err("paint cache has no unpinned tile".into()); }
            // Keep both the pixels and key if writeback fails.
            self.save(slot)?;
            let previous_key = self.entries[slot].key;
            if self.index.get(&previous_key) == Some(&slot) { self.index.remove(&previous_key); }
            slot
        };
        let entry = &mut self.entries[slot];
        let pixels = Rc::get_mut(entry.pixels.as_mut().ok_or("missing paint cache pixels")?)
            .ok_or("paint cache pin mismatch")?;
        pixels.fill(0);
        let loaded = self.backing.load(key, pixels);
        entry.key = key;
        entry.dirty = false;
        match loaded {
            Ok(_) => { self.index.insert(key, slot); Ok(slot) }
            Err(error) => {
                // This slot stays reusable, but no key can expose partial input.
                Err(error)
            }
        }
    }

    fn pin(&mut self, slot: usize) -> Result<(), String> {
        let pins = self.entries[slot].pins.checked_add(1).ok_or("paint tile pin overflow")?;
        if pins == 1 { self.unlink(slot); }
        self.entries[slot].pins = pins;
        Ok(())
    }

    fn unpin(&mut self, slot: usize) {
        self.entries[slot].pins -= 1;
        if self.entries[slot].pins == 0 { self.link_head(slot); }
    }
}

impl TileCache {
    pub fn new(backing: Box<dyn Backing>, initial: usize, maximum: usize, growth: usize) -> Result<Self, String> {
        if initial == 0 || maximum < initial || growth == 0 { return Err("invalid paint cache capacity".into()); }
        Ok(Self(Rc::new(RefCell::new(Inner { backing, entries: Vec::new(), index: HashMap::new(),
            head: NONE, tail: NONE, maximum, growth, admitted: initial,
            #[cfg(not(target_arch = "wasm32"))]
            memory: None,
            #[cfg(not(target_arch = "wasm32"))]
            _metadata_memory: None,
        }))))
    }

    /// Admit metadata before resident allocation. Shared pressure causes LRU reuse.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_memory(&self, memory: std::sync::Arc<afterglow_memory::EngineMemory>, owner_bytes: u64) -> Result<(), String> {
        let mut inner = self.0.borrow_mut();
        if !inner.entries.is_empty() || inner.memory.is_some() { return Err("paint memory is already active".into()); }
        let bytes = (inner.maximum as u64).checked_mul(512).and_then(|bytes| bytes.checked_add(owner_bytes))
            .ok_or("paint metadata memory overflow")?;
        inner._metadata_memory = Some(memory.reserve(bytes).ok_or("native paint memory capacity reached")?);
        inner.memory = Some(memory);
        Ok(())
    }

    pub fn read(&self, key: u64) -> Result<ReadPin, String> {
        let mut inner = self.0.borrow_mut();
        let slot = inner.ensure(key)?;
        let pixels = inner.entries[slot].pixels.as_ref().ok_or("paint tile has a write pin")?.clone();
        inner.pin(slot)?;
        Ok(ReadPin { cache: self.clone(), slot, pixels })
    }

    pub fn write(&self, key: u64) -> Result<WritePin, String> {
        let mut inner = self.0.borrow_mut();
        let slot = inner.ensure(key)?;
        if inner.entries[slot].pins != 0 { return Err("paint tile already has a pin".into()); }
        inner.pin(slot)?;
        let pixels = inner.entries[slot].pixels.take();
        Ok(WritePin { cache: self.clone(), slot, pixels })
    }

    pub fn remove(&self, key: u64) -> Result<(), String> {
        let mut inner = self.0.borrow_mut();
        let slot = inner.index.get(&key).copied();
        if slot.is_some_and(|slot| inner.entries[slot].pins != 0) { return Err("paint tile has live pins".into()); }
        inner.backing.remove(key)?;
        if let Some(slot) = slot {
            inner.entries[slot].dirty = false;
            inner.index.remove(&key);
        }
        Ok(())
    }

    /// A completed stroke can publish a root only after all dirty tiles are saved.
    /// This cold operation visits at most the configured resident capacity.
    pub fn flush(&self) -> Result<(), String> {
        let mut inner = self.0.borrow_mut();
        for slot in 0..inner.entries.len() { inner.save(slot)?; }
        Ok(())
    }

    /// Discard working pixels after the owner selects a completed storage root.
    /// Refuse to invalidate a live read or tile job.
    pub fn reset(&self) -> Result<(), String> {
        let mut inner = self.0.borrow_mut();
        if inner.entries.iter().any(|entry| entry.pins != 0) { return Err("paint cache has live pins".into()); }
        inner.entries.clear();
        inner.index.clear();
        inner.head = NONE;
        inner.tail = NONE;
        Ok(())
    }

    pub fn resident_tiles(&self) -> usize { self.0.borrow().entries.len() }
    pub fn admitted_tiles(&self) -> usize { self.0.borrow().admitted }
    pub fn maximum_tiles(&self) -> usize { self.0.borrow().maximum }
}

pub(crate) enum TileStorage {
    Heap(Box<Pixels>),
    Paged { cache: TileCache, key: u64 },
}

pub enum TileRead<'a> { Heap(&'a Pixels), Paged(ReadPin) }
pub enum TileWrite<'a> { Heap(&'a mut Pixels), Paged(WritePin) }

impl TileStorage {
    pub fn read(&self) -> Result<TileRead<'_>, String> {
        match self {
            Self::Heap(pixels) => Ok(TileRead::Heap(pixels)),
            Self::Paged { cache, key } => cache.read(*key).map(TileRead::Paged),
        }
    }
    pub fn write(&mut self) -> Result<TileWrite<'_>, String> {
        match self {
            Self::Heap(pixels) => Ok(TileWrite::Heap(pixels)),
            Self::Paged { cache, key } => cache.write(*key).map(TileWrite::Paged),
        }
    }
}
impl Deref for TileRead<'_> {
    type Target = Pixels;
    fn deref(&self) -> &Pixels {
        match self { Self::Heap(pixels) => pixels, Self::Paged(pin) => pin }
    }
}
impl AsRef<Pixels> for TileRead<'_> { fn as_ref(&self) -> &Pixels { self } }
impl Deref for TileWrite<'_> {
    type Target = Pixels;
    fn deref(&self) -> &Pixels {
        match self { Self::Heap(pixels) => pixels, Self::Paged(pin) => pin }
    }
}
impl DerefMut for TileWrite<'_> {
    fn deref_mut(&mut self) -> &mut Pixels {
        match self { Self::Heap(pixels) => pixels, Self::Paged(pin) => pin }
    }
}

impl Deref for ReadPin {
    type Target = Pixels;
    fn deref(&self) -> &Pixels { &self.pixels }
}
impl Drop for ReadPin {
    fn drop(&mut self) { self.cache.0.borrow_mut().unpin(self.slot); }
}
impl Deref for WritePin {
    type Target = Pixels;
    fn deref(&self) -> &Pixels { self.pixels.as_ref().expect("live write pin") }
}
impl DerefMut for WritePin {
    fn deref_mut(&mut self) -> &mut Pixels {
        Rc::get_mut(self.pixels.as_mut().expect("live write pin")).expect("exclusive write pin")
    }
}
impl Drop for WritePin {
    fn drop(&mut self) {
        let mut inner = self.cache.0.borrow_mut();
        inner.entries[self.slot].pixels = self.pixels.take();
        inner.entries[self.slot].dirty = true;
        inner.unpin(self.slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn shared_memory_pressure_reuses_unpinned_tiles() {
        let memory = afterglow_memory::EngineMemory::new(8 * 512 + 2 * (32768 + 32));
        let storage = Rc::new(RefCell::new(Storage::default()));
        let cache = TileCache::new(Box::new(Store(storage)), 1, 8, 1).unwrap();
        cache.set_memory(memory.clone(), 0).unwrap();
        cache.write(1).unwrap()[0] = 101;
        cache.write(2).unwrap()[0] = 202;
        let first = cache.read(1).unwrap();
        let second = cache.read(2).unwrap();
        assert!(cache.write(3).is_err());
        assert_eq!((first[0], second[0]), (101, 202));
        drop(first); drop(second);
        cache.write(3).unwrap()[0] = 303;
        assert_eq!(cache.resident_tiles(), 2);
        assert_eq!(cache.read(1).unwrap()[0], 101);
        assert!(memory.stats().used <= memory.stats().limit);
        drop(cache);
        assert_eq!(memory.stats().used, 0);
    }

    #[derive(Default)]
    struct Storage { values: HashMap<u64, Box<Pixels>>, fail_load: bool, fail_save: bool }
    struct Store(Rc<RefCell<Storage>>);
    impl Backing for Store {
        fn load(&mut self, key: u64, output: &mut Pixels) -> Result<bool, String> {
            let store = self.0.borrow();
            if store.fail_load { return Err("injected read failure".into()); }
            if let Some(value) = store.values.get(&key) { output.copy_from_slice(&value[..]); Ok(true) }
            else { Ok(false) }
        }
        fn save(&mut self, key: u64, pixels: &Pixels) -> Result<(), String> {
            let mut store = self.0.borrow_mut();
            if store.fail_save { return Err("injected write failure".into()); }
            store.values.insert(key, Box::new(*pixels));
            Ok(())
        }
        fn remove(&mut self, key: u64) -> Result<(), String> {
            let mut store = self.0.borrow_mut();
            if store.fail_save { return Err("injected removal failure".into()); }
            store.values.remove(&key);
            Ok(())
        }
    }
    fn cache(initial: usize, maximum: usize) -> (TileCache, Rc<RefCell<Storage>>) {
        let storage = Rc::new(RefCell::new(Storage::default()));
        (TileCache::new(Box::new(Store(storage.clone())), initial, maximum, 1).unwrap(), storage)
    }
    #[test]
    fn paged_surface_matches_heap_pixels_and_smudge_with_two_resident_tiles() {
        use crate::app::PaintApp;
        let _lock = crate::web_surface::JOB_TEST_LOCK.lock().unwrap();
        let (cache, _) = cache(1, 2);
        let mut paged = PaintApp::new_with_tile_limits(512, 512, 1, 2).unwrap();
        assert!(paged.set_tile_cache(cache.clone()));
        let mut heap = PaintApp::new_with_tile_limits(512, 512, 64, 64).unwrap();
        for app in [&mut paged, &mut heap] {
            for index in 0..12 {
                app.active().begin_atomic();
                assert!(app.active().draw_dab(32.0 + (index % 4) as f32 * 128.0,
                    32.0 + (index / 4) as f32 * 128.0, 85.0,
                    0.4, 0.8, 0.2, 0.7, 0.6, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0));
                app.active().end_atomic();
                assert!(!app.take_storage_error());
            }
        }
        assert_eq!(paged.resident_tile_count(), 2);
        assert!(paged.layers[0].used_tile_count() > 2);
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(paged.render_rgba8_tile(x, y), heap.render_rgba8_tile(x, y));
                assert!(!paged.take_storage_error());
            }
        }
        for paint in [-1.0, 0.0, 1.0] {
            assert_eq!(paged.active().get_color(256.0, 200.0, 160.0, paint),
                heap.active().get_color(256.0, 200.0, 160.0, paint));
            assert!(!paged.take_storage_error());
        }
        paged.active().clear_tiles();
        assert!(!paged.take_storage_error());
        assert!(paged.active().get_or_create_tile_mut(0, 0).unwrap().iter().all(|&pixel| pixel == 0));
    }

    #[test]
    fn growth_precedes_eviction_and_all_pixels_survive_replacement() {
        let (cache, _) = cache(1, 3);
        for key in 0..20 { cache.write(key).unwrap()[0] = key as u16 + 1; }
        assert_eq!(cache.resident_tiles(), 3);
        assert_eq!(cache.admitted_tiles(), 3);
        for key in 0..20 { assert_eq!(cache.read(key).unwrap()[0], key as u16 + 1); }
        cache.flush().unwrap();
    }
    #[test]
    fn live_pins_prevent_eviction_and_mutable_aliases() {
        let (cache, _) = cache(1, 1);
        let read = cache.read(1).unwrap();
        assert!(cache.write(1).is_err());
        assert!(cache.read(2).is_err());
        assert!(cache.reset().is_err());
        drop(read);
        let mut write = cache.write(1).unwrap();
        write[3] = 17;
        assert!(cache.read(1).is_err());
        assert!(cache.write(1).is_err());
        drop(write);
        assert_eq!(cache.read(1).unwrap()[3], 17);
    }
    #[test]
    fn write_failure_preserves_dirty_pixels_and_read_failure_exposes_no_partial_tile() {
        let (cache, store) = cache(1, 1);
        cache.write(1).unwrap()[0] = 55;
        store.borrow_mut().fail_save = true;
        assert!(cache.read(2).is_err());
        assert_eq!(cache.read(1).unwrap()[0], 55);
        store.borrow_mut().fail_save = false;
        store.borrow_mut().fail_load = true;
        assert!(cache.read(2).is_err());
        store.borrow_mut().fail_load = false;
        assert_eq!(cache.read(1).unwrap()[0], 55);
        assert_eq!(cache.read(2).unwrap()[0], 0);
    }
}
