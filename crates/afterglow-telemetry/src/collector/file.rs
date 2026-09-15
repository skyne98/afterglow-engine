//! Bounded cold file input and agent-facing JSON output.
use super::*;
use crate::batch::{decode_batch_header, decode_batch_into};
use crate::descriptor::{ArgumentDescriptor, ArgumentType, CategoryId, Severity, Unit};
use serde_json::json;
use std::collections::BTreeMap;

const ARGUMENT_TYPES: [ArgumentType; 8] = [ArgumentType::None, ArgumentType::Unsigned,
    ArgumentType::Signed, ArgumentType::FloatBits, ArgumentType::Boolean,
    ArgumentType::Identifier, ArgumentType::Bytes, ArgumentType::Duration];
const UNITS: [Unit; 9] = [Unit::None, Unit::Count, Unit::Bytes, Unit::Nanoseconds,
    Unit::Microseconds, Unit::Milliseconds, Unit::Frames, Unit::Samples, Unit::Percent];

fn arguments_json(descriptor: &OwnedDescriptor, record: &TraceRecord) -> serde_json::Value {
    let arguments: Vec<_> = [(&descriptor.argument0, record.argument0), (&descriptor.argument1, record.argument1)]
        .into_iter().enumerate().filter(|(_, (argument, _))| argument.kind != ArgumentType::None as u8)
        .map(|(slot, (argument, raw))| {
            let kind = ARGUMENT_TYPES[argument.kind as usize];
            let (value, error) = match kind {
                ArgumentType::Signed => (json!((raw as i64).to_string()), None),
                ArgumentType::FloatBits => {
                    let value = f64::from_bits(raw);
                    if value.is_finite() { (json!(value), None) }
                    else { (serde_json::Value::Null, Some("non-finite-float")) }
                }
                ArgumentType::Boolean => match raw {
                    0 | 1 => (json!(raw == 1), None),
                    _ => (serde_json::Value::Null, Some("invalid-boolean")),
                },
                _ => (json!(raw.to_string()), None),
            };
            json!({"slot": slot, "name": argument.name, "type": kind,
                "unit": UNITS[argument.unit as usize], "raw": raw.to_string(), "value": value, "error": error})
        }).collect();
    serde_json::Value::Array(arguments)
}

fn invalid(value: impl std::fmt::Debug) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{value:?}"))
}
fn validate_page(limit: usize) -> io::Result<()> {
    if !(1..=10_000).contains(&limit) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "Record limit must be 1..10000")); }
    Ok(())
}
fn write_page_end(output: &mut impl Write, matched: usize, returned: usize, offset: usize) -> io::Result<()> {
    let next = if offset < matched && returned < matched - offset { Some(offset + returned) } else { None };
    write!(output, "],\"matched\":{matched},\"returned\":{returned},\"offset\":{offset},\"next_offset\":{}", json!(next))
}
#[derive(Default)]
struct SpanStatistics {
    durations: Vec<u64>,
    missing_begins: usize,
    missing_ends: usize,
    ambiguous_pairs: usize,
    excluded_records: usize,
}
#[derive(Default)]
struct OpenSpans {
    begins: Vec<(usize, u64)>,
    ambiguous: bool,
}
struct ObservedSpan<'a> {
    source: &'a Source,
    begin: &'a TraceRecord,
    end: &'a TraceRecord,
    begin_sequence: u64,
    end_sequence: u64,
    start: u64,
    end_ns: u64,
    duration: u64,
}
fn close_pending(pending: &mut BTreeMap<(u32, u64), OpenSpans>, stats: &mut [SpanStatistics]) {
    for ((descriptor, _), open) in std::mem::take(pending) {
        stats[descriptor as usize].missing_ends += open.begins.len();
    }
}
struct Input<'a>(&'a [u8]);
impl<'a> Input<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let (value, rest) = self.0.split_at_checked(count).ok_or_else(|| invalid("Truncated DGTL"))?;
        self.0 = rest;
        Ok(value)
    }
    fn u8(&mut self) -> io::Result<u8> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> io::Result<u16> { Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> io::Result<u32> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> io::Result<u64> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn count(&mut self, limit: usize, minimum_bytes: usize) -> io::Result<usize> {
        let count = self.u32()? as usize;
        if count > limit || count > self.0.len() / minimum_bytes { return Err(invalid("DGTL count exceeds capacity")); }
        Ok(count)
    }
    fn text(&mut self) -> io::Result<&'a str> {
        let count = self.u32()? as usize;
        std::str::from_utf8(self.take(count)?).map_err(invalid)
    }
    fn enumeration<T: Copy>(&mut self, values: &[T], base: u8) -> io::Result<T> {
        let index = self.u8()?.checked_sub(base).ok_or_else(|| invalid("Invalid DGTL enum"))?;
        values.get(index as usize).copied().ok_or_else(|| invalid("Invalid DGTL enum"))
    }
    fn unit(&mut self) -> io::Result<Unit> {
        self.enumeration(&UNITS, 0)
    }
    fn argument(&mut self) -> io::Result<ArgumentDescriptor<'a>> {
        Ok(ArgumentDescriptor {
            name: self.text()?,
            kind: self.enumeration(&ARGUMENT_TYPES, 0)?,
            unit: self.unit()?,
        })
    }
}

