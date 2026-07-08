use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::{action_state::ActionState, input_map::InputMap};
use lightyear::prelude::*;

use super::protocol::*;

use crate::{
    input::{AfterglowAction, default_gameplay_input_map},
    network::connection::LocalPlayerId,
};

#[derive(Resource, Default)]
pub struct DemoInput(pub Vec2);

#[doc(hidden)]
#[derive(Default)]
pub struct RopeInputEdgeMemory {
    was_pressed: bool,
    pending_release: bool,
}

pub fn collect_input(
    keyboard: Option<Res<ButtonInput<KeyCode>>>,
    mut input: ResMut<DemoInput>,
    local_player_id: Option<Res<LocalPlayerId>>,
    mut maps: Query<
        (&PlayerBox, &mut ActionState<AfterglowAction>),
        (With<InputMap<AfterglowAction>>, With<Predicted>),
    >,
    rollback: Query<(), With<Rollback>>,
    mut rope_edges: Local<RopeInputEdgeMemory>,
) {
    let keyboard = keyboard.as_deref();
    let rope_pressed = keyboard.is_some_and(|keyboard| keyboard.pressed(KeyCode::KeyF));
    let physical_rope_released = rope_edges.was_pressed && !rope_pressed;

    // Do not write to ActionState during rollback replay, but do not lose a
    // physical release edge that happened during the guarded window.
    if rollback.iter().next().is_some() {
        if physical_rope_released {
            rope_edges.pending_release = true;
        }
        rope_edges.was_pressed = rope_pressed;
        return;
    }

    let dir = keyboard.map(keyboard_direction).unwrap_or(input.0);
    let rope_just_released = rope_edges.pending_release || physical_rope_released;
    rope_edges.pending_release = false;
    rope_edges.was_pressed = rope_pressed;
    input.0 = dir;
    let Some(local_owner) = local_player_id.as_deref().map(|id| id.0.to_string()) else {
        return;
    };
    write_local_input_actions(
        dir,
        rope_pressed,
        rope_just_released,
        &local_owner,
        &mut maps,
    );
}

fn write_local_input_actions(
    dir: Vec2,
    rope_pressed: bool,
    rope_just_released: bool,
    local_owner: &str,
    maps: &mut Query<
        (&PlayerBox, &mut ActionState<AfterglowAction>),
        (With<InputMap<AfterglowAction>>, With<Predicted>),
    >,
) {
    for (player, mut action_state) in maps.iter_mut() {
        if player.owner != local_owner {
            continue;
        }
        action_state.set_axis_pair(&AfterglowAction::Move, Vec2::new(-dir.x, dir.y));
        if rope_just_released {
            // Lightyear rollback/input restoration can leave the local
            // `ActionState` no longer marked pressed by the time the physical
            // release is sampled. Force the button edge from fixed input state
            // so release-driven gameplay (rope detach) remains deterministic.
            if !action_state.pressed(&AfterglowAction::RopeToggle) {
                action_state.press(&AfterglowAction::RopeToggle);
            }
            action_state.release(&AfterglowAction::RopeToggle);
        } else if rope_pressed {
            if !action_state.pressed(&AfterglowAction::RopeToggle) {
                action_state.press(&AfterglowAction::RopeToggle);
            }
        } else if action_state.pressed(&AfterglowAction::RopeToggle) {
            action_state.release(&AfterglowAction::RopeToggle);
        }
    }
}

pub fn add_input_map_to_local_predicted_player(
    mut commands: Commands,
    local_player_id: Option<Res<LocalPlayerId>>,
    players: Query<(Entity, &PlayerBox, Has<InputMap<AfterglowAction>>), With<Predicted>>,
) {
    let local_str = local_player_id.as_deref().map(|id| id.0.to_string());
    for (entity, player, has_map) in &players {
        if has_map || local_str.as_deref() != Some(player.owner.as_str()) {
            continue;
        }
        commands.entity(entity).insert(default_gameplay_input_map());
    }
}

pub fn apply_movement(
    time: Res<Time>,
    mut players: Query<
        (
            Option<&mut LinearVelocity>,
            Option<&mut Transform>,
            Option<&ActionState<AfterglowAction>>,
        ),
        With<PlayerBox>,
    >,
) {
    for (linear_vel, transform, action_state) in players.iter_mut() {
        apply_movement_to_player(&time, linear_vel, transform, action_state);
    }
}

/// Client-side movement only drives predicted presentation/simulation copies.
/// Interpolated remote copies are driven by replication interpolation, not
/// local input systems.
pub fn apply_predicted_movement(
    time: Res<Time>,
    mut players: Query<
        (
            Option<&mut LinearVelocity>,
            Option<&mut Transform>,
            Option<&ActionState<AfterglowAction>>,
        ),
        (With<PlayerBox>, With<Predicted>),
    >,
) {
    for (linear_vel, transform, action_state) in players.iter_mut() {
        apply_movement_to_player(&time, linear_vel, transform, action_state);
    }
}

fn apply_movement_to_player(
    time: &Time,
    linear_vel: Option<Mut<LinearVelocity>>,
    mut transform: Option<Mut<Transform>>,
    action_state: Option<&ActionState<AfterglowAction>>,
) {
    let dir = movement_direction(action_state, None);
    let vel = Vec3::new(dir.x, 0.0, dir.y) * PLAYER_SPEED;
    if let Some(mut linear_vel) = linear_vel {
        linear_vel.0 = vel;
    } else if let Some(ref mut t) = transform {
        t.translation += vel * time.delta_secs();
    }
}

fn movement_direction(
    action_state: Option<&ActionState<AfterglowAction>>,
    _fallback: Option<Vec2>,
) -> Vec2 {
    action_state
        .map(|state| {
            let axis = state.clamped_axis_pair(&AfterglowAction::Move);
            Vec2::new(-axis.x, axis.y).clamp_length_max(1.0)
        })
        .unwrap_or_default()
}

fn keyboard_direction(keyboard: &ButtonInput<KeyCode>) -> Vec2 {
    let mut dir = Vec2::ZERO;
    if keyboard.pressed(KeyCode::KeyW) || keyboard.pressed(KeyCode::ArrowUp) {
        dir.y += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyS) || keyboard.pressed(KeyCode::ArrowDown) {
        dir.y -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyA) || keyboard.pressed(KeyCode::ArrowLeft) {
        dir.x += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyD) || keyboard.pressed(KeyCode::ArrowRight) {
        dir.x -= 1.0;
    }
    dir.clamp_length_max(1.0)
}
