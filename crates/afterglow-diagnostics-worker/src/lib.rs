//! Afterglow's native RPC adapter for the profiling WebSocket client.
//! One I/O worker sends records to the CLI server.
#![cfg(unix)]

use afterglow_rpc::{RpcError, RpcResult, ServeFuture};
use afterglow_rpc_macros::rpc;
use afterglow_telemetry::publisher::Publisher;
use afterglow_telemetry::{Clock, MonotonicClock};
use std::cell::RefCell;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_BATCH_RECORDS: usize = 1024;
pub const DEFAULT_PORT: u16 = 8086;

#[rpc(worker = DiagnosticsWorker)]
pub trait Diagnostics {
    /// Connect to the profiling server. Each connection starts a capture epoch.
    async fn start() -> RpcResult<String>;
    async fn ingest(epoch: u32, opcode: u8, payload: Vec<u8>) -> RpcResult<u8>;
    async fn finish() -> RpcResult<String>;
}

#[derive(Default)]
pub struct DiagnosticsWorker {
    publisher: RefCell<Option<Publisher>>,
}

impl DiagnosticsWorker {
    pub fn connect(port: u16) -> RpcResult<Self> {
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| failure("Diagnostics entropy is unavailable"))?;
        let session = std::array::from_fn(|i| {
            u32::from_le_bytes(random[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let publisher = Publisher::new(
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            session,
            MAX_FRAME_BYTES,
            Duration::from_millis(100),
        )
        .map_err(|_| failure("Profiling port is unavailable"))?;
        Ok(Self {
            publisher: RefCell::new(Some(publisher)),
        })
    }

    pub fn address(&self) -> RpcResult<SocketAddr> {
        Ok(self.publisher.borrow().as_ref()
            .ok_or_else(|| failure("Profiling port is disabled"))?.address())
    }

    fn poll_capture(&self) -> RpcResult<String> {
        let mut slot = self.publisher.borrow_mut();
        let publisher = slot
            .as_mut()
            .ok_or_else(|| failure("Profiling port is disabled"))?;
        let connected = publisher
            .poll()
            .map_err(|_| failure("Profiling connection failed"))?;
        Ok(serde_json::json!({ "connected": connected, "session": publisher.session(), "epoch": publisher.epoch(),
            "nativeTick": MonotonicClock.now().to_string(), "maxFrameBytes": MAX_FRAME_BYTES,
            "maxBatchRecords": MAX_BATCH_RECORDS }).to_string())
    }

    fn ingest_capture(&self, epoch: u32, opcode: u8, payload: &[u8]) -> RpcResult<u8> {
        let mut slot = self.publisher.borrow_mut();
        let publisher = slot
            .as_mut()
            .ok_or_else(|| failure("Profiling port is disabled"))?;
        if epoch != publisher.epoch() { return Ok(2); }
        publisher
            .publish(opcode, payload)
            .map(|connected| if connected { 0 } else { 2 })
            .map_err(|_| failure("Profiling frame failed"))
    }

    fn finish_capture(&self) -> RpcResult<String> {
        let mut slot = self.publisher.borrow_mut();
        let publisher = slot
            .as_mut()
            .ok_or_else(|| failure("Profiling port is disabled"))?;
        publisher
            .finish()
            .map_err(|_| failure("Profiling disconnect failed"))?;
        Ok("{\"connected\":false}".into())
    }
}

impl DiagnosticsServer for DiagnosticsWorker {
    fn start(&self) -> ServeFuture {
        let result = self.poll_capture();
        Box::pin(async move { afterglow_rpc::encode(&result?) })
    }
    fn ingest(&self, epoch: u32, opcode: u8, payload: Vec<u8>) -> ServeFuture {
        let result = self.ingest_capture(epoch, opcode, &payload);
        Box::pin(async move { afterglow_rpc::encode(&result?) })
    }
    fn finish(&self) -> ServeFuture {
        let result = self.finish_capture();
        Box::pin(async move { afterglow_rpc::encode(&result?) })
    }
}

fn failure(message: &'static str) -> RpcError {
    RpcError::Server(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_and_invalid_commands_have_no_side_effects() {
        let worker = DiagnosticsWorker::default();
        assert!(worker.poll_capture().is_err());
        assert!(worker.ingest_capture(0, 2, &[]).is_err());
        assert!(worker.finish_capture().is_err());
        assert!(DiagnosticsWorker::connect(0).is_err());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let worker = DiagnosticsWorker::connect(listener.local_addr().unwrap().port()).unwrap();
        assert!(worker.address().unwrap().ip().is_loopback());
        assert!(worker.ingest_capture(0, 2, &vec![0; MAX_FRAME_BYTES]).is_err());
        assert!(worker.ingest_capture(0, 55, &[]).is_err());
        assert!(
            worker
                .poll_capture()
                .unwrap()
                .contains("\"connected\":false")
        );
    }
    #[test]
    fn stale_batches_cannot_enter_a_new_viewer_epoch() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let worker = DiagnosticsWorker::connect(listener.local_addr().unwrap().port()).unwrap();
        let server = std::thread::spawn(move || {
            let socket = afterglow_telemetry::websocket::accept(listener.accept().unwrap().0).unwrap();
            (listener, socket)
        });
        let status: serde_json::Value = serde_json::from_str(&worker.poll_capture().unwrap()).unwrap();
        assert_eq!(status["connected"], true);
        let epoch = status["epoch"].as_u64().unwrap() as u32;
        let (listener, _socket) = server.join().unwrap();
        assert_eq!(worker.ingest_capture(epoch + 1, 2, &[]).unwrap(), 2);
        worker.finish_capture().unwrap();
        let server = std::thread::spawn(move || afterglow_telemetry::websocket::accept(listener.accept().unwrap().0).unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let status: serde_json::Value = serde_json::from_str(&worker.poll_capture().unwrap()).unwrap();
            if status["connected"] == true { assert_eq!(status["epoch"], epoch + 1); break; }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let _next = server.join().unwrap();
        assert_eq!(worker.ingest_capture(epoch, 2, &[]).unwrap(), 2);
    }

    #[test]
    fn generated_startup_gives_transport_ownership_to_the_host() {
        let (client, events) =
            DiagnosticsClient::spawn_worker(DiagnosticsWorker::default()).unwrap();
        let transport = std::sync::Arc::new(client.into_transport());
        drop(events);
        drop(transport);
    }
}
