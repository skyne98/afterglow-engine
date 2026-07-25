//! Unified Rust RPC worker for Afterglow audio.
//!
//! Control always uses the generated `#[rpc]` protocol. The web host may call
//! [`afterglow_audio_pump`] between control frames to satisfy the device clock,
//! but that export drives the exact state owned by [`EngineAudioWorker`]; it is
//! not a second service. Steam Audio remains an FFI implementation detail.

use afterglow_rpc::{RpcError, RpcResult};
use afterglow_rpc_macros::rpc;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

mod sound;
mod voice;
use sound::{INVALID_SOUND_HANDLE, RESIDENT_SOUND_BYTE_CAPACITY, SoundBank};
use voice::{INVALID_VOICE_HANDLE, VoicePlacement, VoiceScheduler};

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: afterglow_rpc::allocation::TrackingAllocator<std::alloc::System> =
    afterglow_rpc::allocation::TrackingAllocator::new(std::alloc::System);

pub const SAMPLE_RATE: u32 = 48_000;
pub const QUANTUM_FRAMES: usize = 128;
pub const OUTPUT_CHANNELS: usize = 2;
pub const PCM_SAMPLES: usize = QUANTUM_FRAMES * OUTPUT_CHANNELS;
/// Native keeps the measured high-capacity mixer. Public web deliberately uses
/// a small profile so the same Worker→PCM-ring architecture remains deadline-
/// safe without a second AudioWorklet-owned DSP implementation.
#[cfg(not(target_arch = "wasm32"))]
pub const VOICE_CAPACITY: u32 = 128;
#[cfg(target_arch = "wasm32")]
pub const VOICE_CAPACITY: u32 = 16;

/// Every admitted world-physical voice receives the complete direct/HRTF/
/// occlusion/transmission/reflection chain. Other slots are only for explicitly
/// nonphysical placement modes.
#[cfg(not(target_arch = "wasm32"))]
pub const PHYSICAL_VOICE_CAPACITY: u32 = 16;
#[cfg(target_arch = "wasm32")]
pub const PHYSICAL_VOICE_CAPACITY: u32 = 4;

pub const ACTIVE_SPATIAL_VOICES: u32 = PHYSICAL_VOICE_CAPACITY;
pub const REFLECTION_CAPACITY: u32 = PHYSICAL_VOICE_CAPACITY;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioWorkerConfig {
    pub target_quanta: u32,
    pub triangles: u32,
    pub reflection_rays: u32,
    pub reflection_bounces: u32,
    pub reflection_duration_ms: u32,
    pub reflection_order: u32,
}

