//! Cold-path batch collection and streaming export.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::{self, Write};

use crate::batch::{
    BATCH_VERSION, BatchError, BatchHeader, ProducerIdentity, encode_batch_into, encoded_batch_len,
    validate_batch,
};
use crate::clock::ClockMapping;
use crate::descriptor::{Descriptor, DescriptorKind};
use crate::metrics::{HISTOGRAM_BUCKETS, MetricDescriptor, MetricKind, MetricSample};
use crate::record::{TracePhase, TraceRecord};

#[cfg(feature = "collector")]
mod file;

pub const RAW_MAGIC: [u8; 4] = *b"DGTL";
pub const RAW_VERSION: u16 = 1;

#[derive(Clone, Debug)]
struct OwnedArgumentDescriptor {
    name: String,
    kind: u8,
    unit: u8,
}

#[derive(Clone, Debug)]
struct OwnedDescriptor {
    category_name: String,
    name: String,
    kind: DescriptorKind,
    argument0: OwnedArgumentDescriptor,
    argument1: OwnedArgumentDescriptor,
    severity: u8,
}

#[derive(Clone, Debug)]
struct OwnedMetricDescriptor {
    category_name: String,
    name: String,
    kind: MetricKind,
    unit: u8,
}

#[derive(Clone, Copy, Debug)]
struct TimedMetricSample {
    timestamp: u64,
    sample: MetricSample,
}

#[derive(Clone, Debug)]
struct Source {
    source_id: u32,
    process_id: u32,
    name: String,
    clock: ClockMapping,
    descriptors: Vec<OwnedDescriptor>,
    metric_descriptors: Vec<OwnedMetricDescriptor>,
    records: Vec<TraceRecord>,
    batches: Vec<BatchHeader>,
    producer_generation: u32,
    clock_generation: u32,
    metric_samples: Vec<TimedMetricSample>,
    dropped_records: u64,
    overwritten_records: u64,
    sequence_gaps: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct SourceRegistration<'a> {
    pub source_id: u32,
    pub producer_generation: u32,
    pub clock_generation: u32,
    pub process_id: u32,
    pub name: &'a str,
    pub clock: ClockMapping,
    pub descriptors: &'a [Descriptor<'a>],
    pub metric_descriptors: &'a [MetricDescriptor<'a>],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectorError {
    InvalidLimits,
    Capacity(&'static str),
    Allocation,
    InvalidBatch(BatchError),
    IdentityMismatch,
    SequenceOverlap,
    DuplicateSource(u32),
    UnknownSource(u32),
    EpochMismatch {
        expected: u32,
        actual: u32,
    },
    ClockDomainMismatch {
        expected: u32,
        actual: u32,
    },
    RecordCountMismatch {
        header: u32,
        records: usize,
    },
    InvalidTickRate,
    DescriptorOutOfRange {
        source: u32,
        descriptor: u32,
    },
    InvalidPhase {
        source: u32,
        descriptor: u32,
        phase: u8,
    },
    TimestampRegression {
        source: u32,
        previous: u64,
        current: u64,
    },
    UnmappableTimestamp {
        source: u32,
        timestamp: u64,
    },
    MetricOutOfRange {
        source: u32,
        metric: u32,
    },
    MetricBucketOutOfRange {
        source: u32,
        metric: u32,
        bucket: u8,
    },
}

/// Explicit capacities for one collection. Metadata bytes count UTF-8 text.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(
    feature = "collector",
    derive(serde::Serialize, serde::Deserialize),
    serde(deny_unknown_fields)
)]
pub struct CollectorLimits {
    pub sources: usize,
    pub descriptors: usize,
    pub metadata_bytes: usize,
    pub records: usize,
    pub batches: usize,
    pub metric_samples: usize,
    /// Maximum encoded DGTL bytes, including metadata and the file header.
    pub raw_bytes: usize,
}

/// Owns bounded cold diagnostic data. Capacity errors keep previous data intact.
pub struct Collector {
    session: [u32; 4],
    epoch: u32,
    limits: CollectorLimits,
    sources: Vec<Source>,
    descriptors: usize,
    metadata_bytes: usize,
    records: usize,
    batches: usize,
    metric_samples: usize,
    raw_bytes: usize,
}

