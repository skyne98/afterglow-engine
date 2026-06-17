//! Fork of `lightyear_avian3d` for Avian 0.6, Transform mode only.
//!
//! The upstream crate (`lightyear_avian3d` 0.26.4) depends on `avian3d = 0.5`,
//! while Afterglow uses `avian3d = 0.6.1`. This crate ports the
//! `AvianReplicationMode::Transform` branch of the upstream plugin, adapting
//! the minor API differences (0.6 removed the `AsF32` trait and
//! `Position::f32`).
//!
//! Only Transform mode is supported because the `Position` and
//! `PositionButInterpolateTransform` modes require `Diffable` impls for
//! Avian's `Position`/`Rotation`, which are gated behind
//! `lightyear_replication/avian3d` (which also depends on Avian 0.5) and cannot
//! be implemented locally due to the orphan rule.

#![allow(clippy::type_complexity)]

use avian3d::{physics_transform::*, prelude::*};
use bevy::{
    ecs::schedule::ScheduleLabel,
    prelude::*,
    transform::systems::{mark_dirty_trees, propagate_parent_transforms, sync_simple_transforms},
};
use lightyear::{frame_interpolation::FrameInterpolationSystems, prelude::*};

/// Plugin that integrates Avian 0.6 with Lightyear for networked physics
/// replication in Transform mode.
///
/// In Transform mode:
/// - `Transform` is the networked/predicted/corrected component.
/// - Avian's `Position`/`Rotation` are local physics internals, synced to/from
///   `Transform` in `FixedPostUpdate`.
/// - Physics history and visual interpolation operate on `Transform`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AfterglowAvianPlugin {
    /// If true, lightyear will update the way avian syncs (Position/Rotation
    /// <-> Transform) are handled.
    ///
    /// Disable if you are an advanced user and want to handle the syncs
    /// manually.
    pub update_syncs_manually: bool,
}

impl Plugin for AfterglowAvianPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhysicsTransformConfig>();

        if !self.update_syncs_manually {
            // Need to run TransformToPosition in FixedPostUpdate since Avian
            // uses Position internally but the user operates on Transform.
            Self::sync_transform_to_position(app, FixedPostUpdate);
            Self::sync_position_to_transform(app, FixedPostUpdate);

            // Make sure the child collider's Position is updated before running
            // PositionToTransform (otherwise the child's Position would not be
            // correct when running PositionToTransform).
            app.add_systems(
                FixedPostUpdate,
                Self::update_child_collider_position
                    .in_set(PhysicsTransformSystems::PositionToTransform)
                    .before(position_to_transform),
            );
        }

        // Proper ordering: TransformToPosition -> physics -> PositionToTransform
        // -> save history -> frame interpolation status.
        app.configure_sets(
            FixedPostUpdate,
            (
                // TransformToPosition
                PhysicsSystems::Prepare,
                // update physics
                PhysicsSystems::StepSimulation,
                // sync updated Position to Transform
                PhysicsSystems::Writeback,
                (
                    // save the new Transform values in the prediction history
                    PredictionSystems::UpdateHistory,
                    // save the values for visual interpolation
                    FrameInterpolationSystems::Update,
                ),
            )
                .chain(),
        );
        app.configure_sets(
            PostUpdate,
            (
                FrameInterpolationSystems::Interpolate,
                // We don't want the correction to be overwritten by FrameInterpolation
                RollbackSystems::VisualCorrection,
                TransformSystems::Propagate,
            )
                .chain(),
        );
    }
}

impl AfterglowAvianPlugin {
    fn sync_transform_to_position(app: &mut App, schedule: impl ScheduleLabel) {
        let schedule = schedule.intern();
        // Also add the system ordering for FixedPostUpdate (for
        // ColliderTransformPlugin)
        app.configure_sets(
            FixedPostUpdate,
            (
                PhysicsTransformSystems::Propagate,
                PhysicsTransformSystems::TransformToPosition,
            )
                .chain()
                .in_set(PhysicsSystems::Prepare),
        );
        // Manually propagate Transform to GlobalTransform before running physics
        app.configure_sets(
            schedule,
            (
                PhysicsTransformSystems::Propagate,
                PhysicsTransformSystems::TransformToPosition,
            )
                .chain()
                .in_set(PhysicsSystems::Prepare),
        );
        app.add_systems(
            schedule,
            (
                mark_dirty_trees,
                propagate_parent_transforms,
                sync_simple_transforms,
            )
                .chain()
                .in_set(PhysicsTransformSystems::Propagate)
                .run_if(|config: Res<PhysicsTransformConfig>| config.propagate_before_physics),
        );
        app.add_systems(
            schedule,
            transform_to_position
                .in_set(PhysicsTransformSystems::TransformToPosition)
                .run_if(|config: Res<PhysicsTransformConfig>| config.transform_to_position),
        );
    }

