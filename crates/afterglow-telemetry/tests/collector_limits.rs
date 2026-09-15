use afterglow_telemetry::*;

const ID: ProducerIdentity = ProducerIdentity {
    session: [1, 2, 3, 4],
    source_id: 1,
    generation: 2,
    clock_domain: 3,
    clock_generation: 4,
};
const LIMITS: CollectorLimits = CollectorLimits {
    sources: 2,
    descriptors: 4,
    metadata_bytes: 512,
    records: 3,
    batches: 3,
    metric_samples: 3,
    raw_bytes: 4096,
};
const DESCRIPTORS: [Descriptor; 1] = [Descriptor::new(
    CategoryId(0),
    "app",
    "event",
    DescriptorKind::Instant,
    ArgumentDescriptor::NONE,
    ArgumentDescriptor::NONE,
)];
const METRICS: [MetricDescriptor; 1] = [MetricDescriptor::new(
    CategoryId(0),
    "app",
    "count",
    MetricKind::Counter,
    Unit::Count,
)];

fn source(id: u32) -> SourceRegistration<'static> {
    SourceRegistration {
        source_id: id,
        producer_generation: ID.generation,
        clock_generation: ID.clock_generation,
        process_id: 1,
        name: "test",
        clock: ClockMapping::native(ID.clock_domain),
        descriptors: &DESCRIPTORS,
        metric_descriptors: &METRICS,
    }
}
fn collector(limits: CollectorLimits) -> Collector {
    let mut collector = Collector::new(ID.session, 1, limits).unwrap();
    collector.register_source(source(1)).unwrap();
    collector
}
fn header(first: u64, count: u32) -> BatchHeader {
    BatchHeader {
        source_id: 1,
        epoch: 1,
        clock_domain: 3,
        session: ID.session,
        producer_generation: 2,
        clock_generation: 4,
        first_sequence: first,
        next_sequence: first + u64::from(count),
        record_count: count,
        ticks_per_second: 1_000_000_000,
        ..BatchHeader::default()
    }
}
fn record(timestamp: u64) -> TraceRecord {
    TraceRecord {
        timestamp,
        phase: TracePhase::Instant as u8,
        ..TraceRecord::default()
    }
}
fn raw(collector: &Collector) -> Vec<u8> {
    let mut output = Vec::new();
    collector.write_raw(&mut output).unwrap();
    assert_eq!(collector.raw_bytes(), output.len());
    output
}

#[test]
fn registration_and_collection_have_explicit_capacities() {
    assert!(Collector::new([0; 4], 1, LIMITS).is_err());
    for limits in [
        CollectorLimits {
            sources: 0,
            ..LIMITS
        },
        CollectorLimits {
            batches: 0,
            ..LIMITS
        },
    ] {
        assert!(matches!(
            Collector::new(ID.session, 1, limits),
            Err(CollectorError::InvalidLimits)
        ));
    }
    for (limits, expected) in [
        (
            CollectorLimits {
                sources: 1,
                ..LIMITS
            },
            "sources",
        ),
        (
            CollectorLimits {
                descriptors: 2,
                ..LIMITS
            },
            "descriptors",
        ),
        (
            CollectorLimits {
                metadata_bytes: 21,
                ..LIMITS
            },
            "metadata_bytes",
        ),
    ] {
        let mut collector = collector(limits);
        let before = raw(&collector);
        assert_eq!(
            collector.register_source(source(2)),
            Err(CollectorError::Capacity(expected))
        );
        assert_eq!(raw(&collector), before);
    }
    let mut collector = collector(LIMITS);
    collector
        .ingest(header(0, 2), &[record(1), record(2)])
        .unwrap();
    let before = raw(&collector);
    assert_eq!(
        collector.ingest(header(2, 2), &[record(3), record(4)]),
        Err(CollectorError::Capacity("records"))
    );
    assert_eq!(raw(&collector), before);
    collector.ingest(header(2, 1), &[record(3)]).unwrap();
    assert_eq!(collector.record_count(), 3);
}

#[test]
fn batch_and_metric_limits_keep_previous_output_intact() {
    let mut collector = collector(CollectorLimits {
        batches: 1,
        metric_samples: 1,
        ..LIMITS
    });
    collector.ingest(header(0, 1), &[record(1)]).unwrap();
    let before = raw(&collector);
    assert_eq!(
        collector.ingest(header(1, 1), &[record(2)]),
        Err(CollectorError::Capacity("batches"))
    );
    assert_eq!(raw(&collector), before);
    let sample = MetricSample {
        metric: 0,
        bucket: 0,
        value: 4,
    };
    collector.ingest_metrics(ID, 2, &[sample]).unwrap();
    let before = raw(&collector);
    assert_eq!(
        collector.ingest_metrics(ID, 3, &[sample]),
        Err(CollectorError::Capacity("metric_samples"))
    );
    assert_eq!(raw(&collector), before);
}

