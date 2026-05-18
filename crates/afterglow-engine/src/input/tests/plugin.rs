use super::*;

#[test]
fn leafwing_plugin_registers_resources() {
    let mut app = unit_app();
    app.add_plugins(bevy::input::InputPlugin);
    app.add_plugins(AfterglowLeafwingPlugin::default());
    app.update();
}

#[test]
fn plugin_adds_input_manager_systems() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        AfterglowLeafwingPlugin::default(),
    ));
    app.update();
}

#[test]
fn action_state_component_works_with_input_map() {
    let mut app = unit_app();
    app.add_plugins(bevy::input::InputPlugin);
    app.add_plugins(AfterglowLeafwingPlugin::default());

    let map = default_gameplay_input_map();
    let entity = app.world_mut().spawn(map).id();

    app.update();

    let action_state = app
        .world()
        .get::<leafwing_input_manager::action_state::ActionState<AfterglowAction>>(entity);
    assert!(action_state.is_some());
}
