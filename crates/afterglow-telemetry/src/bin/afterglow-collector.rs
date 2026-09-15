//! Local WebSocket profiling server. Applications connect to this CLI.
use afterglow_telemetry::connection::{self, CaptureHello, FINISH, HELLO, PROTOCOL_VERSION};
use afterglow_telemetry::websocket::{self, Socket};
use afterglow_telemetry::{Collector, CollectorError, CollectorLimits, TraceRecord};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::time::Duration;

fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidInput, message) }

fn receive(collector: &mut Collector, socket: &mut Socket, records: &mut [TraceRecord], max_bytes: usize) -> io::Result<&'static str> {
    loop {
        let bytes = match websocket::receive(socket) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok("application-disconnected"),
            Err(error) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => return Ok("duration"),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok("application-disconnected"),
            Err(error) => return Err(error),
        };
        if bytes.len() > max_bytes { return Err(invalid("Profiling message exceeds negotiated capacity")); }
        if bytes[0] == FINISH && bytes.len() == 1 { return Ok("finished"); }
        match connection::ingest_frame(collector, bytes[0], &bytes[1..], records, max_bytes - 1) {
            Ok(()) => (),
            Err(connection::IngestError::Collector(CollectorError::Capacity(_))) => return Ok("capacity"),
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Invalid profiling data: {error:?}"))),
        }
    }
}

fn limits(max_bytes: usize) -> CollectorLimits {
    CollectorLimits {
        sources: 64, descriptors: 4096, metadata_bytes: max_bytes.min(1_048_576),
        records: (max_bytes / 40).max(1), batches: (max_bytes / 100).max(1),
        metric_samples: (max_bytes / 24).max(1), raw_bytes: max_bytes,
    }
}

