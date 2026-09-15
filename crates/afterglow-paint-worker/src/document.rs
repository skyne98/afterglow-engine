//! One native owner for document pixels, completed roots, and disk history.

use crate::paged_store::{PaintBacking, PaintStore};
use maipointo::app::PaintApp;
use maipointo::tile_cache::TileCache;
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub struct Document {
    pub app: PaintApp,
    pub store: Rc<RefCell<PaintStore>>,
    cache: TileCache,
    initial: usize,
    maximum: usize,
    growth: usize,
}

impl Document {
    /// Attach an empty app only when the store has no recovery document.
    pub fn new(mut app: PaintApp, store: Rc<RefCell<PaintStore>>,
        initial: usize, maximum: usize, growth: usize) -> Result<Self, String> {
        if store.borrow().has_recovery() { return Err("paint recovery decision is necessary".into()); }
        crate::tile_threads::initialize()?;
        let cache = Self::cache(&store, initial, maximum, growth, app.width, app.height)?;
        Self::attach(&mut app, &cache)?;
        let mut document = Self { app, store, cache, initial, maximum, growth };
        document.commit()?;
        Ok(document)
    }

    /// Prepare a new document before an explicit Discard replaces the recovery root.
    pub fn replacement(mut app: PaintApp, store: Rc<RefCell<PaintStore>>, initial: usize,
        maximum: usize, growth: usize) -> Result<Self, String> {
        crate::tile_threads::initialize()?;
        let cache = Self::cache(&store, initial, maximum, growth, app.width, app.height)?;
        Self::attach(&mut app, &cache)?;
        let metadata = serde_json::to_vec(&app.document_metadata()?).map_err(|error| error.to_string())?;
        store.borrow_mut().replace(&metadata)?;
        Ok(Self { app, store, cache, initial, maximum, growth })
    }

    /// Restore metadata and tile identities without loading all pixels into RAM.
    pub fn restore(store: Rc<RefCell<PaintStore>>, initial: usize, maximum: usize,
        growth: usize) -> Result<Self, String> {
        crate::tile_threads::initialize()?;
        let (app, cache) = {
            let saved = store.borrow();
            if !saved.has_recovery() { return Err("paint recovery document is missing".into()); }
            Self::prepare(saved.metadata(), saved.completed_tiles(), &store, initial, maximum, growth)?
        };
        Ok(Self { app, store, cache, initial, maximum, growth })
    }

    fn cache(store: &Rc<RefCell<PaintStore>>, initial: usize, maximum: usize,
        growth: usize, width: i32, height: i32) -> Result<TileCache, String> {
        let cache = TileCache::new(Box::new(PaintBacking(store.clone())), initial, maximum, growth)?;
        let slots = ((width as u64 + 63) / 64) * ((height as u64 + 63) / 64) * 8;
        // Logical surface arrays, their growth overlap, and owner/pool scratch.
        let owner_bytes = slots * 256 + 192 * 1024 * 1024;
        cache.set_memory(afterglow_memory::process()?.clone(), owner_bytes)?;
        Ok(cache)
    }

    fn attach(app: &mut PaintApp, cache: &TileCache) -> Result<(), String> {
        if !app.set_history_capture_enabled(false) || !app.set_tile_cache(cache.clone()) {
            return Err("paint storage needs an empty document between strokes".into());
        }
        app.set_tile_job_dispatcher(Some(crate::tile_threads::run));
        Ok(())
    }

