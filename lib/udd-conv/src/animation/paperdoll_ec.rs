//! Packs the Enhanced Client's paperdoll frames (`Paperdoll.uop`) and the hue tables of `Hues.uop` into
//! `paperdoll_ec.uddp` (layout in `udd_assets::paperdoll_ec`).

use std::fmt;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::info;
use udd_assets::paperdoll_ec::{
    encode_frames_manifest, encode_hue_tables, frame_entry_path, frame_hue_mask_entry_path, frame_hue_mode,
    PaperdollEcFrameRecord, PaperdollEcSet, FRAMES_ENTRY_PATH, FRAME_HUE_MASKED, HUE_TABLES_ENTRY_PATH,
};
use udd_container::{AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};
use uocf::enhanced::hues::{EcHueTables, ShadowPaletteStats, HUES_UOP_NAME};
use uocf::enhanced::paperdoll::{FrameSets, GENDER_SET_COUNT, PAPERDOLL_UOP_NAME};

use crate::package_progress::build_and_write_package;
use crate::page_compression::LOSSLESS_PAGE_ZSTD_LEVEL;
use crate::source_paths::find_first_existing_file;

pub struct PaperdollEcPackOptions {
    pub compression: CompressionFlag,
}

impl Default for PaperdollEcPackOptions {
    /// Zstd 19, as lossless atlas pages: EC 2024 packs to 8.6 MB, against 10.5 MB at the default level.
    fn default() -> Self {
        Self { compression: CompressionFlag::ZstdNoDictLevel(LOSSLESS_PAGE_ZSTD_LEVEL) }
    }
}

/// What the packer wrote.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PaperdollEcPackSummary {
    /// Frames per set (male, female).
    pub frames: [usize; GENDER_SET_COUNT],
    /// Frames with a stored hue mask.
    pub masked: usize,
    /// Frames without pixels.
    pub empty: usize,
    /// Frame table entries the paperdoll cannot find: another action than their set's, or a repeated number.
    pub skipped: usize,
    pub hue_tables: usize,
    /// Bytes of the frame and mask entries before compression.
    pub pixel_bytes: usize,
}

impl fmt::Display for PaperdollEcPackSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} male and {} female frames ({} with a hue mask, {} empty, {} unreachable skipped), {} hue tables, {} \
             pixel bytes before compression",
            self.frames[0], self.frames[1], self.masked, self.empty, self.skipped, self.hue_tables, self.pixel_bytes
        )
    }
}

/// Packs `Paperdoll.uop` and `Hues.uop`, found in the first of `source_dirs` that has them.
pub fn pack_paperdoll_ec(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &PaperdollEcPackOptions,
) -> eyre::Result<PaperdollEcPackSummary> {
    let paperdoll_path = find_first_existing_file(source_dirs, &[PAPERDOLL_UOP_NAME])
        .ok_or_else(|| eyre::eyre!("missing {PAPERDOLL_UOP_NAME}"))?;
    let hues_path =
        find_first_existing_file(source_dirs, &[HUES_UOP_NAME]).ok_or_else(|| eyre::eyre!("missing {HUES_UOP_NAME}"))?;
    info!("paperdoll_ec: frames from '{}', hue tables from '{}'.", paperdoll_path.display(), hues_path.display());
    let sets = FrameSets::load(&paperdoll_path)?;
    let hues = EcHueTables::load(&hues_path).wrap_err_with(|| format!("cannot read {}", hues_path.display()))?;
    let (builder, summary) = build_paperdoll_ec(&sets, &hues, options)?;
    build_and_write_package(builder, out_file)?;
    Ok(summary)
}

/// The package of decoded frame sets and hue tables.
pub fn build_paperdoll_ec(
    sets: &FrameSets,
    hues: &EcHueTables,
    options: &PaperdollEcPackOptions,
) -> eyre::Result<(UddpBuilder, PaperdollEcPackSummary)> {
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    let mut summary = PaperdollEcPackSummary { hue_tables: hues.tables.len(), ..Default::default() };
    let mut set_records = Vec::with_capacity(GENDER_SET_COUNT);
    let mut frames = Vec::new();
    for set_index in 0..GENDER_SET_COUNT {
        let set = sets.set(set_index);
        set_records.push(PaperdollEcSet { bounds: set.bounds, shadow: ShadowPaletteStats::from_palette(&set.colours) });
        let set_number = u8::try_from(set_index)?;
        let action = u16::from(set_number);
        let first_frame = frames.len();
        for entry in &set.frames {
            // `UOAnimations::FindFrameEntry` looks only at the set's own action; `find_in_set` takes the first of a
            // repeated number.
            let repeated = frames[first_frame..]
                .iter()
                .any(|frame: &PaperdollEcFrameRecord| frame.frame_number == entry.frame_number);
            if entry.action != action || repeated {
                summary.skipped += 1;
                continue;
            }
            let decoded = set
                .decode_frame(entry)
                .wrap_err_with(|| format!("set {set_index} frame {}", entry.frame_number))?;
            let record = PaperdollEcFrameRecord {
                set: set_number,
                hue_mode: frame_hue_mode(&decoded.data, &decoded.hue_mask),
                frame_number: entry.frame_number,
                x: entry.init_coords_x,
                y: entry.init_coords_y,
                width: decoded.width,
                height: decoded.height,
            };
            summary.frames[set_index] += 1;
            if record.pixel_count() == 0 {
                summary.empty += 1;
                frames.push(record);
                continue;
            }
            if record.hue_mode == FRAME_HUE_MASKED {
                summary.masked += 1;
                summary.pixel_bytes += decoded.hue_mask.len();
                add_frame_entry(&mut builder, options, frame_hue_mask_entry_path(set_number, record.frame_number), &record, decoded.hue_mask)?;
            }
            summary.pixel_bytes += decoded.data.len();
            add_frame_entry(&mut builder, options, frame_entry_path(set_number, record.frame_number), &record, decoded.data)?;
            frames.push(record);
        }
    }
    frames.sort_by_key(|frame| (frame.set, frame.frame_number));
    for (path, bytes) in [
        (FRAMES_ENTRY_PATH, encode_frames_manifest(&set_records, &frames)?),
        (HUE_TABLES_ENTRY_PATH, encode_hue_tables(hues)?),
    ] {
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: options.compression,
                width: 0,
                height: 0,
                virtual_path: Some(path),
                path_hash64: None,
                id: None,
                data: &bytes,
            })
            .wrap_err_with(|| format!("add {path}"))?;
    }
    Ok((builder, summary))
}

