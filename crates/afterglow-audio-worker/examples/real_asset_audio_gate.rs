use afterglow_audio_worker::native::NativeAudioRuntime;
use afterglow_audio_worker::{
    AudioWorkerConfig, EngineAudioServiceClient, QUANTUM_FRAMES, SAMPLE_RATE,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const CHUNK_BYTES: usize = 512 * 1024;
const RUN_SECONDS: usize = 10;
const SOUND_NAMES: [&str; 5] = [
    "abcd.wav",
    "counting.wav",
    "impulse.wav",
    "ozymandias.wav",
    "pinknoise.wav",
];
const SCENE_NAMES: [&str; 3] = ["BistroExterior", "BistroInterior", "BistroInterior_Wine"];

fn status(value: i32, operation: &str) -> Result<(), String> {
    if value == 0 {
        Ok(())
    } else {
        Err(format!("{operation} failed with status {value}"))
    }
}

fn upload_scene(
    client: &EngineAudioServiceClient<afterglow_rpc::native::WorkerTransport>,
    bytes: &[u8],
) -> Result<(), String> {
    let total = u32::try_from(bytes.len()).map_err(|_| "scene exceeds u32".to_string())?;
    status(
        client
            .begin_acoustic_scene_upload(total)
            .map_err(|error| error.to_string())?,
        "begin scene upload",
    )?;
    for chunk in bytes.chunks(CHUNK_BYTES) {
        status(
            client
                .append_acoustic_scene_upload(chunk.to_vec())
                .map_err(|error| error.to_string())?,
            "append scene upload",
        )?;
    }
    status(
        client
            .finish_acoustic_scene_upload()
            .map_err(|error| error.to_string())?,
        "finish scene upload",
    )
}

fn upload_sound(
    client: &EngineAudioServiceClient<afterglow_rpc::native::WorkerTransport>,
    bytes: &[u8],
) -> Result<u32, String> {
    let total = u32::try_from(bytes.len()).map_err(|_| "sound exceeds u32".to_string())?;
    status(
        client
            .begin_wav_upload(total, true)
            .map_err(|error| error.to_string())?,
        "begin WAV upload",
    )?;
    for chunk in bytes.chunks(CHUNK_BYTES) {
        status(
            client
                .append_wav_upload(chunk.to_vec())
                .map_err(|error| error.to_string())?,
            "append WAV upload",
        )?;
    }
    let handle = client
        .finish_wav_upload()
        .map_err(|error| error.to_string())?;
    if handle == 0 {
        Err("finish WAV upload rejected the sound".into())
    } else {
        Ok(handle)
    }
}

fn f32_at(bytes: &[u8], offset: usize) -> Result<f32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated acoustic header".to_string())?;
    Ok(f32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn source_position(bytes: &[u8]) -> Result<[f32; 3], String> {
    if bytes.get(0..8) != Some(b"AGBIST1\0") {
        return Err("invalid acoustic magic".into());
    }
    Ok([f32_at(bytes, 60)?, f32_at(bytes, 64)?, f32_at(bytes, 68)?])
}

fn run_scene(scene_path: &Path, sound_dir: &Path) -> Result<String, String> {
    let scene_bytes = fs::read(scene_path).map_err(|error| error.to_string())?;
    let source = source_position(&scene_bytes)?;
    let mut config = AudioWorkerConfig::default();
    config.target_quanta = std::env::var("AFTERGLOW_AUDIO_QUANTA")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    config.reflection_rays = 512;
    config.reflection_bounces = 2;
    config.reflection_duration_ms = 500;
    let runtime = NativeAudioRuntime::spawn(config).map_err(|error| error.to_string())?;
    let telemetry = runtime.telemetry();
    let mut reader = runtime.reader;
    let client = runtime.client;
    let _events = runtime.events;
    status(
        client.stop().map_err(|error| error.to_string())?,
        "stop before warm-up",
    )?;
    let mut discard = [0.0f32; QUANTUM_FRAMES * 2];
    for _ in 0..8 {
        reader.read_interleaved(&mut discard);
    }

    upload_scene(&client, &scene_bytes)?;
    let mut sounds = [0u32; SOUND_NAMES.len()];
    for (index, name) in SOUND_NAMES.iter().enumerate() {
        let bytes = fs::read(sound_dir.join(name)).map_err(|error| format!("{name}: {error}"))?;
        sounds[index] = upload_sound(&client, &bytes)?;
    }
    status(
        client.run_simulation().map_err(|error| error.to_string())?,
        "initial real-scene simulation",
    )?;
    status(client.start().map_err(|error| error.to_string())?, "start")?;
    for index in 0..4 {
        let handle = client
            .spawn_at(
                sounds[index],
                source[0] + index as f32 * 0.12,
                source[1],
                source[2],
            )
            .map_err(|error| error.to_string())?;
        if handle == 0 {
            return Err(format!("physical voice {index} rejected"));
        }
        status(
            client
                .set_voice_volume(handle, 0.2, 0.0)
                .map_err(|error| error.to_string())?,
            "physical voice gain",
        )?;
    }
    let dry = client
        .spawn_2d(sounds[4])
        .map_err(|error| error.to_string())?;
    if dry == 0 {
        return Err("dry voice rejected".into());
    }
    status(
        client
            .set_voice_volume(dry, 0.2, 0.0)
            .map_err(|error| error.to_string())?,
        "dry voice gain",
    )?;

    // Discard startup quanta so measurements contain only the controlled real
    // sounds and real scene, not the pre-control diagnostic mix.
    std::thread::sleep(Duration::from_millis(20));
    for _ in 0..16 {
        reader.read_interleaved(&mut discard);
    }
    telemetry.arm();

    let callbacks = RUN_SECONDS * SAMPLE_RATE as usize / QUANTUM_FRAMES;
    // Match production ownership: device consumption remains on an independent
    // thread while this control thread performs potentially blocking simulation.
    let sink_thread = std::thread::spawn(move || {
        let period = Duration::from_secs_f64(QUANTUM_FRAMES as f64 / SAMPLE_RATE as f64);
        let mut next = Instant::now();
        let mut block = [0.0f32; QUANTUM_FRAMES * 2];
        let mut sum_squares = 0.0f64;
        let mut peak = 0.0f32;
        let mut nonzero_samples = 0u64;
        let mut longest_zero_frames = 0u64;
        let mut current_zero_frames = 0u64;
        for _ in 0..callbacks {
            next += period;
            let now = Instant::now();
            if next > now {
                let remaining = next - now;
                if remaining > Duration::from_micros(200) {
                    std::thread::sleep(remaining - Duration::from_micros(100));
                }
                while Instant::now() < next {
                    std::hint::spin_loop();
                }
            }
            reader.read_interleaved(&mut block);
            for sample in block {
                let absolute = sample.abs();
                peak = peak.max(absolute);
                sum_squares += f64::from(sample) * f64::from(sample);
                nonzero_samples += u64::from(sample != 0.0);
            }
            for frame in block.chunks_exact(2) {
                if frame[0] == 0.0 && frame[1] == 0.0 {
                    current_zero_frames += 1;
                    longest_zero_frames = longest_zero_frames.max(current_zero_frames);
                } else {
                    current_zero_frames = 0;
                }
            }
        }
        (sum_squares, peak, nonzero_samples, longest_zero_frames)
    });
    for update in 1..RUN_SECONDS {
        std::thread::sleep(Duration::from_secs(1));
        let phase = update as f32 / RUN_SECONDS as f32;
        status(
            client
                .update_motion(phase)
                .map_err(|error| error.to_string())?,
            "motion",
        )?;
        status(
            client.run_simulation().map_err(|error| error.to_string())?,
            "simulation",
        )?;
    }
    let (sum_squares, peak, nonzero_samples, longest_zero_frames) = sink_thread
        .join()
        .map_err(|_| "native sink thread panicked".to_string())?;
    let worker = client.stats().map_err(|error| error.to_string())?;
    let sink = telemetry.snapshot();
    status(client.stop().map_err(|error| error.to_string())?, "stop")?;
    status(
        client.shutdown().map_err(|error| error.to_string())?,
        "shutdown",
    )?;
    let sample_count = (callbacks * QUANTUM_FRAMES * 2) as f64;
    let rms = (sum_squares / sample_count).sqrt();
    let scene_name = scene_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    Ok(format!(
        "{{\"scene\":\"{scene_name}\",\"targetQuanta\":{},\"sceneBytes\":{},\"vertices\":{},\"triangles\":{},\"sounds\":{},\"residentBytes\":{},\"seconds\":{},\"callbacks\":{},\"underruns\":{},\"sequenceErrors\":{},\"pumpMeanMs\":{},\"pumpMaxMs\":{},\"simulationUpdates\":{},\"nonzeroSamples\":{},\"rms\":{},\"peak\":{},\"longestZeroMs\":{}}}",
        worker[6],
        worker[27],
        worker[25],
        worker[26],
        worker[23],
        worker[24],
        RUN_SECONDS,
        sink.sink_callbacks,
        sink.sink_underruns,
        sink.sequence_errors,
        sink.pump_nanos as f64 / sink.rendered.max(1) as f64 / 1_000_000.0,
        sink.pump_max_nanos as f64 / 1_000_000.0,
        worker[2],
        nonzero_samples,
        rms,
        peak,
        longest_zero_frames as f64 * 1000.0 / SAMPLE_RATE as f64,
    ))
}

fn main() -> Result<(), String> {
    let mut arguments = std::env::args_os().skip(1);
    let scene_dir = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: real_asset_audio_gate SCENE_DIR SOUND_DIR [OUTPUT]")?,
    );
    let sound_dir = PathBuf::from(arguments.next().ok_or("missing SOUND_DIR")?);
    let output = arguments.next().map(PathBuf::from);
    let mut results = Vec::new();
    for scene in SCENE_NAMES {
        eprintln!("testing real sounds in {scene}...");
        results.push(run_scene(
            &scene_dir.join(format!("{scene}.acoustic.bin")),
            &sound_dir,
        )?);
    }
    let json = format!("{{\"schemaVersion\":1,\"runs\":[{}]}}\n", results.join(","));
    if let Some(path) = output {
        fs::write(path, &json).map_err(|error| error.to_string())?;
    }
    print!("{json}");
    Ok(())
}
