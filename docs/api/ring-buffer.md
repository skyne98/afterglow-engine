# `afterglow-rpc` API — ring buffer and native transport

> Status: working; API checked against the 2026-07-15 source.

Native binaries may install `allocation::TrackingAllocator<System>` as their
`#[global_allocator]` and use `allocation::assert_no_alloc(|| ...)` around
sealed hot-path regression tests. Tracking is opt-in and thread-local, avoiding
noise from parallel test threads. The ring `write` + `read_into` path has a
regression test proving zero allocations after storage construction.

## Purpose

`afterglow_rpc::RingBuffer` is the framed, lock-free SPSC primitive used by the
native and web worker transports. Each worker has separate request and response
rings; native workers also have an event ring.

The ring is the payload transport. Wake-ups (`Thread::unpark` natively and
payload-free `postMessage` on web) only notify the consumer that ring data is
available.

## Memory layout and ordering

```text
[capacity: u32][write_idx: AtomicU32][read_idx: AtomicU32][data...]
<---------------- 12-byte header ----------------->
```

Messages are `[payload_len: u32 LE][payload]` and may wrap around the data-area
end. The producer publishes `write_idx` with `Release`; the consumer observes it
with `Acquire`, then publishes `read_idx` with `Release`. Exactly one producer
and one consumer are allowed per ring.

```rust
pub const HEADER_SIZE: usize = 12;
pub const ALIGN: usize = 4;
```

## Borrowed primitive

### `RingHeader`

```rust
let header = RingHeader::new(capacity);
header.capacity();
header.reset(); // startup/exclusive access only
```

### `RingBuffer<'a>`

The low-level constructor is unsafe because callers provide the initialized
header and interior-mutable backing directly:

```rust
let ring = unsafe {
    RingBuffer::from_header_data(&header, data_ptr, data_len)?
};

ring.capacity();
ring.has_data();
ring.peek_len()?;
ring.write(payload)?;
let bytes = ring.read()?;             // allocates Vec<u8>
let n = ring.read_into(&mut scratch)?; // allocation-free
```

`RingBuffer` itself is `!Send + !Sync`. Cross-thread native code uses the owned,
split halves below. `read_into` returns `BufferTooSmall` without consuming the
frame, allowing a retry with a larger destination.

## Native owned API (`afterglow_rpc::native`)

### Raw ring

```rust
let storage = RingStorage::new(1 << 20)?;
let (producer, consumer) = storage.split();

producer.write(b"request")?;
assert!(consumer.has_data());
let response = consumer.read()?;
```

`RingStorage` owns one compact, 4-byte-aligned heap allocation containing the
header and byte data. The allocation is shared internally with `Arc`; it is not
an `Arc<Vec<u8>>`. Consuming `split` creates exactly one `RingProducer` and one
`RingConsumer`; each half is `Send + !Sync`.

Public half methods:

```rust
RingProducer::{write, can_write, capacity}
RingConsumer::{read, read_into, peek_len, has_data, capacity}
```

### Worker transport

Generated clients expose the normal entry point:

```rust
let (client, events) = PhysicsClient::spawn_worker(PhysicsWorker)?;
let state = client.step(vec![0.0, 1.0], 0.016)?;
```

The `#[rpc(worker = ...)]`-generated `spawn_worker` (see
[`rpc-macro.md`](rpc-macro.md)) uses 1 MiB request, response, and event rings
and calls:

```rust
spawn_worker_loop(impl_, capacity, |server, method, args| {
    server.serve(method, args)
})
```

`spawn_worker_loop_with_idle` / `run_worker_loop_with_idle` add a bounded
non-blocking idle hook and park duration while preserving the same generated
client transport. The hook executes on the service OS thread only while its
request ring is empty. It exists for fixed-work device-clock producers such as
native EngineAudio PCM filling; it must not block or allocate. `can_write`
admits a fixed payload before expensive production so a full output ring never
advances or drops producer state.

`WorkerTransport` implements `Transport::call`. A call:

1. Writes `[method: u32 LE][postcard args]` to the request ring.
2. Unparks the worker.
3. Parks with bounded retries until a response arrives (10 s deadline).
4. Decodes the `Response` envelope and returns its successful payload.

Dropping the transport writes an empty request frame as the shutdown control,
wakes the worker, waits up to 2 s, and joins it. A worker method that never
returns cannot be forcibly cancelled; after the deadline its thread is
detached safely because the ring allocations remain owned by `Arc`.

### Timeout and poison

A timed-out call must never let a late reply be consumed as a later call's
result, so `WorkerTransport` latches a permanent poison flag:

- `call` first checks the poison flag and `JoinHandle::is_finished()`. If
  either is set, it returns `WorkerDead` immediately, without writing a
  request or reading the response ring.
- A call whose bounded wait elapses with no response returns `Timeout` and
  latches the poison flag (a `Release` store). The poison is **permanent** for
  the life of the transport — it is never cleared.
- Thereafter every `call` returns `WorkerDead` immediately, so a late reply to
  the timed-out call stays unconsumed in the response ring rather than being
  mistaken for a later call's reply.
- If the worker thread has exited (the `JoinHandle` is finished), `call`
  returns `WorkerDead` without poisoning — there is no late reply to worry
  about.
- Recover by dropping the transport and spawning a fresh worker.

The production response deadline is `RESPONSE_DEADLINE` (10 s); tests inject a
shorter `response_deadline` to exercise the timeout path without sleeping. The
`timeout_poisons_transport_and_rejects_stale_response` test is the regression
for the stale-reply guarantee.

### Events

```rust
push_event(bytes)?;       // from the current native worker thread
events.try_recv();
events.has_data();
events.drain_into(&mut out);
```

The event sink is worker-local and backed by a third SPSC ring. `push_event`
outside a worker loop is a no-op; a full event ring returns `BufferFull`.

## Framing, codec, and errors

Generated clients serialize arguments/results with postcard. Every server reply
is postcard-encoded as one of:

```rust
Response::Ok(Vec<u8>)
Response::Server { method, message }
Response::Decode { method, message }
```

This keeps a successful zero-byte/unit return distinguishable from an error.
Useful helpers are `encode`, `decode`, `make_response`, and `unwrap_response`.

`RpcError` covers codec/decode failures, unknown methods, full/empty/undersized
rings, corrupt frames/backing, transport/server failures, a dead worker, and
response timeout.

## Performance

Current optimized native raw-ring and end-to-end service-RPC measurements are in
[`docs/research/performance-benchmarks.md`](../research/performance-benchmarks.md).
On the 2026-07-10 host, median service-RPC latency was 2.4 µs for a 64 B
payload each way and 76.4 µs / 1,636 MiB/s aggregate for 64 KiB each way.

See [`web-shared-memory.md`](web-shared-memory.md) for the web exports and JS
worker contract.
