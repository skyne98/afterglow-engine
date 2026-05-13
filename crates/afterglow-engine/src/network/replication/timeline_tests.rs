use super::*;
use crate::core::identity::StableEntityId;
use bevy::prelude::*;

#[derive(Message, Clone, Debug, Eq, PartialEq)]
struct TimelineDamage {
    entity: StableEntityId,
    amount: i32,
    tick: u32,
}

impl ReplicatedMessage for TimelineDamage {
    fn tick(&self) -> u32 {
        self.tick
    }
}

#[derive(Resource, Default)]
struct SeenTimelineDamage(Vec<TimelineDamage>);

fn read_timeline_damage(
    mut messages: MessageReader<TimelineDamage>,
    mut seen: ResMut<SeenTimelineDamage>,
) {
    seen.0.extend(messages.read().cloned());
}

#[test]
fn replay_reissue_skips_messages_older_than_retention_window() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        crate::core::AfterglowCorePlugin,
        crate::network::AfterglowNetworkPlugin,
    ));
    let entity = StableEntityId::from_raw(7);
    app.init_resource::<SeenTimelineDamage>()
        .replicate(message::<TimelineDamage>())
        .add_systems(
            Update,
            read_timeline_damage
                .after(ReplicationSet::ReissueMessages)
                .before(ReplicationSet::CollectChanges),
        );

    app.world_mut()
        .resource_mut::<ReplicatedTimeline<TimelineDamage>>()
        .replace_for_replay([
            (
                79,
                TimelineDamage {
                    entity,
                    amount: 2,
                    tick: 79,
                },
            ),
            (
                200,
                TimelineDamage {
                    entity,
                    amount: 1,
                    tick: 200,
                },
            ),
        ]);
    app.update();

    assert_eq!(
        app.world().resource::<SeenTimelineDamage>().0,
        [TimelineDamage {
            entity,
            amount: 1,
            tick: 200
        }]
    );
    assert!(
        app.world()
            .resource::<ReplicatedTimeline<TimelineDamage>>()
            .messages_at(79)
            .is_empty()
    );
}
