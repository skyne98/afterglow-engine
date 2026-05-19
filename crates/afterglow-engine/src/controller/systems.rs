use avian3d::{
    character_controller::prelude::MoveAndSlide,
    prelude::{CustomPositionIntegration, Position, Rotation, SpeculativeMargin},
};
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;

use crate::{
    input::AfterglowAction,
    physics::{PhysicsBody, PhysicsCollider},
};

use super::{
    FirstPersonController, FirstPersonControllerTrace, FirstPersonControllerTraceFrame,
    FirstPersonEffectStack, FirstPersonImpulseBuffer, FirstPersonMotorState,
    FirstPersonStepRejectReason, FirstPersonStepTrace, ReplayCommand, body,
    body::{
        apply_first_person_gravity, clamp_local_speeds_to_actual_stance,
        integrate_first_person_command, integrate_first_person_input, integrate_first_person_look,
        local_move_delta_from_speeds, write_flat_horizontal_velocity_from_delta,
    },
    impulse_buffer::apply_first_person_linear_impulse,
    physics, stairs,
    util::flat,
};

type ControllerAuthoringItem<'a> = (
    Entity,
    &'a FirstPersonController,
    Option<&'a FirstPersonMotorState>,
    Option<&'a FirstPersonImpulseBuffer>,
    Option<&'a FirstPersonEffectStack>,
);
type ControllerAuthoringFilter = Or<(Added<FirstPersonController>, Changed<FirstPersonController>)>;

pub(super) fn sync_first_person_controller_authoring(
    mut commands: Commands,
    controllers: Query<ControllerAuthoringItem, ControllerAuthoringFilter>,
) {
    for (entity, controller, state, impulse_buffer, effect_stack) in &controllers {
        let config = &controller.config;
        let stance = state.map_or(super::ControllerStance::Standing, |state| state.stance);
        let height = config.height(stance);
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((
            PhysicsBody::kinematic(),
            PhysicsCollider::cylinder(config.body_radius, height),
            CustomPositionIntegration,
            SpeculativeMargin(0.0),
        ));
        if state.is_none() {
            entity_commands.insert(FirstPersonMotorState::default());
        }
        if impulse_buffer.is_none() {
            entity_commands.insert(FirstPersonImpulseBuffer::default());
        }
        if effect_stack.is_none() {
            entity_commands.insert(FirstPersonEffectStack::default());
        }
    }
}

pub(super) fn update_first_person_look(
    mut controllers: Query<(
        &FirstPersonController,
        Option<&ActionState<AfterglowAction>>,
        Option<&FirstPersonEffectStack>,
        &mut FirstPersonMotorState,
    )>,
) {
    for (controller, action_state, effects, mut state) in &mut controllers {
        let config = effects.map_or_else(
            || controller.config.clone(),
            |effects| effects.effective_config(&controller.config),
        );
        integrate_first_person_look(action_state, &config, &mut state);
    }
}

