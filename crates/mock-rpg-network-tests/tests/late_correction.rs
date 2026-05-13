use afterglow_engine::{
    core::{
        AfterglowCorePlugin,
        identity::{Replicated, StableEntityId},
    },
    network::{
        AfterglowNetworkPlugin,
        replication::{
            Replicate, ReplicatedMessage, ReplicatedRollbackWorldExt, ReplicatedTick,
            ReplicationAppExt, RollbackReplicationClock, component, message, resource,
        },
        rollback::RollbackPolicy,
    },
};
use bevy::prelude::*;

const ALICE: StableEntityId = StableEntityId::from_raw(1);
const BOB: StableEntityId = StableEntityId::from_raw(2);
const CAROL: StableEntityId = StableEntityId::from_raw(3);

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct RepCombatant {
    hp: i32,
    shield_through: u32,
}

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct RepProjectile {
    caster: StableEntityId,
    target: StableEntityId,
    impact_tick: u32,
    damage: i32,
    resolved: bool,
}

#[derive(Component, Clone, Debug, Eq, PartialEq)]
struct RepDeathMarker {
    victim: StableEntityId,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
struct RepCombatLog {
    facts: Vec<CombatFact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CombatFact {
    ShieldRaised {
        tick: u32,
        player: StableEntityId,
    },
    SpellCast {
        tick: u32,
        caster: StableEntityId,
        target: StableEntityId,
    },
    SpellBlocked {
        tick: u32,
        target: StableEntityId,
    },
    PlayerDied {
        tick: u32,
        player: StableEntityId,
    },
}

#[derive(Message, Clone, Debug, Eq, PartialEq)]
struct CastSpell {
    tick: u32,
    caster: StableEntityId,
    target: StableEntityId,
    damage: i32,
}

#[derive(Message, Clone, Debug, Eq, PartialEq)]
struct RaiseShield {
    tick: u32,
    player: StableEntityId,
}

impl Replicate for RepCombatant {
    const REPLICATION_NAME: &'static str = "mock_rpg::RepCombatant";
}

impl Replicate for RepProjectile {
    const REPLICATION_NAME: &'static str = "mock_rpg::RepProjectile";
}

impl Replicate for RepDeathMarker {
    const REPLICATION_NAME: &'static str = "mock_rpg::RepDeathMarker";
}

impl Replicate for RepCombatLog {
    const REPLICATION_NAME: &'static str = "mock_rpg::RepCombatLog";
}

impl ReplicatedMessage for CastSpell {
    fn tick(&self) -> u32 {
        self.tick
    }
}

impl ReplicatedMessage for RaiseShield {
    fn tick(&self) -> u32 {
        self.tick
    }
}

fn combat_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AfterglowCorePlugin, AfterglowNetworkPlugin));
    app.replicate(component::<RepCombatant>())
        .replicate(component::<RepProjectile>())
        .replicate(component::<RepDeathMarker>())
        .replicate(resource::<RepCombatLog>())
        .replicate(message::<CastSpell>())
        .replicate(message::<RaiseShield>())
        .add_systems(
            ReplicatedTick,
            (raise_shields, spawn_projectiles, resolve_projectiles).chain(),
        );
    app.world_mut()
        .resource_mut::<RollbackReplicationClock>()
        .policy = RollbackPolicy {
        max_rollback_ticks: 16,
        commit_delay_ticks: 3,
    };
    app
}

fn spawn_combatants(app: &mut App) {
    app.world_mut().insert_resource(RepCombatLog::default());
    app.world_mut()
        .spawn((ALICE, Replicated, RepCombatant::new(100)));
    app.world_mut()
        .spawn((BOB, Replicated, RepCombatant::new(100)));
    app.world_mut()
        .spawn((CAROL, Replicated, RepCombatant::new(100)));
    app.world_mut().save_replicated_state(1);
}

impl RepCombatant {
    fn new(hp: i32) -> Self {
        Self {
            hp,
            shield_through: 0,
        }
    }
}

fn raise_shields(
    clock: Res<RollbackReplicationClock>,
    mut shields: MessageReader<RaiseShield>,
    mut combatants: Query<(&StableEntityId, &mut RepCombatant), With<Replicated>>,
    mut log: ResMut<RepCombatLog>,
) {
    for shield in shields.read() {
        for (stable, mut combatant) in &mut combatants {
            if *stable == shield.player {
                combatant.shield_through = clock.current_tick.saturating_add(1);
                log.facts.push(CombatFact::ShieldRaised {
                    tick: clock.current_tick,
                    player: shield.player,
                });
            }
        }
    }
}