fn capture(mut args: impl Iterator<Item = String>) -> io::Result<()> {
    let address: SocketAddr = args.next().unwrap_or_else(|| "127.0.0.1:8086".into()).parse().map_err(|_| invalid("Invalid profiling address"))?;
    let directory = PathBuf::from(args.next().unwrap_or_else(|| "captures".into()));
    let seconds: u64 = args.next().unwrap_or_else(|| "10".into()).parse().map_err(|_| invalid("Invalid capture duration"))?;
    let max_bytes: usize = args.next().unwrap_or_else(|| "67108864".into()).parse().map_err(|_| invalid("Invalid capture byte limit"))?;
    if args.next().is_some() || !address.ip().is_loopback() || !(1..=86_400).contains(&seconds) || !(32..=u32::MAX as usize).contains(&max_bytes) {
        return Err(invalid("Invalid capture limits or non-local address"));
    }
    let listener = TcpListener::bind(address)?;
    println!("{}", serde_json::json!({"listening": format!("ws://{}/", listener.local_addr()?), "protocol": PROTOCOL_VERSION}));
    io::stdout().flush()?;
    let (stream, _) = listener.accept()?;
    drop(listener);
    let mut socket = websocket::accept(stream)?;
    let bytes = websocket::receive(&mut socket)?.ok_or_else(|| invalid("Missing profiling header"))?;
    if bytes[0] != HELLO { return Err(invalid("Missing profiling header")); }
    let hello: CaptureHello = serde_json::from_slice(&bytes[1..]).map_err(|_| invalid("Invalid profiling header"))?;
    if hello.protocol != PROTOCOL_VERSION || !(256..=websocket::MAX_MESSAGE_BYTES).contains(&hello.max_frame_bytes) {
        return Err(invalid("Unsupported profiling protocol or message size"));
    }
    let mut collector = Collector::new(hello.session, hello.epoch, limits(max_bytes)).map_err(|_| invalid("Invalid capture identity or limits"))?;
    let mut records = vec![TraceRecord::default(); (hello.max_frame_bytes - 97) / 40];
    fs::create_dir_all(&directory)?;
    let directory = fs::canonicalize(directory)?;
    let session = hello.session.iter().map(|word| format!("{word:08x}")).collect::<String>();
    let final_path = directory.join(format!("capture-{session}-{}.dgtl", hello.epoch));
    let partial_path = final_path.with_extension("partial");
    if final_path.try_exists()? { return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Capture already exists")); }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut output = options.open(&partial_path)?;
    socket.get_mut().reset(Duration::from_secs(seconds))?;
    let result = receive(&mut collector, &mut socket, &mut records, hello.max_frame_bytes);
    // Disconnect before file output. The application does not wait for disk I/O.
    drop(socket);
    collector.write_raw(&mut output)?;
    if output.metadata()?.len() != collector.raw_bytes() as u64 { return Err(io::Error::other("Capture byte accounting mismatch")); }
    output.sync_all()?;
    drop(output);
    fs::hard_link(&partial_path, &final_path)?;
    fs::remove_file(&partial_path)?;
    #[cfg(unix)] File::open(&directory)?.sync_all()?;
    println!("{}", serde_json::json!({"capture": final_path, "bytes": collector.raw_bytes(), "records": collector.record_count(), "reason": result.as_ref().copied().unwrap_or("connection-error")}));
    result.map(|_| ())
}
const USAGE: &str = "afterglow-collector capture [127.0.0.1:8086] [directory] [seconds=10] [max-bytes=67108864]\nafterglow-collector summary FILE [source-id|-] [max-bytes=67108864]\nafterglow-collector records FILE [source-id|-] [name|-] [limit=1000] [offset=0] [max-bytes=67108864]\nafterglow-collector spans FILE [source-id|-] [name|-] [limit=1000] [offset=0] [max-bytes=67108864]\nafterglow-collector correlate FILE CORRELATION [source-id|-] [limit=1000] [offset=0] [max-bytes=67108864]\nafterglow-collector metrics FILE [source-id|-] [name|-] [limit=1000] [offset=0] [max-bytes=67108864]\nafterglow-collector chrome FILE [max-bytes=67108864]";

fn analyze(command: &str, mut args: impl Iterator<Item = String>) -> io::Result<()> {
    let path = args.next().ok_or_else(|| invalid(USAGE))?;
    let correlation = if command == "correlate" {
        let id = args.next().ok_or_else(|| invalid("Missing correlation ID"))?.parse::<u64>().map_err(|_| invalid("Invalid correlation ID"))?;
        if id == 0 { return Err(invalid("Correlation zero means no operation")); }
        Some(id)
    } else { None };
    let source = if command == "chrome" { None } else {
        args.next().filter(|value| value != "-").map(|value| value.parse::<u32>().map_err(|_| invalid("Invalid source ID"))).transpose()?
    };
    let (name, limit, offset) = if matches!(command, "records" | "spans" | "metrics" | "correlate") {
        let name = if command == "correlate" { None } else { args.next().filter(|value| value != "-") };
        let limit = args.next().unwrap_or_else(|| "1000".into()).parse::<usize>().map_err(|_| invalid("Invalid record limit"))?;
        let offset = args.next().unwrap_or_else(|| "0".into()).parse::<usize>().map_err(|_| invalid("Invalid record offset"))?;
        if !(1..=10_000).contains(&limit) { return Err(invalid("Record limit must be 1..10000")); }
        (name, limit, offset)
    } else { (None, 0, 0) };
    let max_bytes = args.next().unwrap_or_else(|| "67108864".into()).parse::<usize>().map_err(|_| invalid("Invalid file byte limit"))?;
    if args.next().is_some() || !(32..=u32::MAX as usize).contains(&max_bytes) { return Err(invalid(USAGE)); }
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > max_bytes as u64 { return Err(invalid("Expected a regular file within the byte limit")); }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1).read_to_end(&mut bytes)?;
    let collector = Collector::read_raw(&bytes, limits(max_bytes))?;
    let output = io::stdout().lock();
    match command {
        "summary" => collector.write_summary_json(output, source),
        "records" => collector.write_records_json(output, source, name.as_deref(), offset, limit),
        "spans" => collector.write_spans_json(output, source, name.as_deref(), offset, limit),
        "metrics" => collector.write_metrics_json(output, source, name.as_deref(), offset, limit),
        "correlate" => collector.write_correlation_json(output, correlation.expect("parsed correlation"), source, offset, limit),
        "chrome" => collector.write_chrome_trace(output),
        _ => Err(invalid(USAGE)),
    }
}
fn run() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("capture") => capture(args),
        Some(command @ ("summary" | "records" | "spans" | "correlate" | "metrics" | "chrome")) => analyze(command, args),
        Some("--help" | "-h") => { println!("{USAGE}"); Ok(()) },
        _ => Err(invalid(USAGE)),
    }
}
fn main() {
    if let Err(error) = run() { eprintln!("{}", serde_json::json!({"error": error.to_string()})); std::process::exit(1); }
}
