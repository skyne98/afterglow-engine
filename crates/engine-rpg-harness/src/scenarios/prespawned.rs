use super::components::*;
use crate::rig::LightyearTestRig;
use afterglow_engine::{core::identity::StableEntityId, network::LightyearRole};
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct TestCue {
    value: u32,
}

fn register_test_protocol(app: &mut App, _role: LightyearRole) {
    app.register_component::<StableEntityId>();
    app.register_component::<TestCue>().add_prediction();
}

fn reconcile_client_health(mut query: Query<(&mut Health, &Confirmed<Health>), With<Predicted>>) {
    for (mut predicted, confirmed) in &mut query {
        if *predicted != confirmed.0 {
            *predicted = confirmed.0;
        }
    }
}

fn register_drift_protocol(app: &mut App, _role: LightyearRole) {
    app.register_component::<StableEntityId>();
    app.register_component::<Health>().add_prediction();
    app.add_systems(
        PreUpdate,
        reconcile_client_health.after(ReplicationSystems::Receive),
    );
}

#[test]
fn prespawned_cue_is_preserved_when_server_confirms() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_test_protocol);

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

    assert!(
        rig.client_world(0).get_entity(predicted_cue).is_ok(),
        "PreSpawned entity should be preserved after server confirmation"
    );
    assert_eq!(
        rig.client_world(0)
            .get::<TestCue>(predicted_cue)
            .unwrap()
            .value,
        42
    );
}

#[test]
fn prespawned_cue_expires_when_server_does_not_confirm() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_test_protocol);

    let cue_hash: u64 = 0xBEEF_CAFE_1234_5678;
    let client_link = rig.client_link(0);
    let predicted_cue = rig
        .client_world_mut(0)
        .spawn((
            TestCue { value: 99 },
            PreSpawned::new(cue_hash).for_receiver(client_link),
        ))
        .id();

    rig.advance(80);

    assert!(
        rig.client_world(0).get_entity(predicted_cue).is_err(),
        "Unmatched PreSpawned entity should be despawned"
    );
}

#[test]
fn client_prediction_drift_corrected_by_server() {
    let mut rig = LightyearTestRig::new(1, |_| {}, register_drift_protocol);

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
    assert_eq!(
        rig.client_world(0)
            .get::<Health>(client_entity)
            .unwrap()
            .current,
        100
    );

    rig.advance(3);

    let client_hp = rig
        .client_world(0)
        .get::<Health>(client_entity)
        .unwrap()
        .current;
    assert_eq!(
        client_hp, 90,
        "Client should have been corrected from 100 to 90 by server replication"
    );
}
