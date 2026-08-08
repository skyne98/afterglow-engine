//! Bounded native persistent byte storage. The shell composes this service as a
//! real OS worker; JavaScript transfers values in bounded RingBuffer chunks.

use afterglow_rpc::{RpcError, RpcResult, ServeFuture};
use afterglow_rpc_macros::rpc;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAGIC: u32 = 0x4250_4741; // AGPB
const HEADER_BYTES: u64 = 16;
const TRANSACTION_CAPACITY: usize = 8;
const MAX_LIST_ENTRIES: u32 = 4096;
const VERIFY_SCRATCH_BYTES: usize = 64 * 1024;

#[rpc(worker = BlobStorageWorker, singleton)]
pub trait BlobStorage {
    async fn list(namespace: String, max_entries: u32, max_value_bytes: u64) -> RpcResult<Vec<u8>>;
    async fn size(namespace: String, key: String, max_value_bytes: u64) -> RpcResult<u64>;
    async fn read(
        namespace: String,
        key: String,
        offset: u64,
        len: u32,
        max_value_bytes: u64,
    ) -> RpcResult<Vec<u8>>;
    async fn begin_put(
        namespace: String,
        key: String,
        total_len: u64,
        checksum: u32,
        max_value_bytes: u64,
    ) -> RpcResult<u32>;
    async fn write_chunk(transaction: u32, offset: u64, bytes: Vec<u8>) -> RpcResult<u32>;
    async fn commit_put(transaction: u32) -> RpcResult<bool>;
    async fn abort_put(transaction: u32) -> RpcResult<bool>;
    async fn remove(namespace: String, key: String) -> RpcResult<bool>;
    async fn clear(namespace: String) -> RpcResult<bool>;
}

struct PutTransaction {
    namespace: String,
    key: String,
    target_slot: u8,
    total_len: u64,
    written: u64,
    expected_checksum: u32,
    crc: u32,
    temp_path: PathBuf,
    file: File,
}

struct TransactionSlot {
    generation: u16,
    transaction: Option<PutTransaction>,
}

pub struct BlobStorageWorker {
    root: Option<PathBuf>,
    transactions: Mutex<Vec<TransactionSlot>>,
}

static STORAGE_ROOT: OnceLock<PathBuf> = OnceLock::new();

impl BlobStorageWorker {
    pub fn set_storage_root(root: PathBuf) -> std::io::Result<()> {
        std::fs::create_dir_all(&root)?;
        let _ = STORAGE_ROOT.set(root);
        Ok(())
    }
}

impl Default for BlobStorageWorker {
    fn default() -> Self {
        Self {
            root: STORAGE_ROOT.get().cloned(),
            transactions: Mutex::new(
                (0..TRANSACTION_CAPACITY)
                    .map(|_| TransactionSlot {
                        generation: 0,
                        transaction: None,
                    })
                    .collect(),
            ),
        }
    }
}

fn valid_component(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn confined(root: &Path, namespace: &str, key: Option<&str>) -> Result<PathBuf, RpcError> {
    if !valid_component(namespace, 64) || key.is_some_and(|value| !valid_component(value, 128)) {
        return Err(RpcError::Server("invalid storage namespace or key".into()));
    }
    let directory = root.join(namespace);
    Ok(key.map_or(directory.clone(), |value| directory.join(value)))
}

fn crc_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
        }
    }
    crc
}

fn suffixed_path(base: &Path, suffix: &str) -> PathBuf {
    let mut path = base.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}
fn slot_path(base: &Path, slot: u8) -> PathBuf {
    suffixed_path(base, if slot == 0 { ".0" } else { ".1" })
}
fn pointer_path(base: &Path) -> PathBuf {
    suffixed_path(base, ".ptr")
}

#[derive(Clone, Copy)]
struct GenerationInfo {
    slot: u8,
    generation: u32,
    len: u64,
}

