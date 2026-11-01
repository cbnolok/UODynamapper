//! Classic art and gumps as images, in either container ([`ClassicImageFiles`]): `artidx.mul` + `art.mul` or
//! `artLegacyMUL.uop`, `gumpidx.mul` + `gumpart.mul` or `gumpartLegacyMUL.uop`. A record is exported as the RGBA
//! image it stores ([`ClassicImageFiles::read`]); an image is encoded for an id with the shipped encoders
//! ([`crate::classic::art_write`], [`crate::classic::gump_write`], through
//! [`crate::enhanced::entry_image::encode_entry_image`]) and checked before anything is written
//! ([`ClassicImageFiles::prepare`]); the result goes to a copy, never the source ([`ClassicImageFiles::write_copy`]).
//!
//! The records are the same bytes in both containers, but for the gumps: a `gumpart.mul` slot holds the rows and
//! its `gumpidx.mul` extra field the size (`width << 16 | height`); a UOP entry starts with the `u32` width and
//! height. The `artidx.mul` extra field is 0 in the modern clients (old memory in UO Gold): a replaced record keeps
//! it, an added one gets 0.
//!
//! A MUL copy is the source with the new record appended to the data file and its index record pointing there; every
//! other byte is kept. A UOP copy goes through the entry editor ([`UopEntryEditor`]): the entry is replaced (keeping
//! its name and compression) or added under the eight-digit name of the shipped entries.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use image::RgbaImage;

use crate::audio::validate::FormatPolicy;
use crate::classic::art::{ART_LEGACY_UOP_MAX_ID_EXCLUSIVE, LAND_DIMENSION};
use crate::classic::art_write::{uop_entry_name, ArtColors, ArtKind};
use crate::classic::gump::uop_gump_candidates;
use crate::classic::gump_write::GumpColors;
use crate::enhanced::dds_writer::SizeChange;
use crate::enhanced::entry_edit::{EntryTarget, UopEntryEditor};
use crate::enhanced::entry_image::{encode_entry_image, entry_image_format, EntryImageFormat, CLASSIC_ART_PACKAGE, PLACEHOLDER_GUMP};
use crate::uop_container::hash::hash_file_name_single;
use crate::utils::output_file::{refuse_same_file, write_through_temp};
use crate::utils::path::find_file_ignoring_case;

crate::eyre_imports!();

/// Bytes of one index record: lookup, length and extra, each a little-endian `u32`.
const INDEX_RECORD_BYTES: usize = 12;
const INDEX_FIELD_BYTES: usize = size_of::<u32>();
/// The lookup of an absent record.
const ABSENT_LOOKUP: u32 = u32::MAX;
/// The `artidx.mul` extra field of an added record.
const ADDED_ART_EXTRA: u32 = 0;
/// Bytes of the width/height header of a `gumpartLegacyMUL.uop` entry.
const GUMP_UOP_HEADER_BYTES: usize = 2 * size_of::<u32>();
/// The `gumpidx.mul` extra field holds the width in its high half.
const GUMP_EXTRA_WIDTH_SHIFT: u32 = u16::BITS;
const GUMP_EXTRA_HEIGHT_MASK: u32 = u16::MAX as u32;
/// The largest gump id a `gumpartLegacyMUL.uop` copy takes (the client's ids are below it).
pub const GUMP_UOP_MAX_ID_EXCLUSIVE: u32 = 0x10000;
const LAND_SIDE: u16 = LAND_DIMENSION as u16;

pub const ART_INDEX: &str = "artidx.mul";
pub const ART_DATA: &str = "art.mul";
pub const ART_UOP: &str = "artLegacyMUL.uop";
pub const GUMP_INDEX: &str = "gumpidx.mul";
pub const GUMP_DATA: &str = "gumpart.mul";
pub const GUMP_UOP: &str = "gumpartLegacyMUL.uop";

/// What the files hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicImageKind {
    /// Land tiles (ids below 0x4000) and statics (item id + 0x4000).
    Art,
    Gump,
}

