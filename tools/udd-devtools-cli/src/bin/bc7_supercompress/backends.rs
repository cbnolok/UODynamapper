//! The entropy coders a page is stored with: Zstd and LZ4 HC as the container writes them (one Zstd frame, one LZ4
//! block), and the variants measured here (independent chunks, a shared dictionary, long-distance matching).

use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use zstd::zstd_safe;

/// LZ4 offsets are 16-bit: no match reaches further back, so a dictionary longer than this is never used.
pub const LZ4_MAX_DICTIONARY_BYTES: usize = 64 * 1024;
/// The window of `zstd:<level>:long` (128 MiB), the size Zstd's own `--long` uses.
const LONG_WINDOW_LOG: u32 = 27;
/// Zstd's own parameters for an input over 256 KiB (`clevels.h`, the first table) at the levels the whole-page
/// dictionary runs use: window, chain, hash and search logs, minimum match, target length, strategy. With a dictionary
/// loaded, Zstd sizes its tables for the dictionary instead, and a 4 MiB page came out 3 to 18 % larger than without
/// one (the zstd 1.5.7 tool does the same with `-D`); these keep the frame's parameters equal to the plain frame's.
const LARGE_INPUT_PARAMETERS: [(i32, [u32; 6], zstd_safe::Strategy); 2] = [
    (9, [22, 21, 22, 6, 5, 16], zstd_safe::Strategy::ZSTD_lazy2),
    (19, [23, 24, 22, 7, 3, 256], zstd_safe::Strategy::ZSTD_btultra2),
];
/// Bytes of the chunk-size table of an LZ4 chunked payload, per chunk: LZ4 blocks don't end themselves.
const LZ4_CHUNK_SIZE_BYTES: usize = 4;
const PERCENT: u64 = 100;
const KIB: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// One Zstd frame, as the container's codec 1. `long` turns on long-distance matching with a 128 MiB window.
    Zstd { level: i32, long: bool },
    /// Independent Zstd frames of `chunk` raw bytes, as the container's strips.
    ZstdChunks { level: i32, chunk: usize },
    /// One Zstd frame with the run's trained dictionary.
    ZstdDict { level: i32 },
    /// Independent Zstd frames of `chunk` raw bytes, each with the dictionary.
    ZstdDictChunks { level: i32, chunk: usize },
    /// One LZ4 HC block, as the container's codec 3 (at level 12).
    Lz4Hc { level: i32 },
    /// Independent LZ4 HC blocks of `chunk` raw bytes, with a table of their sizes.
    Lz4HcChunks { level: i32, chunk: usize },
    /// LZ4 HC blocks of `chunk` raw bytes (the whole payload for `None`), each after the dictionary's last 64 KiB.
    Lz4HcDict { level: i32, chunk: Option<usize> },
    /// The builder's `ZstdOrLz4HcLevel` on a page under its strips size: LZ4 HC at `lz4_level` when it is at most
    /// `max_growth_percent` larger than one Zstd frame at `zstd_level`, else that frame.
    Rule { zstd_level: i32, lz4_level: i32, max_growth_percent: u32 },
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Zstd { level, long: false } => write!(f, "zstd:{level}"),
            Self::Zstd { level, long: true } => write!(f, "zstd:{level}:long"),
            Self::ZstdChunks { level, chunk } => write!(f, "zstd-chunks:{level}:{}", chunk / KIB),
            Self::ZstdDict { level } => write!(f, "zstd-dict:{level}"),
            Self::ZstdDictChunks { level, chunk } => write!(f, "zstd-dict-chunks:{level}:{}", chunk / KIB),
            Self::Lz4Hc { level } => write!(f, "lz4hc:{level}"),
            Self::Lz4HcChunks { level, chunk } => write!(f, "lz4hc-chunks:{level}:{}", chunk / KIB),
            Self::Lz4HcDict { level, chunk: None } => write!(f, "lz4hc-dict:{level}"),
            Self::Lz4HcDict { level, chunk: Some(chunk) } => write!(f, "lz4hc-dict-chunks:{level}:{}", chunk / KIB),
            Self::Rule { zstd_level, lz4_level, max_growth_percent } =>
                write!(f, "rule:{zstd_level}:{lz4_level}:{max_growth_percent}"),
        }
    }
}