impl Default for AudioWorkerConfig {
    fn default() -> Self {
        Self {
            target_quanta: 8,
            triangles: 10_000,
            reflection_rays: 512,
            reflection_bounces: 2,
            reflection_duration_ms: 500,
            reflection_order: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct AudioWorkerStats {
    pub sample_clock: u64,
    pub rendered_quanta: u64,
    pub simulation_updates: u64,
    pub output_energy: f32,
    pub output_peak: f32,
    pub last_impulse_sample: u64,
    pub target_quanta: u32,
    pub voice_count: u32,
    pub reflection_voice_count: u32,
    pub resident_voices: u32,
    pub asset_stream_voices: u32,
    pub live_voices: u32,
    pub procedural_voices: u32,
    pub active_spatial_voices: u32,
    pub active_reflection_voices: u32,
    pub active_voices: u32,
    pub active_world_physical_voices: u32,
    pub rejected_voice_capacity: u64,
    pub rejected_physical_capacity: u64,
    pub stale_voice_handles: u64,
    pub completed_voice_fades: u64,
    pub loaded_resident_sounds: u32,
    pub resident_sound_bytes: u64,
    pub acoustic_vertices: u32,
    pub acoustic_triangles: u32,
    pub acoustic_scene_bytes: u64,
    pub running: bool,
    pub fatal: bool,
}

impl AudioWorkerStats {
    /// Stable generated-client wire order. All counters remain exact in f64
    /// for any practical process lifetime.
    pub fn to_f64_vec(self) -> Vec<f64> {
        vec![
            self.sample_clock as f64,
            self.rendered_quanta as f64,
            self.simulation_updates as f64,
            self.output_energy as f64,
            self.output_peak as f64,
            self.last_impulse_sample as f64,
            self.target_quanta as f64,
            self.voice_count as f64,
            self.reflection_voice_count as f64,
            self.resident_voices as f64,
            self.asset_stream_voices as f64,
            self.live_voices as f64,
            self.procedural_voices as f64,
            u8::from(self.running) as f64,
            u8::from(self.fatal) as f64,
            self.active_spatial_voices as f64,
            self.active_reflection_voices as f64,
            self.active_voices as f64,
            self.active_world_physical_voices as f64,
            self.rejected_voice_capacity as f64,
            self.rejected_physical_capacity as f64,
            self.stale_voice_handles as f64,
            self.completed_voice_fades as f64,
            self.loaded_resident_sounds as f64,
            self.resident_sound_bytes as f64,
            self.acoustic_vertices as f64,
            self.acoustic_triangles as f64,
            self.acoustic_scene_bytes as f64,
        ]
    }
}

/// The one control surface for the audio worker. Device-rate pumping is a host
/// wake concern; all configuration, lifecycle and telemetry remain typed RPC.
#[rpc(worker = EngineAudioWorker)]
pub trait EngineAudioService {
    fn configure(
        target_quanta: u32,
        triangles: u32,
        reflection_rays: u32,
        reflection_bounces: u32,
        reflection_duration_ms: u32,
        reflection_order: u32,
    ) -> i32;
    fn start() -> i32;
    fn stop() -> i32;
    fn update_motion(phase: f32) -> i32;
    fn run_simulation() -> i32;
    /// Fixed field order documented by [`AudioWorkerStats::to_f64_vec`].
    fn stats() -> Vec<f64>;
    fn shutdown() -> i32;

    // Fixed voice control. These append-only method IDs preserve the existing
    // Gate-0 lifecycle wire order while the public EngineAudio facade is built.
    fn spawn_2d(sound: u32) -> u32;
    fn spawn_at(sound: u32, x: f32, y: f32, z: f32) -> u32;
    fn spawn_attached(sound: u32, entity: u32) -> u32;
    fn spawn_spatial_only(sound: u32, x: f32, y: f32, z: f32) -> u32;
    fn spawn_listener_relative(sound: u32, x: f32, y: f32, z: f32) -> u32;
    fn crossfade(from: u32, to: u32, seconds: f32) -> i32;
    fn crossfade_to(from: u32, sound: u32, seconds: f32) -> u32;
    fn set_voice_volume(handle: u32, volume: f32, seconds: f32) -> i32;
    fn pause_voice(handle: u32, seconds: f32) -> i32;
    fn resume_voice(handle: u32, seconds: f32) -> i32;
    fn stop_voice(handle: u32, seconds: f32) -> i32;

    // Resident 48 kHz WAV loading is a warm-up operation. Methods are appended
    // so existing lifecycle and voice-control IDs remain stable.
    fn load_wav(data: Vec<u8>, looped: bool) -> u32;
    fn unload_sound(handle: u32) -> i32;
    fn begin_wav_upload(total_bytes: u32, looped: bool) -> i32;
    fn append_wav_upload(data: Vec<u8>) -> i32;
    fn finish_wav_upload() -> u32;
    fn begin_acoustic_scene_upload(total_bytes: u32) -> i32;
    fn append_acoustic_scene_upload(data: Vec<u8>) -> i32;
    fn finish_acoustic_scene_upload() -> i32;
}

struct SyntheticDsp {
    pcm: [f32; PCM_SAMPLES],
    sample_clock: u64,
    phase: f32,
    output_energy: f32,
    output_peak: f32,
    last_impulse_sample: u64,
}

impl SyntheticDsp {
    fn new() -> Self {
        Self {
            pcm: [0.0; PCM_SAMPLES],
            sample_clock: 0,
            phase: 0.0,
            output_energy: 0.0,
            output_peak: 0.0,
            last_impulse_sample: 0,
        }
    }

    fn render(&mut self, voices: &VoiceScheduler, sounds: &SoundBank) {
        self.output_energy = 0.0;
        self.output_peak = 0.0;
        let two_pi = core::f32::consts::TAU;
        for frame in 0..QUANTUM_FRAMES {
            let absolute = self.sample_clock + frame as u64;
            let resident = (absolute as f32 * two_pi * 220.0 / SAMPLE_RATE as f32).sin();
            let stream = ((absolute % 113) as f32 / 56.5) - 1.0;
            let mut bits = absolute as u32 ^ 0x9e37_79b9;
            bits ^= bits << 13;
            bits ^= bits >> 17;
            bits ^= bits << 5;
            let live = (bits & 0xffff) as f32 / 32_767.5 - 1.0;
            let mut procedural =
                (absolute as f32 * two_pi * 330.0 / SAMPLE_RATE as f32 + self.phase).sin();
            if absolute % SAMPLE_RATE as u64 == 0 {
                procedural = 1.0;
                self.last_impulse_sample = absolute;
            }
            let diagnostics = [resident, stream, live, procedural];
            let (mut left, mut right) = (0.0, 0.0);
            if voices.is_controlled() {
                for index in 0..VOICE_CAPACITY as usize {
                    let voice = voices.render_state(index);
                    if !voice.active {
                        continue;
                    }
                    let cursor = voice.cursor + frame as u64;
                    let (sample_left, sample_right) = if let Some(sound) = sounds.view(voice.sound)
                    {
                        let left = sounds.sample(voice.sound, cursor, 0);
                        let right = if sound.channels == 2 {
                            sounds.sample(voice.sound, cursor, 1)
                        } else {
                            left
                        };
                        (left, right)
                    } else {
                        let sample =
                            diagnostics[(voice.sound.wrapping_sub(1) & 3) as usize] * 0.0125;
                        (sample, sample)
                    };
                    match voice.placement {
                        VoicePlacement::TwoD => {
                            left += sample_left * voice.gain;
                            right += sample_right * voice.gain;
                        }
                        VoicePlacement::World(position)
                        | VoicePlacement::SpatialOnly(position)
                        | VoicePlacement::ListenerRelative(position) => {
                            let mono = (sample_left + sample_right) * 0.5 * voice.gain;
                            let pan = (position[0] * 0.25).clamp(-1.0, 1.0);
                            left += mono * (1.0 - pan) * 0.5;
                            right += mono * (1.0 + pan) * 0.5;
                        }
                        VoicePlacement::Attached(_) => {
                            let mono = (sample_left + sample_right) * 0.25 * voice.gain;
                            left += mono;
                            right += mono;
                        }
                    }
                }
            } else {
                let mono = diagnostics.into_iter().sum::<f32>() * 0.025;
                left = mono * 0.92;
                right = mono * 0.78;
            }
            self.pcm[frame * 2] = left;
            self.pcm[frame * 2 + 1] = right;
            self.output_energy += left.abs() + right.abs();
            self.output_peak = self.output_peak.max(left.abs()).max(right.abs());
        }
        self.sample_clock += QUANTUM_FRAMES as u64;
    }
}

#[cfg(feature = "steam-audio")]
mod steam {
    use super::AudioWorkerConfig;
    use std::ffi::c_int;

    unsafe extern "C" {
        fn afterglow_steam_audio_init(
            triangles: u32,
            voices: u32,
            reflection_voices: u32,
            rays: u32,
            bounces: u32,
            duration_ms: u32,
            order: u32,
        ) -> c_int;
        fn afterglow_steam_audio_update_motion(phase: f32) -> c_int;
        fn afterglow_steam_audio_run_simulation() -> c_int;
        fn afterglow_steam_audio_render_quantum() -> c_int;
        fn afterglow_steam_audio_pcm_ptr() -> *const f32;
        fn afterglow_steam_audio_sample_clock() -> u64;
        fn afterglow_steam_audio_output_energy() -> f32;
        fn afterglow_steam_audio_output_peak() -> f32;
        fn afterglow_steam_audio_active_reflection_voices() -> u32;
        fn afterglow_steam_audio_last_impulse_sample() -> u64;
        fn afterglow_steam_audio_set_voice(
            index: u32,
            mode: i32,
            sound: u32,
            x: f32,
            y: f32,
            z: f32,
            gain: f32,
            cursor: u64,
        ) -> c_int;
        fn afterglow_steam_audio_load_acoustic_scene(bytes: *const u8, byte_count: u32) -> c_int;
        fn afterglow_steam_audio_acoustic_vertices() -> u32;
        fn afterglow_steam_audio_acoustic_triangles() -> u32;
        fn afterglow_steam_audio_register_sound(
            handle: u32,
            samples: *const f32,
            frames: u32,
            channels: u32,
            looped: c_int,
        ) -> c_int;
        fn afterglow_steam_audio_unregister_sound(handle: u32) -> c_int;
        fn afterglow_steam_audio_shutdown();
    }

    pub struct SteamAudioDsp;

    // Steam Audio handles never leave this wrapper. The owning RPC worker and
    // the device pump run on one outer worker thread.
    unsafe impl Send for SteamAudioDsp {}

    impl SteamAudioDsp {
        pub fn create(config: AudioWorkerConfig) -> Result<Self, i32> {
            // Keep the promoted tracer crate in this one final Rust staticlib;
            // linking a second Rust staticlib would duplicate libstd/allocator
            // symbols in the Emscripten module.
            let _ = afterglow_obvhs_tracer::afterglow_obvhs_traversal_lanes();
            let status = unsafe {
                afterglow_steam_audio_init(
                    config.triangles,
                    super::VOICE_CAPACITY,
                    super::REFLECTION_CAPACITY,
                    config.reflection_rays,
                    config.reflection_bounces,
                    config.reflection_duration_ms,
                    config.reflection_order,
                )
            };
            if status == 0 { Ok(Self) } else { Err(status) }
        }

        pub fn update_motion(&mut self, phase: f32) -> i32 {
            unsafe { afterglow_steam_audio_update_motion(phase) }
        }
        pub fn run_simulation(&mut self) -> i32 {
            unsafe { afterglow_steam_audio_run_simulation() }
        }
        pub fn render(&mut self) -> i32 {
            unsafe { afterglow_steam_audio_render_quantum() }
        }
        pub fn pcm_ptr(&self) -> *const f32 {
            let pointer = unsafe { afterglow_steam_audio_pcm_ptr() };
            debug_assert!(!pointer.is_null());
            pointer
        }
        pub fn sample_clock(&self) -> u64 {
            unsafe { afterglow_steam_audio_sample_clock() }
        }
        pub fn output_energy(&self) -> f32 {
            unsafe { afterglow_steam_audio_output_energy() }
        }
        pub fn output_peak(&self) -> f32 {
            unsafe { afterglow_steam_audio_output_peak() }
        }
        pub fn last_impulse_sample(&self) -> u64 {
            unsafe { afterglow_steam_audio_last_impulse_sample() }
        }
        pub fn active_reflection_voices(&self) -> u32 {
            unsafe { afterglow_steam_audio_active_reflection_voices() }
        }
        pub fn load_acoustic_scene(&mut self, bytes: &[u8]) -> i32 {
            let Ok(byte_count) = u32::try_from(bytes.len()) else {
                return 210;
            };
            unsafe { afterglow_steam_audio_load_acoustic_scene(bytes.as_ptr(), byte_count) }
        }
        pub fn acoustic_vertices(&self) -> u32 {
            unsafe { afterglow_steam_audio_acoustic_vertices() }
        }
        pub fn acoustic_triangles(&self) -> u32 {
            unsafe { afterglow_steam_audio_acoustic_triangles() }
        }
        pub fn register_sound(&mut self, sound: super::sound::ResidentSoundView) -> i32 {
            unsafe {
                afterglow_steam_audio_register_sound(
                    sound.handle,
                    sound.samples,
                    sound.frames,
                    sound.channels,
                    i32::from(sound.looped),
                )
            }
        }
        pub fn unregister_sound(&mut self, handle: u32) -> i32 {
            unsafe { afterglow_steam_audio_unregister_sound(handle) }
        }
        pub fn sync_voices(&mut self, voices: &super::VoiceScheduler) -> i32 {
            if !voices.is_controlled() {
                return 0;
            }
            for index in 0..super::VOICE_CAPACITY as usize {
                let voice = voices.render_state(index);
                let (mode, position) = if !voice.active {
                    (0, [0.0; 3])
                } else {
                    match voice.placement {
                        super::VoicePlacement::World(position) => (1, position),
                        super::VoicePlacement::Attached(_) => (1, [0.0; 3]),
                        super::VoicePlacement::TwoD => (2, [0.0; 3]),
                        super::VoicePlacement::SpatialOnly(position) => (3, position),
                        super::VoicePlacement::ListenerRelative(position) => (4, position),
                    }
                };
                let status = unsafe {
                    afterglow_steam_audio_set_voice(
                        index as u32,
                        mode,
                        voice.sound,
                        position[0],
                        position[1],
                        position[2],
                        voice.gain,
                        voice.cursor,
                    )
                };
                if status != 0 {
                    return status;
                }
            }
            0
        }
    }

    impl Drop for SteamAudioDsp {
        fn drop(&mut self) {
            unsafe { afterglow_steam_audio_shutdown() }
        }
    }
}

const MAX_WAV_UPLOAD_BYTES: usize = RESIDENT_SOUND_BYTE_CAPACITY + 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const MAX_ACOUSTIC_SCENE_BYTES: usize = 128 * 1024 * 1024;
#[cfg(target_arch = "wasm32")]
const MAX_ACOUSTIC_SCENE_BYTES: usize = 80 * 1024 * 1024;

enum PendingUploadKind {
    Wav { looped: bool },
    AcousticScene,
}

struct PendingUpload {
    kind: PendingUploadKind,
    expected_bytes: usize,
    bytes: Vec<u8>,
}

enum DspBackend {
    Synthetic(SyntheticDsp),
    #[cfg(feature = "steam-audio")]
    Steam(steam::SteamAudioDsp),
}

struct WorkerState {
    config: AudioWorkerConfig,
    backend: DspBackend,
    configured: bool,
    running: bool,
    fatal: bool,
    rendered_quanta: u64,
    simulation_updates: u64,
    voices: VoiceScheduler,
    sounds: SoundBank,
    upload: Option<PendingUpload>,
    acoustic_scene_bytes: u64,
}

impl WorkerState {
    fn new() -> Self {
        Self {
            config: AudioWorkerConfig::default(),
            backend: DspBackend::Synthetic(SyntheticDsp::new()),
            configured: false,
            running: false,
            fatal: false,
            rendered_quanta: 0,
            simulation_updates: 0,
            voices: VoiceScheduler::new(),
            sounds: SoundBank::new(),
            upload: None,
            acoustic_scene_bytes: 0,
        }
    }

    fn validate(config: AudioWorkerConfig) -> RpcResult<()> {
        if !(2..=8).contains(&config.target_quanta)
            || config.triangles < 12
            || config.triangles > 1_000_000
            || config.reflection_rays == 0
            || config.reflection_rays > 65_536
            || config.reflection_bounces == 0
            || config.reflection_bounces > 64
            || config.reflection_duration_ms == 0
            || config.reflection_duration_ms > 4_000
            || config.reflection_order > 3
        {
            return Err(RpcError::Server(
                "invalid EngineAudio worker configuration".into(),
            ));
        }
        Ok(())
    }

    fn configure(&mut self, config: AudioWorkerConfig) -> RpcResult<()> {
        Self::validate(config)?;
        self.running = false;
        self.fatal = false;
        self.rendered_quanta = 0;
        self.simulation_updates = 0;
        self.voices = VoiceScheduler::new();
        self.upload = None;
        self.acoustic_scene_bytes = 0;
        #[cfg(feature = "steam-audio")]
        {
            // Drop the prior RAII owner before initializing global C handles;
            // assigning after create would drop the old wrapper afterward and
            // accidentally shut down the newly created Steam state.
            self.backend = DspBackend::Synthetic(SyntheticDsp::new());
            self.sounds = SoundBank::new();
            self.backend =
                DspBackend::Steam(steam::SteamAudioDsp::create(config).map_err(|status| {
                    RpcError::Server(format!("Steam Audio initialization failed: {status}"))
                })?);
        }
        #[cfg(not(feature = "steam-audio"))]
        {
            self.backend = DspBackend::Synthetic(SyntheticDsp::new());
            self.sounds = SoundBank::new();
        }
        self.config = config;
        self.configured = true;
        Ok(())
    }

    fn pump(&mut self) -> i32 {
        if !self.configured || !self.running || self.fatal {
            return 0;
        }
        #[cfg(feature = "steam-audio")]
        if let DspBackend::Steam(dsp) = &mut self.backend {
            let sync_status = dsp.sync_voices(&self.voices);
            if sync_status != 0 {
                self.running = false;
                self.fatal = true;
                return -sync_status.abs();
            }
        }
        let status: i32 = match &mut self.backend {
            DspBackend::Synthetic(dsp) => {
                dsp.render(&self.voices, &self.sounds);
                0
            }
            #[cfg(feature = "steam-audio")]
            DspBackend::Steam(dsp) => dsp.render(),
        };
        if status != 0 {
            self.running = false;
            self.fatal = true;
            return -status.abs();
        }
        self.voices.advance(QUANTUM_FRAMES as u32, &self.sounds);
        self.rendered_quanta += 1;
        1
    }

    fn update_motion(&mut self, phase: f32) -> RpcResult<()> {
        if !phase.is_finite() {
            return Err(RpcError::Server("non-finite audio motion".into()));
        }
        let status = match &mut self.backend {
            DspBackend::Synthetic(dsp) => {
                dsp.phase = phase;
                0
            }
            #[cfg(feature = "steam-audio")]
            DspBackend::Steam(dsp) => dsp.update_motion(phase),
        };
        self.latch_status(status, "Steam Audio motion update")
    }

    fn run_simulation(&mut self) -> RpcResult<()> {
        let status = match &mut self.backend {
            DspBackend::Synthetic(_) => 0,
            #[cfg(feature = "steam-audio")]
            DspBackend::Steam(dsp) => dsp.run_simulation(),
        };
        self.latch_status(status, "Steam Audio simulation")?;
        self.simulation_updates += 1;
        Ok(())
    }

    fn simulate_motion(&mut self, phase: f32) -> i32 {
        if !self.configured || !self.running || self.fatal {
            return -1;
        }
        if self.update_motion(phase).is_err() || self.run_simulation().is_err() {
            return -2;
        }
        0
    }

    fn latch_status(&mut self, status: i32, operation: &str) -> RpcResult<()> {
        if status == 0 {
            return Ok(());
        }
        self.running = false;
        self.fatal = true;
        Err(RpcError::Server(format!("{operation} failed: {status}")))
    }

    fn sound_valid(&self, handle: u32) -> bool {
        // Handles 1..=4 retain the deterministic producer families used by
        // diagnostics. Public resident sounds are generational bank handles.
        (1..=16).contains(&handle) || self.sounds.contains(handle)
    }

    fn load_wav(&mut self, data: &[u8], looped: bool) -> u32 {
        if !self.configured || self.running || self.fatal {
            return INVALID_SOUND_HANDLE;
        }
        let handle = self.sounds.load_wav(data, looped);
        if handle == INVALID_SOUND_HANDLE {
            return handle;
        }
        #[cfg(feature = "steam-audio")]
        if let DspBackend::Steam(dsp) = &mut self.backend {
            let view = self.sounds.view(handle).expect("new sound must resolve");
            if dsp.register_sound(view) != 0 {
                self.sounds.unload(handle);
                return INVALID_SOUND_HANDLE;
            }
        }
        handle
    }

    fn unload_sound(&mut self, handle: u32) -> bool {
        if !self.configured || self.running || self.fatal || self.voices.uses_sound(handle) {
            return false;
        }
        if !self.sounds.contains(handle) {
            return false;
        }
        #[cfg(feature = "steam-audio")]
        if let DspBackend::Steam(dsp) = &mut self.backend
            && dsp.unregister_sound(handle) != 0
        {
            return false;
        }
        self.sounds.unload(handle)
    }

    fn begin_upload(&mut self, kind: PendingUploadKind, total_bytes: u32, limit: usize) -> bool {
        if !self.configured || self.running || self.fatal || self.upload.is_some() {
            return false;
        }
        let total_bytes = total_bytes as usize;
        if total_bytes == 0 || total_bytes > limit {
            return false;
        }
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(total_bytes).is_err() {
            return false;
        }
        self.upload = Some(PendingUpload {
            kind,
            expected_bytes: total_bytes,
            bytes,
        });
        true
    }

    fn append_upload(&mut self, data: &[u8], wav: bool) -> bool {
        if self.running || self.fatal || data.is_empty() {
            return false;
        }
        let Some(upload) = &mut self.upload else {
            return false;
        };
        if matches!(upload.kind, PendingUploadKind::Wav { .. }) != wav
            || data.len() > upload.expected_bytes.saturating_sub(upload.bytes.len())
        {
            return false;
        }
        upload.bytes.extend_from_slice(data);
        true
    }

    fn finish_wav_upload(&mut self) -> u32 {
        let Some(upload) = self.upload.take() else {
            return INVALID_SOUND_HANDLE;
        };
        let PendingUploadKind::Wav { looped } = upload.kind else {
            return INVALID_SOUND_HANDLE;
        };
        if upload.bytes.len() != upload.expected_bytes {
            return INVALID_SOUND_HANDLE;
        }
        self.load_wav(&upload.bytes, looped)
    }

    fn finish_acoustic_scene_upload(&mut self) -> bool {
        let Some(upload) = self.upload.take() else {
            return false;
        };
        if !matches!(upload.kind, PendingUploadKind::AcousticScene)
            || upload.bytes.len() != upload.expected_bytes
        {
            return false;
        }
        #[cfg(feature = "steam-audio")]
        {
            if let DspBackend::Steam(dsp) = &mut self.backend {
                if dsp.load_acoustic_scene(&upload.bytes) != 0 {
                    return false;
                }
                self.acoustic_scene_bytes = upload.bytes.len() as u64;
                return true;
            }
            false
        }
        #[cfg(not(feature = "steam-audio"))]
        false
    }

    fn spawn_voice(&mut self, sound: u32, placement: VoicePlacement) -> u32 {
        if !self.sound_valid(sound) {
            return INVALID_VOICE_HANDLE;
        }
        self.controlled_voices()
            .map_or(INVALID_VOICE_HANDLE, |voices| {
                voices.spawn(sound, placement, 1.0, 0)
            })
    }

    fn crossfade_to(&mut self, from: u32, sound: u32, seconds: f32) -> u32 {
        if !self.sound_valid(sound) {
            return INVALID_VOICE_HANDLE;
        }
        self.controlled_voices()
            .map_or(INVALID_VOICE_HANDLE, |voices| {
                voices.crossfade_to(from, sound, seconds)
            })
    }

    fn controlled_voices(&mut self) -> Option<&mut VoiceScheduler> {
        if self.configured && self.running && !self.fatal {
            Some(&mut self.voices)
        } else {
            None
        }
    }

    fn pcm_ptr(&self) -> *const f32 {
        match &self.backend {
            DspBackend::Synthetic(dsp) => dsp.pcm.as_ptr(),
            #[cfg(feature = "steam-audio")]
            DspBackend::Steam(dsp) => dsp.pcm_ptr(),
        }
    }

    fn copy_pcm_to(&self, output: &mut [f32; PCM_SAMPLES]) {
        // SAFETY: every backend retains exactly `PCM_SAMPLES` initialized f32
        // values until its next worker-thread pump. This copy runs on that same
        // worker thread immediately after pumping.
        let input = unsafe { std::slice::from_raw_parts(self.pcm_ptr(), PCM_SAMPLES) };
        output.copy_from_slice(input);
    }

    fn stats(&self) -> AudioWorkerStats {
        let (sample_clock, output_energy, output_peak, last_impulse_sample) = match &self.backend {
            DspBackend::Synthetic(dsp) => (
                dsp.sample_clock,
                dsp.output_energy,
                dsp.output_peak,
                dsp.last_impulse_sample,
            ),
            #[cfg(feature = "steam-audio")]
            DspBackend::Steam(dsp) => (
                dsp.sample_clock(),
                dsp.output_energy(),
                dsp.output_peak(),
                dsp.last_impulse_sample(),
            ),
        };
        let voice_stats = self.voices.stats();
        AudioWorkerStats {
            sample_clock,
            rendered_quanta: self.rendered_quanta,
            simulation_updates: self.simulation_updates,
            output_energy,
            output_peak,
            last_impulse_sample,
            target_quanta: self.config.target_quanta,
            voice_count: VOICE_CAPACITY,
            reflection_voice_count: REFLECTION_CAPACITY,
            resident_voices: VOICE_CAPACITY / 4,
            asset_stream_voices: VOICE_CAPACITY / 4,
            live_voices: VOICE_CAPACITY / 4,
            procedural_voices: VOICE_CAPACITY / 4,
            active_spatial_voices: ACTIVE_SPATIAL_VOICES,
            active_reflection_voices: match &self.backend {
                DspBackend::Synthetic(_) => REFLECTION_CAPACITY,
                #[cfg(feature = "steam-audio")]
                DspBackend::Steam(dsp) => dsp.active_reflection_voices(),
            },
            active_voices: voice_stats.active,
            active_world_physical_voices: voice_stats.active_world_physical,
            rejected_voice_capacity: voice_stats.rejected_capacity,
            rejected_physical_capacity: voice_stats.rejected_physical_capacity,
            stale_voice_handles: voice_stats.stale_handles,
            completed_voice_fades: voice_stats.completed_fades,
            loaded_resident_sounds: self.sounds.loaded_count(),
            resident_sound_bytes: self.sounds.used_bytes() as u64,
            acoustic_vertices: match &self.backend {
                DspBackend::Synthetic(_) => 0,
                #[cfg(feature = "steam-audio")]
                DspBackend::Steam(dsp) => dsp.acoustic_vertices(),
            },
            acoustic_triangles: match &self.backend {
                DspBackend::Synthetic(_) => 0,
                #[cfg(feature = "steam-audio")]
                DspBackend::Steam(dsp) => dsp.acoustic_triangles(),
            },
            acoustic_scene_bytes: self.acoustic_scene_bytes,
            running: self.running,
            fatal: self.fatal,
        }
    }
}

type SharedState = Arc<Mutex<WorkerState>>;

thread_local! {
    static WEB_WORKER_STATE: RefCell<Option<SharedState>> = const { RefCell::new(None) };
}

pub struct EngineAudioWorker {
    state: SharedState,
}

impl Default for EngineAudioWorker {
    fn default() -> Self {
        let state = Arc::new(Mutex::new(WorkerState::new()));
        #[cfg(target_arch = "wasm32")]
        WEB_WORKER_STATE.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&state)));
        Self { state }
    }
}

