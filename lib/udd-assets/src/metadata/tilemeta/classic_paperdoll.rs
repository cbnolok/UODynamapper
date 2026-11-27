//! `metadata/classic_paperdoll.bin`: the Classic client's tables its paperdoll gump reads besides tileart
//! (`uocf::classic::ui::paperdoll_gump`): the bodies `mobtypes.txt` flags with UOP frames, `Equipconv.def` and
//! `gump.def`, each row as the client uses it.
//!
//! Little-endian: the magic `TMCP`, `u32` `TILEMETA_FORMAT_VERSION`, then `u32` counts of the UOP-frame bodies, the
//! `Equipconv.def` rows, the `gump.def` rows and the `gump.def` targets; then the bodies (`u32`, ascending), the
//! [`TileMetaEquipConvRow`]s (ascending by body and equipment), the [`TileMetaGumpDefRow`]s (ascending by gump) and
//! the targets (`u32`), each row's `target_start..target_start + target_count`.

use bytemuck::{Pod, Zeroable};
use color_eyre::eyre;
use uocf::classic::ui::paperdoll_gump::{ClassicPaperdollTables, EquipConvGump, GumpDefTarget};

use super::TILEMETA_FORMAT_VERSION;
use crate::common::{ensure_manifest_version, read_pod_vec};

/// The Classic paperdoll's tables ([`TileMetaClassicPaperdoll`]). Required; its tables are empty when the Classic
/// folder the package was built from lacks the files.
pub const TILEMETA_CLASSIC_PAPERDOLL_ENTRY_PATH: &str = "metadata/classic_paperdoll.bin";
/// Magic of the `TILEMETA_CLASSIC_PAPERDOLL_ENTRY_PATH` header.
pub const TILEMETA_CLASSIC_PAPERDOLL_MAGIC: [u8; 4] = *b"TMCP";
/// The `u32` words after the magic: the version and four counts.
const HEADER_WORDS: usize = 5;
const U32_BYTES: usize = std::mem::size_of::<u32>();
const HEADER_SIZE: usize = TILEMETA_CLASSIC_PAPERDOLL_MAGIC.len() + HEADER_WORDS * U32_BYTES;

/// An `Equipconv.def` row: on `body`, `equipment` is drawn as `converted` with `gump`, and `hue` (which the paperdoll
/// does not read).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct TileMetaEquipConvRow {
    pub body: u32,
    pub equipment: u32,
    pub converted: i32,
    pub gump: i32,
    pub hue: u32,
}

const EQUIPCONV_ROW_SIZE: usize = 20;
const _: () = assert!(std::mem::size_of::<TileMetaEquipConvRow>() == EQUIPCONV_ROW_SIZE);

/// A `gump.def` row: `gump`, when missing, is drawn as one of its targets with `hue`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct TileMetaGumpDefRow {
    pub gump: u32,
    pub hue: u32,
    pub target_start: u32,
    pub target_count: u32,
}

const GUMP_DEF_ROW_SIZE: usize = 16;
const _: () = assert!(std::mem::size_of::<TileMetaGumpDefRow>() == GUMP_DEF_ROW_SIZE);

/// The tables of `TILEMETA_CLASSIC_PAPERDOLL_ENTRY_PATH`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TileMetaClassicPaperdoll {
    uop_frame_bodies: Vec<u32>,
    equip_conv: Vec<TileMetaEquipConvRow>,
    gump_def: Vec<TileMetaGumpDefRow>,
    gump_def_targets: Vec<u32>,
}

impl TileMetaClassicPaperdoll {
    /// The tables from the client's rows in file order: the last `Equipconv.def` row of a body and equipment, and
    /// the first `gump.def` row of a gump (`(gump, targets, hue)`), are the ones the client uses.
    pub fn from_client_rows(
        uop_frame_bodies: impl IntoIterator<Item = u32>,
        equip_conv: impl IntoIterator<Item = TileMetaEquipConvRow>,
        gump_def: impl IntoIterator<Item = (u32, Vec<u32>, u16)>,
    ) -> eyre::Result<Self> {
        let uop_frame_bodies: std::collections::BTreeSet<u32> = uop_frame_bodies.into_iter().collect();
        let equip_conv: std::collections::BTreeMap<(u32, u32), TileMetaEquipConvRow> =
            equip_conv.into_iter().map(|row| ((row.body, row.equipment), row)).collect();
        let mut first_gump_def = std::collections::BTreeMap::new();
        for (gump, targets, hue) in gump_def {
            first_gump_def.entry(gump).or_insert((targets, hue));
        }
        let mut tables = Self {
            uop_frame_bodies: uop_frame_bodies.into_iter().collect(),
            equip_conv: equip_conv.into_values().collect(),
            ..Self::default()
        };
        for (gump, (targets, hue)) in first_gump_def {
            eyre::ensure!(!targets.is_empty(), "gump.def row of gump {gump} has no target");
            tables.gump_def.push(TileMetaGumpDefRow {
                gump,
                hue: u32::from(hue),
                target_start: u32::try_from(tables.gump_def_targets.len())?,
                target_count: u32::try_from(targets.len())?,
            });
            tables.gump_def_targets.extend(targets);
        }
        Ok(tables)
    }

