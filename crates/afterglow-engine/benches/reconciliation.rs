use afterglow_engine::{
    input::PlayerCommand,
    network::{
        NetworkPlayerId,
        prediction::ClientPredictionBuffer,
        reconciliation::{
            AuthoritativeCorrection, AuthoritativeUpdateSource, ClientReconciliationQueue,
        },
    },
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    run_case(1, 256, 4_096);
    run_case(8, 256, 1_024);
    run_case(64, 128, 256);
}

fn run_case(players: u64, commands_per_player: u32, iterations: u32) {
    let corrections = (1..=players)
        .map(|player| AuthoritativeCorrection {
            player: NetworkPlayerId(player),
            tick: commands_per_player / 2,
            source: AuthoritativeUpdateSource::Correction,
        })
        .collect::<Vec<_>>();
    let reconcile_time = measure(iterations, || {
        let mut prediction = prediction(players, commands_per_player);
        let mut queue = ClientReconciliationQueue::default();
        for correction in &corrections {
            queue.reconcile(&mut prediction, *correction);
        }
        black_box(queue);
        black_box(prediction);
    });

    println!(
        "reconciliation players={players} commands_per_player={commands_per_player} corrections={} reconcile_half={}",
        corrections.len(),
        fmt(reconcile_time / iterations),
    );
}

fn prediction(players: u64, commands_per_player: u32) -> ClientPredictionBuffer {
    let mut prediction = ClientPredictionBuffer::default();
    for player in 1..=players {
        for tick in 1..=commands_per_player {
            prediction.record(PlayerCommand {
                player: NetworkPlayerId(player),
                tick,
                ..Default::default()
            });
        }
    }
    prediction
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
