use bevy::prelude::*;
use lightyear::prelude::*;
use std::time::Duration;

use crate::core::identity::StableEntityId;

use super::{FpsDemoInputCommand, FpsDemoPlayerState};

pub(super) struct FpsStateChannel;

pub(super) fn register_fps_demo_lightyear_protocol(app: &mut App) {
    app.init_resource::<PeerMetadata>();
    app.add_channel::<FpsStateChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: Duration::ZERO,
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<FpsDemoInputCommand>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_component::<StableEntityId>();
    app.register_component::<FpsDemoPlayerState>()
        .add_prediction();
}

pub(super) fn fps_demo_transport(
    registry: &ChannelRegistry,
    send_input: bool,
    receive_input: bool,
) -> Transport {
    let mut transport = Transport::default();
    if send_input {
        transport.add_sender_from_registry::<FpsStateChannel>(registry);
    }
    if receive_input {
        transport.add_receiver_from_registry::<FpsStateChannel>(registry);
    }
    transport.add_sender_from_registry::<MetadataChannel>(registry);
    transport.add_receiver_from_registry::<MetadataChannel>(registry);
    transport.add_sender_from_registry::<ActionsChannel>(registry);
    transport.add_receiver_from_registry::<ActionsChannel>(registry);
    transport.add_sender_from_registry::<UpdatesChannel>(registry);
    transport.add_receiver_from_registry::<UpdatesChannel>(registry);
    transport
}