impl Backend {
    /// Parses the names [`fmt::Display`] writes; chunk sizes are in KiB.
    pub fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.split(':').collect();
        let number = |index: usize| parts.get(index).and_then(|part| part.parse::<i64>().ok());
        let level = || number(1).map(|level| level as i32);
        let chunk = || number(2).filter(|&kib| kib > 0).map(|kib| kib as usize * KIB);
        Some(match (parts[0], parts.len()) {
            ("zstd", 2) => Self::Zstd { level: level()?, long: false },
            ("zstd", 3) if parts[2] == "long" => Self::Zstd { level: level()?, long: true },
            ("zstd-chunks", 3) => Self::ZstdChunks { level: level()?, chunk: chunk()? },
            ("zstd-dict", 2) => Self::ZstdDict { level: level()? },
            ("zstd-dict-chunks", 3) => Self::ZstdDictChunks { level: level()?, chunk: chunk()? },
            ("lz4hc", 2) => Self::Lz4Hc { level: level()? },
            ("lz4hc-chunks", 3) => Self::Lz4HcChunks { level: level()?, chunk: chunk()? },
            ("lz4hc-dict", 2) => Self::Lz4HcDict { level: level()?, chunk: None },
            ("lz4hc-dict-chunks", 3) => Self::Lz4HcDict { level: level()?, chunk: Some(chunk()?) },
            ("rule", 4) => Self::Rule {
                zstd_level: level()?,
                lz4_level: number(2)? as i32,
                max_growth_percent: u32::try_from(number(3)?).ok()?,
            },
            _ => return None,
        })
    }

    pub fn needs_dictionary(self) -> bool {
        matches!(self, Self::ZstdDict { .. } | Self::ZstdDictChunks { .. } | Self::Lz4HcDict { .. })
    }
}

/// A trained dictionary, prepared once for each coder.
pub struct Dictionary {
    pub bytes: Vec<u8>,
    decoder: zstd::dict::DecoderDictionary<'static>,
}

impl Dictionary {
    pub fn new(bytes: Vec<u8>) -> Self {
        let decoder = zstd::dict::DecoderDictionary::copy(&bytes);
        Self { bytes, decoder }
    }

    /// The bytes LZ4 can reach: the end of the dictionary, where Zstd's trainer puts its most frequent content.
    fn lz4_window(&self) -> &[u8] {
        &self.bytes[self.bytes.len().saturating_sub(LZ4_MAX_DICTIONARY_BYTES)..]
    }
}

/// A compressed payload and how to decode it.
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub codec: StoredCodec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCodec {
    /// Zstd frames, one after another.
    Zstd,
    ZstdDict,
    /// One LZ4 block per chunk; `chunk_sizes` is what the size table holds (counted in the payload size).
    Lz4 { chunk_sizes: Vec<u32>, chunk: usize, dictionary: bool },
}

pub type CoderResult<T> = Result<T, String>;

fn zstd_error(code: usize) -> String {
    zstd_safe::get_error_name(code).to_string()
}

fn zstd_frame(data: &[u8], level: i32, long: bool, dictionary: Option<&Dictionary>, large_input_parameters: bool)
    -> CoderResult<Vec<u8>> {
    let mut compressor = match dictionary {
        Some(dictionary) => zstd::bulk::Compressor::with_dictionary(level, &dictionary.bytes),
        None => zstd::bulk::Compressor::new(level),
    }
    .map_err(|error| error.to_string())?;
    if large_input_parameters {
        use zstd_safe::CParameter::{ChainLog, HashLog, MinMatch, SearchLog, Strategy, TargetLength, WindowLog};
        let (_, [window, chain, hash, search, min_match, target], strategy) = LARGE_INPUT_PARAMETERS.iter()
            .find(|(row, _, _)| *row == level).ok_or_else(|| format!("no large-input parameters for level {level}"))?;
        for parameter in [WindowLog(*window), ChainLog(*chain), HashLog(*hash), SearchLog(*search), MinMatch(*min_match),
            TargetLength(*target), Strategy(*strategy)] {
            compressor.set_parameter(parameter).map_err(|error| error.to_string())?;
        }
    }
    if long {
        compressor.set_parameter(zstd::zstd_safe::CParameter::EnableLongDistanceMatching(true))
            .and_then(|()| compressor.set_parameter(zstd::zstd_safe::CParameter::WindowLog(LONG_WINDOW_LOG)))
            .map_err(|error| error.to_string())?;
    }
    compressor.compress(data).map_err(|error| error.to_string())
}

