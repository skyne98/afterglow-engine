use bevy::prelude::*;
use lightyear::prelude::*;
use std::time::Duration;

use super::protocol::*;

pub fn register_demo_protocol(app: &mut App) {
    app.register_component::<PlayerBox>();
    app.register_component::<KinematicBox>();
    app.register_component::<MoveInput>();

    // Register Transform for replication, Lightyear prediction history, and
    // visual correction. The demo predicts client-owned player transforms and
    // interpolates remote/server-owned transforms.
    app.register_component::<Transform>()
        .add_prediction()
        .add_linear_correction_fn::<Isometry3d>();
    app.world_mut()
        .resource_mut::<InterpolationRegistry>()
        .set_interpolation::<Transform>(TransformLinearInterpolation::lerp);

    app.add_channel::<MoveInputChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: Duration::ZERO,
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<MoveInputMsg>()
        .add_direction(NetworkDirection::ClientToServer);
}