fn verify_generation(base: &Path, slot: u8, max_value_bytes: u64) -> Option<GenerationInfo> {
    let mut file = File::open(slot_path(base, slot)).ok()?;
    let metadata_len = file.metadata().ok()?.len();
    if metadata_len < HEADER_BYTES || metadata_len > HEADER_BYTES + max_value_bytes {
        return None;
    }
    let mut header = [0u8; HEADER_BYTES as usize];
    file.read_exact(&mut header).ok()?;
    if u32::from_le_bytes(header[0..4].try_into().ok()?) != MAGIC {
        return None;
    }
    let generation = u32::from_le_bytes(header[4..8].try_into().ok()?);
    let len = u32::from_le_bytes(header[8..12].try_into().ok()?) as u64;
    let expected = u32::from_le_bytes(header[12..16].try_into().ok()?);
    if len > max_value_bytes || metadata_len != HEADER_BYTES + len {
        return None;
    }
    let mut scratch = vec![0u8; VERIFY_SCRATCH_BYTES];
    let mut remaining = len;
    let mut crc = 0xffff_ffff;
    while remaining != 0 {
        let requested = usize::try_from(remaining.min(VERIFY_SCRATCH_BYTES as u64)).ok()?;
        file.read_exact(&mut scratch[..requested]).ok()?;
        crc = crc_update(crc, &scratch[..requested]);
        remaining -= requested as u64;
    }
    if crc ^ 0xffff_ffff != expected {
        return None;
    }
    Some(GenerationInfo {
        slot,
        generation,
        len,
    })
}