    fn prepare(metadata: &[u8], tiles: &HashMap<u64, u64>, store: &Rc<RefCell<PaintStore>>,
        initial: usize, maximum: usize, growth: usize) -> Result<(PaintApp, TileCache), String> {
        let metadata: Value = serde_json::from_slice(metadata)
            .map_err(|error| format!("paint document metadata: {error}"))?;
        let width = metadata.get("width").and_then(Value::as_i64).filter(|width| (1..=16384).contains(width)).ok_or("invalid saved paint width")? as i32;
        let height = metadata.get("height").and_then(Value::as_i64).filter(|height| (1..=16384).contains(height)).ok_or("invalid saved paint height")? as i32;
        let cache = Self::cache(store, initial, maximum, growth, width, height)?;
        let mut app = PaintApp::from_document_metadata(&metadata, initial, maximum)?;
        Self::attach(&mut app, &cache)?;
        for &key in tiles.keys() {
            let id = (key >> 32) as u32;
            let layer = app.layers.iter_mut().find(|layer| layer.storage_id() == id)
                .ok_or("stored paint tile has no layer")?;
            let tx = (key & 65535) as i32;
            let ty = ((key >> 16) & 65535) as i32;
            if !layer.register_stored_tile(tx, ty) { return Err("stored paint tile metadata is incorrect".into()); }
        }
        Ok((app, cache))
    }

    /// Replace the document only after an explicit New or Discard decision.
    /// Allocation or storage failure leaves the current app and recovery root intact.
    pub fn replace(&mut self, mut app: PaintApp, initial: usize, maximum: usize,
        growth: usize) -> Result<(), String> {
        let cache = Self::cache(&self.store, initial, maximum, growth, app.width, app.height)?;
        Self::attach(&mut app, &cache)?;
        let metadata = serde_json::to_vec(&app.document_metadata()?)
            .map_err(|error| format!("paint document metadata: {error}"))?;
        self.store.borrow_mut().replace(&metadata)?;
        self.initial = initial;
        self.maximum = maximum;
        self.growth = growth;
        self.install(app, cache);
        Ok(())
    }

    /// The caller must complete all tile jobs before a document operation.
    pub fn commit(&mut self) -> Result<(), String> {
        self.check_storage()?;
        let metadata = serde_json::to_vec(&self.app.document_metadata()?)
            .map_err(|error| format!("paint document metadata: {error}"))?;
        self.cache.flush()?;
        self.store.borrow_mut().publish(&metadata)
    }

    /// Never return rendered pixels after a failed backing read.
    pub fn check_storage(&mut self) -> Result<(), String> {
        if self.app.take_storage_error() { return Err("paint tile storage operation failed".into()); }
        if let Some(error) = crate::engine_error_message(self.app.error_code) { return Err(error); }
        Ok(())
    }

    pub fn select(&mut self, delta: i32) -> Result<bool, String> {
        let prepared = self.store.borrow_mut().select_prepared(delta, |metadata, tiles| {
            Self::prepare(metadata, tiles, &self.store, self.initial, self.maximum, self.growth)
        })?;
        if let Some((app, cache)) = prepared {
            self.install(app, cache);
            Ok(true)
        } else { Ok(false) }
    }

    /// Prepare the previous document before discarding any working state.
    pub fn rollback(&mut self) -> Result<(), String> {
        let (app, cache) = {
            let saved = self.store.borrow();
            Self::prepare(saved.metadata(), saved.completed_tiles(), &self.store,
                self.initial, self.maximum, self.growth)?
        };
        self.store.borrow_mut().rollback()?;
        self.install(app, cache);
        Ok(())
    }