#[test]
fn stale_identity_and_replayed_batches_cannot_change_a_capture() {
    let mut collector = collector(LIMITS);
    collector.ingest(header(0, 1), &[record(1)]).unwrap();
    let before = raw(&collector);
    for change in [
        BatchHeader {
            session: [5; 4],
            ..header(1, 1)
        },
        BatchHeader {
            producer_generation: 3,
            ..header(1, 1)
        },
        BatchHeader {
            clock_generation: 5,
            ..header(1, 1)
        },
    ] {
        assert_eq!(
            collector.ingest(change, &[record(2)]),
            Err(CollectorError::IdentityMismatch)
        );
        assert_eq!(raw(&collector), before);
    }
    assert_eq!(
        collector.ingest(header(0, 1), &[record(1)]),
        Err(CollectorError::SequenceOverlap)
    );
    assert_eq!(raw(&collector), before);
    assert_eq!(
        collector.ingest_metrics(
            ProducerIdentity {
                generation: 3,
                ..ID
            },
            2,
            &[]
        ),
        Err(CollectorError::IdentityMismatch)
    );
    assert_eq!(raw(&collector), before);
}

#[test]
fn received_metadata_does_not_need_static_strings() {
    let mut collector = Collector::new(ID.session, 1, LIMITS).unwrap();
    {
        let name = String::from("received event");
        let descriptors = [Descriptor::new(
            CategoryId(0),
            "app",
            &name,
            DescriptorKind::Instant,
            ArgumentDescriptor::NONE,
            ArgumentDescriptor::NONE,
        )];
        collector
            .register_source(SourceRegistration {
                descriptors: &descriptors,
                ..source(1)
            })
            .unwrap();
    }
    collector.ingest(header(0, 1), &[record(1)]).unwrap();
    let mut json = Vec::new();
    collector.write_chrome_trace(&mut json).unwrap();
    assert!(String::from_utf8(json).unwrap().contains("received event"));
}

#[test]
fn encoded_byte_limit_rejects_work_before_retained_data_changes() {
    let initial = raw(&collector(LIMITS)).len();
    let mut bounded = collector(CollectorLimits {
        raw_bytes: initial + 140,
        ..LIMITS
    });
    bounded.ingest(header(0, 1), &[record(1)]).unwrap();
    let before = raw(&bounded);
    assert_eq!(before.len(), initial + 140);
    assert_eq!(
        bounded.ingest(header(1, 1), &[record(2)]),
        Err(CollectorError::Capacity("raw_bytes"))
    );
    assert_eq!(
        bounded.ingest_metrics(
            ID,
            2,
            &[MetricSample {
                metric: 0,
                bucket: 0,
                value: 1
            }]
        ),
        Err(CollectorError::Capacity("raw_bytes"))
    );
    assert_eq!(
        bounded.register_source(source(2)),
        Err(CollectorError::Capacity("raw_bytes"))
    );
    assert_eq!(raw(&bounded), before);
    let mut empty = Collector::new(
        ID.session,
        1,
        CollectorLimits {
            raw_bytes: initial - 1,
            ..LIMITS
        },
    )
    .unwrap();
    assert_eq!(
        empty.register_source(source(1)),
        Err(CollectorError::Capacity("raw_bytes"))
    );
    assert_eq!(raw(&empty).len(), 32);
    assert!(matches!(
        Collector::new(
            ID.session,
            1,
            CollectorLimits {
                raw_bytes: 31,
                ..LIMITS
            }
        ),
        Err(CollectorError::InvalidLimits)
    ));
}

#[test]
fn raw_export_retains_each_batch_and_its_loss_metadata() {
    let mut collector = collector(LIMITS);
    let first = BatchHeader {
        flags: BATCH_ROLLING,
        overwritten_records: 2,
        ..header(2, 1)
    };
    let second = BatchHeader {
        flags: BATCH_ROLLING,
        overwritten_records: 3,
        dropped_records: 7,
        ..header(5, 1)
    };
    collector.ingest(first, &[record(1)]).unwrap();
    collector.ingest(second, &[record(2)]).unwrap();
    let bytes = raw(&collector);
    assert_eq!(&bytes[..4], b"DGTL");
    for (header, event) in [(first, record(1)), (second, record(2))] {
        let mut batch = vec![0; encoded_batch_len(1).unwrap()];
        encode_batch_into(header, &[event], &mut batch).unwrap();
        assert!(bytes.windows(batch.len()).any(|window| window == batch));
    }
    let mut json = Vec::new();
    collector.write_chrome_trace(&mut json).unwrap();
    let json = String::from_utf8(json).unwrap();
    assert!(json.contains("records_overwritten"));
    assert!(json.contains("sequence_gaps"));
    assert!(json.contains("records_dropped"));
}