impl ClassicImageKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Art => "art",
            Self::Gump => "gump",
        }
    }

    /// The rules an image must follow to be stored as id `id`.
    pub fn rules(self, id: u32) -> &'static str {
        match (self, ArtKind::of_id(id)) {
            (Self::Art, ArtKind::Land) => {
                "Land tile (art id below 0x4000): a 44x44 image; the diamond is opaque and the corners transparent; \
                 alpha 0 or 255 only; channels round to 5 bits."
            }
            (Self::Art, ArtKind::Static) => {
                "Static (item id + 0x4000): any size; alpha 0 is transparent, 255 opaque, other alphas are refused; \
                 channels round to 5 bits; opaque black is stored as 0x0001 (0 is transparent)."
            }
            (Self::Gump, _) => {
                "Gump: alpha 0 is transparent, 255 opaque, other alphas are refused; channels round to 5 bits; \
                 opaque black is stored as 0x0001 (0 is transparent). A size other than the gump's needs a size \
                 change allowed (gump layouts may assume it)."
            }
        }
    }
}

/// The files of a kind, in one container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassicImageContainer {
    Mul { index: PathBuf, data: PathBuf },
    Uop(PathBuf),
}

impl ClassicImageContainer {
    /// The same file names in `dir`.
    pub fn in_dir(&self, dir: &Path) -> Self {
        let name = |path: &Path| dir.join(path.file_name().unwrap_or_default());
        match self {
            Self::Mul { index, data } => Self::Mul { index: name(index), data: name(data) },
            Self::Uop(path) => Self::Uop(name(path)),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Mul { index, data } => format!("{} + {}", index.display(), data.display()),
            Self::Uop(path) => path.display().to_string(),
        }
    }

    fn paths(&self) -> Vec<&Path> {
        match self {
            Self::Mul { index, data } => vec![index, data],
            Self::Uop(path) => vec![path],
        }
    }
}

/// Art or gump files ([`ClassicImageKind`]) in one container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicImageFiles {
    pub kind: ClassicImageKind,
    pub container: ClassicImageContainer,
}

/// A stored record as an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredImage {
    pub format: EntryImageFormat,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// An image encoded for an id, not written yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedImage {
    pub id: u32,
    pub format: EntryImageFormat,
    /// The record as the UOP stores it (a gump with its width/height header).
    pub uop_record: Vec<u8>,
    /// The `gumpidx.mul` extra field (gumps).
    pub gump_extra: Option<u32>,
    /// What the conversion changed (rounded colors, a new size).
    pub notes: Vec<String>,
    /// The id holds a record now.
    pub replaces: bool,
    /// The stored image, decoded back from the record: what the client will draw.
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What [`ClassicImageFiles::write_copy`] did to the id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageWrite {
    Added,
    Replaced,
    /// The id already held the record.
    Unchanged,
}

impl ClassicImageFiles {
    /// The files of `kind` in the client folder `dir`, each container found (MUL first).
    pub fn find(dir: &Path, kind: ClassicImageKind) -> Vec<Self> {
        let (index, data, uop) = match kind {
            ClassicImageKind::Art => (ART_INDEX, ART_DATA, ART_UOP),
            ClassicImageKind::Gump => (GUMP_INDEX, GUMP_DATA, GUMP_UOP),
        };
        let mut found = Vec::new();
        if let (Some(index), Some(data)) = (find_file_ignoring_case(dir, index), find_file_ignoring_case(dir, data)) {
            found.push(Self { kind, container: ClassicImageContainer::Mul { index, data } });
        }
        if let Some(uop) = find_file_ignoring_case(dir, uop) {
            found.push(Self { kind, container: ClassicImageContainer::Uop(uop) });
        }
        found
    }

