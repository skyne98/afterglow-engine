//! Fixed-capacity resident sound ownership and strict 48 kHz WAV decoding.
//!
//! Loading is a bootstrap/warm-up operation. Playback only reads stable boxed
//! PCM and therefore performs no allocation or format conversion.

use crate::SAMPLE_RATE;

pub(crate) const INVALID_SOUND_HANDLE: u32 = 0;
pub(crate) const SOUND_CAPACITY: usize = 64;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const RESIDENT_SOUND_BYTE_CAPACITY: usize = 256 * 1024 * 1024;
#[cfg(target_arch = "wasm32")]
pub(crate) const RESIDENT_SOUND_BYTE_CAPACITY: usize = 32 * 1024 * 1024;

const INDEX_BITS: u32 = 8;
const INDEX_MASK: u32 = (1 << INDEX_BITS) - 1;
const MAX_GENERATION: u32 = (1 << (32 - INDEX_BITS)) - 1;

#[derive(Debug)]
struct ResidentSound {
    samples: Box<[f32]>,
    frames: u32,
    channels: u32,
    looped: bool,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(feature = "steam-audio"), allow(dead_code))]
pub(crate) struct ResidentSoundView {
    pub handle: u32,
    pub samples: *const f32,
    pub frames: u32,
    pub channels: u32,
    pub looped: bool,
}

pub(crate) struct SoundBank {
    slots: [Option<ResidentSound>; SOUND_CAPACITY],
    generations: [u32; SOUND_CAPACITY],
    used_bytes: usize,
}

impl SoundBank {
    pub(crate) fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            generations: [1; SOUND_CAPACITY],
            used_bytes: 0,
        }
    }

    pub(crate) fn load_wav(&mut self, bytes: &[u8], looped: bool) -> u32 {
        let Some(index) = self.slots.iter().position(Option::is_none) else {
            return INVALID_SOUND_HANDLE;
        };
        let Ok(decoded) = decode_wav(bytes) else {
            return INVALID_SOUND_HANDLE;
        };
        let byte_len = decoded.samples.len().saturating_mul(size_of::<f32>());
        if byte_len > RESIDENT_SOUND_BYTE_CAPACITY.saturating_sub(self.used_bytes) {
            return INVALID_SOUND_HANDLE;
        }
        self.used_bytes += byte_len;
        self.slots[index] = Some(ResidentSound {
            samples: decoded.samples.into_boxed_slice(),
            frames: decoded.frames,
            channels: decoded.channels,
            looped,
        });
        encode_handle(index, self.generations[index])
    }

    pub(crate) fn unload(&mut self, handle: u32) -> bool {
        let Some(index) = self.resolve(handle) else {
            return false;
        };
        let sound = self.slots[index].take().expect("resolved resident sound");
        self.used_bytes -= sound.samples.len() * size_of::<f32>();
        self.generations[index] = next_generation(self.generations[index]);
        true
    }

    pub(crate) fn contains(&self, handle: u32) -> bool {
        self.resolve(handle).is_some()
    }

    pub(crate) fn view(&self, handle: u32) -> Option<ResidentSoundView> {
        let index = self.resolve(handle)?;
        let sound = self.slots[index].as_ref()?;
        Some(ResidentSoundView {
            handle,
            samples: sound.samples.as_ptr(),
            frames: sound.frames,
            channels: sound.channels,
            looped: sound.looped,
        })
    }

    pub(crate) fn playback_length(&self, handle: u32) -> Option<(u32, bool)> {
        let index = self.resolve(handle)?;
        let sound = self.slots[index].as_ref()?;
        Some((sound.frames, sound.looped))
    }

    pub(crate) fn sample(&self, handle: u32, frame: u64, channel: u32) -> f32 {
        let Some(index) = self.resolve(handle) else {
            return 0.0;
        };
        let sound = self.slots[index].as_ref().expect("resolved resident sound");
        let frame = if sound.looped {
            frame % u64::from(sound.frames)
        } else if frame >= u64::from(sound.frames) {
            return 0.0;
        } else {
            frame
        };
        let channel = channel.min(sound.channels - 1);
        sound.samples[frame as usize * sound.channels as usize + channel as usize]
    }

    pub(crate) fn loaded_count(&self) -> u32 {
        self.slots.iter().filter(|slot| slot.is_some()).count() as u32
    }

    pub(crate) fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    fn resolve(&self, handle: u32) -> Option<usize> {
        let encoded_index = handle & INDEX_MASK;
        let generation = handle >> INDEX_BITS;
        if encoded_index == 0 || generation == 0 {
            return None;
        }
        let index = (encoded_index - 1) as usize;
        if index >= SOUND_CAPACITY
            || self.generations[index] != generation
            || self.slots[index].is_none()
        {
            return None;
        }
        Some(index)
    }
}

