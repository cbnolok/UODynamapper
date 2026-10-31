//! RIFF WAVE files: the chunk list, the `fmt ` fields and the `data` extent ([`analyze_wav`]), and a writer for
//! 16-bit PCM WAV files ([`encode_pcm_wav`]).
//!
//! RIFF pads every chunk of odd size with one byte. Some writers leave the pad out; the walk takes the padded
//! position unless only the unpadded one holds a chunk id, and counts those chunks.

pub const RIFF_MAGIC: &[u8] = b"RIFF";
pub const WAVE_FORM: &[u8] = b"WAVE";
pub const RIFF_HEADER_SIZE: usize = 12;
const RIFF_SIZE_OFFSET: usize = 4;
const RIFF_FORM_OFFSET: usize = 8;
pub const CHUNK_HEADER_SIZE: usize = 8;
const CHUNK_ID_SIZE: usize = 4;
pub const FMT_CHUNK_ID: [u8; 4] = *b"fmt ";
pub const DATA_CHUNK_ID: [u8; 4] = *b"data";
/// `WAVEFORMAT` with `wBitsPerSample` (PCMWAVEFORMAT), `WAVEFORMATEX` with `cbSize`, `WAVEFORMATEXTENSIBLE`.
pub const FMT_PCM_SIZE: u32 = 16;
pub const FMT_EX_SIZE: u32 = 18;
const FMT_CHANNELS_OFFSET: usize = 2;
const FMT_SAMPLE_RATE_OFFSET: usize = 4;
const FMT_BYTE_RATE_OFFSET: usize = 8;
const FMT_BLOCK_ALIGN_OFFSET: usize = 12;
const FMT_BITS_OFFSET: usize = 14;
const FMT_CB_SIZE_OFFSET: usize = 16;
const FMT_SUBFORMAT_OFFSET: usize = 24;
pub const WAVE_FORMAT_PCM: u16 = 0x0001;
pub const WAVE_FORMAT_ADPCM: u16 = 0x0002;
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
pub const WAVE_FORMAT_IMA_ADPCM: u16 = 0x0011;
pub const WAVE_FORMAT_MPEGLAYER3: u16 = 0x0055;
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;
const BITS_PER_BYTE: u16 = 8;
/// The header [`encode_pcm_wav`] writes: RIFF header, a 16-byte `fmt ` chunk and the `data` chunk header.
pub const PCM_WAV_HEADER_SIZE: usize = RIFF_HEADER_SIZE + CHUNK_HEADER_SIZE + FMT_PCM_SIZE as usize + CHUNK_HEADER_SIZE;
/// The sample size [`encode_pcm_wav`] writes.
pub const PCM_WAV_BITS_PER_SAMPLE: u16 = 16;

/// The name of a `wFormatTag`.
pub fn format_tag_label(tag: u16) -> String {
    match tag {
        WAVE_FORMAT_PCM => "PCM".to_owned(),
        WAVE_FORMAT_ADPCM => "MS ADPCM".to_owned(),
        WAVE_FORMAT_IEEE_FLOAT => "IEEE float".to_owned(),
        WAVE_FORMAT_IMA_ADPCM => "IMA ADPCM".to_owned(),
        WAVE_FORMAT_MPEGLAYER3 => "MPEG Layer III".to_owned(),
        WAVE_FORMAT_EXTENSIBLE => "extensible".to_owned(),
        other => format!("format {other:#06x}"),
    }
}

/// The `fmt ` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavFormat {
    pub format_tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub byte_rate: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
    /// The chunk size: 16, 18, 40 or another.
    pub chunk_size: u32,
    /// `cbSize` of a chunk of 18 bytes or more.
    pub cb_size: Option<u16>,
    /// The first two bytes of the sub-format GUID of `WAVE_FORMAT_EXTENSIBLE` (the format tag it stands for).
    pub sub_format: Option<u16>,
}

impl WavFormat {
    /// The format tag, or for `WAVE_FORMAT_EXTENSIBLE` the one its sub-format names.
    pub fn effective_tag(&self) -> u16 {
        match (self.format_tag, self.sub_format) {
            (WAVE_FORMAT_EXTENSIBLE, Some(sub)) => sub,
            (tag, _) => tag,
        }
    }

    /// Integer PCM of 8 or 16 bits, which [`crate::audio::decode`] reads.
    pub fn is_pcm(&self) -> bool {
        self.effective_tag() == WAVE_FORMAT_PCM
    }

