//! Allocation-free diagnostics batches. Old AGTB input is not accepted.

use crate::record::{TRACE_RECORD_BYTES, TraceRecord};
use crate::recorder::{CaptureRetention, CaptureSnapshot};

pub const BATCH_MAGIC: [u8; 4] = *b"DGTB";
pub const BATCH_VERSION: u16 = 1;
pub const BATCH_HEADER_BYTES: usize = 96;
pub const BATCH_ROLLING: u32 = 1;

/// The application assigns an identity before recording starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "collector",
    derive(serde::Serialize, serde::Deserialize),
    serde(deny_unknown_fields)
)]
pub struct ProducerIdentity {
    pub session: [u32; 4],
    pub source_id: u32,
    pub generation: u32,
    pub clock_domain: u32,
    pub clock_generation: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatchHeader {
    pub source_id: u32,
    pub epoch: u32,
    pub clock_domain: u32,
    pub flags: u32,
    pub record_count: u32,
    pub ticks_per_second: u64,
    pub session: [u32; 4],
    pub producer_generation: u32,
    pub clock_generation: u32,
    pub first_sequence: u64,
    pub next_sequence: u64,
    pub dropped_records: u64,
    pub overwritten_records: u64,
}

impl BatchHeader {
    pub fn from_snapshot(
        identity: ProducerIdentity,
        ticks_per_second: u64,
        snapshot: &CaptureSnapshot<'_>,
    ) -> Result<Self, BatchError> {
        let count =
            u32::try_from(snapshot.records.len()).map_err(|_| BatchError::LengthOverflow)?;
        let header = Self {
            source_id: identity.source_id,
            epoch: snapshot.epoch,
            clock_domain: identity.clock_domain,
            flags: if snapshot.retention == CaptureRetention::Rolling {
                BATCH_ROLLING
            } else {
                0
            },
            record_count: count,
            ticks_per_second,
            session: identity.session,
            producer_generation: identity.generation,
            clock_generation: identity.clock_generation,
            first_sequence: snapshot.first_sequence,
            next_sequence: snapshot.next_sequence,
            dropped_records: snapshot.dropped_records,
            overwritten_records: snapshot.overwritten_records,
        };
        validate_header(header)?;
        Ok(header)
    }

