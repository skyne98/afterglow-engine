use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};

use crate::{
    interaction::{
        FocusCrosshair, InteractionKind, InteractionTarget,
        door::HingeJointConfig,
        drawer::PrismaticJointConfig,
    },
    physics::{PhysicsBody, PhysicsCollider},
};

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FpsDemoPlaygroundPiece {
    Stair,
    Slope,
    Crouch,
    Barrier,
    HingedDoor,
    SliderDrawer,
}

pub(super) fn spawn_stairs(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    barrier_material: Handle<StandardMaterial>,
) {
    spawn_stair_run(
        commands,
        meshes,
        material.clone(),
        Vec3::new(0.0, 0.0, 2.5),
        6,
        0.12,
        0.55,
    );
    spawn_stair_run(
        commands,
        meshes,
        material,
        Vec3::new(-4.0, 0.0, 2.5),
        4,
        0.32,
        0.7,
    );
    spawn_tagged_box(
        commands,
        meshes,
        barrier_material,
        Vec3::new(2.2, 0.45, 0.55),
        Transform::from_xyz(-4.0, 0.225, -0.5),
        FpsDemoPlaygroundPiece::Barrier,
    );
}

pub(super) fn spawn_slopes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    barrier_material: Handle<StandardMaterial>,
) {
    spawn_ramp(
        commands,
        meshes,
        material.clone(),
        Vec3::new(5.0, 0.0, 2.0),
        Vec3::new(2.4, 0.35, 4.0),
        12.0_f32.to_radians(),
    );
    spawn_ramp(
        commands,
        meshes,
        material,
        Vec3::new(8.5, 0.0, 2.0),
        Vec3::new(2.4, 0.35, 4.0),
        35.0_f32.to_radians(),
    );
    spawn_ramp(
        commands,
        meshes,
        barrier_material,
        Vec3::new(11.5, 0.0, 2.0),
        Vec3::new(2.4, 0.35, 4.0),
        58.0_f32.to_radians(),
    );
}

pub(super) fn spawn_crouch_playground(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
) {
    let tunnel_z = -6.0;
    spawn_tagged_box(
        commands,
        meshes,
        material.clone(),
        Vec3::new(3.2, 0.18, 4.0),
        Transform::from_xyz(0.0, 1.42, tunnel_z),
        FpsDemoPlaygroundPiece::Crouch,
    );
    for x in [-1.8, 1.8] {
        spawn_tagged_box(
            commands,
            meshes,
            material.clone(),
            Vec3::new(0.25, 1.2, 4.0),
            Transform::from_xyz(x, 0.6, tunnel_z),
            FpsDemoPlaygroundPiece::Crouch,
        );
    }
    spawn_tagged_box(
        commands,
        meshes,
        material,
        Vec3::new(3.0, 0.35, 0.25),
        Transform::from_xyz(0.0, 1.2, tunnel_z - 2.4),
        FpsDemoPlaygroundPiece::Crouch,
    );
}

fn spawn_stair_run(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    origin: Vec3,
    steps: usize,
    rise: f32,
    tread: f32,
) {
    for step in 0..steps {
        let height = rise * (step as f32 + 1.0);
        let z = origin.z - step as f32 * tread;
        spawn_tagged_box(
            commands,
            meshes,
            material.clone(),
            Vec3::new(2.2, height, tread),
            Transform::from_xyz(origin.x, height * 0.5, z),
            FpsDemoPlaygroundPiece::Stair,
        );
    }
}

fn spawn_ramp(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    translation: Vec3,
    size: Vec3,
    angle: f32,
) {
    let height = size.z * angle.tan();
    let mesh = ramp_mesh(size, height);
    let collider = PhysicsCollider::convex_hull(ramp_vertices(size, height));
    commands.spawn((
        FpsDemoPlaygroundPiece::Slope,
        PhysicsBody::static_body(),
        collider,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(material),
        Transform::from_translation(translation),
    ));
}

fn spawn_tagged_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    size: Vec3,
    transform: Transform,
    piece: FpsDemoPlaygroundPiece,
) {
    commands.spawn((
        piece,
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(size),
        Mesh3d(meshes.add(Cuboid::from_size(size))),
        MeshMaterial3d(material),
        transform,
    ));
}

fn ramp_mesh(size: Vec3, height: f32) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, ramp_positions(size, height))
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, ramp_uvs())
    .with_inserted_indices(Indices::U32(
        ramp_indices()
            .into_iter()
            .flat_map(|triangle| triangle.into_iter())
            .collect(),
    ));
    mesh.compute_normals();
    mesh
}

fn ramp_vertices(size: Vec3, height: f32) -> Vec<Vec3> {
    ramp_positions(size, height)
        .into_iter()
        .map(Vec3::from_array)
        .collect()
}

fn ramp_positions(size: Vec3, height: f32) -> Vec<[f32; 3]> {
    let half_width = size.x * 0.5;
    let half_length = size.z * 0.5;
    let bottom = -size.y;
    vec![
        [-half_width, 0.0, half_length],
        [half_width, 0.0, half_length],
        [-half_width, height, -half_length],
        [half_width, height, -half_length],
        [-half_width, bottom, half_length],
        [half_width, bottom, half_length],
        [-half_width, bottom, -half_length],
        [half_width, bottom, -half_length],
    ]
}

