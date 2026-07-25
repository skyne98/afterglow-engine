# Prominent game-audio API ergonomics

**Investigated:** 2026-07-19

## Question

How do easy-to-use, prominent game-audio APIs structure ordinary playback, and
what should Afterglow copy while making physically simulated world audio the
default?

## Compared APIs

### Unreal Engine 5.8

Unreal has the clearest gameplay-facing split:

- `PlaySoundAtLocation`, `PlaySound2D`, and related `PlaySound` calls are
  fire-and-forget.
- `SpawnSoundAtLocation`, `SpawnSound2D`, and `SpawnSoundAttached` return an
  Audio Component for parameter changes, attachment, loops, and stopping.
- Sound Wave/Sound Cue assets and Sound Attenuation assets own most behavior.
- Concurrency assets define limits and resolution such as rejecting the new
  sound or stopping the oldest.
- Global polyphony is target-configurable; priority and final volume determine
  which sources remain rendered.

Epic explicitly describes the API as two categories: `PlaySound` for simple
one-shots that cannot subsequently be modified, and `SpawnSound` for a
controllable Audio Component. Non-spatialized sounds use the explicit 2D calls
or omit/disable attenuation settings.

This is the strongest model for Afterglow's lifecycle API.

### FMOD Studio Unity integration 2.03

FMOD is event-centric rather than clip-centric:

- An authored Event owns routing, randomization, parameters, spatial behavior,
  and DSP.
- `RuntimeManager.PlayOneShot(event, position)` is fire-and-forget.
- `PlayOneShotAttached(event, gameObject)` follows an object automatically.
- `CreateInstance(event)` returns a controllable `EventInstance`.
- `AttachInstanceToGameObject` updates transform and velocity every frame.

This minimizes call-site arguments and gives sound designers control without
changing gameplay code. Its main ergonomic hazard is that whether an event is
2D or 3D can be hidden in authoring data; the call can provide a position that
an incorrectly authored event ignores.

### Unity 6.5 built-in audio

Unity centers playback on an `AudioSource` component attached to a GameObject:

- `Play`, `Pause`, and `Stop` control its primary clip.
- `PlayOneShot` plays overlapping one-shots through the component.
- static `PlayClipAtPoint` plays at a world position and automatically cleans up
  its temporary source.
- `spatialBlend` continuously selects 2D (`0`) through fully 3D (`1`).
- The component exposes many source properties directly: routing, rolloff,
  Doppler, priority, reverb send, spatializer enablement, and bypass flags.

It is approachable in the editor, but the large mutable component surface and a
continuous 2D/3D blend are less semantically clear than separate intent-bearing
calls. Unity's ordinary 3D path also does not imply full geometric acoustics;
Steam Audio integration exposes occlusion and reflections as additional source
options.

### Godot 4.7

Godot encodes placement in the node type:

- `AudioStreamPlayer` is non-positional.
- `AudioStreamPlayer2D` is positional in a 2D world.
- `AudioStreamPlayer3D` is positional in a 3D world.
- Each node owns a stream and calls `play()`; buses and Areas provide routing and
  reverb behavior.

This is highly discoverable and prevents a 2D/3D mode mismatch, but creating or
pooling nodes is more ceremony for transient one-shots than Unreal/FMOD helper
calls.

### Wwise 2025.1

Wwise is also event-centric, but its low-level vocabulary is more explicit:
register a game object, update its position, then `PostEvent` against that game
object. `PostEvent` returns a Playing ID for later control. Engine integrations
hide much of the registration ceremony behind components and Blueprint calls.

The event/game-object separation scales well for large audio teams, but the raw
API is not the easiest model for a small engine's ordinary gameplay call site.

### Steam Audio integrations

Steam Audio is an acoustics extension, not a complete gameplay playback API.
Its Unity, Unreal, FMOD, and Wwise integrations generally add source/effect
settings for HRTF, occlusion, transmission, and reflections. Official guides
instruct users to enable several of these effects. This flexibility is useful,
but it is the opposite of Afterglow's desired guarantee that an ordinary world
sound is fully simulated without remembering a checklist.

## Common successful structure

The prominent APIs converge on five ideas:

1. **An authored sound/event asset owns behavior.** Routing, variation, default
   gain/pitch, loop policy, priority, and concurrency do not belong at every
   call site.
