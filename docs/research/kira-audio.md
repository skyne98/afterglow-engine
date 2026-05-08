# Kira Audio Library — Deep Dive

## Overview

Kira is a backend-agnostic, expressive audio library for games, written in Rust.
Development by Andrew Minnich (tesselode). MIT / Apache-2.0.

- **Current version**: 0.12.0 (May 2026)
- **Repository**: https://github.com/tesselode/kira
- **Docs**: https://docs.rs/kira/latest/kira/
- **License**: MIT / Apache-2.0

## Architecture

```
AudioManager<B: Backend>
├── MainTrack (always exists)
│   ├── Sounds, Effects
│   └── SubTracks (MixingTrack — tree hierarchy)
│       ├── volume, panning, effects, sends
│       ├── child sub-tracks
│       ├── sounds
│       └── spatial_data (optional → SpatialTrack)
├── SendTracks (flat, non-hierarchical)
│   └── effects only, receive from sub-tracks via sends
├── Clocks (musical timing — BPM, Hz)
├── Modulators (LFO, Tweener)
└── Listeners (spatial audio ears)
```

### Audio Pipeline (per chunk)

1. Modulators update values
2. Clocks tick forward
3. Listeners update position/orientation
4. Main mixer processes:
   - Read commands from game thread
   - Process each sub-track recursively (sounds → effects → spatial → volume → sends)
   - Process send tracks (accumulate inputs → effects → volume)
   - Mix everything to stereo output, clamp to [-1, 1]

## AudioManager

```rust
let mut manager = AudioManager::<DefaultBackend>::new(
    AudioManagerSettings::default()
)?;
```

Settings: `capacities`, `main_track_builder`, `internal_buffer_size` (default 128 samples ≈ 2.9ms), `backend_settings`.

## Track System

### Track types

| Type | Hierarchy | Sounds | Effects | Sends | Spatial |
|---|---|---|---|---|---|
| `MainTrack` | — | ✓ | ✓ | — | — |
| `SubTrack` (MixingTrack) | Tree (child tracks) | ✓ | ✓ | ✓ | Optional → SpatialTrack |
| `SendTrack` | Flat | — | ✓ | — | — |

### SubTracks

```rust
let mut track = manager.add_sub_track(
    TrackBuilder::new()
        .volume(-6.0)
        .with_effect(FilterBuilder::new().cutoff(1000.0))
        .with_send(&reverb_send, -6.0)
        .sub_track_capacity(16)
        .persist_until_sounds_finish(true)
)?;
track.set_volume(-3.0, Tween::default());
track.play(sound_data)?;
```

### SendTracks

Non-hierarchical, effects-only. Sub-tracks route to them via `.with_send()`.

```rust
let reverb_send = manager.add_send_track(
    SendTrackBuilder::new()
        .with_effect(ReverbBuilder::new().mix(Mix::WET))
)?;
```

## Sound Data

### StaticSoundData

Entire file in memory via `Arc<[Frame]>`. Cheaply cloneable.

```rust
StaticSoundData::from_file("sound.ogg")?
    .volume(-6.0)
    .playback_rate(0.5)
    .panning(-0.5)
    .loop_region(2.0..4.0)
    .fade_in_tween(Tween::default())
    .reverse(true)
    .slice(3.0..6.0);
```

### StreamingSoundData

Streams from disk. Lower memory, higher CPU. Not available on WASM.

```rust
StreamingSoundData::from_file("music.ogg")?;
```

### Custom Sounds

Implement `SoundData` and `Sound` traits.

## Spatial Audio

**Kira's spatial audio is simple panning + distance attenuation — NOT HRTF.**

```rust
let mut listener = manager.add_listener(
    Vec3::ZERO,           // position
    Quat::IDENTITY,       // orientation (faces -Z)
)?;

let mut spatial_track = manager.add_spatial_sub_track(
    &listener,
    Vec3::new(0.0, 1.0, -6.0),
    SpatialTrackBuilder::new()
        .distances((1.0, 100.0))
        .attenuation_function(Easing::Linear)
        .spatialization_strength(0.75)
)?;

spatial_track.set_position(new_pos, Tween::default());
```

### How it works
1. **Attenuation**: distance → easing function → volume factor
2. **Spatialization**: mono signal → per-ear volume from dot product of emitter direction with ear direction (ears at ±22.5°, 0.1m apart)
3. **`Value::FromListenerDistance`**: link any parameter to listener distance

### No HRTF
No head-related transfer function, no occlusion, no diffraction, no environmental reverb modeling.

