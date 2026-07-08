//! Client-link configuration: input delay and input timeline.
//!
//! The `On<Add, Connected>` observer fires on the client side when the
//! Lightyear netcode connection reaches the Connected state. At that point
//! we insert the input/interpolation timeline components with the configured
//! input delay so that server-side input processing uses delayed inputs
//! (keeping local predicted presentation immediate).

use bevy::prelude::*;

use super::ConnectionConfig;

/// Observer that fires when the client entity gains the `Connected` component
/// (Lightyear netcode handshake complete).
///
/// Inserts the input/interpolation timelines, sync markers, and
/// `InputTimelineConfig` with the input delay from [`ConnectionConfig`]. This
/// runs exactly once per connection because `On<Add, Connected>` fires only
/// when `Connected` is first added, not on subsequent reconfirmations.
pub fn on_client_connected(
    trigger: On<Add, lightyear::prelude::client::Connected>,
    mut commands: Commands,
    config: Res<ConnectionConfig>,
) {
    let entity = trigger.entity;

    commands.entity(entity).insert((
        lightyear::prelude::InputTimeline::default(),
        lightyear::prelude::IsSynced::<lightyear::prelude::InputTimeline>::default(),
        lightyear::prelude::InterpolationTimeline::default(),
        lightyear::prelude::IsSynced::<lightyear::prelude::InterpolationTimeline>::default(),
        lightyear::prelude::InputTimelineConfig::default().with_input_delay(
            lightyear::prelude::client::InputDelayConfig::fixed_input_delay(
                config.input_delay_ticks,
            ),
        ),
    ));

    bevy::log::info!(
        "Client link {} connected with {} tick(s) of input delay",
        entity,
        config.input_delay_ticks,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightyear::prelude::{InputTimeline, InputTimelineConfig, InterpolationTimeline, IsSynced};

    #[test]
    fn client_connected_inserts_input_timeline_sync_components() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ConnectionConfig {
            input_delay_ticks: 7,
            ..Default::default()
        });
        app.add_observer(on_client_connected);

        let client = app
            .world_mut()
            .spawn((
                lightyear::prelude::RemoteId(lightyear::prelude::PeerId::Server),
                lightyear::prelude::client::Connected,
            ))
            .id();
        app.update();

        assert!(
            app.world().get::<InputTimeline>(client).is_some(),
            "live UDP client links must have InputTimeline so Lightyear can buffer/send input"
        );
        assert!(
            app.world().get::<InputTimelineConfig>(client).is_some(),
            "live UDP client links must carry configured input delay"
        );
        assert!(
            app.world().get::<IsSynced<InputTimeline>>(client).is_some(),
            "Lightyear input message systems filter on IsSynced<InputTimeline>"
        );
        assert!(
            app.world().get::<InterpolationTimeline>(client).is_some(),
            "input message interpolation-delay calculation requires InterpolationTimeline"
        );
        assert!(
            app.world()
                .get::<IsSynced<InterpolationTimeline>>(client)
                .is_some(),
            "input send systems also filter on IsSynced<InterpolationTimeline> when interpolation is enabled"
        );
    }
}