fn add_frame_entry(
    builder: &mut UddpBuilder,
    options: &PaperdollEcPackOptions,
    path: String,
    record: &PaperdollEcFrameRecord,
    data: Vec<u8>,
) -> eyre::Result<()> {
    builder
        .add_owned_file(AddOwnedFileRequest {
            data_type: DataType::Paperdoll as u8,
            compression: options.compression,
            width: u32::from(record.width),
            height: u32::from(record.height),
            virtual_path: Some(path.clone()),
            path_hash64: None,
            id: None,
            data,
        })
        .wrap_err_with(|| format!("add {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::paperdoll_ec::{PaperdollEcPackage, FRAME_HUE_ALL, FRAME_HUE_NONE};
    use udd_container::UddpReader;
    use uocf::enhanced::animationframe::{AmouFile, AnimationFrame, IndexedFrame, COVERAGE_SOLID, COVERAGE_TRANSPARENT};
    use uocf::enhanced::hues::{EcHueTable, HUE_PARTIAL_FLAG, HUE_TABLE_ENTRIES, SHADOW_HUE};
    use uocf::enhanced::paperdoll::{PaperdollFrames, FEMALE_SET, MALE_SET};

    const BODY_FRAME: u16 = 400;
    const FEMALE_BODY_FRAME: u16 = 401;
    const HUED_FRAME: u16 = 5;
    const KEPT_FRAME: u16 = 7;
    /// A frame stored under the female set's action in the male file: the paperdoll never finds it.
    const OTHER_ACTION_FRAME: u16 = 9;
    const HUE_NUMBER: u16 = 38;
    /// A hue without a table in [`hue_tables`].
    const MISSING_HUE: u16 = 77;
    const ANCHOR_X: i16 = 2;
    const ANCHOR_Y: i16 = 1;
    const EDGE_COVERAGE: u8 = 9;
    const RGBA_CHANNELS: usize = 4;
    /// Palette `[b0, b1, b2, keep_colour]`: index 1 and 3 take the hue, index 2 keeps its colour.
    const PALETTE: [[u8; RGBA_CHANNELS]; 4] = [[0, 0, 0, 0], [40, 80, 120, 0], [200, 100, 50, 1], [90, 91, 92, 0]];
    /// The server hues each frame is decoded with: none, a table, the partial form of the same table, a hue
    /// without a table and the shadow.
    const HUES: [u16; 5] = [0, HUE_NUMBER, HUE_NUMBER | HUE_PARTIAL_FLAG, MISSING_HUE, SHADOW_HUE];

    fn indexed(indices: [u8; 6], coverage: [u8; 6]) -> IndexedFrame {
        IndexedFrame {
            width: 3,
            height: 2,
            center_x: ANCHOR_X,
            center_y: ANCHOR_Y,
            indices: indices.to_vec(),
            coverage: coverage.to_vec(),
        }
    }

    fn frame_sets() -> FrameSets {
        let solid = COVERAGE_SOLID;
        let clear = COVERAGE_TRANSPARENT;
        let masked = indexed([1, 2, 0, 3, 3, 2], [solid, EDGE_COVERAGE, clear, solid, solid, solid]);
        let hued = indexed([1, 3, 1, 0, 3, 1], [EDGE_COVERAGE, solid, solid, clear, solid, solid]);
        let kept = indexed([2, 2, 0, 0, 2, 2], [solid, solid, clear, clear, EDGE_COVERAGE, solid]);
        let male_action = MALE_SET as u16;
        let female_action = FEMALE_SET as u16;
        let male = AmouFile::from_indexed_frames(
            u32::from(BODY_FRAME),
            PALETTE.to_vec(),
            &[
                (male_action, BODY_FRAME, masked.clone()),
                (male_action, HUED_FRAME, hued.clone()),
                (male_action, KEPT_FRAME, kept),
                (female_action, OTHER_ACTION_FRAME, hued.clone()),
            ],
        )
        .unwrap();
        let female = AmouFile::from_indexed_frames(
            u32::from(FEMALE_BODY_FRAME),
            PALETTE.to_vec(),
            &[(female_action, FEMALE_BODY_FRAME, masked), (female_action, HUED_FRAME, hued)],
        )
        .unwrap();
        let load = |file: AmouFile| AnimationFrame::load(&file.to_bytes().unwrap()).unwrap();
        FrameSets::from_sets([load(male), load(female)])
    }

    fn hue_tables() -> EcHueTables {
        let mut table = EcHueTable { entries: [[0; RGBA_CHANNELS]; HUE_TABLE_ENTRIES] };
        for (index, entry) in table.entries.iter_mut().enumerate() {
            let key = index as u8;
            *entry = [u8::MAX - key, key / 2, key, u8::MAX];
        }
        let mut tables = EcHueTables::default();
        tables.tables.insert(HUE_NUMBER, table);
        tables
    }

    fn pack(sets: &FrameSets, hues: &EcHueTables) -> (PaperdollEcPackage, PaperdollEcPackSummary) {
        let (mut builder, summary) = build_paperdoll_ec(sets, hues, &PaperdollEcPackOptions::default()).unwrap();
        let package = PaperdollEcPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap()).unwrap();
        (package, summary)
    }

    /// Every frame the paperdoll finds in `sets` is found in `package`, at the same place, with the same pixels under
    /// every hue in `hues`; the count of frames compared.
    fn assert_same_frames(sets: &FrameSets, package: &PaperdollEcPackage, hues: &EcHueTables, hue_list: &[u16]) -> usize {
        let mut compared = 0;
        for set in [MALE_SET, FEMALE_SET] {
            for entry in &sets.set(set).frames {
                let Some(source) = sets.find_in_set(set, entry.frame_number) else { continue };
                let packed = package
                    .find_in_set(set, entry.frame_number)
                    .unwrap_or_else(|| panic!("set {set} frame {} is not packed", entry.frame_number));
                for &hue in hue_list {
                    let expected =
                        sets.decode(uocf::enhanced::paperdoll::FrameRef { set, entry: source }, hue, hues).unwrap();
                    let found = package.decode(uocf::enhanced::paperdoll::FrameRef { set, entry: packed }, hue).unwrap();
                    assert!(expected == found, "set {set} frame {} hue {hue:#x}", entry.frame_number);
                }
                compared += 1;
            }
        }
        compared
    }

    #[test]
    fn packed_frames_decode_as_the_client_files_do() {
        let sets = frame_sets();
        let hues = hue_tables();
        let (package, summary) = pack(&sets, &hues);
        assert_eq!(assert_same_frames(&sets, &package, &hues, &HUES), 5);
        assert_eq!(summary.frames, [3, 2]);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.masked, 2);
        assert_eq!(summary.hue_tables, 1);
        assert!(package.find_in_set(MALE_SET, OTHER_ACTION_FRAME).is_none());
        let mode = |set, number| package.find_in_set(set, number).unwrap().hue_mode;
        assert_eq!(mode(MALE_SET, BODY_FRAME), FRAME_HUE_MASKED);
        assert_eq!(mode(MALE_SET, HUED_FRAME), FRAME_HUE_ALL);
        assert_eq!(mode(MALE_SET, KEPT_FRAME), FRAME_HUE_NONE);
        assert_eq!(package.hue_tables(), &hues);
        assert_eq!(package.sets()[MALE_SET].bounds, sets.set(MALE_SET).bounds);
    }

    /// A real `Paperdoll.uop` packs losslessly: every frame the paperdoll finds decodes to the same pixels under the
    /// listed hues, with the tables of the real `Hues.uop`.
    #[test]
    #[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
    fn a_real_client_packs_losslessly() {
        /// The shirt and cloak hues of the pixel checks, another table, a partial hue and the shadow.
        const REAL_HUES: [u16; 6] = [0, 3, 38, 1153, 38 | HUE_PARTIAL_FLAG, SHADOW_HUE];
        let dir = PathBuf::from(std::env::var_os("UO_EC_CLIENT_DIR").expect("set UO_EC_CLIENT_DIR"));
        let source_dirs = [dir];
        let paperdoll = find_first_existing_file(&source_dirs, &[PAPERDOLL_UOP_NAME]).unwrap();
        let hues_path = find_first_existing_file(&source_dirs, &[HUES_UOP_NAME]).unwrap();
        let sets = FrameSets::load(&paperdoll).unwrap();
        let hues = EcHueTables::load(&hues_path).unwrap();
        let (package, summary) = pack(&sets, &hues);
        let compared = assert_same_frames(&sets, &package, &hues, &REAL_HUES);
        assert_eq!(compared, summary.frames.iter().sum::<usize>());
        log::info!("{summary}; {compared} frames compared");
    }
}