struct DecodedWav {
    samples: Vec<f32>,
    frames: u32,
    channels: u32,
}

fn decode_wav(bytes: &[u8]) -> Result<DecodedWav, ()> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(());
    }
    let declared_len = read_u32(bytes, 4)? as usize;
    if declared_len
        .checked_add(8)
        .is_none_or(|length| length > bytes.len())
    {
        return Err(());
    }
    let mut format = None;
    let mut data = None;
    let mut offset = 12usize;
    while offset.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let id = &bytes[offset..offset + 4];
        let size = read_u32(bytes, offset + 4)? as usize;
        let start = offset + 8;
        let end = start.checked_add(size).ok_or(())?;
        if end > bytes.len() {
            return Err(());
        }
        if id == b"fmt " {
            if size < 16 {
                return Err(());
            }
            format = Some((
                read_u16(bytes, start)?,
                read_u16(bytes, start + 2)?,
                read_u32(bytes, start + 4)?,
                read_u16(bytes, start + 12)?,
                read_u16(bytes, start + 14)?,
            ));
        } else if id == b"data" {
            data = Some(&bytes[start..end]);
        }
        offset = end.checked_add(size & 1).ok_or(())?;
    }
    let (encoding, channels, sample_rate, block_align, bits) = format.ok_or(())?;
    let data = data.ok_or(())?;
    if !(channels == 1 || channels == 2) || sample_rate != SAMPLE_RATE || data.is_empty() {
        return Err(());
    }
    let bytes_per_sample = usize::from(bits / 8);
    if bits % 8 != 0
        || bytes_per_sample == 0
        || usize::from(block_align) != usize::from(channels) * bytes_per_sample
        || data.len() % usize::from(block_align) != 0
    {
        return Err(());
    }
    let supported = matches!((encoding, bits), (1, 16 | 24 | 32) | (3, 32));
    if !supported {
        return Err(());
    }
    let frames = data.len() / usize::from(block_align);
    if frames == 0 || frames > u32::MAX as usize {
        return Err(());
    }
    let sample_count = frames.checked_mul(usize::from(channels)).ok_or(())?;
    if sample_count > RESIDENT_SOUND_BYTE_CAPACITY / size_of::<f32>() {
        return Err(());
    }
    let mut samples = Vec::with_capacity(sample_count);
    for chunk in data.chunks_exact(bytes_per_sample) {
        let sample = match (encoding, bits) {
            (1, 16) => f32::from(i16::from_le_bytes([chunk[0], chunk[1]])) / 32_768.0,
            (1, 24) => {
                let raw = i32::from_le_bytes([
                    chunk[0],
                    chunk[1],
                    chunk[2],
                    if chunk[2] & 0x80 == 0 { 0 } else { 0xff },
                ]);
                raw as f32 / 8_388_608.0
            }
            (1, 32) => {
                i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f32
                    / 2_147_483_648.0
            }
            (3, 32) => {
                let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if !value.is_finite() {
                    return Err(());
                }
                value.clamp(-1.0, 1.0)
            }
            _ => unreachable!(),
        };
        samples.push(sample);
    }
    Ok(DecodedWav {
        samples,
        frames: frames as u32,
        channels: u32::from(channels),
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ()> {
    let value = bytes.get(offset..offset + 2).ok_or(())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ()> {
    let value = bytes.get(offset..offset + 4).ok_or(())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn encode_handle(index: usize, generation: u32) -> u32 {
    (generation << INDEX_BITS) | (index as u32 + 1)
}

fn next_generation(generation: u32) -> u32 {
    let next = (generation + 1) & MAX_GENERATION;
    if next == 0 { 1 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_with_data(
        encoding: u16,
        bits: u16,
        channels: u16,
        sample_rate: u32,
        data: &[u8],
    ) -> Vec<u8> {
        let bytes_per_sample = bits / 8;
        let mut wav = Vec::with_capacity(44 + data.len());
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt \x10\0\0\0");
        wav.extend_from_slice(&encoding.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        let byte_rate = sample_rate * u32::from(channels) * u32::from(bytes_per_sample);
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&(channels * bytes_per_sample).to_le_bytes());
        wav.extend_from_slice(&bits.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(data);
        wav
    }

    fn pcm16_wav(channels: u16, sample_rate: u32, samples: &[i16]) -> Vec<u8> {
        let mut data = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            data.extend_from_slice(&sample.to_le_bytes());
        }
        wav_with_data(1, 16, channels, sample_rate, &data)
    }

    #[test]
    fn decodes_mono_pcm16_and_rejects_wrong_rate() {
        let wav = pcm16_wav(1, SAMPLE_RATE, &[-32_768, 0, 16_384, 32_767]);
        let decoded = decode_wav(&wav).unwrap();
        assert_eq!(decoded.frames, 4);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.samples[0], -1.0);
        assert!((decoded.samples[2] - 0.5).abs() < 0.0001);
        assert!(decode_wav(&pcm16_wav(1, 44_100, &[0])).is_err());
    }

    #[test]
    fn decodes_every_supported_integer_and_float_encoding() {
        let pcm24 = wav_with_data(1, 24, 1, SAMPLE_RATE, &[0x00, 0x00, 0x80, 0xff, 0xff, 0x7f]);
        let decoded24 = decode_wav(&pcm24).unwrap();
        assert_eq!(decoded24.samples[0], -1.0);
        assert!((decoded24.samples[1] - 1.0).abs() < 0.0001);

        let mut pcm32_data = Vec::new();
        pcm32_data.extend_from_slice(&i32::MIN.to_le_bytes());
        pcm32_data.extend_from_slice(&i32::MAX.to_le_bytes());
        let decoded32 = decode_wav(&wav_with_data(1, 32, 1, SAMPLE_RATE, &pcm32_data)).unwrap();
        assert_eq!(decoded32.samples[0], -1.0);
        assert!((decoded32.samples[1] - 1.0).abs() < 0.0001);

        let mut float_data = Vec::new();
        float_data.extend_from_slice(&(-0.25f32).to_le_bytes());
        float_data.extend_from_slice(&(1.5f32).to_le_bytes());
        let decoded_float = decode_wav(&wav_with_data(3, 32, 1, SAMPLE_RATE, &float_data)).unwrap();
        assert_eq!(decoded_float.samples, [-0.25, 1.0]);
        let nan = wav_with_data(3, 32, 1, SAMPLE_RATE, &f32::NAN.to_le_bytes());
        assert!(decode_wav(&nan).is_err());
    }

    #[test]
    fn sound_handles_are_generational_and_capacity_is_accounted() {
        let wav = pcm16_wav(2, SAMPLE_RATE, &[100, -100, 200, -200]);
        let mut bank = SoundBank::new();
        let first = bank.load_wav(&wav, false);
        assert_ne!(first, INVALID_SOUND_HANDLE);
        assert_eq!(bank.playback_length(first), Some((2, false)));
        assert_eq!(bank.used_bytes(), 16);
        assert!(bank.unload(first));
        assert!(!bank.contains(first));
        let second = bank.load_wav(&wav, true);
        assert_ne!(first, second);
        assert_eq!(bank.playback_length(second), Some((2, true)));
    }

    #[test]
    fn malformed_and_unsupported_wav_are_rejected() {
        let mut wrong_channels = pcm16_wav(3, SAMPLE_RATE, &[0, 0, 0]);
        assert!(decode_wav(&wrong_channels).is_err());
        wrong_channels.truncate(20);
        assert!(decode_wav(&wrong_channels).is_err());
        assert!(decode_wav(b"not a wave").is_err());
    }
}
