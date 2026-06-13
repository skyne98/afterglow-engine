use super::components::*;
use crate::rig::LightyearTestRig;
use afterglow_engine::{
    core::identity::StableEntityId,
    network::{LightyearRole, register_afterglow_lightyear_protocol},
};
use bevy::prelude::*;
use lightyear::prelude::*;

const ENTITY_COUNT: usize = 50;

fn register_stress(app: &mut App, _role: LightyearRole) {
    register_afterglow_lightyear_protocol(app);
    app.register_component::<Health>().add_prediction();
    app.register_component::<CombatState>().add_prediction();
}

#[test]
fn replicate_many_physics_entities() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_stress);

    let mut server_entities = Vec::new();
    for i in 0..ENTITY_COUNT {
        let sid = StableEntityId::from_raw((i + 1000) as u128);
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
            "Entity {sid:?} should exist on client after replication"
        );
    }

    for (_sid, entity) in &server_entities {
        let hp = rig.server_component::<Health>(*entity).unwrap();
        assert_eq!(hp.current, 100, "Server Health should be intact");
    }

    rig.advance(10);

    for (sid, _) in &server_entities {
        assert!(
            rig.find_client_entity(0, *sid).is_some(),
            "Entity {sid:?} should persist after 10 ticks"
        );
    }
}