## Effects

| Effect | Builder | Params |
|---|---|---|
| **Filter** | `FilterBuilder` | cutoff, resonance, mode (LP/BP/HP/Notch) |
| **Reverb** | `ReverbBuilder` | feedback, damping, stereo_width (Freeverb: 8 comb + 4 all-pass) |
| **Delay** | `DelayBuilder` | delay_time, feedback, mix |
| **Distortion** | `DistortionBuilder` | drive, kind (HardClip/SoftClip) |
| **EQ** | `EqFilterBuilder` | frequency, gain, q, kind (Bell/LowShelf/HighShelf) |
| **Compressor** | `CompressorBuilder` | threshold, ratio, attack, release, makeup_gain |
| **PanningControl** | `PanningControlBuilder` | panning (-1 to 1) |
| **VolumeControl** | `VolumeControlBuilder` | volume (dB) |

All parameters support `Value::Fixed`, `Value::FromModulator`, `Value::FromListenerDistance`.

## Tween System

```rust
sound.set_volume(0.0, Tween {
    duration: Duration::from_secs(3),
    easing: Easing::OutPowi(2),
    ..Default::default()
});
```

Easings: `Linear`, `InPowi(n)`, `OutPowi(n)`, `InOutPowi(n)`, `InPowf(f)`, `OutPowf(f)`, `InOutPowf(f)`.

Tweenable types: `f32`, `f64`, `Vec3`, `Quat`, `Duration`, `Decibels`, `Panning`, `PlaybackRate`, `Mix`, `ClockSpeed`.

## Clock System

```rust
let mut clock = manager.add_clock(ClockSpeed::TicksPerMinute(120.0))?;
clock.start();

StaticSoundData::from_file("hit.ogg")?
    .start_time(clock.time() + 4);  // 4 ticks from now
```

ClockTime: `{ clock: ClockId, ticks: u64, fraction: f64 }`.

## Modulators

```rust
let mut lfo = manager.add_modulator(
    LfoBuilder::new()
        .frequency(2.0)
        .amplitude(1.0)
        .waveform(Waveform::Sine)
)?;

StaticSoundData::from_file("drums.ogg")?
    .volume(Value::FromModulator {
        id: lfo.id(),
        mapping: Mapping {
            input_range: (0.0, 1.0),
            output_range: (Decibels::IDENTITY, Decibels::SILENCE),
            easing: Easing::Linear,
        },
    });
```

## Thread Safety

- **Lock-free command queue** via `triple_buffer` (overwriting — latest value wins)
- **Lock-free SPSC ring buffer** (`rtrb`) for resource transfers
- **No allocation on audio thread** (except at init/sample rate change)
- `assert_no_alloc` feature for debugging

## bevy_kira_audio (v0.25.0)

Wraps Kira for Bevy 0.18. **Uses kira ^0.10.8** (older than current 0.12.0 — API differs).

```rust
app.add_plugins(AudioPlugin);           // basic
app.add_plugins(SpatialAudioPlugin);    // spatial
```

Key types: `Audio`, `AudioChannel<T>`, `DynamicAudioChannel`, `AudioControl` trait, `SpatialAudioEmitter`, `SpatialAudioReceiver`.

## Performance

- `internal_buffer_size`: default 128 samples ≈ 2.9ms latency
- No allocation on audio thread
- Fixed pre-allocation via Capacities
- Profile Kira in debug: `[profile.dev.package.kira] opt-level = 3`

## Kira vs Steam Audio

| Feature | Kira | Steam Audio |
|---|---|---|
| Distance attenuation | ✓ | ✓ |
| Stereo panning from 3D | ✓ | ✓ |
| Listener orientation | ✓ | ✓ |
| Effect → distance linking | ✓ | — (separate system) |
| **HRTF (head-related filtering)** | **✗** | **✓** |
| **Occlusion / diffraction** | **✗** | **✓** |
| **Environmental reverb** | **✗ (manual via Reverb + distance)** | **✓ (ray-traced)** |
| Room geometry | ✗ | ✓ |
| Ambisonics | ✗ | ✓ |

**Kira spatial is good for**: basic 3D audio cues, top-down/2.5D games, when simplicity matters.

**Steam Audio is needed for**: first-person games, realistic spatial audio, headphone binaural, occlusion through walls.

## References

- Docs: https://docs.rs/kira/latest/kira/
- Source: https://github.com/tesselode/kira
- bevy_kira_audio: https://crates.io/crates/bevy_kira_audio
