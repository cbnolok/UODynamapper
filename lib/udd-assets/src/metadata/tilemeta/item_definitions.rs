//! `metadata/item_definitions.bin`: the `tileart.uop` records the Enhanced paperdoll reads, with
//! the properties and appearance filters it reads, so the lookup of `uocf::enhanced::paperdoll` runs on a package.
//!
//! The client looks a definition up by its entry name, `build/tileart/%08d.bin`; the tile id stored in the record is
//! only copied (`GetTileId`), and a few client records store another id than their name's. A record is keyed by its
//! entry id and keeps its stored tile id, which `GetAppearance` falls back to.
//!
//! Little-endian, records sorted by entry id:
//! - `u32` record count;
//! - per record: `u32` entry id, `u8` presence mask (bit i < 7: `STORED_PROPERTIES[i]` is present;
//!   `PRESENT_STORED_TILE_ID`: the stored tile id differs from the entry id), `u8` filter count, the stored tile id
//!   (`u32`) when it differs, one `u32` per present property in `STORED_PROPERTIES` order, then the filters;
//! - per filter: `u8` kind; `FILTER_KIND_BY_OWNER_BODY_TYPE` is followed by a `u16` row count and that many
//!   (`u32` body type, `u32` appearance) rows, `FILTER_KIND_BY_OWNER_GENDER` by a `u8` gender and a `u32` appearance.
//!
//! Animation filters (subtype 2) are not stored: the paperdoll skips them.

use color_eyre::eyre;
use uocf::enhanced::paperdoll::{AppearanceFilter, PaperdollDefinition, PaperdollDefinitions};
use uocf::enhanced::tileart::{PropertyKey, TaeAppearanceFilter, TaeBodyAppearance, TileArtEntry};

/// Which property list a stored value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyList {
    /// `prop_vector1`, which every definition getter of the 3D paperdoll reads.
    First,
    /// `prop_vector2`, which only the old-style gump paperdoll (`UOPaperdoll::BuildImage` 0x0050c790) reads, keys 11
    /// and 8 before `prop_vector1`.
    Second,
}

/// The stored values, in record order: bit i of the presence mask is entry i.
pub const STORED_PROPERTIES: [(PropertyList, PropertyKey); 7] = [
    (PropertyList::First, PropertyKey::Layer),
    (PropertyList::First, PropertyKey::Appearance),
    (PropertyList::First, PropertyKey::Race),
    (PropertyList::First, PropertyKey::Gender),
    (PropertyList::First, PropertyKey::Paperdoll),
    (PropertyList::Second, PropertyKey::Appearance),
    (PropertyList::Second, PropertyKey::Paperdoll),
];
/// Presence bit of a stored tile id other than the entry id.
const PRESENT_STORED_TILE_ID: u8 = 1 << STORED_PROPERTIES.len();
const FILTER_KIND_BY_OWNER_BODY_TYPE: u8 = 0;
const FILTER_KIND_BY_OWNER_GENDER: u8 = 1;
const U8_BYTES: usize = 1;
const U16_BYTES: usize = 2;
const U32_BYTES: usize = 4;

/// One stored filter: rows `start..start + len` of `TileMetaItemDefinitions::body_rows`, or a gender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredFilter {
    ByOwnerBodyType { start: usize, len: usize },
    ByOwnerGender { gender: u8, appearance: u32 },
}

/// One stored record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoredDefinition {
    /// The id of its entry name, by which the client finds it.
    entry_id: u32,
    /// The tile id stored in it.
    tile_id: u32,
    /// Presence bits of `values` (`PRESENT_STORED_TILE_ID` is not kept here).
    present: u8,
    values: [u32; STORED_PROPERTIES.len()],
    filter_start: usize,
    filter_count: usize,
}

/// The records of `metadata/item_definitions.bin`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TileMetaItemDefinitions {
    records: Vec<StoredDefinition>,
    filters: Vec<StoredFilter>,
    body_rows: Vec<TaeBodyAppearance>,
}

