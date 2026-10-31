//! Conversions the audio writers offer without an encoder: any file [`super::decode`] reads (WAV of any sample
//! format [`super::decode::decode_wav`] knows, MP3) to a canonical 16-bit PCM WAV, and stereo to mono.
//!
//! There is no resampling and no MP3 encoding: a file at another rate, or one that must become an MP3, is
//! converted with an external tool (`ffmpeg -ar 22050`, `lame --cbr -b 128`).

use super::decode::{decode, DecodedAudio};
use super::{probe_audio, AudioContent};

crate::eyre_imports!();

/// The channels [`downmix_to_mono`] leaves.
pub const MONO: u16 = 1;

/// Decodes `payload` (any file [`decode`] reads) to 16-bit PCM.
pub fn decode_file(payload: &[u8]) -> eyre::Result<DecodedAudio> {
    let content = probe_audio(payload);
    match &content {
        AudioContent::Mpeg(_) | AudioContent::Wav(_) => decode(payload, &content),
        other => eyre::bail!("{} is not converted (only WAV and MP3 are)", other.format().label()),
    }
}

/// `payload` as a canonical 16-bit PCM WAV of its rate and channels (and mono with `to_mono`).
pub fn to_pcm16_wav(payload: &[u8], to_mono: bool) -> eyre::Result<Vec<u8>> {
    let decoded = decode_file(payload)?;
    let decoded = if to_mono { downmix_to_mono(&decoded) } else { decoded };
    Ok(decoded.to_wav())
}

/// The average of the channels of each sample frame (rounded toward zero); mono audio is returned as is.
pub fn downmix_to_mono(audio: &DecodedAudio) -> DecodedAudio {
    if audio.channels <= MONO {
        return audio.clone();
    }
    let channels = usize::from(audio.channels);
    let samples = audio
        .samples
        .chunks_exact(channels)
        .map(|frame| (frame.iter().map(|&sample| i32::from(sample)).sum::<i32>() / channels as i32) as i16)
        .collect();
    DecodedAudio { sample_rate: audio.sample_rate, channels: MONO, samples, skipped_packets: audio.skipped_packets }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav;

    #[test]
    fn converts_to_canonical_mono_wav() {
        let stereo: Vec<u8> = [100i16, 300, -5, -6].iter().flat_map(|sample| sample.to_le_bytes()).collect();
        let wav_stereo = wav::encode_pcm_wav(44_100, 2, &stereo);
        assert_eq!(to_pcm16_wav(&wav_stereo, false).unwrap(), wav_stereo);
        let mono = to_pcm16_wav(&wav_stereo, true).unwrap();
        let expected: Vec<u8> = [200i16, -5].iter().flat_map(|sample| sample.to_le_bytes()).collect();
        assert_eq!(mono, wav::encode_pcm_wav(44_100, 1, &expected));

        let chunked = wav::tests::sample_wav(&[1, 0, 2, 0]);
        assert_eq!(to_pcm16_wav(&chunked, false).unwrap(), wav::encode_pcm_wav(22_050, 1, &[1, 0, 2, 0]));
        assert!(to_pcm16_wav(b"OggS\0\x02", false).is_err());
    }
}
