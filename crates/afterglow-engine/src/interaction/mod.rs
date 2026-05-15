use bevy::prelude::*;

pub mod door;
pub mod drawer;
pub mod grab;
pub mod pid;
pub mod raycast;

const MAX_INTERACTION_DISTANCE: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub enum InteractionKind {
    Grabbable {
        mass_mul: f32,
        throw_impulse: f32,
        force_mul: f32,
        torque_mul: f32,
        min_depth: f32,
        max_depth: f32,
        max_leave_linear_speed: f32,
        max_leave_angular_speed: f32,
    },
    HingedDoor {
        move_max_speed: f32,
        move_slow_down_factor: f32,
        move_speed_factor: f32,
        move_throw_impulse: f32,
    },
    SliderDrawer {
        move_max_speed: f32,
        move_slow_down_factor: f32,
        move_speed_factor: f32,
        move_throw_impulse: f32,
    },
}

impl InteractionKind {
    pub const fn default_grabbable() -> Self {
        Self::Grabbable {
            mass_mul: 0.1,
            throw_impulse: 10.0,
            force_mul: 1.0,
            torque_mul: 1.0,
            min_depth: 1.0,
            max_depth: 2.0,
            max_leave_linear_speed: 5.0,
            max_leave_angular_speed: 6.0,
        }
    }

    pub const fn default_hinged_door() -> Self {
        Self::HingedDoor {
            move_max_speed: 13.5,
            move_slow_down_factor: 3.0,
            move_speed_factor: 1.0,
            move_throw_impulse: 6.0,
        }
    }

    pub const fn default_slider_drawer() -> Self {
        Self::SliderDrawer {
            move_max_speed: 8.0,
            move_slow_down_factor: 3.0,
            move_speed_factor: 1.0,
            move_throw_impulse: 4.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub enum FocusCrosshair {
    Default,
    Grab,
    Push,
    Pick,
    LevelDoor,
}

impl Default for FocusCrosshair {
    fn default() -> Self {
        Self::Default
    }
}

#[derive(Component, Clone, Debug, Reflect)]
pub struct InteractionTarget {
    pub kind: InteractionKind,
    pub max_focus_distance: f32,
    pub focus_crosshair: FocusCrosshair,
}

impl Default for InteractionTarget {
    fn default() -> Self {
        Self {
            kind: InteractionKind::Grabbable {
                mass_mul: 0.1,
                throw_impulse: 10.0,
                force_mul: 1.0,
                torque_mul: 1.0,
                min_depth: 1.0,
                max_depth: 2.0,
                max_leave_linear_speed: 5.0,
                max_leave_angular_speed: 6.0,
            },
            max_focus_distance: MAX_INTERACTION_DISTANCE,
            focus_crosshair: FocusCrosshair::Default,
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct PlayerInteractionState {
    pub focus_entity: Option<Entity>,
    pub focus_body: Option<Entity>,
    pub focus_distance: f32,
    pub active_interaction: Option<ActiveInteraction>,
}

#[derive(Clone, Debug)]
pub enum ActiveInteraction {
    Grabbing {
        entity: Entity,
        depth: f32,
        body_offset: Vec3,
        body_rotation_offset: Quat,
    },
    PushingDoor {
        entity: Entity,
        joint_entity: Entity,
        rot_speed: f32,
    },
    SlidingDrawer {
        entity: Entity,
        joint_entity: Entity,
        slide_speed: f32,
    },
}

pub struct AfterglowInteractionPlugin;

impl Plugin for AfterglowInteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerInteractionState>()
            .add_systems(Update, raycast::update_focus)
            .add_systems(Update, door::interact_door_system)
            .add_systems(Update, drawer::interact_drawer_system)
            .add_systems(Update, grab::update_grabbed_objects)
            .add_systems(Update, grab::release_distant_grabbed_objects);

        app.add_systems(Update, door::sticky_door_limits);
        app.add_systems(Update, drawer::sticky_drawer_limits);
    }
}

#[cfg(test)]
mod integration_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interaction_kind_default_grabbable_has_sane_values() {
        let kind = InteractionKind::default_grabbable();
        match kind {
            InteractionKind::Grabbable {
                mass_mul,
                throw_impulse,
                force_mul,
                torque_mul,
                min_depth,
                max_depth,
                max_leave_linear_speed,
                max_leave_angular_speed,
            } => {
                assert!((mass_mul - 0.1).abs() < f32::EPSILON);
                assert!((throw_impulse - 10.0).abs() < f32::EPSILON);
                assert!((force_mul - 1.0).abs() < f32::EPSILON);
                assert!((torque_mul - 1.0).abs() < f32::EPSILON);
                assert!((min_depth - 1.0).abs() < f32::EPSILON);
                assert!((max_depth - 2.0).abs() < f32::EPSILON);
                assert!((max_leave_linear_speed - 5.0).abs() < f32::EPSILON);
                assert!((max_leave_angular_speed - 6.0).abs() < f32::EPSILON);
            }
            _ => panic!("expected grabbable kind"),
        }
    }

    #[test]
    fn interaction_kind_default_door_has_sane_values() {
        let kind = InteractionKind::default_hinged_door();
        match kind {
            InteractionKind::HingedDoor {
                move_max_speed,
                move_slow_down_factor,
                move_speed_factor,
                move_throw_impulse,
            } => {
                assert!((move_max_speed - 13.5).abs() < f32::EPSILON);
                assert!((move_slow_down_factor - 3.0).abs() < f32::EPSILON);
                assert!((move_speed_factor - 1.0).abs() < f32::EPSILON);
                assert!((move_throw_impulse - 6.0).abs() < f32::EPSILON);
            }
            _ => panic!("expected hinged door kind"),
        }
    }

    #[test]
    fn interaction_kind_default_drawer_has_sane_values() {
        let kind = InteractionKind::default_slider_drawer();
        match kind {
            InteractionKind::SliderDrawer {
                move_max_speed,
                move_slow_down_factor,
                move_speed_factor,
                move_throw_impulse,
            } => {
                assert!((move_max_speed - 8.0).abs() < f32::EPSILON);
                assert!((move_slow_down_factor - 3.0).abs() < f32::EPSILON);
                assert!((move_speed_factor - 1.0).abs() < f32::EPSILON);
                assert!((move_throw_impulse - 4.0).abs() < f32::EPSILON);
            }
            _ => panic!("expected slider drawer kind"),
        }
    }

    #[test]
    fn interaction_target_has_default_max_distance() {
        let target = InteractionTarget::default();
        assert!((target.max_focus_distance - MAX_INTERACTION_DISTANCE).abs() < f32::EPSILON);
    }

    #[test]
    fn player_interaction_state_starts_empty() {
        let state = PlayerInteractionState::default();
        assert!(state.focus_entity.is_none());
        assert!(state.focus_body.is_none());
        assert!(state.active_interaction.is_none());
        assert!((state.focus_distance - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn plugin_registers_without_panicking() {
        let mut app = App::new();
        app.add_plugins((
            bevy::MinimalPlugins,
            crate::physics::AfterglowPhysicsPlugin,
            AfterglowInteractionPlugin,
        ));
        app.finish();
        app.cleanup();
        let _ = app.world().resource::<PlayerInteractionState>();
    }
}