fn stored_index(list: PropertyList, key: PropertyKey) -> Option<usize> {
    STORED_PROPERTIES.iter().position(|&stored| stored == (list, key))
}

impl TileMetaItemDefinitions {
    /// The paperdoll's view of `entries`, (entry id, record) pairs sorted by entry id.
    pub fn from_tileart<'a>(entries: impl IntoIterator<Item = (u32, &'a TileArtEntry)>) -> eyre::Result<Self> {
        let mut definitions = Self::default();
        for (entry_id, entry) in entries {
            if let Some(last) = definitions.records.last() {
                eyre::ensure!(last.entry_id < entry_id, "tileart entries out of order: {entry_id} after {}", last.entry_id);
            }
            let mut record = StoredDefinition {
                entry_id,
                tile_id: entry.tile_id,
                present: 0,
                values: [0; STORED_PROPERTIES.len()],
                filter_start: definitions.filters.len(),
                filter_count: 0,
            };
            for (index, &(list, key)) in STORED_PROPERTIES.iter().enumerate() {
                let value = match list {
                    PropertyList::First => entry.property1(key),
                    PropertyList::Second => entry.property2(key),
                };
                if let Some(value) = value {
                    record.present |= 1 << index;
                    record.values[index] = value;
                }
            }
            for filter in &entry.appearance_vector {
                let stored = match filter {
                    TaeAppearanceFilter::ByOwnerBodyType(rows) => {
                        let start = definitions.body_rows.len();
                        definitions.body_rows.extend(rows.iter().cloned());
                        StoredFilter::ByOwnerBodyType { start, len: rows.len() }
                    }
                    TaeAppearanceFilter::ByOwnerGender { gender, appearance } => {
                        StoredFilter::ByOwnerGender { gender: *gender, appearance: *appearance }
                    }
                    TaeAppearanceFilter::AnimationByOwnerBodyType(_) => continue,
                };
                definitions.filters.push(stored);
                record.filter_count += 1;
            }
            definitions.records.push(record);
        }
        Ok(definitions)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The entry ids of the records, ascending.
    pub fn entry_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.records.iter().map(|record| record.entry_id)
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let count = u32::try_from(self.records.len()).map_err(|_| eyre::eyre!("too many item definitions"))?;
        bytes.extend_from_slice(&count.to_le_bytes());
        for record in &self.records {
            let renamed = record.tile_id != record.entry_id;
            bytes.extend_from_slice(&record.entry_id.to_le_bytes());
            bytes.push(record.present | if renamed { PRESENT_STORED_TILE_ID } else { 0 });
            let filter_count = u8::try_from(record.filter_count).map_err(|_| {
                eyre::eyre!("tileart entry {} has {} appearance filters", record.entry_id, record.filter_count)
            })?;
            bytes.push(filter_count);
            if renamed {
                bytes.extend_from_slice(&record.tile_id.to_le_bytes());
            }
            for (index, value) in record.values.iter().enumerate() {
                if record.present & (1 << index) != 0 {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            for filter in &self.filters[record.filter_start..record.filter_start + record.filter_count] {
                match *filter {
                    StoredFilter::ByOwnerBodyType { start, len } => {
                        bytes.push(FILTER_KIND_BY_OWNER_BODY_TYPE);
                        let row_count = u16::try_from(len)
                            .map_err(|_| eyre::eyre!("tileart entry {} has a filter of {len} rows", record.entry_id))?;
                        bytes.extend_from_slice(&row_count.to_le_bytes());
                        for row in &self.body_rows[start..start + len] {
                            bytes.extend_from_slice(&row.body_type.to_le_bytes());
                            bytes.extend_from_slice(&row.appearance.to_le_bytes());
                        }
                    }
                    StoredFilter::ByOwnerGender { gender, appearance } => {
                        bytes.push(FILTER_KIND_BY_OWNER_GENDER);
                        bytes.push(gender);
                        bytes.extend_from_slice(&appearance.to_le_bytes());
                    }
                }
            }
        }
        Ok(bytes)
    }

    pub fn from_bytes(bytes: &[u8], entry_path: &str) -> eyre::Result<Self> {
        let mut reader = Reader { bytes, position: 0, entry_path };
        let count = reader.u32()? as usize;
        let mut definitions = Self::default();
        for _ in 0..count {
            let entry_id = reader.u32()?;
            if let Some(last) = definitions.records.last() {
                eyre::ensure!(last.entry_id < entry_id, "{entry_path}: entry {entry_id} after {}", last.entry_id);
            }
            let flags = reader.u8()?;
            let present = flags & !PRESENT_STORED_TILE_ID;
            let filter_count = usize::from(reader.u8()?);
            let tile_id = if flags & PRESENT_STORED_TILE_ID != 0 { reader.u32()? } else { entry_id };
            let mut values = [0; STORED_PROPERTIES.len()];
            for (index, value) in values.iter_mut().enumerate() {
                if present & (1 << index) != 0 {
                    *value = reader.u32()?;
                }
            }
            let filter_start = definitions.filters.len();
            for _ in 0..filter_count {
                let filter = match reader.u8()? {
                    FILTER_KIND_BY_OWNER_BODY_TYPE => {
                        let len = usize::from(reader.u16()?);
                        let start = definitions.body_rows.len();
                        for _ in 0..len {
                            let body_type = reader.u32()?;
                            let appearance = reader.u32()?;
                            definitions.body_rows.push(TaeBodyAppearance { body_type, appearance });
                        }
                        StoredFilter::ByOwnerBodyType { start, len }
                    }
                    FILTER_KIND_BY_OWNER_GENDER => {
                        let gender = reader.u8()?;
                        StoredFilter::ByOwnerGender { gender, appearance: reader.u32()? }
                    }
                    kind => eyre::bail!("{entry_path}: entry {entry_id} has a filter of unknown kind {kind}"),
                };
                definitions.filters.push(filter);
            }
            definitions.records.push(StoredDefinition { entry_id, tile_id, present, values, filter_start, filter_count });
        }
        eyre::ensure!(reader.position == bytes.len(), "{entry_path}: {} trailing bytes", bytes.len() - reader.position);
        Ok(definitions)
    }

    /// The record of entry `build/tileart/<entry_id>.bin`: `UOTileArt::GetObjectDefinition`.
    pub fn definition(&self, entry_id: u32) -> Option<TileMetaItemDefinition<'_>> {
        let index = self.records.binary_search_by_key(&entry_id, |record| record.entry_id).ok()?;
        Some(TileMetaItemDefinition { definitions: self, record: &self.records[index] })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    entry_path: &'a str,
}

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> eyre::Result<[u8; N]> {
        let end = self.position + N;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| eyre::eyre!("{}: truncated at byte {}", self.entry_path, self.position))?;
        self.position = end;
        Ok(slice.try_into().expect("slice of N bytes"))
    }

    fn u8(&mut self) -> eyre::Result<u8> {
        Ok(self.take::<U8_BYTES>()?[0])
    }

    fn u16(&mut self) -> eyre::Result<u16> {
        Ok(u16::from_le_bytes(self.take::<U16_BYTES>()?))
    }

    fn u32(&mut self) -> eyre::Result<u32> {
        Ok(u32::from_le_bytes(self.take::<U32_BYTES>()?))
    }
}

