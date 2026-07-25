//! Compact dynamic trace records.

/// One operation identity shared by related events across producers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TraceContext(pub u64);

impl TraceContext {
    pub const NONE: Self = Self(0);

    pub const fn from_parts(namespace: u32, local: u32) -> Self {
        Self(((namespace as u64) << 32) | local as u64)
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TracePhase {
    #[default]
    Instant = 1,
    SpanBegin = 2,
    SpanEnd = 3,
    AsyncBegin = 4,
    AsyncEnd = 5,
    FlowStart = 6,
    FlowStep = 7,
    FlowEnd = 8,
}

/// Fixed-width event payload. Producer identity and clock metadata live in the
/// surrounding batch header, keeping every hot-path record at 40 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TraceRecord {
    pub timestamp: u64,
    pub correlation: u64,
    pub argument0: u64,
    pub argument1: u64,
    pub descriptor: u32,
    pub phase: u8,
    pub flags: u8,
    pub reserved: u16,
}

pub const TRACE_RECORD_BYTES: usize = 40;

const _: () = assert!(size_of::<TraceRecord>() == TRACE_RECORD_BYTES);
