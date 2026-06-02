use super::*;

#[test]
fn udp_replicate_many_entities() {
    let mut rig = udp_rig(1, register_stress);

    let mut server_entities = Vec::new();
    for i in 0..UDP_STRESS_ENTITY_COUNT {
        let sid = StableEntityId::from_raw((i + 2000) as u128);
        let entity = rig.spawn_replicated(
            sid,
            (
                Health {
                    current: 100,
                    max: 100,
                },
                CombatState::default(),
                Transform::from_xyz(i as f32 * 2.0, 0.0, 0.0),
            ),
        );
        server_entities.push((sid, entity));
    }

    for (sid, _) in &server_entities {
        assert!(
            rig.find_client_entity(0, *sid).is_some(),
            "Entity {sid:?} missing on client"
        );
    }

    rig.advance(10);

    for (sid, _) in &server_entities {
        assert!(
            rig.find_client_entity(0, *sid).is_some(),
            "Entity {sid:?} should persist"
        );
    }
}
