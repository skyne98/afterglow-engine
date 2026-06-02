use bevy::prelude::*;
use delta::TickHistory;
use serde::{Serialize, de::DeserializeOwned};
use std::marker::PhantomData;

// ── Snapshot trait ──────────────────────────────────────────────────────

/// A snapshot that can be extracted from the ECS world and applied back.
///
/// Implement this for your game's full state struct:
///
/// ```ignore
/// impl Snapshot for GameSnapshot {
///     fn extract(world: &mut World) -> Self { ... }
///     fn apply(&self, world: &mut World) { ... }
/// }
/// ```
pub trait Snapshot: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    fn extract(world: &mut World) -> Self;
    fn apply(&self, world: &mut World);
}

// ── RollbackHistory resource ───────────────────────────────────────────

/// A [`TickHistory`] exposed as a Bevy resource.
///
/// **Server**: call [`at`](Self::at) or [`at_or_latest`](Self::at_or_latest)
/// to retrieve past state for lag-compensation.
///
/// **Client**: the plugin stores snapshots every tick. Write your own system
/// to restore before Lightyear's prediction rollback:
///
/// ```ignore
/// fn restore_physics(world: &mut World) {
///     let history = world.resource::<RollbackHistory<GameSnapshot>>();
///     let target_tick = /* your rollback target */;
///     let snapshot = history.at(target_tick);
///     snapshot.apply(world);
/// }
/// ```
#[derive(Resource)]
pub struct RollbackHistory<T: Snapshot> {
    inner: TickHistory<T>,
    tick_counter: u32,
}

impl<T: Snapshot> Default for RollbackHistory<T> {
    fn default() -> Self {
        Self {
            inner: TickHistory::new(240),
            tick_counter: 0,
        }
    }
}

impl<T: Snapshot> RollbackHistory<T> {
    pub fn at(&self, tick: u32) -> T {
        self.inner.at(tick)
    }

    pub fn at_or_latest(&self, tick: u32) -> T {
        self.inner.at_or_latest(tick)
    }

    pub fn oldest_tick(&self) -> u32 {
        self.inner.oldest_tick()
    }

    pub fn latest_tick(&self) -> u32 {
        self.inner.latest_tick()
    }
}

// ── Plugin ──────────────────────────────────────────────────────────────

/// Stores a [`Snapshot`] from the ECS into [`RollbackHistory`] every tick.
///
/// ```ignore
/// app.add_plugins(RollbackPlugin::<GameSnapshot>::new(240));
/// ```
pub struct RollbackPlugin<T: Snapshot> {
    capacity: usize,
    _marker: PhantomData<T>,
}

impl<T: Snapshot> RollbackPlugin<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            _marker: PhantomData,
        }
    }
}

impl<T: Snapshot> Plugin for RollbackPlugin<T> {
    fn build(&self, app: &mut App) {
        app.init_resource::<RollbackHistory<T>>();
        let mut history = app.world_mut().resource_mut::<RollbackHistory<T>>();
        history.inner = TickHistory::new(self.capacity);
        app.add_systems(FixedUpdate, store_snapshot::<T>);
    }
}

fn store_snapshot<T: Snapshot>(world: &mut World) {
    let snapshot = T::extract(world);
    let mut history = world.resource_mut::<RollbackHistory<T>>();
    let tick = history.tick_counter;
    history.inner.push(&snapshot, tick);
    history.tick_counter += 1;
}
