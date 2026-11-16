//! Making `kr_delta.uddp` from a Kingdom Reborn and an Enhanced Client, and rebuilding the Kingdom Reborn files the
//! packers read from it (the format is `udd_assets::kr_delta`).
//!
//! The Enhanced packages carry the Kingdom Reborn data, packed from a Kingdom Reborn client folder that is either the
//! real client or one rebuilt from the delta in the git checkout ([`KrClientSource`]). Both go through the same
//! rebuild ([`prepare_kr_client`]): a real client is turned into a delta in memory first. So a texture Kingdom Reborn
//! only re-encoded (its image within [`KR_IMAGE_EQUIVALENCE_THRESHOLD`] of the Enhanced one of the same name) is the
//! Enhanced Client's texture either way, and the packages are the same from either source.
//!
//! An entry the rebuild takes from the Enhanced Client is checked against the hash the delta recorded. Another
//! Enhanced Client may hold another texture there, or none: the rebuild takes that Enhanced entry (or leaves the
//! entry out), says so in one warning listing what it replaced, and carries on.

use std::borrow::Cow;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::{info, warn};
use rayon::prelude::*;
use xxhash_rust::xxh64::xxh64;

use udd_assets::kr_delta::{
    layout_entry_path, rebuild_uop, stored_entry_path, uop_layout, uop_payloads, KrDeltaContents, KrDeltaEntry,
    KrDeltaFile, KrDeltaFileContents, KrDeltaFileKind, KrDeltaPackage, KrDeltaSource, UopPayload,
    KR_DELTA_CLIENT_FILES, KR_DELTA_FILE_NAME, KR_DELTA_MANIFEST_ENTRY_PATH,
};
use udd_container::{AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder, UddpReader};
use uocf::client::pe::PeImage;
use uocf::client::version::read_exe_version;
use uocf::enhanced::graphics::textures::{decode_dds_rgba8, decode_tga_rgba8, TextureItem};
use uocf::uop_container::decode_payload;
use uocf::uop_container::file::CompressionFlag as UopCompressionFlag;
use uocf::uop_container::package::{LoadMode, UopPackage};
use uocf::uop_container::encode_payload;
use uocf::utils::bundled_files::BundledFile;

use crate::kr_items::{image_difference, KR_IMAGE_EQUIVALENCE_THRESHOLD};
use crate::package_progress::build_and_write_package;

/// The delta in the source checkout (`lib/uocf/assets/kr`, beside the other shipped Kingdom Reborn data), and in the
/// tools release's `shared/` folder.
pub const KR_DELTA: BundledFile = BundledFile {
    name: KR_DELTA_FILE_NAME,
    subdir: "",
    crate_dir: env!("CARGO_MANIFEST_DIR"),
    checkout_dir_from_crate: "../uocf/assets/kr",
};

/// The executables whose version resource names each client in the delta, for information.
const KR_CLIENT_EXE: &str = "UOKR.exe";
const EC_CLIENT_EXE: &str = "UOSA.exe";
/// Zstd level of the delta's entries.
const KR_DELTA_ZSTD_LEVEL: i32 = 19;
/// The data block header timestamp of an entry the rebuild replaced, fixed so that rebuilds are byte-identical.
const REPLACED_ENTRY_FILETIME: u64 = 0;
/// The magic of a DDS file.
const DDS_MAGIC: &[u8] = b"DDS ";
/// How many replaced entries the warning names one by one.
const LOGGED_REPLACEMENTS: usize = 10;
/// The work folder holding a rebuilt client, beside the package being packed.
const REBUILT_CLIENT_DIR_PREFIX: &str = ".kr_client_rebuilt";
/// The seed of the XXH64 hashes the delta records.
const HASH_SEED: u64 = 0;

fn hash64(bytes: &[u8]) -> u64 {
    xxh64(bytes, HASH_SEED)
}

/// `name`, and the file version of `exe` in `dir` when it has one.
fn describe_client(name: &str, dir: &Path, exe: &str) -> String {
    let version = fs::read(dir.join(exe))
        .ok()
        .and_then(|bytes| PeImage::parse(&bytes).ok().and_then(|pe| read_exe_version(&bytes, &pe)));
    match version {
        Some(version) => format!("{name} ({exe} {version})"),
        None => format!("{name} ({exe} version unknown)"),
    }
}

/// The image of a texture entry: a DDS or TGA file, possibly after a `TextureItem` header.
fn decode_texture(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let decode = |bytes: &[u8]| {
        if bytes.starts_with(DDS_MAGIC) { decode_dds_rgba8(bytes).ok() } else { decode_tga_rgba8(bytes).ok() }
    };
    let image = decode(bytes).or_else(|| {
        let mut cursor = Cursor::new(bytes);
        TextureItem::read(&mut cursor).ok()?;
        decode(&bytes[cursor.position() as usize..])
    })?;
    Some((image.width(), image.height(), image.into_raw()))
}