impl Collector {
    /// Validate a complete DGTL file before it becomes available for analysis.
    /// Limits apply before count-based allocation. The caller owns the input bytes.
    pub fn read_raw(bytes: &[u8], limits: CollectorLimits) -> io::Result<Self> {
        if bytes.len() > limits.raw_bytes { return Err(invalid("DGTL file exceeds capacity")); }
        let mut input = Input(bytes);
        if input.take(4)? != RAW_MAGIC || input.u16()? != RAW_VERSION || input.u16()? != BATCH_VERSION {
            return Err(invalid("Unsupported DGTL header"));
        }
        let epoch = input.u32()?;
        let session = [input.u32()?, input.u32()?, input.u32()?, input.u32()?];
        let mut collector = Self::new(session, epoch, limits).map_err(invalid)?;
        let sources = input.count(limits.sources, 80)?;
        for _ in 0..sources {
            let source_id = input.u32()?;
            let process_id = input.u32()?;
            let producer_generation = input.u32()?;
            let clock_generation = input.u32()?;
            let name = input.text()?;
            let clock = ClockMapping {
                clock_domain: input.u32()?, origin_tick: input.u64()?, origin_reference_ns: input.u64()?,
                rate_numerator: input.u64()?, rate_denominator: input.u64()?, uncertainty_ns: input.u64()?,
            };
            let count = input.count(limits.descriptors - collector.descriptors, 22)?;
            let mut descriptors = Vec::with_capacity(count);
            for _ in 0..count {
                descriptors.push(Descriptor {
                    category: CategoryId(0), category_name: input.text()?, name: input.text()?,
                    kind: input.enumeration(&[DescriptorKind::Instant, DescriptorKind::Span, DescriptorKind::AsyncSpan, DescriptorKind::Flow], 1)?,
                    severity: input.enumeration(&[Severity::Trace, Severity::Debug, Severity::Info, Severity::Warning, Severity::Error, Severity::Fatal], 0)?,
                    argument0: input.argument()?, argument1: input.argument()?,
                });
            }
            let count = input.count(limits.descriptors - collector.descriptors - count, 10)?;
            let mut metric_descriptors = Vec::with_capacity(count);
            for _ in 0..count {
                metric_descriptors.push(MetricDescriptor {
                    category: CategoryId(0), category_name: input.text()?, name: input.text()?,
                    kind: input.enumeration(&[MetricKind::Counter, MetricKind::Gauge, MetricKind::Maximum, MetricKind::HistogramLog2], 1)?,
                    unit: input.unit()?,
                });
            }
            collector.register_source(SourceRegistration { source_id, process_id, producer_generation, clock_generation,
                name, clock, descriptors: &descriptors, metric_descriptors: &metric_descriptors }).map_err(invalid)?;
            let count = input.count(limits.batches - collector.batches, 100)?;
            let mut scan = Input(input.0);
            let mut total = 0usize;
            for _ in 0..count {
                let length = scan.u32()? as usize;
                let header = decode_batch_header(scan.take(length)?).map_err(invalid)?;
                total = total.checked_add(header.record_count as usize).ok_or_else(|| invalid("DGTL record count overflow"))?;
                if header.source_id != source_id || total > limits.records - collector.records {
                    return Err(invalid("DGTL batch source or capacity mismatch"));
                }
            }
            // Reserve once. A file with many small batches must not cause repeated record copies.
            let source = collector.sources.last_mut().unwrap();
            source.records.try_reserve_exact(total).map_err(invalid)?;
            source.batches.try_reserve_exact(count).map_err(invalid)?;
            for _ in 0..count {
                let length = input.u32()? as usize;
                let bytes = input.take(length)?;
                let header = decode_batch_header(bytes).map_err(invalid)?;
                if header.source_id != source_id || header.record_count as usize > limits.records - collector.records {
                    return Err(invalid("DGTL batch source or capacity mismatch"));
                }
                let mut records = vec![TraceRecord::default(); header.record_count as usize];
                decode_batch_into(bytes, &mut records).map_err(invalid)?;
                collector.ingest(header, &records).map_err(invalid)?;
            }
            let count = input.count(limits.metric_samples - collector.metric_samples, 24)?;
            collector.sources.last_mut().unwrap().metric_samples.try_reserve_exact(count).map_err(invalid)?;
            let identity = ProducerIdentity { session, source_id, generation: producer_generation, clock_generation, clock_domain: clock.clock_domain };
            for _ in 0..count {
                let timestamp = input.u64()?;
                let metric = input.u32()?;
                let bucket = input.u8()?;
                if input.take(3)? != [0; 3] { return Err(invalid("Invalid DGTL metric padding")); }
                let value = input.u64()?;
                collector.ingest_metrics(identity, timestamp, &[MetricSample { metric, bucket, value }]).map_err(invalid)?;
            }
        }
        if !input.0.is_empty() || collector.raw_bytes() != bytes.len() { return Err(invalid("Unexpected DGTL bytes")); }
        Ok(collector)
    }

