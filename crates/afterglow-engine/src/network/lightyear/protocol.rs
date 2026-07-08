use bevy::prelude::*;

use crate::{core::identity::StableEntityId, network::HistoryTick};

/// Marker resource set after the first call to
/// [`register_afterglow_lightyear_protocol`] to make the function idempotent.
#[derive(Resource)]
struct AfterglowLightyearProtocolRegistered;

/// Register all engine-provided Lightyear protocol types with the app.
///
/// Registers:
/// - `StableEntityId` (with prediction)
/// - `Transform` (with prediction + linear correction + interpolation)
/// - `LinearVelocity` (with prediction)
///
/// Initializes [`HistoryTick`] and registers reflection types.
///
/// Call this AFTER Lightyear client/server plugins have been added.
///
/// Idempotent: repeated calls after the first are a no-op.
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
        use avian3d::prelude::LinearVelocity;
        use bevy::math::Isometry3d;
        use lightyear::prelude::*;

        // Engine-level replicated components.
        // StableEntityId is predicted so predicted copies keep the same id.
        app.register_component::<StableEntityId>().add_prediction();

        // Transform is the canonical networked pose. Predicted + linearly
        // corrected + interpolated for smooth visual presentation.
        app.register_component::<Transform>()
            .add_prediction()
            .add_linear_correction_fn::<Isometry3d>()
            .add_interpolation_with(TransformLinearInterpolation::lerp);

        // LinearVelocity is predicted so predicted copies maintain correct
        // velocities for rollback reconciliation.
        app.register_component::<LinearVelocity>().add_prediction();
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
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / 60.0),
        });
        // Should not panic:
        register_afterglow_lightyear_protocol(&mut app);
    }

    #[cfg(feature = "lightyear")]
    #[test]
    fn protocol_registration_is_idempotent() {
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / 60.0),
        });
        register_afterglow_lightyear_protocol(&mut app);
        // Second call — must not panic.
        register_afterglow_lightyear_protocol(&mut app);

        assert!(app.world().contains_resource::<HistoryTick>());
    }
}