/// How far the Kingdom Reborn and Enhanced textures are apart ([`image_difference`]); `None` when either is no image
/// or their extents differ.
fn texture_difference(kr: &[u8], ec: &[u8]) -> Option<f32> {
    let (kr, ec) = (decode_texture(kr)?, decode_texture(ec)?);
    image_difference((kr.0, kr.1, &kr.2), (ec.0, ec.1, &ec.2))
}

fn uop_compression(payload: &UopPayload) -> eyre::Result<UopCompressionFlag> {
    UopCompressionFlag::from_raw_i16(payload.compression)
        .wrap_err_with(|| format!("entry {:016x}", payload.name_hash))
}

/// The entry's stored payload, compressed as the client compresses it.
fn encode(content: &[u8], payload: &UopPayload) -> eyre::Result<Vec<u8>> {
    Ok(encode_payload(content, uop_compression(payload)?.as_uop_compression())?)
}

/// What a texture entry became in the delta, for the summary.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KrDeltaFileSummary {
    pub name: String,
    /// Entries per [`KrDeltaSource`], in [`KrDeltaSource::ALL`] order.
    pub sources: [usize; KrDeltaSource::ALL.len()],
    pub stored_bytes: u64,
    /// The textures taken from the Enhanced Client in place of Kingdom Reborn's re-encode: name hash and difference.
    pub substituted: Vec<(u64, f32)>,
    /// The textures stored because they differ from the Enhanced ones by more than the threshold: name hash and
    /// difference (`None`: the Enhanced Client lacks the texture, or either is no image, or their extents differ).
    pub kept: Vec<(u64, Option<f32>)>,
}

/// The decision for one entry when making the delta.
struct MadeEntry {
    entry: KrDeltaEntry,
    stored: Vec<u8>,
    difference: Option<Option<f32>>,
}

/// Makes the delta of the client files in `kr_dir` over the Enhanced Client in `ec_dir`, in memory.
pub fn make_kr_delta(kr_dir: &Path, ec_dir: &Path) -> eyre::Result<(KrDeltaContents, Vec<KrDeltaFileSummary>)> {
    let mut files = Vec::new();
    let mut summaries = Vec::new();
    for (name, kind) in KR_DELTA_CLIENT_FILES {
        let path = kr_dir.join(name);
        let bytes = fs::read(&path).wrap_err_with(|| format!("read {}", path.display()))?;
        let enhanced = match kind {
            KrDeltaFileKind::Textures => {
                let ec_path = ec_dir.join(name);
                Some(UopPackage::load_with_mode(&ec_path, LoadMode::Lazy).wrap_err_with(|| format!("load {}", ec_path.display()))?)
            }
            KrDeltaFileKind::Data => None,
        };
        let (contents, summary) =
            make_file_delta(name, &bytes, enhanced.as_ref()).wrap_err_with(|| format!("make the delta of {}", path.display()))?;
        files.push(contents);
        summaries.push(summary);
    }
    let contents = KrDeltaContents {
        kr_client: describe_client("Kingdom Reborn", kr_dir, KR_CLIENT_EXE),
        ec_client: describe_client("Enhanced Client", ec_dir, EC_CLIENT_EXE),
        files,
    };
    contents.check()?;
    Ok((contents, summaries))
}

