use super::{
    ReplicationSet, RollbackReplicationClock,
    runtime::{
        ReplicationRuntimeRegistry, run_reissue_callbacks, run_restore_callbacks,
        run_save_callbacks,
    },
};
use crate::core::identity::maintain_stable_entity_registry;
use bevy::{ecs::schedule::ScheduleLabel, prelude::*};

#[derive(ScheduleLabel, Clone, Debug, Eq, Hash, PartialEq)]
pub struct ReplicatedTick;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum ReplicatedRollbackError {
    InvalidRange { anchor_tick: u32, through_tick: u32 },
    MissingSnapshot { tick: u32 },
}

pub trait ReplicatedRollbackWorldExt {
    fn save_replicated_state(&mut self, tick: u32);
    fn restore_replicated_state(&mut self, tick: u32) -> Result<(), ReplicatedRollbackError>;
    fn run_replicated_tick(&mut self, tick: u32);
    fn replay_replicated_ticks(
        &mut self,
        anchor_tick: u32,
        through_tick: u32,
    ) -> Result<(), ReplicatedRollbackError>;
}

impl ReplicatedRollbackWorldExt for World {
    fn save_replicated_state(&mut self, tick: u32) {
        maintain_stable_entity_registry(self);
        run_save_callbacks(self, tick);
    }

    fn restore_replicated_state(&mut self, tick: u32) -> Result<(), ReplicatedRollbackError> {
        if run_restore_callbacks(self, tick) {
            if let Some(mut clock) = self.get_resource_mut::<RollbackReplicationClock>() {
                clock.current_tick = tick;
            }
            Ok(())
        } else {
            Err(ReplicatedRollbackError::MissingSnapshot { tick })
        }
    }

    fn run_replicated_tick(&mut self, tick: u32) {
        if let Some(mut clock) = self.get_resource_mut::<RollbackReplicationClock>() {
            clock.current_tick = tick;
        }
        run_reissue_callbacks(self, tick);
        self.run_schedule(ReplicatedTick);
        self.save_replicated_state(tick);
    }

    fn replay_replicated_ticks(
        &mut self,
        anchor_tick: u32,
        through_tick: u32,
    ) -> Result<(), ReplicatedRollbackError> {
        if through_tick < anchor_tick {
            return Err(ReplicatedRollbackError::InvalidRange {
                anchor_tick,
                through_tick,
            });
        }
        self.restore_replicated_state(anchor_tick)?;
        for tick in anchor_tick.saturating_add(1)..=through_tick {
            self.run_replicated_tick(tick);
        }
        Ok(())
    }
}

pub fn configure_replication_sets(app: &mut App) {
    app.init_resource::<ReplicationRuntimeRegistry>()
        .init_schedule(ReplicatedTick)
        .configure_sets(
            Update,
            (
                ReplicationSet::RestoreState,
                ReplicationSet::ReissueMessages,
                ReplicationSet::CollectMessages,
                ReplicationSet::CollectChanges,
            )
                .chain()
                .in_set(crate::core::schedule::AfterglowSet::ApplyGameplay),
        );
}
