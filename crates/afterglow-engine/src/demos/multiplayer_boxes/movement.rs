use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::*;

use super::{protocol::*, scene::PlayerName};

use crate::network::lightyear::SessionLightyearLinks;
use crate::network::session::AfterglowSessionState;

#[derive(Resource, Default)]
pub struct DemoInput(pub Vec2);

#[derive(Component)]
pub(crate) struct MoveInputSenderReady;

#[derive(Component)]
pub(crate) struct MoveInputReceiverReady;

pub fn collect_input(
    keyboard: Option<Res<ButtonInput<KeyCode>>>,
    mut input: ResMut<DemoInput>,
) {
    let Some(keyboard) = keyboard else { return; };
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
    input.0 = dir.clamp_length_max(1.0);
}

pub fn apply_velocity_to_player(
    velocity: Vec3,
    query: &mut Query<&mut LinearVelocity, (With<PlayerBox>, Without<Predicted>)>,
) {
    for mut linear_vel in query.iter_mut() {
        linear_vel.0 = velocity;
    }
}

pub fn apply_movement(
    input: Res<DemoInput>,
    player_name: Res<PlayerName>,
    mut players: Query<(&mut LinearVelocity, &PlayerBox), Without<Predicted>>,
) {
    let vel = Vec3::new(input.0.x, 0.0, input.0.y) * PLAYER_SPEED;
    for (mut linear_vel, player_box) in players.iter_mut() {
        if player_box.owner == player_name.0 {
            linear_vel.0 = vel;
        }
    }
}

pub(crate) fn ensure_message_sender(
    links: Res<SessionLightyearLinks>,
    mut commands: Commands,
    senders: Query<(), With<MessageSender<MoveInputMsg>>>,
    ready: Query<(), With<MoveInputSenderReady>>,
    registry: Option<Res<ChannelRegistry>>,
    mut transports: Query<&mut Transport>,
) {
    let Some(entity) = links.client_link else { return; };
    if senders.get(entity).is_err() {
        commands
            .entity(entity)
            .insert(MessageSender::<MoveInputMsg>::default());
    }
    if ready.get(entity).is_ok() {
        return;
    }
    let Some(ref registry) = registry else { return; };
    if let Ok(mut transport) = transports.get_mut(entity) {
        transport.add_sender_from_registry::<MoveInputChannel>(registry);
    } else {
        let mut transport = Transport::default();
        transport.add_sender_from_registry::<MoveInputChannel>(registry);
        commands.entity(entity).insert(transport);
    }
    commands.entity(entity).insert(MoveInputSenderReady);
}

pub(crate) fn ensure_message_receivers(
    mut commands: Commands,
    links: Query<Entity, (With<LinkOf>, Without<MoveInputReceiverReady>)>,
    receivers: Query<(), With<MessageReceiver<MoveInputMsg>>>,
    registry: Option<Res<ChannelRegistry>>,
    mut transports: Query<&mut Transport>,
) {
    let Some(ref registry) = registry else { return; };
    for entity in &links {
        if receivers.get(entity).is_err() {
            commands
                .entity(entity)
                .insert(MessageReceiver::<MoveInputMsg>::default());
        }
        if let Ok(mut transport) = transports.get_mut(entity) {
            transport.add_receiver_from_registry::<MoveInputChannel>(registry);
        } else {
            let mut transport = Transport::default();
            transport.add_receiver_from_registry::<MoveInputChannel>(registry);
            commands.entity(entity).insert(transport);
        }
        commands.entity(entity).insert(MoveInputReceiverReady);
    }
}

pub fn client_send_input(
    links: Res<SessionLightyearLinks>,
    input: Res<DemoInput>,
    session_state: Res<AfterglowSessionState>,
    mut senders: Query<&mut MessageSender<MoveInputMsg>>,
) {
    let Some(entity) = links.client_link else { return; };
    let Ok(mut sender) = senders.get_mut(entity) else { return; };
    if !session_state.local_member_id.is_valid() {
        return;
    }
    sender.send::<MoveInputChannel>(MoveInputMsg {
        owner: session_state.local_member_id.as_raw().to_string(),
        direction: input.0,
    });
}

pub fn server_receive_input(
    mut receivers: Query<&mut MessageReceiver<MoveInputMsg>>,
    mut players: Query<(&mut LinearVelocity, &PlayerBox)>,
) {
    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            let vel = Vec3::new(msg.direction.x, 0.0, msg.direction.y) * PLAYER_SPEED;
            for (mut linear_vel, player_box) in players.iter_mut() {
                if player_box.owner == msg.owner {
                    linear_vel.0 = vel;
                }
            }
        }
    }
}
