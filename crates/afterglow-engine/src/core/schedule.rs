use bevy::prelude::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub enum AfterglowSet {
    ReadInput,
    BuildCommands,
    Simulate,
    ApplyGameplay,
    PreparePersistence,
    DebugAndMetrics,
}

pub(super) fn configure_engine_sets(app: &mut App) {
    app.configure_sets(
        Update,
        (
            AfterglowSet::ReadInput,
            AfterglowSet::BuildCommands,
            AfterglowSet::Simulate,
            AfterglowSet::ApplyGameplay,
            AfterglowSet::PreparePersistence,
            AfterglowSet::DebugAndMetrics,
        )
            .chain(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Order(Vec<&'static str>);

    fn read_input(mut order: ResMut<Order>) {
        order.0.push("read_input");
    }

    fn build_commands(mut order: ResMut<Order>) {
        order.0.push("build_commands");
    }

    fn simulate(mut order: ResMut<Order>) {
        order.0.push("simulate");
    }

    fn apply_gameplay(mut order: ResMut<Order>) {
        order.0.push("apply_gameplay");
    }

    fn prepare_persistence(mut order: ResMut<Order>) {
        order.0.push("prepare_persistence");
    }

    fn debug_and_metrics(mut order: ResMut<Order>) {
        order.0.push("debug_and_metrics");
    }

    #[test]
    fn engine_sets_run_in_dependency_order() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<Order>();
        configure_engine_sets(&mut app);
        app.add_systems(Update, read_input.in_set(AfterglowSet::ReadInput))
            .add_systems(Update, build_commands.in_set(AfterglowSet::BuildCommands))
            .add_systems(Update, simulate.in_set(AfterglowSet::Simulate))
            .add_systems(Update, apply_gameplay.in_set(AfterglowSet::ApplyGameplay))
            .add_systems(
                Update,
                prepare_persistence.in_set(AfterglowSet::PreparePersistence),
            )
            .add_systems(
                Update,
                debug_and_metrics.in_set(AfterglowSet::DebugAndMetrics),
            );

        app.update();

        assert_eq!(
            app.world().resource::<Order>().0,
            [
                "read_input",
                "build_commands",
                "simulate",
                "apply_gameplay",
                "prepare_persistence",
                "debug_and_metrics"
            ]
        );
    }
}
