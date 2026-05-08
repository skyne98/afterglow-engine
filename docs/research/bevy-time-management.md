# Bevy 0.18 Time Management — Deep Dive

## `Time<T>` — The Generic Clock

```rust
pub struct Time<T: Default = ()> {
    context: T,
    delta: Duration,
    elapsed: Duration,
    // ... delta_secs, elapsed_secs, wrapping variants
}
```

`Time<T>` is a **generic resource** where `T` is a zero-sized marker. Each specialization
adds its own behavior while the core fields (delta, elapsed) are shared.

**Built-in clocks:**

| Marker | Behavior | Used For |
|---|---|---|
| `Time<Real>` | Wall clock, never pauses or scales | Diagnostics, UI animations that must keep running |
| `Time<Virtual>` | Can be paused, scaled (speed), max delta clamped | **Default game time** — most systems use this |
| `Time<Fixed>` | Fixed timestep, advances in discrete steps | Physics, anything needing stable intervals |
| `Time<()>` (aka `Time`) | Copied from Virtual or Fixed each frame | **Most systems use this** — works in Update and FixedUpdate |

## How Clocks Relate

```
time_system (First schedule):
  Time<Real>.update_with_instant(Instant::now())
  → Time<Virtual>.advance_with_raw_delta(Time<Real>.delta())
  → generic Time = Time<Virtual>.as_generic()

run_fixed_main_schedule (RunFixedMainLoop):
  Time<Fixed>.accumulate_overstep(Time<Virtual>.delta())
  while Time<Fixed>.expend():
    generic Time = Time<Fixed>.as_generic()
    run FixedMain schedule
  generic Time = Time<Virtual>.as_generic()  // restore
```

## Real Time (`Time<Real>`)

```rust
pub struct Real { startup: Instant, first_update: Option<Instant>, last_update: Option<Instant> }
```

- Tracks wall clock via `Instant::now()`
- First frame has `delta = 0`, `elapsed = 0`
- Delta is `Instant::now() - last_update`
- Never affected by pause/scale
- Panics if time goes backwards

## Virtual Time (`Time<Virtual>`)

```rust
pub struct Virtual { max_delta: Duration, paused: bool, relative_speed: f64, effective_speed: f64 }
```

- **`set_relative_speed(2.0)`** — game runs at 2× speed
- **`pause()` / `unpause()`** — freeze/resume game time
- **`max_delta`** (default 250ms) — clamps delta to prevent spiral-of-death after freezes
- `effective_speed` = 0.0 if paused, else `relative_speed`
- Delta = `clamped_raw_delta × effective_speed`

## Fixed Time (`Time<Fixed>`)

```rust
pub struct Fixed { timestep: Duration, overstep: Duration }
```

- Default timestep: **15625 µs = 64 Hz** (power of two, avoids 60Hz beat frequency)
- **`accumulate_overstep(delta)`** — add virtual delta to accumulator
- **`expend()` → bool** — consume one timestep if accumulator has enough
- Runs 0, 1, or N fixed steps per frame depending on accumulation

### Schedule Order
```
First → PreUpdate → RunFixedMainLoop → Update → SpawnScene → PostUpdate → Last
                        │
                  FixedMain
                  (while expend()):
                    FixedFirst → FixedPreUpdate → FixedUpdate → FixedPostUpdate → FixedLast
```

## Timer & Stopwatch

### `Stopwatch`
- `tick(delta)` — adds delta if not paused, no upper bound
- `pause()` / `unpause()` / `reset()`
- `elapsed()` / `elapsed_secs()`

### `Timer`
- Wraps Stopwatch with `duration` and `mode: Once | Repeating`
- `tick(delta)` — if elapsed >= duration, fires `finished`
- `just_finished()` — true on the tick where it completed
- `fraction()` — 0.0 to 1.0
- `times_finished_this_tick()` — repeating timers can wrap multiple times per tick

### Run Conditions
- `on_timer(duration)` — repeats on virtual time
- `on_real_timer(duration)` — repeats on real time
- `once_after_delay(duration)` — fires once after delay

## Custom Clock Pattern

Used by avian3d (`Time<Physics>`), bevy_ggrs (`Time<GgrsTime>`):

```rust
#[derive(Default)]
struct MyClock;

// Add methods
impl Time<MyClock> {
    fn set_speed(&mut self, speed: f64) { self.context_mut().speed = speed; }
}

// Override generic Time during your schedule
fn run_my_schedule(world: &mut World) {
    let old = *world.resource::<Time>();  // save
    *world.resource_mut::<Time>() = world.resource::<Time<MyClock>>().as_generic();
    // run schedule — systems see Res<Time> as MyClock's values
    *world.resource_mut::<Time>() = old;  // restore
}
```

## Key Patterns

| Pattern | How |
|---|---|
| **Most systems** | Use `Res<Time>` — works in Update and FixedUpdate automatically |
| **Fixed-rate logic** | Add systems to `FixedUpdate` schedule |
| **Slow motion** | `Time<Virtual>.set_relative_speed(0.3)` |
| **Pause** | `Time<Virtual>.pause()` / `.unpause()` |
| **Keep UI running while paused** | Use `Time<Real>` for UI systems |
| **Frame-rate independent** | Multiply by `time.delta_secs()` |
| **Interpolation** | `Time<Fixed>.overstep_fraction()` for smooth rendering |
| **Cooldowns** | `Timer` with `TimerMode::Once` |
| **Periodic events** | `Timer` with `TimerMode::Repeating` or run conditions |
| **Sequencing** | No built-in timeline — use Timer + state machines or third-party |

## References

- Source: `bevy_time-0.18.1/src/` (time.rs, real.rs, virt.rs, fixed.rs, timer.rs, stopwatch.rs, lib.rs)
- avian3d Time<Physics>: `avian3d-0.6.1/src/schedule/time.rs`
- Run conditions: `bevy_ecs-0.18.1/src/schedule/common_conditions.rs`
