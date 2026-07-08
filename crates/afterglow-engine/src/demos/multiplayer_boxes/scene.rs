use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

use super::protocol::*;

use crate::{
    core::identity::{AutoStableEntityId, StableEntityId},
    input::AfterglowAction,
};

/// The configured player name (used as a fallback for local-player detection
/// Marker inserted on a player entity once its local-only presentation
/// components (mesh, material) have been attached.
#[derive(Component)]
pub struct PlayerVisualAttached;

/// Tracks the base hue of a box so we can restore it after highlighting.
#[derive(Component)]
pub struct BoxMaterial {
    pub base_hue: f32,
}

// ---------------------------------------------------------------------------
// Arena
// ---------------------------------------------------------------------------

pub fn spawn_arena(mut commands: Commands) {
    let floor_half = ARENA_HALF + WALL_THICKNESS;
    commands.spawn((
        RigidBody::Static,
        Collider::cuboid(floor_half * 2.0, 0.4, floor_half * 2.0),
        Position::from(Vec3::new(0.0, -0.2, 0.0)),
        Transform::from_xyz(0.0, -0.2, 0.0),
        Replicate::to_clients(NetworkTarget::All),
    ));

    let wall_up = WALL_HEIGHT * 0.5;
    let half_extent = ARENA_HALF + WALL_THICKNESS * 0.5;

    spawn_wall(
        &mut commands,
        Vec3::new(half_extent * 2.0, WALL_HEIGHT, WALL_THICKNESS),
        Vec3::new(0.0, wall_up, -ARENA_HALF - WALL_THICKNESS * 0.5),
    );
    spawn_wall(
        &mut commands,
        Vec3::new(half_extent * 2.0, WALL_HEIGHT, WALL_THICKNESS),
        Vec3::new(0.0, wall_up, ARENA_HALF + WALL_THICKNESS * 0.5),
    );
    spawn_wall(
        &mut commands,
        Vec3::new(WALL_THICKNESS, WALL_HEIGHT, half_extent * 2.0),
        Vec3::new(-ARENA_HALF - WALL_THICKNESS * 0.5, wall_up, 0.0),
    );
    spawn_wall(
        &mut commands,
        Vec3::new(WALL_THICKNESS, WALL_HEIGHT, half_extent * 2.0),
        Vec3::new(ARENA_HALF + WALL_THICKNESS * 0.5, wall_up, 0.0),
    );

    let box_positions = [
        Vec3::new(-4.0, KINEMATIC_BOX_SIZE, -4.0),
        Vec3::new(4.0, KINEMATIC_BOX_SIZE, -4.0),
        Vec3::new(-4.0, KINEMATIC_BOX_SIZE, 4.0),
        Vec3::new(4.0, KINEMATIC_BOX_SIZE, 4.0),
        Vec3::new(-2.0, KINEMATIC_BOX_SIZE, 0.0),
        Vec3::new(2.0, KINEMATIC_BOX_SIZE, 0.0),
        Vec3::new(0.0, KINEMATIC_BOX_SIZE, -2.0),
        Vec3::new(0.0, KINEMATIC_BOX_SIZE, 2.0),
    ];

    for (i, pos) in box_positions.iter().enumerate() {
        let hue = (i as f32) * 45.0;
        commands.spawn((
            KinematicBox { initial_pos: *pos },
            AutoStableEntityId,
            BoxMaterial { base_hue: hue },
            RigidBody::Dynamic,
            Collider::cuboid(
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
            ),
            Position::from(*pos),
            Rotation::default(),
            LinearVelocity::ZERO,
            LockedAxes::ROTATION_LOCKED,
            Transform::from_translation(*pos),
            Replicate::to_clients(NetworkTarget::All),
            PredictionTarget::to_clients(NetworkTarget::All),
        ));
    }
}

fn spawn_wall(commands: &mut Commands, size: Vec3, translation: Vec3) {
    commands.spawn((
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        Position::from(translation),
        Transform::from_translation(translation),
        Replicate::to_clients(NetworkTarget::All),
    ));
}

/// Spawn a player box without Replicate (used by tests).
pub fn spawn_player_box(commands: &mut Commands, owner: &str, pos: Vec3) -> Entity {
    commands
        .spawn((
            PlayerBox {
                owner: owner.to_string(),
            },
            RigidBody::Dynamic,
            Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
            Position::from(pos),
            Rotation::default(),
            LinearVelocity::ZERO,
            LockedAxes::ROTATION_LOCKED,
            Transform::from_translation(pos),
            ActionState::<AfterglowAction>::default(),
        ))
        .id()
}

// ---------------------------------------------------------------------------
// Client visuals
// ---------------------------------------------------------------------------

