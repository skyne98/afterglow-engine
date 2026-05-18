use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;

use crate::input::AfterglowAction;

pub fn action_state(
    move_axis: Vec2,
    look_axis: Vec2,
    pressed: &[AfterglowAction],
) -> ActionState<AfterglowAction> {
    let mut state = ActionState::default();
    state.set_axis_pair(&AfterglowAction::Move, move_axis);
    state.set_axis_pair(&AfterglowAction::Look, look_axis);
    for action in pressed {
        state.press(action);
    }
    state
}

pub fn command(axes: &[(&str, f32)], pressed: &[AfterglowAction]) -> ActionState<AfterglowAction> {
    let mut move_axis = Vec2::ZERO;
    let mut look_axis = Vec2::ZERO;
    for (axis, value) in axes {
        match *axis {
            "move.x" => move_axis.x = *value,
            "move.y" => move_axis.y = *value,
            "look.x" => look_axis.x = *value,
            "look.y" => look_axis.y = *value,
            _ => unreachable!("unknown test axis: {axis}"),
        }
    }
    action_state(move_axis, look_axis, pressed)
}

pub fn set_input(app: &mut App, entity: Entity, state: ActionState<AfterglowAction>) {
    app.world_mut().entity_mut(entity).insert(state);
}

pub fn clear_input(app: &mut App, entity: Entity) {
    app.world_mut()
        .entity_mut(entity)
        .remove::<ActionState<AfterglowAction>>();
}