/// A stored record.
#[derive(Debug, Clone, Copy)]
pub struct TileMetaItemDefinition<'a> {
    definitions: &'a TileMetaItemDefinitions,
    record: &'a StoredDefinition,
}

impl<'a> TileMetaItemDefinition<'a> {
    /// The id of its entry name.
    pub fn entry_id(self) -> u32 {
        self.record.entry_id
    }

    /// The value of `key` in `list`, if it is stored and present.
    pub fn value(self, list: PropertyList, key: PropertyKey) -> Option<u32> {
        let index = stored_index(list, key)?;
        (self.record.present & (1 << index) != 0).then_some(self.record.values[index])
    }

    /// The value of `key` in `prop_vector2`, for keys 8 and 11.
    pub fn property2(self, key: PropertyKey) -> Option<u32> {
        self.value(PropertyList::Second, key)
    }
}

impl<'a> PaperdollDefinition<'a> for TileMetaItemDefinition<'a> {
    /// The stored tile id (`GetTileId`).
    fn tile_id(self) -> u32 {
        self.record.tile_id
    }

    fn property(self, key: PropertyKey) -> Option<u32> {
        self.value(PropertyList::First, key)
    }

    fn appearance_filters(self) -> impl Iterator<Item = AppearanceFilter<'a>> + 'a {
        let definitions = self.definitions;
        let filters = &definitions.filters[self.record.filter_start..self.record.filter_start + self.record.filter_count];
        filters.iter().map(move |filter| match *filter {
            StoredFilter::ByOwnerBodyType { start, len } => {
                AppearanceFilter::ByOwnerBodyType(&definitions.body_rows[start..start + len])
            }
            StoredFilter::ByOwnerGender { gender, appearance } => AppearanceFilter::ByOwnerGender { gender, appearance },
        })
    }
}