    fn install(&mut self, mut app: PaintApp, cache: TileCache) {
        app.brush = self.app.brush.take();
        if let Some(brush) = app.brush.as_mut() { brush.request_reset(); brush.new_stroke(); }
        self.app = app;
        self.cache = cache;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paged_store::DiskLimits;
    use std::{path::PathBuf, sync::atomic::{AtomicU64, Ordering}};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("afterglow-paint-document-{}-{}",
                std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn open(&self) -> Rc<RefCell<PaintStore>> {
            Rc::new(RefCell::new(PaintStore::open(&self.0.join("paint.sqlite"),
                DiskLimits { scratch_bytes: 128 * 1024 * 1024, free_reserve_bytes: 0 }).unwrap()))
        }
    }
    impl Drop for Directory { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    fn pixels(document: &Document) -> Vec<u16> {
        let mut pixels = Vec::new();
        for ty in 0..2 { for tx in 0..4 {
            pixels.extend_from_slice(&document.app.layers[0].get_tile(tx, ty).unwrap()[..]);
        } }
        pixels
    }

    #[test]
    fn native_document_pages_strokes_without_capture_and_retains_composition_history() {
        let _lock = crate::tests::TEST_LOCK.lock().unwrap();
        let directory = Directory::new();
        let mut document = Document::new(PaintApp::new_with_tile_limits(256, 128, 1, 2).unwrap(),
            directory.open(), 1, 2, 1).unwrap();
        for ty in 0..2 { for tx in 0..4 {
            document.app.active().get_or_create_tile_mut(tx, ty).unwrap().fill(16000);
        } }
        document.commit().unwrap();
        let before = pixels(&document);
        document.app.begin_stroke(32.0, 64.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        for index in 0..12 {
            let mut result = document.app.stroke_to(32.0 + index as f32 * 16.0, 64.0, 0.8,
                0.0, 0.0, 0.016, 1.0, 0.0, 0.0, false);
            while result == 0 && document.app.has_stroke_continuation() { result = document.app.continue_stroke_to(); }
            assert!(result > 0);
        }
        document.app.history_commit();
        assert_eq!(document.app.external_history_capture_count(), 0);
        assert!(!document.app.history_can_undo());
        document.commit().unwrap();
        let after = pixels(&document);
        assert_ne!(after, before);
        assert_eq!(document.cache.resident_tiles(), 2);
        assert!(document.select(-1).unwrap());
        assert_eq!(pixels(&document), before);
        assert!(document.select(1).unwrap());
        assert_eq!(pixels(&document), after);
        document.app.create_layer();
        document.app.active().get_or_create_tile_mut(0, 0).unwrap().fill(9000);
        let group = document.app.create_group();
        assert!(document.app.set_layer_group(1, group));
        document.commit().unwrap();
        let grouped = document.app.document_metadata().unwrap();
        assert!(document.app.delete_layer(1));
        document.commit().unwrap();
        assert!(document.select(-1).unwrap());
        assert_eq!(document.app.document_metadata().unwrap(), grouped);
        assert_eq!(document.app.layers[1].get_tile(0, 0).unwrap()[0], 9000);
        drop(document);
        let restored = Document::restore(directory.open(), 1, 2, 1).unwrap();
        assert_eq!(restored.app.document_metadata().unwrap(), grouped);
        assert_eq!(pixels(&restored), after);
        assert!(restored.store.borrow().can_redo());
    }

    #[test]
    fn native_document_rollback_after_write_failure_keeps_the_next_change_available() {
        let _lock = crate::tests::TEST_LOCK.lock().unwrap();
        let directory = Directory::new();
        let mut document = Document::new(PaintApp::new_with_tile_limits(256, 128, 1, 2).unwrap(),
            directory.open(), 1, 2, 1).unwrap();
        document.app.active().get_or_create_tile_mut(0, 0).unwrap().fill(12000);
        document.commit().unwrap();
        document.app.active().get_or_create_tile_mut(0, 0).unwrap().fill(20000);
        document.store.borrow_mut().fail_write = true;
        assert!(document.commit().is_err());
        document.rollback().unwrap();
        assert_eq!(document.app.layers[0].get_tile(0, 0).unwrap()[0], 12000);
        document.store.borrow_mut().fail_write = false;
        document.app.active().get_or_create_tile_mut(0, 0).unwrap().fill(22000);
        document.commit().unwrap();
        assert!(document.select(-1).unwrap());
        assert_eq!(document.app.layers[0].get_tile(0, 0).unwrap()[0], 12000);
    }

    #[test]
    fn invalid_recovery_metadata_never_replaces_the_live_document() {
        let _lock = crate::tests::TEST_LOCK.lock().unwrap();
        let directory = Directory::new();
        let mut document = Document::new(PaintApp::new_with_tile_limits(256, 128, 1, 2).unwrap(),
            directory.open(), 1, 2, 1).unwrap();
        document.app.active().get_or_create_tile_mut(0, 0).unwrap().fill(12000);
        document.commit().unwrap();
        document.store.borrow_mut().publish(b"invalid document").unwrap();
        document.commit().unwrap();
        let before = document.app.document_metadata().unwrap();
        assert!(document.select(-1).is_err());
        assert_eq!(document.app.document_metadata().unwrap(), before);
        assert_eq!(document.app.layers[0].get_tile(0, 0).unwrap()[0], 12000);
        let store = document.store.clone();
        drop(document);
        assert!(Document::restore(store.clone(), 1, 2, 1).is_ok());
        let metadata = store.borrow().metadata().to_vec();
        store.borrow_mut().save(99u64 << 32, &[16000; 64 * 64 * 4]).unwrap();
        store.borrow_mut().publish(&metadata).unwrap();
        assert!(Document::restore(store, 1, 2, 1).is_err());
    }

    #[test]
    fn failed_replacement_retains_pixels_and_success_clears_durable_history() {
        let _lock = crate::tests::TEST_LOCK.lock().unwrap();
        let directory = Directory::new();
        let mut document = Document::new(PaintApp::new_with_tile_limits(256, 128, 1, 2).unwrap(),
            directory.open(), 1, 2, 1).unwrap();
        document.app.active().get_or_create_tile_mut(0, 0).unwrap().fill(12000);
        document.commit().unwrap();
        assert!(document.store.borrow().can_undo());
        document.store.borrow_mut().fail_write = true;
        assert!(document.replace(PaintApp::new_with_tile_limits(64, 64, 1, 1).unwrap(), 1, 1, 1).is_err());
        assert_eq!(document.app.width, 256);
        assert_eq!(document.app.layers[0].get_tile(0, 0).unwrap()[0], 12000);
        assert!(document.store.borrow().can_undo());
        drop(document);
        let mut document = Document::restore(directory.open(), 1, 2, 1).unwrap();
        assert_eq!(document.app.width, 256);
        assert_eq!(document.app.layers[0].get_tile(0, 0).unwrap()[0], 12000);
        document.replace(PaintApp::new_with_tile_limits(64, 64, 1, 1).unwrap(), 1, 1, 1).unwrap();
        assert_eq!(document.app.width, 64);
        assert_eq!(document.cache.resident_tiles(), 0);
        assert_eq!(document.store.borrow().tile_keys().count(), 0);
        assert!(!document.store.borrow().can_undo());
        assert!(!document.store.borrow().can_redo());
        document.store.borrow_mut().collect_garbage().unwrap();
        let entries = document.store.borrow().stats().unwrap().entries;
        document.commit().unwrap();
        assert_eq!(document.store.borrow().stats().unwrap().entries, entries);
        assert!(!document.store.borrow().can_undo());
        drop(document);
        let document = Document::restore(directory.open(), 1, 1, 1).unwrap();
        assert_eq!(document.app.width, 64);
        assert_eq!(document.app.height, 64);
        assert!(document.app.layers[0].get_tile(0, 0).is_none());
        assert!(!document.store.borrow().can_undo());
        assert_eq!(document.store.borrow().stats().unwrap().entries, 3);
    }

    #[test]
    fn failed_document_preparation_never_changes_the_durable_history_head() {
        let directory = Directory::new();
        let store = directory.open();
        store.borrow_mut().publish(b"invalid document").unwrap();
        store.borrow_mut().publish(b"another document").unwrap();
        let result: Result<Option<()>, _> = store.borrow_mut().select_prepared(-1, |_, _| Err("allocation failed".into()));
        assert!(result.is_err());
        assert_eq!(store.borrow().metadata(), b"another document");
        drop(store);
        let restored = directory.open();
        assert_eq!(restored.borrow().metadata(), b"another document");
        assert!(restored.borrow().can_undo());
        assert!(!restored.borrow().can_redo());
    }
}
