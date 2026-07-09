//! Native worker transport: workers are OS threads, RPC is over engineered
//! mpsc channels that post results/events back to the main game context.
//! **No web/JS messages** — this is pure Rust thread-to-thread.
//!
//! `spawn_worker` (generated per service by the `#[rpc]` macro) creates the
//! channel pair, spawns the worker thread running the generated `serve` loop,
//! and returns a client + an event receiver the game loop drains each frame.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;

use crate::{RpcError, RpcResult, Transport};

/// (method_id, args) — main -> worker.
pub(crate) type Req = (u32, Vec<u8>);
/// worker -> main (RPC response).
pub(crate) type Resp = RpcResult<Vec<u8>>;

/// Client side of a native worker channel. `call` blocks until the worker
/// responds — the game loop stays in control (no async runtime needed).
pub struct ChannelTransport {
    req_tx: Sender<Req>,
    resp_rx: Receiver<Resp>,
}
impl ChannelTransport {
    pub fn new(req_tx: Sender<Req>, resp_rx: Receiver<Resp>) -> Self {
        Self { req_tx, resp_rx }
    }
}
impl Transport for ChannelTransport {
    fn call(&self, _service: &str, method: u32, args: &[u8]) -> RpcResult<Vec<u8>> {
        self.req_tx
            .send((method, args.to_vec()))
            .map_err(|_| RpcError::Transport("worker request channel closed".into()))?;
        self.resp_rx
            .recv()
            .map_err(|_| RpcError::Transport("worker died".into()))?
    }
}

/// Worker -> main event stream. The game loop drains this each frame
/// (`try_recv`) for unsolicited pushes (e.g. physics state updates).
pub struct EventReceiver {
    rx: Receiver<Vec<u8>>,
}
impl EventReceiver {
    pub fn new(rx: Receiver<Vec<u8>>) -> Self { Self { rx } }
    /// Non-blocking: take one pending event, if any.
    pub fn try_recv(&self) -> Option<Vec<u8>> {
        self.rx.try_recv().ok()
    }
    /// Drain all pending events into `out`.
    pub fn drain_into(&self, out: &mut Vec<Vec<u8>>) {
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
    }
}

// A worker sets its event sender on its thread so the impl can push events
// to the main game context via [`push_event`].
static EVENT_TX: Mutex<Option<Sender<Vec<u8>>>> = Mutex::new(None);

/// Set the current thread's event channel (called by the generated worker loop).
pub fn set_event_sender(tx: Sender<Vec<u8>>) {
    *EVENT_TX.lock().expect("event tx lock") = Some(tx);
}

/// Push an event from a worker thread to the main game context. No-op if not
/// on a worker thread.
pub fn push_event(bytes: Vec<u8>) {
    if let Some(tx) = EVENT_TX.lock().expect("event tx lock").as_ref() {
        let _ = tx.send(bytes);
    }
}
