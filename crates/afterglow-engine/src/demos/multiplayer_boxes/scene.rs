use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::*;
use std::collections::HashMap;

use super::protocol::*;

use crate::{
    input::default_gameplay_input_map,
    network::{
        AfterglowNetworkContext,
        session::{SessionEvent, SessionLeaveReason, SessionMemberId},
    },
};

#[derive(Resource, Default)]
pub struct PlayerName(pub String);

#[derive(Resource, Default)]
pub struct MemberToPlayer(pub HashMap<SessionMemberId, Entity>);

#[derive(Component)]
pub struct PlayerVisualAttached;

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
        Transform::from_xyz(0.0, -0.2, 0.0),
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
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(KINEMATIC_BOX_SIZE * 2.0)))),
            MeshMaterial3d(mat),
            RigidBody::Dynamic,
            Collider::cuboid(
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
            ),
            Position::from(*pos),
            Rotation::default(),
            LinearVelocity::ZERO,
            Transform::from_translation(*pos),
            Replicate::to_clients(NetworkTarget::All),
            PredictionTarget::to_clients(NetworkTarget::All),
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
        Transform::from_translation(translation),
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
            LinearVelocity::ZERO,
            Transform::from_translation(pos),
            Replicate::to_clients(NetworkTarget::All),
            player_prediction_target(owner),
            player_interpolation_target(owner),
        ))
        .id()
}

fn player_peer(owner: &str) -> Option<PeerId> {
    owner.parse::<u64>().ok().map(PeerId::Netcode)
}

fn player_prediction_target(owner: &str) -> PredictionTarget {
    match player_peer(owner) {
        Some(peer) => PredictionTarget::to_clients(NetworkTarget::Single(peer)),
        None => PredictionTarget::manual(vec![]),
    }
}

fn player_interpolation_target(owner: &str) -> InterpolationTarget {
    match player_peer(owner) {
        Some(peer) => InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(peer)),
        None => InterpolationTarget::to_clients(NetworkTarget::All),
    }
}

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
    players: Query<Entity, (With<PlayerBox>, With<Predicted>, Without<RigidBody>)>,
) {
    // Predicted player physics is now attached in attach_replicated_player_visuals
    // along with visuals, for both predicted and interpolated entities.
    // This system is kept for the PreUpdate slot so physics is ready before
    // FixedUpdate, but the query will be empty if visuals were already attached.
    for entity in &players {
        let transform = Transform::default();
        commands.entity(entity).insert((
            RigidBody::Dynamic,
            Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
            Position::from(transform.translation),
            Rotation::from(transform.rotation),
            LinearVelocity::ZERO,
            lightyear::frame_interpolation::FrameInterpolate::<Transform>::default(),
        ));
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
        let pos = transform.map_or(box_.initial_pos, |transform| transform.translation);
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((
            RigidBody::Dynamic,
            Collider::cuboid(
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
                KINEMATIC_BOX_SIZE * 2.0,
            ),
            Position::from(pos),
            Rotation::from(transform.map_or(Quat::IDENTITY, |transform| transform.rotation)),
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
    context: Option<Res<AfterglowNetworkContext>>,
    players: Query<(
        Entity,
        &PlayerBox,
        Option<&Transform>,
        Has<Predicted>,
        Has<Interpolated>,
        Option<&PlayerVisualAttached>,
    )>,
) {
    let local_owner = context
        .as_deref()
        .and_then(|ctx| ctx.get_connection_status().local_member_owner());
    for (entity, player, transform, predicted, interpolated, attached) in &players {
        if attached.is_some() {
            continue;
        }
        let is_local_owner = local_owner.as_deref() == Some(player.owner.as_str());
        if (is_local_owner && !predicted) || (!is_local_owner && !interpolated) {
            continue;
        }

        let hue = if player.owner == "alice" {
            200.0
        } else {
            330.0
        };
        let pos = transform.map_or(Vec3::ZERO, |t| t.translation);
        let rot = transform.map_or(Quat::IDENTITY, |t| t.rotation);
        // Attach physics to BOTH predicted and interpolated entities so they
        // participate in local collision. Predicted entities are simulated
        // locally; interpolated entities get their Position/Rotation from
        // Lightyear's interpolation, but still need a RigidBody+Collider for
        // collision detection against the local predicted player.
        commands.entity(entity).insert((
            PlayerVisualAttached,
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(PLAYER_SIZE * 2.0)))),
            MeshMaterial3d(materials.add(Color::hsla(hue, 0.8, 0.5, 1.0))),
            RigidBody::Dynamic,
            Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
            Position::from(pos),
            Rotation::from(rot),
            LinearVelocity::ZERO,
        ));
    }
}

pub fn attach_replicated_kinematic_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    boxes: Query<(Entity, &KinematicBox), (With<Predicted>, Without<Mesh3d>)>,
) {
    for (entity, box_) in &boxes {
        let hue = (box_.id as f32) * 45.0;
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(KINEMATIC_BOX_SIZE * 2.0)))),
            MeshMaterial3d(materials.add(Color::hsla(hue, 0.7, 0.5, 1.0))),
        ));
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

pub fn spawn_host_player(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    player_name: Res<PlayerName>,
) {
    let entity = spawn_player_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        &player_name.0,
        Vec3::new(-5.0, PLAYER_SIZE, 0.0),
    );
    commands.entity(entity).insert(default_gameplay_input_map());
}

pub fn spawn_player_on_member_joined(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut map: ResMut<MemberToPlayer>,
    mut events: MessageReader<SessionEvent>,
) {
    for event in events.read() {
        let member = match event {
            SessionEvent::MemberJoined { member, .. } => *member,
            _ => continue,
        };
        if map.0.contains_key(&member) {
            continue;
        }
        let owner = member.as_raw().to_string();
        let idx = map.0.len() as f32;
        let pos = Vec3::new(5.0 + idx * 2.0, PLAYER_SIZE, 0.0);
        let entity = spawn_player_box(&mut commands, &mut meshes, &mut materials, &owner, pos);
        // Tag the entity so the engine's ControlledEntityPlugin can bind
        // ControlledBy automatically when the ClientOf link appears.
        commands
            .entity(entity)
            .insert(crate::network::PlayerOwned::from_member(member));
        map.0.insert(member, entity);
    }
}

pub fn despawn_player_on_member_left(
    mut commands: Commands,
    mut map: ResMut<MemberToPlayer>,
    mut events: MessageReader<SessionEvent>,
) {
    for event in events.read() {
        let (member, reason) = match event {
            SessionEvent::MemberLeft { member, reason, .. } => (*member, reason.clone()),
            _ => continue,
        };
        if reason == SessionLeaveReason::Disconnected || reason == SessionLeaveReason::Left {
            if let Some((_, entity)) = map.0.remove_entry(&member) {
                commands.entity(entity).despawn();
            }
        }
    }
}
