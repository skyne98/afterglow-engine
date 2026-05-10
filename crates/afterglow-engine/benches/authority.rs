use afterglow_engine::{
    input::PlayerCommand,
    network::{
        NetworkPlayerId, PeerId,
        authority::ServerCommandBuffer,
        session::{NetworkSession, PlatformIdentity},
    },
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    run_case(1_024, 128);
    run_case(10_000, 32);
    run_case(100_000, 8);
}

fn run_case(command_count: u64, iterations: u32) {
    let session = session(command_count);
    let commands = commands(command_count);
    let accepted_time = measure(iterations, || {
        let mut buffer = ServerCommandBuffer::default();
        submit_commands(&mut buffer, &session, &commands);
        black_box(buffer);
    });

    let mut duplicate_buffer = ServerCommandBuffer::default();
    submit_commands(&mut duplicate_buffer, &session, &commands);
    let duplicate_time = measure(iterations, || {
        submit_commands(&mut duplicate_buffer, &session, &commands);
        black_box(&duplicate_buffer);
    });

    println!(
        "authority commands={command_count} accept_all={} duplicate_all={}",
        fmt(accepted_time / iterations),
        fmt(duplicate_time / iterations),
    );
}

fn submit_commands(
    buffer: &mut ServerCommandBuffer,
    session: &NetworkSession,
    commands: &[(PeerId, PlayerCommand)],
) {
    buffer.begin_frame();
    for (peer, command) in commands {
        buffer.submit(*peer, command.clone(), session);
    }
}

fn session(player_count: u64) -> NetworkSession {
    let mut session = NetworkSession::default();
    for id in 1..=player_count {
        let peer = PeerId(id);
        session.connect_peer(peer, PlatformIdentity::Local);
        assert_eq!(session.add_player(peer), Some(NetworkPlayerId(id)));
    }
    session
}

fn commands(command_count: u64) -> Vec<(PeerId, PlayerCommand)> {
    (1..=command_count)
        .map(|id| {
            (
                PeerId(id),
                PlayerCommand {
                    player: NetworkPlayerId(id),
                    tick: 1,
                    ..Default::default()
                },
            )
        })
        .collect()
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
