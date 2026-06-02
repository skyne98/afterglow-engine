use super::*;

#[test]
fn udp_door_opens_on_grab() {
    let mut rig = LightyearTestRig::new_with_transport(
        1,
        |app| {
            app.add_plugins(AfterglowPhysicsPlugin);
        },
        register_doors,
        TransportConfig::Udp { server_port: 0 },
    );
    rig.connect();
    let t = rig.current_tick();

    zero_gravity(&mut rig.server_app);
    for client in &mut rig.client_apps {
        zero_gravity(client);
    }

    let door = rig.spawn_replicated(DOOR, door_bundle(Vec3::new(0.0, 0.0, -3.0), false, false));
    let player = rig.spawn_replicated(PLAYER, door_player(Vec3::ZERO));

    let player_c0 = rig
        .find_client_entity(0, PLAYER)
        .expect("PLAYER on client 0");
    let door_c0 = rig.find_client_entity(0, DOOR).expect("DOOR on client 0");
    rig.register_entity(PLAYER, vec![player, player_c0]);
    rig.register_entity(DOOR, vec![door, door_c0]);

    let client_link = rig.client_link(0);
    let hash = door_grab_hash(PLAYER, DOOR);

    let predicted_grab = rig
        .client_world_mut(0)
        .spawn((
            DoorGrab {
                player: PLAYER,
                door: DOOR,
            },
            PreSpawned::new(hash).for_receiver(client_link),
        ))
        .id();

    rig.queue_action(t + 1, {
        let player = player;
        move |app| {
            let mut state = ActionState::<AfterglowAction>::default();
            state.press(&AfterglowAction::Use);
            app.world_mut().entity_mut(player).insert(state);
        }
    });
    rig.queue_action(t + 2, {
        let player = player;
        move |app| clear_action(app.world_mut(), player)
    });

    let player_start_z = rig
        .server_component::<Transform>(player)
        .unwrap()
        .translation
        .z;

    rig.advance_to(t + 30);

    assert!(rig.client_world(0).get_entity(predicted_grab).is_ok());
    assert!(rig.server_component::<DoorState>(door).unwrap().open);

    let player_end_z = rig
        .server_component::<Transform>(player)
        .unwrap()
        .translation
        .z;
    assert!(player_end_z < player_start_z, "Player pulled toward door");
}
