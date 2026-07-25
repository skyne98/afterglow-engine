use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadinessSubsystem {
    Adapter,
    Device,
    Canvas,
    Pipeline,
    Resource,
    Worker,
    Renderer,
}

#[derive(Debug, Clone)]
pub struct PendingOperation {
    pub id: u64,
    pub subsystem: ReadinessSubsystem,
    pub operation: String,
    pub source: Option<String>,
    pub elapsed: Duration,
}

struct OperationState {
    subsystem: ReadinessSubsystem,
    operation: String,
    source: Option<String>,
    started: Instant,
}

#[derive(Debug, Clone)]
pub struct ReadinessFailure {
    pub subsystem: ReadinessSubsystem,
    pub operation: String,
    pub source: Option<String>,
    pub error: String,
}

impl fmt::Display for ReadinessFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} readiness operation {:?} failed: {}",
            self.subsystem, self.operation, self.error
        )?;
        if let Some(source) = &self.source {
            write!(formatter, " ({source})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ReadinessFailure {}

#[derive(Default)]
struct ReadinessState {
    pending: BTreeMap<u64, OperationState>,
    failures: Vec<ReadinessFailure>,
}

#[derive(Default)]
pub struct ReadinessRegistry {
    next_id: AtomicU64,
    state: Mutex<ReadinessState>,
    canvas_count: AtomicU32,
    gpu_submissions: AtomicU64,
    submitted_canvases: Mutex<BTreeSet<u64>>,
}

impl ReadinessRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn begin(
        self: &Arc<Self>,
        subsystem: ReadinessSubsystem,
        operation: impl Into<String>,
        source: Option<String>,
    ) -> ReadinessToken {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.state.lock().unwrap().pending.insert(
            id,
            OperationState {
                subsystem,
                operation: operation.into(),
                source,
                started: Instant::now(),
            },
        );
        ReadinessToken {
            id,
            registry: self.clone(),
            completed: false,
        }
    }

    pub fn pending(&self) -> Vec<PendingOperation> {
        self.state
            .lock()
            .unwrap()
            .pending
            .iter()
            .map(|(id, operation)| PendingOperation {
                id: *id,
                subsystem: operation.subsystem,
                operation: operation.operation.clone(),
                source: operation.source.clone(),
                elapsed: operation.started.elapsed(),
            })
            .collect()
    }

    pub fn failures(&self) -> Vec<ReadinessFailure> {
        self.state.lock().unwrap().failures.clone()
    }

    pub fn has_pending(&self) -> bool {
        !self.state.lock().unwrap().pending.is_empty()
    }

    pub fn register_canvas(&self) {
        self.canvas_count.fetch_add(1, Ordering::AcqRel);
    }

    pub fn unregister_canvas(&self) {
        self.canvas_count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                Some(count.saturating_sub(1))
            })
            .ok();
    }

    pub fn canvas_count(&self) -> u32 {
        self.canvas_count.load(Ordering::Acquire)
    }

    pub fn mark_gpu_submission(&self) {
        self.gpu_submissions.fetch_add(1, Ordering::AcqRel);
    }

    pub fn mark_canvas_submission(&self, canvas_id: u64) {
        self.mark_gpu_submission();
        self.submitted_canvases.lock().unwrap().insert(canvas_id);
    }

    pub fn submitted_canvas_count(&self) -> usize {
        self.submitted_canvases.lock().unwrap().len()
    }

    pub fn gpu_submissions(&self) -> u64 {
        self.gpu_submissions.load(Ordering::Acquire)
    }

    pub fn renderer_ready(&self) -> bool {
        self.canvas_count() != 0
            && self.submitted_canvas_count() >= self.canvas_count() as usize
            && !self.has_pending()
            && self.failures().is_empty()
    }

    fn complete(&self, id: u64, error: Option<String>) {
        let mut state = self.state.lock().unwrap();
        let Some(operation) = state.pending.remove(&id) else {
            return;
        };
        if let Some(error) = error {
            state.failures.push(ReadinessFailure {
                subsystem: operation.subsystem,
                operation: operation.operation,
                source: operation.source,
                error,
            });
        }
    }
}

pub struct ReadinessToken {
    id: u64,
    registry: Arc<ReadinessRegistry>,
    completed: bool,
}

impl ReadinessToken {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn complete(mut self) {
        self.registry.complete(self.id, None);
        self.completed = true;
    }

    pub fn fail(mut self, error: impl Into<String>) {
        self.registry.complete(self.id, Some(error.into()));
        self.completed = true;
    }
}

impl Drop for ReadinessToken {
    fn drop(&mut self) {
        if !self.completed {
            self.registry.complete(
                self.id,
                Some("operation token dropped before completion".into()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_canvas_submission_and_no_pending_work() {
        let registry = ReadinessRegistry::new();
        let token = registry.begin(ReadinessSubsystem::Device, "requestDevice", None);
        registry.register_canvas();
        registry.mark_canvas_submission(1);
        assert!(!registry.renderer_ready());
        token.complete();
        assert!(registry.renderer_ready());
    }

    #[test]
    fn dropped_tokens_fail_loudly() {
        let registry = ReadinessRegistry::new();
        drop(registry.begin(
            ReadinessSubsystem::Resource,
            "load",
            Some("asset.bin".into()),
        ));
        assert_eq!(registry.failures().len(), 1);
        assert!(!registry.has_pending());
    }
}
