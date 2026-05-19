use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::{
    controller::{
        FirstPersonCommandState, FirstPersonControllerConfig, FirstPersonMotorState,
        integrate_first_person_command, integrate_first_person_command_look,
        local_move_delta_from_speeds, write_flat_horizontal_velocity_from_delta,
    },
    core::identity::StableEntityId,
    input::AfterglowAction,
};

use super::FpsDemoPlayerState;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct FpsDemoInputCommand {
    pub player: StableEntityId,
    pub tick: u32,
    pub move_axis: Vec2,
    pub look_axis: Vec2,
    pub jump_held: bool,
    pub crouch_held: bool,
    pub sprint_held: bool,
}

#[derive(Resource, Debug)]
pub(super) struct FpsDemoPredictionBuffer {
    commands: VecDeque<FpsDemoInputCommand>,
    max_commands: usize,
    last_authoritative_tick: u32,
}

impl Default for FpsDemoPredictionBuffer {
    fn default() -> Self {
        Self {
            commands: VecDeque::new(),
            max_commands: 180,
            last_authoritative_tick: 0,
        }
    }
}

impl FpsDemoPredictionBuffer {
    pub(super) fn push(&mut self, command: FpsDemoInputCommand) {
        if command.tick <= self.last_authoritative_tick {
            return;
        }
        if let Some(last) = self.commands.back_mut()
            && last.tick == command.tick
        {
            *last = command;
            return;
        }
        self.commands.push_back(command);
        while self.commands.len() > self.max_commands {
            self.commands.pop_front();
        }
    }

    pub(super) fn replay_from_authoritative(
        &mut self,
        authoritative: FpsDemoPlayerState,
    ) -> (FpsDemoPlayerState, FirstPersonMotorState) {
        self.last_authoritative_tick = self
            .last_authoritative_tick
            .max(authoritative.authoritative_tick);
        while self
            .commands
            .front()
            .is_some_and(|command| command.tick <= authoritative.authoritative_tick)
        {
            self.commands.pop_front();
        }
        let mut state = authoritative;
        let mut motor = motor_from_player_state(&state);
        for command in &self.commands {
            (state, motor) = integrate_authoritative_state(state, motor, command);
        }
        (state, motor)
    }
}

pub(super) fn fps_demo_input_command(
    player: StableEntityId,
    tick: u32,
    action_state: Option<&ActionState<AfterglowAction>>,
) -> FpsDemoInputCommand {
    let Some(action_state) = action_state else {
        return FpsDemoInputCommand {
            player,
            tick,
            move_axis: Vec2::ZERO,
            look_axis: Vec2::ZERO,
            jump_held: false,
            crouch_held: false,
            sprint_held: false,
        };
    };

    FpsDemoInputCommand {
        player,
        tick,
        move_axis: action_state.clamped_axis_pair(&AfterglowAction::Move),
        look_axis: action_state.axis_pair(&AfterglowAction::Look),
        jump_held: action_state.pressed(&AfterglowAction::Jump),
        crouch_held: action_state.pressed(&AfterglowAction::Crouch),
        sprint_held: action_state.pressed(&AfterglowAction::Sprint),
    }
}

pub(super) fn integrate_authoritative_state(
    mut state: FpsDemoPlayerState,
    mut motor: FirstPersonMotorState,
    command: &FpsDemoInputCommand,
) -> (FpsDemoPlayerState, FirstPersonMotorState) {
    const DT: f32 = 1.0 / 60.0;
    let config = FirstPersonControllerConfig::default();
    let command_state = first_person_command_state(command);
    integrate_first_person_command_look(&command_state, &config, &mut motor);
    let input = integrate_first_person_command(&command_state, &config, &mut motor, DT);
    let mut translation = state.to_translation();
    let horizontal = local_move_delta_from_speeds(&motor, DT);
    translation += horizontal;
    write_flat_horizontal_velocity_from_delta(&mut motor, horizontal, DT);
    if input.jumped {
        translation.y += motor.velocity.y * DT;
    }
    state = FpsDemoPlayerState::from_translation(translation);
    state.yaw_milliradians = radians_to_milliradians(motor.yaw);
    state.pitch_milliradians = radians_to_milliradians(motor.pitch);
    state.authoritative_tick = command.tick;
    (state, motor)
}

#[cfg(test)]
pub(super) fn default_authoritative_state() -> (FpsDemoPlayerState, FirstPersonMotorState) {
    let state = FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0));
    (state.clone(), motor_from_player_state(&state))
}

pub(super) fn motor_from_player_state(state: &FpsDemoPlayerState) -> FirstPersonMotorState {
    FirstPersonMotorState {
        grounded: true,
        ground_contact_ticks: FirstPersonControllerConfig::default().ground_sticky_ticks,
        yaw: state.yaw_milliradians as f32 / 1000.0,
        pitch: state.pitch_milliradians as f32 / 1000.0,
        ..default()
    }
}

pub(super) fn first_person_command_state(command: &FpsDemoInputCommand) -> FirstPersonCommandState {
    FirstPersonCommandState {
        tick: command.tick,
        move_axis: command.move_axis,
        look_axis: command.look_axis,
        jump_pressed: false,
        jump_held: command.jump_held,
        crouch_pressed: false,
        crouch_held: command.crouch_held,
        sprint_pressed: false,
        sprint_held: command.sprint_held,
    }
}

fn radians_to_milliradians(value: f32) -> i32 {
    (value * 1000.0).round() as i32
}
