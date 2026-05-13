use crate::core::identity::StableEntityId;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

pub mod events;
pub use events::*;

#[derive(Resource, Clone, Debug, Eq, PartialEq, Reflect)]
pub struct DeterministicRollbackBuffer {
    max_saved_ticks: u32,
    states: BTreeMap<u32, Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RollbackCommand {
    pub tick: u32,
    pub source: u64,
    pub sequence: u64,
    pub payload: Vec<u8>,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Reflect,
    Serialize,
    Deserialize,
)]
/// Stable identifier for an authoritative rollback domain.
pub struct RollbackDomainId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct RollbackReplay {
    pub from_tick: u32,
    pub to_tick: u32,
    pub initial_state: Vec<u8>,
    pub commands: Vec<RollbackCommand>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RollbackPolicy {
    pub max_rollback_ticks: u32,
    pub commit_delay_ticks: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum RollbackCommandDecision {
    Replay,
    TooOld,
    FromFuture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum RollbackReplayError {
    TooOld,
    FromFuture,
    MissingState,
    AlreadyCommitted,
    DuplicateCommand,
}

impl Default for DeterministicRollbackBuffer {
    fn default() -> Self {
        Self {
            max_saved_ticks: 120,
            states: BTreeMap::new(),
        }
    }
}

impl Default for RollbackPolicy {
    fn default() -> Self {
        Self {
            max_rollback_ticks: 12,
            commit_delay_ticks: 4,
        }
    }
}

impl RollbackPolicy {
    pub fn classify_command(
        &self,
        current_tick: u32,
        command_tick: u32,
    ) -> RollbackCommandDecision {
        if command_tick > current_tick {
            return RollbackCommandDecision::FromFuture;
        }
        if current_tick - command_tick > self.max_rollback_ticks {
            RollbackCommandDecision::TooOld
        } else {
            RollbackCommandDecision::Replay
        }
    }

    pub fn event_is_final(&self, current_tick: u32, event_tick: u32) -> bool {
        current_tick >= event_tick.saturating_add(self.commit_delay_ticks)
    }

    pub fn pending_until(&self, event_tick: u32) -> u32 {
        event_tick.saturating_add(self.commit_delay_ticks)
    }

    pub fn committed_tick(&self, current_tick: u32) -> u32 {
        current_tick.saturating_sub(self.commit_delay_ticks)
    }

    pub fn tick_is_provisional(&self, current_tick: u32, tick: u32) -> bool {
        tick > self.committed_tick(current_tick) && tick <= current_tick
    }
}

impl DeterministicRollbackBuffer {
    pub fn with_capacity_ticks(mut self, max_saved_ticks: u32) -> Self {
        self.max_saved_ticks = max_saved_ticks;
        self
    }

    pub fn save_state(&mut self, tick: u32, state: impl Into<Vec<u8>>) {
        self.states.insert(tick, state.into());
        self.prune_older_than(tick.saturating_sub(self.max_saved_ticks));
    }

    pub fn state(&self, tick: u32) -> Option<&[u8]> {
        self.states.get(&tick).map(Vec::as_slice)
    }

    pub fn build_replay(
        &self,
        from_tick: u32,
        to_tick: u32,
        commands: impl IntoIterator<Item = RollbackCommand>,
    ) -> Result<RollbackReplay, RollbackReplayError> {
        let initial_state = self
            .states
            .get(&from_tick)
            .cloned()
            .ok_or(RollbackReplayError::MissingState)?;
        let commands = commands
            .into_iter()
            .filter(|command| command.tick > from_tick && command.tick <= to_tick)
            .collect::<Vec<_>>();
        let commands = canonical_commands(commands)?;
        Ok(RollbackReplay {
            from_tick,
            to_tick,
            initial_state,
            commands,
        })
    }

    pub fn build_late_command_replay(
        &self,
        policy: RollbackPolicy,
        current_tick: u32,
        command_tick: u32,
        commands: impl IntoIterator<Item = RollbackCommand>,
    ) -> Result<RollbackReplay, RollbackReplayError> {
        if command_tick == 0 {
            return Err(RollbackReplayError::MissingState);
        }
        match policy.classify_command(current_tick, command_tick) {
            RollbackCommandDecision::Replay => {}
            RollbackCommandDecision::TooOld => return Err(RollbackReplayError::TooOld),
            RollbackCommandDecision::FromFuture => return Err(RollbackReplayError::FromFuture),
        }
        let from_tick = command_tick.saturating_sub(1);
        self.build_replay(from_tick, current_tick, commands)
    }

    pub fn prune_older_than(&mut self, tick: u32) {
        while let Some((&oldest, _)) = self.states.first_key_value() {
            if oldest >= tick {
                break;
            }
            self.states.remove(&oldest);
        }
    }

    pub fn clear(&mut self) {
        self.states.clear();
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

pub fn replay_bytes(
    replay: &RollbackReplay,
    mut apply: impl FnMut(&mut Vec<u8>, &RollbackCommand),
) -> Vec<u8> {
    let mut state = replay.initial_state.clone();
    for command in &replay.commands {
        apply(&mut state, command);
    }
    state
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Reflect, Serialize, Deserialize)]
pub struct RollbackCue {
    pub tick: u32,
    pub sequence: u64,
    pub kind: String,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Reflect)]
/// Difference between the previous provisional replay's cues and the corrected
/// replay's cues.
pub struct RollbackCueDiff {
    pub added: Vec<RollbackCue>,
    pub removed: Vec<RollbackCue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
/// Stable entity lifetime record generated by deterministic replay.
///
/// Provisional spawns/despawns remain reversible until their ticks are
/// committed.
pub struct RollbackEntityLifecycle {
    pub entity: StableEntityId,
    pub spawn_tick: u32,
    pub despawn_tick: Option<u32>,
    pub despawn_reason: Option<String>,
}

impl RollbackEntityLifecycle {
    pub fn spawned(entity: StableEntityId, spawn_tick: u32) -> Self {
        Self {
            entity,
            spawn_tick,
            despawn_tick: None,
            despawn_reason: None,
        }
    }

    pub fn mark_despawned(&mut self, tick: u32, reason: impl Into<String>) {
        self.despawn_tick = Some(tick);
        self.despawn_reason = Some(reason.into());
    }

    pub fn is_alive_at(&self, tick: u32) -> bool {
        self.spawn_tick <= tick && self.despawn_tick.is_none_or(|despawn| tick < despawn)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Reflect)]
/// Side outputs generated while replaying a rollback domain.
pub struct RollbackDomainOutputs {
    pub cues: Vec<RollbackCue>,
    pub lifecycles: BTreeMap<StableEntityId, RollbackEntityLifecycle>,
}

impl RollbackDomainOutputs {
    pub fn cue(
        &mut self,
        tick: u32,
        sequence: u64,
        kind: impl Into<String>,
        payload: impl Into<Vec<u8>>,
    ) {
        self.cues.push(RollbackCue {
            tick,
            sequence,
            kind: kind.into(),
            payload: payload.into(),
        });
    }

    pub fn spawn_entity(&mut self, entity: StableEntityId, tick: u32) {
        self.lifecycles
            .insert(entity, RollbackEntityLifecycle::spawned(entity, tick));
    }

    pub fn despawn_entity(&mut self, entity: StableEntityId, tick: u32, reason: impl Into<String>) {
        self.lifecycles
            .entry(entity)
            .or_insert_with(|| RollbackEntityLifecycle::spawned(entity, tick))
            .mark_despawned(tick, reason);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
/// Result of rebuilding a domain's live provisional state from its committed
/// anchor.
pub struct RollbackDomainReplay {
    pub committed_tick: u32,
    pub current_tick: u32,
    pub previous_provisional_state: Vec<u8>,
    pub provisional_state: Vec<u8>,
    pub cue_diff: RollbackCueDiff,
    pub outputs: RollbackDomainOutputs,
}

#[derive(Resource, Clone, Debug, Eq, PartialEq, Reflect)]
/// Authoritative committed/provisional rollback domain.
///
/// `committed_state` is the final durable anchor. `provisional_state` is the
/// live gameplay truth rebuilt by replaying accepted commands after
/// `committed_tick`.
pub struct CommittedRollbackDomain {
    pub id: RollbackDomainId,
    pub policy: RollbackPolicy,
    committed_tick: u32,
    current_tick: u32,
    committed_state: Vec<u8>,
    provisional_state: Vec<u8>,
    commands: Vec<RollbackCommand>,
    provisional_outputs: RollbackDomainOutputs,
}

impl CommittedRollbackDomain {
    pub fn new(
        id: RollbackDomainId,
        committed_tick: u32,
        committed_state: impl Into<Vec<u8>>,
        policy: RollbackPolicy,
    ) -> Self {
        let committed_state = committed_state.into();
        Self {
            id,
            policy,
            committed_tick,
            current_tick: committed_tick,
            provisional_state: committed_state.clone(),
            committed_state,
            commands: Vec::new(),
            provisional_outputs: RollbackDomainOutputs::default(),
        }
    }

    pub fn committed_tick(&self) -> u32 {
        self.committed_tick
    }

    pub fn current_tick(&self) -> u32 {
        self.current_tick
    }

    pub fn committed_state(&self) -> &[u8] {
        &self.committed_state
    }

    pub fn provisional_state(&self) -> &[u8] {
        &self.provisional_state
    }

    pub fn commands(&self) -> &[RollbackCommand] {
        &self.commands
    }

    pub fn provisional_outputs(&self) -> &RollbackDomainOutputs {
        &self.provisional_outputs
    }

    pub fn insert_command(
        &mut self,
        current_tick: u32,
        command: RollbackCommand,
    ) -> Result<(), RollbackReplayError> {
        if command.tick <= self.committed_tick {
            return Err(RollbackReplayError::AlreadyCommitted);
        }
        if self
            .commands
            .iter()
            .any(|existing| command_key(existing) == command_key(&command))
        {
            return Err(RollbackReplayError::DuplicateCommand);
        }
        match self.policy.classify_command(current_tick, command.tick) {
            RollbackCommandDecision::Replay => {
                self.current_tick = self.current_tick.max(current_tick);
                self.commands.push(command);
                self.commands.sort_by_key(command_key);
                Ok(())
            }
            RollbackCommandDecision::TooOld => Err(RollbackReplayError::TooOld),
            RollbackCommandDecision::FromFuture => Err(RollbackReplayError::FromFuture),
        }
    }

    /// Rebuilds live gameplay truth from the committed anchor.
    ///
    /// Gameplay should query the rebuilt provisional state for movement, hit
    /// detection, command legality, projectiles, and other live outcomes.
    /// The committed state is only the durable rollback anchor; using it
    /// for live combat would make every interaction operate on old
    /// positions.
    pub fn rebuild_provisional(
        &mut self,
        current_tick: u32,
        mut apply: impl FnMut(&mut Vec<u8>, &RollbackCommand, &mut RollbackDomainOutputs),
    ) -> RollbackDomainReplay {
        self.current_tick = current_tick;
        let previous_provisional_state = self.provisional_state.clone();
        let previous_cues = self.provisional_outputs.cues.clone();
        let mut state = self.committed_state.clone();
        let mut outputs = RollbackDomainOutputs::default();

        for command in self.replay_commands(self.committed_tick, current_tick) {
            apply(&mut state, command, &mut outputs);
        }

        let cue_diff = cue_diff(&previous_cues, &outputs.cues);
        self.provisional_state = state.clone();
        self.provisional_outputs = outputs.clone();

        RollbackDomainReplay {
            committed_tick: self.committed_tick,
            current_tick,
            previous_provisional_state,
            provisional_state: state,
            cue_diff,
            outputs,
        }
    }

    pub fn promote_committed(
        &mut self,
        current_tick: u32,
        mut apply: impl FnMut(&mut Vec<u8>, &RollbackCommand, &mut RollbackDomainOutputs),
    ) -> RollbackDomainReplay {
        let new_committed_tick = self.policy.committed_tick(current_tick);
        if new_committed_tick > self.committed_tick {
            let mut committed = self.committed_state.clone();
            let mut ignored_outputs = RollbackDomainOutputs::default();
            for command in self.replay_commands(self.committed_tick, new_committed_tick) {
                apply(&mut committed, command, &mut ignored_outputs);
            }
            self.committed_state = committed;
            self.committed_tick = new_committed_tick;
            self.commands
                .retain(|command| command.tick > self.committed_tick);
        }
        self.rebuild_provisional(current_tick, apply)
    }

    fn replay_commands(
        &self,
        from_tick: u32,
        to_tick: u32,
    ) -> impl Iterator<Item = &RollbackCommand> {
        self.commands
            .iter()
            .filter(move |command| command.tick > from_tick && command.tick <= to_tick)
    }
}

fn canonical_commands(
    commands: Vec<RollbackCommand>,
) -> Result<Vec<RollbackCommand>, RollbackReplayError> {
    let mut canonical = BTreeMap::new();
    for command in commands {
        match canonical.entry(command_key(&command)) {
            Entry::Vacant(entry) => {
                entry.insert(command);
            }
            Entry::Occupied(entry) if entry.get().payload == command.payload => {}
            Entry::Occupied(_) => return Err(RollbackReplayError::DuplicateCommand),
        }
    }
    Ok(canonical.into_values().collect())
}

fn command_key(command: &RollbackCommand) -> (u32, u64, u64) {
    (command.tick, command.source, command.sequence)
}

fn cue_diff(previous: &[RollbackCue], next: &[RollbackCue]) -> RollbackCueDiff {
    let previous_set = previous.iter().collect::<BTreeSet<_>>();
    let next_set = next.iter().collect::<BTreeSet<_>>();
    RollbackCueDiff {
        added: next_set
            .difference(&previous_set)
            .map(|cue| (*cue).clone())
            .collect(),
        removed: previous_set
            .difference(&next_set)
            .map(|cue| (*cue).clone())
            .collect(),
    }
}

#[cfg(test)]
mod tests;