impl EngineAudioWorker {
    fn with_state<T>(&self, operation: impl FnOnce(&mut WorkerState) -> T) -> T {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        operation(&mut state)
    }
}

impl EngineAudioServiceServer for EngineAudioWorker {
    fn configure(
        &mut self,
        target_quanta: u32,
        triangles: u32,
        reflection_rays: u32,
        reflection_bounces: u32,
        reflection_duration_ms: u32,
        reflection_order: u32,
    ) -> i32 {
        self.with_state(|state| {
            match state.configure(AudioWorkerConfig {
                target_quanta,
                triangles,
                reflection_rays,
                reflection_bounces,
                reflection_duration_ms,
                reflection_order,
            }) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        })
    }

    fn start(&mut self) -> i32 {
        self.with_state(|state| {
            if !state.configured {
                return -1;
            }
            if state.fatal {
                return -2;
            }
            state.running = true;
            0
        })
    }

    fn stop(&mut self) -> i32 {
        self.with_state(|state| {
            state.running = false;
            0
        })
    }

    fn update_motion(&mut self, phase: f32) -> i32 {
        self.with_state(|state| {
            if state.update_motion(phase).is_ok() {
                0
            } else {
                -1
            }
        })
    }

    fn run_simulation(&mut self) -> i32 {
        self.with_state(|state| {
            if state.run_simulation().is_ok() {
                0
            } else {
                -1
            }
        })
    }

