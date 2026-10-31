//! Decoding audio payloads to interleaved 16-bit PCM ([`decode`]): MP3 through Symphonia (pure Rust, so it builds
//! on every release target, the static musl CLI builds included), PCM WAV and Classic PCM directly.

use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::mp3::MpegLayer;
use super::wav::{WavAnalysis, PCM_WAV_BITS_PER_SAMPLE};
use super::AudioContent;
use crate::classic::sound::{CHANNELS as CLASSIC_CHANNELS, SAMPLE_RATE as CLASSIC_RATE};

crate::eyre_imports!();

const MP3_EXTENSION: &str = "mp3";
/// 8-bit WAV samples are unsigned around this value.
const PCM8_MIDPOINT: i16 = 128;
/// An 8-bit sample shifted into the high byte of a 16-bit one.
const PCM8_TO_16_SHIFT: u32 = 8;
const PCM8_BITS: u16 = 8;
const PCM16_BYTES: usize = 2;

/// Interleaved 16-bit samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
    /// Packets the MP3 decoder rejected and skipped.
    pub skipped_packets: u32,
}

impl DecodedAudio {
    /// Samples per channel.
    pub fn frames(&self) -> u64 {
        if self.channels == 0 { 0 } else { self.samples.len() as u64 / u64::from(self.channels) }
    }

    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 { 0.0 } else { self.frames() as f64 / f64::from(self.sample_rate) }
    }

    /// The samples as little-endian bytes.
    pub fn pcm_le_bytes(&self) -> Vec<u8> {
        self.samples.iter().flat_map(|sample| sample.to_le_bytes()).collect()
    }

    /// A 16-bit PCM WAV file of the samples.
    pub fn to_wav(&self) -> Vec<u8> {
        super::wav::encode_pcm_wav(self.sample_rate, self.channels, &self.pcm_le_bytes())
    }

    /// The samples scaled to -1.0..1.0, for playback.
    pub fn samples_f32(&self) -> Vec<f32> {
        self.samples.iter().map(|&sample| f32::from(sample) / -f32::from(i16::MIN)).collect()
    }
}

/// Decodes a payload whose content is `content` ([`super::probe_audio`], or a Classic slot's PCM bytes for
/// [`AudioContent::ClassicPcm`]).
pub fn decode(payload: &[u8], content: &AudioContent) -> eyre::Result<DecodedAudio> {
    match content {
        AudioContent::Mpeg(analysis) if analysis.first.layer == MpegLayer::L3 => decode_mp3(payload),
        AudioContent::Wav(analysis) => decode_wav(payload, analysis),
        AudioContent::ClassicPcm { .. } => Ok(decode_classic_pcm(payload)),
        other => eyre::bail!("{} is not decoded", other.format().label()),
    }
}

/// The raw PCM of a Classic sound slot (after its header): 22050 Hz mono 16-bit; an odd last byte is dropped.
pub fn decode_classic_pcm(pcm: &[u8]) -> DecodedAudio {
    DecodedAudio {
        sample_rate: CLASSIC_RATE,
        channels: CLASSIC_CHANNELS,
        samples: pcm.chunks_exact(PCM16_BYTES).map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect(),
        skipped_packets: 0,
    }
}

/// Integer PCM WAV of 8 or 16 bits.
pub fn decode_wav(payload: &[u8], analysis: &WavAnalysis) -> eyre::Result<DecodedAudio> {
    let format = analysis.format.ok_or_else(|| eyre!("WAV without a fmt chunk"))?;
    let data = analysis.data(payload).ok_or_else(|| eyre!("WAV without a data chunk"))?;
    if !format.is_pcm() || format.channels == 0 {
        eyre::bail!("unsupported WAV format: {}", format.label());
    }
    let samples = match format.bits_per_sample {
        PCM8_BITS => data.iter().map(|&byte| (i16::from(byte) - PCM8_MIDPOINT) << PCM8_TO_16_SHIFT).collect(),
        PCM_WAV_BITS_PER_SAMPLE => data.chunks_exact(PCM16_BYTES).map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect(),
        bits => eyre::bail!("unsupported WAV sample size: {bits} bit"),
    };
    Ok(DecodedAudio { sample_rate: format.sample_rate, channels: format.channels, samples, skipped_packets: 0 })
}

/// MPEG Layer III through Symphonia's probe (which skips ID3v2 tags) and decoder. Packets the decoder rejects are
/// skipped and counted, as players do; any other error fails.
pub fn decode_mp3(payload: &[u8]) -> eyre::Result<DecodedAudio> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(payload.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(MP3_EXTENSION);
    let probed = symphonia::default::get_probe()
        .format(&hint, source, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|error| eyre!("MP3 probe: {error}"))?;
    let mut reader = probed.format;
    let track = reader.default_track().ok_or_else(|| eyre!("MP3 without an audio track"))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| eyre!("MP3 decoder: {error}"))?;
    let mut decoded = DecodedAudio {
        sample_rate: track.codec_params.sample_rate.unwrap_or(0),
        channels: track.codec_params.channels.map_or(0, |channels| channels.count() as u16),
        samples: Vec::new(),
        skipped_packets: 0,
    };
    let mut buffer: Option<SampleBuffer<i16>> = None;
    loop {
        let packet = match reader.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => eyre::bail!("MP3 packet: {error}"),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = *audio.spec();
                decoded.sample_rate = spec.rate;
                decoded.channels = spec.channels.count() as u16;
                let capacity = audio.capacity() as u64;
                let samples = match &mut buffer {
                    Some(samples) if samples.capacity() as u64 >= capacity * u64::from(decoded.channels) => samples,
                    slot => slot.insert(SampleBuffer::new(capacity, spec)),
                };
                samples.copy_interleaved_ref(audio);
                decoded.samples.extend_from_slice(samples.samples());
            }
            Err(SymphoniaError::DecodeError(_)) => decoded.skipped_packets += 1,
            Err(error) => eyre::bail!("MP3 decode: {error}"),
        }
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{mp3, probe_audio, wav};

    #[test]
    fn decodes_pcm_wav_and_classic_pcm() {
        let pcm: Vec<u8> = [1i16, -2, 300].iter().flat_map(|sample| sample.to_le_bytes()).collect();
        let wav = wav::tests::sample_wav(&pcm);
        let decoded = decode(&wav, &probe_audio(&wav)).unwrap();
        assert_eq!((decoded.sample_rate, decoded.channels), (22_050, 1));
        assert_eq!(decoded.samples, [1, -2, 300]);
        assert_eq!(decoded.to_wav(), wav::encode_pcm_wav(22_050, 1, &pcm));

        let classic = decode(&pcm, &AudioContent::ClassicPcm { pcm_len: pcm.len() }).unwrap();
        assert_eq!(classic.samples, [1, -2, 300]);
        assert_eq!(classic.frames(), 3);
    }

    #[test]
    fn decodes_silent_mp3_frames() {
        let frames = 8;
        let stream = mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, frames);
        let content = probe_audio(&stream);
        let AudioContent::Mpeg(analysis) = &content else { panic!("not MPEG: {content:?}") };
        let decoded = decode(&stream, &content).unwrap();
        assert_eq!((decoded.sample_rate, decoded.channels), (22_050, 1));
        assert_eq!(decoded.frames(), analysis.samples());
        assert!(decoded.samples.iter().all(|&sample| sample == 0));
    }

    #[test]
    fn rejects_unsupported_content() {
        assert!(decode(b"OggS", &AudioContent::Ogg).is_err());
    }
}
