use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::*;

use super::protocol::*;

#[derive(Resource, Default)]
pub struct PlayerName(pub String);

pub fn configure_physics(app: &mut App) {
    // Use standard Avian PhysicsPlugins. LightyearAvianPlugin is not available
    // due to avian3d version incompatibility with lightyear_avian3d.
    // Physics simulation runs server-authoritative; Position/Rotation are
    // replicated via custom wrapper components registered in network.rs.
    app.add_plugins(PhysicsPlugins::default());
}

pub fn spawn_arena(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let floor_material = materials.add(Color::srgb(0.18, 0.2, 0.19));
    let wall_material = materials.add(Color::srgb(0.24, 0.23, 0.2));

    let floor_half = ARENA_HALF + WALL_THICKNESS;
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::from_size(Vec3::new(
            floor_half * 2.0,
            0.4,
            floor_half * 2.0,
        )))),
        MeshMaterial3d(floor_material),
        RigidBody::Static,
        Collider::cuboid(floor_half * 2.0, 0.4, floor_half * 2.0),
        Position::from(Vec3::new(0.0, -0.2, 0.0)),
        Replicate::to_clients(NetworkTarget::All),
    ));

    let wall_up = WALL_HEIGHT * 0.5;
    let half_extent = ARENA_HALF + WALL_THICKNESS * 0.5;

    spawn_wall(
        &mut commands,
        &mut meshes,
        wall_material.clone(),
        Vec3::new(half_extent * 2.0, WALL_HEIGHT, WALL_THICKNESS),
        Vec3::new(0.0, wall_up, -ARENA_HALF - WALL_THICKNESS * 0.5),
    );
    spawn_wall(
        &mut commands,
        &mut meshes,
        wall_material.clone(),
        Vec3::new(half_extent * 2.0, WALL_HEIGHT, WALL_THICKNESS),
        Vec3::new(0.0, wall_up, ARENA_HALF + WALL_THICKNESS * 0.5),
    );
    spawn_wall(
        &mut commands,
        &mut meshes,
        wall_material.clone(),
        Vec3::new(WALL_THICKNESS, WALL_HEIGHT, half_extent * 2.0),
        Vec3::new(-ARENA_HALF - WALL_THICKNESS * 0.5, wall_up, 0.0),
    );
    spawn_wall(
        &mut commands,
        &mut meshes,
        wall_material,
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
        let mat = materials.add(Color::hsla(hue, 0.7, 0.5, 1.0));
        commands.spawn((
            KinematicBox {
                id: i as u32,
                initial_pos: *pos,
            },
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(
                KINEMATIC_BOX_SIZE * 2.0,
            )))),
            MeshMaterial3d(mat),
            RigidBody::Dynamic,
            Collider::cuboid(
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
            ),
            Position::from(*pos),
            Rotation::default(),
            Replicate::to_clients(NetworkTarget::All),
        ));
    }
}

fn spawn_wall(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    size: Vec3,
    translation: Vec3,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::from_size(size))),
        MeshMaterial3d(material),
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        Position::from(translation),
        Replicate::to_clients(NetworkTarget::All),
    ));
}

pub fn spawn_player_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    owner: &str,
    pos: Vec3,
) -> Entity {
    let hue = if owner == "alice" { 200.0 } else { 330.0 };
    let color = Color::hsla(hue, 0.8, 0.5, 1.0);
    let mat = materials.add(color);
    commands
        .spawn((
            PlayerBox {
                owner: owner.to_string(),
            },
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(PLAYER_SIZE * 2.0)))),
            MeshMaterial3d(mat),
            RigidBody::Dynamic,
            Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
            Position::from(pos),
            Rotation::default(),
            MoveInput {
                direction: Vec2::ZERO,
            },
            Replicate::to_clients(NetworkTarget::All),
            PredictionTarget::to_clients(NetworkTarget::All),
        ))
        .id()
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

pub fn spawn_host_player(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    player_name: Res<PlayerName>,
) {
    spawn_player_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        &player_name.0,
        Vec3::new(-5.0, PLAYER_SIZE, 0.0),
    );
}