    /// `byte_rate` and `block_align` follow from the channels, rate and bits.
    pub fn is_consistent(&self) -> bool {
        let block = u32::from(self.channels) * u32::from(self.bits_per_sample).div_ceil(u32::from(BITS_PER_BYTE));
        !self.is_pcm() || (u32::from(self.block_align) == block && self.byte_rate == self.sample_rate * block)
    }

    /// `PCM 22050 Hz mono 16 bit`.
    pub fn label(&self) -> String {
        let channels = match self.channels {
            1 => "mono".to_owned(),
            2 => "stereo".to_owned(),
            other => format!("{other} channels"),
        };
        format!("{} {} Hz {} {} bit", format_tag_label(self.effective_tag()), self.sample_rate, channels, self.bits_per_sample)
    }
}

/// One chunk of the RIFF form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavChunk {
    pub id: [u8; 4],
    /// Where its header starts.
    pub offset: usize,
    /// Its size field.
    pub size: u32,
    /// An odd-sized chunk followed by the next one without the pad byte.
    pub unpadded: bool,
}

impl WavChunk {
    pub fn id_label(&self) -> String {
        String::from_utf8_lossy(&self.id).into_owned()
    }
}

/// A RIFF WAVE payload.
#[derive(Debug, Clone, PartialEq)]
pub struct WavAnalysis {
    /// The RIFF size field (the payload length less 8 when consistent).
    pub riff_size: u32,
    pub chunks: Vec<WavChunk>,
    pub format: Option<WavFormat>,
    /// Where the `data` bytes start and how many of them the payload holds.
    pub data_offset: Option<usize>,
    pub data_len: usize,
    /// The `data` size field runs past the end of the payload.
    pub data_truncated: bool,
    /// Bytes after the last chunk that are not one (the walk stopped there).
    pub trailing_bytes: usize,
    pub trailing_zero: bool,
    /// The payload length.
    pub length: usize,
}

impl WavAnalysis {
    /// The RIFF size field is the payload length less the RIFF header's first 8 bytes.
    pub fn riff_size_matches(&self) -> bool {
        self.riff_size as usize + CHUNK_HEADER_SIZE == self.length
    }

    pub fn unpadded_chunks(&self) -> usize {
        self.chunks.iter().filter(|chunk| chunk.unpadded).count()
    }

    /// The chunk ids in order other than `fmt ` and `data`.
    pub fn extra_chunks(&self) -> Vec<String> {
        self.chunks
            .iter()
            .filter(|chunk| chunk.id != FMT_CHUNK_ID && chunk.id != DATA_CHUNK_ID)
            .map(WavChunk::id_label)
            .collect()
    }

    /// Sample frames (one sample per channel) in the data.
    pub fn sample_frames(&self) -> Option<u64> {
        let format = self.format?;
        (format.block_align > 0).then(|| self.data_len as u64 / u64::from(format.block_align))
    }

    pub fn duration_seconds(&self) -> Option<f64> {
        let format = self.format?;
        (format.sample_rate > 0).then(|| self.sample_frames().unwrap_or(0) as f64 / f64::from(format.sample_rate))
    }

    /// The data bytes.
    pub fn data<'a>(&self, payload: &'a [u8]) -> Option<&'a [u8]> {
        self.data_offset.map(|offset| &payload[offset..offset + self.data_len])
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes.get(offset..offset + size_of::<u16>()).map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes.get(offset..offset + size_of::<u32>()).map(|quad| u32::from_le_bytes(quad.try_into().expect("four bytes")))
}

/// Printable ASCII, as every registered chunk id is.
fn is_chunk_id(bytes: &[u8], offset: usize) -> bool {
    bytes.get(offset..offset + CHUNK_ID_SIZE).is_some_and(|id| id.iter().all(|byte| (b' '..=b'~').contains(byte)))
}

