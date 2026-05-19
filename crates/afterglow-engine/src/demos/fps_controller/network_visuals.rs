use bevy::prelude::*;

use super::{FpsDemoNetworkStatus, FpsDemoRemoteAvatar};

#[cfg(feature = "lightyear")]
use super::{
    FPS_DEMO_PLAYER_ID, FpsDemoConnectionState, FpsDemoNetworkRuntime, FpsDemoPlayer,
    FpsDemoPlayerState,
};

#[cfg(feature = "lightyear")]
use crate::{
    core::identity::StableEntityId,
    network::{NetworkTransformInterpolationBuffer, NetworkTransformSample},
};

#[cfg(feature = "lightyear")]
pub(super) fn sync_visible_network_avatars(
    mut commands: Commands,
    mut status: ResMut<FpsDemoNetworkStatus>,
    mut runtime: NonSendMut<FpsDemoNetworkRuntime>,
    mut queries: ParamSet<(
        Query<
            (
                Entity,
                &FpsDemoRemoteAvatar,
                &mut Transform,
                &mut FpsDemoPlayerState,
                Option<&mut NetworkTransformInterpolationBuffer>,
            ),
            Without<FpsDemoPlayer>,
        >,
        Query<
            (
                Entity,
                &StableEntityId,
                &FpsDemoPlayerState,
                Option<&FpsDemoRemoteAvatar>,
                Option<&mut NetworkTransformInterpolationBuffer>,
            ),
            Without<FpsDemoPlayer>,
        >,
        Query<&StableEntityId, With<FpsDemoPlayer>>,
    )>,
) {
    if let Some(local) = runtime.local.as_mut() {
        let replicated = local.replicated_avatar_states();
        status.replicated_avatar_count = replicated.len();
        status.local_player_round_trip = replicated
            .iter()
            .any(|(stable_id, _)| *stable_id == FPS_DEMO_PLAYER_ID);
        let remote_states = replicated
            .into_iter()
            .filter(|(stable_id, _)| *stable_id != FPS_DEMO_PLAYER_ID)
            .collect::<Vec<_>>();
        status.visible_remote_avatar_count = remote_states.len();
        sync_remote_avatar_entities(
            &mut commands,
            &remote_states,
            status.ticks,
            &mut queries.p0(),
        );
        return;
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let has_native_client = runtime.native.local_player_id().is_some();
        if matches!(status.connection, FpsDemoConnectionState::Remote(_))
            || (matches!(status.connection, FpsDemoConnectionState::Server(_)) && has_native_client)
        {
            let controlled_ids = queries.p2().iter().copied().collect::<Vec<_>>();
            status.visible_remote_avatar_count = sync_native_remote_avatar_entities(
                &mut commands,
                &mut queries.p1(),
                runtime.native.local_player_id(),
                &controlled_ids,
                status.ticks,
            );
            return;
        }
    }
    status.visible_remote_avatar_count = queries.p0().iter().count();
}

#[cfg(not(feature = "lightyear"))]
pub(super) fn sync_visible_network_avatars(
    mut status: ResMut<FpsDemoNetworkStatus>,
    avatars: Query<&FpsDemoRemoteAvatar>,
) {
    status.visible_remote_avatar_count = avatars.iter().count();
}

pub(super) fn ensure_remote_avatar_visuals(
    mut commands: Commands,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    avatars: Query<Entity, (With<FpsDemoRemoteAvatar>, Without<Mesh3d>)>,
) {
    if avatars.is_empty() {
        return;
    }
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let mesh = meshes.add(Cuboid::from_size(Vec3::new(0.65, 1.8, 0.65)));
    let material = materials.add(Color::srgb(0.1, 0.45, 0.9));
    for entity in &avatars {
        commands
            .entity(entity)
            .insert((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())));
    }
}