    /// The bodies with UOP frames, ascending.
    pub fn uop_frame_bodies(&self) -> &[u32] {
        &self.uop_frame_bodies
    }

    /// The `Equipconv.def` rows, ascending by body and equipment.
    pub fn equip_conv_rows(&self) -> &[TileMetaEquipConvRow] {
        &self.equip_conv
    }

    /// The `gump.def` rows, ascending by gump.
    pub fn gump_def_rows(&self) -> &[TileMetaGumpDefRow] {
        &self.gump_def
    }

    /// The targets of a `gump.def` row, as written.
    pub fn gump_def_targets(&self, row: &TileMetaGumpDefRow) -> &[u32] {
        let start = row.target_start as usize;
        &self.gump_def_targets[start..start + row.target_count as usize]
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let count = |len: usize| u32::try_from(len).expect("classic paperdoll table fits u32");
        let mut bytes = Vec::with_capacity(HEADER_SIZE);
        bytes.extend_from_slice(&TILEMETA_CLASSIC_PAPERDOLL_MAGIC);
        for word in [
            TILEMETA_FORMAT_VERSION,
            count(self.uop_frame_bodies.len()),
            count(self.equip_conv.len()),
            count(self.gump_def.len()),
            count(self.gump_def_targets.len()),
        ] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(bytemuck::cast_slice(&self.uop_frame_bodies));
        bytes.extend_from_slice(bytemuck::cast_slice(&self.equip_conv));
        bytes.extend_from_slice(bytemuck::cast_slice(&self.gump_def));
        bytes.extend_from_slice(bytemuck::cast_slice(&self.gump_def_targets));
        bytes
    }

    /// Reads the entry, refusing another magic or version, sizes that do not add up, unsorted rows or targets out
    /// of range.
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let path = TILEMETA_CLASSIC_PAPERDOLL_ENTRY_PATH;
        eyre::ensure!(bytes.len() >= HEADER_SIZE && bytes.starts_with(&TILEMETA_CLASSIC_PAPERDOLL_MAGIC),
            "{path}: no TMCP header; repack with the current udd-cli");
        let word = |index: usize| {
            let start = TILEMETA_CLASSIC_PAPERDOLL_MAGIC.len() + index * U32_BYTES;
            u32::from_le_bytes(bytes[start..start + U32_BYTES].try_into().expect("slice length checked")) as usize
        };
        ensure_manifest_version(path, word(0) as u32, TILEMETA_FORMAT_VERSION)?;
        let mut rest = &bytes[HEADER_SIZE..];
        let mut take = |len: usize, size: usize| -> eyre::Result<&[u8]> {
            let len = len.checked_mul(size).filter(|&len| len <= rest.len())
                .ok_or_else(|| eyre::eyre!("{path}: truncated"))?;
            let (taken, remaining) = rest.split_at(len);
            rest = remaining;
            Ok(taken)
        };
        let tables = Self {
            uop_frame_bodies: read_pod_vec(take(word(1), U32_BYTES)?, path)?,
            equip_conv: read_pod_vec(take(word(2), EQUIPCONV_ROW_SIZE)?, path)?,
            gump_def: read_pod_vec(take(word(3), GUMP_DEF_ROW_SIZE)?, path)?,
            gump_def_targets: read_pod_vec(take(word(4), U32_BYTES)?, path)?,
        };
        eyre::ensure!(rest.is_empty(), "{path}: {} trailing bytes", rest.len());
        eyre::ensure!(tables.uop_frame_bodies.windows(2).all(|pair| pair[0] < pair[1]), "{path}: bodies not sorted");
        let equip_conv_key = |row: &TileMetaEquipConvRow| (row.body, row.equipment);
        eyre::ensure!(
            tables.equip_conv.windows(2).all(|pair| equip_conv_key(&pair[0]) < equip_conv_key(&pair[1])),
            "{path}: Equipconv.def rows not sorted"
        );
        eyre::ensure!(
            tables.gump_def.windows(2).all(|pair| pair[0].gump < pair[1].gump),
            "{path}: gump.def rows not sorted"
        );
        eyre::ensure!(
            tables.gump_def.iter().all(|row| row.target_count > 0
                && (row.target_start as usize).checked_add(row.target_count as usize)
                    .is_some_and(|end| end <= tables.gump_def_targets.len())
                && u16::try_from(row.hue).is_ok()),
            "{path}: a gump.def row has no targets, targets out of range or a hue past u16"
        );
        Ok(tables)
    }
}

