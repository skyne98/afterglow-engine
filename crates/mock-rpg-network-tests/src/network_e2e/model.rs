use afterglow_engine::{
    core::identity::StableEntityId, input::AfterglowAction, network::RewindDomainId,
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::Vec3i;

pub const DOMAIN: RewindDomainId = RewindDomainId(1);
pub const ALICE: StableEntityId = StableEntityId::from_raw(1);
pub const BOB: StableEntityId = StableEntityId::from_raw(2);
pub const CAROL: StableEntityId = StableEntityId::from_raw(3);

#[derive(Component, Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct Combatant {
    pub hp: i32,
    pub shield_through: u32,
    pub position: Vec3i,
}

#[derive(Component, Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct Projectile {
    pub caster: StableEntityId,
    pub target: StableEntityId,
    pub impact_tick: u32,
    pub damage: i32,
    pub resolved: bool,
}

#[derive(Component, Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct DeathMarker {
    pub victim: StableEntityId,
}

#[derive(Component, Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct Corpse {
    pub victim: StableEntityId,
}

#[derive(Component, Clone, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct Loot {
    pub owner: StableEntityId,
    pub item: Item,
    pub picked_by: Option<StableEntityId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect, Serialize, Deserialize)]
pub enum Item {
    Food,
}

#[derive(Component, Clone, Debug, Default, Eq, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct Inventory {
    pub food: u32,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CombatLog {
    pub facts: Vec<CombatFact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CombatFact {
    MoveAccepted {
        tick: u32,
        player: StableEntityId,
    },
    MoveRejected {
        tick: u32,
        player: StableEntityId,
    },
    ShieldRaised {
        tick: u32,
        player: StableEntityId,
    },
    SpellCast {
        tick: u32,
        caster: StableEntityId,
        target: StableEntityId,
    },
    SpellRejectedOutOfRange {
        tick: u32,
        caster: StableEntityId,
        target: StableEntityId,
    },
    SpellBlocked {
        tick: u32,
        target: StableEntityId,
    },
    PlayerDied {
        tick: u32,
        player: StableEntityId,
    },
    FoodPickedUp {
        tick: u32,
        player: StableEntityId,
        from: StableEntityId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClientInput {
    pub player: StableEntityId,
    pub tick: u32,
    pub sequence: u64,
    pub action: RpgAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RpgAction {
    MoveTo(Vec3i),
    RaiseShield,
    AttackPrimary { target: StableEntityId, damage: i32 },
    PickUpFood { from: StableEntityId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Correction {
    ComponentChanged {
        stable_id: StableEntityId,
        component: &'static str,
    },
    DespawnEntity(StableEntityId),
    SpawnEntity(StableEntityId),
    AddFact(CombatFact),
    RemoveFact(CombatFact),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RejectedInput {
    Duplicate {
        player: StableEntityId,
        sequence: u64,
    },
    Stale {
        player: StableEntityId,
        sequence: u64,
        tick: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CombatSnapshot {
    pub combatants: Vec<(StableEntityId, Combatant)>,
    pub inventories: Vec<(StableEntityId, Inventory)>,
    pub projectiles: Vec<(StableEntityId, Projectile)>,
    pub death_markers: Vec<(StableEntityId, DeathMarker)>,
    pub corpses: Vec<(StableEntityId, Corpse)>,
    pub loot: Vec<(StableEntityId, Loot)>,
    pub log: CombatLog,
}

impl Combatant {
    pub fn new(hp: i32, position: Vec3i) -> Self {
        Self {
            hp,
            shield_through: 0,
            position,
        }
    }
}

impl RpgAction {
    pub fn afterglow_action(&self) -> AfterglowAction {
        match self {
            Self::MoveTo(_) => AfterglowAction::Move,
            Self::RaiseShield => AfterglowAction::RaiseShield,
            Self::AttackPrimary { .. } => AfterglowAction::AttackPrimary,
            Self::PickUpFood { .. } => AfterglowAction::Use,
        }
    }
}

pub fn move_to(sequence: u64, tick: u32, player: StableEntityId, target: Vec3i) -> ClientInput {
    ClientInput {
        player,
        tick,
        sequence,
        action: RpgAction::MoveTo(target),
    }
}

pub fn raise_shield(sequence: u64, tick: u32, player: StableEntityId) -> ClientInput {
    ClientInput {
        player,
        tick,
        sequence,
        action: RpgAction::RaiseShield,
    }
}

pub fn attack(
    sequence: u64,
    tick: u32,
    caster: StableEntityId,
    target: StableEntityId,
    damage: i32,
) -> ClientInput {
    ClientInput {
        player: caster,
        tick,
        sequence,
        action: RpgAction::AttackPrimary { target, damage },
    }
}

pub fn pick_up_food(
    sequence: u64,
    tick: u32,
    player: StableEntityId,
    from: StableEntityId,
) -> ClientInput {
    ClientInput {
        player,
        tick,
        sequence,
        action: RpgAction::PickUpFood { from },
    }
}
