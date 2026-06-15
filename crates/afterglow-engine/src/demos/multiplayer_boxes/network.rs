use bevy::prelude::*;
use lightyear::prelude::*;
use std::time::Duration;

use super::protocol::*;

pub fn register_demo_protocol(app: &mut App) {
    app.register_component::<PlayerBox>();
    app.register_component::<KinematicBox>();
    app.register_component::<MoveInput>();

    // Register Transform for replication so server-side Avian physics positions
    // are sent to clients. The client-side PhysicsTransformPlugin syncs
    // Transform → Position/Rotation for Avian simulation.
    app.register_component::<Transform>();

    app.add_channel::<MoveInputChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: Duration::ZERO,
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<MoveInputMsg>()
        .add_direction(NetworkDirection::ClientToServer);
}