    /// Source and descriptor counts include loss evidence. All u64 values use decimal strings.
    pub fn write_summary_json(&self, mut output: impl Write, source_id: Option<u32>) -> io::Result<()> {
        self.validate_source_filter(source_id)?;
        write!(output, "{{\"session\":{},\"epoch\":{},\"file_bytes\":{},\"sources\":[", json!(self.session), self.epoch, self.raw_bytes)?;
        let mut first = true;
        for source in self.sources.iter().filter(|s| source_id.is_none_or(|id| s.source_id == id)) {
            write_separator(&mut output, &mut first)?;
            let mut counts = vec![[0usize; 9]; source.descriptors.len()];
            for record in &source.records { counts[record.descriptor as usize][record.phase as usize] += 1; }
            let sites: Vec<_> = source.descriptors.iter().enumerate().map(|(id, descriptor)| json!({
                "id": id, "name": descriptor.name, "category": descriptor.category_name,
                "kind": descriptor.kind, "phase_counts": counts[id],
                "argument0": { "name": descriptor.argument0.name, "type": descriptor.argument0.kind, "unit": descriptor.argument0.unit },
                "argument1": { "name": descriptor.argument1.name, "type": descriptor.argument1.kind, "unit": descriptor.argument1.unit },
            })).collect();
            let metrics: Vec<_> = source.metric_descriptors.iter().enumerate().map(|(id, descriptor)| json!({
                "id": id, "name": descriptor.name, "category": descriptor.category_name, "kind": descriptor.kind, "unit": descriptor.unit,
            })).collect();
            serde_json::to_writer(&mut output, &json!({
                "id": source.source_id, "name": source.name, "process_id": source.process_id,
                "producer_generation": source.producer_generation, "clock_generation": source.clock_generation,
                "clock": { "domain": source.clock.clock_domain, "origin_tick": source.clock.origin_tick.to_string(),
                    "origin_reference_ns": source.clock.origin_reference_ns.to_string(), "rate_numerator": source.clock.rate_numerator.to_string(),
                    "rate_denominator": source.clock.rate_denominator.to_string(), "uncertainty_ns": source.clock.uncertainty_ns.to_string() },
                "records": source.records.len(), "batches": source.batches.len(), "metric_samples": source.metric_samples.len(),
                "dropped_records": source.dropped_records.to_string(), "overwritten_records": source.overwritten_records.to_string(),
                "sequence_gaps": source.sequence_gaps.to_string(), "descriptors": sites, "metrics": metrics,
            }))?;
        }
        output.write_all(b"]}\n")
    }

