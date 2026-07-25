//! Producer-local finite trace capture.

use std::cell::{Cell, UnsafeCell};

use crate::clock::Clock;
use crate::descriptor::{Descriptor, DescriptorId, DescriptorKind};
use crate::record::{TraceContext, TracePhase, TraceRecord};

const CATEGORY_WORDS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CategoryMask([u64; CATEGORY_WORDS]);

impl CategoryMask {
    pub const fn none() -> Self {
        Self([0; CATEGORY_WORDS])
    }

    pub const fn all() -> Self {
        Self([u64::MAX; CATEGORY_WORDS])
    }

    pub fn enable(&mut self, category: u8) {
        self.0[category as usize / 64] |= 1_u64 << (category as usize % 64);
    }

    pub fn disable(&mut self, category: u8) {
        self.0[category as usize / 64] &= !(1_u64 << (category as usize % 64));
    }

    pub fn contains(self, category: u8) -> bool {
        self.0[category as usize / 64] & (1_u64 << (category as usize % 64)) != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureConfig {
    pub epoch: u32,
    pub categories: CategoryMask,
}

impl CaptureConfig {
    pub const fn all(epoch: u32) -> Self {
        Self {
            epoch,
            categories: CategoryMask::all(),
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureState {
    #[default]
    Idle = 0,
    Armed = 1,
    Frozen = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureError {
    ZeroCapacity,
    InvalidTransition {
        from: CaptureState,
        operation: &'static str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordStatus {
    Recorded,
    Disabled,
    CategoryDisabled,
    InvalidDescriptor,
    WrongDescriptorKind,
    BufferFull,
}

pub struct CaptureSnapshot<'a> {
    pub epoch: u32,
    pub records: &'a [TraceRecord],
    pub dropped_records: u64,
    pub capacity: usize,
}

/// One fixed-capacity recorder owned by exactly one execution domain.
///
/// `Recorder` is deliberately `!Sync`: producer writes are local and avoid
/// atomic cursor traffic. Capture coordination must arm/stop the recorder on
/// its owning thread or worker. The internal `UnsafeCell` permits allocation-
/// free RAII span guards to retain `&Recorder` while nested events are emitted.
pub struct Recorder<C> {
    descriptors: &'static [Descriptor],
    clock: C,
    records: UnsafeCell<Box<[TraceRecord]>>,
    enabled_descriptors: Box<[u64]>,
    state: Cell<CaptureState>,
    epoch: Cell<u32>,
    length: Cell<usize>,
    dropped: Cell<u64>,
}

impl<C: Clock> Recorder<C> {
    pub fn new(
        descriptors: &'static [Descriptor],
        capacity: usize,
        clock: C,
    ) -> Result<Self, CaptureError> {
        if capacity == 0 {
            return Err(CaptureError::ZeroCapacity);
        }
        let records = vec![TraceRecord::default(); capacity].into_boxed_slice();
        let enabled_descriptors = vec![0_u64; descriptors.len().div_ceil(64)].into_boxed_slice();
        Ok(Self {
            descriptors,
            clock,
            records: UnsafeCell::new(records),
            enabled_descriptors,
            state: Cell::new(CaptureState::Idle),
            epoch: Cell::new(0),
            length: Cell::new(0),
            dropped: Cell::new(0),
        })
    }

    pub fn descriptors(&self) -> &'static [Descriptor] {
        self.descriptors
    }

    pub fn capacity(&self) -> usize {
        // SAFETY: the boxed slice is never resized or replaced.
        unsafe { (&*self.records.get()).len() }
    }

    pub fn state(&self) -> CaptureState {
        self.state.get()
    }

    pub fn arm(&mut self, config: CaptureConfig) -> Result<(), CaptureError> {
        if self.state.get() != CaptureState::Idle {
            return Err(CaptureError::InvalidTransition {
                from: self.state.get(),
                operation: "arm",
            });
        }
        self.length.set(0);
        self.dropped.set(0);
        self.epoch.set(config.epoch);
        self.enabled_descriptors.fill(0);
        for (index, descriptor) in self.descriptors.iter().enumerate() {
            if config.categories.contains(descriptor.category.0) {
                self.enabled_descriptors[index / 64] |= 1_u64 << (index % 64);
            }
        }
        self.state.set(CaptureState::Armed);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), CaptureError> {
        if self.state.get() != CaptureState::Armed {
            return Err(CaptureError::InvalidTransition {
                from: self.state.get(),
                operation: "stop",
            });
        }
        self.state.set(CaptureState::Frozen);
        Ok(())
    }

    pub fn snapshot(&self) -> Result<CaptureSnapshot<'_>, CaptureError> {
        if self.state.get() != CaptureState::Frozen {
            return Err(CaptureError::InvalidTransition {
                from: self.state.get(),
                operation: "snapshot",
            });
        }
        // SAFETY: Frozen recorders cannot be written. `reset` requires `&mut
        // self`, so the returned borrow prevents a concurrent state change.
        let records = unsafe { &*self.records.get() };
        Ok(CaptureSnapshot {
            epoch: self.epoch.get(),
            records: &records[..self.length.get()],
            dropped_records: self.dropped.get(),
            capacity: records.len(),
        })
    }

    pub fn reset(&mut self) -> Result<(), CaptureError> {
        if self.state.get() != CaptureState::Frozen {
            return Err(CaptureError::InvalidTransition {
                from: self.state.get(),
                operation: "reset",
            });
        }
        self.length.set(0);
        self.dropped.set(0);
        self.enabled_descriptors.fill(0);
        self.state.set(CaptureState::Idle);
        Ok(())
    }

    #[inline]
    pub fn instant(
        &self,
        descriptor: DescriptorId,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> RecordStatus {
        self.record_checked(
            descriptor,
            DescriptorKind::Instant,
            TracePhase::Instant,
            context,
            argument0,
            argument1,
        )
    }

    #[inline]
    pub fn span_begin(
        &self,
        descriptor: DescriptorId,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> RecordStatus {
        self.record_checked(
            descriptor,
            DescriptorKind::Span,
            TracePhase::SpanBegin,
            context,
            argument0,
            argument1,
        )
    }

    #[inline]
    pub fn span_end(
        &self,
        descriptor: DescriptorId,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> RecordStatus {
        self.record_checked(
            descriptor,
            DescriptorKind::Span,
            TracePhase::SpanEnd,
            context,
            argument0,
            argument1,
        )
    }

    pub fn span(
        &self,
        descriptor: DescriptorId,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> SpanGuard<'_, C> {
        let active =
            self.span_begin(descriptor, context, argument0, argument1) == RecordStatus::Recorded;
        SpanGuard {
            recorder: self,
            descriptor,
            context,
            active,
        }
    }

    #[inline]
    pub fn async_begin(
        &self,
        descriptor: DescriptorId,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> RecordStatus {
        self.record_checked(
            descriptor,
            DescriptorKind::AsyncSpan,
            TracePhase::AsyncBegin,
            context,
            argument0,
            argument1,
        )
    }

    #[inline]
    pub fn async_end(
        &self,
        descriptor: DescriptorId,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> RecordStatus {
        self.record_checked(
            descriptor,
            DescriptorKind::AsyncSpan,
            TracePhase::AsyncEnd,
            context,
            argument0,
            argument1,
        )
    }

    #[inline]
    pub fn flow_start(
        &self,
        descriptor: DescriptorId,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> RecordStatus {
        self.record_checked(
            descriptor,
            DescriptorKind::Flow,
            TracePhase::FlowStart,
            context,
            argument0,
            argument1,
        )
    }

    #[inline]
    pub fn flow_step(
        &self,
        descriptor: DescriptorId,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> RecordStatus {
        self.record_checked(
            descriptor,
            DescriptorKind::Flow,
            TracePhase::FlowStep,
            context,
            argument0,
            argument1,
        )
    }

    #[inline]
    pub fn flow_end(
        &self,
        descriptor: DescriptorId,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> RecordStatus {
        self.record_checked(
            descriptor,
            DescriptorKind::Flow,
            TracePhase::FlowEnd,
            context,
            argument0,
            argument1,
        )
    }

    #[inline]
    fn record_checked(
        &self,
        descriptor: DescriptorId,
        expected: DescriptorKind,
        phase: TracePhase,
        context: TraceContext,
        argument0: u64,
        argument1: u64,
    ) -> RecordStatus {
        // Keep the disabled branch before descriptor lookup and clock access.
        if self.state.get() != CaptureState::Armed {
            return RecordStatus::Disabled;
        }
        let index = descriptor.0 as usize;
        let Some(metadata) = self.descriptors.get(index) else {
            return RecordStatus::InvalidDescriptor;
        };
        if metadata.kind != expected {
            return RecordStatus::WrongDescriptorKind;
        }
        if self.enabled_descriptors[index / 64] & (1_u64 << (index % 64)) == 0 {
            return RecordStatus::CategoryDisabled;
        }
        let slot = self.length.get();
        if slot == self.capacity() {
            self.dropped.set(self.dropped.get().saturating_add(1));
            return RecordStatus::BufferFull;
        }
        let record = TraceRecord {
            timestamp: self.clock.now(),
            correlation: context.0,
            argument0,
            argument1,
            descriptor: descriptor.0,
            phase: phase as u8,
            flags: 0,
            reserved: 0,
        };
        // SAFETY: Recorder is !Sync and has one producer. Each call writes the
        // next unique slot; snapshots are only exposed in Frozen state.
        unsafe {
            (&mut *self.records.get())[slot] = record;
        }
        self.length.set(slot + 1);
        RecordStatus::Recorded
    }
}

#[must_use = "dropping the guard closes the telemetry span"]
pub struct SpanGuard<'a, C: Clock> {
    recorder: &'a Recorder<C>,
    descriptor: DescriptorId,
    context: TraceContext,
    active: bool,
}

impl<C: Clock> SpanGuard<'_, C> {
    pub fn finish(mut self, argument0: u64, argument1: u64) -> RecordStatus {
        if !self.active {
            return RecordStatus::Disabled;
        }
        self.active = false;
        self.recorder
            .span_end(self.descriptor, self.context, argument0, argument1)
    }
}

impl<C: Clock> Drop for SpanGuard<'_, C> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.recorder.span_end(self.descriptor, self.context, 0, 0);
        }
    }
}
