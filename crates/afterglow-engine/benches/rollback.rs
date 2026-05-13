use afterglow_engine::network::rollback::{
    CommittedRollbackDomain, DeterministicRollbackBuffer, RollbackCommand, RollbackDomainId,
    RollbackDomainOutputs, RollbackEvent, RollbackEventId, RollbackEventStream, RollbackPolicy,
    replay_bytes,
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    run_case(128, 64, 8_192);
    run_case(1_024, 128, 1_024);
    run_case(8_192, 256, 128);
    run_domain_case(128, 64, 8_192);
    run_domain_case(1_024, 128, 1_024);
    run_domain_case(8_192, 256, 128);
    run_event_case(128, 128, 8_192);
    run_event_case(1_024, 128, 1_024);
    run_event_case(8_192, 128, 128);
    run_event_case(100_000, 128, 8);
}

fn run_case(saved_ticks: u32, command_count: u32, iterations: u32) {
    let commands = commands(command_count);
    let rollback = buffer(saved_ticks);
    let replay = rollback
        .build_replay(saved_ticks / 2, saved_ticks - 1, commands.clone())
        .unwrap();
    let policy = RollbackPolicy {
        max_rollback_ticks: saved_ticks,
        commit_delay_ticks: 4,
    };
    let save_time = measure(iterations, || {
        black_box(buffer(saved_ticks));
    });
    let replay_time = measure(iterations, || {
        black_box(
            rollback
                .build_replay(saved_ticks / 2, saved_ticks - 1, commands.clone())
                .unwrap(),
        );
    });
    let apply_time = measure(iterations, || {
        black_box(replay_bytes(&replay, |state, command| {
            state[0] = state[0].wrapping_add(command.payload[0]);
        }));
    });
    let late_policy_time = measure(iterations, || {
        let result = rollback.build_late_command_replay(
            policy,
            saved_ticks - 1,
            saved_ticks / 2 + 1,
            commands.clone(),
        );
        black_box(result.unwrap());
    });

    println!(
        "rollback saved_ticks={saved_ticks} commands={command_count} save_states={} build_replay={} apply_replay={} late_policy_replay={}",
        fmt(save_time / iterations),
        fmt(replay_time / iterations),
        fmt(apply_time / iterations),
        fmt(late_policy_time / iterations),
    );
}

fn run_event_case(event_count: u32, tick_span: u32, iterations: u32) {
    let initial = events(event_count, tick_span, 0);
    let corrected = events(event_count, tick_span, 1);
    let replace_time = measure(iterations, || {
        let mut stream = RollbackEventStream::default();
        stream.replace_provisional(initial.clone());
        black_box(stream.replace_provisional(corrected.clone()));
    });
    let commit_time = measure(iterations, || {
        let mut stream = RollbackEventStream::default();
        stream.replace_provisional(initial.clone());
        black_box(stream.commit_through(tick_span / 2));
    });

    println!(
        "rollback_events events={event_count} ticks={tick_span} replace_provisional={} commit_half={}",
        fmt(replace_time / iterations),
        fmt(commit_time / iterations),
    );
}

fn run_domain_case(current_tick: u32, command_count: u32, iterations: u32) {
    let policy = RollbackPolicy {
        max_rollback_ticks: current_tick,
        commit_delay_ticks: current_tick / 2,
    };
    let commands = commands(command_count);

    let rebuild_time = measure(iterations, || {
        let mut domain = domain(policy, 0, commands.clone());
        black_box(domain.rebuild_provisional(current_tick, apply_domain_command));
    });
    let promote_time = measure(iterations, || {
        let mut domain = domain(policy, 0, commands.clone());
        black_box(domain.promote_committed(current_tick, apply_domain_command));
    });

    println!(
        "rollback_domain current_tick={current_tick} commands={command_count} rebuild_provisional={} promote_committed={}",
        fmt(rebuild_time / iterations),
        fmt(promote_time / iterations),
    );
}

fn buffer(saved_ticks: u32) -> DeterministicRollbackBuffer {
    let mut rollback = DeterministicRollbackBuffer::default().with_capacity_ticks(saved_ticks);
    for tick in 0..saved_ticks {
        rollback.save_state(tick, vec![(tick & 0xff) as u8; 64]);
    }
    rollback
}

fn commands(count: u32) -> Vec<RollbackCommand> {
    (0..count)
        .map(|tick| RollbackCommand {
            tick,
            source: 0,
            sequence: tick as u64,
            payload: vec![(tick & 0xff) as u8],
        })
        .collect()
}

fn domain(
    policy: RollbackPolicy,
    committed_tick: u32,
    commands: Vec<RollbackCommand>,
) -> CommittedRollbackDomain {
    let mut domain =
        CommittedRollbackDomain::new(RollbackDomainId(1), committed_tick, vec![0; 64], policy);
    for command in commands
        .into_iter()
        .filter(|command| command.tick > committed_tick)
    {
        domain
            .insert_command(policy.max_rollback_ticks, command)
            .unwrap();
    }
    domain
}

#[allow(clippy::ptr_arg)]
fn apply_domain_command(
    state: &mut Vec<u8>,
    command: &RollbackCommand,
    outputs: &mut RollbackDomainOutputs,
) {
    let index = command.tick as usize % state.len();
    state[index] = state[index].wrapping_add(command.payload[0]);
    if command.tick.is_multiple_of(16) {
        outputs.cue(
            command.tick,
            command.tick as u64,
            "bench_cue",
            [state[index]],
        );
    }
}

fn events(count: u32, tick_span: u32, variant: u8) -> Vec<RollbackEvent<u32>> {
    (0..count)
        .map(|index| {
            let tick = (index as u64 * tick_span as u64 / count.max(1) as u64) as u32;
            RollbackEvent::new(
                RollbackEventId::new(RollbackDomainId(1), tick, index as u64),
                ((variant as u32) << 24) | (index & 0x00ff_ffff),
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
