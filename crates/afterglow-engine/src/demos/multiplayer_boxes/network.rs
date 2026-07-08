use bevy::prelude::*;
use lightyear::prelude::*;

use super::protocol::*;

pub fn register_demo_protocol(app: &mut App) {
    // Demo-specific components. PlayerBox and KinematicBox are predicted to
    // all clients; player-facing presentation is rendered from Predicted
    // copies, not from parallel interpolated copies.
    app.register_component::<PlayerBox>().add_prediction();
    app.register_component::<KinematicBox>().add_prediction();
    app.register_component::<RopeLink>().add_prediction();

    // Engine-level components (StableEntityId, Transform, LinearVelocity)
    // are already registered by register_afterglow_lightyear_protocol called
    // from AfterglowLightyearPlugin before this demo plugin runs.
}
