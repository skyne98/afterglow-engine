//! Versioned allocation-free trace batch encoding.

use crate::record::{TRACE_RECORD_BYTES, TraceRecord};
use crate::recorder::CaptureSnapshot;

pub const BATCH_MAGIC: [u8; 4] = *b"AGTB";
pub const BATCH_VERSION: u16 = 1;
pub const BATCH_HEADER_BYTES: usize = 40;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatchHeader {
    pub source_id: u32,
    pub epoch: u32,
    pub clock_domain: u32,
    pub flags: u32,
    pub record_count: u32,
    pub dropped_records: u32,
    pub ticks_per_second: u64,
}

impl BatchHeader {
    pub fn from_snapshot(
        source_id: u32,
        clock_domain: u32,
        ticks_per_second: u64,
        snapshot: &CaptureSnapshot<'_>,
    ) -> Self {
        Self {
            source_id,
            epoch: snapshot.epoch,
            clock_domain,
            flags: 0,
            record_count: snapshot.records.len().min(u32::MAX as usize) as u32,
            dropped_records: snapshot.dropped_records.min(u32::MAX as u64) as u32,
            ticks_per_second,
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
    if header.record_count as usize != records.len() {
        return Err(BatchError::RecordCountMismatch {
            header: header.record_count,
            records: records.len(),
        });
    }
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
    put_u32(output, 28, header.dropped_records);
    put_u64(output, 32, header.ticks_per_second);
    let mut offset = BATCH_HEADER_BYTES;
    for record in records {
        put_u64(output, offset, record.timestamp);
        put_u64(output, offset + 8, record.correlation);
        put_u64(output, offset + 16, record.argument0);
        put_u64(output, offset + 24, record.argument1);
        put_u32(output, offset + 32, record.descriptor);
        output[offset + 36] = record.phase;
        output[offset + 37] = record.flags;
        put_u16(output, offset + 38, record.reserved);
        offset += TRACE_RECORD_BYTES;
    }
    Ok(needed)
}

pub fn decode_batch_into(
    input: &[u8],
    records: &mut [TraceRecord],
) -> Result<(BatchHeader, usize), BatchError> {
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
    let header_length = get_u16(input, 6);
    if header_length as usize != BATCH_HEADER_BYTES {
        return Err(BatchError::BadHeaderLength(header_length));
    }
    let header = BatchHeader {
        source_id: get_u32(input, 8),
        epoch: get_u32(input, 12),
        clock_domain: get_u32(input, 16),
        flags: get_u32(input, 20),
        record_count: get_u32(input, 24),
        dropped_records: get_u32(input, 28),
        ticks_per_second: get_u64(input, 32),
    };
    let count = header.record_count as usize;
    let expected = encoded_batch_len(count).ok_or(BatchError::LengthOverflow)?;
    if input.len() != expected {
        return Err(BatchError::LengthMismatch {
            expected,
            actual: input.len(),
        });
    }
    if records.len() < count {
        return Err(BatchError::OutputTooSmall {
            needed: count,
            provided: records.len(),
        });
    }
    let mut offset = BATCH_HEADER_BYTES;
    for record in &mut records[..count] {
        *record = TraceRecord {
            timestamp: get_u64(input, offset),
            correlation: get_u64(input, offset + 8),
            argument0: get_u64(input, offset + 16),
            argument1: get_u64(input, offset + 24),
            descriptor: get_u32(input, offset + 32),
            phase: input[offset + 36],
            flags: input[offset + 37],
            reserved: get_u16(input, offset + 38),
        };
        offset += TRACE_RECORD_BYTES;
    }
    Ok((header, count))
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
            .expect("validated batch bounds"),
    )
}

fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        input[offset..offset + 4]
            .try_into()
            .expect("validated batch bounds"),
    )
}

fn get_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        input[offset..offset + 8]
            .try_into()
            .expect("validated batch bounds"),
    )
}
