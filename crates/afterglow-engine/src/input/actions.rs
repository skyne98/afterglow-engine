use bevy::prelude::Reflect;
use leafwing_input_manager::Actionlike;
use serde::{Deserialize, Serialize};

#[derive(Actionlike, Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
pub enum AfterglowAction {
    #[actionlike(DualAxis)]
    Move,
    #[actionlike(DualAxis)]
    Look,
    Use,
    AttackPrimary,
    AttackSecondary,
    RaiseShield,
    Jump,
    Crouch,
    Sprint,
    Menu,
    DebugToggle,
}
