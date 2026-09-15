# Native transactional byte store

Source: `crates/afterglow-storage-worker/src/byte_store.rs`.

## Status and ownership

`byte_store::ByteStore` is a native Rust component, not a new RPC transport.
One native worker owns its SQLite connection.
The application supplies a confined database path and keeps application keys and recovery policy outside this component.
The web build does not link SQLite.

The native paint worker uses this component for tile versions and completed-document roots.
See `../implementation/native-large-canvas-paint-plan.md` for the accepted RAM, scratch, thread, and recovery decisions.
The existing `BlobStorageWorker` API and its file format do not change.

## Operations

- `ByteStore::open(path, limits)` opens or creates the database.
- `ByteStore::open_bounded(path, limits, file_bytes)` also bounds the combined database and rollback-journal file extents.
- `peak_file_bytes()` gives the combined high-water charge for a bounded connection.
- `get(key, output)` copies a value into caller storage and returns its length.
  `None` means a missing key. `Some(0)` means an empty value.
- `keys_after(after, output)` reads up to 1,024 keys in unsigned numeric order.
  `None` starts at zero. A supplied key is exclusive, and `u64::MAX` returns no keys.
  The caller supplies the output buffer and uses the last returned key for the next batch.
- `apply(mutations)` commits a slice of `Put` and `Remove` operations together.
  Repeated keys follow slice order.
- `stats()` returns the entry count, live value bytes, and allocated database bytes.

Keys are exact `u64` values. SQLite stores their bit patterns as signed integers.
Two indexed SQL ranges retain unsigned key order across the signed-integer boundary.
Key batches do not form a snapshot across separate calls.

## Capacities and errors

`Limits` supplies database bytes, maximum value bytes, transaction operation count, transaction input bytes, and SQLite cache bytes.
The transaction operation count is at most 1,024.
The store validates the complete input slice before a transaction starts.
A short output buffer remains unchanged.
A failed transaction retains the previous completed transaction.
Database-full errors do not permanently disable the connection.

`Error::Limit` identifies an input or output capacity error.
`Error::Format` identifies an unsupported store format.
`Error::Database` retains the SQLite error, including full-database and I/O failures.
The caller decides how these errors affect its current operation.

## Persistence and limits of the guarantee

The database uses 4,096-byte pages, `journal_mode=DELETE`, and `synchronous=FULL`.
The store uses an exclusive SQLite lock and a zero busy timeout.
An incompatible database remains intact.
The database format has an application identifier and version 1.

`database_bytes` limits the main SQLite database, not the rollback journal or its directory.
`open_bounded` checks combined file growth in a connection-owned SQLite VFS before writes and truncation.
It disables file-size and allocation-chunk hints that could bypass these checks.
The connection closes before its VFS is removed.
The wrapper accepts only the bundled `unix` or `win32` parent VFS and checks exact database/journal paths.
It rejects a second connection and disk file types other than the database and its rollback journal.
The private connection does not expose ATTACH, VACUUM, or WAL operations. Startup verifies DELETE journal mode.
The limit concerns logical file extents, not filesystem metadata or block-allocation overhead.
The application must still maintain its free-space reserve.
The SQLite cache setting is a target, not a hard total-memory limit.
This component does not yet prove allocation-free operation or the complete paint RAM/disk limits.
Do not treat successful transaction tests as complete paint recovery evidence.

## Checks

Run `nix-shell shell.nix --run 'CARGO_BUILD_JOBS=$(nproc) cargo test -p afterglow-storage-worker --lib'`.
The tests cover reopen, exact keys, empty values, ordered mutations, input/output bounds, full-database rollback, and foreign-database protection.
The final byte-store lane passed eight tests, including combined journal exhaustion, retained values after rollback, subsequent writes, and blocked file-size hints.
It also rejected a second connection and deletion of another journal path.
The integrated ten-minute native 16K run measured a combined extent peak of 201,658,944 bytes under its 64 GiB limit.
The key-batch regression includes the signed-integer boundary, `u64::MAX`, output capacity, and removal between batches.
A child-process check flushes uncommitted pages and exits without destructors.
Recovery must return both old values before commit or both new values after commit.
These checks do not simulate physical power loss or every filesystem failure.
