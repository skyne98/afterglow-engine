use afterglow_telemetry::*;

static DESCRIPTORS: [Descriptor<'static>; 1] = [Descriptor::new(
    CategoryId(0),
    "app",
    "sample",
    DescriptorKind::Instant,
    ArgumentDescriptor::NONE,
    ArgumentDescriptor::NONE,
)];
const ID: ProducerIdentity = ProducerIdentity {
    session: [1, 2, 3, 4],
    source_id: 1,
    generation: 1,
    clock_domain: 1,
    clock_generation: 1,
};

#[test]
fn continued_windows_have_unique_sequences_and_cumulative_losses() {
    for rolling in [false, true] {
        let mut recorder = Recorder::new(&DESCRIPTORS, 2, MonotonicClock).unwrap();
        assert!(recorder.resume().is_err());
        recorder
            .arm(if rolling {
                CaptureConfig::flight(1)
            } else {
                CaptureConfig::all(1)
            })
            .unwrap();
        assert!(recorder.resume().is_err());
        let mut collector = Collector::new(
            ID.session,
            1,
            CollectorLimits {
                sources: 1,
                descriptors: 1,
                metadata_bytes: 128,
                records: 8,
                batches: 4,
                metric_samples: 1,
                raw_bytes: 2048,
            },
        )
        .unwrap();
        collector
            .register_source(SourceRegistration {
                source_id: 1,
                producer_generation: 1,
                clock_generation: 1,
                process_id: 1,
                name: "app",
                clock: ClockMapping::native(1),
                descriptors: &DESCRIPTORS,
                metric_descriptors: &[],
            })
            .unwrap();
        for window in 0..4 {
            for index in 0..3 {
                recorder.instant(DescriptorId(0), TraceContext(window * 3 + index), 0, 0);
            }
            recorder.stop().unwrap();
            let snapshot = recorder.snapshot().unwrap();
            let header = BatchHeader::from_snapshot(ID, 1_000_000_000, &snapshot).unwrap();
            let first = if rolling { window * 3 + 1 } else { window * 2 };
            assert_eq!(
                (header.first_sequence, header.next_sequence),
                (first, first + 2)
            );
            assert_eq!(
                header.overwritten_records,
                if rolling { window + 1 } else { 0 }
            );
            assert_eq!(header.dropped_records, if rolling { 0 } else { window + 1 });
            collector.ingest(header, snapshot.records).unwrap();
            assert!(collector.ingest(header, snapshot.records).is_err());
            recorder.resume().unwrap();
        }
        assert_eq!(collector.record_count(), 8);
        recorder.stop().unwrap();
        let empty = recorder.snapshot().unwrap();
        assert_eq!(empty.first_sequence, empty.next_sequence);
        assert_eq!(empty.next_sequence, if rolling { 12 } else { 8 });
        recorder.reset().unwrap();
        recorder.arm(CaptureConfig::all(2)).unwrap();
        recorder.stop().unwrap();
        let snapshot = recorder.snapshot().unwrap();
        assert_eq!(
            (
                snapshot.first_sequence,
                snapshot.next_sequence,
                snapshot.dropped_records,
                snapshot.overwritten_records
            ),
            (0, 0, 0, 0)
        );
    }
}
