use super::{
    runtime::{ensure_rewinded_entities_have_stable_ids, record_rewind_histories},
    *,
};
use crate::core::identity::{RuntimeOnly, StableEntityId};

const DOMAIN: RewindDomainId = RewindDomainId(0);
const OTHER_DOMAIN: RewindDomainId = RewindDomainId(1);

#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
struct DummyComponent {
    value: f32,
}

#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
struct OtherComponent {
    value: u32,
}

fn rewinded(domain: RewindDomainId) -> RewindedEntity {
    RewindedEntity {
        domain,
        budget_override: None,
    }
}

#[test]
fn rewind_id_is_an_alias_for_stable_entity_id() {
    let id: RewindId = StableEntityId::from_raw(42);
    assert_eq!(id, StableEntityId::from_raw(42));
}

#[test]
fn rewinded_entity_marker() {
    let mut app = App::new();
    let entity = app.world_mut().spawn(rewinded(OTHER_DOMAIN)).id();
    let marker = app.world().get::<RewindedEntity>(entity).unwrap();
    assert_eq!(marker.domain, OTHER_DOMAIN);
}

#[test]
fn component_history_push_and_query() {
    let mut hist = ComponentHistory::with_capacity(5, DOMAIN);

    hist.push(10, vec![1, 2, 3], true);
    hist.push(20, vec![4, 5, 6], true);
    hist.push(30, vec![7, 8, 9], true);

    assert_eq!(hist.at(20).unwrap().snapshot, vec![4, 5, 6]);
    assert_eq!(hist.at_or_before(25).unwrap().snapshot, vec![4, 5, 6]);
    assert_eq!(hist.at_or_before(30).unwrap().snapshot, vec![7, 8, 9]);
    assert!(hist.at(15).is_none());
}

#[test]
fn component_history_zero_capacity_keeps_empty() {
    let mut hist = ComponentHistory::with_capacity(0, DOMAIN);

    hist.push(10, vec![1], true);

    assert!(hist.is_empty());
    assert_eq!(hist.len(), 0);
}

#[test]
fn component_history_duplicate_tick_overwrites_snapshot() {
    let mut hist = ComponentHistory::with_capacity(3, DOMAIN);

    hist.push(10, vec![1], true);
    hist.push(10, vec![2], true);

    assert_eq!(hist.len(), 1);
    assert_eq!(hist.at(10).unwrap().snapshot, vec![2]);
}

#[test]
fn component_history_out_of_order_ticks_stay_sorted_and_drop_stale() {
    let mut hist = ComponentHistory::with_capacity(3, DOMAIN);

    hist.push(10, vec![1], true);
    hist.push(30, vec![3], true);
    hist.push(20, vec![2], true);
    hist.push(5, vec![0], true);
    hist.push(40, vec![4], true);

    let ticks: Vec<u32> = hist.iter().map(|entry| entry.tick).collect();
    assert_eq!(ticks, vec![20, 30, 40]);
    assert!(hist.at(5).is_none());
}

#[test]
fn component_history_overflow_drops_oldest() {
    let mut hist = ComponentHistory::with_capacity(3, DOMAIN);

    hist.push(10, vec![0], true);
    hist.push(20, vec![1], true);
    hist.push(30, vec![2], true);
    hist.push(40, vec![3], true);

    assert_eq!(hist.len(), 3);
    assert!(hist.at(10).is_none());
    assert_eq!(hist.at(40).unwrap().snapshot, vec![3]);
}

#[test]
fn component_history_prune() {
    let mut hist = ComponentHistory::with_capacity(10, DOMAIN);

    for tick in (0..=100).step_by(10) {
        hist.push(tick, vec![(tick / 10) as u8], true);
    }

    hist.prune_up_to(50);
    assert_eq!(hist.at(50), None);
    assert!(hist.at(60).is_some());
}

#[test]
fn rewind_app_ext_registers_plugin() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_server_rewind();

    assert!(app.world().contains_resource::<RewindHistoryBudget>());
    assert!(app.world().contains_resource::<RewindComponentRegistry>());
    assert!(app.world().contains_resource::<RewindHistoryStore>());
    assert_eq!(app.world().resource::<RewindHistoryBudget>().max_ticks, 120);
}

#[test]
fn rewind_app_ext_sets_budget_before_plugin_install() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .set_rewind_budget(64)
        .add_server_rewind();

    assert_eq!(app.world().resource::<RewindHistoryBudget>().max_ticks, 64);
    assert!(app.world().contains_resource::<RewindComponentRegistry>());
}

#[test]
fn rewind_app_ext_register_component() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_server_rewind()
        .register_rewind_component::<DummyComponent>(DOMAIN);

    let registry = app.world().resource::<AppTypeRegistry>();
    let registry = registry.read();
    assert!(
        registry
            .get(std::any::TypeId::of::<DummyComponent>())
            .is_some()
    );
}

#[test]
fn rewind_component_registry_deduplicates_by_domain_and_type() {
    let mut registry = RewindComponentRegistry::default();

    registry.register::<DummyComponent>(DOMAIN);
    registry.register::<DummyComponent>(DOMAIN);
    registry.register::<DummyComponent>(OTHER_DOMAIN);
    registry.register::<OtherComponent>(DOMAIN);

    assert_eq!(registry.entries().len(), 3);
}

