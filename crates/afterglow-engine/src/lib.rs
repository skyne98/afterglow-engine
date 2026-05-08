mod setup;

use bevy::prelude::*;

pub struct AfterglowEnginePlugin;

impl Plugin for AfterglowEnginePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup::spawn_scene);
    }
}

pub fn run() -> AppExit {
    App::new()
        .add_plugins((DefaultPlugins, AfterglowEnginePlugin))
        .run()
}

#[cfg(test)]
mod tests {
    use crate::AfterglowEnginePlugin;
    use bevy::app::App;

    #[test]
    fn plugin_registers() {
        let mut app = App::new();
        app.add_plugins((bevy::MinimalPlugins, AfterglowEnginePlugin));
    }
}