    fn stats(&mut self) -> Vec<f64> {
        self.with_state(|state| state.stats().to_f64_vec())
    }

    fn shutdown(&mut self) -> i32 {
        self.with_state(|state| {
            state.running = false;
            state.configured = false;
            state.backend = DspBackend::Synthetic(SyntheticDsp::new());
            state.voices = VoiceScheduler::new();
            state.sounds = SoundBank::new();
            state.upload = None;
            state.acoustic_scene_bytes = 0;
            0
        })
    }

    fn spawn_2d(&mut self, sound: u32) -> u32 {
        self.with_state(|state| state.spawn_voice(sound, VoicePlacement::TwoD))
    }

    fn spawn_at(&mut self, sound: u32, x: f32, y: f32, z: f32) -> u32 {
        self.with_state(|state| state.spawn_voice(sound, VoicePlacement::World([x, y, z])))
    }

    fn spawn_attached(&mut self, sound: u32, entity: u32) -> u32 {
        self.with_state(|state| state.spawn_voice(sound, VoicePlacement::Attached(entity)))
    }

    fn spawn_spatial_only(&mut self, sound: u32, x: f32, y: f32, z: f32) -> u32 {
        self.with_state(|state| state.spawn_voice(sound, VoicePlacement::SpatialOnly([x, y, z])))
    }

