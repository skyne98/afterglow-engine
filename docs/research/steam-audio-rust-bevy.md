# Steam Audio + Rust + Bevy — Integration Guide

## Verification

Tested successfully on **Fedora 43, Ryzen 7 6800U, Mesa/RADV**:

```
$ cargo add audionimbus --features auto-install
$ cargo run
Creating Steam Audio context...
✓ Context created successfully
✓ HRTF loaded
=== Steam Audio WORKS on this Linux machine ===
```

The `auto-install` feature downloads Steam Audio 4.8.1 (~172MB) at build time
and links it automatically. Your final binary needs `libphonon.so` alongside it.

## Overview

Steam Audio is Valve's industry-standard spatial audio SDK. It provides physics-based
(ray-traced) audio: **HRTF binaural rendering, occlusion, reflections, pathing, and
convolution reverb** — all driven by scene geometry.

The Rust ecosystem wraps Steam Audio 4.8.1 via `audionimbus` (safe, idiomatic bindings).
Bevy integration is available through `audionimbus`'s built-in `bevy` feature or the
higher-level `bevy_steam_audio` plugin.

## The Audio Pipeline (Source → Ears)

```
Mono Audio → DirectEffect (distance, occlusion, air absorption)
           → BinauralEffect (HRTF spatialization)
           → Optional: ReflectionEffect (convolution reverb from ray-traced IR)
           → Mix → Output
```

The **simulation** (ray tracing) runs on separate threads and communicates results
to the audio thread via lock-free triple buffers.

## Crate Ecosystem

| Crate | Level | Bevy Integration | Status |
|---|---|---|---|
| **audionimbus** | Raw bindings + Bevy ECS | Built-in `bevy` feature | **Stable** v0.14.0 |
| **bevy_steam_audio** | High-level plugin | Native | **Beta** v0.3.0-rc.1 |
| **avian_steam_audio** | Physics geometry bridge | Plugin for bevy_steam_audio | **Beta** v0.3.0-rc.3 |
| **petalsonic** | Standalone spatial engine | None (Bevy-agnostic) | **Stable** v0.4.0 |
| **firewheel** | Audio graph engine | Via bevy_steam_audio | **Stable** v0.10.0 |

## audionimbus — Core Types

### Context & Initialization

```rust
use audionimbus::*;
let context = Context::default();
```

### Geometry Types

| Type | Purpose |
|---|---|
| `Scene` | 3D scene with meshes |
| `StaticMesh` | Immovable triangle mesh (walls, floors) |
| `InstancedMesh` | Movable mesh (dynamic objects, characters) |
| `Material` | Acoustic surface (`CONCRETE`, `WOOD`, `GLASS`, etc.) |
| `Material::CONCRETE` | Preset: (0.5, 0.5, 0.5) reflection/scattering/absorption |

### Simulation Types

| Type | Purpose |
|---|---|
| `Simulator` | Manages acoustic simulation |
| `Source` | A sound source |
| `Simulator::run_direct()` | Ray-traced occlusion + transmission |
| `Simulator::run_reflections()` | Ray-traced early reflections + late reverb |
| `Simulator::run_pathing()` | Sound propagation around obstacles |
| `SimulationInputs` | Per-frame source pose + parameters |
| `SimulationOutputs` | Per-frame occlusion, reflections, pathing results |

### Effect Types

| Effect | What it does |
|---|---|
| `BinauralEffect` | HRTF spatialization (mono → stereo) |
| `DirectEffect` | Distance attenuation, occlusion, air absorption |
| `ReflectionEffect` | Convolution reverb from simulated IR |
| `PanningEffect` | Speaker layout panning |
| `AmbisonicsEncodeEffect` | Encode to Ambisonics |
| `AmbisonicsBinauralEffect` | Ambisonics → binaural |

### Baking (Precomputed Acoustics)

```rust
use audionimbus::baking::*;
let probe_array = ProbeArray::new(&context, &ProbeArraySettings { scene, num_probes: 100, .. })?;
let mut probe_batch = ProbeBatch::new(&context)?;
probe_batch.add_probe_array(&probe_array);
let baker = ReflectionsBaker::new(&context, &scene, &BakingSettings::default())?;
baker.bake(&mut probe_batch, &ReflectionsBakeParams { .. }, &progress_callback)?;
```

## bevy_steam_audio — Plugin Architecture

```rust
use bevy_steam_audio::prelude::*;

App::new()
    .add_plugins((
        DefaultPlugins,
        SeedlingPlugin::default(),           // audio playback
        SteamAudioPlugin::default(),         // spatial audio
        Mesh3dSteamAudioScenePlugin::default(), // geometry from meshes
    ))
    .add_systems(Startup, setup);

fn setup(mut commands: Commands) {
    // Listener
    commands.spawn((Camera3d::default(), SteamAudioListener));
    // Source
    commands.spawn((
        SamplePlayer::new(asset_server.load("sound.ogg")).looping(),
        SteamAudioPool,
        Transform::from_xyz(-1.5, 0.0, -3.0),
    ));
    // Occluding geometry (auto-imported into Steam Audio)
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.1, 1.0, 3.0))),
        MeshMaterial3d(materials.add(Color::BLACK)),
        Transform::from_xyz(1.0, 0.0, 0.0),
        SteamAudioMesh::default(),
    ));
}
```

## Performance Budget

| Operation | Cost | Notes |
|---|---|---|
| HRTF per source | ~0.01-0.05ms | 32-64 simultaneous sources on modern CPUs |
| Direct occlusion (raycast) | 0.1-0.5ms/source | Scales with scene complexity |
| Reflections (4096 rays) | 10-50ms | Must run on separate thread or bake |
| Pathing | 1-5ms/source | Scales with probe count |

**Simulation runs on separate threads** — never on the audio callback. Results arrive
via lock-free triple buffers.

## Recommended Approach for Bevy

**Use `audionimbus` with its `bevy` feature** for stability and full control.
`bevy_steam_audio` is a valid higher-level option but is still in pre-release.
`avian_steam_audio` provides automatic geometry sync from Avian physics.

```toml
[dependencies]
audionimbus = { version = "0.14", features = ["auto-install", "bevy"] }
bevy = "0.18"
```

The `auto-install` feature downloads the Steam Audio native library (~180MB) at build time.

## References

- audionimbus: https://crates.io/crates/audionimbus
- bevy_steam_audio: https://github.com/janhohenheim/bevy_steam_audio
- Steam Audio SDK: https://github.com/ValveSoftware/steam-audio
- Steam Audio docs: https://valvesoftware.github.io/steam-audio/doc/phonon_reference/html/