    /// The record of `id` as stored (a UOP gump with its header), with the index extra field for a MUL record;
    /// `None` when the id holds none.
    fn record(&self, id: u32) -> eyre::Result<Option<(Vec<u8>, Option<u32>)>> {
        match &self.container {
            ClassicImageContainer::Mul { index, data } => {
                let Some((lookup, length, extra)) = read_index_record(index, id)? else {
                    return Ok(None);
                };
                if lookup == ABSENT_LOOKUP || length == 0 {
                    return Ok(None);
                }
                let mut file = std::fs::File::open(data).wrap_err_with(|| format!("open {}", data.display()))?;
                let mut record = vec![0; length as usize];
                file.seek(SeekFrom::Start(u64::from(lookup)))
                    .and_then(|_| file.read_exact(&mut record))
                    .wrap_err_with(|| format!("{}: record {id} ({length} bytes at {lookup}) is past the end", data.display()))?;
                if self.kind == ClassicImageKind::Gump {
                    let (width, height) = (extra >> GUMP_EXTRA_WIDTH_SHIFT, extra & GUMP_EXTRA_HEIGHT_MASK);
                    let mut uop = Vec::with_capacity(GUMP_UOP_HEADER_BYTES + record.len());
                    uop.extend_from_slice(&width.to_le_bytes());
                    uop.extend_from_slice(&height.to_le_bytes());
                    uop.extend_from_slice(&record);
                    return Ok(Some((uop, Some(extra))));
                }
                Ok(Some((record, Some(extra))))
            }
            ClassicImageContainer::Uop(path) => {
                let editor = UopEntryEditor::load(path)?;
                let Some(target) = self.uop_target(&editor, id) else {
                    return Ok(None);
                };
                Ok(editor.package().unpack_file_by_hash(target.hash())?.map(|record| (record, None)))
            }
        }
    }

    /// The record of `id` as a UOP stores it (a gump with its width/height header), `None` when it holds none.
    pub fn stored_record(&self, id: u32) -> eyre::Result<Option<Vec<u8>>> {
        self.check_id(id)?;
        Ok(self.record(id)?.map(|(record, _)| record))
    }

    /// The entry of `id` in a UOP: the existing one, else the eight-digit name of the shipped entries.
    fn uop_target(&self, editor: &UopEntryEditor, id: u32) -> Option<EntryTarget> {
        let names = match self.kind {
            ClassicImageKind::Art => vec![uop_entry_name(id)],
            ClassicImageKind::Gump => uop_gump_candidates(id).to_vec(),
        };
        names.into_iter().map(EntryTarget::Name).find(|target| editor.package().get_file_by_hash(target.hash()).is_some())
    }

    fn check_id(&self, id: u32) -> eyre::Result<()> {
        let limit = match (&self.container, self.kind) {
            (ClassicImageContainer::Mul { index, .. }, _) => {
                let length = std::fs::metadata(index).wrap_err_with(|| format!("read {}", index.display()))?.len();
                u32::try_from(length / INDEX_RECORD_BYTES as u64).unwrap_or(u32::MAX)
            }
            (ClassicImageContainer::Uop(_), ClassicImageKind::Art) => ART_LEGACY_UOP_MAX_ID_EXCLUSIVE,
            (ClassicImageContainer::Uop(_), ClassicImageKind::Gump) => GUMP_UOP_MAX_ID_EXCLUSIVE,
        };
        eyre::ensure!(id < limit, "{} id {id:#x} is past the files' last id ({:#x})", self.kind.label(), limit.saturating_sub(1));
        Ok(())
    }

    /// The image `id` stores, `None` when it holds no record (or a 0x0 gump placeholder).
    pub fn read(&self, id: u32) -> eyre::Result<Option<StoredImage>> {
        self.check_id(id)?;
        let Some((record, _)) = self.record(id)? else {
            return Ok(None);
        };
        decode_record(self.kind, id, &record).map(|image| image.filter(|image| image.width > 0 && image.height > 0))
    }