/// The delta of one client file, its textures compared with `enhanced`, the Enhanced file of the same name.
fn make_file_delta(
    name: &str,
    bytes: &[u8],
    enhanced: Option<&UopPackage>,
) -> eyre::Result<(KrDeltaFileContents, KrDeltaFileSummary)> {
    let payloads = uop_payloads(bytes)?;
    let made = payloads
        .par_iter()
        .map(|payload| {
            let stored = &bytes[payload.address as usize..][..payload.stored_length as usize];
            let compression = uop_compression(payload)?.as_uop_compression();
            let Ok(content) = decode_payload(stored, payload.decompressed_length as usize, compression) else {
                return Ok(MadeEntry::verbatim(stored));
            };
            let exact = encode_payload(&content, compression).is_ok_and(|encoded| encoded == stored);
            let ec_content = match enhanced {
                Some(enhanced) => enhanced.unpack_file_by_hash(payload.name_hash)
                    .wrap_err_with(|| format!("read Enhanced entry {:016x}", payload.name_hash))?,
                None => None,
            };
            let Some(ec_content) = ec_content else {
                let difference = enhanced.is_some().then_some(None);
                return Ok(if exact { MadeEntry::stored(content, difference) } else { MadeEntry::verbatim(stored) });
            };
            if ec_content == content && exact {
                return Ok(MadeEntry { entry: KrDeltaEntry { source: KrDeltaSource::EcCopy, hash64: hash64(&content) },
                    stored: Vec::new(), difference: None });
            }
            let difference = if ec_content == content { Some(0.0) } else { texture_difference(&content, &ec_content) };
            if difference.is_some_and(|difference| difference <= KR_IMAGE_EQUIVALENCE_THRESHOLD) {
                return Ok(MadeEntry {
                    entry: KrDeltaEntry { source: KrDeltaSource::EcSubstitute, hash64: hash64(&ec_content) },
                    stored: Vec::new(),
                    difference: Some(difference),
                });
            }
            Ok(if exact { MadeEntry::stored(content, Some(difference)) } else { MadeEntry::verbatim(stored) })
        })
        .collect::<eyre::Result<Vec<_>>>()?;

    let mut summary = KrDeltaFileSummary { name: name.to_owned(), ..Default::default() };
    let mut stored = Vec::new();
    let mut entries = Vec::with_capacity(made.len());
    for (payload, made) in payloads.iter().zip(made) {
        summary.sources[made.entry.source as usize] += 1;
        match (made.entry.source, made.difference) {
            (KrDeltaSource::EcSubstitute, Some(Some(difference))) => summary.substituted.push((payload.name_hash, difference)),
            (KrDeltaSource::Stored | KrDeltaSource::Verbatim, Some(difference)) => summary.kept.push((payload.name_hash, difference)),
            _ => {}
        }
        stored.extend_from_slice(&made.stored);
        entries.push(made.entry);
    }
    summary.stored_bytes = stored.len() as u64;
    let file = KrDeltaFile { name: name.to_owned(), file_length: bytes.len() as u64, file_hash64: hash64(bytes), entries };
    Ok((KrDeltaFileContents { file, layout: uop_layout(bytes, &payloads), stored }, summary))
}

impl MadeEntry {
    fn stored(content: Vec<u8>, difference: Option<Option<f32>>) -> Self {
        Self { entry: KrDeltaEntry { source: KrDeltaSource::Stored, hash64: hash64(&content) }, stored: content, difference }
    }

    fn verbatim(stored: &[u8]) -> Self {
        Self {
            entry: KrDeltaEntry { source: KrDeltaSource::Verbatim, hash64: hash64(stored) },
            stored: stored.to_vec(),
            difference: None,
        }
    }
}

/// Logs what the delta takes from the Enhanced Client and what it stores.
pub fn log_kr_delta_summary(summaries: &[KrDeltaFileSummary]) {
    for summary in summaries {
        let [copied, substituted, stored, verbatim] = summary.sources;
        info!(
            "{}: {copied} entries the Enhanced Client's byte for byte, {substituted} re-encoded textures taken from it, \
            {stored} stored ({verbatim} as compressed), {} bytes stored",
            summary.name,
            summary.stored_bytes
        );
        if !summary.kept.is_empty() {
            info!("{}: textures kept as Kingdom Reborn's (name hash: difference, none where the Enhanced Client lacks it \
                or the images cannot be compared): {:?}", summary.name, summary.kept.iter()
                .map(|(hash, difference)| format!("{hash:016x}: {difference:?}")).collect::<Vec<_>>());
        }
    }
}

/// Writes the delta as a UDDP package.
pub fn write_kr_delta(contents: &KrDeltaContents, out_file: &Path) -> eyre::Result<()> {
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    let mut add = |path: String, data: Vec<u8>| {
        builder.add_owned_file(AddOwnedFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDictLevel(KR_DELTA_ZSTD_LEVEL),
            width: 0,
            height: 0,
            virtual_path: Some(path.clone()),
            path_hash64: None,
            id: None,
            data,
        }).map_err(|error| eyre::eyre!("add {path}: {error:?}"))
    };
    add(KR_DELTA_MANIFEST_ENTRY_PATH.to_owned(), contents.manifest().encode()?)?;
    for file in &contents.files {
        add(layout_entry_path(&file.file.name), file.layout.clone())?;
        add(stored_entry_path(&file.file.name), file.stored.clone())?;
    }
    build_and_write_package(builder, out_file)
}