fn lz4_block(data: &[u8], level: i32) -> CoderResult<Vec<u8>> {
    lz4::block::compress(data, Some(lz4::block::CompressionMode::HIGHCOMPRESSION(level)), false)
        .map_err(|error| error.to_string())
}

fn lz4_chunks(data: &[u8], level: i32, chunk: usize, dictionary: Option<&Dictionary>) -> CoderResult<Encoded> {
    let mut bytes = Vec::new();
    let mut chunk_sizes = Vec::new();
    for part in data.chunks(chunk.max(1)) {
        let block = match dictionary {
            Some(dictionary) => lz4_dict::compress(part, level, dictionary.lz4_window())?,
            None => lz4_block(part, level)?,
        };
        chunk_sizes.push(u32::try_from(block.len()).map_err(|_| "LZ4 chunk too large")?);
        bytes.extend_from_slice(&block);
    }
    // A single block needs no size table: the container's stored size ends it.
    if chunk_sizes.len() > 1 {
        bytes.extend(std::iter::repeat_n(0u8, chunk_sizes.len() * LZ4_CHUNK_SIZE_BYTES));
    }
    Ok(Encoded { bytes, codec: StoredCodec::Lz4 { chunk_sizes, chunk, dictionary: dictionary.is_some() } })
}

pub fn encode(backend: Backend, data: &[u8], dictionary: Option<&Dictionary>) -> CoderResult<Encoded> {
    let need = || dictionary.ok_or_else(|| format!("{backend} needs a dictionary"));
    Ok(match backend {
        Backend::Zstd { level, long } => Encoded { bytes: zstd_frame(data, level, long, None, false)?, codec: StoredCodec::Zstd },
        Backend::ZstdChunks { level, chunk } => {
            let mut bytes = Vec::new();
            for part in data.chunks(chunk) {
                bytes.extend_from_slice(&zstd_frame(part, level, false, None, false)?);
            }
            Encoded { bytes, codec: StoredCodec::Zstd }
        }
        Backend::ZstdDict { level } => {
            Encoded { bytes: zstd_frame(data, level, false, Some(need()?), true)?, codec: StoredCodec::ZstdDict }
        }
        Backend::ZstdDictChunks { level, chunk } => {
            let dictionary = need()?;
            let mut bytes = Vec::new();
            for part in data.chunks(chunk) {
                bytes.extend_from_slice(&zstd_frame(part, level, false, Some(dictionary), false)?);
            }
            Encoded { bytes, codec: StoredCodec::ZstdDict }
        }
        Backend::Lz4Hc { level } => lz4_chunks(data, level, data.len(), None)?,
        Backend::Lz4HcChunks { level, chunk } => lz4_chunks(data, level, chunk, None)?,
        Backend::Lz4HcDict { level, chunk } => lz4_chunks(data, level, chunk.unwrap_or(data.len()), Some(need()?))?,
        Backend::Rule { zstd_level, lz4_level, max_growth_percent } => {
            let (zstd, lz4) = rayon::join(|| zstd_frame(data, zstd_level, false, None, false), || lz4_block(data, lz4_level));
            let (zstd, lz4) = (zstd?, lz4?);
            if lz4.len() as u64 * PERCENT <= zstd.len() as u64 * (PERCENT + u64::from(max_growth_percent)) {
                Encoded { bytes: lz4, codec: StoredCodec::Lz4 { chunk_sizes: Vec::new(), chunk: data.len(), dictionary: false } }
            } else {
                Encoded { bytes: zstd, codec: StoredCodec::Zstd }
            }
        }
    })
}

thread_local! {
    /// One context per thread, reused for every page, as the container's reader keeps it.
    static DCTX: RefCell<zstd_safe::DCtx<'static>> = RefCell::new(zstd_safe::DCtx::create());
}