    /// Stable source/sequence order, not cross-clock timestamp order. Pagination never drops matches silently.
    pub fn write_records_json(&self, mut output: impl Write, source_id: Option<u32>, name: Option<&str>, offset: usize, limit: usize) -> io::Result<()> {
        self.write_filtered_records_json(&mut output, source_id, name, None, offset, limit)
    }

    /// Return records with the same nonzero operation ID. Keep each source identity.
    pub fn write_correlation_json(&self, output: impl Write, correlation: u64, source_id: Option<u32>, offset: usize, limit: usize) -> io::Result<()> {
        if correlation == 0 { return Err(io::Error::new(io::ErrorKind::InvalidInput, "Correlation zero means no operation")); }
        self.write_filtered_records_json(output, source_id, None, Some(correlation), offset, limit)
    }

    fn write_filtered_records_json(&self, mut output: impl Write, source_id: Option<u32>, name: Option<&str>, correlation: Option<u64>, offset: usize, limit: usize) -> io::Result<()> {
        self.validate_source_filter(source_id)?;
        validate_page(limit)?;
        output.write_all(b"{\"records\":[")?;
        let mut returned = 0;
        let mut matched = 0;
        let mut first = true;
        for source in self.sources.iter().filter(|s| source_id.is_none_or(|id| s.source_id == id)) {
            let mut index = 0;
            for batch in &source.batches {
                for ordinal in 0..batch.record_count as usize {
                    let record = &source.records[index];
                    index += 1;
                    let descriptor = &source.descriptors[record.descriptor as usize];
                    if name.is_some_and(|name| descriptor.name != name) || correlation.is_some_and(|id| record.correlation != id) { continue; }
                    matched += 1;
                    if matched <= offset || returned == limit { continue; }
                    write_separator(&mut output, &mut first)?;
                    serde_json::to_writer(&mut output, &json!({
                        "source_id": source.source_id, "source": source.name, "descriptor": record.descriptor,
                        "name": descriptor.name, "category": descriptor.category_name, "phase": record.phase,
                        "sequence": (batch.first_sequence + ordinal as u64).to_string(), "tick": record.timestamp.to_string(),
                        "time_ns": source.clock.map_to_reference_ns(record.timestamp).expect("validated timestamp").to_string(),
                        "correlation": record.correlation.to_string(), "argument0": record.argument0.to_string(),
                        "argument1": record.argument1.to_string(), "flags": record.flags,
                        "arguments": arguments_json(descriptor, record),
                    }))?;
                    returned += 1;
                }
            }
        }
        write_page_end(&mut output, matched, returned, offset)?;
        self.write_analysis_context(&mut output, source_id)
    }