/// An entry the rebuild took from the Enhanced Client other than the one the delta recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacedEntry {
    pub file: String,
    pub name_hash: u64,
    pub reason: ReplacedReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacedReason {
    /// The Enhanced entry holds other bytes than the delta was made against: the entry is that Enhanced entry.
    EnhancedDiffers,
    /// The Enhanced Client lacks the entry: the rebuilt file lacks it too.
    EnhancedLacks,
    /// The texture stored in the delta does not match its hash: the entry is the Enhanced entry.
    StoredDamaged,
}

/// Rebuilds the client files of `contents` into `out_dir` from the Enhanced Client in `ec_dir`. Returns the entries
/// taken from the Enhanced Client other than the recorded ones, after one warning listing them.
pub fn rebuild_kr_client(contents: &KrDeltaContents, ec_dir: &Path, out_dir: &Path) -> eyre::Result<Vec<ReplacedEntry>> {
    fs::create_dir_all(out_dir).wrap_err_with(|| format!("create {}", out_dir.display()))?;
    let mut replaced = Vec::new();
    for file in &contents.files {
        let enhanced = match file.file.kind() {
            KrDeltaFileKind::Textures => {
                let ec_path = ec_dir.join(&file.file.name);
                Some(UopPackage::load_with_mode(&ec_path, LoadMode::Lazy).wrap_err_with(|| format!("load {}", ec_path.display()))?)
            }
            KrDeltaFileKind::Data => None,
        };
        let out_file = out_dir.join(&file.file.name);
        replaced.extend(rebuild_file(file, enhanced.as_ref(), &out_file).wrap_err_with(|| format!("rebuild {}", out_file.display()))?);
    }
    if !replaced.is_empty() {
        let listed: Vec<String> = replaced.iter().take(LOGGED_REPLACEMENTS)
            .map(|entry| format!("{} {:016x} ({:?})", entry.file, entry.name_hash, entry.reason)).collect();
        warn!(
            "The Enhanced Client in {} is not the one the Kingdom Reborn delta was made against ({}): {} Kingdom Reborn \
            textures are its own or left out instead (first: {}). The packages hold its textures there; the rest is \
            Kingdom Reborn's.",
            ec_dir.display(), contents.ec_client, replaced.len(), listed.join(", ")
        );
    }
    Ok(replaced)
}

/// What the rebuild puts at an entry the walk leaves a placeholder in: the Enhanced bytes, or nothing.
struct Replacement {
    name_hash: u64,
    compression: UopCompressionFlag,
    content: Option<Vec<u8>>,
}

/// An entry's payload, or the Enhanced bytes to replace it with afterwards.
enum Rebuilt {
    Payload(Vec<u8>),
    Replace(Replacement, Option<ReplacedReason>),
}

fn rebuild_file(file: &KrDeltaFileContents, enhanced: Option<&UopPackage>, out_file: &Path) -> eyre::Result<Vec<ReplacedEntry>> {
    let name = &file.file.name;
    // The payload records, in file order, from a walk with placeholders; then each payload, in parallel.
    let mut payloads = Vec::with_capacity(file.file.entries.len());
    rebuild_uop(&file.layout, |_, payload| {
        payloads.push(*payload);
        Ok(Cow::Owned(vec![0u8; payload.stored_length as usize]))
    })?;
    eyre::ensure!(payloads.len() == file.file.entries.len(), "{} payloads, the delta has {} entries",
        payloads.len(), file.file.entries.len());
    let mut stored_at = 0usize;
    let stored_ranges: Vec<std::ops::Range<usize>> = file.file.entries.iter().zip(&payloads).map(|(entry, payload)| {
        let start = stored_at;
        stored_at += entry.source.stored_length(payload) as usize;
        start..stored_at
    }).collect();
    eyre::ensure!(stored_at == file.stored.len(), "the stored entries take {stored_at} bytes, the delta holds {}", file.stored.len());

    let rebuilt = file.file.entries.par_iter().zip(&payloads).zip(stored_ranges).map(|((entry, payload), range)| {
        rebuild_entry(name, entry, payload, &file.stored[range], enhanced)
    }).collect::<eyre::Result<Vec<_>>>()?;

    let mut replacements = Vec::new();
    let mut replaced = Vec::new();
    let mut rebuilt = rebuilt.into_iter();
    let bytes = rebuild_uop(&file.layout, |_, payload| {
        Ok(Cow::Owned(match rebuilt.next().expect("one per payload") {
            Rebuilt::Payload(bytes) => bytes,
            Rebuilt::Replace(replacement, reason) => {
                if let Some(reason) = reason {
                    replaced.push(ReplacedEntry { file: name.clone(), name_hash: replacement.name_hash, reason });
                }
                replacements.push(replacement);
                vec![0u8; payload.stored_length as usize]
            }
        }))
    })?;
    if replacements.is_empty() {
        eyre::ensure!(
            bytes.len() as u64 == file.file.file_length && hash64(&bytes) == file.file.file_hash64,
            "the rebuilt file is not the Kingdom Reborn one, although every entry is"
        );
        fs::write(out_file, &bytes).wrap_err_with(|| format!("write {}", out_file.display()))?;
        return Ok(replaced);
    }
    // The walk left placeholders: the package editor writes the replacements in their place.
    let partial = out_file.with_extension("partial");
    fs::write(&partial, &bytes).wrap_err_with(|| format!("write {}", partial.display()))?;
    let mut package = UopPackage::load(&partial).wrap_err_with(|| format!("load {}", partial.display()))?;
    package.set_edit_filetime(Some(REPLACED_ENTRY_FILETIME));
    for replacement in replacements {
        match replacement.content {
            Some(content) => {
                package.replace_file(replacement.name_hash, &content, replacement.compression)?;
            }
            None => {
                package.remove_entry(replacement.name_hash);
            }
        }
    }
    package.finalize_and_save(out_file).wrap_err_with(|| format!("write {}", out_file.display()))?;
    fs::remove_file(&partial).wrap_err_with(|| format!("remove {}", partial.display()))?;
    Ok(replaced)
}

