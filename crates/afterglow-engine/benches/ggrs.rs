use ggrs::{Config, GameStateCell, GgrsRequest, InputStatus, PredictRepeatLast, SessionBuilder};
use serde::{Deserialize, Serialize};
use std::{
    hint::black_box,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

#[derive(Copy, Clone, Default, PartialEq, Serialize, Deserialize)]
struct BenchInput {
    axes: [i16; 3],
    buttons: u16,
}

#[derive(Clone)]
struct BenchEntity {
    position: [f32; 3],
    velocity: [f32; 3],
    health: u16,
    flags: u16,
}

#[derive(Clone)]
struct BenchState {
    frame: i32,
    entities: Vec<BenchEntity>,
}

struct BenchConfig;

impl Config for BenchConfig {
    type Input = BenchInput;
    type InputPredictor = PredictRepeatLast;
    type State = BenchState;
    type Address = SocketAddr;
}

fn main() {
    run_case("coordinator_only", 2, 0, 1, 20_000);
    run_case("no_rollback", 2, 0, 10_000, 1_000);
    run_case("rollback_4", 2, 4, 10_000, 512);
    run_case("rollback_8", 2, 8, 10_000, 256);
    run_case("rollback_8_large", 2, 8, 100_000, 32);
}

fn run_case(name: &str, players: usize, check_distance: usize, entities: usize, frames: u32) {
    let mut state = BenchState::new(entities);
    let mut session = SessionBuilder::<BenchConfig>::new()
        .with_num_players(players)
        .unwrap()
        .with_max_prediction_window(check_distance.max(8) + 1)
        .with_check_distance(check_distance)
        .start_synctest_session()
        .unwrap();

    let start = Instant::now();
    let mut saves = 0_u64;
    let mut loads = 0_u64;
    let mut advances = 0_u64;
    let mut request_count = 0_u64;

    for frame in 0..frames {
        for player in 0..players {
            session
                .add_local_input(player, input_for(frame, player))
                .unwrap();
        }
        let requests = session.advance_frame().unwrap();
        request_count += requests.len() as u64;
        handle_requests(&mut state, requests, &mut saves, &mut loads, &mut advances);
        black_box(&state);
    }

    let elapsed = start.elapsed();
    let bytes_per_state = entities * std::mem::size_of::<BenchEntity>();
    println!(
        "ggrs case={name} players={players} check_distance={check_distance} entities={entities} state_bytes={} frames={frames} requests={} saves={saves} loads={loads} advances={advances} total={} avg_frame={} avg_request={}",
        bytes_per_state,
        request_count,
        fmt(elapsed),
        fmt(elapsed / frames),
        fmt(elapsed / request_count as u32),
    );
}

fn handle_requests(
    state: &mut BenchState,
    requests: Vec<GgrsRequest<BenchConfig>>,
    saves: &mut u64,
    loads: &mut u64,
    advances: &mut u64,
) {
    for request in requests {
        match request {
            GgrsRequest::SaveGameState { cell, frame } => {
                debug_assert_eq!(state.frame, frame);
                save_state(cell, frame, state);
                *saves += 1;
            }
            GgrsRequest::LoadGameState { cell, .. } => {
                *state = cell.load().expect("GGRS requested a missing saved state");
                *loads += 1;
            }
            GgrsRequest::AdvanceFrame { inputs } => {
                advance_state(state, &inputs);
                *advances += 1;
            }
        }
    }
}

fn save_state(cell: GameStateCell<BenchState>, frame: i32, state: &BenchState) {
    cell.save(frame, Some(state.clone()), Some(checksum(state)));
}

fn advance_state(state: &mut BenchState, inputs: &[(BenchInput, InputStatus)]) {
    let mut impulse = [0.0_f32; 3];
    for (input, status) in inputs {
        if *status == InputStatus::Disconnected {
            continue;
        }
        impulse[0] += input.axes[0] as f32 * 0.0001;
        impulse[1] += input.axes[1] as f32 * 0.0001;
        impulse[2] += input.axes[2] as f32 * 0.0001;
    }
    for entity in &mut state.entities {
        entity.velocity[0] = (entity.velocity[0] + impulse[0]) * 0.98;
        entity.velocity[1] = (entity.velocity[1] + impulse[1]) * 0.98;
        entity.velocity[2] = (entity.velocity[2] + impulse[2]) * 0.98;
        entity.position[0] += entity.velocity[0];
        entity.position[1] += entity.velocity[1];
        entity.position[2] += entity.velocity[2];
    }
    state.frame += 1;
}

impl BenchState {
    fn new(entities: usize) -> Self {
        let entities = (0..entities)
            .map(|index| BenchEntity {
                position: [index as f32, (index % 97) as f32, (index % 31) as f32],
                velocity: [0.01, 0.02, 0.03],
                health: 100,
                flags: (index & 0xffff) as u16,
            })
            .collect();
        Self { frame: 0, entities }
    }
}

fn input_for(frame: u32, player: usize) -> BenchInput {
    BenchInput {
        axes: [
            ((frame + player as u32) % 7) as i16 - 3,
            ((frame * 3 + player as u32) % 11) as i16 - 5,
            ((frame * 5 + player as u32) % 13) as i16 - 6,
        ],
        buttons: ((frame as usize + player) & 0xf) as u16,
    }
}

fn checksum(state: &BenchState) -> u128 {
    let mut hash = state.frame as u128;
    for entity in state.entities.iter().step_by(16) {
        hash = hash
            .wrapping_mul(16_777_619)
            .wrapping_add(entity.position[0].to_bits() as u128)
            .wrapping_add(entity.position[1].to_bits() as u128)
            .wrapping_add(entity.position[2].to_bits() as u128)
            .wrapping_add(entity.health as u128)
            .wrapping_add(entity.flags as u128);
    }
    hash
}

fn fmt(duration: Duration) -> String {
    if duration.as_micros() >= 1_000 {
        format!("{:.3}ms", duration.as_secs_f64() * 1_000.0)
    } else if duration.as_nanos() >= 1_000 {
        format!("{:.3}us", duration.as_nanos() as f64 / 1_000.0)
    } else {
        format!("{}ns", duration.as_nanos())
    }
}

fn _addr(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}
