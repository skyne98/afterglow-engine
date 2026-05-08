# Bevy 0.18 Audio & Spatial Audio — Deep Dive

## Built-in: `bevy_audio`

Bevy's default audio uses **rodio** as the backend. Pipeline:

```
Source file → AudioLoader → AudioSource { bytes } → rodio::Decoder
  → Source filters (speed, repeat, volume) → [Sink thread]
  → DynamicMixer → OutputStreamHandle → CPAL → ALSA/CoreAudio/WASAPI
```

### Key Types

| Type | Purpose |
|---|---|
| `AudioPlayer<T>` | Component, holds `Handle<T>` to play |
| `PlaybackSettings` | `mode`, `volume`, `speed`, `paused`, `spatial`, `spatial_scale` |
| `AudioSink` | Component, volume/speed/seek control during playback |
| `SpatialAudioSink` | Component, 3D position + ear position control |
| `SpatialListener` | Component, ear offset positions (default 4-unit X gap) |
| `SpatialScale` | Component, world-unit scaling for spatial audio |
| `AudioSource` | Asset, `bytes: Arc<[u8]>` — full file in memory |
| `GlobalVolume` | Resource, global volume control |
| `Volume` | Enum: `Linear(f32)` or `Decibels(f32)` |
| `Pitch` | Asset for procedural sine wave generation |

### Spatial Audio in `bevy_audio`

**Very basic.** Simple left-right stereo panning based on distance:

```
left_volume = 1.0 / max(left_ear_distance², 1.0)
right_volume = 1.0 / max(right_ear_distance², 1.0)
```

- **No HRTF** — no elevation, no head filtering
- **No occlusion** — sound passes through walls
- **No reverb** — no environmental acoustics
- **No Doppler** — no pitch shift from relative velocity
- **Single listener only** — warns if multiple `SpatialListener`s exist
- Distance attenuation uses inverse square law, minimal customization

## `bevy_kira_audio` (v0.25.0)

The primary alternative. Kira-based, significantly more features:

| Feature | bevy_audio | bevy_kira_audio |
|---|---|---|
| Decoding | rodio (feature-gated) | symphonia (ogg, mp3, m4a, flac, wav) |
| Mixer hierarchy | None | Sub-tracks, send tracks, main track |
| Effects | None | Filter, reverb, distortion, EQ, delay, compressor, pitch shift |
| Tweening | None | Smooth parameter transitions with easing |
| Clock/timing | None | Musical clock (BPM, tick) |
| Spatial audio | Basic panning | Spatial tracks + listener + distance effects |
| Streaming | No | `StreamingSoundData` for large files |
| Procedural | Pitch (sine) | Via Kira's custom sound system |

**Spatial in bevy_kira_audio:** Still stereo panning + distance attenuation only (no HRTF). Adds distance-to-effect mapping (e.g., more reverb at range).

## HRTF-Based Spatial Audio

### Available Rust Options

| Crate | Type | Quality | Effort | Notes |
|---|---|---|---|---|
| **`bevy_steam_audio`** | Bevy plugin | AAA | Medium | v0.3.0-rc.1, uses audionimbus + firewheel, WIP |
| **`audionimbus`** | Steam Audio wrapper | AAA | Medium | v0.13.0, HRTF + occlusion + reverb + pathing |
| **`hrtf`** | Pure Rust | Good | High | v0.8.1, IRCAM data, clicks on fast-moving sources |
| **`sofar`** | Pure Rust | Good | High | v0.3.0, SOFA file support, uniformly partitioned convolution |
| **`bevy_fmod`** | FMOD plugin | AAA | Medium | v0.10.0, proprietary SDK, free for indie |
| **`bevy-rrise`** | Wwise plugin | AAA | Medium | v0.2.1, proprietary SDK |

### Steam Audio Features (via `audionimbus`)
- HRTF binaural rendering (industry standard, Valve)
- Physics-based sound occlusion (ray-traced)
- Specular and diffuse sound reflections
- Pathing simulation for sound propagation
- Ambisonics encode/decode/rotate
- Real-time convolution reverb
- Geometry import from scene

## Rust Audio Ecosystem

| Crate | Purpose | Notes |
|---|---|---|
| **cpal** | Low-level audio I/O | Cross-platform, callbacks, device enumeration |
| **rodio** | Playback library | Default Bevy backend, Sink + Source model |
| **kira** | Game audio engine | Hierarchical mixer, effects, tweens, clocks |
| **symphonia** | Audio decoding | Pure Rust, ogg/wav/mp3/flac/m4a |
| **fundsp** | Audio DSP | Filters, oscillators, reverb, convolution, SIMD |
| **firewheel** | Audio graph engine | Real-time safe node graph, used by bevy_steam_audio |
| **midir** | MIDI I/O | Cross-platform MIDI devices |
| **bevy_fundsp** | Bevy + fundsp | Procedural audio generation |
| **bevy_rustysynth** | MIDI + SoundFont | Play .mid files with .sf2 |

## Streaming Audio

- `bevy_audio` — **no streaming**, loads entire file into memory
- `bevy_kira_audio` — supports `StreamingSoundData` (symphonia decoder, disk-backed)
- DIY — implement `rodio::Source` with a buffered decoder

## MIDI & Procedural

- MIDI: `bevy_rustysynth` (SoundFont synthesis), `midir` (MIDI device I/O)
- Procedural: built-in `Pitch` (sine), `fundsp` (full synthesis + DSP), `bevy_fundsp`

## Recommended Path for Afterglow Engine

**v0.2.0 (immediate):** `bevy_kira_audio` for effects, tweens, streaming, spatial panning. Drop-in replacement for `bevy_audio`.

**v0.3.0 (near future):** Evaluate `bevy_steam_audio` for true HRTF spatial audio + occlusion + reverb. If it reaches stable, integrate as the spatial backend.

**Long term:** Pure Rust HRTF via `sofar` + `firewheel` for a fully open-source, no-native-dependency pipeline.

## References

- bevy_audio source: `bevy_audio-0.18.1/src/`
- bevy_kira_audio: https://crates.io/crates/bevy_kira_audio
- bevy_steam_audio: https://crates.io/crates/bevy_steam_audio
- audionimbus: https://crates.io/crates/audionimbus
- kira: https://crates.io/crates/kira
- fundsp: https://crates.io/crates/fundsp
- hrtf: https://crates.io/crates/hrtf
- sofar: https://crates.io/crates/sofar
