use bevy::prelude::*;

use super::*;
use crate::demos::multiplayer_boxes::rope::Highlighted;

fn rope_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        leafwing_input_manager::plugin::InputManagerPlugin::<crate::input::AfterglowAction>::default(),
    ));
    app.init_resource::<PlayerName>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(crate::network::AfterglowNetworkContext::from_status(
            crate::network::AfterglowConnectionStatus {
                role: crate::network::LightyearRole::Host,
                ..Default::default()
            },
        ));
    app
}

// ---------------------------------------------------------------------------
// Highlight tests
// ---------------------------------------------------------------------------

/// Nearest box within range gets Highlighted component.
#[test]
fn highlight_nearest_box_adds_highlight() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();

    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
    ));

    let box1 = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            Transform::from_xyz(1.0, 0.5, 0.0),
        ))
        .id();

    let box2 = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 1,
                initial_pos: Vec3::new(5.0, 0.5, 0.0),
            },
            Transform::from_xyz(5.0, 0.5, 0.0),
        ))
        .id();

    app.add_systems(Update, super::super::rope::highlight_nearest_box);
    app.update();

    assert!(
        app.world().get::<Highlighted>(box1).is_some(),
        "nearest box (box1) should be highlighted"
    );
    assert!(
        app.world().get::<Highlighted>(box2).is_none(),
        "farther box (box2) should not be highlighted"
    );
}

/// Box outside range is not highlighted.
#[test]
fn highlight_nearest_box_outside_range_not_highlighted() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();

    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
    ));

    let box_far = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(100.0, 0.5, 0.0),
            },
            Transform::from_xyz(100.0, 0.5, 0.0),
        ))
        .id();

    app.add_systems(Update, super::super::rope::highlight_nearest_box);
    app.update();

    assert!(
        app.world().get::<Highlighted>(box_far).is_none(),
        "far box should not be highlighted"
    );
}

/// Highlight selection is sticky so small physics jitter between similarly
/// distant boxes does not flicker the material every frame.
#[test]
fn highlight_nearest_box_uses_hysteresis() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();

    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
    ));

    let current = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            Transform::from_xyz(1.0, 0.5, 0.0),
            Highlighted,
        ))
        .id();
    let challenger = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 1,
                initial_pos: Vec3::new(0.85, 0.5, 0.0),
            },
            Transform::from_xyz(0.85, 0.5, 0.0),
        ))
        .id();

    app.add_systems(Update, super::super::rope::highlight_nearest_box);
    app.update();
    assert!(app.world().get::<Highlighted>(current).is_some());
    assert!(app.world().get::<Highlighted>(challenger).is_none());

    app.world_mut()
        .get_mut::<Transform>(challenger)
        .unwrap()
        .translation = Vec3::new(0.5, 0.5, 0.0);
    app.update();
    assert!(app.world().get::<Highlighted>(current).is_none());
    assert!(app.world().get::<Highlighted>(challenger).is_some());
}

/// Roped box is not highlighted (only un-roped boxes are highlighted).
#[test]
fn highlight_nearest_box_skips_roped() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();

    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
    ));

    let box_roped = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            Transform::from_xyz(1.0, 0.5, 0.0),
            RopedTo {
                player_owner: "alice".to_string(),
            },
        ))
        .id();

    let box_free = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 1,
                initial_pos: Vec3::new(2.0, 0.5, 0.0),
            },
            Transform::from_xyz(2.0, 0.5, 0.0),
        ))
        .id();

    app.add_systems(Update, super::super::rope::highlight_nearest_box);
    app.update();

    assert!(
        app.world().get::<Highlighted>(box_roped).is_none(),
        "roped box should not be highlighted"
    );
    assert!(
        app.world().get::<Highlighted>(box_free).is_some(),
        "nearest free box should be highlighted"
    );
}

/// Highlight clears when no boxes are in range.
#[test]
fn highlight_clears_when_no_boxes_in_range() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();

    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
    ));

    let box_far = app
        .world_mut()
        .spawn((
            KinematicBox {
                id: 0,
                initial_pos: Vec3::new(100.0, 0.5, 0.0),
            },
            Transform::from_xyz(100.0, 0.5, 0.0),
        ))
        .id();

    app.add_systems(Update, super::super::rope::highlight_nearest_box);
    app.update();

    assert!(
        app.world().get::<Highlighted>(box_far).is_none(),
        "far box should not be highlighted"
    );
}