pub fn spawn_client_arena_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let floor_material = materials.add(Color::srgb(0.18, 0.2, 0.19));
    let wall_material = materials.add(Color::srgb(0.24, 0.23, 0.2));
    let floor_half = ARENA_HALF + WALL_THICKNESS;
    spawn_visual_cuboid(
        &mut commands,
        &mut meshes,
        floor_material,
        Vec3::new(floor_half * 2.0, 0.4, floor_half * 2.0),
        Vec3::new(0.0, -0.2, 0.0),
    );

    let wall_up = WALL_HEIGHT * 0.5;
    let half_extent = ARENA_HALF + WALL_THICKNESS * 0.5;
    for (size, translation) in [
        (
            Vec3::new(half_extent * 2.0, WALL_HEIGHT, WALL_THICKNESS),
            Vec3::new(0.0, wall_up, -ARENA_HALF - WALL_THICKNESS * 0.5),
        ),
        (
            Vec3::new(half_extent * 2.0, WALL_HEIGHT, WALL_THICKNESS),
            Vec3::new(0.0, wall_up, ARENA_HALF + WALL_THICKNESS * 0.5),
        ),
        (
            Vec3::new(WALL_THICKNESS, WALL_HEIGHT, half_extent * 2.0),
            Vec3::new(-ARENA_HALF - WALL_THICKNESS * 0.5, wall_up, 0.0),
        ),
        (
            Vec3::new(WALL_THICKNESS, WALL_HEIGHT, half_extent * 2.0),
            Vec3::new(ARENA_HALF + WALL_THICKNESS * 0.5, wall_up, 0.0),
        ),
    ] {
        spawn_visual_cuboid(
            &mut commands,
            &mut meshes,
            wall_material.clone(),
            size,
            translation,
        );
    }
}

pub fn attach_predicted_player_physics(
    mut commands: Commands,
    players: Query<
        (Entity, Option<&Transform>, Has<LinearVelocity>),
        (With<PlayerBox>, With<Predicted>, Without<RigidBody>),
    >,
) {
    for (entity, transform, has_velocity) in &players {
        let Some(transform) = transform else {
            continue;
        };
        let pos = transform.translation;
        let rot = transform.rotation;
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((
            RigidBody::Dynamic,
            Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
            Position::from(pos),
            Rotation::from(rot),
            LockedAxes::ROTATION_LOCKED,
            lightyear::frame_interpolation::FrameInterpolate::<Transform>::default(),
        ));
        if !has_velocity {
            entity_commands.insert(LinearVelocity::ZERO);
        }
    }
}

pub fn attach_predicted_kinematic_physics(
    mut commands: Commands,
    boxes: Query<
        (
            Entity,
            &KinematicBox,
            Option<&Transform>,
            Has<LinearVelocity>,
        ),
        (With<Predicted>, Without<RigidBody>),
    >,
) {
    for (entity, box_, transform, has_velocity) in &boxes {
        let pos = transform.map_or(box_.initial_pos, |t| t.translation);
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((
            RigidBody::Dynamic,
            Collider::cuboid(
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
            ),
            Position::from(pos),
            Rotation::from(transform.map_or(Quat::IDENTITY, |t| t.rotation)),
            LockedAxes::ROTATION_LOCKED,
            lightyear::frame_interpolation::FrameInterpolate::<Transform>::default(),
        ));
        if !has_velocity {
            entity_commands.insert(LinearVelocity::ZERO);
        }
    }
}

pub fn attach_replicated_player_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    players: Query<
        (
            Entity,
            &PlayerBox,
            Option<&Transform>,
            Option<&PlayerVisualAttached>,
        ),
        With<Predicted>,
    >,
) {
    for (entity, player, transform, attached) in &players {
        if attached.is_some() {
            continue;
        }
        if transform.is_none() {
            continue;
        }

        let hue = if player.owner == "alice" {
            200.0
        } else {
            330.0
        };
        commands.entity(entity).insert((
            PlayerVisualAttached,
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(PLAYER_SIZE * 2.0)))),
            MeshMaterial3d(materials.add(Color::hsla(hue, 0.8, 0.5, 1.0))),
        ));
    }
}

pub fn attach_replicated_kinematic_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    boxes: Query<(Entity, &StableEntityId), (With<KinematicBox>, With<Predicted>, Without<Mesh3d>)>,
) {
    for (entity, stable_id) in &boxes {
        if !stable_id.is_valid() {
            continue;
        }
        let hue = kinematic_box_hue(*stable_id);
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(KINEMATIC_BOX_SIZE * 2.0)))),
            MeshMaterial3d(materials.add(Color::hsla(hue, 0.7, 0.5, 1.0))),
            BoxMaterial { base_hue: hue },
        ));
    }
}

pub fn sync_kinematic_box_materials(
    mut boxes: Query<
        (&StableEntityId, &mut BoxMaterial),
        (With<KinematicBox>, Changed<StableEntityId>),
    >,
) {
    for (stable_id, mut box_mat) in &mut boxes {
        if stable_id.is_valid() {
            box_mat.base_hue = kinematic_box_hue(*stable_id);
        }
    }
}

fn spawn_visual_cuboid(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    size: Vec3,
    translation: Vec3,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::from_size(size))),
        MeshMaterial3d(material),
        Transform::from_translation(translation),
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        Position::from(translation),
    ));
}

pub fn spawn_lights(mut commands: Commands) {
    commands.spawn((
        PointLight {
            intensity: 3500.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(0.0, 8.0, 0.0),
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
