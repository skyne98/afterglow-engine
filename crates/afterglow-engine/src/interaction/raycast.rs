use avian3d::prelude::{SpatialQuery, SpatialQueryFilter};
use bevy::prelude::*;

use super::{InteractionTarget, PlayerInteractionState};

const FOCUS_RAY_LENGTH: f32 = 20.0;

pub fn update_focus(
    mut state: ResMut<PlayerInteractionState>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    target_query: Query<(&InteractionTarget, &GlobalTransform)>,
    spatial_query: SpatialQuery,
) {
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(camera_transform, Vec2::new(0.5, 0.5)) else {
        return;
    };

    let filter = SpatialQueryFilter::default();
    let closest = spatial_query.cast_ray(
        ray.origin,
        ray.direction,
        FOCUS_RAY_LENGTH,
        true,
        &filter,
    );

    let Some(hit) = closest else {
        state.focus_entity = None;
        state.focus_body = None;
        return;
    };

    let hit_entity = hit.entity;
    let distance = hit.distance;
    let Ok((target, _)) = target_query.get(hit_entity) else {
        state.focus_entity = None;
        state.focus_body = None;
        return;
    };

    if distance > target.max_focus_distance {
        state.focus_entity = None;
        state.focus_body = None;
        return;
    }

    state.focus_entity = Some(hit_entity);
    state.focus_body = Some(hit_entity);
    state.focus_distance = distance;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{physics::AfterglowPhysicsPlugin, testing::unit_app};

    #[test]
    fn focus_returns_none_when_no_camera() {
        let mut app = unit_app();
        app.add_plugins(AfterglowPhysicsPlugin);
        app.add_systems(Update, update_focus);
        app.init_resource::<PlayerInteractionState>();
        app.update();
        let state = app.world().resource::<PlayerInteractionState>();
        assert!(state.focus_entity.is_none());
    }

    #[test]
    fn focus_returns_none_when_no_interactable_targets() {
        let mut app = unit_app();
        app.add_plugins(AfterglowPhysicsPlugin);
        app.add_systems(Update, update_focus);
        app.init_resource::<PlayerInteractionState>();
        app.world_mut().spawn((
            Camera3d::default(),
            GlobalTransform::IDENTITY,
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));
        app.update();
        let state = app.world().resource::<PlayerInteractionState>();
        assert!(state.focus_entity.is_none());
    }
}
