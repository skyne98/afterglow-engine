use bevy::prelude::*;

use super::*;
use crate::{core::identity::StableEntityId, demos::multiplayer_boxes::rope::Highlighted};

fn test_box_id(index: u32) -> StableEntityId {
    StableEntityId::new(10_000 + u128::from(index))
}

fn test_rope_id() -> StableEntityId {
    StableEntityId::new(20_000)
}

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

fn spawn_player(app: &mut App) {
    app.world_mut().spawn((
        PlayerBox {
            owner: "alice".to_string(),
        },
        Transform::from_xyz(0.0, 0.4, 0.0),
    ));
}

fn spawn_box(app: &mut App, index: u32, pos: Vec3) -> Entity {
    app.world_mut()
        .spawn((
            KinematicBox { initial_pos: pos },
            test_box_id(index),
            Transform::from_translation(pos),
        ))
        .id()
}

#[test]
fn highlight_nearest_box_adds_highlight() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    spawn_player(&mut app);
    let box1 = spawn_box(&mut app, 0, Vec3::new(1.0, 0.5, 0.0));
    let box2 = spawn_box(&mut app, 1, Vec3::new(5.0, 0.5, 0.0));

    app.add_systems(Update, super::super::rope::highlight_nearest_box);
    app.update();

    assert!(app.world().get::<Highlighted>(box1).is_some());
    assert!(app.world().get::<Highlighted>(box2).is_none());
}

#[test]
fn highlight_nearest_box_outside_range_not_highlighted() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    spawn_player(&mut app);
    let box_far = spawn_box(&mut app, 0, Vec3::new(100.0, 0.5, 0.0));

    app.add_systems(Update, super::super::rope::highlight_nearest_box);
    app.update();

    assert!(app.world().get::<Highlighted>(box_far).is_none());
}

#[test]
fn highlight_nearest_box_uses_hysteresis() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    spawn_player(&mut app);
    let current = app
        .world_mut()
        .spawn((
            KinematicBox {
                initial_pos: Vec3::new(1.0, 0.5, 0.0),
            },
            test_box_id(0),
            Transform::from_xyz(1.0, 0.5, 0.0),
            Highlighted,
        ))
        .id();
    let challenger = spawn_box(&mut app, 1, Vec3::new(0.85, 0.5, 0.0));

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

#[test]
fn highlight_nearest_box_skips_linked() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    spawn_player(&mut app);
    let box_linked = spawn_box(&mut app, 0, Vec3::new(1.0, 0.5, 0.0));
    app.world_mut().spawn(RopeLink {
        rope_id: test_rope_id(),
        player_owner: "alice".to_string(),
        target: test_box_id(0),
    });
    let box_free = spawn_box(&mut app, 1, Vec3::new(2.0, 0.5, 0.0));

    app.add_systems(Update, super::super::rope::highlight_nearest_box);
    app.update();

    assert!(app.world().get::<Highlighted>(box_linked).is_none());
    assert!(app.world().get::<Highlighted>(box_free).is_some());
}

#[test]
fn highlight_clears_when_no_boxes_in_range() {
    let mut app = rope_test_app();
    app.world_mut().resource_mut::<PlayerName>().0 = "alice".to_string();
    spawn_player(&mut app);
    let box_far = spawn_box(&mut app, 0, Vec3::new(100.0, 0.5, 0.0));

    app.add_systems(Update, super::super::rope::highlight_nearest_box);
    app.update();

    assert!(app.world().get::<Highlighted>(box_far).is_none());
}
