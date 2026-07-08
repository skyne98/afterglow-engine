use bevy::prelude::*;

use crate::network::{
    AfterglowLightyearConfig, AfterglowLightyearPlugin, LightyearRole,
    connection::{ConnectionEvent, ConnectionEventKind},
};

use super::super::{protocol::PlayerBox, server::MultiplayerBoxesServerPlugin};

fn server_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(crate::core::AfterglowCorePlugin);
    app.insert_resource(AfterglowLightyearConfig {
        role: LightyearRole::Server,
        rebroadcast_inputs: false,
        ..Default::default()
    });
    app.add_plugins((AfterglowLightyearPlugin, MultiplayerBoxesServerPlugin));
    app.finish();
    app.cleanup();
    app
}

fn player_owners(app: &mut App) -> Vec<String> {
    app.world_mut()
        .query::<&PlayerBox>()
        .iter(app.world())
        .map(|player| player.owner.clone())
        .collect()
}

#[test]
fn connected_player_spawn_is_deferred_until_post_update() {
    let mut app = server_app();

    app.world_mut().commands().trigger(ConnectionEvent {
        kind: ConnectionEventKind::Connected,
        player_id: 7,
        link_entity: Entity::PLACEHOLDER,
    });

    app.world_mut().run_schedule(PreUpdate);
    assert_eq!(
        player_owners(&mut app),
        Vec::<String>::new(),
        "connection observers must not spawn replicated players in the same PreUpdate frame"
    );

    app.world_mut().run_schedule(PostUpdate);
    assert_eq!(player_owners(&mut app), vec!["7".to_string()]);
}

#[test]
fn disconnect_before_post_update_cancels_pending_player_spawn() {
    let mut app = server_app();

    app.world_mut().commands().trigger(ConnectionEvent {
        kind: ConnectionEventKind::Connected,
        player_id: 7,
        link_entity: Entity::PLACEHOLDER,
    });
    app.world_mut().commands().trigger(ConnectionEvent {
        kind: ConnectionEventKind::Disconnected {
            reason: "same-frame disconnect".to_string(),
        },
        player_id: 7,
        link_entity: Entity::PLACEHOLDER,
    });

    app.world_mut().run_schedule(PreUpdate);
    app.world_mut().run_schedule(PostUpdate);

    assert_eq!(
        player_owners(&mut app),
        Vec::<String>::new(),
        "disconnect before the deferred spawn flush must not leave a ghost player"
    );
}
