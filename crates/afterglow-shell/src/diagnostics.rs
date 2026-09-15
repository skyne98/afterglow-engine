//! Host-local records. Only cold snapshot bytes cross the JavaScript boundary.
use afterglow_telemetry::connection::{WireArgument, WireDescriptor, WireSource};
use afterglow_telemetry::*;
use deno_core::convert::Uint8Array;
use deno_core::{OpState, op2};
use deno_error::JsErrorBox;
use std::cell::Cell;

pub const CAPACITY: usize = 1024;
const ARG0: ArgumentDescriptor<'static> =
    ArgumentDescriptor::new("value0", ArgumentType::Unsigned, Unit::None);
const ARG1: ArgumentDescriptor<'static> =
    ArgumentDescriptor::new("value1", ArgumentType::Unsigned, Unit::None);
pub const RUNTIME_TURN: u32 = 3;
pub const FRAME: u32 = 4;
pub const PRESENT_WORK: u32 = 5;
pub const REDRAW_REQUEST: u32 = 6;
pub const FOCUS: u32 = 7;
pub const OCCLUDED: u32 = 8;
pub const SUSPENDED: u32 = 9;
pub const HUD_SCENE: u32 = 10;
pub const HUD_COMPOSITE: u32 = 11;
pub const SURFACE_PRESENT: u32 = 12;
static DESCRIPTORS: [Descriptor<'static>; 13] = [
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.present",
        DescriptorKind::Instant,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(1),
        "input",
        "input.dispatch",
        DescriptorKind::Instant,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(2),
        "rpc",
        "rpc.round_trip",
        DescriptorKind::AsyncSpan,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.runtime_turn",
        DescriptorKind::Span,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.frame",
        DescriptorKind::Span,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.present_work",
        DescriptorKind::Span,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.redraw_request",
        DescriptorKind::Instant,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.focus",
        DescriptorKind::Instant,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.occluded",
        DescriptorKind::Instant,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.suspended",
        DescriptorKind::Instant,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.hud_scene",
        DescriptorKind::Span,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.hud_composite",
        DescriptorKind::Span,
        ARG0,
        ARG1,
    ),
    Descriptor::new(
        CategoryId(0),
        "host",
        "host.surface_present",
        DescriptorKind::Span,
        ARG0,
        ARG1,
    ),
];