impl ClassicPaperdollTables for TileMetaClassicPaperdoll {
    fn uses_uop_frames(&self, body: u32) -> bool {
        self.uop_frame_bodies.binary_search(&body).is_ok()
    }

    fn equip_conv(&self, body: u32, equipment: u32) -> Option<EquipConvGump> {
        let index = self.equip_conv.binary_search_by_key(&(body, equipment), |row| (row.body, row.equipment)).ok()?;
        let row = &self.equip_conv[index];
        Some(EquipConvGump { converted: row.converted, gump: row.gump })
    }

    /// The row's first target: the client picks one of several at random.
    fn gump_def(&self, gump: u32) -> Option<GumpDefTarget> {
        let index = self.gump_def.binary_search_by_key(&gump, |row| row.gump).ok()?;
        let row = &self.gump_def[index];
        let hue = u16::try_from(row.hue).expect("hue checked on reading");
        Some(GumpDefTarget { target: self.gump_def_targets(row)[0], hue })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUMAN_FEMALE: u32 = 401;
    const GARGOYLE_MALE: u32 = 666;
    const GARGOYLE_FEMALE: u32 = 667;
    const EQUIPMENT: u32 = 1249;
    const MISSING_GUMP: u32 = 50_759;
    const FIRST_TARGET: u32 = 50_468;
    const SECOND_TARGET: u32 = 50_469;
    const HUE: u16 = 1_153;

    fn equip_conv_row(converted: i32, gump: i32) -> TileMetaEquipConvRow {
        TileMetaEquipConvRow { body: HUMAN_FEMALE, equipment: EQUIPMENT, converted, gump, hue: 0 }
    }

    fn tables() -> TileMetaClassicPaperdoll {
        TileMetaClassicPaperdoll::from_client_rows(
            [GARGOYLE_FEMALE, GARGOYLE_MALE, GARGOYLE_MALE],
            [equip_conv_row(1, 2), equip_conv_row(1250, 61_250)],
            [
                (MISSING_GUMP, vec![FIRST_TARGET, SECOND_TARGET], HUE),
                (MISSING_GUMP, vec![SECOND_TARGET], 0),
            ],
        )
        .unwrap()
    }

    /// The rows the client uses are kept and read back as it reads them.
    #[test]
    fn tables_round_trip_with_the_rows_the_client_uses() {
        let tables = tables();
        let bytes = tables.to_bytes();
        let read = TileMetaClassicPaperdoll::from_bytes(&bytes).unwrap();
        assert_eq!(read, tables);
        assert_eq!(read.uop_frame_bodies(), [GARGOYLE_MALE, GARGOYLE_FEMALE]);
        assert!(read.uses_uop_frames(GARGOYLE_MALE) && !read.uses_uop_frames(HUMAN_FEMALE));
        assert_eq!(read.equip_conv(HUMAN_FEMALE, EQUIPMENT), Some(EquipConvGump { converted: 1250, gump: 61_250 }));
        assert_eq!(read.equip_conv(GARGOYLE_MALE, EQUIPMENT), None);
        assert_eq!(read.gump_def(MISSING_GUMP), Some(GumpDefTarget { target: FIRST_TARGET, hue: HUE }));
        assert_eq!(read.gump_def_targets(&read.gump_def_rows()[0]), [FIRST_TARGET, SECOND_TARGET]);
    }

    #[test]
    fn damaged_tables_are_refused() {
        let bytes = tables().to_bytes();
        let truncated = TileMetaClassicPaperdoll::from_bytes(&bytes[..bytes.len() - 1]).unwrap_err();
        assert!(truncated.to_string().contains("truncated"), "{truncated}");
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(TileMetaClassicPaperdoll::from_bytes(&trailing).unwrap_err().to_string().contains("trailing"));
        assert!(TileMetaClassicPaperdoll::from_bytes(b"TMIL").unwrap_err().to_string().contains("repack"));
        assert!(TileMetaClassicPaperdoll::from_client_rows([], [], [(MISSING_GUMP, Vec::new(), 0)]).is_err());
    }
}