fn ramp_indices() -> Vec<[u32; 3]> {
    vec![
        [0, 1, 2],
        [1, 3, 2],
        [4, 6, 5],
        [5, 6, 7],
        [0, 4, 1],
        [1, 4, 5],
        [2, 3, 6],
        [3, 7, 6],
        [0, 2, 4],
        [2, 6, 4],
        [1, 5, 3],
        [3, 5, 7],
    ]
}

fn ramp_uvs() -> Vec<[f32; 2]> {
    vec![
        [0.0, 0.0],
        [1.0, 0.0],
        [0.0, 1.0],
        [1.0, 1.0],
        [0.0, 0.0],
        [1.0, 0.0],
        [0.0, 1.0],
        [1.0, 1.0],
    ]
}

pub(super) fn spawn_hinged_door(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let door_mat = materials.add(Color::srgb(0.45, 0.28, 0.12));
    let frame_mat = materials.add(Color::srgb(0.2, 0.18, 0.15));

    let frame_size = Vec3::new(0.15, 2.4, 0.15);
    let door_size = Vec3::new(0.9, 2.0, 0.05);
    let hinge_pos = Vec3::new(4.0, 1.0, -6.0);

    commands.spawn((
        FpsDemoPlaygroundPiece::Barrier,
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(frame_size),
        Mesh3d(meshes.add(Cuboid::from_size(frame_size))),
        MeshMaterial3d(frame_mat.clone()),
        Transform::from_translation(hinge_pos + Vec3::new(0.0, 0.0, 0.0)),
    ));

    let door_entity = commands
        .spawn((
            FpsDemoPlaygroundPiece::HingedDoor,
            PhysicsBody::dynamic(),
            PhysicsCollider::cuboid(door_size),
            Mesh3d(meshes.add(Cuboid::from_size(door_size))),
            MeshMaterial3d(door_mat),
            Transform::from_translation(hinge_pos + Vec3::new(0.5, 0.0, 0.0)),
            HingeJointConfig::new_door(Vec3::Y),
            InteractionTarget {
                kind: InteractionKind::default_hinged_door(),
                max_focus_distance: 3.0,
                focus_crosshair: FocusCrosshair::LevelDoor,
            },
        ))
        .id();

    let _body1 = commands
        .spawn((
            PhysicsBody::static_body(),
            PhysicsCollider::cuboid(Vec3::new(0.2, 1.2, 0.2)),
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::new(0.2, 1.2, 0.2)))),
            MeshMaterial3d(frame_mat),
            Transform::from_translation(hinge_pos + Vec3::new(-0.1, 0.0, 0.0)),
        ))
        .id();
    let _ = door_entity;
}

pub(super) fn spawn_slider_drawer(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let drawer_mat = materials.add(Color::srgb(0.3, 0.22, 0.13));
    let cabinet_mat = materials.add(Color::srgb(0.2, 0.15, 0.1));

    let cabinet_pos = Vec3::new(-5.0, 0.5, -4.0);

    commands.spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(1.2, 1.6, 0.6)),
        Mesh3d(meshes.add(Cuboid::from_size(Vec3::new(1.2, 1.6, 0.6)))),
        MeshMaterial3d(cabinet_mat),
        Transform::from_translation(cabinet_pos),
    ));

    let drawer_size = Vec3::new(0.8, 0.3, 0.4);
    commands.spawn((
        FpsDemoPlaygroundPiece::SliderDrawer,
        PhysicsBody::dynamic(),
        PhysicsCollider::cuboid(drawer_size),
        Mesh3d(meshes.add(Cuboid::from_size(drawer_size))),
        MeshMaterial3d(drawer_mat),
        Transform::from_translation(cabinet_pos + Vec3::new(0.0, 0.3, -0.5)),
        PrismaticJointConfig::new_drawer(Vec3::Z, 0.4),
        InteractionTarget {
            kind: InteractionKind::default_slider_drawer(),
            max_focus_distance: 3.0,
            focus_crosshair: FocusCrosshair::Push,
        },
    ));
}

pub(super) fn spawn_grabbable_objects(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    _accent_mat: Handle<StandardMaterial>,
) {
    let red_mat = materials.add(Color::srgb(0.7, 0.15, 0.1));
    let blue_mat = materials.add(Color::srgb(0.1, 0.3, 0.7));
    let green_mat = materials.add(Color::srgb(0.15, 0.6, 0.2));

    for (i, (pos, mat)) in [
        (Vec3::new(5.0, 0.3, 2.0), red_mat),
        (Vec3::new(6.0, 0.3, 2.0), blue_mat),
        (Vec3::new(5.5, 0.6, 1.0), green_mat),
    ]
    .into_iter()
    .enumerate()
    {
        let size = 0.2 + i as f32 * 0.1;
        commands.spawn((
            PhysicsBody::dynamic(),
            PhysicsCollider::sphere(size),
            Mesh3d(meshes.add(Sphere::new(size))),
            MeshMaterial3d(mat),
            Transform::from_translation(pos),
            InteractionTarget {
                kind: InteractionKind::default_grabbable(),
                max_focus_distance: 5.0,
                focus_crosshair: FocusCrosshair::Grab,
            },
        ));
    }
}