fn parse_format(chunk: &[u8]) -> Option<WavFormat> {
    let chunk_size = chunk.len() as u32;
    if chunk_size < FMT_PCM_SIZE {
        return None;
    }
    let cb_size = (chunk_size >= FMT_EX_SIZE).then(|| read_u16(chunk, FMT_CB_SIZE_OFFSET)).flatten();
    let format_tag = read_u16(chunk, 0)?;
    Some(WavFormat {
        format_tag,
        channels: read_u16(chunk, FMT_CHANNELS_OFFSET)?,
        sample_rate: read_u32(chunk, FMT_SAMPLE_RATE_OFFSET)?,
        byte_rate: read_u32(chunk, FMT_BYTE_RATE_OFFSET)?,
        block_align: read_u16(chunk, FMT_BLOCK_ALIGN_OFFSET)?,
        bits_per_sample: read_u16(chunk, FMT_BITS_OFFSET)?,
        chunk_size,
        cb_size,
        sub_format: (format_tag == WAVE_FORMAT_EXTENSIBLE).then(|| read_u16(chunk, FMT_SUBFORMAT_OFFSET)).flatten(),
    })
}

/// The chunks of a `RIFF....WAVE` payload, walked to the end of the payload (not to the RIFF size).
pub fn analyze_wav(payload: &[u8]) -> Option<WavAnalysis> {
    if !payload.starts_with(RIFF_MAGIC) || payload.get(RIFF_FORM_OFFSET..RIFF_HEADER_SIZE) != Some(WAVE_FORM) {
        return None;
    }
    let mut analysis = WavAnalysis {
        riff_size: read_u32(payload, RIFF_SIZE_OFFSET)?,
        chunks: Vec::new(),
        format: None,
        data_offset: None,
        data_len: 0,
        data_truncated: false,
        trailing_bytes: 0,
        trailing_zero: true,
        length: payload.len(),
    };
    let mut offset = RIFF_HEADER_SIZE;
    while offset + CHUNK_HEADER_SIZE <= payload.len() && is_chunk_id(payload, offset) {
        let id: [u8; 4] = payload[offset..offset + CHUNK_ID_SIZE].try_into().expect("four bytes");
        let size = read_u32(payload, offset + CHUNK_ID_SIZE)?;
        let body = offset + CHUNK_HEADER_SIZE;
        let end = body.saturating_add(size as usize);
        let available = end.min(payload.len());
        match id {
            FMT_CHUNK_ID if analysis.format.is_none() => analysis.format = parse_format(&payload[body..available]),
            DATA_CHUNK_ID if analysis.data_offset.is_none() => {
                analysis.data_offset = Some(body);
                analysis.data_len = available - body;
                analysis.data_truncated = end > payload.len();
            }
            _ => {}
        }
        let padded_end = end.saturating_add(size as usize & 1);
        let unpadded = size & 1 == 1
            && padded_end < payload.len()
            && !is_chunk_id(payload, padded_end)
            && (end == payload.len() || is_chunk_id(payload, end));
        analysis.chunks.push(WavChunk { id, offset, size, unpadded });
        offset = if unpadded { end } else { padded_end };
    }
    if offset < payload.len() {
        analysis.trailing_bytes = payload.len() - offset;
        analysis.trailing_zero = payload[offset..].iter().all(|&byte| byte == 0);
    }
    Some(analysis)
}

