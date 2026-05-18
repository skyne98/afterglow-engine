mod diff;
pub mod harness;
pub mod lightyear;
pub mod model;
mod net;
mod world;

pub use harness::NetworkedRpg;
pub use lightyear::LightyearNetworkedRpg;
pub use model::{
    ALICE, BOB, CAROL, CombatFact, Combatant, Corpse, Correction, DOMAIN, Inventory, Loot,
    Projectile, RejectedInput, RpgAction, attack, move_to, pick_up_food, raise_shield,
};
