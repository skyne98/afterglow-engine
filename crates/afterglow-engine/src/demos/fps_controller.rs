use bevy::{
    log::{info, warn},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

use crate::{
    controller::{
        FirstPersonCameraRig, FirstPersonCameraTraceFrame, FirstPersonController,
        FirstPersonControllerConfig, FirstPersonControllerTrace, FirstPersonControllerTraceFrame,
        FirstPersonStepRejectReason,
    },
    core::schedule::AfterglowSet,
    input::default_gameplay_input_map,
    physics::{PhysicsBody, PhysicsCollider},
};

mod playground;
#[cfg(test)]
use playground::FpsDemoPlaygroundPiece;
use playground::{spawn_crouch_playground, spawn_slopes, spawn_stairs};

pub struct FpsControllerDemoPlugin;

const TRACE_FRAMES: usize = 8192;
const COLLISION_PUSHBACK_EPSILON: f32 = 0.0005;
const CAMERA_OFFSET_JITTER_EPSILON: f32 = 0.006;

#[derive(Component)]
pub(super) struct FpsDemoPlayer;

#[derive(Component)]
struct FpsDemoCamera;

#[derive(Default)]
struct FpsTraceLogCursor {
    controller_index: usize,
    camera_index: usize,
    last_controller: Option<LastControllerFrame>,
    last_camera: Option<LastCameraFrame>,
}

#[derive(Clone, Copy)]
struct LastControllerFrame {
    entity: Entity,
    pushback: Vec3,
}

#[derive(Clone, Copy)]
struct LastCameraFrame {
    camera: Entity,
    base_position: Vec3,
    bob_offset: Vec3,
    final_position: Vec3,
}

impl Plugin for FpsControllerDemoPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            (enable_controller_trace, spawn_scene, capture_cursor),
        )
        .add_systems(
            Update,
            log_controller_trace_events.in_set(AfterglowSet::DebugAndMetrics),
        );
    }
}

fn enable_controller_trace(mut trace: ResMut<FirstPersonControllerTrace>) {
    *trace = FirstPersonControllerTrace::enabled(TRACE_FRAMES);
    info!(
        target: "afterglow::fps_controller_trace",
        "enabled first-person controller trace frames={TRACE_FRAMES}; reproduce jitter and keep this terminal output"
    );
}

fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let floor_material = materials.add(Color::srgb(0.18, 0.2, 0.19));
    let wall_material = materials.add(Color::srgb(0.24, 0.23, 0.2));
    let accent_material = materials.add(Color::srgb(0.45, 0.18, 0.12));
    let stair_material = materials.add(Color::srgb(0.31, 0.32, 0.28));
    let slope_material = materials.add(Color::srgb(0.2, 0.32, 0.38));
    let crouch_material = materials.add(Color::srgb(0.22, 0.18, 0.3));
    let barrier_material = materials.add(Color::srgb(0.5, 0.16, 0.13));
    spawn_box(
        &mut commands,
        &mut meshes,
        floor_material.clone(),
        Vec3::new(28.0, 0.4, 28.0),
        Vec3::new(0.0, -0.2, 0.0),
    );
    spawn_box(
        &mut commands,
        &mut meshes,
        wall_material.clone(),
        Vec3::new(28.0, 3.0, 0.4),
        Vec3::new(0.0, 1.3, -14.0),
    );
    spawn_box(
        &mut commands,
        &mut meshes,
        wall_material.clone(),
        Vec3::new(28.0, 3.0, 0.4),
        Vec3::new(0.0, 1.3, 14.0),
    );
    spawn_box(
        &mut commands,
        &mut meshes,
        wall_material.clone(),
        Vec3::new(0.4, 3.0, 28.0),
        Vec3::new(-14.0, 1.3, 0.0),
    );
    spawn_box(
        &mut commands,
        &mut meshes,
        wall_material,
        Vec3::new(0.4, 3.0, 28.0),
        Vec3::new(14.0, 1.3, 0.0),
    );
    spawn_box(
        &mut commands,
        &mut meshes,
        accent_material,
        Vec3::new(1.5, 0.5, 3.0),
        Vec3::new(2.5, 0.25, -2.0),
    );
    spawn_stairs(
        &mut commands,
        &mut meshes,
        stair_material,
        barrier_material.clone(),
    );
    spawn_slopes(&mut commands, &mut meshes, slope_material, barrier_material);
    spawn_crouch_playground(&mut commands, &mut meshes, crouch_material);

    let config = FirstPersonControllerConfig {
        look_sensitivity: Vec2::new(0.0025, 0.0025),
        ..default()
    };
    let player_transform = Transform::from_xyz(0.0, config.standing_height * 0.5 + 0.05, 4.0);
    let player = commands
        .spawn((
            FpsDemoPlayer,
            FirstPersonController {
                config: config.clone(),
            },
            default_gameplay_input_map(),
            player_transform,
        ))
        .id();

    commands.spawn((
        FpsDemoCamera,
        FirstPersonCameraRig::new(player),
        Camera3d::default(),
        Msaa::Off,
        Transform::from_xyz(0.0, 1.6, 6.0).looking_at(Vec3::new(0.0, 1.2, 0.0), Vec3::Y),
    ));
    commands.spawn((
        PointLight {
            intensity: 3500.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(1.5, 5.0, 3.0),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 2500.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
}

fn spawn_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    size: Vec3,
    translation: Vec3,
) {
    commands.spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(size),
        Mesh3d(meshes.add(Cuboid::from_size(size))),
        MeshMaterial3d(material),
        Transform::from_translation(translation),
    ));
}

