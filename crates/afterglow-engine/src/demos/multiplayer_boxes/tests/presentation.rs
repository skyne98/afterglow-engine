use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;

use super::test_app;
use crate::demos::multiplayer_boxes::scene::{
    LocalPlayerPresentation, PlayerVisual, smooth_local_player_visuals,
};

#[test]
fn smooth_local_player_visuals_queries_are_disjoint() {
    let mut app = test_app();
    app.add_systems(Startup, |mut commands: Commands| {
        commands
            .spawn((
                Transform::from_xyz(1.0, 0.0, 0.0),
                LocalPlayerPresentation {
                    visual_translation: Vec3::ZERO,
                },
                LinearVelocity(Vec3::X),
            ))
            .with_children(|children| {
                children.spawn((PlayerVisual, Transform::default()));
            });
    });
    app.add_systems(Update, smooth_local_player_visuals);

    app.update();
}