/// Decodes `encoded` to its `raw_len` bytes into a new buffer, as the container's reader does.
pub fn decode(encoded: &Encoded, raw_len: usize, dictionary: Option<&Dictionary>) -> CoderResult<Vec<u8>> {
    match &encoded.codec {
        StoredCodec::Zstd | StoredCodec::ZstdDict => DCTX.with(|context| {
            let mut context = context.borrow_mut();
            let mut out = Vec::with_capacity(raw_len);
            let mut rest = encoded.bytes.as_slice();
            // Chunked payloads are frames one after another; the reader walks them in order.
            while !rest.is_empty() {
                let frame_len = zstd_safe::find_frame_compressed_size(rest).map_err(zstd_error)?;
                let (frame, tail) = rest.split_at(frame_len);
                let mut part = Vec::with_capacity(raw_len - out.len());
                match (&encoded.codec, dictionary) {
                    (StoredCodec::ZstdDict, Some(dictionary)) => {
                        context.decompress_using_ddict(&mut part, frame, dictionary.decoder.as_ddict())
                    }
                    (StoredCodec::ZstdDict, None) => return Err("Zstd dictionary payload without a dictionary".into()),
                    _ => context.decompress(&mut part, frame),
                }
                .map_err(zstd_error)?;
                if out.is_empty() && tail.is_empty() {
                    out = part;
                } else {
                    out.extend_from_slice(&part);
                }
                rest = tail;
            }
            Ok(out)
        }),
        StoredCodec::Lz4 { chunk_sizes, chunk, dictionary: with_dictionary } => {
            let mut out = vec![0u8; raw_len];
            let sizes: Vec<usize> = if chunk_sizes.is_empty() {
                vec![encoded.bytes.len()]
            } else {
                chunk_sizes.iter().map(|&size| size as usize).collect()
            };
            let mut at = 0;
            for (index, (size, part)) in sizes.iter().zip(out.chunks_mut((*chunk).max(1))).enumerate() {
                let block = encoded.bytes.get(at..at + size).ok_or("LZ4 chunk past the payload")?;
                at += size;
                let written = if *with_dictionary {
                    let dictionary = dictionary.ok_or("LZ4 dictionary payload without a dictionary")?;
                    lz4_dict::decompress(block, part, dictionary.lz4_window())?
                } else {
                    lz4::block::decompress_to_buffer(block, Some(part.len() as i32), part)
                        .map_err(|error| error.to_string())?
                };
                if written != part.len() {
                    return Err(format!("LZ4 chunk {index} decoded to {written} bytes, not {}", part.len()));
                }
            }
            Ok(out)
        }
    }
}

/// LZ4 HC with an external dictionary, through the C library the `lz4` crate already links (lz4-sys builds
/// lz4hc.c), as `lz4hc.h` and `lz4.h` declare these functions. Isolated here: nothing outside the experiment uses it.
mod lz4_dict {
    use std::ffi::{c_char, c_int, c_void};

    unsafe extern "C" {
        fn LZ4_createStreamHC() -> *mut c_void;
        fn LZ4_freeStreamHC(stream: *mut c_void) -> c_int;
        fn LZ4_resetStreamHC_fast(stream: *mut c_void, level: c_int);
        fn LZ4_loadDictHC(stream: *mut c_void, dictionary: *const c_char, size: c_int) -> c_int;
        fn LZ4_compress_HC_continue(
            stream: *mut c_void,
            source: *const c_char,
            destination: *mut c_char,
            source_size: c_int,
            capacity: c_int,
        ) -> c_int;
        fn LZ4_compressBound(size: c_int) -> c_int;
        fn LZ4_decompress_safe_usingDict(
            source: *const c_char,
            destination: *mut c_char,
            compressed_size: c_int,
            capacity: c_int,
            dictionary: *const c_char,
            dictionary_size: c_int,
        ) -> c_int;
    }

    fn int(value: usize) -> Result<c_int, String> {
        c_int::try_from(value).map_err(|_| format!("{value} bytes is too large for LZ4"))
    }

