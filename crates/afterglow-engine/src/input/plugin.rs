use bevy::prelude::*;
use leafwing_input_manager::plugin::InputManagerPlugin;

use super::AfterglowAction;

#[derive(Default)]
pub struct AfterglowInputPlugin;

pub type AfterglowLeafwingPlugin = AfterglowInputPlugin;

impl Plugin for AfterglowInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InputManagerPlugin::<AfterglowAction>::default());
    }
}
