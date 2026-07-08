use avian3d::schedule::PhysicsSystems;
use bevy::prelude::*;
use lightyear::{
    frame_interpolation::FrameInterpolationSystems,
    prelude::{ReplicationSystems, RollbackSystems, client::input::InputSystems},
};

use super::{
    camera::{follow_camera_system, setup_camera},
    movement::{add_input_map_to_local_predicted_player, apply_predicted_movement, collect_input},
    rope::{
        LocallyReleasedRopes, draw_ropes, hide_local_rope_on_physical_release,
        highlight_nearest_box, on_rope_link_removed, suppress_locally_released_rope_reappearances,
        sync_rope_joints, toggle_rope, update_highlight_colors,
    },
    scene::{
        attach_predicted_kinematic_physics, attach_predicted_player_physics,
        attach_replicated_kinematic_visuals, attach_replicated_player_visuals,
        spawn_client_arena_visuals, spawn_lights, sync_kinematic_box_materials,
    },
};

/// Client-side plugin for the multiplayer boxes demo.
///
/// Sets up visuals, camera, input collection, and client-side prediction.
pub struct MultiplayerBoxesClientPlugin;

impl Plugin for MultiplayerBoxesClientPlugin {
    fn build(&self, app: &mut App) {
        super::network::register_demo_protocol(app);

        app.init_resource::<super::movement::DemoInput>();
        app.init_resource::<LocallyReleasedRopes>();

        app.add_systems(Startup, (spawn_client_arena_visuals, spawn_lights));
        app.add_systems(
            PreUpdate,
            (
                attach_predicted_player_physics,
                attach_predicted_kinematic_physics,
                add_input_map_to_local_predicted_player,
                suppress_locally_released_rope_reappearances,
            )
                .after(ReplicationSystems::Receive),
        );
        app.add_systems(
            Update,
            (
                attach_replicated_player_visuals,
                attach_replicated_kinematic_visuals,
                setup_camera,
            ),
        );
        app.add_systems(
            FixedUpdate,
            (
                apply_predicted_movement,
                toggle_rope,
                hide_local_rope_on_physical_release,
                sync_rope_joints,
            )
                .chain()
                .before(PhysicsSystems::Prepare),
        );
        app.add_systems(
            PostUpdate,
            follow_camera_system
                .after(FrameInterpolationSystems::Interpolate)
                .after(RollbackSystems::VisualCorrection),
        );
        app.add_systems(
            Update,
            (
                highlight_nearest_box,
                sync_kinematic_box_materials,
                update_highlight_colors,
                draw_ropes,
            )
                .chain(),
        );
        app.add_systems(
            FixedPreUpdate,
            collect_input.in_set(InputSystems::WriteClientInputs),
        );
        app.add_observer(on_rope_link_removed);
    }
}
