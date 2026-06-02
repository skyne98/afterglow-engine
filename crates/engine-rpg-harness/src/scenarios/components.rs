use afterglow_engine::core::identity::StableEntityId;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub current: i32,
    pub max: i32,
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CombatState {
    pub shield_active_until: u32,
    pub dead: bool,
    pub last_attack_tick: u32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Corpse {
    pub victim: StableEntityId,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Loot {
    pub owner: StableEntityId,
    pub picked_up: bool,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManaPool {
    pub current: i32,
    pub max: i32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BurnEffect {
    pub remaining_ticks: u32,
    pub damage_per_tick: i32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpawnPoint {
    pub position: Vec3,
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeadTimer {
    pub remaining: u32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoorState {
    pub open: bool,
    pub locked: bool,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team(pub u32);

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Enemy {
    pub attack_range: f32,
    pub damage: i32,
    pub detection_range: f32,
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Boss {
    pub phase: u32,
    pub max_phases: u32,
    pub phase_hp_thresholds: Vec<i32>,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoorGrab {
    pub player: StableEntityId,
    pub door: StableEntityId,
}