pub struct HostDiagnostics {
    recorder: Recorder<MonotonicClock>,
    identity: ProducerIdentity,
    correlation: Cell<u64>,
    window_state: [Cell<u8>; 3],
    epoch: u32,
    pub excluded_worker: u32,
}
impl HostDiagnostics {
    pub fn new(excluded_worker: u32) -> Self {
        Self {
            recorder: Recorder::new(&DESCRIPTORS, CAPACITY, MonotonicClock).unwrap(),
            identity: ProducerIdentity::default(),
            correlation: Cell::new(0),
            window_state: [const { Cell::new(2) }; 3],
            epoch: 0,
            excluded_worker,
        }
    }
    fn configure(&mut self, epoch: u32, session: [u32; 4]) -> Result<(), JsErrorBox> {
        if epoch != 0 && session == [0; 4] {
            return Err(JsErrorBox::generic("Invalid capture identity"));
        }
        if self.recorder.state() == CaptureState::Armed {
            self.recorder.stop().unwrap();
        }
        if self.recorder.state() == CaptureState::Frozen {
            self.recorder.reset().unwrap();
        }
        self.epoch = epoch;
        if epoch != 0 {
            self.identity = ProducerIdentity {
                session,
                source_id: 0,
                generation: 1,
                clock_domain: 0,
                clock_generation: 1,
            };
            self.recorder.arm(CaptureConfig::all(epoch)).unwrap();
            for (index, value) in self.window_state.iter().enumerate() {
                self.instant(FOCUS + index as u32, u64::from(value.get()));
            }
        }
        Ok(())
    }
    fn drain(&mut self) -> Vec<u8> {
        if self.recorder.state() != CaptureState::Armed {
            return Vec::new();
        }
        self.recorder.stop().unwrap();
        let snapshot = self.recorder.snapshot().unwrap();
        if snapshot.records.is_empty() {
            self.recorder.resume().unwrap();
            return Vec::new();
        }
        let header = BatchHeader::from_snapshot(self.identity, 1_000_000_000, &snapshot).unwrap();
        let mut bytes = vec![0; encoded_batch_len(snapshot.records.len()).unwrap()];
        encode_batch_into(header, snapshot.records, &mut bytes).unwrap();
        self.recorder.resume().unwrap();
        bytes
    }
    pub fn instant(&self, descriptor: u32, value: u64) {
        self.recorder
            .instant(DescriptorId(descriptor), TraceContext::NONE, value, 0);
    }
    fn next_correlation(&self) -> Option<u64> {
        if self.recorder.state() != CaptureState::Armed {
            return None;
        }
        let id = self.correlation.get().checked_add(1)?;
        self.correlation.set(id);
        Some(id)
    }
    pub fn span_begin(&self, descriptor: u32) -> (u32, u64) {
        let Some(id) = self.next_correlation() else {
            return (0, 0);
        };
        self.recorder
            .span_begin(DescriptorId(descriptor), TraceContext(id), 0, 0);
        (self.epoch, id)
    }
    pub fn span_end(&self, (epoch, id): (u32, u64), descriptor: u32) {
        if id != 0 && epoch == self.epoch {
            self.recorder
                .span_end(DescriptorId(descriptor), TraceContext(id), 0, 0);
        }
    }
    pub fn window_state(&self, descriptor: u32, value: bool) {
        if let Some(index) = descriptor.checked_sub(FOCUS)
            && let Some(state) = self.window_state.get(index as usize)
        {
            state.set(u8::from(value));
            self.instant(descriptor, u64::from(value));
        }
    }
    pub fn rpc_begin(&self, worker: u32, method: u32) -> (u32, u64) {
        if worker == self.excluded_worker || self.recorder.state() != CaptureState::Armed {
            return (0, 0);
        }
        let Some(id) = self.next_correlation() else {
            return (0, 0);
        };
        self.recorder.async_begin(
            DescriptorId(2),
            TraceContext(id),
            u64::from(worker),
            u64::from(method),
        );
        (self.epoch, id)
    }
    pub fn rpc_end(&self, (epoch, id): (u32, u64), bytes: usize, failed: bool) {
        if id != 0 && epoch == self.epoch {
            self.recorder.async_end(
                DescriptorId(2),
                TraceContext(id),
                bytes as u64,
                u64::from(failed),
            );
        }
    }
}

pub fn instant(state: &OpState, descriptor: u32, value: u64) {
    if let Some(host) = state.try_borrow::<HostDiagnostics>() {
        host.instant(descriptor, value);
    }
}

pub fn span_begin(state: &OpState, descriptor: u32) -> (u32, u64) {
    state
        .try_borrow::<HostDiagnostics>()
        .map_or((0, 0), |host| host.span_begin(descriptor))
}
pub fn span_end(state: &OpState, ticket: (u32, u64), descriptor: u32) {
    if let Some(host) = state.try_borrow::<HostDiagnostics>() {
        host.span_end(ticket, descriptor);
    }
}
pub fn window_state(state: &OpState, descriptor: u32, value: bool) {
    if let Some(host) = state.try_borrow::<HostDiagnostics>() {
        host.window_state(descriptor, value);
    }
}

#[op2]
#[string]
pub fn op_diagnostics_capture(
    state: &mut OpState,
    epoch: u32,
    a: u32,
    b: u32,
    c: u32,
    d: u32,
) -> Result<String, JsErrorBox> {
    let Some(host) = state.try_borrow_mut::<HostDiagnostics>() else {
        return Ok(String::new());
    };
    host.configure(epoch, [a, b, c, d])?;
    let argument = |a: ArgumentDescriptor<'_>| WireArgument {
        name: a.name.into(),
        kind: a.kind,
        unit: a.unit,
    };
    let source = WireSource {
        source_id: 0,
        producer_generation: 1,
        clock_generation: 1,
        process_id: std::process::id(),
        name: "afterglow-shell".into(),
        clock: ClockMapping::native(0),
        descriptors: DESCRIPTORS
            .iter()
            .map(|d| WireDescriptor {
                category: d.category.0,
                category_name: d.category_name.into(),
                name: d.name.into(),
                kind: d.kind,
                argument0: argument(d.argument0),
                argument1: argument(d.argument1),
                severity: d.severity,
            })
            .collect(),
        metric_descriptors: Vec::new(),
    };
    serde_json::to_string(&source).map_err(|e| JsErrorBox::generic(e.to_string()))
}

