use super::*;

#[test]
fn udp_prespawned_cue_is_preserved_when_server_confirms() {
    let mut rig = udp_rig(1, register_test_protocol);

    let cue_hash: u64 = 0xA6AB_0000_CAFE_BEEF;
    let client_link = rig.client_link(0);
    let predicted_cue = rig
        .client_world_mut(0)
        .spawn((
            TestCue { value: 42 },
            PreSpawned::new(cue_hash).for_receiver(client_link),
        ))
        .id();

    rig.server_world_mut().spawn((
        TestCue { value: 42 },
        PreSpawned::new(cue_hash),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
    ));

    rig.advance(2);

    assert!(rig.client_world(0).get_entity(predicted_cue).is_ok());
    assert_eq!(
        rig.client_world(0)
            .get::<TestCue>(predicted_cue)
            .unwrap()
            .value,
        42
    );
}

#[test]
fn udp_prespawned_cue_expires_when_server_does_not_confirm() {
    let mut rig = udp_rig(1, register_test_protocol);

    let cue_hash: u64 = 0xBEEF_CAFE_1234_5678;
    let client_link = rig.client_link(0);

    if rig.client_world(0).get::<Connected>(client_link).is_none() {
        let srv = PeerId::Server;
        rig.client_world_mut(0)
            .entity_mut(client_link)
            .insert((Connected, RemoteId(srv)));
    }

    let predicted_cue = rig
        .client_world_mut(0)
        .spawn((
            TestCue { value: 99 },
            PreSpawned::new(cue_hash).for_receiver(client_link),
        ))
        .id();

    assert!(rig.client_world(0).get_entity(predicted_cue).is_ok());

    rig.advance(120);

    assert!(
        rig.client_world(0).get_entity(predicted_cue).is_err(),
        "Unmatched PreSpawned entity should be despawned after timeout"
    );
}

#[test]
fn udp_client_prediction_drift_corrected_by_server() {
    let mut rig = udp_rig(1, register_drift_protocol);

    let sid = StableEntityId::from_raw(100);
    let _server_entity = rig.spawn_replicated(
        sid,
        Health {
            current: 90,
            max: 100,
        },
    );

    rig.advance(2);

    let client_entity = rig.find_client_entity(0, sid).unwrap();

    assert_eq!(
        rig.client_world(0)
            .get::<Health>(client_entity)
            .unwrap()
            .current,
        90
    );

    rig.client_world_mut(0)
        .get_mut::<Health>(client_entity)
        .unwrap()
        .current = 100;

    rig.advance(3);

    let client_hp = rig
        .client_world(0)
        .get::<Health>(client_entity)
        .unwrap()
        .current;
    assert_eq!(client_hp, 90, "Server should correct drift");
}