    fn sync_position_to_transform(app: &mut App, schedule: impl ScheduleLabel) {
        if app
            .world()
            .resource::<PhysicsTransformConfig>()
            .position_to_transform
        {
            // Make sure that PositionToTransform sync also runs for Interpolated entities
            app.register_required_components::<Position, ApplyPosToTransform>();
            app.register_required_components::<Rotation, ApplyPosToTransform>();
        }
        let schedule = schedule.intern();

        app.configure_sets(
            FixedPostUpdate,
            PhysicsTransformSystems::PositionToTransform.in_set(PhysicsSystems::Writeback),
        );
        app.configure_sets(
            schedule,
            PhysicsTransformSystems::PositionToTransform.in_set(PhysicsSystems::Writeback),
        );
        app.add_systems(
            schedule,
            (position_to_transform, Self::add_transform)
                .in_set(PhysicsTransformSystems::PositionToTransform)
                .run_if(|config: Res<PhysicsTransformConfig>| config.position_to_transform),
        );
    }

    /// Add Transform only when Position/Rotation are both present and Transform
    /// is not.
    ///
    /// This is necessary because the PositionToTransform systems require
    /// `Transform`.
    ///
    /// - We cannot run this as an observer because the `ChildOf` component
    ///   might be inserted after Position/Rotation.
    /// - We cannot add Transform::default because if the entity is spawned in
    ///   PreUpdate, the TransformToPosition will overwrite the correct
    ///   Position/Rotation.
    /// - We cannot just add GlobalTransform because the PositionToTransform
    ///   systems requires the `Transform` component to be present.
    /// - Therefore we try to compute the correct `Transform`.
    fn add_transform(
        query: Query<(Entity, Ref<Position>, Ref<Rotation>, Option<&ChildOf>), Without<Transform>>,
        parents: Query<(
            Option<&GlobalTransform>,
            Option<&Position>,
            Option<&Rotation>,
        )>,
        mut commands: Commands,
    ) {
        query.iter().for_each(|(entity, pos, rot, parent)| {
            if !(pos.is_added() || rot.is_added()) {
                return;
            }
            let mut transform = Transform::default();
            if let Some(&ChildOf(parent)) = parent {
                if let Ok((parent_global_transform, parent_pos, parent_rot)) = parents.get(parent) {
                    // Compute the global transform of the parent using its Position and Rotation
                    let parent_transform = parent_global_transform
                        .unwrap_or(&GlobalTransform::IDENTITY)
                        .compute_transform();
                    // Avian 0.6: Position.0 is already Vec3 (f32 feature)
                    let parent_pos = parent_pos.map_or(parent_transform.translation, |p| p.0);
                    // Avian 0.6: Rotation.0 is already Quat (f32 feature)
                    let parent_rot = parent_rot.map_or(parent_transform.rotation, |r| r.0);
                    let parent_scale = parent_transform.scale;
                    let parent_transform = Transform::from_translation(parent_pos)
                        .with_rotation(parent_rot)
                        .with_scale(parent_scale);

                    // The new local transform of the child body, computed from
                    // the its global transform and its parents global transform
                    let new_transform = GlobalTransform::from(
                        Transform::from_translation(pos.0).with_rotation(rot.0),
                    )
                    .reparented_to(&GlobalTransform::from(parent_transform));

                    transform.translation = new_transform.translation;
                    transform.rotation = new_transform.rotation;
                }
            } else {
                transform.translation = pos.0;
                transform.rotation = rot.0;
            }

            commands.entity(entity).insert(transform);
        });
    }

    /// Update the child's Position based on the parent's Position and the
    /// child's Transform.
    ///
    /// In Avian, this is done in PhysicsSystems::First, so we need to manually
    /// run it after PhysicsSystems run to have an accurate Position of child
    /// entities for replication.
    pub fn update_child_collider_position(
        mut collider_query: Query<
            (
                &ColliderTransform,
                &mut Position,
                &mut Rotation,
                &ColliderOf,
            ),
            Without<RigidBody>,
        >,
        rb_query: Query<(&Position, &Rotation), (With<RigidBody>, With<Children>)>,
    ) {
        for (collider_transform, mut position, mut rotation, collider_of) in &mut collider_query {
            let Ok((rb_pos, rb_rot)) = rb_query.get(collider_of.body) else {
                continue;
            };

            position.0 = rb_pos.0 + rb_rot.0 * collider_transform.translation;
            // Avian 0.6: Rotation.0 is Quat, direct multiply + normalize
            rotation.0 = (rb_rot.0 * collider_transform.rotation.0).normalize();
        }
    }
}

pub mod prelude {
    pub use crate::AfterglowAvianPlugin;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_builds_without_panicking() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.add_plugins(AfterglowAvianPlugin::default());
    }

    #[test]
    fn plugin_adds_physics_transform_config() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.add_plugins(AfterglowAvianPlugin::default());
        assert!(
            app.world()
                .contains_resource::<avian3d::physics_transform::PhysicsTransformConfig>()
        );
    }
}