fn capture_cursor(mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    let Ok(mut cursor) = cursor.single_mut() else {
        return;
    };
    cursor.visible = false;
    cursor.grab_mode = CursorGrabMode::Locked;
}

fn log_controller_trace_events(
    trace: Res<FirstPersonControllerTrace>,
    mut cursor: Local<FpsTraceLogCursor>,
) {
    if !trace.enabled {
        return;
    }
    if cursor.controller_index > trace.controller_frames.len() {
        cursor.controller_index = 0;
    }
    if cursor.camera_index > trace.camera_frames.len() {
        cursor.camera_index = 0;
    }

    for frame in trace.controller_frames.iter().skip(cursor.controller_index) {
        log_controller_trace_frame(frame, cursor.last_controller);
        cursor.last_controller = Some(LastControllerFrame {
            entity: frame.entity,
            pushback: frame.horizontal_pushback,
        });
    }
    cursor.controller_index = trace.controller_frames.len();

    for frame in trace.camera_frames.iter().skip(cursor.camera_index) {
        log_camera_trace_frame(frame, cursor.last_camera);
        cursor.last_camera = Some(LastCameraFrame {
            camera: frame.camera,
            base_position: frame.base_position,
            bob_offset: frame.bob_offset,
            final_position: frame.final_position,
        });
    }
    cursor.camera_index = trace.camera_frames.len();
}

fn log_controller_trace_frame(
    frame: &FirstPersonControllerTraceFrame,
    previous: Option<LastControllerFrame>,
) {
    if frame.horizontal_pushback.length() > COLLISION_PUSHBACK_EPSILON {
        warn!(
            target: "afterglow::fps_controller_trace",
            "body_collision entity={:?} tick={} pos_start={:?} intent={:?} pushback={:?} pos_after_horizontal={:?} vertical_delta={:?} vertical_pushback={:?} grounded={} climbing={} local_speed={:?}",
            frame.entity,
            frame.tick,
            frame.start_position,
            frame.intended_horizontal_delta,
            frame.horizontal_pushback,
            frame.after_horizontal_position,
            frame.vertical_delta,
            frame.vertical_pushback,
            frame.grounded,
            frame.climbing,
            frame.local_speed,
        );
    }
    if let Some(previous) = previous {
        if previous.entity == frame.entity
            && pushback_flipped(previous.pushback, frame.horizontal_pushback)
        {
            warn!(
                target: "afterglow::fps_controller_trace",
                "body_pushback_flip entity={:?} tick={} previous={:?} current={:?} pos={:?}; likely collision/depenetration jitter",
                frame.entity,
                frame.tick,
                previous.pushback,
                frame.horizontal_pushback,
                frame.after_horizontal_position,
            );
        }
    }
    if frame.step.accepted {
        info!(
            target: "afterglow::fps_controller_trace",
            "step_accepted entity={:?} tick={} lift={} forward_len={} max_step={} pos_after_step={:?} rays={:?}",
            frame.entity,
            frame.tick,
            frame.step.lift,
            frame.step.forward_len,
            frame.step.max_step,
            frame.after_step_position,
            frame.step.rays,
        );
    } else if matches!(
        frame.step.reject_reason,
        FirstPersonStepRejectReason::TooHigh | FirstPersonStepRejectReason::ShapeBlocked
    ) {
        warn!(
            target: "afterglow::fps_controller_trace",
            "step_rejected entity={:?} tick={} reason={:?} forward_len={} max_step={} rays={:?}",
            frame.entity,
            frame.tick,
            frame.step.reject_reason,
            frame.step.forward_len,
            frame.step.max_step,
            frame.step.rays,
        );
    }
}