/// A canonical PCM WAV file ([`PCM_WAV_HEADER_SIZE`]-byte header) of `pcm_le16`: interleaved little-endian 16-bit
/// samples.
pub fn encode_pcm_wav(sample_rate: u32, channels: u16, pcm_le16: &[u8]) -> Vec<u8> {
    let block_align = channels * PCM_WAV_BITS_PER_SAMPLE / BITS_PER_BYTE;
    let byte_rate = sample_rate * u32::from(block_align);
    let data_len = pcm_le16.len() as u32;
    let mut wav = Vec::with_capacity(PCM_WAV_HEADER_SIZE + pcm_le16.len());
    wav.extend_from_slice(RIFF_MAGIC);
    wav.extend_from_slice(&((PCM_WAV_HEADER_SIZE - CHUNK_HEADER_SIZE) as u32 + data_len).to_le_bytes());
    wav.extend_from_slice(WAVE_FORM);
    wav.extend_from_slice(&FMT_CHUNK_ID);
    wav.extend_from_slice(&FMT_PCM_SIZE.to_le_bytes());
    wav.extend_from_slice(&WAVE_FORMAT_PCM.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&PCM_WAV_BITS_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(&DATA_CHUNK_ID);
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm_le16);
    wav
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A WAV with an 18-byte `fmt ` (cbSize 0), an odd-sized `LIST` chunk without its pad byte, and `data`.
    pub(crate) fn sample_wav(pcm: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(WAVE_FORM);
        body.extend_from_slice(&FMT_CHUNK_ID);
        body.extend_from_slice(&FMT_EX_SIZE.to_le_bytes());
        for value in [WAVE_FORMAT_PCM, 1] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.extend_from_slice(&22_050u32.to_le_bytes());
        body.extend_from_slice(&44_100u32.to_le_bytes());
        for value in [2u16, 16, 0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.extend_from_slice(b"LIST");
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(b"abc");
        body.extend_from_slice(&DATA_CHUNK_ID);
        body.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        body.extend_from_slice(pcm);
        let mut wav = RIFF_MAGIC.to_vec();
        wav.extend_from_slice(&(body.len() as u32).to_le_bytes());
        wav.extend_from_slice(&body);
        wav
    }

    /// A canonical 22050 Hz mono WAV of `data` with another format tag and sample size.
    pub(crate) fn wav_with_format(format_tag: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        const SAMPLE_RATE: u32 = 22_050;
        let fmt = RIFF_HEADER_SIZE + CHUNK_HEADER_SIZE;
        let block = bits.div_ceil(BITS_PER_BYTE);
        let mut wav = encode_pcm_wav(SAMPLE_RATE, 1, data);
        wav[fmt..fmt + size_of::<u16>()].copy_from_slice(&format_tag.to_le_bytes());
        wav[fmt + FMT_BYTE_RATE_OFFSET..fmt + FMT_BLOCK_ALIGN_OFFSET].copy_from_slice(&(SAMPLE_RATE * u32::from(block)).to_le_bytes());
        wav[fmt + FMT_BLOCK_ALIGN_OFFSET..fmt + FMT_BITS_OFFSET].copy_from_slice(&block.to_le_bytes());
        wav[fmt + FMT_BITS_OFFSET..fmt + FMT_PCM_SIZE as usize].copy_from_slice(&bits.to_le_bytes());
        wav
    }

    #[test]
    fn walks_chunks_with_an_unpadded_odd_chunk() {
        let pcm = [1u8, 0, 2, 0, 3, 0];
        let wav = sample_wav(&pcm);
        let analysis = analyze_wav(&wav).unwrap();
        assert!(analysis.riff_size_matches());
        let format = analysis.format.unwrap();
        assert_eq!(format.chunk_size, FMT_EX_SIZE);
        assert_eq!(format.cb_size, Some(0));
        assert!(format.is_pcm() && format.is_consistent());
        assert_eq!(format.label(), "PCM 22050 Hz mono 16 bit");
        assert_eq!(analysis.extra_chunks(), ["LIST"]);
        assert_eq!(analysis.unpadded_chunks(), 1);
        assert_eq!(analysis.data(&wav), Some(&pcm[..]));
        assert_eq!(analysis.sample_frames(), Some(3));
        assert_eq!(analysis.trailing_bytes, 0);
        assert!(!analysis.data_truncated);
    }

    #[test]
    fn reports_truncated_data_and_trailing_bytes() {
        let mut wav = sample_wav(&[0u8; 4]);
        let data_size_at = wav.len() - 4 - size_of::<u32>();
        wav[data_size_at..data_size_at + 4].copy_from_slice(&8u32.to_le_bytes());
        let analysis = analyze_wav(&wav).unwrap();
        assert!(analysis.data_truncated);
        assert_eq!(analysis.data_len, 4);

        let mut padded = sample_wav(&[0u8; 2]);
        padded.extend_from_slice(&[0xff, 0x00, 0x01]);
        let analysis = analyze_wav(&padded).unwrap();
        assert_eq!(analysis.trailing_bytes, 3);
        assert!(!analysis.trailing_zero);
        assert!(!analysis.riff_size_matches());
    }

    #[test]
    fn writes_a_canonical_pcm_wav() {
        let pcm = [1u8, 2, 3, 4];
        let wav = encode_pcm_wav(22_050, 1, &pcm);
        let analysis = analyze_wav(&wav).unwrap();
        assert!(analysis.riff_size_matches());
        assert_eq!(analysis.format.unwrap().label(), "PCM 22050 Hz mono 16 bit");
        assert_eq!(analysis.data(&wav), Some(&pcm[..]));
        assert_eq!(wav.len(), PCM_WAV_HEADER_SIZE + pcm.len());
    }
}