2. **Fire-and-forget is the shortest path.** Ordinary one-shots need an asset
   and either a position or attachment target.
3. **A separate spawn/instance path returns control.** Handles are only needed
   for loops, parameters, fades, stopping, or later attachment changes.
4. **Moving emitters attach to scene objects.** The engine copies position and
   velocity; game code does not push transforms manually each frame.
5. **Non-positional playback is explicit.** Unreal uses `2D`, Godot uses a
   distinct non-positional node, and middleware events are explicitly authored
   as 2D or 3D.

## Afterglow decision

Adopt an Unreal/FMOD-style event API, but make acoustic intent impossible to
silently mis-author.

```text
// Fire-and-forget.
audio.play_at(sound, world_position)       // Full physical world acoustics.
audio.play_attached(sound, entity)         // Full physical acoustics; follows entity.
audio.play_2d(sound)                       // Omnipresent music/UI/stereo bed.
audio.play_spatial_only(sound, position)   // Explicit environment bypass.
audio.play_listener_relative(sound, offset)// Explicit in-head/listener-space sound.

// Same placement semantics, but returns a generational Voice handle.
audio.spawn_at(...)
audio.spawn_attached(...)
audio.spawn_2d(...)

// Sample-accurate control; no game-frame lerp loop.
audio.set_volume(voice, target, seconds)
audio.stop(voice, fade_out_seconds)
audio.set_master_volume(target, seconds)

// Generic transitions between any sounds; not a music subsystem.
audio.crossfade(outgoing_voice, incoming_voice, seconds)
next_voice = audio.crossfade_to(outgoing_voice, next_sound, seconds)
```

All methods submit one fixed command mechanism. `play_*` relinquishes the
handle immediately; `spawn_*` returns it. There are not separate backend paths.

A `Sound` asset owns default gain/pitch, looping eligibility, bus, priority,
concurrency group, streaming/residency policy, and variation. Call sites may
supply a small explicit `PlayOptions` value only when overriding those defaults.

There is no music-only player or transition path. Music, ambience, dialogue,
engines and effects are ordinary voices. `crossfade` transitions any two
existing handles; `crossfade_to` atomically prebuffers a new sound, inherits the
outgoing handle's placement, performs the transition, releases the outgoing
voice and returns the incoming handle. Voice and master volume changes schedule
sample-clock ramps in the worker; callers never write a per-frame lerp. A
replacement ramp begins from the currently evaluated value, so repeated calls
remain click-free. Failure to admit the incoming sound leaves the outgoing voice
unchanged.

`play_at` and `play_attached` mean the complete supported physical chain:
distance attenuation, air absorption, directivity where authored, HRTF,
occlusion, transmission, and reflections. Capacity pressure may virtualize,
reject, or steal according to the sound's concurrency policy, but must never
silently play a partially simulated downgrade. Platform profiles change fixed
capacities and render-ahead depth, not API semantics.

## What not to copy

- Unity's broad mutable `AudioSource` property bag as the primary API.
- A numeric 2D/3D blend for ordinary semantic selection.
- Wwise's raw register/set-position/post ceremony at gameplay call sites.
- Separate public calls for every acoustic effect.
- Hidden asset metadata that can turn `play_at` into a non-spatial event.
- Returning a heavyweight object for every throwaway one-shot.

## Primary sources

- Unity `AudioSource` and `spatialBlend`:
  <https://docs.unity3d.com/ScriptReference/AudioSource.html>,
  <https://docs.unity3d.com/ScriptReference/AudioSource-spatialBlend.html>
- Unreal Audio Engine Overview:
  <https://dev.epicgames.com/documentation/en-us/unreal-engine/audio-engine-overview-in-unreal-engine>
- Godot Audio Streams:
  <https://docs.godotengine.org/en/stable/tutorials/audio/audio_streams.html>
- FMOD Unity `RuntimeManager`:
  <https://www.fmod.com/docs/2.03/unity/api-runtimemanager.html>
- Wwise events and position API:
  <https://www.audiokinetic.com/library/edge/?id=soundengine__events.html>,
  <https://www.audiokinetic.com/library/edge/?id=ak_soundengine_setposition.html>
- Steam Audio Unity guide/source:
  <https://valvesoftware.github.io/steam-audio/doc/unity/guide.html>,
  <https://valvesoftware.github.io/steam-audio/doc/unity/source.html>