#[cfg(feature = "lightyear")]
fn sync_remote_avatar_entities(
    commands: &mut Commands,
    states: &[(crate::core::identity::StableEntityId, FpsDemoPlayerState)],
    latest_tick: u32,
    avatars: &mut Query<
        (
            Entity,
            &FpsDemoRemoteAvatar,
            &mut Transform,
            &mut FpsDemoPlayerState,
            Option<&mut NetworkTransformInterpolationBuffer>,
        ),
        Without<FpsDemoPlayer>,
    >,
) {
    let mut seen = Vec::new();
    for (stable_id, state) in states {
        seen.push(*stable_id);
        let mut found = false;
        for (entity, avatar, mut transform, mut current_state, interpolation) in avatars.iter_mut()
        {
            if avatar.stable_id == *stable_id {
                apply_avatar_state(
                    commands,
                    entity,
                    &mut transform,
                    interpolation,
                    state,
                    latest_tick,
                );
                *current_state = state.clone();
                found = true;
            }
        }
        if !found {
            let sample = avatar_sample(state);
            let interpolation = NetworkTransformInterpolationBuffer::with_sample(sample);
            commands.spawn((
                FpsDemoRemoteAvatar {
                    stable_id: *stable_id,
                },
                *stable_id,
                state.clone(),
                transform_from_sample(interpolation.sample_delayed(latest_tick).unwrap_or(sample)),
                interpolation,
            ));
        }
    }
    for (entity, avatar, _, _, _) in avatars.iter_mut() {
        if !seen.contains(&avatar.stable_id) {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(feature = "lightyear")]
fn sync_native_remote_avatar_entities(
    commands: &mut Commands,
    states: &mut Query<
        (
            Entity,
            &StableEntityId,
            &FpsDemoPlayerState,
            Option<&FpsDemoRemoteAvatar>,
            Option<&mut NetworkTransformInterpolationBuffer>,
        ),
        Without<FpsDemoPlayer>,
    >,
    local_player_id: Option<StableEntityId>,
    controlled_ids: &[StableEntityId],
    latest_tick: u32,
) -> usize {
    let mut visible = 0;
    for (entity, stable_id, state, avatar, interpolation) in states.iter_mut() {
        if Some(*stable_id) == local_player_id || controlled_ids.contains(stable_id) {
            commands.entity(entity).remove::<(
                FpsDemoRemoteAvatar,
                NetworkTransformInterpolationBuffer,
                Mesh3d,
                MeshMaterial3d<StandardMaterial>,
            )>();
            continue;
        }
        visible += 1;
        let sample = avatar_sample(state);
        let transform = if let Some(mut interpolation) = interpolation {
            interpolation.push_sample(sample);
            transform_from_sample(
                interpolation
                    .sample_delayed(latest_tick.max(sample.tick))
                    .unwrap_or(sample),
            )
        } else {
            let interpolation = NetworkTransformInterpolationBuffer::with_sample(sample);
            let transform = transform_from_sample(
                interpolation
                    .sample_delayed(latest_tick.max(sample.tick))
                    .unwrap_or(sample),
            );
            commands.entity(entity).insert(interpolation);
            transform
        };
        if avatar.is_some() {
            commands.entity(entity).insert(transform);
        } else {
            commands.entity(entity).insert((
                FpsDemoRemoteAvatar {
                    stable_id: *stable_id,
                },
                transform,
            ));
        }
    }
    visible
}

#[cfg(feature = "lightyear")]
fn apply_avatar_state(
    commands: &mut Commands,
    entity: Entity,
    transform: &mut Transform,
    interpolation: Option<Mut<NetworkTransformInterpolationBuffer>>,
    state: &FpsDemoPlayerState,
    latest_tick: u32,
) {
    let sample = avatar_sample(state);
    if let Some(mut interpolation) = interpolation {
        interpolation.push_sample(sample);
        *transform =
            transform_from_sample(interpolation.sample_delayed(latest_tick).unwrap_or(sample));
        return;
    }
    let interpolation = NetworkTransformInterpolationBuffer::with_sample(sample);
    *transform = transform_from_sample(interpolation.sample_delayed(latest_tick).unwrap_or(sample));
    commands.entity(entity).insert(interpolation);
}

#[cfg(feature = "lightyear")]
fn avatar_sample(state: &FpsDemoPlayerState) -> NetworkTransformSample {
    NetworkTransformSample::new(
        state.authoritative_tick,
        state.to_translation(),
        Quat::from_rotation_y(state.yaw_milliradians as f32 / 1000.0),
    )
}

#[cfg(feature = "lightyear")]
fn transform_from_sample(sample: NetworkTransformSample) -> Transform {
    Transform {
        translation: sample.translation,
        rotation: sample.rotation,
        ..default()
    }
}