    /// Return metric snapshots without inferring deltas or histogram percentiles.
    pub fn write_metrics_json(&self, mut output: impl Write, source_id: Option<u32>, name: Option<&str>, offset: usize, limit: usize) -> io::Result<()> {
        self.validate_source_filter(source_id)?;
        validate_page(limit)?;
        output.write_all(b"{\"metrics\":[")?;
        let (mut matched, mut returned, mut first) = (0, 0, true);
        for source in self.sources.iter().filter(|s| source_id.is_none_or(|id| s.source_id == id)) {
            for (index, timed) in source.metric_samples.iter().enumerate() {
                let descriptor = &source.metric_descriptors[timed.sample.metric as usize];
                if name.is_some_and(|name| descriptor.name != name) { continue; }
                matched += 1;
                if matched <= offset || returned == limit { continue; }
                write_separator(&mut output, &mut first)?;
                serde_json::to_writer(&mut output, &json!({
                    "source_id": source.source_id, "source": source.name, "sample_index": index,
                    "metric": timed.sample.metric, "name": descriptor.name, "category": descriptor.category_name,
                    "kind": descriptor.kind, "unit": descriptor.unit, "bucket": timed.sample.bucket,
                    "tick": timed.timestamp.to_string(), "time_ns": source.clock.map_to_reference_ns(timed.timestamp).expect("validated timestamp").to_string(),
                    "value": timed.sample.value.to_string(),
                }))?;
                returned += 1;
            }
        }
        write_page_end(&mut output, matched, returned, offset)?;
        self.write_analysis_context(&mut output, source_id)
    }

