mod bindings;
mod command;
mod evaluation;

pub use bindings::{
    ActionBinding, ActionInput, AxisBinding, AxisComponent, AxisSource, InputContext,
    InputContextId, PlayerInputBindings,
};
pub use command::{
    InputAction, InputActionPhase, InputActionValue, InputAxis, InputAxisValue, InputDeviceRoute,
    LocalInputRoute, LocalInputRoutes, LocalPlayers, PlayerCommand, PlayerCommandQueue,
    PlayerVirtualInput, PointerDevice, PointerInput, SimulationTick, VirtualInputBuffer,
    VirtualInputState,
};
pub use evaluation::{GamepadInput, RawInputState, device_allowed, read_bound_inputs};

use bevy::{
    input::{
        mouse::{AccumulatedMouseMotion, MouseMotion},
        touch::ForceTouch,
    },
    prelude::*,
};

use crate::{core::schedule::AfterglowSet, network::NetworkPlayerId};

pub struct AfterglowInputPlugin;

impl Plugin for AfterglowInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerInputBindings>()
            .init_resource::<VirtualInputState>()
            .init_resource::<LocalPlayers>()
            .init_resource::<LocalInputRoutes>()
            .init_resource::<SimulationTick>()
            .init_resource::<PlayerCommandQueue>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<Touches>()
            .add_message::<MouseMotion>()
            .add_systems(
                Update,
                collect_player_commands.in_set(AfterglowSet::ReadInput),
            );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn collect_player_commands(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    gamepads: Query<(Entity, &Gamepad)>,
    mut mouse_motion: ResMut<AccumulatedMouseMotion>,
    bindings: Res<PlayerInputBindings>,
    mut virtual_input: ResMut<VirtualInputState>,
    local_players: Res<LocalPlayers>,
    input_routes: Res<LocalInputRoutes>,
    mut tick: ResMut<SimulationTick>,
    mut commands: ResMut<PlayerCommandQueue>,
) {
    let gamepads = gamepads
        .iter()
        .map(|(entity, gamepad)| GamepadInput { entity, gamepad })
        .collect::<Vec<_>>();
    let mouse_delta = mouse_motion.delta;
    mouse_motion.delta = Vec2::ZERO;
    let mut next_commands = Vec::with_capacity(local_players.players().len());
    for player in local_players.players() {
        next_commands.push(read_player_command(
            &keyboard,
            &mouse,
            &touches,
            &gamepads,
            mouse_delta,
            &bindings,
            &virtual_input,
            input_routes.devices_for(*player),
            *player,
            tick.0,
        ));
    }
    commands.replace(next_commands);
    virtual_input.clear();
    tick.0 = tick.0.saturating_add(1);
}

#[allow(clippy::too_many_arguments)]
pub fn read_player_command<'a>(
    keyboard: &'a ButtonInput<KeyCode>,
    mouse: &'a ButtonInput<MouseButton>,
    touches: &'a Touches,
    gamepads: &'a [GamepadInput<'a>],
    mouse_delta: Vec2,
    bindings: &PlayerInputBindings,
    virtual_input: &VirtualInputState,
    devices: Option<&'a [InputDeviceRoute]>,
    player: NetworkPlayerId,
    tick: u32,
) -> PlayerCommand {
    let raw = RawInputState {
        keyboard,
        mouse,
        touches,
        gamepads,
        mouse_delta,
        devices,
    };
    let (mut axes, mut actions) = read_bound_inputs(bindings, &raw);
    let mut pointers = Vec::new();
    if device_allowed(devices, InputDeviceRoute::Touch) {
        pointers.extend(touch_pointers(touches));
    }
    if device_allowed(devices, InputDeviceRoute::Virtual) {
        append_virtual_input(
            &mut axes,
            &mut actions,
            &mut pointers,
            virtual_input.shared(),
        );
        if let Some(player_input) = virtual_input.player(player) {
            append_virtual_input(&mut axes, &mut actions, &mut pointers, player_input);
        }
    }

    PlayerCommand {
        player,
        tick,
        axes,
        actions,
        pointers,
    }
}

fn append_virtual_input(
    axes: &mut Vec<InputAxisValue>,
    actions: &mut Vec<InputActionValue>,
    pointers: &mut Vec<PointerInput>,
    input: &VirtualInputBuffer,
) {
    axes.extend(input.axes().iter().cloned());
    actions.extend(input.actions().iter().cloned());
    pointers.extend(input.pointers().iter().cloned());
}

fn touch_pointers(touches: &Touches) -> impl Iterator<Item = PointerInput> + '_ {
    touches.iter().map(|touch| PointerInput {
        device: PointerDevice::Touch,
        id: touch.id(),
        position: touch.position(),
        delta: touch.delta(),
        pressure: force_to_pressure(touch.force()),
        tilt: None,
        twist: None,
        primary: false,
    })
}

fn force_to_pressure(force: Option<ForceTouch>) -> Option<f32> {
    match force {
        Some(ForceTouch::Calibrated {
            force,
            max_possible_force,
            ..
        }) if max_possible_force > 0.0 => Some((force / max_possible_force) as f32),
        Some(ForceTouch::Calibrated { force, .. }) => Some(force as f32),
        Some(ForceTouch::Normalized(force)) => Some(force as f32),
        None => None,
    }
}

#[cfg(test)]
mod tests;