    pub fn identity(self) -> ProducerIdentity {
        ProducerIdentity {
            session: self.session,
            source_id: self.source_id,
            generation: self.producer_generation,
            clock_domain: self.clock_domain,
            clock_generation: self.clock_generation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchError {
    OutputTooSmall { needed: usize, provided: usize },
    InputTooSmall,
    BadMagic,
    UnsupportedVersion(u16),
    BadHeaderLength(u16),
    LengthOverflow,
    LengthMismatch { expected: usize, actual: usize },
    RecordCountMismatch { header: u32, records: usize },
    InvalidIdentity,
    InvalidSequence,
    InvalidTickRate,
    InvalidFlags,
    InvalidRecord { index: usize },
}

pub(crate) fn validate_batch(
    header: BatchHeader,
    records: &[TraceRecord],
) -> Result<(), BatchError> {
    validate_header(header)?;
    if header.record_count as usize != records.len() {
        return Err(BatchError::RecordCountMismatch {
            header: header.record_count,
            records: records.len(),
        });
    }
    let mut previous = 0;
    for (index, record) in records.iter().enumerate() {
        validate_record(record, previous, index)?;
        previous = record.timestamp;
    }
    Ok(())
}

fn validate_header(header: BatchHeader) -> Result<(), BatchError> {
    if header.session == [0; 4] || header.producer_generation == 0 || header.clock_generation == 0 {
        return Err(BatchError::InvalidIdentity);
    }
    if header.ticks_per_second == 0 {
        return Err(BatchError::InvalidTickRate);
    }
    if header.flags & !BATCH_ROLLING != 0 {
        return Err(BatchError::InvalidFlags);
    }
    if header.next_sequence.checked_sub(header.first_sequence) != Some(header.record_count as u64)
        || header.overwritten_records > header.first_sequence
        || (header.flags & BATCH_ROLLING == 0 && header.overwritten_records != 0)
    {
        return Err(BatchError::InvalidSequence);
    }
    Ok(())
}

fn validate_record(record: &TraceRecord, previous: u64, index: usize) -> Result<(), BatchError> {
    if !(1..=8).contains(&record.phase)
        || record.flags != 0
        || record.reserved != 0
        || record.timestamp < previous
    {
        return Err(BatchError::InvalidRecord { index });
    }
    Ok(())
}

pub fn encoded_batch_len(record_count: usize) -> Option<usize> {
    record_count
        .checked_mul(TRACE_RECORD_BYTES)?
        .checked_add(BATCH_HEADER_BYTES)
}

pub fn encode_batch_into(
    header: BatchHeader,
    records: &[TraceRecord],
    output: &mut [u8],
) -> Result<usize, BatchError> {
    validate_batch(header, records)?;
    let needed = encoded_batch_len(records.len()).ok_or(BatchError::LengthOverflow)?;
    if output.len() < needed {
        return Err(BatchError::OutputTooSmall {
            needed,
            provided: output.len(),
        });
    }
    output[..4].copy_from_slice(&BATCH_MAGIC);
    put_u16(output, 4, BATCH_VERSION);
    put_u16(output, 6, BATCH_HEADER_BYTES as u16);
    put_u32(output, 8, header.source_id);
    put_u32(output, 12, header.epoch);
    put_u32(output, 16, header.clock_domain);
    put_u32(output, 20, header.flags);
    put_u32(output, 24, header.record_count);
    put_u32(output, 28, 0);
    put_u64(output, 32, header.ticks_per_second);
    for (index, word) in header.session.iter().enumerate() {
        put_u32(output, 40 + index * 4, *word);
    }
    put_u32(output, 56, header.producer_generation);
    put_u32(output, 60, header.clock_generation);
    put_u64(output, 64, header.first_sequence);
    put_u64(output, 72, header.next_sequence);
    put_u64(output, 80, header.dropped_records);
    put_u64(output, 88, header.overwritten_records);
    for (index, record) in records.iter().enumerate() {
        let offset = BATCH_HEADER_BYTES + index * TRACE_RECORD_BYTES;
        put_u64(output, offset, record.timestamp);
        put_u64(output, offset + 8, record.correlation);
        put_u64(output, offset + 16, record.argument0);
        put_u64(output, offset + 24, record.argument1);
        put_u32(output, offset + 32, record.descriptor);
        output[offset + 36] = record.phase;
        output[offset + 37] = record.flags;
        put_u16(output, offset + 38, record.reserved);
    }
    Ok(needed)
}

pub fn decode_batch_into(
    input: &[u8],
    records: &mut [TraceRecord],
) -> Result<(BatchHeader, usize), BatchError> {
    let header = decode_batch_header(input)?;
    let count = header.record_count as usize;
    if records.len() < count {
        return Err(BatchError::OutputTooSmall {
            needed: count,
            provided: records.len(),
        });
    }
    let mut previous = 0;
    for index in 0..count {
        let record = read_record(input, BATCH_HEADER_BYTES + index * TRACE_RECORD_BYTES);
        validate_record(&record, previous, index)?;
        previous = record.timestamp;
    }
    for (index, record) in records[..count].iter_mut().enumerate() {
        *record = read_record(input, BATCH_HEADER_BYTES + index * TRACE_RECORD_BYTES);
    }
    Ok((header, count))
}

/// Read framing before allocation. Record validation remains mandatory.
pub fn decode_batch_header(input: &[u8]) -> Result<BatchHeader, BatchError> {
    if input.len() < BATCH_HEADER_BYTES {
        return Err(BatchError::InputTooSmall);
    }
    if input[..4] != BATCH_MAGIC {
        return Err(BatchError::BadMagic);
    }
    let version = get_u16(input, 4);
    if version != BATCH_VERSION {
        return Err(BatchError::UnsupportedVersion(version));
    }
    let length = get_u16(input, 6);
    if length as usize != BATCH_HEADER_BYTES {
        return Err(BatchError::BadHeaderLength(length));
    }
    if get_u32(input, 28) != 0 {
        return Err(BatchError::InvalidFlags);
    }
    let header = BatchHeader {
        source_id: get_u32(input, 8),
        epoch: get_u32(input, 12),
        clock_domain: get_u32(input, 16),
        flags: get_u32(input, 20),
        record_count: get_u32(input, 24),
        ticks_per_second: get_u64(input, 32),
        session: [
            get_u32(input, 40),
            get_u32(input, 44),
            get_u32(input, 48),
            get_u32(input, 52),
        ],
        producer_generation: get_u32(input, 56),
        clock_generation: get_u32(input, 60),
        first_sequence: get_u64(input, 64),
        next_sequence: get_u64(input, 72),
        dropped_records: get_u64(input, 80),
        overwritten_records: get_u64(input, 88),
    };
    validate_header(header)?;
    let expected =
        encoded_batch_len(header.record_count as usize).ok_or(BatchError::LengthOverflow)?;
    if input.len() != expected {
        return Err(BatchError::LengthMismatch {
            expected,
            actual: input.len(),
        });
    }
    Ok(header)
}

fn read_record(input: &[u8], offset: usize) -> TraceRecord {
    TraceRecord {
        timestamp: get_u64(input, offset),
        correlation: get_u64(input, offset + 8),
        argument0: get_u64(input, offset + 16),
        argument1: get_u64(input, offset + 24),
        descriptor: get_u32(input, offset + 32),
        phase: input[offset + 36],
        flags: input[offset + 37],
        reserved: get_u16(input, offset + 38),
    }
}
fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn get_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        input[offset..offset + 2]
            .try_into()
            .expect("validated bounds"),
    )
}
fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        input[offset..offset + 4]
            .try_into()
            .expect("validated bounds"),
    )
}
fn get_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        input[offset..offset + 8]
            .try_into()
            .expect("validated bounds"),
    )
}