fn rebuild_entry(
    file: &str,
    entry: &KrDeltaEntry,
    payload: &UopPayload,
    stored: &[u8],
    enhanced: Option<&UopPackage>,
) -> eyre::Result<Rebuilt> {
    let name_hash = payload.name_hash;
    let compression = uop_compression(payload)?;
    let ec_content = || -> eyre::Result<Option<Vec<u8>>> {
        let Some(enhanced) = enhanced else { return Ok(None) };
        enhanced.unpack_file_by_hash(name_hash).wrap_err_with(|| format!("read Enhanced entry {name_hash:016x}"))
    };
    let replace = |content: Option<Vec<u8>>, reason| {
        let reason = match (&content, reason) {
            (None, Some(_)) => Some(ReplacedReason::EnhancedLacks),
            _ => reason,
        };
        Rebuilt::Replace(Replacement { name_hash, compression, content }, reason)
    };
    let damaged = || -> eyre::Result<Rebuilt> {
        match ec_content()? {
            Some(content) => Ok(replace(Some(content), Some(ReplacedReason::StoredDamaged))),
            None => eyre::bail!("{file}: entry {name_hash:016x} in the delta does not match its hash; the delta is damaged"),
        }
    };
    match entry.source {
        KrDeltaSource::Stored if hash64(stored) == entry.hash64 => {
            let bytes = encode(stored, payload)?;
            eyre::ensure!(bytes.len() == payload.stored_length as usize,
                "{file}: entry {name_hash:016x} compresses to {} bytes, not the client's {}", bytes.len(), payload.stored_length);
            Ok(Rebuilt::Payload(bytes))
        }
        KrDeltaSource::Verbatim if hash64(stored) == entry.hash64 => Ok(Rebuilt::Payload(stored.to_vec())),
        KrDeltaSource::Stored | KrDeltaSource::Verbatim => damaged(),
        KrDeltaSource::EcCopy | KrDeltaSource::EcSubstitute => match ec_content()? {
            Some(content) if hash64(&content) == entry.hash64 => {
                if entry.source == KrDeltaSource::EcCopy {
                    let bytes = encode(&content, payload)?;
                    if bytes.len() == payload.stored_length as usize {
                        return Ok(Rebuilt::Payload(bytes));
                    }
                }
                Ok(replace(Some(content), None))
            }
            content => Ok(replace(content, Some(ReplacedReason::EnhancedDiffers))),
        },
    }
}

/// Where the Kingdom Reborn client files come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KrClientSource {
    /// A Kingdom Reborn client folder.
    Client(PathBuf),
    /// A `kr_delta.uddp`; `None` for the one shipped with the tools ([`KR_DELTA`]).
    Delta(Option<PathBuf>),
}

impl KrClientSource {
    /// `--krdir` when given, else `--kr-delta` or the shipped delta.
    pub fn from_options(kr_dir: Option<&Path>, kr_delta: Option<&Path>) -> Self {
        match kr_dir {
            Some(dir) => Self::Client(dir.to_path_buf()),
            None => Self::Delta(kr_delta.map(Path::to_path_buf)),
        }
    }
}

/// A Kingdom Reborn client folder rebuilt for packing; removed when dropped.
#[derive(Debug)]
pub struct RebuiltKrClient {
    dir: PathBuf,
}

impl RebuiltKrClient {
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for RebuiltKrClient {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.dir) {
            warn!("Could not remove the rebuilt Kingdom Reborn files in {}: {error}", self.dir.display());
        }
    }
}

