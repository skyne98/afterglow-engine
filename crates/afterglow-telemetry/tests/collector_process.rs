#![cfg(feature = "collector")]
use afterglow_telemetry::connection::*;
use afterglow_telemetry::publisher::Publisher;
use afterglow_telemetry::websocket;
use afterglow_telemetry::{BatchHeader, Collector, CollectorLimits, TracePhase, TraceRecord, encode_batch_into};
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
const SESSION: [u32; 4] = [1, 2, 3, 4];
const LIMITS: CollectorLimits = CollectorLimits { sources: 2, descriptors: 8, metadata_bytes: 4096, records: 16, batches: 16, metric_samples: 16, raw_bytes: 16_384 };
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Run { child: Child, directory: PathBuf }
impl Drop for Run {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); let _ = std::fs::remove_dir_all(&self.directory); }
}
fn source() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"source_id":1,"producer_generation":1,"clock_generation":1,"process_id":42,
        "name":"non-engine-app","clock":{"clock_domain":1,"origin_tick":0,"origin_reference_ns":0,"rate_numerator":1,"rate_denominator":1,"uncertainty_ns":0},
        "descriptors":[{"category":0,"category_name":"app","name":"event","kind":"Instant","severity":"Info",
            "argument0":{"name":"","kind":"None","unit":"None"},"argument1":{"name":"","kind":"None","unit":"None"}}],"metric_descriptors":[]})).unwrap()
}
fn publisher(address: SocketAddr) -> Publisher { Publisher::new(address, SESSION, 4096, Duration::from_millis(100)).unwrap() }
fn connect(publisher: &mut Publisher) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !publisher.poll().unwrap() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(2)); }
}
fn start(max_bytes: usize) -> (Publisher, Run, BufReader<std::process::ChildStdout>) {
    let directory = std::env::temp_dir().join(format!("afterglow-ws-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir(&directory).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_afterglow-collector"))
        .args(["capture", "127.0.0.1:0"]).arg(&directory).arg("1").arg(max_bytes.to_string())
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
    let mut run = Run { child, directory };
    let mut stdout = BufReader::new(run.child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let header: serde_json::Value = serde_json::from_str(&line).unwrap();
    let address = header["listening"].as_str().unwrap().strip_prefix("ws://").unwrap().trim_end_matches('/').parse().unwrap();
    let mut publisher = publisher(address);
    connect(&mut publisher);
    (publisher, run, stdout)
}
fn batch(sequence: u64, epoch: u32) -> Vec<u8> {
    let header = BatchHeader { source_id:1, epoch, clock_domain:1, session:SESSION, producer_generation:1, clock_generation:1,
        first_sequence:sequence, next_sequence:sequence+1, record_count:1, ticks_per_second:1_000_000_000, ..BatchHeader::default() };
    let record = TraceRecord { timestamp:sequence+1, phase:TracePhase::Instant as u8, ..TraceRecord::default() };
    let mut bytes = vec![0;136]; encode_batch_into(header, &[record], &mut bytes).unwrap(); bytes
}
fn result(run: &mut Run, stdout: &mut impl BufRead, success: bool) -> serde_json::Value {
    let mut line = String::new(); stdout.read_line(&mut line).unwrap();
    let result: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(run.child.wait().unwrap().success(), success);
    let path = PathBuf::from(result["capture"].as_str().unwrap());
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, result["bytes"].as_u64().unwrap());
    assert_eq!(&bytes[..4], b"DGTL");
    assert_eq!(std::fs::read_dir(&run.directory).unwrap().count(),1);
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777,0o600); }
    result
}
#[test]
fn application_connects_to_cli_websocket_without_tokens() {
    let (mut publisher, mut run, mut stdout) = start(LIMITS.raw_bytes);
    assert!(publisher.publish(REGISTER, &source()).unwrap());
    assert!(publisher.publish(BATCH, &batch(0,publisher.epoch())).unwrap());
    publisher.finish().unwrap();
    let result = result(&mut run,&mut stdout,true);
    assert_eq!(result["records"],1); assert_eq!(result["reason"],"finished");
}
#[test]
fn capacity_and_application_death_preserve_previous_records() {
    for capacity in [false,true] {
        let mut expected = Collector::new(SESSION,1,LIMITS).unwrap();
        serde_json::from_slice::<WireSource>(&source()).unwrap().register(&mut expected).unwrap();
        let max_bytes = expected.raw_bytes()+140;
        let (mut publisher,mut run,mut stdout) = start(max_bytes);
        publisher.publish(REGISTER,&source()).unwrap();
        publisher.publish(BATCH,&batch(0,publisher.epoch())).unwrap();
        if capacity { publisher.publish(BATCH,&batch(1,publisher.epoch())).unwrap(); }
        drop(publisher);
        let result = result(&mut run,&mut stdout,true);
        assert_eq!(result["records"],1); assert_eq!(result["bytes"],max_bytes);
        assert_eq!(result["reason"],if capacity {"capacity"} else {"application-disconnected"});
    }
}
#[test]
fn invalid_capture_preserves_accepted_data() {
    let (mut publisher,mut run,mut stdout) = start(LIMITS.raw_bytes);
    publisher.publish(REGISTER,&source()).unwrap();
    publisher.publish(BATCH,&batch(0,publisher.epoch())).unwrap();
    publisher.publish(BATCH,&batch(0,publisher.epoch())).unwrap();
    let result = result(&mut run,&mut stdout,false);
    assert_eq!(result["records"],1); assert_eq!(result["reason"],"connection-error");
}
#[test]
fn idle_capture_reaches_its_deadline_without_app_failure() {
    let (mut publisher,mut run,mut stdout) = start(LIMITS.raw_bytes);
    let result = result(&mut run,&mut stdout,true);
    assert_eq!(result["records"],0); assert_eq!(result["reason"],"duration");
    assert!(!publisher.poll().unwrap());
}
#[test]
fn a_server_that_does_not_read_cannot_hold_the_publisher() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut publisher = Publisher::new(listener.local_addr().unwrap(),SESSION,65_536,Duration::from_millis(100)).unwrap();
    let server = std::thread::spawn(move || websocket::accept(listener.accept().unwrap().0).unwrap());
    connect(&mut publisher);
    let _socket = server.join().unwrap();
    let bytes = vec![0;65_535];
    let mut timed_out = false;
    for _ in 0..1024 { if publisher.publish(BATCH,&bytes).is_err() { timed_out=true; break; } }
    assert!(timed_out); assert!(!publisher.connected()); assert!(!publisher.publish(BATCH,&[]).unwrap());
}
#[test]
fn local_connection_and_reconnection_preserve_epoch_boundaries() {
    assert!(Publisher::new("0.0.0.0:8086".parse().unwrap(),SESSION,4096,Duration::from_secs(1)).is_err());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut publisher = publisher(listener.local_addr().unwrap());
    assert!(!publisher.publish(BATCH,&[]).unwrap());
    assert!(publisher.publish(55,b"eval").is_err());
    assert!(publisher.publish(BATCH,&[0;4096]).is_err());
    for epoch in [1,2] {
        let listener = listener.try_clone().unwrap();
        let server = std::thread::spawn(move || websocket::accept(listener.accept().unwrap().0).unwrap());
        connect(&mut publisher);
        let mut socket = server.join().unwrap();
        let bytes = websocket::receive(&mut socket).unwrap().unwrap();
        assert_eq!(bytes[0],HELLO);
        let hello: CaptureHello = serde_json::from_slice(&bytes[1..]).unwrap();
        assert_eq!(hello.protocol,PROTOCOL_VERSION); assert_eq!(hello.epoch,epoch); assert_eq!(publisher.epoch(),epoch);
        publisher.finish().unwrap();
    }
}
#[test]
fn absent_server_is_not_an_application_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut publisher = publisher(listener.local_addr().unwrap());
    drop(listener);
    assert!(!publisher.poll().unwrap());
    assert!(!publisher.poll().unwrap());
    assert!(!publisher.connected());
}
