use bevy::prelude::*;
use lightyear::prelude::*;

use super::protocol::*;

pub fn register_demo_protocol(app: &mut App) {
    app.register_component::<PlayerBox>();
    app.register_component::<KinematicBox>();
    app.register_component::<MoveInput>();

    // Register Transform for replication so server-side Avian physics positions
    // are sent to clients. The client-side PhysicsTransformPlugin syncs
    // Transform → Position/Rotation for Avian simulation.
    app.register_component::<Transform>();
}
