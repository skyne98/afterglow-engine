use afterglow_engine::{
    input::PlayerCommand,
    network::{NetworkPlayerId, prediction::ClientPredictionBuffer},
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    run_case(1, 256, 4_096);
    run_case(4, 512, 1_024);
    run_case(64, 128, 256);
}

fn run_case(players: u64, commands_per_player: u32, iterations: u32) {
    let commands = commands(players, commands_per_player);
    let record_time = measure(iterations, || {
        let mut buffer = ClientPredictionBuffer::default();
        for command in &commands {
            buffer.record(command.clone());
        }
        black_box(buffer);
    });

    let rebase_tick = commands_per_player / 2;
    let rebase_time = measure(iterations, || {
        let mut buffer = ClientPredictionBuffer::default();
        for command in &commands {
            buffer.record(command.clone());
        }
        for player in 1..=players {
            black_box(buffer.replay_after(NetworkPlayerId(player), rebase_tick));
        }
    });

    println!(
        "prediction players={players} commands_per_player={commands_per_player} total={} record_all={} rebase_half={}",
        commands.len(),
        fmt(record_time / iterations),
        fmt(rebase_time / iterations),
    );
}

fn commands(players: u64, commands_per_player: u32) -> Vec<PlayerCommand> {
    let mut commands = Vec::with_capacity(players as usize * commands_per_player as usize);
    for player in 1..=players {
        for tick in 1..=commands_per_player {
            commands.push(PlayerCommand {
                player: NetworkPlayerId(player),
                tick,
                ..Default::default()
            });
        }
    }
    commands
}

fn measure(iterations: u32, mut f: impl FnMut()) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    start.elapsed()
}

fn fmt(duration: Duration) -> String {
    if duration.as_micros() >= 1_000 {
        format!("{:.3}ms", duration.as_secs_f64() * 1_000.0)
    } else {
        format!("{}us", duration.as_micros())
    }
}
