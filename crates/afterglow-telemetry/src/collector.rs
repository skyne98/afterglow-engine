//! Cold-path batch collection and streaming export.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::{self, Write};

use crate::batch::{BATCH_VERSION, BatchHeader};
use crate::clock::ClockMapping;
use crate::descriptor::{Descriptor, DescriptorKind};
use crate::metrics::{HISTOGRAM_BUCKETS, MetricDescriptor, MetricKind, MetricSample};
use crate::record::{TracePhase, TraceRecord};

pub const RAW_MAGIC: [u8; 4] = *b"AGTL";
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
    metric_samples: Vec<TimedMetricSample>,
    dropped_records: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct SourceRegistration<'a> {
    pub source_id: u32,
    pub process_id: u32,
    pub name: &'a str,
    pub clock: ClockMapping,
    pub descriptors: &'a [Descriptor],
    pub metric_descriptors: &'a [MetricDescriptor],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectorError {
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

/// Owns cold diagnostic data after producers freeze their finite captures.
pub struct Collector {
    epoch: u32,
    sources: Vec<Source>,
}

impl Collector {
    pub fn new(epoch: u32) -> Self {
        Self {
            epoch,
            sources: Vec::new(),
        }
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn register_source(
        &mut self,
        registration: SourceRegistration<'_>,
    ) -> Result<(), CollectorError> {
        if self
            .sources
            .iter()
            .any(|source| source.source_id == registration.source_id)
        {
            return Err(CollectorError::DuplicateSource(registration.source_id));
        }
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
            metric_samples: Vec::new(),
            dropped_records: 0,
        });
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
        source.records.extend_from_slice(records);
        source.dropped_records = source
            .dropped_records
            .saturating_add(header.dropped_records as u64);
        Ok(())
    }

    /// Attach one fixed metric snapshot to a source timeline. Snapshotting and
    /// ingestion are cold diagnostic work; metric updates remain always-on and
    /// allocation-free in [`crate::MetricBank`].
    pub fn ingest_metrics(
        &mut self,
        source_id: u32,
        timestamp: u64,
        samples: &[MetricSample],
    ) -> Result<(), CollectorError> {
        let Some(source) = self
            .sources
            .iter_mut()
            .find(|source| source.source_id == source_id)
        else {
            return Err(CollectorError::UnknownSource(source_id));
        };
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
        source.metric_samples.extend(
            samples
                .iter()
                .copied()
                .map(|sample| TimedMetricSample { timestamp, sample }),
        );
        Ok(())
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    pub fn record_count(&self) -> usize {
        self.sources.iter().map(|source| source.records.len()).sum()
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

    /// Stream the lossless versioned `.agt` diagnostic format.
    pub fn write_raw(&self, mut output: impl Write) -> io::Result<()> {
        output.write_all(&RAW_MAGIC)?;
        output.write_all(&RAW_VERSION.to_le_bytes())?;
        output.write_all(&BATCH_VERSION.to_le_bytes())?;
        output.write_all(&self.epoch.to_le_bytes())?;
        write_u32(&mut output, self.sources.len())?;
        for source in &self.sources {
            output.write_all(&source.source_id.to_le_bytes())?;
            output.write_all(&source.process_id.to_le_bytes())?;
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
            write_u32(&mut output, source.records.len())?;
            output.write_all(&source.dropped_records.to_le_bytes())?;
            for record in &source.records {
                output.write_all(&record.timestamp.to_le_bytes())?;
                output.write_all(&record.correlation.to_le_bytes())?;
                output.write_all(&record.argument0.to_le_bytes())?;
                output.write_all(&record.argument1.to_le_bytes())?;
                output.write_all(&record.descriptor.to_le_bytes())?;
                output.write_all(&[record.phase, record.flags])?;
                output.write_all(&record.reserved.to_le_bytes())?;
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