    /// Encodes `image` for `id` as the shipped records are, checked by the encoders' rules (see
    /// [`ClassicImageKind::rules`]); an image of another size than the record is refused unless `size_change`
    /// allows it. Nothing is written.
    pub fn prepare(&self, id: u32, image: &RgbaImage, size_change: SizeChange) -> eyre::Result<PreparedImage> {
        self.check_id(id)?;
        let current = self.record(id)?;
        let format = match self.kind {
            ClassicImageKind::Art => match current.as_ref().map(|(record, _)| record) {
                Some(record) => entry_image_format(CLASSIC_ART_PACKAGE, hash_file_name_single(&uop_entry_name(id)), record)
                    .ok_or_else(|| eyre!("art {id:#x} is no art record"))?,
                None => {
                    let kind = ArtKind::of_id(id);
                    let size = (kind == ArtKind::Land).then_some((LAND_SIDE, LAND_SIDE));
                    EntryImageFormat::ClassicArt { art_id: id, kind, size }
                }
            },
            ClassicImageKind::Gump => match current.as_ref().map(|(record, _)| record) {
                Some(record) if record[..] != PLACEHOLDER_GUMP => {
                    let gump = GumpColors::decode_uop(record).wrap_err_with(|| format!("gump {id} does not decode"))?;
                    EntryImageFormat::ClassicGump(Some((gump.width, gump.height)))
                }
                _ => EntryImageFormat::ClassicGump(None),
            },
        };
        let encoded = encode_entry_image(format, image, size_change)?;
        let stored = decode_record(self.kind, id, &encoded.bytes)?.ok_or_else(|| eyre!("the encoded record does not decode"))?;
        let gump_extra = match self.kind {
            ClassicImageKind::Gump => Some(GumpColors::decode_uop(&encoded.bytes)?.index_extra()),
            ClassicImageKind::Art => None,
        };
        Ok(PreparedImage {
            id,
            format: encoded.format,
            uop_record: encoded.bytes,
            gump_extra,
            notes: encoded.notes,
            replaces: current.is_some(),
            width: stored.width,
            height: stored.height,
            rgba: stored.rgba,
        })
    }

    /// Writes a copy of the files to `output` (the same container; no source file) with `prepared` as its id.
    pub fn write_copy(&self, prepared: &PreparedImage, output: &ClassicImageContainer) -> eyre::Result<ImageWrite> {
        for path in output.paths() {
            eyre::ensure!(path.is_absolute(), "the output must be an absolute path: {}", path.display());
            refuse_same_file(self.container.paths(), path, "source file")?;
        }
        let id = prepared.id;
        let current = self.record(id)?;
        let write = match &current {
            Some((record, _)) if *record == prepared.uop_record => ImageWrite::Unchanged,
            Some(_) => ImageWrite::Replaced,
            None => ImageWrite::Added,
        };
        match (&self.container, output) {
            (ClassicImageContainer::Mul { index, data }, ClassicImageContainer::Mul { index: index_out, data: data_out }) => {
                eyre::ensure!(index_out != data_out, "the index and data outputs are one path: {}", index_out.display());
                let payload = match self.kind {
                    ClassicImageKind::Art => &prepared.uop_record[..],
                    ClassicImageKind::Gump => &prepared.uop_record[GUMP_UOP_HEADER_BYTES..],
                };
                let extra = match (self.kind, &current) {
                    (ClassicImageKind::Gump, _) => prepared.gump_extra.ok_or_else(|| eyre!("a gump without its size"))?,
                    (ClassicImageKind::Art, Some((_, Some(extra)))) => *extra,
                    (ClassicImageKind::Art, _) => ADDED_ART_EXTRA,
                };
                write_mul_copy(index, data, index_out, data_out, id, payload, extra, write == ImageWrite::Unchanged)?;
            }
            (ClassicImageContainer::Uop(source), ClassicImageContainer::Uop(path)) => {
                let mut editor = UopEntryEditor::load(source)?;
                match self.uop_target(&editor, id) {
                    Some(target) => {
                        editor.replace(&target, &prepared.uop_record, FormatPolicy::VerifiedOnly)?;
                    }
                    None => {
                        let name = match self.kind {
                            ClassicImageKind::Art => uop_entry_name(id),
                            ClassicImageKind::Gump => uop_gump_candidates(id)[0].clone(),
                        };
                        editor.add(&EntryTarget::Name(name), &prepared.uop_record, None, FormatPolicy::VerifiedOnly)?;
                    }
                }
                editor.save_as(path)?;
            }
            _ => eyre::bail!("the output is another container than the source ({})", self.container.label()),
        }
        Ok(write)
    }
}

