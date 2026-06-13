use bevy::prelude::*;

use crate::{core::identity::StableEntityId, network::HistoryTick};

/// Marker resource set after the first call to
/// [`register_afterglow_lightyear_protocol`] to make the function idempotent.
#[derive(Resource)]
struct AfterglowLightyearProtocolRegistered;

/// Register all engine-provided Lightyear protocol types with the app.
///
/// Always initializes [`HistoryTick`] and registers reflection for
/// [`HistoryTick`] and [`StableEntityId`]. When the `lightyear` feature is
/// active, also registers [`StableEntityId`] as a Lightyear-replicated
/// component. Call this after Lightyear client/server plugins have been added;
/// the helper needs Lightyear's protocol registry to exist before component
/// registration.
///
/// Idempotent: repeated calls after the first are a no-op.
///
/// Returns `&mut App` for chaining.
pub fn register_afterglow_lightyear_protocol(app: &mut App) -> &mut App {
    if app
        .world()
        .contains_resource::<AfterglowLightyearProtocolRegistered>()
    {
        return app;
    }

    app.init_resource::<HistoryTick>()
        .register_type::<HistoryTick>()
        .register_type::<StableEntityId>();

    #[cfg(feature = "lightyear")]
    {
        use lightyear::prelude::*;
        app.register_component::<StableEntityId>();
    }

    app.insert_resource(AfterglowLightyearProtocolRegistered);
    app
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_init_resource_history_tick() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_afterglow_lightyear_protocol(&mut app);
        assert!(app.world().contains_resource::<HistoryTick>());
    }

    #[test]
    fn protocol_registration_is_idempotent_without_lightyear() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_afterglow_lightyear_protocol(&mut app);
        register_afterglow_lightyear_protocol(&mut app);
        assert!(app.world().contains_resource::<HistoryTick>());
    }

    #[test]
    fn protocol_registers_reflect_types() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_afterglow_lightyear_protocol(&mut app);

        let registry = app.world().resource::<AppTypeRegistry>();
        let read = registry.read();
        assert!(read.get(std::any::TypeId::of::<StableEntityId>()).is_some());
    }

    #[cfg(feature = "lightyear")]
    #[test]
    fn protocol_helper_adds_lightyear_components() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: std::time::Duration::from_secs_f64(1.0 / 60.0),
        });
        // Should not panic:
        register_afterglow_lightyear_protocol(&mut app);
    }

    #[cfg(feature = "lightyear")]
    #[test]
    fn protocol_registration_is_idempotent() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: std::time::Duration::from_secs_f64(1.0 / 60.0),
        });
        register_afterglow_lightyear_protocol(&mut app);
        // Second call — must not panic.
        register_afterglow_lightyear_protocol(&mut app);

        assert!(app.world().contains_resource::<HistoryTick>());
    }
}
