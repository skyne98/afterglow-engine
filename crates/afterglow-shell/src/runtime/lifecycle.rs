use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum RuntimePhase {
    Created = 0,
    EnvironmentReady = 1,
    AdapterReady = 2,
    DeviceReady = 3,
    CanvasReady = 4,
    ResourcesReady = 5,
    RendererReady = 6,
    Running = 7,
    Suspended = 8,
    DeviceLost = 9,
    Stopped = 10,
}

impl RuntimePhase {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Created,
            1 => Self::EnvironmentReady,
            2 => Self::AdapterReady,
            3 => Self::DeviceReady,
            4 => Self::CanvasReady,
            5 => Self::ResourcesReady,
            6 => Self::RendererReady,
            7 => Self::Running,
            8 => Self::Suspended,
            9 => Self::DeviceLost,
            10 => Self::Stopped,
            _ => unreachable!("invalid runtime phase {value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleError {
    pub from: RuntimePhase,
    pub to: RuntimePhase,
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "illegal runtime lifecycle transition {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl std::error::Error for LifecycleError {}

#[derive(Debug)]
pub struct RuntimeLifecycle {
    phase: AtomicU8,
}

impl Default for RuntimeLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeLifecycle {
    pub fn new() -> Self {
        Self {
            phase: AtomicU8::new(RuntimePhase::Created as u8),
        }
    }

    pub fn phase(&self) -> RuntimePhase {
        RuntimePhase::from_u8(self.phase.load(Ordering::Acquire))
    }

    pub fn transition(&self, to: RuntimePhase) -> Result<(), LifecycleError> {
        loop {
            let from = self.phase();
            if from == to {
                return Ok(());
            }
            if !valid_transition(from, to) {
                return Err(LifecycleError { from, to });
            }
            if self
                .phase
                .compare_exchange(from as u8, to as u8, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(());
            }
        }
    }
}

fn valid_transition(from: RuntimePhase, to: RuntimePhase) -> bool {
    use RuntimePhase::*;
    matches!(
        (from, to),
        (Created, EnvironmentReady)
            | (EnvironmentReady, AdapterReady)
            | (AdapterReady, DeviceReady)
            | (DeviceReady, CanvasReady)
            | (CanvasReady, ResourcesReady)
            | (ResourcesReady, RendererReady)
            | (RendererReady, Running)
            | (Running, Suspended)
            | (Suspended, Running)
            | (DeviceLost, AdapterReady)
    ) || (to == DeviceLost && !matches!(from, Created | Stopped))
        || (to == Stopped && from != Stopped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_requires_ordered_startup() {
        let lifecycle = RuntimeLifecycle::new();
        assert!(lifecycle.transition(RuntimePhase::DeviceReady).is_err());
        for phase in [
            RuntimePhase::EnvironmentReady,
            RuntimePhase::AdapterReady,
            RuntimePhase::DeviceReady,
            RuntimePhase::CanvasReady,
            RuntimePhase::ResourcesReady,
            RuntimePhase::RendererReady,
            RuntimePhase::Running,
        ] {
            lifecycle.transition(phase).unwrap();
        }
        assert_eq!(lifecycle.phase(), RuntimePhase::Running);
    }

    #[test]
    fn lifecycle_supports_suspend_and_device_recovery() {
        let lifecycle = RuntimeLifecycle::new();
        lifecycle
            .transition(RuntimePhase::EnvironmentReady)
            .unwrap();
        lifecycle.transition(RuntimePhase::AdapterReady).unwrap();
        lifecycle.transition(RuntimePhase::DeviceReady).unwrap();
        lifecycle.transition(RuntimePhase::DeviceLost).unwrap();
        lifecycle.transition(RuntimePhase::AdapterReady).unwrap();
        lifecycle.transition(RuntimePhase::Stopped).unwrap();
        assert_eq!(lifecycle.phase(), RuntimePhase::Stopped);
    }
}
