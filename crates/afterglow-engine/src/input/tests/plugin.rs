use super::*;
use crate::{core::AfterglowCorePlugin, testing::unit_app};
use bevy::{ecs::message::Messages, input::mouse::MouseMotion};

#[test]
fn input_plugin_enqueues_one_command_per_local_player() {
    let mut app = unit_app();
    app.add_plugins(AfterglowInputPlugin);
    app.world_mut()
        .resource_mut::<LocalPlayers>()
        .add_player(NetworkPlayerId(2));

    app.world_mut()
        .resource_mut::<PlayerInputBindings>()
        .context_mut(InputContext::DEFAULT_GAMEPLAY)
        .add_key_axis("move.y", KeyCode::KeyS, KeyCode::KeyW);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);

    app.update();

    let queue = app.world().resource::<PlayerCommandQueue>();
    assert_eq!(queue.commands().len(), 2);
    assert_eq!(queue.commands()[0].player, NetworkPlayerId(1));
    assert_eq!(queue.commands()[1].player, NetworkPlayerId(2));
    assert_eq!(queue.commands()[0].tick, 0);
    assert_eq!(queue.commands()[0].axis("move.y"), 1.0);

    app.update();

    let queue = app.world().resource::<PlayerCommandQueue>();
    assert_eq!(queue.commands().len(), 2);
    assert_eq!(queue.commands()[0].tick, 1);
}

#[test]
fn input_plugin_routes_devices_per_local_player() {
    let mut app = unit_app();
    app.add_plugins(AfterglowInputPlugin);
    app.world_mut()
        .resource_mut::<LocalPlayers>()
        .add_player(NetworkPlayerId(2));
    app.world_mut()
        .resource_mut::<LocalInputRoutes>()
        .set_player_devices(NetworkPlayerId(1), [InputDeviceRoute::KeyboardMouse]);
    app.world_mut()
        .resource_mut::<LocalInputRoutes>()
        .set_player_devices(NetworkPlayerId(2), [InputDeviceRoute::Virtual]);
    app.world_mut()
        .resource_mut::<PlayerInputBindings>()
        .context_mut(InputContext::DEFAULT_GAMEPLAY)
        .add_key_axis("move.x", KeyCode::KeyA, KeyCode::KeyD);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyD);
    app.world_mut()
        .resource_mut::<VirtualInputState>()
        .set_axis("move.x", -0.25);

    app.update();

    let queue = app.world().resource::<PlayerCommandQueue>();
    assert_eq!(queue.commands()[0].player, NetworkPlayerId(1));
    assert_eq!(queue.commands()[0].axis("move.x"), 1.0);
    assert_eq!(queue.commands()[1].player, NetworkPlayerId(2));
    assert_eq!(queue.commands()[1].axis("move.x"), -0.25);
}

#[test]
fn input_plugin_supports_scripted_control_for_one_player() {
    let mut app = unit_app();
    app.add_plugins(AfterglowInputPlugin);
    app.world_mut()
        .resource_mut::<LocalPlayers>()
        .add_player(NetworkPlayerId(2));
    app.world_mut()
        .resource_mut::<LocalInputRoutes>()
        .set_player_devices(NetworkPlayerId(1), [InputDeviceRoute::Virtual]);
    app.world_mut()
        .resource_mut::<LocalInputRoutes>()
        .set_player_devices(NetworkPlayerId(2), [InputDeviceRoute::Virtual]);
    app.world_mut()
        .resource_mut::<VirtualInputState>()
        .set_player_axis(NetworkPlayerId(2), "move.x", 1.0);
    app.world_mut()
        .resource_mut::<VirtualInputState>()
        .press_player_action(NetworkPlayerId(2), "cutscene.interact");

    app.update();

    let queue = app.world().resource::<PlayerCommandQueue>();
    assert_eq!(queue.commands()[0].player, NetworkPlayerId(1));
    assert!(queue.commands()[0].axes.is_empty());
    assert!(queue.commands()[0].actions.is_empty());
    assert_eq!(queue.commands()[1].player, NetworkPlayerId(2));
    assert_eq!(queue.commands()[1].axis("move.x"), 1.0);
    assert!(queue.commands()[1].action_pressed("cutscene.interact"));
}

#[test]
fn input_plugin_collects_mouse_motion_messages() {
    let mut app = unit_app();
    app.add_plugins(AfterglowInputPlugin);
    app.world_mut()
        .resource_mut::<PlayerInputBindings>()
        .context_mut(InputContext::DEFAULT_GAMEPLAY)
        .add_mouse_motion_axis("look.x", AxisComponent::X, 0.1);
    app.world_mut()
        .resource_mut::<Messages<MouseMotion>>()
        .write(MouseMotion {
            delta: Vec2::new(30.0, 4.0),
        });

    app.update();

    let queue = app.world().resource::<PlayerCommandQueue>();
    assert_eq!(queue.commands()[0].axis("look.x"), 3.0);
}

#[test]
fn input_plugin_registers_with_core_plugin() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AfterglowCorePlugin, AfterglowInputPlugin));
    assert!(app.world().contains_resource::<PlayerCommandQueue>());
    assert!(app.world().contains_resource::<LocalInputRoutes>());
}