impl<'a> PaperdollDefinitions<'a> for &'a TileMetaItemDefinitions {
    type Definition = TileMetaItemDefinition<'a>;

    fn get(self, entry_id: u32) -> Option<TileMetaItemDefinition<'a>> {
        self.definition(entry_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::mobile_body::MOBILE_DEFINITION_BASE;
    use uocf::enhanced::paperdoll::SortedTileArt;
    use uocf::enhanced::tileart::TaeProp;

    const ROBE: u32 = 7939;
    const SHIRT: u32 = 5399;
    const HUMAN_MALE: u32 = 400;
    const ROBE_LAYER: u32 = 22;
    const NAMED_APPEARANCE: u32 = 77;
    const BODY_APPEARANCE: u32 = 498;
    const GENDER_APPEARANCE: u32 = 497;
    const OLD_GUMP: u32 = 1234;
    const FEMALE: u8 = 1;
    const ELF: u32 = 2;
    /// An entry whose record stores another tile id, as `build/tileart/00042091.bin` stores 41516.
    const RENAMED_ENTRY: u32 = 42091;
    const RENAMED_STORED_ID: u32 = 41516;

    fn prop(key: PropertyKey, val: u32) -> TaeProp {
        TaeProp { id: key as u8, val }
    }

    fn records() -> Vec<TileArtEntry> {
        vec![
            TileArtEntry {
                tile_id: SHIRT,
                appearance_vector: vec![
                    TaeAppearanceFilter::AnimationByOwnerBodyType(vec![TaeBodyAppearance { body_type: 1, appearance: 2 }]),
                    TaeAppearanceFilter::ByOwnerGender { gender: FEMALE, appearance: GENDER_APPEARANCE },
                ],
                ..TileArtEntry::default()
            },
            TileArtEntry {
                tile_id: ROBE,
                prop_vector1: vec![prop(PropertyKey::Layer, ROBE_LAYER), prop(PropertyKey::Paperdoll, NAMED_APPEARANCE)],
                prop_vector2: vec![prop(PropertyKey::Paperdoll, OLD_GUMP), prop(PropertyKey::Weight, 1)],
                appearance_vector: vec![TaeAppearanceFilter::ByOwnerBodyType(vec![TaeBodyAppearance {
                    body_type: MOBILE_DEFINITION_BASE + HUMAN_MALE,
                    appearance: BODY_APPEARANCE,
                }])],
                ..TileArtEntry::default()
            },
            TileArtEntry {
                tile_id: MOBILE_DEFINITION_BASE + NAMED_APPEARANCE,
                prop_vector1: vec![prop(PropertyKey::Appearance, NAMED_APPEARANCE), prop(PropertyKey::Race, ELF)],
                ..TileArtEntry::default()
            },
            TileArtEntry { tile_id: MOBILE_DEFINITION_BASE + HUMAN_MALE, ..TileArtEntry::default() },
        ]
    }

    /// The records keyed by their stored tile id, as every entry but the renamed ones is.
    fn keyed(records: &[TileArtEntry]) -> Vec<(u32, &TileArtEntry)> {
        records.iter().map(|record| (record.tile_id, record)).collect()
    }

    /// Each record reads back as the paperdoll reads its `tileart.uop` record, and the bytes are stable.
    #[test]
    fn definitions_round_trip_as_the_paperdoll_reads_them() {
        let source = records();
        let definitions = TileMetaItemDefinitions::from_tileart(keyed(&source)).unwrap();
        let bytes = definitions.to_bytes().unwrap();
        let read = TileMetaItemDefinitions::from_bytes(&bytes, "test").unwrap();
        assert_eq!(read, definitions);
        assert_eq!(read.to_bytes().unwrap(), bytes);
        let tileart = SortedTileArt(&source);
        for entry in &source {
            let stored = (&read).get(entry.tile_id).unwrap();
            let original = tileart.get(entry.tile_id).unwrap();
            assert_eq!(stored.tile_id(), original.tile_id);
            for &(list, key) in &STORED_PROPERTIES {
                let expected = match list {
                    PropertyList::First => original.property1(key),
                    PropertyList::Second => original.property2(key),
                };
                assert_eq!(stored.value(list, key), expected, "{} {list:?} {key:?}", entry.tile_id);
            }
            let filters: Vec<_> = stored.appearance_filters().collect();
            let expected: Vec<_> = original
                .appearance_filters()
                .filter(|filter| *filter != AppearanceFilter::Animation)
                .collect();
            assert_eq!(filters, expected, "{}", entry.tile_id);
        }
        assert!(read.definition(OLD_GUMP).is_none());
        assert_eq!(read.definition(ROBE).unwrap().property(PropertyKey::Weight), None);
    }

    /// A record is found by its entry name and keeps the tile id it stores.
    #[test]
    fn a_renamed_entry_is_found_by_its_name_and_keeps_its_stored_id() {
        let renamed = TileArtEntry { tile_id: RENAMED_STORED_ID, ..TileArtEntry::default() };
        let original = TileArtEntry { tile_id: RENAMED_STORED_ID, ..TileArtEntry::default() };
        let definitions = TileMetaItemDefinitions::from_tileart([
            (RENAMED_STORED_ID, &original),
            (RENAMED_ENTRY, &renamed),
        ])
        .unwrap();
        let read = TileMetaItemDefinitions::from_bytes(&definitions.to_bytes().unwrap(), "test").unwrap();
        let found = read.definition(RENAMED_ENTRY).unwrap();
        assert_eq!((found.entry_id(), found.tile_id()), (RENAMED_ENTRY, RENAMED_STORED_ID));
        assert_eq!(read.definition(RENAMED_STORED_ID).unwrap().entry_id(), RENAMED_STORED_ID);
    }

    /// Packed `tilemeta.uddp` and `paperdoll_ec.uddp` find, for a man and a woman, the layer and frame `tileart.uop`
    /// and `Paperdoll.uop` find for every item with a drawn layer, and the same body frames.
    #[test]
    #[ignore = "needs packed tilemeta.uddp and paperdoll_ec.uddp and their Enhanced Client: set UDD_TILEMETA, \
                UDD_PAPERDOLL_EC and UO_EC_CLIENT_DIR, and run with --ignored"]
    fn a_packed_tilemeta_finds_what_tileart_finds() {
        use uocf::enhanced::paperdoll::{
            body_frame, item_frame, item_layer, FrameRef, FrameSets, NamedTileArt, Wearer, ENTRY_ID_SPAN,
            PAPERDOLL_UOP_NAME, SKIPPED_LAYERS,
        };
        use uocf::enhanced::tileart::TILEART_UOP_NAME;
        use uocf::uop_container::package::UopPackage;
        use uocf::utils::path::find_file_ignoring_case;
        const TILEMETA_ENV: &str = "UDD_TILEMETA";
        const PAPERDOLL_EC_ENV: &str = "UDD_PAPERDOLL_EC";
        const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
        const HUMAN_FEMALE: u32 = 401;
        let tilemeta = crate::TileMetaPackage::load(std::env::var_os(TILEMETA_ENV).expect("set UDD_TILEMETA")).unwrap();
        let dir = std::path::PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("set UO_EC_CLIENT_DIR"));
        let named = NamedTileArt::read(&UopPackage::load(find_file_ignoring_case(&dir, TILEART_UOP_NAME).unwrap()).unwrap());
        let sets = FrameSets::load(&find_file_ignoring_case(&dir, PAPERDOLL_UOP_NAME).unwrap()).unwrap();
        let frames =
            crate::PaperdollEcPackage::load(std::env::var_os(PAPERDOLL_EC_ENV).expect("set UDD_PAPERDOLL_EC")).unwrap();
        let packed = tilemeta.item_definitions();
        // The frames compared by set and number, as the two sides keep different frame records.
        let client_key = |frame: FrameRef| (frame.set, frame.entry.frame_number);
        let package_key = |frame: FrameRef<crate::paperdoll_ec::PaperdollEcFrameRecord>| (frame.set, frame.entry.frame_number);
        let mut pairs = 0usize;
        for body in [HUMAN_MALE, HUMAN_FEMALE] {
            let (from_tileart, from_package) = (Wearer::new(&named, body), Wearer::new(packed, body));
            assert_eq!(
                body_frame(&sets, &from_tileart).map(|(frame, fell_back)| (client_key(frame), fell_back)),
                body_frame(&frames, &from_package).map(|(frame, fell_back)| (package_key(frame), fell_back)),
                "body {body}"
            );
            for item_id in 0..ENTRY_ID_SPAN {
                let fallback = tilemeta.item_tile(item_id).map(|tile| tile.quality).filter(|&quality| quality != 0);
                let layer = item_layer(&named, item_id, fallback);
                assert_eq!(layer, item_layer(packed, item_id, fallback), "item {item_id}");
                if layer.is_none_or(|layer| SKIPPED_LAYERS.contains(&layer)) {
                    continue;
                }
                pairs += 1;
                assert_eq!(
                    item_frame(&sets, &named, &from_tileart, item_id).map(client_key),
                    item_frame(&frames, packed, &from_package, item_id).map(package_key),
                    "body {body} item {item_id}"
                );
            }
        }
        // EC 2024: 2,316 items with a drawn layer, for each body.
        assert!(pairs > 0);
    }

    #[test]
    fn damaged_definitions_are_refused() {
        let source = records();
        let bytes = TileMetaItemDefinitions::from_tileart(keyed(&source)).unwrap().to_bytes().unwrap();
        let truncated = TileMetaItemDefinitions::from_bytes(&bytes[..bytes.len() - 1], "test").unwrap_err();
        assert!(truncated.to_string().contains("truncated"), "{truncated}");
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(TileMetaItemDefinitions::from_bytes(&trailing, "test").unwrap_err().to_string().contains("trailing"));
        let mut unsorted = keyed(&source);
        unsorted.swap(0, 1);
        assert!(TileMetaItemDefinitions::from_tileart(unsorted).is_err());
    }
}
