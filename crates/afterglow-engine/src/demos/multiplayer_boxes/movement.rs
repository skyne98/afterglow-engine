use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::*;

use super::protocol::*;

use crate::network::lightyear::SessionLightyearLinks;
use crate::network::session::AfterglowSessionState;

#[derive(Resource, Default)]
pub struct DemoInput(pub Vec2);

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
    mut players: Query<&mut LinearVelocity, (With<PlayerBox>, Without<Predicted>)>,
) {
    let vel = Vec3::new(input.0.x, 0.0, input.0.y) * PLAYER_SPEED;
    apply_velocity_to_player(vel, &mut players);
}

pub fn ensure_message_sender(
    links: Res<SessionLightyearLinks>,
    mut commands: Commands,
    senders: Query<(), With<MessageSender<MoveInputMsg>>>,
    registry: Option<Res<ChannelRegistry>>,
) {
    let Some(entity) = links.client_link else { return; };
    if senders.get(entity).is_err() {
        commands.entity(entity).insert(MessageSender::<MoveInputMsg>::default());

        // Add the MoveInputChannel to the transport so messages can be sent.
        if let Some(ref registry) = registry {
            let mut transport = Transport::default();
            transport.add_sender_from_registry::<MetadataChannel>(&registry);
            transport.add_receiver_from_registry::<MetadataChannel>(&registry);
            transport.add_sender_from_registry::<UpdatesChannel>(&registry);
            transport.add_receiver_from_registry::<UpdatesChannel>(&registry);
            transport.add_sender_from_registry::<MoveInputChannel>(&registry);
            commands.entity(entity).insert(transport);
        }
    }
}

pub fn ensure_message_receivers(
    mut commands: Commands,
    links: Query<Entity, (With<LinkOf>, Without<MessageReceiver<MoveInputMsg>>)>,
) {
    for entity in &links {
        commands.entity(entity).insert(MessageReceiver::<MoveInputMsg>::default());
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