/// The image of a record stored as a UOP holds it.
fn decode_record(kind: ClassicImageKind, id: u32, record: &[u8]) -> eyre::Result<Option<StoredImage>> {
    match kind {
        ClassicImageKind::Art => {
            let art_kind = ArtKind::of_id(id);
            let art = ArtColors::decode(art_kind, record).wrap_err_with(|| format!("art {id:#x} does not decode"))?;
            let (width, height, rgba) = art.to_rgba();
            let size = (width, height);
            let format = EntryImageFormat::ClassicArt { art_id: id, kind: art_kind, size: Some(size) };
            Ok(Some(StoredImage { format, width: u32::from(width), height: u32::from(height), rgba }))
        }
        ClassicImageKind::Gump => {
            if record == PLACEHOLDER_GUMP {
                return Ok(None);
            }
            let gump = GumpColors::decode_uop(record).wrap_err_with(|| format!("gump {id} does not decode"))?;
            let format = EntryImageFormat::ClassicGump(Some((gump.width, gump.height)));
            Ok(Some(StoredImage { format, width: u32::from(gump.width), height: u32::from(gump.height), rgba: gump.to_rgba() }))
        }
    }
}

fn read_index_record(index: &Path, id: u32) -> eyre::Result<Option<(u32, u32, u32)>> {
    let mut file = std::fs::File::open(index).wrap_err_with(|| format!("open {}", index.display()))?;
    let mut record = [0; INDEX_RECORD_BYTES];
    if file.seek(SeekFrom::Start(u64::from(id) * INDEX_RECORD_BYTES as u64)).is_err() || file.read_exact(&mut record).is_err() {
        return Ok(None);
    }
    let field = |at: usize| u32::from_le_bytes(record[at..at + INDEX_FIELD_BYTES].try_into().expect("4 bytes"));
    Ok(Some((field(0), field(INDEX_FIELD_BYTES), field(2 * INDEX_FIELD_BYTES))))
}