#[op2]
pub fn op_diagnostics_drain(state: &mut OpState) -> Uint8Array {
    state
        .try_borrow_mut::<HostDiagnostics>()
        .map_or_else(Vec::new, HostDiagnostics::drain)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_spans_keep_identity_and_window_state_across_capture_boundaries() {
        let mut host = HostDiagnostics::new(9);
        assert_eq!(host.span_begin(FRAME), (0, 0));
        host.window_state(FOCUS, true);
        host.window_state(SUSPENDED, false);
        host.configure(1, [1, 2, 3, 4]).unwrap();
        let mut records = [TraceRecord::default(); 16];
        let (_, count) = decode_batch_into(&host.drain(), &mut records).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            records[..3]
                .iter()
                .map(|r| (r.descriptor, r.argument0))
                .collect::<Vec<_>>(),
            [(FOCUS, 1), (OCCLUDED, 2), (SUSPENDED, 0)]
        );
        let frame = host.span_begin(FRAME);
        let present = host.span_begin(PRESENT_WORK);
        host.span_end(present, PRESENT_WORK);
        host.span_end(frame, FRAME);
        let (_, count) = decode_batch_into(&host.drain(), &mut records).unwrap();
        assert_eq!(count, 4);
        assert_ne!(frame.1, present.1);
        assert_eq!(records[0].correlation, records[3].correlation);
        assert_eq!(records[1].correlation, records[2].correlation);
        assert_eq!((records[0].phase, records[3].phase), (2, 3));
        host.configure(2, [1, 2, 3, 4]).unwrap();
        host.drain();
        host.span_end(frame, FRAME);
        assert!(host.drain().is_empty());
        host.correlation.set(u64::MAX);
        assert_eq!(host.span_begin(FRAME), (0, 0));
        assert_eq!(host.rpc_begin(1, 0), (0, 0));
        assert!(host.drain().is_empty());
    }

    #[test]
    fn host_capture_is_bounded_continuous_and_disabled_without_a_viewer() {
        for descriptor in DESCRIPTORS {
            assert_ne!(descriptor.argument0.name, descriptor.argument1.name);
        }
        let mut host = HostDiagnostics::new(9);
        assert!(host.drain().is_empty());
        host.configure(1, [1, 2, 3, 4]).unwrap();
        assert_eq!(decode_batch_header(&host.drain()).unwrap().record_count, 3);
        assert_eq!(host.rpc_begin(9, 0), (0, 0));
        for _ in 0..CAPACITY + 5 {
            host.instant(0, 1);
        }
        let first = decode_batch_header(&host.drain()).unwrap();
        assert_eq!(first.record_count as usize, CAPACITY);
        assert_eq!(first.dropped_records, 5);
        host.instant(1, 0);
        assert_eq!(
            decode_batch_header(&host.drain()).unwrap().first_sequence,
            CAPACITY as u64 + 3
        );
        let old_rpc = host.rpc_begin(1, 0);
        host.configure(0, [0; 4]).unwrap();
        assert!(host.drain().is_empty());
        host.configure(2, [1, 2, 3, 4]).unwrap();
        host.drain();
        host.rpc_end(old_rpc, 10, false);
        assert!(host.drain().is_empty());
        host.instant(0, 1);
        let next = decode_batch_header(&host.drain()).unwrap();
        assert_eq!(next.record_count, 1);
        assert_eq!(next.epoch, 2);
        assert_eq!(next.first_sequence, 3);
    }
}