pub(super) fn drive_first_person_controllers(
    mut entity_commands: Commands,
    time: Res<Time>,
    mut controllers: Query<(
        Entity,
        &FirstPersonController,
        Option<&ActionState<AfterglowAction>>,
        &mut FirstPersonMotorState,
        Option<&mut FirstPersonImpulseBuffer>,
        Option<&mut FirstPersonEffectStack>,
        &mut Transform,
        Option<&ReplayCommand>,
    )>,
    move_and_slide: MoveAndSlide,
    spatial_query: avian3d::prelude::SpatialQuery,
    mut trace: ResMut<FirstPersonControllerTrace>,
) {
    let dt = time.delta_secs();
    let record_trace = trace.enabled;
    for (
        entity,
        controller,
        action_state,
        mut state,
        impulse_buffer,
        effects,
        mut transform,
        replay,
    ) in &mut controllers
    {
        let config = effects.as_ref().map_or_else(
            || controller.config.clone(),
            |effects| effects.effective_config(&controller.config),
        );
        let start_position = transform.translation;
        physics::update_step_climbing(&config, &mut state, dt);
        let after_step_latch_position = transform.translation;
        let input = if let Some(replay) = replay {
            // Server path: apply both look and movement from the received command.
            let step = integrate_first_person_command(&replay.0, &config, &mut state, dt);
            entity_commands.entity(entity).remove::<ReplayCommand>();
            step
        } else {
            // Client path: look is applied from ActionState in this same schedule,
            // no separate Update look path to double-apply.
            integrate_first_person_input(action_state, &config, &mut state, dt)
        };
        let after_input_position = transform.translation;
        let mut active_collider = physics::controller_collider(&config, state.stance);
        if let Some((new_collider, new_authored_collider)) =
            physics::sync_body_stance(entity, &config, &mut state, &mut transform, &spatial_query)
        {
            active_collider = new_collider.clone();
            entity_commands
                .entity(entity)
                .insert((new_collider, new_authored_collider));
        }
        let after_stance_position = transform.translation;
        clamp_local_speeds_to_actual_stance(&input.command, &config, &mut state);
        if let Some(mut impulse_buffer) = impulse_buffer {
            let impulse = impulse_buffer.drain_linear_impulse();
            apply_first_person_linear_impulse(&mut state, impulse);
        }
        transform.rotation = Quat::from_rotation_y(state.yaw);
        let move_delta = local_move_delta_from_speeds(&state, dt);
        let horizontal_pushback = physics::apply_horizontal_move(physics::CharacterMove {
            entity,
            config: &config,
            state: &mut state,
            transform: &mut transform,
            collider: &active_collider,
            move_and_slide: &move_and_slide,
            spatial_query: &spatial_query,
            delta: move_delta,
        });
        let after_horizontal_position = transform.translation;
        let horizontal_step_up =
            after_horizontal_position.y - after_stance_position.y > config.min_step_height;
        let step = if horizontal_step_up {
            state.climbing = true;
            state.velocity.y = 0.0;
            FirstPersonStepTrace::skipped(FirstPersonStepRejectReason::NotRun)
        } else {
            stairs::apply_step_attempt(stairs::StepAttempt {
                entity,
                config: &config,
                state: &mut state,
                transform: &mut transform,
                collider: &active_collider,
                spatial_query: &spatial_query,
                desired_delta: move_delta,
                dt,
                record_trace,
            })
        };
        let after_step_position = transform.translation;
        let actual = after_step_position - after_stance_position;
        write_flat_horizontal_velocity_from_delta(&mut state, flat(actual), dt);
        let gravity_applied = !state.climbing && !input.jumped;
        if gravity_applied {
            apply_first_person_gravity(&config, &mut state, dt);
        }
        let vertical_delta = Vec3::Y * state.velocity.y * dt;
        let vertical_pushback = physics::apply_vertical_force_collision(physics::CharacterMove {
            entity,
            config: &config,
            state: &mut state,
            transform: &mut transform,
            collider: &active_collider,
            move_and_slide: &move_and_slide,
            spatial_query: &spatial_query,
            delta: vertical_delta,
        });
        let after_vertical_position = transform.translation;
        physics::probe_ground_normal(entity, &config, &mut state, &transform, &spatial_query);
        if record_trace {
            trace.push_controller(FirstPersonControllerTraceFrame {
                entity,
                tick: input.command.tick,
                dt,
                command_move: input.command.move_axis,
                command_look: input.command.look_axis,
                jump_down: input.command.jump_down(),
                crouch_pressed: input.command.crouch_pressed,
                sprint_down: input.command.sprint_down(),
                start_position,
                after_step_latch_position,
                after_input_position,
                after_stance_position,
                intended_horizontal_delta: move_delta,
                horizontal_pushback,
                after_horizontal_position,
                step,
                after_step_position,
                gravity_applied,
                vertical_delta,
                vertical_pushback,
                after_vertical_position,
                grounded: state.grounded,
                ground_contact_ticks: state.ground_contact_ticks,
                climbing: state.climbing,
                ground_normal: state.ground_normal,
                local_speed: body::local_speeds_from_velocity(&state),
                velocity: state.velocity,
            });
        }
        sync_physics_transform(entity, &transform, &mut entity_commands);
        if let Some(mut effects) = effects {
            effects.tick_fixed();
        }
    }
}

fn sync_physics_transform(entity: Entity, transform: &Transform, commands: &mut Commands) {
    commands.entity(entity).insert((
        Position(transform.translation),
        Rotation(transform.rotation),
    ));
}
