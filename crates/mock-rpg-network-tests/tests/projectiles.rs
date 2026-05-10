#[path = "projectiles/math.rs"]
mod math;
#[path = "projectiles/world.rs"]
mod world;

#[test]
fn moving_players_exchange_spell_projectiles_over_delayed_reordered_network() {
    world::moving_players_exchange_spell_projectiles_over_delayed_reordered_network();
}