    fn spawn_listener_relative(&mut self, sound: u32, x: f32, y: f32, z: f32) -> u32 {
        self.with_state(|state| {
            state.spawn_voice(sound, VoicePlacement::ListenerRelative([x, y, z]))
        })
    }

    fn crossfade(&mut self, from: u32, to: u32, seconds: f32) -> i32 {
        self.with_state(|state| {
            state.controlled_voices().map_or(-1, |voices| {
                if voices.crossfade(from, to, seconds) {
                    0
                } else {
                    -1
                }
            })
        })
    }

    fn crossfade_to(&mut self, from: u32, sound: u32, seconds: f32) -> u32 {
        self.with_state(|state| state.crossfade_to(from, sound, seconds))
    }

    fn set_voice_volume(&mut self, handle: u32, volume: f32, seconds: f32) -> i32 {
        self.with_state(|state| {
            state.controlled_voices().map_or(-1, |voices| {
                if voices.set_volume(handle, volume, seconds) {
                    0
                } else {
                    -1
                }
            })
        })
    }

    fn pause_voice(&mut self, handle: u32, seconds: f32) -> i32 {
        self.with_state(|state| {
            state.controlled_voices().map_or(
                -1,
                |voices| {
                    if voices.pause(handle, seconds) { 0 } else { -1 }
                },
            )
        })
    }

    fn resume_voice(&mut self, handle: u32, seconds: f32) -> i32 {
        self.with_state(|state| {
            state.controlled_voices().map_or(-1, |voices| {
                if voices.resume(handle, seconds) {
                    0
                } else {
                    -1
                }
            })
        })
    }

    fn stop_voice(&mut self, handle: u32, seconds: f32) -> i32 {
        self.with_state(|state| {
            state.controlled_voices().map_or(
                -1,
                |voices| {
                    if voices.stop(handle, seconds) { 0 } else { -1 }
                },
            )
        })
    }

    fn load_wav(&mut self, data: Vec<u8>, looped: bool) -> u32 {
        self.with_state(|state| state.load_wav(&data, looped))
    }

    fn unload_sound(&mut self, handle: u32) -> i32 {
        self.with_state(|state| if state.unload_sound(handle) { 0 } else { -1 })
    }

    fn begin_wav_upload(&mut self, total_bytes: u32, looped: bool) -> i32 {
        self.with_state(|state| {
            if state.begin_upload(
                PendingUploadKind::Wav { looped },
                total_bytes,
                MAX_WAV_UPLOAD_BYTES,
            ) {
                0
            } else {
                -1
            }
        })
    }

    fn append_wav_upload(&mut self, data: Vec<u8>) -> i32 {
        self.with_state(|state| {
            if state.append_upload(&data, true) {
                0
            } else {
                -1
            }
        })
    }

    fn finish_wav_upload(&mut self) -> u32 {
        self.with_state(WorkerState::finish_wav_upload)
    }

    fn begin_acoustic_scene_upload(&mut self, total_bytes: u32) -> i32 {
        self.with_state(|state| {
            if state.begin_upload(
                PendingUploadKind::AcousticScene,
                total_bytes,
                MAX_ACOUSTIC_SCENE_BYTES,
            ) {
                0
            } else {
                -1
            }
        })
    }

