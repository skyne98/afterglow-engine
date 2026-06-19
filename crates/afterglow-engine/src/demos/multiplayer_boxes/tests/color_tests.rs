use bevy::prelude::*;

use super::*;

#[test]
fn kinematic_box_material_syncs_to_stable_id_hue() {
    let mut app = test_app();
    let stable_id = StableEntityId::new(42);
    let wrong_spawn_hue = 0.0;
    assert_ne!(wrong_spawn_hue, kinematic_box_hue(stable_id));

    app.world_mut().spawn((
        KinematicBox {
            initial_pos: Vec3::new(2.0, 0.5, 0.0),
        },
        stable_id,
        BoxMaterial {
            base_hue: wrong_spawn_hue,
        },
    ));
    app.add_systems(Update, sync_kinematic_box_materials);
    app.update();

    let box_mat = app
        .world_mut()
        .query::<&BoxMaterial>()
        .single(app.world())
        .unwrap();
    assert_eq!(box_mat.base_hue, kinematic_box_hue(stable_id));
}
