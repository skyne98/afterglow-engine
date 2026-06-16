use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::{action_state::ActionState, input_map::InputMap};
use lightyear::prelude::{
    client::{InputDelayConfig, InputTimelineConfig},
    input::{InputConfig, server::ServerInputConfig},
    *,
};

use super::{protocol::*, scene::PlayerName};

use crate::{
    input::{AfterglowAction, default_gameplay_input_map},
    network::{AfterglowNetworkContext, lightyear::SessionLightyearLinks},
};

#[derive(Resource, Default)]
pub struct DemoInput(pub Vec2);

pub fn configure_demo_input_rebroadcast(
    mut client: Option<ResMut<InputConfig<AfterglowAction>>>,
    mut server: Option<ResMut<ServerInputConfig<AfterglowAction>>>,
) {
    if let Some(ref mut client) = client {
        client.rebroadcast_inputs = true;
    }
    if let Some(ref mut server) = server {
        server.rebroadcast_inputs = true;
    }
}

pub fn configure_demo_input_timeline(
    mut commands: Commands,
    links: Option<Res<SessionLightyearLinks>>,
    timelines: Query<(), With<InputTimelineConfig>>,
) {
    let Some(client_link) = links.and_then(|links| links.client_link) else {
        return;
    };
    if timelines.get(client_link).is_ok() {
        return;
    }
    commands.entity(client_link).insert(
        InputTimelineConfig::default().with_input_delay(InputDelayConfig::fixed_input_delay(2)),
    );
}

pub fn collect_input(
    keyboard: Option<Res<ButtonInput<KeyCode>>>,
    mut input: ResMut<DemoInput>,
    mut maps: Query<&mut ActionState<AfterglowAction>, With<InputMap<AfterglowAction>>>,
) {
    let dir = keyboard
        .as_deref()
        .map(keyboard_direction)
        .unwrap_or(input.0);
    input.0 = dir;
    write_move_action(dir, &mut maps);
}

fn write_move_action(
    dir: Vec2,
    maps: &mut Query<&mut ActionState<AfterglowAction>, With<InputMap<AfterglowAction>>>,
) {
    for mut action_state in maps.iter_mut() {
        action_state.set_axis_pair(&AfterglowAction::Move, Vec2::new(-dir.x, dir.y));
        // Lightyear restores delayed/rollback snapshots into `ActionState`.
        // Keep Leafwing's update and fixed-update mirrors in sync after our
        // manual write so the next buffer/send pass cannot serialize an older
        // zero-input mirror after focus changes or timeline corrections.
        action_state.set_update_state_from_state();
        action_state.set_fixed_update_state_from_state();
    }
}

pub fn apply_velocity_to_player(
    velocity: Vec3,
    query: &mut Query<&mut LinearVelocity, (With<PlayerBox>, Without<Predicted>)>,
) {
    for mut linear_vel in query.iter_mut() {
        linear_vel.0 = velocity;
    }
}

pub fn add_input_map_to_local_predicted_player(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    players: Query<(Entity, &PlayerBox, Has<InputMap<AfterglowAction>>), With<Predicted>>,
) {
    let local_member = context
        .as_deref()
        .and_then(|ctx| ctx.get_connection_status().local_member_owner());
    for (entity, player, has_map) in &players {
        if has_map || !is_local_player(player, &player_name, local_member.as_deref()) {
            continue;
        }
        commands.entity(entity).insert(default_gameplay_input_map());
    }
}

pub fn apply_movement(
    time: Res<Time>,
    fallback_input: Res<DemoInput>,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    rollback_links: Query<(), With<Rollback>>,
    mut players: Query<(
        Option<&mut LinearVelocity>,
        Option<&mut Transform>,
        &PlayerBox,
        Option<&ActionState<AfterglowAction>>,
        Has<Predicted>,
    )>,
) {
    let status = context.as_deref().map(|ctx| ctx.get_connection_status());
    if status.is_some_and(|status| !status.runs_authority() && !status.runs_client_prediction()) {
        return;
    }

    let local_member = status.and_then(|status| status.local_member_owner());
    let client_only = status.is_some_and(|status| status.is_client_only());
    let authority = status.is_some_and(|status| status.runs_authority());
    let in_rollback = rollback_links.iter().next().is_some();

    for (linear_vel, transform, player_box, action_state, predicted) in players.iter_mut() {
        if client_only && !predicted {
            continue;
        }
        if authority && predicted {
            continue;
        }
        let is_local = is_local_player(player_box, &player_name, local_member.as_deref());
        if client_only && !is_local {
            continue;
        }

        let dir = if client_only && is_local && !in_rollback {
            // Local prediction must stay immediate even when Lightyear has an
            // input delay configured for server-side consumption. The native
            // input pipeline still sends the ActionState written by
            // `collect_input`, but presentation uses the current keyboard
            // sample so losing/regaining focus cannot leave visuals stuck on a
            // delayed zero-input snapshot. During rollback replay, use the
            // historical ActionState restored by Lightyear instead.
            fallback_input.0.clamp_length_max(1.0)
        } else {
            movement_direction(
                action_state,
                (is_local && !in_rollback).then_some(fallback_input.0),
            )
        };
        let vel = Vec3::new(dir.x, 0.0, dir.y) * PLAYER_SPEED;
        if let Some(mut linear_vel) = linear_vel {
            linear_vel.0 = vel;
        } else if let Some(mut transform) = transform {
            transform.translation += vel * time.delta_secs();
        }
    }
}

fn movement_direction(
    action_state: Option<&ActionState<AfterglowAction>>,
    fallback: Option<Vec2>,
) -> Vec2 {
    action_state
        .map(|state| {
            let axis = state.clamped_axis_pair(&AfterglowAction::Move);
            Vec2::new(-axis.x, axis.y).clamp_length_max(1.0)
        })
        .unwrap_or_else(|| fallback.unwrap_or(Vec2::ZERO))
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

fn is_local_player(
    player_box: &PlayerBox,
    player_name: &PlayerName,
    local_member: Option<&str>,
) -> bool {
    player_box.owner == player_name.0 || local_member == Some(player_box.owner.as_str())
}
