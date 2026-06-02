pub mod rig;
pub use rig::TransportConfig;
pub mod scenarios;

#[cfg(test)]
pub mod controller;

#[cfg(test)]
mod tests {
    use crate::rig::LightyearTestRig;
    use afterglow_engine::{
        core::identity::StableEntityId, input::AfterglowAction, network::LightyearRole,
    };
    use bevy::prelude::*;
    use leafwing_input_manager::action_state::ActionState;
    use lightyear::prelude::*;
    use serde::{Deserialize, Serialize};
    use std::time::Duration;

    struct TestChannel;

    #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
    struct TestPing(u32);

    fn register_ping_protocol(app: &mut App, _role: LightyearRole) {
        app.add_channel::<TestChannel>(ChannelSettings {
            mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
            send_frequency: Duration::ZERO,
            priority: 1.0,
        })
        .add_direction(NetworkDirection::ClientToServer);
        app.register_message::<TestPing>()
            .add_direction(NetworkDirection::ClientToServer);
    }

    #[test]
    fn rig_boots_and_delivers_message() {
        let mut rig = LightyearTestRig::new(1, |_app| {}, register_ping_protocol);

        let client_link = rig.client_link(0);
        let server_link = rig.server_link(0);

        rig.client_world_mut(0)
            .entity_mut(client_link)
            .insert(MessageSender::<TestPing>::default());
        rig.server_world_mut()
            .entity_mut(server_link)
            .insert(MessageReceiver::<TestPing>::default());

        // Assert pipeline starts clean
        assert_eq!(
            rig.server_world_mut()
                .entity_mut(server_link)
                .get_mut::<MessageReceiver<TestPing>>()
                .expect("server link should have TestPing receiver")
                .receive()
                .count(),
            0,
            "no messages should have arrived before advance_to"
        );

        rig.client_world_mut(0)
            .entity_mut(client_link)
            .get_mut::<MessageSender<TestPing>>()
            .expect("client link should have TestPing sender")
            .send::<TestChannel>(TestPing(42));

        rig.advance(1);

        let received: Vec<TestPing> = rig
            .server_world_mut()
            .entity_mut(server_link)
            .get_mut::<MessageReceiver<TestPing>>()
            .expect("server link should have TestPing receiver")
            .receive()
            .collect();

        assert_eq!(
            received,
            vec![TestPing(42)],
            "server should receive exactly one ping"
        );

        // Deliver another message over multiple ticks to prove batching works
        rig.client_world_mut(0)
            .entity_mut(client_link)
            .get_mut::<MessageSender<TestPing>>()
            .expect("client link should have TestPing sender")
            .send::<TestChannel>(TestPing(99));

        rig.advance(3);

        let batch: Vec<TestPing> = rig
            .server_world_mut()
            .entity_mut(server_link)
            .get_mut::<MessageReceiver<TestPing>>()
            .expect("server link should have TestPing receiver")
            .receive()
            .collect();

        assert_eq!(
            batch,
            vec![TestPing(99)],
            "second message delivered after 3 ticks"
        );
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Resource)]
    struct DelayedFlag(bool);

    #[test]
    fn input_delay_defers_server_processing() {
        let mut rig = LightyearTestRig::new(1, |_app| {}, |_app, _role| {}).with_input_delay_ms(50);

        rig.server_world_mut().insert_resource(DelayedFlag(false));

        // Queue action at tick 1; with ~3 ticks delay (50ms at 60Hz),
        // it should deliver at tick 4.
        rig.queue_action(1, |app| {
            app.world_mut().insert_resource(DelayedFlag(true));
        });

        // Advance 2 ticks (-> tick 2). Action should not have fired.
        rig.advance(2);
        assert_eq!(
            *rig.server_world().resource::<DelayedFlag>(),
            DelayedFlag(false),
            "queue_action should not fire before input_delay_ticks elapses"
        );

        // Advance 2 more ticks (-> tick 4). Action should fire now.
        rig.advance(2);
        assert_eq!(
            *rig.server_world().resource::<DelayedFlag>(),
            DelayedFlag(true),
            "queue_action should fire after input_delay_ticks elapses"
        );
    }

    /// ActionState sent as a custom Lightyear message.
    #[test]
    fn crossbeam_action_state_flows_as_custom_message() {
        let mut rig = LightyearTestRig::new(
            1,
            |_app| {},
            |app, _role| {
                app.add_channel::<TestChannel>(ChannelSettings {
                    mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
                    send_frequency: Duration::ZERO,
                    priority: 1.0,
                })
                .add_direction(NetworkDirection::ClientToServer);
                app.register_message::<ActionState<AfterglowAction>>()
                    .add_direction(NetworkDirection::ClientToServer);
            },
        );

        let client_link = rig.client_link(0);
        let server_link = rig.server_link(0);

        rig.client_world_mut(0)
            .entity_mut(client_link)
            .insert(MessageSender::<ActionState<AfterglowAction>>::default());
        rig.server_world_mut()
            .entity_mut(server_link)
            .insert(MessageReceiver::<ActionState<AfterglowAction>>::default());

        let mut state = ActionState::<AfterglowAction>::default();
        state.press(&AfterglowAction::Jump);

        rig.client_world_mut(0)
            .entity_mut(client_link)
            .get_mut::<MessageSender<ActionState<AfterglowAction>>>()
            .expect("client link should have ActionState sender")
            .send::<TestChannel>(state);

        rig.advance(1);

        let received: Vec<ActionState<AfterglowAction>> = rig
            .server_world_mut()
            .entity_mut(server_link)
            .get_mut::<MessageReceiver<ActionState<AfterglowAction>>>()
            .expect("server link should have ActionState receiver")
            .receive()
            .collect();

        assert_eq!(
            received.len(),
            1,
            "server should receive exactly one ActionState"
        );
        assert!(
            received[0].pressed(&AfterglowAction::Jump),
            "Jump should be pressed"
        );
    }

    #[test]
    fn udp_spawn_replicated_arrives_on_client() {
        let mut rig = LightyearTestRig::new_with_transport(
            1,
            |_app| {},
            |app, _role| {
                app.register_component::<StableEntityId>();
            },
            crate::TransportConfig::Udp { server_port: 0 },
        );
        rig.connect();

        let alice = StableEntityId::from_raw(42);
        let server_entity = rig.spawn_replicated(alice, (Transform::default(),));
        rig.advance(3);

        let client_entity = rig.find_client_entity(0, alice);
        assert!(
            client_entity.is_some(),
            "UDP: replicated entity should appear on client"
        );
        assert_ne!(
            client_entity.unwrap(),
            server_entity,
            "UDP: client entity should differ from server entity"
        );
    }

    #[test]
    fn udp_connect_adds_server_replication_sender_and_is_idempotent() {
        let mut rig = LightyearTestRig::new_with_transport(
            1,
            |_app| {},
            register_ping_protocol,
            crate::TransportConfig::Udp { server_port: 0 },
        );

        rig.connect();
        let original_links = rig.server_links.clone();
        let server_link = rig.server_link(0);

        assert!(
            rig.server_world()
                .get::<ReplicationSender>(server_link)
                .is_some(),
            "server UDP link should get ReplicationSender from the LinkOf observer"
        );
        assert!(
            rig.server_world()
                .get::<MessageManager>(server_link)
                .is_some(),
            "server UDP link should keep Lightyear-managed MessageManager"
        );
        assert!(
            rig.server_world().get::<Transport>(server_link).is_some(),
            "server UDP link should keep Lightyear-managed Transport"
        );

        rig.connect();
        assert_eq!(
            rig.server_links, original_links,
            "connect should be idempotent after UDP links are established"
        );
    }

    #[test]
    fn udp_rig_connects_and_delivers_message() {
        let mut rig = LightyearTestRig::new_with_transport(
            1,
            |_app| {},
            register_ping_protocol,
            crate::TransportConfig::Udp { server_port: 0 },
        );
        rig.connect();

        let client_link = rig.client_link(0);
        let server_link = rig.server_link(0);

        rig.client_world_mut(0)
            .entity_mut(client_link)
            .insert(MessageSender::<TestPing>::default());
        rig.server_world_mut()
            .entity_mut(server_link)
            .insert(MessageReceiver::<TestPing>::default());

        rig.client_world_mut(0)
            .entity_mut(client_link)
            .get_mut::<MessageSender<TestPing>>()
            .expect("client link should have TestPing sender")
            .send::<TestChannel>(TestPing(42));

        rig.advance(3);

        let received: Vec<TestPing> = rig
            .server_world_mut()
            .entity_mut(server_link)
            .get_mut::<MessageReceiver<TestPing>>()
            .expect("server link should have TestPing receiver")
            .receive()
            .collect();

        assert_eq!(
            received,
            vec![TestPing(42)],
            "UDP: server should receive exactly one ping"
        );
    }
}