    /// Pair source-local spans and return the longest observed wall-time intervals first.
    /// Sequence gaps break pairs. Batches with new drops do not supply timing evidence.
    pub fn write_spans_json(&self, mut output: impl Write, source_id: Option<u32>, name: Option<&str>, offset: usize, limit: usize) -> io::Result<()> {
        self.validate_source_filter(source_id)?;
        validate_page(limit)?;
        let mut spans = Vec::new();
        let mut statistics = Vec::new();
        for source in self.sources.iter().filter(|s| source_id.is_none_or(|id| s.source_id == id)) {
            let mut stats: Vec<_> = source.descriptors.iter().map(|_| SpanStatistics::default()).collect();
            let mut pending: BTreeMap<(u32, u64), OpenSpans> = BTreeMap::new();
            let (mut index, mut next_sequence, mut dropped) = (0, None, 0);
            for batch in &source.batches {
                let new_drops = batch.dropped_records > dropped;
                if new_drops || next_sequence.is_some_and(|next| next != batch.first_sequence) {
                    close_pending(&mut pending, &mut stats);
                }
                for ordinal in 0..batch.record_count {
                    let record = &source.records[index];
                    index += 1;
                    let descriptor = &source.descriptors[record.descriptor as usize];
                    if !matches!(descriptor.kind, DescriptorKind::Span | DescriptorKind::AsyncSpan)
                        || name.is_some_and(|name| descriptor.name != name) { continue; }
                    let stat = &mut stats[record.descriptor as usize];
                    if new_drops { stat.excluded_records += 1; continue; }
                    let sequence = batch.first_sequence + ordinal as u64;
                    let key = (record.descriptor, record.correlation);
                    if record.phase == TracePhase::SpanBegin as u8 || record.phase == TracePhase::AsyncBegin as u8 {
                        let open = pending.entry(key).or_default();
                        if descriptor.kind == DescriptorKind::AsyncSpan && !open.begins.is_empty() { open.ambiguous = true; }
                        open.begins.push((index - 1, sequence));
                    } else if let Some(open) = pending.get_mut(&key) {
                        let (begin_index, begin_sequence) = open.begins.pop().expect("nonempty pending spans");
                        if open.ambiguous { stat.ambiguous_pairs += 1; } else {
                            let begin = &source.records[begin_index];
                            let start = source.clock.map_to_reference_ns(begin.timestamp).expect("validated timestamp");
                            let end = source.clock.map_to_reference_ns(record.timestamp).expect("validated timestamp");
                            let duration = end - start;
                            stat.durations.push(duration);
                            spans.push(ObservedSpan { source, begin, end: record, begin_sequence, end_sequence: sequence, start, end_ns: end, duration });
                        }
                        if open.begins.is_empty() { pending.remove(&key); }
                    } else { stat.missing_begins += 1; }
                }
                next_sequence = Some(batch.next_sequence);
                dropped = batch.dropped_records;
            }
            close_pending(&mut pending, &mut stats);
            for (id, mut stat) in stats.into_iter().enumerate() {
                let descriptor = &source.descriptors[id];
                if !matches!(descriptor.kind, DescriptorKind::Span | DescriptorKind::AsyncSpan)
                    || name.is_some_and(|name| descriptor.name != name) { continue; }
                stat.durations.sort_unstable();
                let total: u128 = stat.durations.iter().map(|&ns| ns as u128).sum();
                let count = stat.durations.len();
                let percentile = |percent: usize| {
                    let rank = (count / 100) * percent + ((count % 100) * percent).div_ceil(100);
                    rank.checked_sub(1).map(|index| stat.durations[index].to_string())
                };
                statistics.push(json!({
                    "source_id": source.source_id, "source": source.name, "descriptor": id, "name": descriptor.name, "category": descriptor.category_name,
                    "count": count, "total_ns": total.to_string(), "mean_ns": (count != 0).then(|| (total / count as u128).to_string()),
                    "min_ns": stat.durations.first().map(u64::to_string), "max_ns": stat.durations.last().map(u64::to_string),
                    "p50_ns": percentile(50), "p95_ns": percentile(95), "p99_ns": percentile(99),
                    "missing_begins": stat.missing_begins, "missing_ends": stat.missing_ends,
                    "ambiguous_pairs": stat.ambiguous_pairs, "excluded_records": stat.excluded_records,
                }));
            }
        }
        spans.sort_unstable_by_key(|span| (Reverse(span.duration), span.source.source_id, span.begin_sequence));
        output.write_all(b"{\"spans\":[")?;
        let (mut returned, mut first) = (0, true);
        for span in spans.iter().skip(offset).take(limit) {
            write_separator(&mut output, &mut first)?;
            let descriptor = &span.source.descriptors[span.begin.descriptor as usize];
            serde_json::to_writer(&mut output, &json!({
                "source_id": span.source.source_id, "source": span.source.name, "descriptor": span.begin.descriptor,
                "name": descriptor.name, "category": descriptor.category_name, "kind": descriptor.kind,
                "correlation": span.begin.correlation.to_string(), "begin_sequence": span.begin_sequence.to_string(), "end_sequence": span.end_sequence.to_string(),
                "begin_tick": span.begin.timestamp.to_string(), "end_tick": span.end.timestamp.to_string(),
                "start_ns": span.start.to_string(), "end_ns": span.end_ns.to_string(), "duration_ns": span.duration.to_string(),
                "argument0": span.begin.argument0.to_string(), "argument1": span.begin.argument1.to_string(),
                "begin_arguments": arguments_json(descriptor, span.begin),
                "end_arguments": arguments_json(descriptor, span.end),
            }))?;
            returned += 1;
        }
        write_page_end(&mut output, spans.len(), returned, offset)?;
        write!(output, ",\"statistics\":{},\"time_basis\":\"observed-wall-time\"", json!(statistics))?;
        self.write_analysis_context(&mut output, source_id)
    }

    fn write_analysis_context(&self, output: &mut impl Write, source_id: Option<u32>) -> io::Result<()> {
        write!(output, ",\"session\":{},\"epoch\":{},\"sources\":[", json!(self.session), self.epoch)?;
        let mut first = true;
        for source in self.sources.iter().filter(|s| source_id.is_none_or(|id| s.source_id == id)) {
            write_separator(output, &mut first)?;
            serde_json::to_writer(&mut *output, &json!({
                "id": source.source_id, "name": source.name, "producer_generation": source.producer_generation,
                "clock_generation": source.clock_generation, "clock_domain": source.clock.clock_domain,
                "clock_uncertainty_ns": source.clock.uncertainty_ns.to_string(),
                "dropped_records": source.dropped_records.to_string(), "overwritten_records": source.overwritten_records.to_string(),
                "sequence_gaps": source.sequence_gaps.to_string(),
            }))?;
        }
        output.write_all(b"]}\n")
    }
    fn validate_source_filter(&self, source_id: Option<u32>) -> io::Result<()> {
        if source_id.is_some_and(|id| !self.sources.iter().any(|source| source.source_id == id)) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Unknown source ID"));
        }
        Ok(())
    }
}