/// Copies the index and data files with `payload` appended to the data and record `id` pointing at it (`unchanged`:
/// plain copies).
#[allow(clippy::too_many_arguments)]
fn write_mul_copy(index: &Path, data: &Path, index_out: &Path, data_out: &Path, id: u32, payload: &[u8], extra: u32, unchanged: bool) -> eyre::Result<()> {
    let mut index_bytes = std::fs::read(index).wrap_err_with(|| format!("read {}", index.display()))?;
    let end = std::fs::metadata(data).wrap_err_with(|| format!("read {}", data.display()))?.len();
    if !unchanged {
        let lookup = u32::try_from(end).map_err(|_| eyre!("{} is past the 4 GiB the index addresses", data.display()))?;
        let length = u32::try_from(payload.len()).map_err(|_| eyre!("a record of {} bytes", payload.len()))?;
        eyre::ensure!(end + u64::from(length) <= u64::from(u32::MAX), "{} would pass the 4 GiB the index addresses", data.display());
        let at = id as usize * INDEX_RECORD_BYTES;
        let record = index_bytes.get_mut(at..at + INDEX_RECORD_BYTES).ok_or_else(|| eyre!("{} has no record {id}", index.display()))?;
        for (field, value) in [lookup, length, extra].into_iter().enumerate() {
            record[field * INDEX_FIELD_BYTES..][..INDEX_FIELD_BYTES].copy_from_slice(&value.to_le_bytes());
        }
    }
    write_through_temp(data_out, |temp| {
        std::fs::copy(data, temp).wrap_err_with(|| format!("copy {} to {}", data.display(), temp.display()))?;
        if !unchanged {
            let mut file = std::fs::OpenOptions::new().append(true).open(temp).wrap_err_with(|| format!("open {}", temp.display()))?;
            file.write_all(payload).and_then(|()| file.sync_all()).wrap_err_with(|| format!("write {}", temp.display()))?;
        }
        Ok(())
    })?;
    write_through_temp(index_out, |temp| std::fs::write(temp, &index_bytes).wrap_err_with(|| format!("write {}", temp.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::art_write::STATIC_ART_ID_BASE;
    use crate::uop_container::file::CompressionFlag;
    use crate::uop_container::package::{NewEntryCompression, UopPackage};

    const RED: [u8; 4] = [0xF8, 0, 0, 0xFF];
    const CLEAR: [u8; 4] = [0; 4];
    /// Records in the test index files.
    const RECORDS: u32 = 0x4010;
    const LAND_ID: u32 = 3;
    const STATIC_ID: u32 = STATIC_ART_ID_BASE + 5;
    const GUMP_ID: u32 = 12;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_image_files_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn image(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| image::Rgba(pixel(x, y)))
    }

    fn land_image() -> RgbaImage {
        // The diamond: rows of 2, 4, .. 44 then 44, .. 2 pixels, centred.
        let half = LAND_DIMENSION as i32 / 2;
        image(LAND_DIMENSION as u32, LAND_DIMENSION as u32, |x, y| {
            let row = if (y as i32) < half { y as i32 } else { LAND_DIMENSION as i32 - 1 - y as i32 };
            let start = half - 1 - row;
            if (x as i32) >= start && (x as i32) < start + 2 * (row + 1) { RED } else { CLEAR }
        })
    }

    /// Empty index files of [`RECORDS`] absent records and an empty data file.
    fn empty_mul(dir: &Path, index: &str, data: &str) -> ClassicImageContainer {
        let record: Vec<u8> = [ABSENT_LOOKUP, 0, 0].iter().flat_map(|value| value.to_le_bytes()).collect();
        std::fs::write(dir.join(index), record.repeat(RECORDS as usize)).unwrap();
        std::fs::write(dir.join(data), []).unwrap();
        ClassicImageContainer::Mul { index: dir.join(index), data: dir.join(data) }
    }

    #[test]
    fn art_imports_into_a_mul_copy_and_reads_back() {
        let dir = temp("art_mul");
        let (source_dir, out_dir) = (dir.join("source"), dir.join("out"));
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::create_dir_all(&out_dir).unwrap();
        let files = ClassicImageFiles { kind: ClassicImageKind::Art, container: empty_mul(&source_dir, ART_INDEX, ART_DATA) };
        let output = files.container.in_dir(&out_dir);

        let land = files.prepare(LAND_ID, &land_image(), SizeChange::Refuse).unwrap();
        assert!(!land.replaces);
        assert_eq!(files.write_copy(&land, &output).unwrap(), ImageWrite::Added);
        let copy = ClassicImageFiles { kind: ClassicImageKind::Art, container: output.clone() };
        assert_eq!(copy.read(LAND_ID).unwrap().unwrap().rgba, land_image().into_raw());

        // A land tile has one size, and its corners are transparent.
        assert!(files.prepare(LAND_ID, &image(8, 8, |_, _| RED), SizeChange::Allow).is_err());
        let error = format!("{:#}", files.prepare(LAND_ID, &image(44, 44, |_, _| RED), SizeChange::Allow).unwrap_err());
        assert!(error.contains("outside the land diamond"), "{error}");
        // Partial alpha is refused.
        assert!(files.prepare(STATIC_ID, &image(2, 2, |_, _| [9, 9, 9, 0x80]), SizeChange::Allow).is_err());

        // A static into the copy, then replaced in a second copy (another size needs the change allowed).
        let first = image(3, 2, |x, _| if x == 1 { CLEAR } else { RED });
        let prepared = files.prepare(STATIC_ID, &first, SizeChange::Refuse).unwrap();
        files.write_copy(&prepared, &ClassicImageContainer::Mul { index: out_dir.join("a.idx"), data: out_dir.join("a.mul") }).unwrap();
        let second_source = ClassicImageFiles { kind: ClassicImageKind::Art, container: ClassicImageContainer::Mul { index: out_dir.join("a.idx"), data: out_dir.join("a.mul") } };
        assert!(second_source.prepare(STATIC_ID, &image(4, 4, |_, _| RED), SizeChange::Refuse).is_err());
        let black = second_source.prepare(STATIC_ID, &image(3, 2, |_, _| [0, 0, 0, 0xFF]), SizeChange::Refuse).unwrap();
        assert!(black.replaces && black.notes.iter().any(|note| note.contains("0x0001")), "{:?}", black.notes);
        let replaced = ClassicImageContainer::Mul { index: out_dir.join("b.idx"), data: out_dir.join("b.mul") };
        assert_eq!(second_source.write_copy(&black, &replaced).unwrap(), ImageWrite::Replaced);
        let back = ClassicImageFiles { kind: ClassicImageKind::Art, container: replaced };
        assert_eq!(back.read(STATIC_ID).unwrap().unwrap().rgba, black.rgba);
        assert_eq!(back.read(LAND_ID).unwrap(), None);

        // Never the source.
        assert!(files.write_copy(&land, &files.container).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gumps_import_into_mul_and_uop_copies() {
        let dir = temp("gump");
        let source_dir = dir.join("source");
        std::fs::create_dir_all(&source_dir).unwrap();
        let mul = ClassicImageFiles { kind: ClassicImageKind::Gump, container: empty_mul(&source_dir, GUMP_INDEX, GUMP_DATA) };
        let picture = image(5, 3, |x, y| if (x + y) % 2 == 0 { RED } else { CLEAR });
        let prepared = mul.prepare(GUMP_ID, &picture, SizeChange::Refuse).unwrap();
        assert_eq!(prepared.gump_extra, Some(5 << GUMP_EXTRA_WIDTH_SHIFT | 3));
        let mul_out = ClassicImageContainer::Mul { index: dir.join(GUMP_INDEX), data: dir.join(GUMP_DATA) };
        mul.write_copy(&prepared, &mul_out).unwrap();
        let mul_copy = ClassicImageFiles { kind: ClassicImageKind::Gump, container: mul_out };
        assert_eq!(mul_copy.read(GUMP_ID).unwrap().unwrap().rgba, picture.clone().into_raw());
        // The same size is required once the gump exists.
        assert!(mul_copy.prepare(GUMP_ID, &image(6, 3, |_, _| RED), SizeChange::Refuse).is_err());
        assert!(mul_copy.prepare(GUMP_ID, &image(6, 3, |_, _| RED), SizeChange::Allow).is_ok());

        // A package with one other gump: the gump is added under the eight-digit name, then replaced.
        let mut package = UopPackage::new_default();
        package.add_entry_by_name(&uop_gump_candidates(1)[0], &prepared.uop_record, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
        let source_uop = source_dir.join(GUMP_UOP);
        package.finalize_and_save(&source_uop).unwrap();
        let uop = ClassicImageFiles { kind: ClassicImageKind::Gump, container: ClassicImageContainer::Uop(source_uop) };
        let added = dir.join("added.uop");
        assert_eq!(uop.write_copy(&uop.prepare(GUMP_ID, &picture, SizeChange::Refuse).unwrap(), &ClassicImageContainer::Uop(added.clone())).unwrap(), ImageWrite::Added);
        let added = ClassicImageFiles { kind: ClassicImageKind::Gump, container: ClassicImageContainer::Uop(added) };
        assert_eq!(added.read(GUMP_ID).unwrap().unwrap().rgba, picture.clone().into_raw());
        let flipped = image(5, 3, |x, y| if (x + y) % 2 == 1 { RED } else { CLEAR });
        let replaced = dir.join("replaced.uop");
        let write = added.write_copy(&added.prepare(GUMP_ID, &flipped, SizeChange::Refuse).unwrap(), &ClassicImageContainer::Uop(replaced.clone())).unwrap();
        assert_eq!(write, ImageWrite::Replaced);
        let replaced = ClassicImageFiles { kind: ClassicImageKind::Gump, container: ClassicImageContainer::Uop(replaced) };
        assert_eq!(replaced.read(GUMP_ID).unwrap().unwrap().rgba, flipped.into_raw());
        assert!(replaced.read(1).unwrap().is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
