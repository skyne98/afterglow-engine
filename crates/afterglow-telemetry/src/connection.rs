//! Cold profiling frames. Not a worker payload transport.

use crate::{
    ArgumentDescriptor, ArgumentType, CategoryId, ClockMapping, Collector, CollectorError,
    Descriptor, DescriptorKind, MetricDescriptor, MetricKind, MetricSample, ProducerIdentity,
    Severity, SourceRegistration, TraceRecord, Unit, decode_batch_header, decode_batch_into,
};
use serde::{Deserialize, Serialize};

/// The application sends this header after connection to the profiling server.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureHello {
    pub protocol: u32,
    pub session: [u32; 4],
    pub epoch: u32,
    pub max_frame_bytes: usize,
}

pub const PROTOCOL_VERSION: u32 = 3;
pub const HELLO: u8 = 0;
pub const REGISTER: u8 = 1;
pub const BATCH: u8 = 2;
pub const METRICS: u8 = 3;
pub const FINISH: u8 = 4;

// One binary WebSocket message contains one opcode followed by payload bytes.

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireArgument {
    pub name: String,
    pub kind: ArgumentType,
    pub unit: Unit,
}
impl WireArgument {
    fn borrowed(&self) -> ArgumentDescriptor<'_> {
        ArgumentDescriptor::new(&self.name, self.kind, self.unit)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireDescriptor {
    pub category: u8,
    pub category_name: String,
    pub name: String,
    pub kind: DescriptorKind,
    pub argument0: WireArgument,
    pub argument1: WireArgument,
    pub severity: Severity,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireMetricDescriptor {
    pub category: u8,
    pub category_name: String,
    pub name: String,
    pub kind: MetricKind,
    pub unit: Unit,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireSource {
    pub source_id: u32,
    pub producer_generation: u32,
    pub clock_generation: u32,
    pub process_id: u32,
    pub name: String,
    pub clock: ClockMapping,
    pub descriptors: Vec<WireDescriptor>,
    pub metric_descriptors: Vec<WireMetricDescriptor>,
}
impl WireSource {
    pub fn register(&self, collector: &mut Collector) -> Result<(), CollectorError> {
        let descriptors: Vec<_> = self
            .descriptors
            .iter()
            .map(|d| {
                Descriptor::new(
                    CategoryId(d.category),
                    &d.category_name,
                    &d.name,
                    d.kind,
                    d.argument0.borrowed(),
                    d.argument1.borrowed(),
                )
                .with_severity(d.severity)
            })
            .collect();
        let metrics: Vec<_> = self
            .metric_descriptors
            .iter()
            .map(|d| {
                MetricDescriptor::new(
                    CategoryId(d.category),
                    &d.category_name,
                    &d.name,
                    d.kind,
                    d.unit,
                )
            })
            .collect();
        collector.register_source(SourceRegistration {
            source_id: self.source_id,
            producer_generation: self.producer_generation,
            clock_generation: self.clock_generation,
            process_id: self.process_id,
            name: &self.name,
            clock: self.clock,
            descriptors: &descriptors,
            metric_descriptors: &metrics,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireMetrics {
    pub identity: ProducerIdentity,
    pub timestamp: u64,
    pub samples: Vec<MetricSample>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum IngestError {
    InvalidMessage,
    Collector(CollectorError),
}

/// The receiver bounds payload bytes and scratch records first.
/// JSON parsing and metadata copies are cold work, not sealed application work.
pub fn ingest_frame(
    collector: &mut Collector,
    opcode: u8,
    payload: &[u8],
    records: &mut [TraceRecord],
    max_payload_bytes: usize,
) -> Result<(), IngestError> {
    if payload.len() > max_payload_bytes {
        return Err(IngestError::InvalidMessage);
    }
    match opcode {
        REGISTER => {
            let source: WireSource =
                serde_json::from_slice(payload).map_err(|_| IngestError::InvalidMessage)?;
            source.register(collector).map_err(IngestError::Collector)
        }
        BATCH => {
            let header = decode_batch_header(payload).map_err(|_| IngestError::InvalidMessage)?;
            if header.record_count as usize > records.len() {
                return Err(IngestError::InvalidMessage);
            }
            let (_, count) =
                decode_batch_into(payload, records).map_err(|_| IngestError::InvalidMessage)?;
            collector
                .ingest(header, &records[..count])
                .map_err(IngestError::Collector)
        }
        METRICS => {
            let snapshot: WireMetrics =
                serde_json::from_slice(payload).map_err(|_| IngestError::InvalidMessage)?;
            collector
                .ingest_metrics(snapshot.identity, snapshot.timestamp, &snapshot.samples)
                .map_err(IngestError::Collector)
        }
        _ => Err(IngestError::InvalidMessage),
    }
}
