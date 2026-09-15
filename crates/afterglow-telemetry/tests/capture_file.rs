#![cfg(feature = "collector")]
use afterglow_telemetry::*;
use serde_json::Value;

const LIMITS: CollectorLimits = CollectorLimits { sources: 2, descriptors: 4, metadata_bytes: 512,
    records: 3, batches: 3, metric_samples: 3, raw_bytes: 4096 };
const ID: ProducerIdentity = ProducerIdentity { session: [1, 2, 3, 4], source_id: 1,
    generation: 2, clock_domain: 3, clock_generation: 4 };
const SITES: [Descriptor; 1] = [Descriptor::new(CategoryId(0), "test", "event\"name", DescriptorKind::Instant,
    ArgumentDescriptor::new("bytes", ArgumentType::Bytes, Unit::Bytes), ArgumentDescriptor::NONE)];
const METRICS: [MetricDescriptor; 1] = [MetricDescriptor::new(CategoryId(0), "test", "count", MetricKind::Counter, Unit::Count)];
fn fixture() -> Vec<u8> {
    let mut collector = Collector::new(ID.session, 5, LIMITS).unwrap();
    for source_id in 1..=2 {
        collector.register_source(SourceRegistration { source_id, producer_generation: 2, clock_generation: 4, process_id: 6,
            name: "test\nsource", clock: ClockMapping::native(3), descriptors: &SITES, metric_descriptors: &METRICS }).unwrap();
    }
    for index in 0..2 {
        let first = 9_007_199_254_740_993 + index * 2;
        let header = BatchHeader { session: ID.session, source_id: 1, producer_generation: 2, clock_generation: 4,
            clock_domain: 3, epoch: 5, ticks_per_second: 1_000_000_000, first_sequence: first, next_sequence: first + 1,
            record_count: 1, dropped_records: 4, overwritten_records: 2, flags: BATCH_ROLLING };
        let record = TraceRecord { timestamp: u64::MAX - 2 + index, argument0: u64::MAX,
            phase: TracePhase::Instant as u8, ..TraceRecord::default() };
        collector.ingest(header, &[record]).unwrap();
    }
    collector.ingest_metrics(ID, u64::MAX, &[MetricSample { metric: 0, bucket: 0, value: u64::MAX }]).unwrap();
    let mut bytes = Vec::new(); collector.write_raw(&mut bytes).unwrap(); bytes
}
#[test]
fn raw_round_trip_keeps_metadata_batches_metrics_and_exact_integers() {
    let bytes = fixture();
    let collector = Collector::read_raw(&bytes, LIMITS).unwrap();
    let mut output = Vec::new(); collector.write_raw(&mut output).unwrap();
    assert_eq!(output, bytes);
    output.clear(); collector.write_summary_json(&mut output, None).unwrap();
    let summary: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(summary["sources"][0]["records"], 2);
    assert_eq!(summary["sources"][0]["dropped_records"], "4");
    assert_eq!(summary["sources"][0]["sequence_gaps"], "9007199254740992");
    assert_eq!(summary["sources"][0]["descriptors"][0]["name"], "event\"name");
    output.clear(); collector.write_records_json(&mut output, Some(1), Some("event\"name"), 0, 1).unwrap();
    let records: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(records["records"][0]["argument0"], u64::MAX.to_string());
    assert_eq!(records["records"][0]["tick"], (u64::MAX - 2).to_string());
    assert_eq!(records["records"][0]["sequence"], "9007199254740993");
    assert_eq!(records["next_offset"], 1);
    assert_eq!(records["matched"], 2);
    output.clear(); collector.write_records_json(&mut output, Some(1), None, 1, 1).unwrap();
    let last: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(last["next_offset"], Value::Null);
    assert_eq!(last["records"][0]["sequence"], "9007199254740995");
    output.clear(); collector.write_records_json(&mut output, None, None, usize::MAX, 1).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&output).unwrap()["returned"], 0);
    assert!(collector.write_records_json(Vec::new(), Some(999), None, 0, 1).is_err());
    assert!(collector.write_records_json(Vec::new(), None, None, 0, 0).is_err());
}
#[test]
fn truncated_malformed_and_over_capacity_files_fail_without_partial_output() {
    let bytes = fixture();
    for end in 0..bytes.len() { assert!(Collector::read_raw(&bytes[..end], LIMITS).is_err(), "accepted prefix {end}"); }
    let mut trailing = bytes.clone(); trailing.push(0);
    assert!(Collector::read_raw(&trailing, LIMITS).is_err());
    for limits in [CollectorLimits { raw_bytes: bytes.len() - 1, ..LIMITS },
        CollectorLimits { records: 1, ..LIMITS }, CollectorLimits { descriptors: 1, ..LIMITS },
        CollectorLimits { sources: 1, ..LIMITS }, CollectorLimits { metadata_bytes: 1, ..LIMITS }] {
        assert!(Collector::read_raw(&bytes, limits).is_err());
    }
    for offset in 0..bytes.len() {
        let mut changed = bytes.clone(); changed[offset] ^= 255;
        // Any accepted byte change must survive a canonical round trip without data loss.
        if let Ok(collector) = Collector::read_raw(&changed, LIMITS) {
            let mut output = Vec::new(); collector.write_raw(&mut output).unwrap(); assert_eq!(changed, output);
        }
    }
}
#[test]
fn cli_filters_pages_and_rejects_invalid_files() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let path = std::env::temp_dir().join(format!("afterglow-file-test-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let directory = Directory(path);
    let file = directory.0.join("input.dgtl"); std::fs::write(&file, fixture()).unwrap();
    let run = |command: &str, args: &[&str]| std::process::Command::new(env!("CARGO_BIN_EXE_afterglow-collector"))
        .arg(command).arg(&file).args(args).output().unwrap();
    let summary = run("summary", &["1"]); assert!(summary.status.success());
    assert_eq!(serde_json::from_slice::<Value>(&summary.stdout).unwrap()["sources"].as_array().unwrap().len(), 1);
    let page = run("records", &["1", "-", "1", "1"]); assert!(page.status.success());
    assert_eq!(serde_json::from_slice::<Value>(&page.stdout).unwrap()["returned"], 1);
    assert!(run("chrome", &[]).status.success());
    assert!(!run("summary", &["99"]).status.success());
    assert!(!run("records", &["-", "-", "10001"]).status.success());
    std::fs::write(&file, b"bad file").unwrap();
    let bad = run("summary", &[]); assert!(!bad.status.success()); assert!(bad.stdout.is_empty());
    assert!(serde_json::from_slice::<Value>(&bad.stderr).unwrap()["error"].is_string());
}