/// Loads the delta at `path`, or the shipped one.
pub fn load_kr_delta(path: Option<&Path>) -> eyre::Result<KrDeltaContents> {
    let path = uocf::utils::bundled_files::resolve(path, KR_DELTA)?;
    info!("Kingdom Reborn: rebuilding from {}", path.display());
    Ok(KrDeltaPackage::load(&path)?.into_contents())
}

/// The Kingdom Reborn client files the packers read, rebuilt from `source` and the Enhanced Client in `ec_dir` into a
/// work folder in `work_parent` (beside the packages being written).
pub fn prepare_kr_client(source: &KrClientSource, ec_dir: &Path, work_parent: &Path) -> eyre::Result<RebuiltKrClient> {
    let contents = match source {
        KrClientSource::Client(kr_dir) => {
            info!("Kingdom Reborn: comparing {} with {}", kr_dir.display(), ec_dir.display());
            let (contents, summaries) = make_kr_delta(kr_dir, ec_dir)?;
            log_kr_delta_summary(&summaries);
            contents
        }
        KrClientSource::Delta(path) => load_kr_delta(path.as_deref())?,
    };
    let dir = work_parent.join(format!("{REBUILT_CLIENT_DIR_PREFIX}.{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).wrap_err_with(|| format!("remove the stale {}", dir.display()))?;
    }
    let rebuilt = RebuiltKrClient { dir };
    rebuild_kr_client(&contents, ec_dir, rebuilt.dir())?;
    Ok(rebuilt)
}

/// Makes the delta of `kr_dir` over `ec_dir`, checks that it rebuilds the Kingdom Reborn data files byte for byte
/// and every texture as Kingdom Reborn's or the Enhanced one it stands for, and only then writes it to `out_file`.
pub fn make_and_write_kr_delta(kr_dir: &Path, ec_dir: &Path, out_file: &Path) -> eyre::Result<Vec<KrDeltaFileSummary>> {
    let (contents, summaries) = make_kr_delta(kr_dir, ec_dir)?;
    log_kr_delta_summary(&summaries);
    let out_dir = out_file.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let partial = out_file.with_extension("uddp.partial");
    write_kr_delta(&contents, &partial)?;
    let check = (|| -> eyre::Result<()> {
        let written = KrDeltaPackage::load(&partial)?.into_contents();
        eyre::ensure!(written == contents, "{} does not read back as written", partial.display());
        let rebuilt = RebuiltKrClient { dir: out_dir.join(format!("{REBUILT_CLIENT_DIR_PREFIX}.{}", std::process::id())) };
        let replaced = rebuild_kr_client(&written, ec_dir, rebuilt.dir())?;
        eyre::ensure!(replaced.is_empty(), "the delta does not rebuild from the Enhanced Client it was made against");
        verify_rebuilt_client(&written, kr_dir, ec_dir, rebuilt.dir())
    })();
    if let Err(error) = check {
        let _ = fs::remove_file(&partial);
        return Err(error.wrap_err("the delta does not rebuild the Kingdom Reborn files; nothing written"));
    }
    fs::rename(&partial, out_file).wrap_err_with(|| format!("write {}", out_file.display()))?;
    Ok(summaries)
}

/// Compares the rebuilt files with the Kingdom Reborn ones: byte for byte where the delta takes no texture from the
/// Enhanced Client in place of Kingdom Reborn's; else entry by entry, each Kingdom Reborn's or the Enhanced one.
pub fn verify_rebuilt_client(contents: &KrDeltaContents, kr_dir: &Path, ec_dir: &Path, rebuilt_dir: &Path) -> eyre::Result<()> {
    for file in &contents.files {
        let name = &file.file.name;
        let (kr_path, rebuilt_path) = (kr_dir.join(name), rebuilt_dir.join(name));
        let substitutes = file.file.entries.iter().any(|entry| entry.source == KrDeltaSource::EcSubstitute);
        if !substitutes {
            eyre::ensure!(fs::read(&kr_path)? == fs::read(&rebuilt_path)?, "rebuilt {name} differs from {}", kr_path.display());
            continue;
        }
        let (kr, rebuilt) = (UopPackage::load_with_mode(&kr_path, LoadMode::Lazy)?, UopPackage::load_with_mode(&rebuilt_path, LoadMode::Lazy)?);
        let enhanced = UopPackage::load_with_mode(ec_dir.join(name), LoadMode::Lazy)?;
        eyre::ensure!(kr.file_count() == rebuilt.file_count(), "rebuilt {name} has {} entries, Kingdom Reborn's {}",
            rebuilt.file_count(), kr.file_count());
        let hashes: Vec<u64> = kr.iter_files().filter(|entry| !entry.is_empty_slot()).map(|entry| entry.filename_hash()).collect();
        hashes.par_iter().try_for_each(|&hash| -> eyre::Result<()> {
            let rebuilt_content = rebuilt.unpack_file_by_hash(hash)?;
            eyre::ensure!(
                rebuilt_content == kr.unpack_file_by_hash(hash)? || rebuilt_content == enhanced.unpack_file_by_hash(hash)?,
                "rebuilt {name} entry {hash:016x} is neither Kingdom Reborn's nor the Enhanced Client's"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::uop_container::hash::hash_file_name_single;

    /// The UOP version of the Kingdom Reborn client files.
    const TEST_UOP_VERSION: u32 = 4;
    const TEST_BLOCK_SIZE: u32 = 100;
    /// Extent of the test textures.
    const TEXTURE_SIDE: u16 = 4;
    /// Grey levels of the test textures: a re-encode of [`GREY`] within the threshold, and another image.
    const GREY: u8 = 100;
    const GREY_REENCODED: u8 = 103;
    const OTHER_GREY: u8 = 200;
    /// TGA header fields of an uncompressed 32-bit image stored top row first.
    const TGA_HEADER_SIZE: usize = 18;
    const TGA_UNCOMPRESSED_TRUE_COLOR: u8 = 2;
    const TGA_BITS_PER_PIXEL: u8 = 32;
    const TGA_ALPHA_BITS_TOP_LEFT: u8 = 0x28;
    const OPAQUE: u8 = u8::MAX;
    /// Repeats of a data record, so that it compresses.
    const RECORD_REPEATS: usize = 9;

    const TEXTURES: &str = "Texture.uop";
    const SAME: &str = "build/worldart/same.tga";
    const REENCODED: &str = "build/worldart/reencoded.tga";
    const REDRAWN: &str = "build/worldart/redrawn.tga";
    const KR_ONLY: &str = "build/worldart/kr_only.tga";
    const RECORD: &str = "build/tileart/00000001.dat";

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("udd_conv_kr_delta_{test}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A grey TGA image.
    fn tga(grey: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; TGA_HEADER_SIZE];
        bytes[2] = TGA_UNCOMPRESSED_TRUE_COLOR;
        bytes[12..14].copy_from_slice(&TEXTURE_SIDE.to_le_bytes());
        bytes[14..16].copy_from_slice(&TEXTURE_SIDE.to_le_bytes());
        bytes[16] = TGA_BITS_PER_PIXEL;
        bytes[17] = TGA_ALPHA_BITS_TOP_LEFT;
        for _ in 0..usize::from(TEXTURE_SIDE) * usize::from(TEXTURE_SIDE) {
            bytes.extend_from_slice(&[grey, grey, grey, OPAQUE]);
        }
        bytes
    }

    fn write_uop(path: &Path, entries: &[(&str, Vec<u8>)]) {
        let mut package = UopPackage::new(TEST_UOP_VERSION, TEST_BLOCK_SIZE).unwrap();
        for (name, content) in entries {
            package.add_file_from_memory(content, name, UopCompressionFlag::Zlib).unwrap();
        }
        package.finalize_and_save(path).unwrap();
    }

    /// The six client files of both clients: data files, and Kingdom Reborn textures that are the Enhanced ones,
    /// re-encoded, redrawn or its alone; `ec_textures` are the Enhanced `Texture.uop`.
    fn write_clients(dir: &Path, ec_textures: &[(&str, Vec<u8>)]) -> (PathBuf, PathBuf) {
        let (kr, ec) = (dir.join("kr"), dir.join("ec"));
        fs::create_dir_all(&kr).unwrap();
        fs::create_dir_all(&ec).unwrap();
        for (name, kind) in KR_DELTA_CLIENT_FILES {
            match kind {
                KrDeltaFileKind::Data => {
                    write_uop(&kr.join(name), &[(RECORD, format!("{name} record").repeat(RECORD_REPEATS).into_bytes())])
                }
                KrDeltaFileKind::Textures if name == TEXTURES => {
                    let textures = [(SAME, tga(GREY)), (REENCODED, tga(GREY)), (REDRAWN, tga(GREY)), (KR_ONLY, tga(GREY))];
                    write_uop(&kr.join(name), &textures);
                    write_uop(&ec.join(name), ec_textures);
                }
                KrDeltaFileKind::Textures => {
                    write_uop(&kr.join(name), &[(SAME, tga(GREY))]);
                    write_uop(&ec.join(name), &[(SAME, tga(GREY))]);
                }
            }
        }
        (kr, ec)
    }

    fn enhanced_textures() -> Vec<(&'static str, Vec<u8>)> {
        vec![(SAME, tga(GREY)), (REENCODED, tga(GREY_REENCODED)), (REDRAWN, tga(OTHER_GREY))]
    }

    /// The texture `name` in the `Texture.uop` in `dir`.
    fn texture(dir: &Path, name: &str) -> Option<Vec<u8>> {
        UopPackage::load(dir.join(TEXTURES)).unwrap().unpack_file_by_hash(hash_file_name_single(name)).unwrap()
    }

    #[test]
    fn the_delta_rebuilds_kingdom_reborn_with_its_reencoded_textures_from_the_enhanced_client() {
        let dir = temp_dir("rebuild");
        let (kr, ec) = write_clients(&dir, &enhanced_textures());
        let out = dir.join(KR_DELTA_FILE_NAME);
        let summaries = make_and_write_kr_delta(&kr, &ec, &out).unwrap();
        let textures = summaries.iter().find(|summary| summary.name == TEXTURES).unwrap();
        assert_eq!(textures.sources, [1, 1, 2, 0], "copied, substituted, stored, verbatim");
        assert_eq!(textures.kept.len(), 2, "the redrawn texture and Kingdom Reborn's own");

        let contents = KrDeltaPackage::load(&out).unwrap().into_contents();
        let rebuilt = dir.join("rebuilt");
        assert_eq!(rebuild_kr_client(&contents, &ec, &rebuilt).unwrap(), []);
        for (name, _) in KR_DELTA_CLIENT_FILES.iter().filter(|(name, _)| *name != TEXTURES) {
            assert_eq!(fs::read(rebuilt.join(name)).unwrap(), fs::read(kr.join(name)).unwrap(), "{name}");
        }
        assert_eq!(texture(&rebuilt, REENCODED), Some(tga(GREY_REENCODED)), "the Enhanced texture");
        for name in [SAME, REDRAWN, KR_ONLY] {
            assert_eq!(texture(&rebuilt, name), Some(tga(GREY)), "{name}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_enhanced_client_gives_its_own_textures_with_a_warning_and_no_error() {
        let dir = temp_dir("other_ec");
        let (kr, ec) = write_clients(&dir, &enhanced_textures());
        let (contents, _) = make_kr_delta(&kr, &ec).unwrap();
        write_uop(&ec.join(TEXTURES), &[(SAME, tga(OTHER_GREY)), (REDRAWN, tga(OTHER_GREY))]);
        let rebuilt = dir.join("rebuilt");
        let replaced = rebuild_kr_client(&contents, &ec, &rebuilt).unwrap();
        let reasons: Vec<(u64, ReplacedReason)> = replaced.iter().map(|entry| (entry.name_hash, entry.reason)).collect();
        assert_eq!(reasons, [
            (hash_file_name_single(SAME), ReplacedReason::EnhancedDiffers),
            (hash_file_name_single(REENCODED), ReplacedReason::EnhancedLacks),
        ]);
        assert_eq!(texture(&rebuilt, SAME), Some(tga(OTHER_GREY)));
        assert_eq!(texture(&rebuilt, REENCODED), None);
        assert_eq!(texture(&rebuilt, REDRAWN), Some(tga(GREY)), "stored in the delta");
        assert_eq!(fs::read(rebuilt.join("tileart.uop")).unwrap(), fs::read(kr.join("tileart.uop")).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_stored_entry_is_caught_by_its_hash() {
        let dir = temp_dir("damaged");
        let (kr, ec) = write_clients(&dir, &enhanced_textures());
        let (contents, _) = make_kr_delta(&kr, &ec).unwrap();
        let damage = |contents: &mut KrDeltaContents, name: &str| {
            contents.files.iter_mut().find(|file| file.file.name == name).unwrap().stored[0] ^= 1;
        };

        let mut textures = contents.clone();
        damage(&mut textures, TEXTURES);
        let rebuilt = dir.join("textures");
        let replaced = rebuild_kr_client(&textures, &ec, &rebuilt).unwrap();
        assert_eq!(replaced.iter().map(|entry| entry.reason).collect::<Vec<_>>(), [ReplacedReason::StoredDamaged]);
        assert_eq!(texture(&rebuilt, REDRAWN), Some(tga(OTHER_GREY)), "the Enhanced texture in its place");

        let mut data = contents;
        damage(&mut data, "tileart.uop");
        let error = format!("{:?}", rebuild_kr_client(&data, &ec, &dir.join("data")).unwrap_err());
        assert!(error.contains("damaged"), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }
}
