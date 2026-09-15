//! Bound the combined database and rollback-journal file extents before writes.
use super::Error;
use rusqlite::ffi;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicU64, Ordering};

#[repr(C)]
pub(super) struct BoundedVfs {
    vfs: ffi::sqlite3_vfs,
    parent: *mut ffi::sqlite3_vfs,
    name: CString,
    limit: u64,
    sizes: [u64; 2],
    peak: u64,
    offset: usize,
    paths: [Option<CString>; 2],
    opened: [bool; 2],
}
#[repr(C)]
struct File {
    methods: ffi::sqlite3_io_methods,
    original: *const ffi::sqlite3_io_methods,
    owner: *mut BoundedVfs,
    slot: usize,
}
impl BoundedVfs {
    pub fn new(limit: u64) -> Result<Box<Self>, Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = CString::new(format!("afterglow-bytes-{}", NEXT.fetch_add(1, Ordering::Relaxed))).unwrap();
        // SQLite initializes its default VFS before this copy. Each store owns one VFS.
        unsafe {
            if ffi::sqlite3_initialize() != ffi::SQLITE_OK { return Err(Error::Limit("SQLite initialization")); }
            let parent = ffi::sqlite3_vfs_find(std::ptr::null());
            if parent.is_null() { return Err(Error::Limit("SQLite VFS")); }
            // Only the bundled OS implementations have this callback contract.
            let parent_name = CStr::from_ptr((*parent).zName).to_bytes();
            if parent_name != b"unix" && parent_name != b"win32" { return Err(Error::Limit("unsupported SQLite parent VFS")); }
            let offset = ((*parent).szOsFile as usize).next_multiple_of(std::mem::align_of::<File>());
            let mut value = Box::new(Self { vfs: std::ptr::read(parent), parent, name, limit,
                sizes: [0; 2], peak: 0, offset, paths: [None, None], opened: [false; 2] });
            value.vfs.zName = value.name.as_ptr();
            value.vfs.pNext = std::ptr::null_mut();
            value.vfs.szOsFile = (offset + std::mem::size_of::<File>()).try_into().map_err(|_| Error::Limit("VFS file size"))?;
            value.vfs.xOpen = Some(open);
            value.vfs.xDelete = Some(delete);
            // Other parent callbacks retain the original pAppData and VFS fields.
            if ffi::sqlite3_vfs_register(&mut value.vfs, 0) != ffi::SQLITE_OK { return Err(Error::Limit("VFS registration")); }
            Ok(value)
        }
    }
    pub fn name(&self) -> &str { self.name.to_str().unwrap() }
    pub fn peak(&self) -> u64 { self.peak }
    fn admit(&mut self, slot: usize, size: u64) -> bool {
        let Some(total) = self.sizes[1 - slot].checked_add(size) else { return false; };
        if total > self.limit { return false; }
        self.sizes[slot] = size;
        self.peak = self.peak.max(total);
        true
    }
}
impl Drop for BoundedVfs {
    fn drop(&mut self) {
        // The owning ByteStore drops its connection before this field.
        unsafe { ffi::sqlite3_vfs_unregister(&mut self.vfs); }
    }
}
unsafe extern "C" fn open(vfs: *mut ffi::sqlite3_vfs, name: *const c_char, file: *mut ffi::sqlite3_file,
    flags: c_int, output: *mut c_int) -> c_int {
    unsafe {
        let owner = vfs.cast::<BoundedVfs>();
        // ByteStore uses one database, DELETE journaling, and memory-only temporary storage.
        let slot = if flags & ffi::SQLITE_OPEN_MAIN_DB != 0 { 0 }
            else if flags & ffi::SQLITE_OPEN_MAIN_JOURNAL != 0 { 1 }
            else { (*file).pMethods = std::ptr::null(); return ffi::SQLITE_CANTOPEN; };
        (*file).pMethods = std::ptr::null();
        if name.is_null() || (*owner).opened[slot] { return ffi::SQLITE_CANTOPEN; }
        let name_bytes = CStr::from_ptr(name).to_bytes();
        if slot == 0 && (*owner).paths[0].is_none() {
            let mut main = Vec::new();
            let mut journal = Vec::new();
            if main.try_reserve_exact(name_bytes.len() + 1).is_err() || journal.try_reserve_exact(name_bytes.len() + 9).is_err() { return ffi::SQLITE_NOMEM; }
            main.extend_from_slice(name_bytes); main.push(0);
            journal.extend_from_slice(name_bytes); journal.extend_from_slice(b"-journal\0");
            (*owner).paths = [Some(CString::from_vec_with_nul_unchecked(main)), Some(CString::from_vec_with_nul_unchecked(journal))];
        }
        if (*owner).paths[slot].as_ref().is_none_or(|path| path.as_bytes() != name_bytes) { return ffi::SQLITE_CANTOPEN; }
        let parent = (*owner).parent;
        let code = ((*parent).xOpen.unwrap())(parent, name, file, flags, output);
        if code != ffi::SQLITE_OK { return code; }
        let original = (*file).pMethods;
        let mut size = 0;
        let code = ((*original).xFileSize.unwrap())(file, &mut size);
        if code != ffi::SQLITE_OK || size < 0 || !(*owner).admit(slot, size as u64) {
            ((*original).xClose.unwrap())(file);
            (*file).pMethods = std::ptr::null();
            return if code == ffi::SQLITE_OK { ffi::SQLITE_FULL } else { code };
        }
        let state = file.cast::<u8>().add((*owner).offset).cast::<File>();
        state.write(File { methods: std::ptr::read(original), original, owner, slot });
        (*owner).opened[slot] = true;
        (*state).methods.xClose = Some(close);
        (*state).methods.xWrite = Some(write);
        (*state).methods.xTruncate = Some(truncate);
        (*state).methods.xFileControl = Some(control);
        // The bundled OS callbacks use the original file at offset zero.
        // They do not dispatch these operations through the replaced method table.
        (*file).pMethods = &(*state).methods;
        ffi::SQLITE_OK
    }
}
unsafe extern "C" fn close(file: *mut ffi::sqlite3_file) -> c_int {
    unsafe {
        let state = &*((*file).pMethods as *const File);
        let owner = state.owner;
        let slot = state.slot;
        let code = ((*state.original).xClose.unwrap())(file);
        (*owner).opened[slot] = false;
        code
    }
}
unsafe extern "C" fn write(file: *mut ffi::sqlite3_file, data: *const c_void, amount: c_int, offset: i64) -> c_int {
    unsafe {
        let state = &mut *((*file).pMethods as *mut File);
        if amount < 0 || offset < 0 { return ffi::SQLITE_IOERR_WRITE; }
        let Some(end) = (offset as u64).checked_add(amount as u64) else { return ffi::SQLITE_FULL; };
        let owner = &mut *state.owner;
        if !owner.admit(state.slot, end.max(owner.sizes[state.slot])) { return ffi::SQLITE_FULL; }
        // Retain the charge after an I/O error: an OS write can be partial.
        (state.original.as_ref().unwrap().xWrite.unwrap())(file, data, amount, offset)
    }
}
unsafe extern "C" fn truncate(file: *mut ffi::sqlite3_file, size: i64) -> c_int {
    unsafe {
        let state = &mut *((*file).pMethods as *mut File);
        if size < 0 { return ffi::SQLITE_IOERR_TRUNCATE; }
        let owner = &mut *state.owner;
        let previous = owner.sizes[state.slot];
        if !owner.admit(state.slot, previous.max(size as u64)) { return ffi::SQLITE_FULL; }
        let code = (state.original.as_ref().unwrap().xTruncate.unwrap())(file, size);
        if code == ffi::SQLITE_OK { owner.sizes[state.slot] = size as u64; }
        code
    }
}
unsafe extern "C" fn control(file: *mut ffi::sqlite3_file, operation: c_int, argument: *mut c_void) -> c_int {
    unsafe {
        // These hints can grow files without xWrite or xTruncate. Disable them.
        if operation == ffi::SQLITE_FCNTL_SIZE_HINT || operation == ffi::SQLITE_FCNTL_CHUNK_SIZE { return ffi::SQLITE_OK; }
        let state = &*((*file).pMethods as *const File);
        ((*state.original).xFileControl.unwrap())(file, operation, argument)
    }
}
unsafe extern "C" fn delete(vfs: *mut ffi::sqlite3_vfs, name: *const c_char, sync: c_int) -> c_int {
    unsafe {
        let owner = &mut *vfs.cast::<BoundedVfs>();
        if name.is_null() || owner.paths[1].as_ref().is_none_or(|path| path.as_c_str() != CStr::from_ptr(name)) { return ffi::SQLITE_IOERR_DELETE; }
        let code = ((*owner.parent).xDelete.unwrap())(owner.parent, name, sync);
        if code == ffi::SQLITE_OK {
            owner.sizes[1] = 0;
        }
        code
    }
}
