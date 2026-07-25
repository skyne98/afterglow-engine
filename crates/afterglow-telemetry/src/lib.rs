//! Bounded, transport-neutral engine telemetry.
//!
//! `afterglow-telemetry` provides two correlated planes:
//!
//! - fixed metric cells for cheap always-on counters, gauges, maxima, and
//!   logarithmic histograms;
//! - explicitly armed, producer-local finite trace buffers containing compact
//!   spans, async operations, events, and cross-track flows.
//!
//! The hot path never allocates, formats strings, locks, exports, or performs
//! transport I/O. Worker integration drains frozen batches through the engine's
//! existing RPC rings; GPU and TypeScript producers use adapters over the same
//! record ABI.

pub mod batch;
pub mod clock;
pub mod collector;
pub mod descriptor;
pub mod metrics;
pub mod record;
pub mod recorder;

pub use batch::{
    BATCH_HEADER_BYTES, BATCH_MAGIC, BATCH_VERSION, BatchError, BatchHeader, decode_batch_into,
    encode_batch_into, encoded_batch_len,
};
pub use clock::{Clock, ClockMapping, MonotonicClock};
pub use collector::{Collector, CollectorError, RAW_MAGIC, RAW_VERSION, SourceRegistration};
pub use descriptor::{
    ArgumentDescriptor, ArgumentType, CategoryId, Descriptor, DescriptorId, DescriptorKind,
    Severity, Unit,
};
pub use metrics::{
    HISTOGRAM_BUCKETS, MetricBank, MetricDescriptor, MetricId, MetricKind, MetricSample,
    MetricSnapshotError, MetricStatus,
};
pub use record::{TRACE_RECORD_BYTES, TraceContext, TracePhase, TraceRecord};
pub use recorder::{
    CaptureConfig, CaptureError, CaptureSnapshot, CaptureState, CategoryMask, RecordStatus,
    Recorder, SpanGuard,
};

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    const IO: CategoryId = CategoryId(3);
    const GPU: CategoryId = CategoryId(9);
    static DESCRIPTORS: [Descriptor; 4] = [
        Descriptor::new(
            IO,
            "io",
            "asset.pread",
            DescriptorKind::Span,
            ArgumentDescriptor::new("offset", ArgumentType::Bytes, Unit::Bytes),
            ArgumentDescriptor::new("length", ArgumentType::Bytes, Unit::Bytes),
        ),
        Descriptor::new(
            IO,
            "io",
            "asset.complete",
            DescriptorKind::Instant,
            ArgumentDescriptor::new("bytes", ArgumentType::Bytes, Unit::Bytes),
            ArgumentDescriptor::NONE,
        ),
        Descriptor::new(
            GPU,
            "gpu",
            "upload",
            DescriptorKind::AsyncSpan,
            ArgumentDescriptor::new("bytes", ArgumentType::Bytes, Unit::Bytes),
            ArgumentDescriptor::NONE,
        ),
        Descriptor::new(
            IO,
            "rpc",
            "request",
            DescriptorKind::Flow,
            ArgumentDescriptor::NONE,
            ArgumentDescriptor::NONE,
        ),
    ];

    #[derive(Clone)]
    struct TestClock {
        tick: Rc<Cell<u64>>,
        reads: Rc<Cell<u32>>,
    }

    impl TestClock {
        fn new() -> Self {
            Self {
                tick: Rc::new(Cell::new(100)),
                reads: Rc::new(Cell::new(0)),
            }
        }
    }

    impl Clock for TestClock {
        fn now(&self) -> u64 {
            self.reads.set(self.reads.get() + 1);
            let value = self.tick.get();
            self.tick.set(value + 10);
            value
        }
    }

    #[test]
    fn disabled_and_filtered_paths_do_not_read_the_clock() {
        let clock = TestClock::new();
        let reads = clock.reads.clone();
        let mut recorder = Recorder::new(&DESCRIPTORS, 4, clock).unwrap();
        assert_eq!(
            recorder.instant(DescriptorId(1), TraceContext::NONE, 1, 0),
            RecordStatus::Disabled
        );
        assert_eq!(reads.get(), 0);

        let mut categories = CategoryMask::none();
        categories.enable(GPU.0);
        recorder
            .arm(CaptureConfig {
                epoch: 7,
                categories,
            })
            .unwrap();
        assert_eq!(
            recorder.instant(DescriptorId(1), TraceContext::NONE, 1, 0),
            RecordStatus::CategoryDisabled
        );
        assert_eq!(reads.get(), 0);
    }

    #[test]
    fn finite_capture_preserves_records_and_counts_drops() {
        let mut recorder = Recorder::new(&DESCRIPTORS, 2, TestClock::new()).unwrap();
        recorder.arm(CaptureConfig::all(19)).unwrap();
        assert_eq!(
            recorder.instant(DescriptorId(1), TraceContext(5), 11, 12),
            RecordStatus::Recorded
        );
        assert_eq!(
            recorder.instant(DescriptorId(1), TraceContext(6), 13, 14),
            RecordStatus::Recorded
        );
        assert_eq!(
            recorder.instant(DescriptorId(1), TraceContext(7), 15, 16),
            RecordStatus::BufferFull
        );
        recorder.stop().unwrap();
        let snapshot = recorder.snapshot().unwrap();
        assert_eq!(snapshot.epoch, 19);
        assert_eq!(snapshot.records.len(), 2);
        assert_eq!(snapshot.records[0].correlation, 5);
        assert_eq!(snapshot.records[1].correlation, 6);
        assert_eq!(snapshot.dropped_records, 1);
    }

    #[test]
    fn span_guard_supports_nested_events_and_closes_on_drop() {
        let mut recorder = Recorder::new(&DESCRIPTORS, 8, TestClock::new()).unwrap();
        recorder.arm(CaptureConfig::all(1)).unwrap();
        {
            let _span = recorder.span(DescriptorId(0), TraceContext(8), 4, 16);
            assert_eq!(
                recorder.instant(DescriptorId(1), TraceContext(8), 16, 0),
                RecordStatus::Recorded
            );
        }
        recorder.stop().unwrap();
        let snapshot = recorder.snapshot().unwrap();
        assert_eq!(snapshot.records.len(), 3);
        assert_eq!(snapshot.records[0].phase, TracePhase::SpanBegin as u8);
        assert_eq!(snapshot.records[1].phase, TracePhase::Instant as u8);
        assert_eq!(snapshot.records[2].phase, TracePhase::SpanEnd as u8);
    }

    #[test]
    fn rejects_wrong_descriptor_kinds_without_clock_reads() {
        let clock = TestClock::new();
        let reads = clock.reads.clone();
        let mut recorder = Recorder::new(&DESCRIPTORS, 2, clock).unwrap();
        recorder.arm(CaptureConfig::all(1)).unwrap();
        assert_eq!(
            recorder.instant(DescriptorId(0), TraceContext::NONE, 0, 0),
            RecordStatus::WrongDescriptorKind
        );
        assert_eq!(reads.get(), 0);
    }

    #[test]
    fn enforces_capture_state_machine() {
        let mut recorder = Recorder::new(&DESCRIPTORS, 1, TestClock::new()).unwrap();
        assert!(recorder.stop().is_err());
        recorder.arm(CaptureConfig::all(2)).unwrap();
        assert!(recorder.arm(CaptureConfig::all(3)).is_err());
        assert!(recorder.snapshot().is_err());
        recorder.stop().unwrap();
        assert!(recorder.arm(CaptureConfig::all(4)).is_err());
        recorder.reset().unwrap();
        assert_eq!(recorder.state(), CaptureState::Idle);
    }

    static METRICS: [MetricDescriptor; 4] = [
        MetricDescriptor::new(IO, "io", "bytes", MetricKind::Counter, Unit::Bytes),
        MetricDescriptor::new(IO, "io", "pending", MetricKind::Gauge, Unit::Count),
        MetricDescriptor::new(IO, "io", "max_ns", MetricKind::Maximum, Unit::Nanoseconds),
        MetricDescriptor::new(
            IO,
            "io",
            "latency",
            MetricKind::HistogramLog2,
            Unit::Nanoseconds,
        ),
    ];

    #[test]
    fn metric_bank_updates_all_fixed_cell_kinds() {
        let bank = MetricBank::new(&METRICS);
        assert_eq!(bank.counter_add(MetricId(0), 7), MetricStatus::Updated);
        assert_eq!(bank.counter_add(MetricId(0), 5), MetricStatus::Updated);
        assert_eq!(bank.gauge_set(MetricId(1), -2), MetricStatus::Updated);
        assert_eq!(bank.maximum(MetricId(2), 20), MetricStatus::Updated);
        assert_eq!(bank.maximum(MetricId(2), 10), MetricStatus::Updated);
        assert_eq!(bank.histogram_log2(MetricId(3), 0), MetricStatus::Updated);
        assert_eq!(bank.histogram_log2(MetricId(3), 8), MetricStatus::Updated);
        assert_eq!(
            bank.counter_add(MetricId(1), 1),
            MetricStatus::WrongMetricKind
        );

        let mut samples = vec![MetricSample::default(); bank.required_sample_capacity()];
        let count = bank.snapshot_into(&mut samples).unwrap();
        assert_eq!(count, 35);
        assert_eq!(samples[0].value, 12);
        assert_eq!(samples[1].value as i64, -2);
        assert_eq!(samples[2].value, 20);
        assert_eq!(samples[3].value, 1); // histogram zero bucket
        assert_eq!(samples[6].value, 1); // log2(8) bucket 3
    }

    #[test]
    fn batch_codec_round_trips_and_rejects_corruption() {
        let records = [TraceRecord {
            timestamp: 10,
            correlation: 20,
            argument0: 30,
            argument1: 40,
            descriptor: 1,
            phase: TracePhase::Instant as u8,
            flags: 2,
            reserved: 0,
        }];
        let header = BatchHeader {
            source_id: 4,
            epoch: 5,
            clock_domain: 6,
            flags: 7,
            record_count: 1,
            dropped_records: 8,
            ticks_per_second: 1_000_000_000,
        };
        let mut bytes = vec![0; encoded_batch_len(1).unwrap()];
        assert_eq!(
            encode_batch_into(header, &records, &mut bytes).unwrap(),
            bytes.len()
        );
        let mut decoded = [TraceRecord::default(); 1];
        let (decoded_header, count) = decode_batch_into(&bytes, &mut decoded).unwrap();
        assert_eq!(decoded_header, header);
        assert_eq!(count, 1);
        assert_eq!(decoded, records);
        bytes[0] = b'X';
        assert_eq!(
            decode_batch_into(&bytes, &mut decoded),
            Err(BatchError::BadMagic)
        );
    }

    #[test]
    fn collector_rejects_a_malformed_batch_without_partial_ingest() {
        let mut collector = Collector::new(3);
        collector
            .register_source(SourceRegistration {
                source_id: 1,
                process_id: 1,
                name: "worker",
                clock: ClockMapping::native(1),
                descriptors: &DESCRIPTORS,
                metric_descriptors: &METRICS,
            })
            .unwrap();
        let records = [
            TraceRecord {
                timestamp: 1,
                descriptor: 1,
                phase: TracePhase::Instant as u8,
                ..TraceRecord::default()
            },
            TraceRecord {
                timestamp: 2,
                descriptor: 99,
                phase: TracePhase::Instant as u8,
                ..TraceRecord::default()
            },
        ];
        assert_eq!(
            collector.ingest(
                BatchHeader {
                    source_id: 1,
                    epoch: 3,
                    clock_domain: 1,
                    record_count: 2,
                    ticks_per_second: 1_000_000_000,
                    ..BatchHeader::default()
                },
                &records,
            ),
            Err(CollectorError::DescriptorOutOfRange {
                source: 1,
                descriptor: 99,
            })
        );
        assert_eq!(collector.record_count(), 0);
    }

    #[test]
    fn collector_merges_clock_domains_and_streams_valid_json() {
        let mut collector = Collector::new(9);
        collector
            .register_source(SourceRegistration {
                source_id: 2,
                process_id: 1,
                name: "worker\"two",
                clock: ClockMapping::native(1),
                descriptors: &DESCRIPTORS,
                metric_descriptors: &METRICS,
            })
            .unwrap();
        collector
            .register_source(SourceRegistration {
                source_id: 1,
                process_id: 1,
                name: "page",
                clock: ClockMapping {
                    clock_domain: 2,
                    origin_tick: 100,
                    origin_reference_ns: 5,
                    rate_numerator: 2,
                    rate_denominator: 1,
                    uncertainty_ns: 3,
                },
                descriptors: &DESCRIPTORS,
                metric_descriptors: &METRICS,
            })
            .unwrap();
        collector
            .ingest(
                BatchHeader {
                    source_id: 2,
                    epoch: 9,
                    clock_domain: 1,
                    record_count: 1,
                    ticks_per_second: 1_000_000_000,
                    ..BatchHeader::default()
                },
                &[TraceRecord {
                    timestamp: 20,
                    descriptor: 1,
                    phase: TracePhase::Instant as u8,
                    ..TraceRecord::default()
                }],
            )
            .unwrap();
        collector
            .ingest(
                BatchHeader {
                    source_id: 1,
                    epoch: 9,
                    clock_domain: 2,
                    record_count: 1,
                    dropped_records: 2,
                    ticks_per_second: 500_000_000,
                    ..BatchHeader::default()
                },
                &[TraceRecord {
                    timestamp: 105,
                    correlation: 44,
                    descriptor: 2,
                    phase: TracePhase::AsyncBegin as u8,
                    ..TraceRecord::default()
                }],
            )
            .unwrap();
        collector
            .ingest_metrics(
                1,
                106,
                &[MetricSample {
                    metric: 0,
                    bucket: 0,
                    value: 99,
                }],
            )
            .unwrap();
        let mut json = Vec::new();
        collector.write_chrome_trace(&mut json).unwrap();
        let json = String::from_utf8(json).unwrap();
        assert!(json.starts_with("{\"traceEvents\":["));
        assert!(json.contains("worker\\\"two"));
        assert!(json.contains("telemetry.records_dropped"));
        assert!(json.contains("\"id\":\"44\""));
        assert!(json.contains("\"ph\":\"C\""));
        assert!(json.contains("\"value\":99"));
        assert!(json.ends_with("]}"));

        let mut raw = Vec::new();
        collector.write_raw(&mut raw).unwrap();
        assert_eq!(&raw[..4], &RAW_MAGIC);
        assert_eq!(u16::from_le_bytes([raw[4], raw[5]]), RAW_VERSION);
    }
}