fn selected_generation(base: &Path, max_value_bytes: u64) -> Option<GenerationInfo> {
    if let Ok(pointer) = std::fs::read(pointer_path(base)) {
        if pointer.len() == 1 && pointer[0] <= 1 {
            if let Some(info) = verify_generation(base, pointer[0], max_value_bytes) {
                return Some(info);
            }
        }
    }
    match (
        verify_generation(base, 0, max_value_bytes),
        verify_generation(base, 1, max_value_bytes),
    ) {
        (Some(a), Some(b)) => Some(if b.generation > a.generation { b } else { a }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn transaction_parts(handle: u32) -> (usize, u16) {
    ((handle & 0xffff) as usize, (handle >> 16) as u16)
}
fn transaction_handle(slot: usize, generation: u16) -> u32 {
    (u32::from(generation) << 16) | slot as u32
}

impl BlobStorageServer for BlobStorageWorker {
    fn list(&self, namespace: String, max_entries: u32, max_value_bytes: u64) -> ServeFuture {
        let root = self.root.clone();
        Box::pin(async move {
            if max_entries > MAX_LIST_ENTRIES {
                return Err(RpcError::Server(
                    "stored item capacity exceeds native index limit".into(),
                ));
            }
            let root = root.ok_or_else(|| RpcError::Server("storage worker has no root".into()))?;
            let directory = confined(&root, &namespace, None)?;
            let mut entries = Vec::<(String, u64)>::new();
            if directory.exists() {
                for item in
                    std::fs::read_dir(&directory).map_err(|e| RpcError::Server(e.to_string()))?
                {
                    let item = item.map_err(|e| RpcError::Server(e.to_string()))?;
                    let name = item.file_name().to_string_lossy().into_owned();
                    let Some(key) = name.strip_suffix(".ptr") else {
                        continue;
                    };
                    if !valid_component(key, 128) {
                        return Err(RpcError::Server("invalid stored key".into()));
                    }
                    let base = directory.join(key);
                    let info = selected_generation(&base, max_value_bytes).ok_or_else(|| {
                        RpcError::Server("stored blob has no valid generation".into())
                    })?;
                    entries.push((key.to_owned(), info.len));
                    if entries.len() > max_entries as usize {
                        return Err(RpcError::Server("stored item capacity exceeded".into()));
                    }
                }
            }
            entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            let mut output = Vec::new();
            output.extend_from_slice(&(entries.len() as u32).to_le_bytes());
            for (key, size) in entries {
                output.push(key.len() as u8);
                output.extend_from_slice(key.as_bytes());
                output.extend_from_slice(&size.to_le_bytes());
            }
            afterglow_rpc::encode(&output)
        })
    }

    fn size(&self, namespace: String, key: String, max_value_bytes: u64) -> ServeFuture {
        let root = self.root.clone();
        Box::pin(async move {
            let root = root.ok_or_else(|| RpcError::Server("storage worker has no root".into()))?;
            let base = confined(&root, &namespace, Some(&key))?;
            let info = selected_generation(&base, max_value_bytes)
                .ok_or_else(|| RpcError::Server("blob not found".into()))?;
            afterglow_rpc::encode(&info.len)
        })
    }

    fn read(
        &self,
        namespace: String,
        key: String,
        offset: u64,
        len: u32,
        max_value_bytes: u64,
    ) -> ServeFuture {
        let root = self.root.clone();
        Box::pin(async move {
            let root = root.ok_or_else(|| RpcError::Server("storage worker has no root".into()))?;
            let base = confined(&root, &namespace, Some(&key))?;
            let info = selected_generation(&base, max_value_bytes)
                .ok_or_else(|| RpcError::Server("blob not found".into()))?;
            if offset > info.len {
                return Err(RpcError::Server("read offset exceeds blob".into()));
            }
            let count = u64::from(len).min(info.len - offset) as usize;
            let mut bytes = vec![0u8; count];
            let mut file = File::open(slot_path(&base, info.slot))
                .map_err(|e| RpcError::Server(e.to_string()))?;
            file.seek(SeekFrom::Start(HEADER_BYTES + offset))
                .map_err(|e| RpcError::Server(e.to_string()))?;
            file.read_exact(&mut bytes)
                .map_err(|e| RpcError::Server(e.to_string()))?;
            afterglow_rpc::encode(&bytes)
        })
    }

    fn begin_put(
        &self,
        namespace: String,
        key: String,
        total_len: u64,
        checksum: u32,
        max_value_bytes: u64,
    ) -> ServeFuture {
        let root = self.root.clone();
        let transactions = &self.transactions;
        let result = (|| -> Result<u32, RpcError> {
            if total_len > max_value_bytes || total_len > u32::MAX as u64 {
                return Err(RpcError::Server("blob value capacity exceeded".into()));
            }
            let root = root.ok_or_else(|| RpcError::Server("storage worker has no root".into()))?;
            let base = confined(&root, &namespace, Some(&key))?;
            let directory = base
                .parent()
                .ok_or_else(|| RpcError::Server("invalid storage path".into()))?;
            std::fs::create_dir_all(directory).map_err(|e| RpcError::Server(e.to_string()))?;
            let active = selected_generation(&base, max_value_bytes);
            let target_slot = if active.is_some_and(|info| info.slot == 0) {
                1
            } else {
                0
            };
            let generation = active.map_or(1, |info| info.generation.wrapping_add(1).max(1));
            let mut slots = transactions
                .lock()
                .map_err(|_| RpcError::Server("storage transaction lock poisoned".into()))?;
            if slots.iter().any(|slot| {
                slot.transaction
                    .as_ref()
                    .is_some_and(|tx| tx.namespace == namespace && tx.key == key)
            }) {
                return Err(RpcError::Server(
                    "blob key already has a transaction".into(),
                ));
            }
            let index = slots
                .iter()
                .position(|slot| slot.transaction.is_none())
                .ok_or_else(|| RpcError::Server("storage transaction capacity exceeded".into()))?;
            let slot = &mut slots[index];
            slot.generation = slot.generation.wrapping_add(1).max(1);
            let temp_path = directory.join(format!(
                ".{key}.{}.tmp",
                transaction_handle(index, slot.generation)
            ));
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .read(true)
                .open(&temp_path)
                .map_err(|e| RpcError::Server(e.to_string()))?;
            file.write_all(&MAGIC.to_le_bytes())
                .map_err(|e| RpcError::Server(e.to_string()))?;
            file.write_all(&generation.to_le_bytes())
                .map_err(|e| RpcError::Server(e.to_string()))?;
            file.write_all(&(total_len as u32).to_le_bytes())
                .map_err(|e| RpcError::Server(e.to_string()))?;
            file.write_all(&0u32.to_le_bytes())
                .map_err(|e| RpcError::Server(e.to_string()))?;
            slot.transaction = Some(PutTransaction {
                namespace,
                key,
                target_slot,
                total_len,
                written: 0,
                expected_checksum: checksum,
                crc: 0xffff_ffff,
                temp_path,
                file,
            });
            Ok(transaction_handle(index, slot.generation))
        })();
        Box::pin(async move { afterglow_rpc::encode(&result?) })
    }

    fn write_chunk(&self, transaction: u32, offset: u64, bytes: Vec<u8>) -> ServeFuture {
        let result = (|| -> Result<u32, RpcError> {
            let (index, generation) = transaction_parts(transaction);
            let mut slots = self
                .transactions
                .lock()
                .map_err(|_| RpcError::Server("storage transaction lock poisoned".into()))?;
            let slot = slots
                .get_mut(index)
                .ok_or_else(|| RpcError::Server("invalid storage transaction".into()))?;
            if slot.generation != generation {
                return Err(RpcError::Server("stale storage transaction".into()));
            }
            let tx = slot
                .transaction
                .as_mut()
                .ok_or_else(|| RpcError::Server("closed storage transaction".into()))?;
            if offset != tx.written || tx.written + bytes.len() as u64 > tx.total_len {
                return Err(RpcError::Server(
                    "storage chunks must be sequential and in bounds".into(),
                ));
            }
            tx.file
                .write_all(&bytes)
                .map_err(|e| RpcError::Server(e.to_string()))?;
            tx.crc = crc_update(tx.crc, &bytes);
            tx.written += bytes.len() as u64;
            Ok(bytes.len() as u32)
        })();
        Box::pin(async move { afterglow_rpc::encode(&result?) })
    }

    fn commit_put(&self, transaction: u32) -> ServeFuture {
        let root = self.root.clone();
        let result = (|| -> Result<bool, RpcError> {
            let (index, generation) = transaction_parts(transaction);
            let mut slots = self
                .transactions
                .lock()
                .map_err(|_| RpcError::Server("storage transaction lock poisoned".into()))?;
            let slot = slots
                .get_mut(index)
                .ok_or_else(|| RpcError::Server("invalid storage transaction".into()))?;
            if slot.generation != generation {
                return Err(RpcError::Server("stale storage transaction".into()));
            }
            let mut tx = slot
                .transaction
                .take()
                .ok_or_else(|| RpcError::Server("closed storage transaction".into()))?;
            let checksum = tx.crc ^ 0xffff_ffff;
            if tx.written != tx.total_len || checksum != tx.expected_checksum {
                let _ = std::fs::remove_file(&tx.temp_path);
                return Err(RpcError::Server(
                    "storage transaction length or checksum mismatch".into(),
                ));
            }
            tx.file
                .seek(SeekFrom::Start(12))
                .map_err(|e| RpcError::Server(e.to_string()))?;
            tx.file
                .write_all(&checksum.to_le_bytes())
                .map_err(|e| RpcError::Server(e.to_string()))?;
            tx.file
                .sync_all()
                .map_err(|e| RpcError::Server(e.to_string()))?;
            drop(tx.file);
            let root = root.ok_or_else(|| RpcError::Server("storage worker has no root".into()))?;
            let base = confined(&root, &tx.namespace, Some(&tx.key))?;
            std::fs::rename(&tx.temp_path, slot_path(&base, tx.target_slot))
                .map_err(|e| RpcError::Server(e.to_string()))?;
            let pointer_temp = suffixed_path(&base, &format!(".ptr.{}.tmp", transaction));
            {
                let mut pointer =
                    File::create(&pointer_temp).map_err(|e| RpcError::Server(e.to_string()))?;
                pointer
                    .write_all(&[tx.target_slot])
                    .map_err(|e| RpcError::Server(e.to_string()))?;
                pointer
                    .sync_all()
                    .map_err(|e| RpcError::Server(e.to_string()))?;
            }
            std::fs::rename(pointer_temp, pointer_path(&base))
                .map_err(|e| RpcError::Server(e.to_string()))?;
            if let Some(parent) = base.parent() {
                if let Ok(directory) = File::open(parent) {
                    let _ = directory.sync_all();
                }
            }
            Ok(true)
        })();
        Box::pin(async move { afterglow_rpc::encode(&result?) })
    }

    fn abort_put(&self, transaction: u32) -> ServeFuture {
        let result = (|| -> Result<bool, RpcError> {
            let (index, generation) = transaction_parts(transaction);
            let mut slots = self
                .transactions
                .lock()
                .map_err(|_| RpcError::Server("storage transaction lock poisoned".into()))?;
            let Some(slot) = slots.get_mut(index) else {
                return Ok(false);
            };
            if slot.generation != generation {
                return Ok(false);
            }
            let Some(tx) = slot.transaction.take() else {
                return Ok(false);
            };
            drop(tx.file);
            let _ = std::fs::remove_file(tx.temp_path);
            Ok(true)
        })();
        Box::pin(async move { afterglow_rpc::encode(&result?) })
    }

    fn remove(&self, namespace: String, key: String) -> ServeFuture {
        let root = self.root.clone();
        Box::pin(async move {
            let root = root.ok_or_else(|| RpcError::Server("storage worker has no root".into()))?;
            let base = confined(&root, &namespace, Some(&key))?;
            let existed = pointer_path(&base).exists();
            for path in [
                pointer_path(&base),
                slot_path(&base, 0),
                slot_path(&base, 1),
            ] {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(RpcError::Server(error.to_string())),
                }
            }
            afterglow_rpc::encode(&existed)
        })
    }

    fn clear(&self, namespace: String) -> ServeFuture {
        let root = self.root.clone();
        Box::pin(async move {
            let root = root.ok_or_else(|| RpcError::Server("storage worker has no root".into()))?;
            let directory = confined(&root, &namespace, None)?;
            if directory.exists() {
                std::fs::remove_dir_all(&directory).map_err(|e| RpcError::Server(e.to_string()))?;
            }
            std::fs::create_dir_all(directory).map_err(|e| RpcError::Server(e.to_string()))?;
            afterglow_rpc::encode(&true)
        })
    }
}

impl Drop for BlobStorageWorker {
    fn drop(&mut self) {
        if let Ok(slots) = self.transactions.get_mut() {
            for slot in slots {
                if let Some(tx) = slot.transaction.take() {
                    drop(tx.file);
                    let _ = std::fs::remove_file(tx.temp_path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    static VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );

    fn drive<F: Future>(client: &BlobStorageClient, future: F) -> F::Output {
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            client.poll();
            if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn crc(bytes: &[u8]) -> u32 {
        crc_update(0xffff_ffff, bytes) ^ 0xffff_ffff
    }

    #[test]
    fn native_worker_atomically_replaces_chunked_values() {
        let root = std::env::temp_dir().join(format!("afterglow-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        BlobStorageWorker::set_storage_root(root.clone()).unwrap();
        let client = BlobStorageClient::spawn_worker().unwrap();
        let first = b"first persistent value";
        let tx = drive(
            &client,
            client
                .begin_put(
                    "game".into(),
                    "paint".into(),
                    first.len() as u64,
                    crc(first),
                    1024,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            drive(&client, client.write_chunk(tx, 0, first.to_vec()).unwrap()).unwrap(),
            first.len() as u32
        );
        assert!(drive(&client, client.commit_put(tx).unwrap()).unwrap());
        assert_eq!(
            drive(
                &client,
                client.size("game".into(), "paint".into(), 1024).unwrap()
            )
            .unwrap(),
            first.len() as u64
        );
        assert_eq!(
            drive(
                &client,
                client
                    .read("game".into(), "paint".into(), 6, 10, 1024)
                    .unwrap()
            )
            .unwrap(),
            b"persistent".to_vec()
        );

        let bad = b"bad replacement";
        let tx = drive(
            &client,
            client
                .begin_put("game".into(), "paint".into(), bad.len() as u64, 123, 1024)
                .unwrap(),
        )
        .unwrap();
        drive(&client, client.write_chunk(tx, 0, bad.to_vec()).unwrap()).unwrap();
        assert!(drive(&client, client.commit_put(tx).unwrap()).is_err());
        assert_eq!(
            drive(
                &client,
                client
                    .read("game".into(), "paint".into(), 0, first.len() as u32, 1024)
                    .unwrap()
            )
            .unwrap(),
            first
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
