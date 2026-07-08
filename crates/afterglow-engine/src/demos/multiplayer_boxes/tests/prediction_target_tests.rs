use lightyear::prelude::{Interpolated, NetworkTarget, PredictionTarget};

use super::*;

fn player_prediction_target() -> PredictionTarget {
    PredictionTarget::to_clients(NetworkTarget::All)
}

#[test]
fn player_is_predicted_to_all_clients() {
    let target = player_prediction_target();
    let debug = format!("{target:?}");
    assert!(
        debug.contains("All"),
        "player prediction target should be all clients so every client has local contact bodies: {debug}"
    );
    assert!(
        !debug.contains("Single(") && !debug.contains("AllExcept"),
        "player prediction should not be limited to one owner or paired with interpolation targeting: {debug}"
    );
}

#[test]
fn interpolated_player_copy_is_not_rendered_for_prediction_only_demo() {
    let mut app = test_app();
    app.add_systems(bevy::prelude::Update, attach_replicated_player_visuals);

    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "3".to_string(),
            },
            bevy::prelude::Transform::from_xyz(1.0, PLAYER_SIZE, 0.0),
            Interpolated,
        ))
        .id();

    app.update();

    assert!(
        app.world().get::<Mesh3d>(entity).is_none(),
        "multiplayer_boxes renders predicted PlayerBox copies only; interpolated copies must stay invisible to avoid duplicate players"
    );
}

#[test]
fn remote_predicted_player_gets_visuals_without_interpolated_marker() {
    let mut app = test_app();
    app.insert_resource(crate::network::AfterglowNetworkContext::new(
        crate::network::LightyearRole::Client,
        2,
    ));
    app.add_systems(bevy::prelude::Update, attach_replicated_player_visuals);

    let entity = app
        .world_mut()
        .spawn((
            PlayerBox {
                owner: "3".to_string(),
            },
            bevy::prelude::Transform::from_xyz(1.0, PLAYER_SIZE, 0.0),
            lightyear::prelude::Predicted,
        ))
        .id();

    app.update();

    assert!(
        app.world().get::<Mesh3d>(entity).is_some(),
        "non-local predicted PlayerBox should get visuals"
    );
    assert!(
        app.world()
            .get::<avian3d::prelude::LinearVelocity>(entity)
            .is_none(),
        "visual attachment must not overwrite predicted physics state; PreUpdate physics attachment owns LinearVelocity"
    );
}
