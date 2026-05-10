pub mod chunk;

use bevy::prelude::*;
use chunk::{DemoCellState, load_demo_cell};

pub struct AfterglowWorldPlugin;

impl Plugin for AfterglowWorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DemoCellState>()
            .add_systems(Startup, load_demo_cell);
    }
}