    fn append_acoustic_scene_upload(&mut self, data: Vec<u8>) -> i32 {
        self.with_state(|state| {
            if state.append_upload(&data, false) {
                0
            } else {
                -1
            }
        })
    }

    fn finish_acoustic_scene_upload(&mut self) -> i32 {
        self.with_state(|state| {
            if state.finish_acoustic_scene_upload() {
                0
            } else {
                -1
            }
        })
    }
}

/// Device-clock pump for the web worker host. It advances the same state owned
/// by the generated RPC service and performs at most one fixed quantum.
#[unsafe(no_mangle)]
pub extern "C" fn afterglow_audio_pump() -> i32 {
    WEB_WORKER_STATE.with(|slot| {
        let borrowed = slot.borrow();
        let Some(shared) = borrowed.as_ref() else {
            return -1;
        };
        let mut state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
        state.pump()
    })
}

/// Pointer to the latest fixed interleaved stereo quantum in worker memory.
#[unsafe(no_mangle)]
pub extern "C" fn afterglow_audio_pcm_ptr() -> usize {
    WEB_WORKER_STATE.with(|slot| {
        let borrowed = slot.borrow();
        let Some(shared) = borrowed.as_ref() else {
            return 0;
        };
        let state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
        state.pcm_ptr() as usize
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn afterglow_audio_pcm_samples() -> usize {
    PCM_SAMPLES
}

/// Bounded web-worker simulation tick. The host calls this only after the final
/// PCM ring reaches its target depth, so simulation and final mixing retain one
/// owner and one state machine without involving the AudioWorklet.
#[unsafe(no_mangle)]
pub extern "C" fn afterglow_audio_simulate_motion(phase: f32) -> i32 {
    WEB_WORKER_STATE.with(|slot| {
        let borrowed = slot.borrow();
        let Some(shared) = borrowed.as_ref() else {
            return -1;
        };
        let mut state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
        state.simulate_motion(phase)
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub mod native {
    use super::*;
    use afterglow_rpc::RpcError;
    use afterglow_rpc::native::{
        EventReceiver, RingConsumer, RingStorage, WorkerTransport, spawn_worker_loop_with_idle,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    const SEQUENCE_BYTES: usize = 4;
    const PCM_BYTES: usize = PCM_SAMPLES * size_of::<f32>();
    const PAYLOAD_BYTES: usize = SEQUENCE_BYTES + PCM_BYTES;
    const FRAME_BYTES: usize = 4 + PAYLOAD_BYTES;

    #[derive(Default)]
    pub struct NativeAudioTelemetry {
        armed: AtomicBool,
        rendered: AtomicU64,
        ring_full: AtomicU64,
        malformed: AtomicU64,
        sequence_errors: AtomicU64,
        sink_callbacks: AtomicU64,
        sink_underruns: AtomicU64,
        pump_nanos: AtomicU64,
        pump_max_nanos: AtomicU64,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct NativeAudioStats {
        pub rendered: u64,
        pub ring_full: u64,
        pub malformed: u64,
        pub sequence_errors: u64,
        pub sink_callbacks: u64,
        pub sink_underruns: u64,
        pub pump_nanos: u64,
        pub pump_max_nanos: u64,
    }

    impl NativeAudioTelemetry {
        /// Arm ring consumption immediately before starting the native stream.
        pub fn arm(&self) {
            self.armed.store(true, Ordering::Release);
        }

        pub fn snapshot(&self) -> NativeAudioStats {
            NativeAudioStats {
                rendered: self.rendered.load(Ordering::Relaxed),
                ring_full: self.ring_full.load(Ordering::Relaxed),
                malformed: self.malformed.load(Ordering::Relaxed),
                sequence_errors: self.sequence_errors.load(Ordering::Relaxed),
                sink_callbacks: self.sink_callbacks.load(Ordering::Relaxed),
                sink_underruns: self.sink_underruns.load(Ordering::Relaxed),
                pump_nanos: self.pump_nanos.load(Ordering::Relaxed),
                pump_max_nanos: self.pump_max_nanos.load(Ordering::Relaxed),
            }
        }
    }

    pub struct NativeAudioRuntime {
        pub client: EngineAudioServiceClient<WorkerTransport>,
        pub events: EventReceiver,
        pub reader: NativePcmReader,
        telemetry: Arc<NativeAudioTelemetry>,
    }

    impl NativeAudioRuntime {
        pub fn spawn(config: AudioWorkerConfig) -> RpcResult<Self> {
            WorkerState::validate(config)?;
            let pcm_storage = RingStorage::new(config.target_quanta as usize * FRAME_BYTES)?;
            let (pcm_producer, pcm_consumer) = pcm_storage.split();
            let telemetry = Arc::new(NativeAudioTelemetry::default());
            let worker_telemetry = telemetry.clone();
            let mut worker = EngineAudioWorker::default();
            if worker.configure(
                config.target_quanta,
                config.triangles,
                config.reflection_rays,
                config.reflection_bounces,
                config.reflection_duration_ms,
                config.reflection_order,
            ) != 0
            {
                return Err(RpcError::Server(
                    "native EngineAudio configuration failed".into(),
                ));
            }
            if worker.run_simulation() != 0 || worker.start() != 0 {
                return Err(RpcError::Server("native EngineAudio startup failed".into()));
            }
            let mut sequence = 0_u32;
            let mut payload = [0_u8; PAYLOAD_BYTES];
            let idle = move |worker: &mut EngineAudioWorker| {
                match pcm_producer.can_write(PAYLOAD_BYTES) {
                    Ok(true) => {}
                    Ok(false) => {
                        worker_telemetry.ring_full.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    Err(_) => {
                        worker_telemetry.malformed.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                }
                let started = Instant::now();
                let pumped = worker.with_state(WorkerState::pump);
                if pumped != 1 {
                    return;
                }
                payload[..SEQUENCE_BYTES].copy_from_slice(&sequence.to_le_bytes());
                let mut pcm = [0.0_f32; PCM_SAMPLES];
                worker.with_state(|state| state.copy_pcm_to(&mut pcm));
                for (index, sample) in pcm.iter().enumerate() {
                    let offset = SEQUENCE_BYTES + index * size_of::<f32>();
                    payload[offset..offset + size_of::<f32>()]
                        .copy_from_slice(&sample.to_le_bytes());
                }
                if pcm_producer.write(&payload).is_err() {
                    worker_telemetry.ring_full.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                sequence = sequence.wrapping_add(1);
                worker_telemetry.rendered.fetch_add(1, Ordering::Relaxed);
                let nanos = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                worker_telemetry
                    .pump_nanos
                    .fetch_add(nanos, Ordering::Relaxed);
                worker_telemetry
                    .pump_max_nanos
                    .fetch_max(nanos, Ordering::Relaxed);
            };
            let (transport, events) = spawn_worker_loop_with_idle(
                worker,
                1 << 20,
                |service, method, args| service.serve(method, args),
                idle,
                Duration::from_micros(250),
            )?;
            Ok(Self {
                client: EngineAudioServiceClient::new(transport),
                events,
                reader: NativePcmReader {
                    consumer: pcm_consumer,
                    telemetry: telemetry.clone(),
                    expected_sequence: 0,
                    payload: [0; PAYLOAD_BYTES],
                    decoded: [0.0; PCM_SAMPLES],
                    decoded_cursor: PCM_SAMPLES,
                },
                telemetry,
            })
        }

        pub fn telemetry(&self) -> Arc<NativeAudioTelemetry> {
            self.telemetry.clone()
        }
    }

    pub struct NativePcmReader {
        consumer: RingConsumer,
        telemetry: Arc<NativeAudioTelemetry>,
        expected_sequence: u32,
        payload: [u8; PAYLOAD_BYTES],
        decoded: [f32; PCM_SAMPLES],
        decoded_cursor: usize,
    }

    impl NativePcmReader {
        fn load_quantum(&mut self) -> bool {
            match self.consumer.read_into(&mut self.payload) {
                Ok(length) if length == PAYLOAD_BYTES => {}
                Ok(_) => {
                    self.telemetry.malformed.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
                Err(RpcError::BufferEmpty) => {
                    self.telemetry
                        .sink_underruns
                        .fetch_add(1, Ordering::Relaxed);
                    return false;
                }
                Err(_) => {
                    self.telemetry.malformed.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
            }
            let sequence = u32::from_le_bytes(self.payload[..4].try_into().unwrap());
            if sequence != self.expected_sequence {
                self.telemetry
                    .sequence_errors
                    .fetch_add(1, Ordering::Relaxed);
                return false;
            }
            self.expected_sequence = self.expected_sequence.wrapping_add(1);
            for (index, sample) in self.decoded.iter_mut().enumerate() {
                let offset = SEQUENCE_BYTES + index * size_of::<f32>();
                *sample = f32::from_le_bytes(
                    self.payload[offset..offset + size_of::<f32>()]
                        .try_into()
                        .unwrap(),
                );
            }
            self.decoded_cursor = 0;
            true
        }

        /// Fill an interleaved stereo device buffer, preserving partial-quantum
        /// state when the native backend requests a different callback size.
        /// Empty or malformed input emits silence; this path does not allocate.
        pub fn read_interleaved(&mut self, output: &mut [f32]) -> bool {
            if !self.telemetry.armed.load(Ordering::Acquire) {
                output.fill(0.0);
                return true;
            }
            self.telemetry
                .sink_callbacks
                .fetch_add(1, Ordering::Relaxed);
            if output.is_empty() || output.len() % 2 != 0 {
                output.fill(0.0);
                self.telemetry.malformed.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            let mut output_cursor = 0;
            while output_cursor < output.len() {
                if self.decoded_cursor == PCM_SAMPLES && !self.load_quantum() {
                    output[output_cursor..].fill(0.0);
                    return false;
                }
                let count = (PCM_SAMPLES - self.decoded_cursor).min(output.len() - output_cursor);
                output[output_cursor..output_cursor + count].copy_from_slice(
                    &self.decoded[self.decoded_cursor..self.decoded_cursor + count],
                );
                self.decoded_cursor += count;
                output_cursor += count;
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn pcm16_wav(samples: &[i16]) -> Vec<u8> {
        let data_len = samples.len() * 2;
        let mut wav = Vec::with_capacity(44 + data_len);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
        wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data_len as u32).to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    #[cfg(feature = "steam-audio")]
    fn acoustic_fixture() -> Vec<u8> {
        let vertices = [
            [-2.0f32, 0.0, -2.0],
            [2.0, 0.0, -2.0],
            [2.0, 0.0, 2.0],
            [-2.0, 0.0, 2.0],
        ];
        let indices = [0u32, 1, 2, 0, 2, 3];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"AGBIST1\0");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&(vertices.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&6u32.to_le_bytes());
        for value in [
            -2.0f32, 0.0, -2.0, 2.0, 0.0, 2.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for vertex in vertices {
            for value in vertex {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        bytes
    }

    fn configured_worker(target_quanta: u32) -> EngineAudioWorker {
        let mut worker = EngineAudioWorker::default();
        let mut config = AudioWorkerConfig::default();
        config.target_quanta = target_quanta;
        assert_eq!(
            worker.configure(
                config.target_quanta,
                config.triangles,
                config.reflection_rays,
                config.reflection_bounces,
                config.reflection_duration_ms,
                config.reflection_order,
            ),
            0
        );
        worker
    }

    #[test]
    fn generated_native_rpc_worker_runs_on_the_os_thread_backend() {
        let _guard = serial();
        let (client, _events) =
            EngineAudioServiceClient::spawn_worker(EngineAudioWorker::default()).unwrap();
        assert_eq!(client.configure(2, 10_000, 512, 2, 500, 0).unwrap(), 0);
        let sound = client
            .load_wav(pcm16_wav(&[4_096; QUANTUM_FRAMES]), false)
            .unwrap();
        assert_ne!(sound, INVALID_SOUND_HANDLE);
        assert_eq!(client.run_simulation().unwrap(), 0);
        assert_eq!(client.start().unwrap(), 0);
        let stats = client.stats().unwrap();
        assert_eq!(stats[2], 1.0);
        assert_eq!(stats[13], 1.0);
        assert_eq!(stats[23], 1.0);
        assert_eq!(stats[24], (QUANTUM_FRAMES * 4) as f64);
        assert_eq!(client.stop().unwrap(), 0);
        assert_eq!(client.shutdown().unwrap(), 0);
    }

    #[test]
    fn native_render_ahead_ring_handles_partial_device_callbacks() {
        let _guard = serial();
        let mut config = AudioWorkerConfig::default();
        config.target_quanta = 4;
        let runtime = native::NativeAudioRuntime::spawn(config).unwrap();
        let telemetry = runtime.telemetry();
        let mut reader = runtime.reader;
        let client = runtime.client;
        let _events = runtime.events;
        std::thread::sleep(std::time::Duration::from_millis(10));
        telemetry.arm();
        let mut first_half = [0.0_f32; PCM_SAMPLES / 2];
        let mut second_half = [0.0_f32; PCM_SAMPLES / 2];
        assert!(afterglow_rpc::allocation::assert_no_alloc(
            || reader.read_interleaved(&mut first_half)
        ));
        assert!(afterglow_rpc::allocation::assert_no_alloc(
            || reader.read_interleaved(&mut second_half)
        ));
        assert!(first_half.iter().any(|sample| *sample != 0.0));
        assert!(second_half.iter().any(|sample| *sample != 0.0));
        let stats = telemetry.snapshot();
        assert_eq!(stats.sink_underruns, 0);
        assert_eq!(stats.sequence_errors, 0);
        assert_eq!(client.stop().unwrap(), 0);
        assert_eq!(client.shutdown().unwrap(), 0);
    }

    #[test]
    fn rejects_invalid_render_ahead_depths() {
        let _guard = serial();
        let mut worker = EngineAudioWorker::default();
        for depth in [0, 1, 9, u32::MAX] {
            let mut config = AudioWorkerConfig::default();
            config.target_quanta = depth;
            assert_ne!(
                worker.configure(
                    config.target_quanta,
                    config.triangles,
                    config.reflection_rays,
                    config.reflection_bounces,
                    config.reflection_duration_ms,
                    config.reflection_order,
                ),
                0
            );
        }
    }

    #[test]
    fn one_worker_owns_all_producer_kinds_and_one_sample_clock() {
        let _guard = serial();
        let mut worker = configured_worker(2);
        assert_eq!(worker.start(), 0);
        assert_eq!(worker.with_state(WorkerState::pump), 1);
        let stats = worker.with_state(|state| state.stats());
        assert_eq!(stats.sample_clock, QUANTUM_FRAMES as u64);
        assert_eq!(stats.rendered_quanta, 1);
        assert_eq!(stats.voice_count, VOICE_CAPACITY);
        assert_eq!(stats.reflection_voice_count, REFLECTION_CAPACITY);
        assert_eq!(stats.resident_voices, VOICE_CAPACITY / 4);
        assert_eq!(stats.asset_stream_voices, VOICE_CAPACITY / 4);
        assert_eq!(stats.live_voices, VOICE_CAPACITY / 4);
        assert_eq!(stats.procedural_voices, VOICE_CAPACITY / 4);
        assert_eq!(stats.active_spatial_voices, PHYSICAL_VOICE_CAPACITY);
        assert!(stats.output_energy > 0.0);
        assert!(stats.output_peak > 0.0);
    }

    #[test]
    fn stop_and_fatal_state_never_emit_an_extra_quantum() {
        let _guard = serial();
        let mut worker = configured_worker(4);
        assert_eq!(worker.with_state(WorkerState::pump), 0);
        assert_eq!(worker.start(), 0);
        assert_eq!(worker.with_state(WorkerState::pump), 1);
        assert_eq!(worker.stop(), 0);
        assert_eq!(worker.with_state(WorkerState::pump), 0);
        assert_eq!(worker.with_state(|state| state.stats()).rendered_quanta, 1);
    }

    #[test]
    fn reconfigure_replaces_backend_before_restart() {
        let _guard = serial();
        let mut worker = configured_worker(2);
        assert_eq!(worker.start(), 0);
        assert_eq!(worker.with_state(WorkerState::pump), 1);
        let config = AudioWorkerConfig::default();
        assert_eq!(
            worker.configure(
                config.target_quanta,
                config.triangles,
                config.reflection_rays,
                config.reflection_bounces,
                config.reflection_duration_ms,
                config.reflection_order,
            ),
            0
        );
        let stats = worker.with_state(|state| state.stats());
        assert_eq!(stats.sample_clock, 0);
        assert!(!stats.running);
        assert_eq!(worker.with_state(WorkerState::pump), 0);
    }

    #[cfg(feature = "steam-audio")]
    #[test]
    fn chunked_warmup_uploads_sound_and_acoustic_scene() {
        let _guard = serial();
        let mut worker = configured_worker(2);
        assert_eq!(worker.begin_wav_upload(4, false), 0);
        assert_eq!(worker.append_wav_upload(vec![1, 2]), 0);
        assert_eq!(worker.finish_wav_upload(), INVALID_SOUND_HANDLE);
        assert_ne!(worker.begin_acoustic_scene_upload(0), 0);
        let scene = acoustic_fixture();
        assert_eq!(worker.begin_acoustic_scene_upload(scene.len() as u32), 0);
        assert_ne!(worker.begin_wav_upload(44, false), 0);
        assert_eq!(worker.append_acoustic_scene_upload(scene[..61].to_vec()), 0);
        assert_eq!(worker.append_acoustic_scene_upload(scene[61..].to_vec()), 0);
        assert_eq!(worker.finish_acoustic_scene_upload(), 0);
        let scene_stats = worker.with_state(|state| state.stats());
        assert_eq!(scene_stats.acoustic_vertices, 4);
        assert_eq!(scene_stats.acoustic_triangles, 2);
        assert_eq!(scene_stats.acoustic_scene_bytes, scene.len() as u64);

        let wav = pcm16_wav(&[2_048; QUANTUM_FRAMES]);
        assert_eq!(worker.begin_wav_upload(wav.len() as u32, false), 0);
        assert_eq!(worker.append_wav_upload(wav[..17].to_vec()), 0);
        assert_eq!(worker.append_wav_upload(wav[17..].to_vec()), 0);
        let sound = worker.finish_wav_upload();
        assert_ne!(sound, INVALID_SOUND_HANDLE);
        assert_eq!(
            worker
                .with_state(|state| state.stats())
                .loaded_resident_sounds,
            1
        );
    }

    #[test]
    fn resident_wav_loads_before_start_and_releases_at_end() {
        let _guard = serial();
        let mut worker = configured_worker(2);
        assert_eq!(worker.load_wav(vec![1, 2, 3], false), INVALID_SOUND_HANDLE);
        let wav = pcm16_wav(&[8_192; QUANTUM_FRAMES * 2]);
        let sound = worker.load_wav(wav, false);
        assert_ne!(sound, INVALID_SOUND_HANDLE);
        let loaded = worker.with_state(|state| state.stats());
        assert_eq!(loaded.loaded_resident_sounds, 1);
        assert_eq!(loaded.resident_sound_bytes, (QUANTUM_FRAMES * 2 * 4) as u64);
        assert_eq!(worker.start(), 0);
        let voice = worker.spawn_2d(sound);
        assert_ne!(voice, INVALID_VOICE_HANDLE);
        assert_eq!(worker.with_state(WorkerState::pump), 1);
        assert!(worker.with_state(|state| state.stats()).output_energy > 0.0);
        assert_eq!(worker.with_state(WorkerState::pump), 1);
        assert_eq!(worker.with_state(|state| state.stats()).active_voices, 0);
        assert_eq!(worker.stop(), 0);
        assert_eq!(worker.unload_sound(sound), 0);
        assert_ne!(worker.unload_sound(sound), 0);
    }

    #[test]
    fn rpc_voice_controls_require_running_and_share_sample_clock() {
        let _guard = serial();
        let mut worker = configured_worker(8);
        assert_eq!(worker.spawn_2d(1), INVALID_VOICE_HANDLE);
        assert_eq!(worker.start(), 0);
        let first = worker.spawn_2d(1);
        assert_ne!(first, INVALID_VOICE_HANDLE);
        let second = worker.crossfade_to(first, 2, 0.01);
        assert_ne!(second, INVALID_VOICE_HANDLE);
        assert_eq!(worker.set_voice_volume(second, 0.5, 0.01), 0);
        for _ in 0..4 {
            assert_eq!(worker.with_state(WorkerState::pump), 1);
        }
        let stats = worker.with_state(|state| state.stats());
        assert_eq!(stats.active_voices, 1);
        assert_eq!(stats.completed_voice_fades, 1);
        assert_eq!(worker.pause_voice(second, 0.0), 0);
        assert_eq!(worker.resume_voice(second, 0.0), 0);
        assert_eq!(worker.stop_voice(second, 0.0), 0);
        assert_eq!(worker.with_state(|state| state.stats()).active_voices, 0);
    }

    #[test]
    fn worker_owned_simulation_tick_requires_running_state() {
        let _guard = serial();
        let mut worker = configured_worker(8);
        assert_eq!(worker.with_state(|state| state.simulate_motion(0.25)), -1);
        assert_eq!(worker.start(), 0);
        assert_eq!(worker.with_state(|state| state.simulate_motion(0.25)), 0);
        assert_eq!(
            worker.with_state(|state| state.stats()).simulation_updates,
            1
        );
    }

    #[test]
    fn configuration_and_motion_fail_closed() {
        let _guard = serial();
        let mut worker = configured_worker(3);
        assert_ne!(worker.update_motion(f32::NAN), 0);
        assert!(!worker.with_state(|state| state.stats()).fatal);
        assert_eq!(worker.run_simulation(), 0);
        assert_eq!(
            worker.with_state(|state| state.stats()).simulation_updates,
            1
        );
    }
}