fn spawn_projectiles(
    mut casts: MessageReader<CastSpell>,
    mut commands: Commands,
    mut log: ResMut<RepCombatLog>,
) {
    for cast in casts.read() {
        commands.spawn((
            projectile_id(cast),
            Replicated,
            RepProjectile {
                caster: cast.caster,
                target: cast.target,
                impact_tick: cast.tick.saturating_add(1),
                damage: cast.damage,
                resolved: false,
            },
        ));
        log.facts.push(CombatFact::SpellCast {
            tick: cast.tick,
            caster: cast.caster,
            target: cast.target,
        });
    }
}

fn resolve_projectiles(
    clock: Res<RollbackReplicationClock>,
    mut projectiles: Query<(&mut RepProjectile, &StableEntityId), With<Replicated>>,
    mut combatants: Query<(&StableEntityId, &mut RepCombatant), With<Replicated>>,
    death_markers: Query<&RepDeathMarker, With<Replicated>>,
    mut commands: Commands,
    mut log: ResMut<RepCombatLog>,
) {
    let mut impacts = Vec::new();
    for (mut projectile, projectile_id) in &mut projectiles {
        if projectile.resolved || projectile.impact_tick > clock.current_tick {
            continue;
        }
        projectile.resolved = true;
        impacts.push((*projectile_id, projectile.clone()));
    }

    for (_, projectile) in impacts {
        let Some((_, mut target)) = combatants
            .iter_mut()
            .find(|(stable, _)| **stable == projectile.target)
        else {
            continue;
        };

        if target.shield_through >= clock.current_tick {
            log.facts.push(CombatFact::SpellBlocked {
                tick: clock.current_tick,
                target: projectile.target,
            });
            continue;
        }

        target.hp = (target.hp - projectile.damage).max(0);
        let already_dead = death_markers
            .iter()
            .any(|marker| marker.victim == projectile.target)
            || log.facts.iter().any(|fact| {
                matches!(
                    fact,
                    CombatFact::PlayerDied { player, .. } if *player == projectile.target
                )
            });
        if target.hp == 0 && !already_dead {
            commands.spawn((
                death_marker_id(projectile.target),
                Replicated,
                RepDeathMarker {
                    victim: projectile.target,
                },
            ));
            log.facts.push(CombatFact::PlayerDied {
                tick: clock.current_tick,
                player: projectile.target,
            });
        }
    }
}

#[test]
fn late_shield_replay_removes_provisional_death_without_manual_cleanup() {
    let mut app = combat_app();
    spawn_combatants(&mut app);
    replace_casts(&mut app, [alice_hits_bob()]);

    app.world_mut().replay_replicated_ticks(1, 3).unwrap();
    assert_hp(&mut app, BOB, 0);
    assert_death_markers(&mut app, BOB, 1);
    assert!(combat_log(&app).contains(&CombatFact::PlayerDied {
        tick: 3,
        player: BOB,
    }));

    replace_casts(&mut app, [alice_hits_bob()]);
    replace_shields(&mut app, [bob_shields()]);
    app.world_mut().replay_replicated_ticks(1, 3).unwrap();

    assert_hp(&mut app, BOB, 100);
    assert_death_markers(&mut app, BOB, 0);
    assert!(!combat_log(&app).contains(&CombatFact::PlayerDied {
        tick: 3,
        player: BOB,
    }));
    assert!(combat_log(&app).contains(&CombatFact::SpellBlocked {
        tick: 3,
        target: BOB,
    }));
}

#[test]
fn simultaneous_lethal_impacts_emit_one_death_marker_and_fact() {
    let mut app = combat_app();
    spawn_combatants(&mut app);
    replace_casts(
        &mut app,
        [
            alice_hits_bob(),
            CastSpell {
                tick: 2,
                caster: CAROL,
                target: BOB,
                damage: 120,
            },
        ],
    );

    app.world_mut().replay_replicated_ticks(1, 3).unwrap();

    assert_hp(&mut app, BOB, 0);
    assert_death_markers(&mut app, BOB, 1);
    assert_eq!(death_facts_for(&app, BOB), 1);
}

#[test]
fn shield_after_impact_does_not_rewrite_a_valid_death() {
    let mut app = combat_app();
    spawn_combatants(&mut app);
    replace_casts(&mut app, [alice_hits_bob()]);
    replace_shields(
        &mut app,
        [RaiseShield {
            tick: 4,
            player: BOB,
        }],
    );

    app.world_mut().replay_replicated_ticks(1, 4).unwrap();

    assert_hp(&mut app, BOB, 0);
    assert_death_markers(&mut app, BOB, 1);
    assert!(combat_log(&app).contains(&CombatFact::PlayerDied {
        tick: 3,
        player: BOB,
    }));
}