    /// One LZ4 HC block of `data` after `dictionary`, on a fresh stream: no history from an earlier call.
    pub fn compress(data: &[u8], level: i32, dictionary: &[u8]) -> Result<Vec<u8>, String> {
        let (data_len, dictionary_len) = (int(data.len())?, int(dictionary.len())?);
        // SAFETY: the stream is created, used and freed here; every pointer comes with its slice's length, and the
        // dictionary outlives the compression call, as LZ4_loadDictHC requires.
        unsafe {
            let stream = LZ4_createStreamHC();
            if stream.is_null() {
                return Err("LZ4_createStreamHC failed".into());
            }
            LZ4_resetStreamHC_fast(stream, level);
            LZ4_loadDictHC(stream, dictionary.as_ptr().cast(), dictionary_len);
            let capacity = LZ4_compressBound(data_len);
            let mut out = vec![0u8; capacity as usize];
            let written =
                LZ4_compress_HC_continue(stream, data.as_ptr().cast(), out.as_mut_ptr().cast(), data_len, capacity);
            LZ4_freeStreamHC(stream);
            if written <= 0 {
                return Err("LZ4_compress_HC_continue failed".into());
            }
            out.truncate(written as usize);
            Ok(out)
        }
    }

    /// Decodes one block written by [`compress`] into `out`, returning the bytes written.
    pub fn decompress(block: &[u8], out: &mut [u8], dictionary: &[u8]) -> Result<usize, String> {
        // SAFETY: the safe decoder writes at most `out.len()` bytes and reads at most `block.len()`.
        let written = unsafe {
            LZ4_decompress_safe_usingDict(
                block.as_ptr().cast(),
                out.as_mut_ptr().cast(),
                int(block.len())?,
                int(out.len())?,
                dictionary.as_ptr().cast(),
                int(dictionary.len())?,
            )
        };
        if written < 0 { Err("malformed LZ4 block".into()) } else { Ok(written as usize) }
    }
}

/// Trains a Zstd dictionary of `bytes` from `chunk`-byte samples of `pages`.
pub fn train_dictionary(pages: &[Arc<Vec<u8>>], chunk: usize, sample_every: usize, bytes: usize)
    -> CoderResult<Dictionary> {
    let mut samples = Vec::new();
    let mut sizes = Vec::new();
    for (index, part) in pages.iter().flat_map(|page| page.chunks(chunk)).enumerate() {
        if index % sample_every.max(1) == 0 {
            samples.extend_from_slice(part);
            sizes.push(part.len());
        }
    }
    let trained = zstd::dict::from_continuous(&samples, &sizes, bytes).map_err(|error| error.to_string())?;
    Ok(Dictionary::new(trained))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_page() -> Vec<u8> {
        let mut page = Vec::new();
        for block in 0u32..20_000 {
            let value = if block % 3 == 0 { 0 } else { block.wrapping_mul(2_654_435_761) };
            page.extend_from_slice(&value.to_le_bytes().repeat(4));
        }
        page
    }

    #[test]
    fn every_backend_round_trips() {
        let page = sample_page();
        let pages = vec![Arc::new(page.clone()); 4];
        let dictionary = train_dictionary(&pages, 4 * KIB, 1, 8 * KIB).unwrap();
        for name in ["zstd:3", "zstd:3:long", "zstd-chunks:3:16", "zstd-dict:9", "zstd-dict:19", "zstd-dict-chunks:3:16", "lz4hc:9",
            "lz4hc-chunks:9:16", "lz4hc-dict:9", "lz4hc-dict-chunks:9:16", "rule:3:9:20", "rule:3:9:0"] {
            let backend = Backend::parse(name).unwrap();
            assert_eq!(backend.to_string(), name);
            let encoded = encode(backend, &page, Some(&dictionary)).unwrap();
            assert_eq!(decode(&encoded, page.len(), Some(&dictionary)).unwrap(), page, "{name}");
        }
    }

    #[test]
    fn a_damaged_payload_fails_to_decode() {
        let page = sample_page();
        for name in ["zstd:3", "lz4hc:9", "lz4hc-chunks:9:16"] {
            let mut encoded = encode(Backend::parse(name).unwrap(), &page, None).unwrap();
            encoded.bytes.truncate(encoded.bytes.len() / 2);
            assert!(decode(&encoded, page.len(), None).is_err(), "{name}");
        }
    }

    #[test]
    fn a_dictionary_payload_needs_its_dictionary() {
        let page = sample_page();
        let dictionary = train_dictionary(&[Arc::new(page.clone())], 4 * KIB, 1, 8 * KIB).unwrap();
        for name in ["zstd-dict:9", "lz4hc-dict:9"] {
            let encoded = encode(Backend::parse(name).unwrap(), &page, Some(&dictionary)).unwrap();
            assert!(decode(&encoded, page.len(), None).is_err(), "{name}");
        }
    }
}