impl Collector {
    pub fn new(
        session: [u32; 4],
        epoch: u32,
        limits: CollectorLimits,
    ) -> Result<Self, CollectorError> {
        if session == [0; 4] {
            return Err(CollectorError::IdentityMismatch);
        }
        if limits.raw_bytes < 32 {
            return Err(CollectorError::InvalidLimits);
        }
        if [
            limits.sources,
            limits.descriptors,
            limits.metadata_bytes,
            limits.records,
            limits.batches,
            limits.metric_samples,
        ]
        .iter()
        .any(|&value| value == 0 || value > u32::MAX as usize)
        {
            return Err(CollectorError::InvalidLimits);
        }
        Ok(Self {
            session,
            epoch,
            limits,
            sources: Vec::new(),
            descriptors: 0,
            metadata_bytes: 0,
            records: 0,
            batches: 0,
            metric_samples: 0,
            raw_bytes: 32,
        })
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn register_source(
        &mut self,
        registration: SourceRegistration<'_>,
    ) -> Result<(), CollectorError> {
        if self.session == [0; 4]
            || registration.producer_generation == 0
            || registration.clock_generation == 0
        {
            return Err(CollectorError::IdentityMismatch);
        }
        if self
            .sources
            .iter()
            .any(|source| source.source_id == registration.source_id)
        {
            return Err(CollectorError::DuplicateSource(registration.source_id));
        }
        if registration.clock.rate_numerator == 0 || registration.clock.rate_denominator == 0 {
            return Err(CollectorError::InvalidTickRate);
        }
        if self.sources.len() == self.limits.sources {
            return Err(CollectorError::Capacity("sources"));
        }
        let descriptor_count = registration
            .descriptors
            .len()
            .checked_add(registration.metric_descriptors.len())
            .ok_or(CollectorError::Capacity("descriptors"))?;
        if descriptor_count > self.limits.descriptors - self.descriptors {
            return Err(CollectorError::Capacity("descriptors"));
        }
        let metadata_bytes = registration
            .descriptors
            .iter()
            .flat_map(|d| [d.name, d.category_name, d.argument0.name, d.argument1.name])
            .chain(
                registration
                    .metric_descriptors
                    .iter()
                    .flat_map(|d| [d.name, d.category_name]),
            )
            .try_fold(registration.name.len(), |total, text| {
                total.checked_add(text.len())
            })
            .ok_or(CollectorError::Capacity("metadata_bytes"))?;
        if metadata_bytes > self.limits.metadata_bytes - self.metadata_bytes {
            return Err(CollectorError::Capacity("metadata_bytes"));
        }
        let raw_bytes = registration
            .descriptors
            .len()
            .checked_mul(22)
            .and_then(|bytes| {
                registration
                    .metric_descriptors
                    .len()
                    .checked_mul(10)
                    .and_then(|metrics| bytes.checked_add(metrics))
            })
            .and_then(|bytes| bytes.checked_add(metadata_bytes))
            .and_then(|bytes| bytes.checked_add(80))
            .ok_or(CollectorError::Capacity("raw_bytes"))?;
        if raw_bytes > self.limits.raw_bytes - self.raw_bytes {
            return Err(CollectorError::Capacity("raw_bytes"));
        }
        self.sources
            .try_reserve_exact(1)
            .map_err(|_| CollectorError::Allocation)?;
        let descriptors = registration
            .descriptors
            .iter()
            .map(|descriptor| OwnedDescriptor {
                category_name: descriptor.category_name.to_owned(),
                name: descriptor.name.to_owned(),
                kind: descriptor.kind,
                argument0: OwnedArgumentDescriptor {
                    name: descriptor.argument0.name.to_owned(),
                    kind: descriptor.argument0.kind as u8,
                    unit: descriptor.argument0.unit as u8,
                },
                argument1: OwnedArgumentDescriptor {
                    name: descriptor.argument1.name.to_owned(),
                    kind: descriptor.argument1.kind as u8,
                    unit: descriptor.argument1.unit as u8,
                },
                severity: descriptor.severity as u8,
            })
            .collect();
        let metric_descriptors = registration
            .metric_descriptors
            .iter()
            .map(|descriptor| OwnedMetricDescriptor {
                category_name: descriptor.category_name.to_owned(),
                name: descriptor.name.to_owned(),
                kind: descriptor.kind,
                unit: descriptor.unit as u8,
            })
            .collect();
        self.sources.push(Source {
            source_id: registration.source_id,
            process_id: registration.process_id,
            name: registration.name.to_owned(),
            clock: registration.clock,
            descriptors,
            metric_descriptors,
            records: Vec::new(),
            batches: Vec::new(),
            producer_generation: registration.producer_generation,
            clock_generation: registration.clock_generation,
            metric_samples: Vec::new(),
            dropped_records: 0,
            overwritten_records: 0,
            sequence_gaps: 0,
        });
        self.descriptors += descriptor_count;
        self.metadata_bytes += metadata_bytes;
        self.raw_bytes += raw_bytes;
        Ok(())
    }

    pub fn ingest(
        &mut self,
        header: BatchHeader,
        records: &[TraceRecord],
    ) -> Result<(), CollectorError> {
        let Some(source) = self
            .sources
            .iter_mut()
            .find(|source| source.source_id == header.source_id)
        else {
            return Err(CollectorError::UnknownSource(header.source_id));
        };
        if header.epoch != self.epoch {
            return Err(CollectorError::EpochMismatch {
                expected: self.epoch,
                actual: header.epoch,
            });
        }
        if header.clock_domain != source.clock.clock_domain {
            return Err(CollectorError::ClockDomainMismatch {
                expected: source.clock.clock_domain,
                actual: header.clock_domain,
            });
        }
        if header.record_count as usize != records.len() {
            return Err(CollectorError::RecordCountMismatch {
                header: header.record_count,
                records: records.len(),
            });
        }
        if header.ticks_per_second == 0 {
            return Err(CollectorError::InvalidTickRate);
        }
        validate_batch(header, records).map_err(CollectorError::InvalidBatch)?;
        if header.session != self.session
            || header.producer_generation != source.producer_generation
            || header.clock_generation != source.clock_generation
        {
            return Err(CollectorError::IdentityMismatch);
        }
        if let Some(previous) = source.batches.last()
            && (header.first_sequence < previous.next_sequence
                || header.next_sequence <= previous.next_sequence
                || header.dropped_records < previous.dropped_records
                || header.overwritten_records < previous.overwritten_records)
        {
            return Err(CollectorError::SequenceOverlap);
        }
        let mut previous = source.records.last().map(|record| record.timestamp);
        for record in records {
            let Some(descriptor) = source.descriptors.get(record.descriptor as usize) else {
                return Err(CollectorError::DescriptorOutOfRange {
                    source: source.source_id,
                    descriptor: record.descriptor,
                });
            };
            let phase_valid = matches!(
                (descriptor.kind, record.phase),
                (DescriptorKind::Instant, value) if value == TracePhase::Instant as u8
            ) || matches!(
                (descriptor.kind, record.phase),
                (DescriptorKind::Span, value)
                    if value == TracePhase::SpanBegin as u8 || value == TracePhase::SpanEnd as u8
            ) || matches!(
                (descriptor.kind, record.phase),
                (DescriptorKind::AsyncSpan, value)
                    if value == TracePhase::AsyncBegin as u8 || value == TracePhase::AsyncEnd as u8
            ) || matches!(
                (descriptor.kind, record.phase),
                (DescriptorKind::Flow, value)
                    if value == TracePhase::FlowStart as u8
                        || value == TracePhase::FlowStep as u8
                        || value == TracePhase::FlowEnd as u8
            );
            if !phase_valid {
                return Err(CollectorError::InvalidPhase {
                    source: source.source_id,
                    descriptor: record.descriptor,
                    phase: record.phase,
                });
            }
            if let Some(prior) = previous
                && record.timestamp < prior
            {
                return Err(CollectorError::TimestampRegression {
                    source: source.source_id,
                    previous: prior,
                    current: record.timestamp,
                });
            }
            source.clock.map_to_reference_ns(record.timestamp).ok_or(
                CollectorError::UnmappableTimestamp {
                    source: source.source_id,
                    timestamp: record.timestamp,
                },
            )?;
            previous = Some(record.timestamp);
        }
        if records.len() > self.limits.records - self.records {
            return Err(CollectorError::Capacity("records"));
        }
        if self.batches == self.limits.batches {
            return Err(CollectorError::Capacity("batches"));
        }
        let raw_bytes = encoded_batch_len(records.len())
            .filter(|&bytes| u32::try_from(bytes).is_ok())
            .and_then(|bytes| bytes.checked_add(4))
            .ok_or(CollectorError::Capacity("raw_bytes"))?;
        if raw_bytes > self.limits.raw_bytes - self.raw_bytes {
            return Err(CollectorError::Capacity("raw_bytes"));
        }
        source
            .records
            .try_reserve_exact(records.len())
            .map_err(|_| CollectorError::Allocation)?;
        source
            .batches
            .try_reserve_exact(1)
            .map_err(|_| CollectorError::Allocation)?;
        if let Some(previous) = source.batches.last() {
            source.sequence_gaps = source
                .sequence_gaps
                .saturating_add(header.first_sequence - previous.next_sequence);
        } else {
            source.sequence_gaps = header.first_sequence - header.overwritten_records;
        }
        source.records.extend_from_slice(records);
        source.batches.push(header);
        source.dropped_records = header.dropped_records;
        source.overwritten_records = header.overwritten_records;
        self.records += records.len();
        self.batches += 1;
        self.raw_bytes += raw_bytes;
        Ok(())
    }

    /// Attach one fixed metric snapshot to a source timeline. Snapshotting and
    /// ingestion are cold diagnostic work; metric updates remain always-on and
    /// allocation-free in [`crate::MetricBank`].
    pub fn ingest_metrics(
        &mut self,
        identity: ProducerIdentity,
        timestamp: u64,
        samples: &[MetricSample],
    ) -> Result<(), CollectorError> {
        let source_id = identity.source_id;
        let Some(source) = self
            .sources
            .iter_mut()
            .find(|source| source.source_id == source_id)
        else {
            return Err(CollectorError::UnknownSource(source_id));
        };
        if identity.session != self.session
            || identity.generation != source.producer_generation
            || identity.clock_generation != source.clock_generation
            || identity.clock_domain != source.clock.clock_domain
        {
            return Err(CollectorError::IdentityMismatch);
        }
        if let Some(previous) = source.metric_samples.last()
            && timestamp < previous.timestamp
        {
            return Err(CollectorError::TimestampRegression {
                source: source_id,
                previous: previous.timestamp,
                current: timestamp,
            });
        }
        source
            .clock
            .map_to_reference_ns(timestamp)
            .ok_or(CollectorError::UnmappableTimestamp {
                source: source_id,
                timestamp,
            })?;
        for sample in samples {
            let Some(descriptor) = source.metric_descriptors.get(sample.metric as usize) else {
                return Err(CollectorError::MetricOutOfRange {
                    source: source_id,
                    metric: sample.metric,
                });
            };
            let bucket_count = if descriptor.kind == MetricKind::HistogramLog2 {
                HISTOGRAM_BUCKETS
            } else {
                1
            };
            if sample.bucket as usize >= bucket_count {
                return Err(CollectorError::MetricBucketOutOfRange {
                    source: source_id,
                    metric: sample.metric,
                    bucket: sample.bucket,
                });
            }
        }
        if samples.len() > self.limits.metric_samples - self.metric_samples {
            return Err(CollectorError::Capacity("metric_samples"));
        }
        let raw_bytes = samples
            .len()
            .checked_mul(24)
            .ok_or(CollectorError::Capacity("raw_bytes"))?;
        if raw_bytes > self.limits.raw_bytes - self.raw_bytes {
            return Err(CollectorError::Capacity("raw_bytes"));
        }
        source
            .metric_samples
            .try_reserve_exact(samples.len())
            .map_err(|_| CollectorError::Allocation)?;
        source.metric_samples.extend(
            samples
                .iter()
                .copied()
                .map(|sample| TimedMetricSample { timestamp, sample }),
        );
        self.metric_samples += samples.len();
        self.raw_bytes += raw_bytes;
        Ok(())
    }

    /// Exact encoded DGTL size. Does not serialize or scan retained records.
    pub fn raw_bytes(&self) -> usize {
        self.raw_bytes
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    pub fn record_count(&self) -> usize {
        self.records
    }

    /// Stream Catapult/Chrome Trace JSON. Perfetto opens this format directly.
    pub fn write_chrome_trace(&self, mut output: impl Write) -> io::Result<()> {
        output.write_all(b"{\"traceEvents\":[")?;
        let mut first = true;
        for source in &self.sources {
            write_separator(&mut output, &mut first)?;
            write!(
                output,
                "{{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":{},\"tid\":{},\"args\":{{\"name\":",
                source.process_id, source.source_id
            )?;
            write_json_string(&mut output, &source.name)?;
            output.write_all(b"}}")?;
            if source.clock.uncertainty_ns != 0 {
                write_separator(&mut output, &mut first)?;
                write!(
                    output,
                    "{{\"name\":\"telemetry.clock_uncertainty\",\"cat\":\"telemetry\",\"ph\":\"i\",\"s\":\"t\",\"ts\":0,\"pid\":{},\"tid\":{},\"args\":{{\"nanoseconds\":{}}}}}",
                    source.process_id, source.source_id, source.clock.uncertainty_ns
                )?;
            }
            for (name, count) in [
                ("records_overwritten", source.overwritten_records),
                ("sequence_gaps", source.sequence_gaps),
            ] {
                if count != 0 {
                    write_separator(&mut output, &mut first)?;
                    write!(
                        output,
                        "{{\"name\":\"telemetry.{name}\",\"cat\":\"telemetry\",\"ph\":\"i\",\"s\":\"t\",\"ts\":0,\"pid\":{},\"tid\":{},\"args\":{{\"count\":{count}}}}}",
                        source.process_id, source.source_id
                    )?;
                }
            }
            if source.dropped_records != 0 {
                write_separator(&mut output, &mut first)?;
                write!(
                    output,
                    "{{\"name\":\"telemetry.records_dropped\",\"cat\":\"telemetry\",\"ph\":\"i\",\"s\":\"t\",\"ts\":0,\"pid\":{},\"tid\":{},\"args\":{{\"count\":{}}}}}",
                    source.process_id, source.source_id, source.dropped_records
                )?;
            }
        }

        let mut heap = BinaryHeap::new();
        for (source_index, source) in self.sources.iter().enumerate() {
            if let Some(record) = source.records.first() {
                let timestamp = source
                    .clock
                    .map_to_reference_ns(record.timestamp)
                    .expect("timestamps validated during ingest");
                heap.push(Reverse((timestamp, source_index, 0_usize)));
            }
        }
        while let Some(Reverse((timestamp_ns, source_index, record_index))) = heap.pop() {
            let source = &self.sources[source_index];
            let record = &source.records[record_index];
            let descriptor = &source.descriptors[record.descriptor as usize];
            write_separator(&mut output, &mut first)?;
            write_trace_event(&mut output, source, descriptor, record, timestamp_ns)?;
            let next_index = record_index + 1;
            if let Some(next) = source.records.get(next_index) {
                let next_timestamp = source
                    .clock
                    .map_to_reference_ns(next.timestamp)
                    .expect("timestamps validated during ingest");
                heap.push(Reverse((next_timestamp, source_index, next_index)));
            }
        }
        for source in &self.sources {
            for timed in &source.metric_samples {
                let timestamp_ns = source
                    .clock
                    .map_to_reference_ns(timed.timestamp)
                    .expect("metric timestamps validated during ingest");
                let descriptor = &source.metric_descriptors[timed.sample.metric as usize];
                write_separator(&mut output, &mut first)?;
                write_metric_event(&mut output, source, descriptor, timed.sample, timestamp_ns)?;
            }
        }
        output.write_all(b"]}")
    }

    /// Stream DGTL with identities and original batch sequence/loss metadata.
    pub fn write_raw(&self, mut output: impl Write) -> io::Result<()> {
        output.write_all(&RAW_MAGIC)?;
        output.write_all(&RAW_VERSION.to_le_bytes())?;
        output.write_all(&BATCH_VERSION.to_le_bytes())?;
        output.write_all(&self.epoch.to_le_bytes())?;
        for word in self.session {
            output.write_all(&word.to_le_bytes())?;
        }
        write_u32(&mut output, self.sources.len())?;
        for source in &self.sources {
            output.write_all(&source.source_id.to_le_bytes())?;
            output.write_all(&source.process_id.to_le_bytes())?;
            output.write_all(&source.producer_generation.to_le_bytes())?;
            output.write_all(&source.clock_generation.to_le_bytes())?;
            write_string(&mut output, &source.name)?;
            output.write_all(&source.clock.clock_domain.to_le_bytes())?;
            output.write_all(&source.clock.origin_tick.to_le_bytes())?;
            output.write_all(&source.clock.origin_reference_ns.to_le_bytes())?;
            output.write_all(&source.clock.rate_numerator.to_le_bytes())?;
            output.write_all(&source.clock.rate_denominator.to_le_bytes())?;
            output.write_all(&source.clock.uncertainty_ns.to_le_bytes())?;
            write_u32(&mut output, source.descriptors.len())?;
            for descriptor in &source.descriptors {
                write_string(&mut output, &descriptor.category_name)?;
                write_string(&mut output, &descriptor.name)?;
                output.write_all(&[descriptor.kind as u8, descriptor.severity])?;
                write_owned_argument(&mut output, &descriptor.argument0)?;
                write_owned_argument(&mut output, &descriptor.argument1)?;
            }
            write_u32(&mut output, source.metric_descriptors.len())?;
            for descriptor in &source.metric_descriptors {
                write_string(&mut output, &descriptor.category_name)?;
                write_string(&mut output, &descriptor.name)?;
                output.write_all(&[descriptor.kind as u8, descriptor.unit])?;
            }
            write_u32(&mut output, source.batches.len())?;
            let mut offset = 0;
            for header in &source.batches {
                let end = offset + header.record_count as usize;
                let mut bytes = vec![
                    0;
                    encoded_batch_len(header.record_count as usize)
                        .expect("validated batch length")
                ];
                encode_batch_into(*header, &source.records[offset..end], &mut bytes).map_err(
                    |error| io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}")),
                )?;
                write_u32(&mut output, bytes.len())?;
                output.write_all(&bytes)?;
                offset = end;
            }
            write_u32(&mut output, source.metric_samples.len())?;
            for timed in &source.metric_samples {
                output.write_all(&timed.timestamp.to_le_bytes())?;
                output.write_all(&timed.sample.metric.to_le_bytes())?;
                output.write_all(&[timed.sample.bucket, 0, 0, 0])?;
                output.write_all(&timed.sample.value.to_le_bytes())?;
            }
        }
        Ok(())
    }
}

fn write_metric_event(
    output: &mut impl Write,
    source: &Source,
    descriptor: &OwnedMetricDescriptor,
    sample: MetricSample,
    timestamp_ns: u64,
) -> io::Result<()> {
    output.write_all(b"{\"name\":")?;
    write_json_string(output, &descriptor.name)?;
    output.write_all(b",\"cat\":")?;
    write_json_string(output, &descriptor.category_name)?;
    write!(
        output,
        ",\"ph\":\"C\",\"ts\":{:.3},\"pid\":{},\"tid\":{},\"args\":{{",
        timestamp_ns as f64 / 1_000.0,
        source.process_id,
        source.source_id
    )?;
    if descriptor.kind == MetricKind::HistogramLog2 {
        write!(output, "\"bucket_{}\":{}", sample.bucket, sample.value)?;
    } else {
        write!(output, "\"value\":{}", sample.value)?;
    }
    output.write_all(b"}}")
}

fn write_trace_event(
    output: &mut impl Write,
    source: &Source,
    descriptor: &OwnedDescriptor,
    record: &TraceRecord,
    timestamp_ns: u64,
) -> io::Result<()> {
    output.write_all(b"{\"name\":")?;
    write_json_string(output, &descriptor.name)?;
    output.write_all(b",\"cat\":")?;
    write_json_string(output, &descriptor.category_name)?;
    let phase = match record.phase {
        value if value == TracePhase::Instant as u8 => "i",
        value if value == TracePhase::SpanBegin as u8 => "B",
        value if value == TracePhase::SpanEnd as u8 => "E",
        value if value == TracePhase::AsyncBegin as u8 => "b",
        value if value == TracePhase::AsyncEnd as u8 => "e",
        value if value == TracePhase::FlowStart as u8 => "s",
        value if value == TracePhase::FlowStep as u8 => "t",
        value if value == TracePhase::FlowEnd as u8 => "f",
        _ => "i",
    };
    write!(
        output,
        ",\"ph\":\"{}\",\"ts\":{:.3},\"pid\":{},\"tid\":{}",
        phase,
        timestamp_ns as f64 / 1_000.0,
        source.process_id,
        source.source_id
    )?;
    if phase == "i" {
        output.write_all(b",\"s\":\"t\"")?;
    }
    if matches!(phase, "b" | "e" | "s" | "t" | "f") {
        write!(output, ",\"id\":\"{}\"", record.correlation)?;
    }
    output.write_all(b",\"args\":{")?;
    let mut first = true;
    if !descriptor.argument0.name.is_empty() {
        write_json_argument(
            output,
            &mut first,
            &descriptor.argument0.name,
            record.argument0,
        )?;
    }
    if !descriptor.argument1.name.is_empty() {
        write_json_argument(
            output,
            &mut first,
            &descriptor.argument1.name,
            record.argument1,
        )?;
    }
    if record.correlation != 0 && !matches!(phase, "b" | "e" | "s" | "t" | "f") {
        write_json_argument(output, &mut first, "correlation", record.correlation)?;
    }
    output.write_all(b"}}")
}

fn write_json_argument(
    output: &mut impl Write,
    first: &mut bool,
    name: &str,
    value: u64,
) -> io::Result<()> {
    if !*first {
        output.write_all(b",")?;
    }
    *first = false;
    write_json_string(output, name)?;
    write!(output, ":{value}")
}

fn write_separator(output: &mut impl Write, first: &mut bool) -> io::Result<()> {
    if !*first {
        output.write_all(b",")?;
    }
    *first = false;
    Ok(())
}

fn write_json_string(output: &mut impl Write, value: &str) -> io::Result<()> {
    output.write_all(b"\"")?;
    for character in value.chars() {
        match character {
            '\"' => output.write_all(b"\\\"")?,
            '\\' => output.write_all(b"\\\\")?,
            '\n' => output.write_all(b"\\n")?,
            '\r' => output.write_all(b"\\r")?,
            '\t' => output.write_all(b"\\t")?,
            character if character < ' ' => write!(output, "\\u{:04x}", character as u32)?,
            character => write!(output, "{character}")?,
        }
    }
    output.write_all(b"\"")
}

fn write_owned_argument(
    output: &mut impl Write,
    argument: &OwnedArgumentDescriptor,
) -> io::Result<()> {
    write_string(output, &argument.name)?;
    output.write_all(&[argument.kind, argument.unit])
}

fn write_string(output: &mut impl Write, value: &str) -> io::Result<()> {
    write_u32(output, value.len())?;
    output.write_all(value.as_bytes())
}

fn write_u32(output: &mut impl Write, value: usize) -> io::Result<()> {
    let value = u32::try_from(value)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "telemetry value exceeds u32"))?;
    output.write_all(&value.to_le_bytes())
}