#[test]
fn corrected_replay_is_idempotent_and_does_not_duplicate_outputs() {
    let mut app = combat_app();
    spawn_combatants(&mut app);
    replace_casts(&mut app, [alice_hits_bob()]);
    replace_shields(&mut app, [bob_shields()]);

    app.world_mut().replay_replicated_ticks(1, 3).unwrap();
    let first_log = combat_log(&app).to_vec();
    let first_projectiles = projectiles(&mut app);

    app.world_mut().replay_replicated_ticks(1, 3).unwrap();

    assert_eq!(combat_log(&app), first_log.as_slice());
    assert_eq!(projectiles(&mut app), first_projectiles);
    assert_death_markers(&mut app, BOB, 0);
}

#[test]
fn simultaneous_exchange_replays_to_one_block_and_one_death() {
    let mut app = combat_app();
    spawn_combatants(&mut app);
    replace_casts(
        &mut app,
        [
            alice_hits_bob(),
            CastSpell {
                tick: 2,
                caster: BOB,
                target: ALICE,
                damage: 120,
            },
        ],
    );
    replace_shields(&mut app, [bob_shields()]);

    app.world_mut().replay_replicated_ticks(1, 3).unwrap();

    assert_hp(&mut app, ALICE, 0);
    assert_hp(&mut app, BOB, 100);
    assert_death_markers(&mut app, ALICE, 1);
    assert_death_markers(&mut app, BOB, 0);
    assert!(combat_log(&app).contains(&CombatFact::PlayerDied {
        tick: 3,
        player: ALICE,
    }));
    assert!(combat_log(&app).contains(&CombatFact::SpellBlocked {
        tick: 3,
        target: BOB,
    }));
}

fn alice_hits_bob() -> CastSpell {
    CastSpell {
        tick: 2,
        caster: ALICE,
        target: BOB,
        damage: 120,
    }
}

fn bob_shields() -> RaiseShield {
    RaiseShield {
        tick: 2,
        player: BOB,
    }
}

fn replace_casts(app: &mut App, casts: impl IntoIterator<Item = CastSpell>) {
    app.world_mut()
        .resource_mut::<afterglow_engine::network::replication::ReplicatedTimeline<CastSpell>>()
        .replace_for_replay(casts.into_iter().map(|cast| (cast.tick, cast)));
}

fn replace_shields(app: &mut App, shields: impl IntoIterator<Item = RaiseShield>) {
    app.world_mut()
        .resource_mut::<afterglow_engine::network::replication::ReplicatedTimeline<RaiseShield>>()
        .replace_for_replay(shields.into_iter().map(|shield| (shield.tick, shield)));
}

fn assert_hp(app: &mut App, stable: StableEntityId, hp: i32) {
    let world = app.world_mut();
    let mut query = world.query::<(&StableEntityId, &RepCombatant)>();
    let (_, combatant) = query
        .iter(world)
        .find(|(entity_stable, _)| **entity_stable == stable)
        .unwrap();
    assert_eq!(combatant.hp, hp);
}

fn assert_death_markers(app: &mut App, stable: StableEntityId, expected: usize) {
    let world = app.world_mut();
    let mut query = world.query::<&RepDeathMarker>();
    let count = query
        .iter(world)
        .filter(|marker| marker.victim == stable)
        .count();
    assert_eq!(count, expected);
}

fn combat_log(app: &App) -> &[CombatFact] {
    &app.world().resource::<RepCombatLog>().facts
}

fn projectiles(app: &mut App) -> Vec<RepProjectile> {
    let world = app.world_mut();
    let mut query = world.query::<&RepProjectile>();
    let mut projectiles = query.iter(world).cloned().collect::<Vec<_>>();
    projectiles.sort_by_key(|projectile| {
        (
            projectile.caster,
            projectile.target,
            projectile.impact_tick,
            projectile.damage,
        )
    });
    projectiles
}

fn death_facts_for(app: &App, stable: StableEntityId) -> usize {
    combat_log(app)
        .iter()
        .filter(|fact| matches!(fact, CombatFact::PlayerDied { player, .. } if *player == stable))
        .count()
}

fn projectile_id(cast: &CastSpell) -> StableEntityId {
    StableEntityId::from_raw(
        10_000 + u128::from(cast.tick) * 1_000 + cast.caster.as_raw() * 10 + cast.target.as_raw(),
    )
}

fn death_marker_id(victim: StableEntityId) -> StableEntityId {
    StableEntityId::from_raw(20_000 + victim.as_raw())
}
