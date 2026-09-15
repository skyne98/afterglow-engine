#![cfg(feature = "collector")]
use afterglow_telemetry::*;
use serde_json::Value;

const LIMITS: CollectorLimits = CollectorLimits { sources: 3, descriptors: 16, metadata_bytes: 2048,
    records: 128, batches: 16, metric_samples: 16, raw_bytes: 16384 };
const SITES: [Descriptor; 4] = [
    Descriptor::new(CategoryId(0), "test", "slice", DescriptorKind::Span, ArgumentDescriptor::NONE, ArgumentDescriptor::NONE),
    Descriptor::new(CategoryId(0), "test", "request", DescriptorKind::AsyncSpan, ArgumentDescriptor::NONE, ArgumentDescriptor::NONE),
    Descriptor::new(CategoryId(0), "test", "event", DescriptorKind::Instant, ArgumentDescriptor::NONE, ArgumentDescriptor::NONE),
    Descriptor::new(CategoryId(0), "test", "flow", DescriptorKind::Flow, ArgumentDescriptor::NONE, ArgumentDescriptor::NONE),
];
const METRICS: [MetricDescriptor; 2] = [
    MetricDescriptor::new(CategoryId(0), "test", "count", MetricKind::Counter, Unit::Count),
    MetricDescriptor::new(CategoryId(0), "test", "sizes", MetricKind::HistogramLog2, Unit::Bytes),
];
fn collector() -> Collector {
    let mut collector = Collector::new([1, 2, 3, 4], 7, LIMITS).unwrap();
    for id in 1..=2 {
        collector.register_source(SourceRegistration { source_id: id, producer_generation: id + 1,
            clock_generation: 3, process_id: 123, name: "test source",
            clock: ClockMapping { rate_numerator: id as u64, uncertainty_ns: 9, ..ClockMapping::native(id) },
            descriptors: &SITES, metric_descriptors: &METRICS }).unwrap();
    }
    collector
}
fn record(tick: u64, descriptor: u32, phase: TracePhase, correlation: u64) -> TraceRecord {
    TraceRecord { timestamp: tick, descriptor, phase: phase as u8, correlation, argument0: u64::MAX, ..TraceRecord::default() }
}
fn batch(collector: &mut Collector, source: u32, sequence: u64, drops: u64, records: &[TraceRecord]) {
    collector.ingest(BatchHeader { session: [1, 2, 3, 4], source_id: source, producer_generation: source + 1,
        clock_generation: 3, clock_domain: source, epoch: 7, ticks_per_second: 1_000_000_000,
        first_sequence: sequence, next_sequence: sequence + records.len() as u64,
        record_count: records.len() as u32, dropped_records: drops, overwritten_records: 0, flags: 0 }, records).unwrap();
}
fn json(write: impl FnOnce(&mut Vec<u8>) -> std::io::Result<()>) -> Value {
    let mut output = Vec::new(); write(&mut output).unwrap(); serde_json::from_slice(&output).unwrap()
}
fn fixture() -> Collector {
    let mut c = collector();
    use TracePhase::*;
    // Nested synchronous calls can share context zero. Async calls can finish in a different order.
    batch(&mut c, 1, 0, 0, &[
        record(10, 0, SpanBegin, 0), record(12, 0, SpanBegin, 0), record(15, 0, SpanEnd, 0),
        record(16, 1, AsyncBegin, u64::MAX), record(17, 1, AsyncBegin, 6), record(20, 1, AsyncEnd, u64::MAX),
    ]);
    batch(&mut c, 1, 6, 0, &[
        record(21, 1, AsyncEnd, 6), record(30, 0, SpanEnd, 0), record(31, 0, SpanBegin, 9),
        record(31, 0, SpanEnd, 9), record(32, 0, SpanEnd, 99), record(33, 0, SpanBegin, 99),
    ]);
    // The same correlation on another source is not an interval continuation.
    batch(&mut c, 2, 0, 0, &[
        record(1, 1, AsyncEnd, u64::MAX), record(2, 1, AsyncBegin, u64::MAX),
        record(22, 1, AsyncEnd, u64::MAX), record(23, 3, FlowStart, u64::MAX),
        record(24, 3, FlowEnd, u64::MAX),
    ]);
    c.ingest_metrics(ProducerIdentity { session: [1, 2, 3, 4], source_id: 1, generation: 2,
        clock_domain: 1, clock_generation: 3 }, u64::MAX, &[
        MetricSample { metric: 0, bucket: 0, value: u64::MAX }, MetricSample { metric: 1, bucket: 5, value: 3 },
    ]).unwrap();
    c
}
fn typed_fixture() -> Collector {
    static SITES: [Descriptor; 4] = [
        Descriptor::new(CategoryId(0), "test", "typed span", DescriptorKind::Span,
            ArgumentDescriptor::new("value", ArgumentType::Signed, Unit::Count),
            ArgumentDescriptor::new("value", ArgumentType::Unsigned, Unit::Samples)),
        Descriptor::new(CategoryId(0), "test", "float and bool", DescriptorKind::Instant,
            ArgumentDescriptor::new("ratio", ArgumentType::FloatBits, Unit::Percent),
            ArgumentDescriptor::new("active", ArgumentType::Boolean, Unit::None)),
        Descriptor::new(CategoryId(0), "test", "identity and bytes", DescriptorKind::Instant,
            ArgumentDescriptor::new("id", ArgumentType::Identifier, Unit::None),
            ArgumentDescriptor::new("size", ArgumentType::Bytes, Unit::Bytes)),
        Descriptor::new(CategoryId(0), "test", "duration", DescriptorKind::Instant,
            ArgumentDescriptor::new("time", ArgumentType::Duration, Unit::Nanoseconds), ArgumentDescriptor::NONE),
    ];
    struct Ticks(std::cell::Cell<u64>);
    impl Clock for Ticks { fn now(&self) -> u64 { let tick = self.0.get(); self.0.set(tick + 1); tick } }
    let mut recorder = Recorder::new(&SITES, 16, Ticks(std::cell::Cell::new(0))).unwrap();
    recorder.arm(CaptureConfig::all(7)).unwrap();
    assert_eq!(recorder.span_begin(DescriptorId(0), TraceContext(17), i64::MIN as u64, u64::MAX), RecordStatus::Recorded);
    assert_eq!(recorder.span_end(DescriptorId(0), TraceContext(17), (-1_i64) as u64, 0), RecordStatus::Recorded);
    for (value, active) in [(-2.5_f64, 1), (f64::INFINITY, 2), (f64::NAN, 0), (-0.0, 0),
        (f64::NEG_INFINITY, 1), (f64::from_bits(1), 0), (f64::MAX, 1)] {
        assert_eq!(recorder.instant(DescriptorId(1), TraceContext(17), value.to_bits(), active), RecordStatus::Recorded);
    }
    assert_eq!(recorder.instant(DescriptorId(2), TraceContext(17), u64::MAX, u64::MAX), RecordStatus::Recorded);
    assert_eq!(recorder.instant(DescriptorId(3), TraceContext(17), u64::MAX, 49), RecordStatus::Recorded);
    recorder.stop().unwrap();
    let snapshot = recorder.snapshot().unwrap();
    let header = BatchHeader::from_snapshot(ProducerIdentity { session: [1, 2, 3, 4], source_id: 1,
        generation: 2, clock_domain: 1, clock_generation: 3 }, 1_000_000_000, &snapshot).unwrap();
    let mut encoded = vec![0; encoded_batch_len(snapshot.records.len()).unwrap()];
    encode_batch_into(header, snapshot.records, &mut encoded).unwrap();
    let mut records = vec![TraceRecord::default(); snapshot.records.len()];
    let (header, _) = decode_batch_into(&encoded, &mut records).unwrap();
    let mut c = Collector::new([1, 2, 3, 4], 7, LIMITS).unwrap();
    c.register_source(SourceRegistration { source_id: 1, producer_generation: 2, clock_generation: 3,
        process_id: 123, name: "typed source", clock: ClockMapping::native(1),
        descriptors: &SITES, metric_descriptors: &[] }).unwrap();
    c.ingest(header, &records).unwrap();
    let mut file = Vec::new(); c.write_raw(&mut file).unwrap();
    Collector::read_raw(&file, LIMITS).unwrap()
}
#[test]
fn typed_arguments_survive_recorder_batch_file_and_analysis() {
    let c = typed_fixture();
    let result = json(|out| c.write_records_json(out, None, None, 0, 100));
    let records = result["records"].as_array().unwrap();
    let args = &records[0]["arguments"];
    assert_eq!(args[0], serde_json::json!({"slot": 0, "name": "value", "type": "Signed",
        "unit": "Count", "raw": (i64::MIN as u64).to_string(), "value": i64::MIN.to_string(), "error": null}));
    assert_eq!(args[1]["name"], "value"); assert_eq!(args[1]["slot"], 1);
    assert_eq!(args[1]["value"], u64::MAX.to_string());
    assert_eq!(records[2]["arguments"][0]["value"], -2.5);
    assert_eq!(records[2]["arguments"][1]["value"], true);
    for index in [3, 4, 6] {
        assert_eq!(records[index]["arguments"][0]["value"], Value::Null);
        assert_eq!(records[index]["arguments"][0]["error"], "non-finite-float");
        assert_eq!(records[index]["arguments"][0]["raw"], records[index]["argument0"]);
    }
    assert_eq!(records[3]["arguments"][1]["value"], Value::Null);
    assert_eq!(records[3]["arguments"][1]["error"], "invalid-boolean");
    assert_eq!(records[4]["arguments"][1]["value"], false);
    assert_eq!(records[5]["arguments"][0]["value"].as_f64().unwrap().to_bits(), (-0.0_f64).to_bits());
    assert_eq!(records[7]["arguments"][0]["value"].as_f64().unwrap().to_bits(), 1);
    assert_eq!(records[8]["arguments"][0]["value"].as_f64().unwrap(), f64::MAX);
    assert_eq!(records[9]["arguments"][0]["type"], "Identifier");
    assert_eq!(records[9]["arguments"][1]["unit"], "Bytes");
    assert_eq!(records[9]["arguments"][1]["value"], u64::MAX.to_string());
    assert_eq!(records[10]["arguments"].as_array().unwrap().len(), 1);
    assert_eq!(records[10]["arguments"][0]["type"], "Duration");
    assert_eq!(records[10]["arguments"][0]["value"], u64::MAX.to_string());
    let span = json(|out| c.write_spans_json(out, None, None, 0, 1));
    assert_eq!(span["spans"][0]["begin_arguments"], records[0]["arguments"]);
    assert_eq!(span["spans"][0]["end_arguments"], records[1]["arguments"]);
    assert_eq!(span["spans"][0]["end_arguments"][0]["value"], "-1");
    let correlation = json(|out| c.write_correlation_json(out, 17, None, 0, 100));
    assert_eq!(correlation["records"], result["records"]);
    let undeclared = json(|out| fixture().write_records_json(out, None, None, 0, 1));
    assert_eq!(undeclared["records"][0]["arguments"], serde_json::json!([]));
}
#[test]
fn spans_pair_nested_and_async_records_with_exact_timings_and_pages() {
    let c = fixture();
    let result = json(|out| c.write_spans_json(out, None, None, 0, 2));
    assert_eq!(result["matched"], 6); assert_eq!(result["next_offset"], 2);
    assert_eq!(result["spans"][0]["duration_ns"], "40");
    assert_eq!(result["spans"][0]["source_id"], 2);
    assert_eq!(result["spans"][0]["correlation"], u64::MAX.to_string());
    assert_eq!(result["spans"][1]["duration_ns"], "20");
    assert_eq!(result["spans"][1]["begin_sequence"], "0");
    assert_eq!(result["spans"][1]["end_sequence"], "7");
    assert_eq!(result["spans"][1]["argument0"], u64::MAX.to_string());
    assert_eq!(result["statistics"][0]["count"], 3);
    assert_eq!(result["statistics"][0]["total_ns"], "23");
    assert_eq!(result["statistics"][0]["mean_ns"], "7");
    assert_eq!(result["statistics"][0]["p50_ns"], "3");
    assert_eq!(result["statistics"][0]["p95_ns"], "20");
    assert_eq!(result["statistics"][0]["min_ns"], "0");
    assert_eq!(result["statistics"][0]["missing_begins"], 1);
    assert_eq!(result["statistics"][0]["missing_ends"], 1);
    assert_eq!(result["statistics"][2]["mean_ns"], Value::Null);
    assert_eq!(result["sources"][0]["clock_uncertainty_ns"], "9");
    let page = json(|out| c.write_spans_json(out, Some(1), Some("request"), 1, 1));
    assert_eq!(page["matched"], 2); assert_eq!(page["next_offset"], Value::Null);
    assert_eq!(page["statistics"].as_array().unwrap().len(), 1);
    assert_eq!(page["spans"][0]["correlation"], "6");
    assert_eq!(json(|out| c.write_spans_json(out, None, None, usize::MAX, 1))["returned"], 0);
}
#[test]
fn gaps_drops_and_duplicate_async_ids_do_not_create_false_durations() {
    use TracePhase::*;
    let mut c = collector();
    batch(&mut c, 1, 0, 0, &[record(1, 0, SpanBegin, 1)]);
    batch(&mut c, 1, 2, 0, &[record(3, 0, SpanEnd, 1), record(4, 0, SpanBegin, 2)]);
    batch(&mut c, 1, 4, 1, &[record(5, 0, SpanEnd, 2), record(6, 0, SpanBegin, 3)]);
    batch(&mut c, 1, 6, 1, &[
        record(7, 0, SpanEnd, 3), record(8, 1, AsyncBegin, 8), record(9, 1, AsyncBegin, 8),
        record(10, 1, AsyncEnd, 8), record(11, 1, AsyncEnd, 8),
        record(12, 1, AsyncBegin, 8), record(14, 1, AsyncEnd, 8),
    ]);
    let result = json(|out| c.write_spans_json(out, Some(1), None, 0, 100));
    assert_eq!(result["matched"], 1); assert_eq!(result["spans"][0]["duration_ns"], "2");
    assert_eq!(result["statistics"][0]["missing_begins"], 2);
    assert_eq!(result["statistics"][0]["missing_ends"], 2);
    assert_eq!(result["statistics"][0]["excluded_records"], 2);
    assert_eq!(result["statistics"][1]["ambiguous_pairs"], 2);
    assert_eq!(result["sources"][0]["sequence_gaps"], "1");
    assert_eq!(result["sources"][0]["dropped_records"], "1");
}
#[test]
fn correlations_and_metrics_keep_exact_values_identity_and_filters() {
    let c = fixture();
    let result = json(|out| c.write_correlation_json(out, u64::MAX, None, 0, 3));
    assert_eq!(result["matched"], 7); assert_eq!(result["next_offset"], 3);
    assert_eq!(result["records"][0]["source_id"], 1);
    assert_eq!(result["records"][2]["source_id"], 2);
    assert_eq!(result["records"][2]["sequence"], "0");
    assert_eq!(result["sources"][1]["producer_generation"], 3);
    let filtered = json(|out| c.write_correlation_json(out, u64::MAX, Some(1), 0, 100));
    assert_eq!(filtered["matched"], 2);
    assert_eq!(json(|out| c.write_correlation_json(out, 42, None, 0, 1))["matched"], 0);
    let metrics = json(|out| c.write_metrics_json(out, Some(1), None, 0, 1));
    assert_eq!(metrics["matched"], 2); assert_eq!(metrics["next_offset"], 1);
    assert_eq!(metrics["metrics"][0]["value"], u64::MAX.to_string());
    assert_eq!(metrics["metrics"][0]["tick"], u64::MAX.to_string());
    let histogram = json(|out| c.write_metrics_json(out, None, Some("sizes"), 0, 10));
    assert_eq!(histogram["matched"], 1); assert_eq!(histogram["metrics"][0]["bucket"], 5);
    assert_eq!(histogram["metrics"][0]["value"], "3");
    assert_eq!(histogram["next_offset"], Value::Null);
}
#[test]
fn nested_duration_totals_do_not_overflow_u64() {
    let mut c = collector();
    batch(&mut c, 1, 0, 0, &[
        record(0, 0, TracePhase::SpanBegin, 0), record(0, 0, TracePhase::SpanBegin, 0),
        record(u64::MAX, 0, TracePhase::SpanEnd, 0), record(u64::MAX, 0, TracePhase::SpanEnd, 0),
    ]);
    let result = json(|out| c.write_spans_json(out, Some(1), Some("slice"), 0, 1));
    assert_eq!(result["statistics"][0]["total_ns"], (u64::MAX as u128 * 2).to_string());
    assert_eq!(result["statistics"][0]["mean_ns"], u64::MAX.to_string());
    assert_eq!(result["statistics"][0]["p99_ns"], u64::MAX.to_string());
}
#[test]
fn invalid_queries_never_write_partial_output() {
    let c = fixture();
    let mut out = Vec::new();
    assert!(c.write_spans_json(&mut out, Some(99), None, 0, 1).is_err());
    assert!(c.write_spans_json(&mut out, None, None, 0, 0).is_err());
    assert!(c.write_correlation_json(&mut out, 0, None, 0, 1).is_err());
    assert!(c.write_correlation_json(&mut out, 1, None, 0, 10001).is_err());
    assert!(c.write_metrics_json(&mut out, Some(99), None, 0, 1).is_err());
    assert!(c.write_metrics_json(&mut out, None, None, 0, 10001).is_err());
    assert!(out.is_empty());
}
#[test]
fn cli_analysis_commands_read_the_same_capture() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let directory = Directory(std::env::temp_dir().join(format!("afterglow-analysis-test-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    let file = directory.0.join("input.dgtl");
    fixture().write_raw(std::fs::File::create(&file).unwrap()).unwrap();
    let run = |command: &str, args: &[&str]| std::process::Command::new(env!("CARGO_BIN_EXE_afterglow-collector"))
        .arg(command).arg(&file).args(args).output().unwrap();
    for (command, args, expected) in [
        ("spans", vec!["1", "slice", "1", "1"], 3),
        ("correlate", vec!["18446744073709551615", "1", "1", "1"], 2),
        ("metrics", vec!["1", "sizes"], 1),
    ] {
        let result = run(command, &args); assert!(result.status.success(), "{command}: {:?}", result.stderr);
        assert_eq!(serde_json::from_slice::<Value>(&result.stdout).unwrap()["matched"], expected);
    }
    for (command, args) in [
        ("correlate", vec!["0"]), ("correlate", vec!["18446744073709551616"]),
        ("correlate", vec!["-1"]), ("metrics", vec!["-", "-", "10001"]),
        ("spans", vec!["99"]), ("spans", vec!["-", "-", "0"]),
        ("metrics", vec!["-", "-", "1", "0", "32"]),
    ] {
        let result = run(command, &args); assert!(!result.status.success()); assert!(result.stdout.is_empty());
        assert!(serde_json::from_slice::<Value>(&result.stderr).unwrap()["error"].is_string());
    }
    std::fs::write(&file, b"invalid").unwrap();
    for command in ["spans", "metrics", "correlate"] {
        let result = run(command, if command == "correlate" { &["1"] } else { &[] });
        assert!(!result.status.success()); assert!(result.stdout.is_empty());
    }
    typed_fixture().write_raw(std::fs::File::create(&file).unwrap()).unwrap();
    for command in ["records", "correlate", "spans"] {
        let result = run(command, if command == "correlate" { &["17"] } else { &[] });
        assert!(result.status.success(), "{:?}", result.stderr);
        let result: Value = serde_json::from_slice(&result.stdout).unwrap();
        let argument = if command == "spans" { &result["spans"][0]["end_arguments"][0] }
            else { &result["records"][1]["arguments"][0] };
        assert_eq!(argument["type"], "Signed"); assert_eq!(argument["value"], "-1");
    }
}
