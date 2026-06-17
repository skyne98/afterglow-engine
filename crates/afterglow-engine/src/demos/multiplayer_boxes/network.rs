use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

use super::protocol::*;
use crate::input::AfterglowAction;

pub fn register_demo_protocol(app: &mut App) {
    app.register_component::<PlayerBox>();
    app.register_component::<KinematicBox>();
    app.register_component::<LinearVelocity>().add_prediction();

    // Register Transform as the single networked pose representation for this
    // Avian 0.6 demo. Lightyear 0.26's official lightyear_avian3d bridge targets
    // Avian 0.5, so we must not mix predicted Avian Position/Rotation with a
    // predicted Transform here. Avian Position/Rotation remain local physics
    // internals and are initialized before fixed simulation on predicted copies.
    app.register_component::<Transform>()
        .add_prediction()
        .add_linear_correction_fn::<Isometry3d>();
    app.world_mut()
        .resource_mut::<InterpolationRegistry>()
        .set_interpolation::<Transform>(TransformLinearInterpolation::lerp);

    app.register_component::<ActionState<AfterglowAction>>();

    // Enable frame interpolation for Transform so movement is visually smooth
    // between fixed ticks at any frame rate. Entities must also receive the
    // `FrameInterpolate<Transform>` component (added in scene.rs).
    app.add_plugins(lightyear::frame_interpolation::FrameInterpolationPlugin::<
        Transform,
    >::default());
}