#[test]
fn record_rewind_histories_captures_registered_matching_domain_components() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_server_rewind()
        .register_rewind_component::<DummyComponent>(DOMAIN);
    app.world_mut().resource_mut::<RewindTick>().0 = 7;
    let tracked_id = StableEntityId::from_raw(11);
    let unmatched_id = StableEntityId::from_raw(12);
    let missing_component_id = StableEntityId::from_raw(13);
    let tracked = app
        .world_mut()
        .spawn((tracked_id, rewinded(DOMAIN), DummyComponent { value: 1.5 }))
        .id();
    assert!(app.world().get_entity(tracked).is_ok());
    app.world_mut().spawn((
        unmatched_id,
        rewinded(OTHER_DOMAIN),
        DummyComponent { value: 2.5 },
    ));
    app.world_mut()
        .spawn((missing_component_id, rewinded(DOMAIN)));

    record_rewind_histories(app.world_mut());

    let type_key = rewind_type_key::<DummyComponent>();
    let store = app.world().resource::<RewindHistoryStore>();
    let entry = store.history(tracked_id, type_key).unwrap().at(7).unwrap();
    let restored: DummyComponent = serde_json::from_slice(&entry.snapshot).unwrap();
    assert_eq!(restored, DummyComponent { value: 1.5 });
    assert!(store.history(unmatched_id, type_key).is_none());
    assert!(store.history(missing_component_id, type_key).is_none());
}

#[test]
fn rewinded_entities_receive_stable_ids_before_history_capture() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_server_rewind()
        .register_rewind_component::<DummyComponent>(DOMAIN);
    app.world_mut().resource_mut::<RewindTick>().0 = 3;
    app.world_mut()
        .spawn((StableEntityId::from_raw(1), rewinded(OTHER_DOMAIN)));
    let entity = app
        .world_mut()
        .spawn((rewinded(DOMAIN), DummyComponent { value: 4.0 }))
        .id();

    ensure_rewinded_entities_have_stable_ids(app.world_mut());
    record_rewind_histories(app.world_mut());

    let stable_id = app.world().get::<StableEntityId>(entity).copied().unwrap();
    assert_eq!(stable_id, StableEntityId::from_raw(2));
    assert!(
        app.world()
            .resource::<RewindHistoryStore>()
            .history(stable_id, rewind_type_key::<DummyComponent>())
            .is_some()
    );
}

#[test]
fn runtime_only_rewinded_entities_do_not_receive_stable_ids_or_history() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_server_rewind()
        .register_rewind_component::<DummyComponent>(DOMAIN);
    let entity = app
        .world_mut()
        .spawn((RuntimeOnly, rewinded(DOMAIN), DummyComponent { value: 4.0 }))
        .id();

    ensure_rewinded_entities_have_stable_ids(app.world_mut());
    record_rewind_histories(app.world_mut());

    assert!(app.world().get::<StableEntityId>(entity).is_none());
    assert!(app.world().resource::<RewindHistoryStore>().is_empty());
}

#[test]
fn record_rewind_histories_honors_per_entity_budget_override() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_server_rewind()
        .register_rewind_component::<DummyComponent>(DOMAIN);
    let entity = app
        .world_mut()
        .spawn((
            StableEntityId::from_raw(21),
            RewindedEntity {
                domain: DOMAIN,
                budget_override: Some(1),
            },
            DummyComponent { value: 1.0 },
        ))
        .id();

    for (tick, value) in [(1, 1.0), (2, 2.0)] {
        app.world_mut().resource_mut::<RewindTick>().0 = tick;
        app.world_mut()
            .get_mut::<DummyComponent>(entity)
            .unwrap()
            .value = value;
        record_rewind_histories(app.world_mut());
    }

    let type_key = rewind_type_key::<DummyComponent>();
    let history = app
        .world()
        .resource::<RewindHistoryStore>()
        .history(StableEntityId::from_raw(21), type_key)
        .unwrap();
    assert_eq!(history.len(), 1);
    assert!(history.at(1).is_none());
    assert!(history.at(2).is_some());
}

#[test]
fn rewind_history_store_prunes_empty_domain_histories() {
    let stable_id = StableEntityId::from_raw(7);
    let mut store = RewindHistoryStore::default();
    store.record_snapshot(
        DOMAIN,
        stable_id,
        11,
        10,
        vec![1],
        RewindHistoryBudget::default(),
    );

    store.prune_domain_up_to(DOMAIN, 10);

    assert!(store.is_empty());
}

#[test]
fn checkpoint_round_trip() {
    let cp = RewindCheckpoint {
        stable_id: StableEntityId::from_raw(7),
        tick: 42,
        components: vec![CheckpointComponent {
            type_key: 0xABCD,
            data: vec![1, 2, 3],
        }],
    };

    let json = serde_json::to_string(&cp).unwrap();
    let restored: RewindCheckpoint = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.stable_id, StableEntityId::from_raw(7));
    assert_eq!(restored.tick, 42);
    assert_eq!(restored.components[0].type_key, 0xABCD);
}

#[test]
fn component_change_serialization() {
    let change = ComponentChange {
        tick: 50,
        stable_id: StableEntityId::from_raw(1),
        type_key: 0x42,
        old_data: Some(vec![0]),
        new_data: vec![1],
    };

    let json = serde_json::to_string(&change).unwrap();
    let restored: ComponentChange = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.tick, 50);
    assert_eq!(restored.new_data, vec![1]);
}

#[test]
fn component_history_iter() {
    let mut hist = ComponentHistory::with_capacity(10, DOMAIN);

    hist.push(1, vec![10], true);
    hist.push(2, vec![20], true);
    hist.push(3, vec![30], true);

    let ticks: Vec<u32> = hist.iter().map(|e| e.tick).collect();
    assert_eq!(ticks, vec![1, 2, 3]);
}

#[test]
fn empty_component_history_queries() {
    let hist = ComponentHistory::with_capacity(5, DOMAIN);

    assert!(hist.is_empty());
    assert_eq!(hist.len(), 0);
    assert!(hist.at(0).is_none());
    assert!(hist.at_or_before(100).is_none());
}