fn log_camera_trace_frame(frame: &FirstPersonCameraTraceFrame, previous: Option<LastCameraFrame>) {
    let Some(previous) = previous else {
        return;
    };
    if previous.camera != frame.camera {
        return;
    }

    let base_delta = frame.base_position - previous.base_position;
    let bob_delta = frame.bob_offset - previous.bob_offset;
    let final_delta = frame.final_position - previous.final_position;
    if base_delta.length() < CAMERA_OFFSET_JITTER_EPSILON
        && bob_delta.length() > CAMERA_OFFSET_JITTER_EPSILON
    {
        warn!(
            target: "afterglow::fps_controller_trace",
            "camera_offset_motion camera={:?} target={:?} base_delta={:?} bob_delta={:?} final_delta={:?} bobbing={} bob_phase={} bob_amplitude={:?} landing_bounce={}",
            frame.camera,
            frame.target,
            base_delta,
            bob_delta,
            final_delta,
            frame.bobbing,
            frame.bob_phase,
            frame.current_bob_amplitude,
            frame.landing_bounce,
        );
    }
}

fn pushback_flipped(previous: Vec3, current: Vec3) -> bool {
    axis_flipped(previous.x, current.x) || axis_flipped(previous.z, current.z)
}

fn axis_flipped(previous: f32, current: f32) -> bool {
    previous.abs() > COLLISION_PUSHBACK_EPSILON
        && current.abs() > COLLISION_PUSHBACK_EPSILON
        && previous.signum() != current.signum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        controller::AfterglowFirstPersonControllerPlugin, core::AfterglowCorePlugin,
        input::AfterglowInputPlugin, physics::AfterglowPhysicsPlugin,
    };

    #[test]
    fn fps_controller_demo_spawns_player_camera_and_physics_room() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AfterglowCorePlugin,
            AfterglowPhysicsPlugin,
            AfterglowFirstPersonControllerPlugin,
            FpsControllerDemoPlugin,
        ))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
        app.finish();
        app.cleanup();

        app.update();

        let players = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<FirstPersonController>>();
            query.iter(world).count()
        };
        let cameras = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<FpsDemoCamera>>();
            query.iter(world).count()
        };

        assert_eq!(players, 1);
        assert_eq!(cameras, 1);
        assert!(app.world().resource::<FirstPersonControllerTrace>().enabled);
    }

    #[test]
    fn fps_controller_demo_spawns_controller_playground() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            AfterglowCorePlugin,
            AfterglowInputPlugin,
            AfterglowPhysicsPlugin,
            AfterglowFirstPersonControllerPlugin,
            FpsControllerDemoPlugin,
        ))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
        app.finish();
        app.cleanup();

        app.update();

        let world = app.world_mut();
        let mut query = world.query::<(&FpsDemoPlaygroundPiece, &PhysicsBody, &Transform)>();
        let mut stairs = 0;
        let mut forward_path_stairs = 0;
        let mut slopes = 0;
        let mut crouch = 0;
        let mut barriers = 0;
        for (piece, body, transform) in query.iter(world) {
            assert_eq!(*body, PhysicsBody::static_body());
            match piece {
                FpsDemoPlaygroundPiece::Stair => {
                    stairs += 1;
                    if transform.translation.x.abs() < 1.2 && transform.translation.z > 0.0 {
                        forward_path_stairs += 1;
                    }
                }
                FpsDemoPlaygroundPiece::Slope => slopes += 1,
                FpsDemoPlaygroundPiece::Crouch => crouch += 1,
                FpsDemoPlaygroundPiece::Barrier => barriers += 1,
            }
        }

        assert_eq!(stairs, 10);
        assert_eq!(forward_path_stairs, 5);
        assert_eq!(slopes, 3);
        assert_eq!(crouch, 4);
        assert_eq!(barriers, 1);
    }

    #[test]
    fn fps_controller_demo_requests_pointer_lock() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, capture_cursor);
        app.world_mut()
            .spawn((Window::default(), CursorOptions::default(), PrimaryWindow));

        app.update();

        let cursor = app
            .world_mut()
            .query::<&CursorOptions>()
            .single(app.world())
            .unwrap();
        assert!(!cursor.visible);
        assert_eq!(cursor.grab_mode, CursorGrabMode::Locked);
    }
}
