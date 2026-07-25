use afterglow_audio_worker::AudioWorkerConfig;
use afterglow_audio_worker::native::NativeAudioRuntime;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, SampleRate, StreamConfig};
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

#[derive(Default)]
struct WaveformStats {
    samples: AtomicU64,
    nonzero_samples: AtomicU64,
    flat_callbacks: AtomicU64,
    clipped_samples: AtomicU64,
    callback_errors: AtomicU64,
    sum_squares_bits: AtomicU64,
    peak_bits: AtomicU32,
}

impl WaveformStats {
    fn observe(&self, samples: &[f32]) {
        let mut sum_squares = 0.0_f64;
        let mut peak = 0.0_f32;
        let mut nonzero = 0_u64;
        let mut clipped = 0_u64;
        for &sample in samples {
            let absolute = sample.abs();
            peak = peak.max(absolute);
            sum_squares += f64::from(sample) * f64::from(sample);
            nonzero += u64::from(absolute > 1.0e-7);
            clipped += u64::from(absolute >= 0.999);
        }
        self.samples
            .fetch_add(samples.len() as u64, Ordering::Relaxed);
        self.nonzero_samples.fetch_add(nonzero, Ordering::Relaxed);
        self.clipped_samples.fetch_add(clipped, Ordering::Relaxed);
        if nonzero == 0 {
            self.flat_callbacks.fetch_add(1, Ordering::Relaxed);
        }
        let mut prior = self.sum_squares_bits.load(Ordering::Relaxed);
        loop {
            let next = (f64::from_bits(prior) + sum_squares).to_bits();
            match self.sum_squares_bits.compare_exchange_weak(
                prior,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => prior = actual,
            }
        }
        let mut prior_peak = self.peak_bits.load(Ordering::Relaxed);
        while f32::from_bits(prior_peak) < peak {
            match self.peak_bits.compare_exchange_weak(
                prior_peak,
                peak.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => prior_peak = actual,
            }
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let seconds = std::env::args()
        .nth(1)
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(10);
    let depth = std::env::args()
        .nth(2)
        .map(|value| value.parse::<u32>())
        .transpose()?
        .unwrap_or(8);
    let mut config = AudioWorkerConfig::default();
    config.target_quanta = depth;
    let runtime = NativeAudioRuntime::spawn(config)?;
    let telemetry = runtime.telemetry();
    let mut reader = runtime.reader;
    let client = runtime.client;
    let _events = runtime.events;

    // Give the native worker time to fill the bounded render-ahead ring before
    // the physical device starts consuming it.
    std::thread::sleep(Duration::from_millis(30));

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default native output device")?;
    let device_name = device.name()?;
    let supports = device.supported_output_configs()?.any(|range| {
        range.sample_format() == SampleFormat::F32
            && range.channels() == 2
            && range.min_sample_rate() <= SampleRate(48_000)
            && range.max_sample_rate() >= SampleRate(48_000)
    });
    if !supports {
        return Err("default device lacks stereo f32 48 kHz output".into());
    }
    let stream_config = StreamConfig {
        channels: 2,
        sample_rate: SampleRate(48_000),
        buffer_size: BufferSize::Fixed(128),
    };
    let waveform = Arc::new(WaveformStats::default());
    let callback_waveform = waveform.clone();
    let error_waveform = waveform.clone();
    let stream = device.build_output_stream(
        &stream_config,
        move |output: &mut [f32], _| {
            if !reader.read_interleaved(output) {
                // The reader already emitted deterministic silence.
            }
            for sample in output.iter_mut() {
                *sample *= 0.10;
            }
            callback_waveform.observe(output);
        },
        move |error| {
            error_waveform
                .callback_errors
                .fetch_add(1, Ordering::Relaxed);
            eprintln!("native audio device error: {error}");
        },
        None,
    )?;
    telemetry.arm();
    stream.play()?;
    std::thread::sleep(Duration::from_secs(seconds));
    stream.pause()?;
    drop(stream);

    let worker = telemetry.snapshot();
    let samples = waveform.samples.load(Ordering::Relaxed);
    let sum_squares = f64::from_bits(waveform.sum_squares_bits.load(Ordering::Relaxed));
    let rms = if samples == 0 {
        0.0
    } else {
        (sum_squares / samples as f64).sqrt()
    };
    let peak = f32::from_bits(waveform.peak_bits.load(Ordering::Relaxed));
    let rpc_stats = client.stats()?;
    client.stop()?;
    client.shutdown()?;

    println!(
        "NATIVE_AUDIO_GATE {{\"device\":{:?},\"seconds\":{},\"depth\":{},\"samples\":{},\"nonzeroSamples\":{},\"rms\":{:.9},\"peak\":{:.9},\"flatCallbacks\":{},\"clippedSamples\":{},\"deviceErrors\":{},\"rendered\":{},\"callbacks\":{},\"underruns\":{},\"sequenceErrors\":{},\"malformed\":{},\"ringFullPolls\":{},\"pumpMeanMs\":{:.6},\"pumpMaxMs\":{:.6},\"workerSampleClock\":{},\"activeReflectionVoices\":{}}}",
        device_name,
        seconds,
        depth,
        samples,
        waveform.nonzero_samples.load(Ordering::Relaxed),
        rms,
        peak,
        waveform.flat_callbacks.load(Ordering::Relaxed),
        waveform.clipped_samples.load(Ordering::Relaxed),
        waveform.callback_errors.load(Ordering::Relaxed),
        worker.rendered,
        worker.sink_callbacks,
        worker.sink_underruns,
        worker.sequence_errors,
        worker.malformed,
        worker.ring_full,
        if worker.rendered == 0 {
            0.0
        } else {
            worker.pump_nanos as f64 / worker.rendered as f64 / 1_000_000.0
        },
        worker.pump_max_nanos as f64 / 1_000_000.0,
        rpc_stats[0],
        rpc_stats[16],
    );
    if samples == 0
        || waveform.nonzero_samples.load(Ordering::Relaxed) == 0
        || rms <= 1.0e-6
        || peak <= 1.0e-5
        || worker.sink_underruns != 0
        || worker.sequence_errors != 0
        || worker.malformed != 0
        || waveform.callback_errors.load(Ordering::Relaxed) != 0
    {
        return Err("native audio waveform/glitch acceptance failed".into());
    }
    Ok(())
}
