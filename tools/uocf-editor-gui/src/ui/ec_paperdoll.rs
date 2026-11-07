//! The Enhanced Client's paperdoll images: the body and worn-item frames of `Paperdoll.uop`, found and ordered as
//! `ECPaperdollTexture::Build` (UOSA 0x0050b340) does. The lookup is `uocf::enhanced::paperdoll`, shared with
//! dynamapper; this module reads the definitions from the loaded `tileart.uop` and holds the rules shipped with
//! dynamapper.

use uocf::enhanced::paperdoll::{item_frame, item_layer, PaperdollDefinitions, Wearer, MALE_SET, SLOT_NAMES};
pub(crate) use uocf::enhanced::paperdoll::{build, compose, hue_tables, FrameSets, WearableRules, WornItem, PAPERDOLL_UOP_NAME};
use uocf::enhanced::tileart::{tileart_entry_hash, TileArtEntry};

use crate::app::TileArtFileEntry;

/// `wearables.xml` of `MainMisc.uop`, as dynamapper reads it.
const WEARABLE_RULES_KDL: &str =
    include_str!("../../../../dynamapper/assets/runtime_specs/paperdoll/WearableRules.kdl");

/// `tileart.uop` records by the id of their entry name: `UOTileArt::GetObjectDefinition`.
#[derive(Clone, Copy)]
pub(crate) struct Definitions<'a>(pub &'a [TileArtFileEntry]);

impl<'a> PaperdollDefinitions<'a> for Definitions<'a> {
    type Definition = &'a TileArtEntry;

    /// The record of entry `build/tileart/<entry_id>.bin`; the entries are sorted by their stored tile id, which is
    /// their name's but in a few client records (43957..=43963 store 43956, 42091 stores 41516).
    fn get(self, entry_id: u32) -> Option<&'a TileArtEntry> {
        let hash = tileart_entry_hash(entry_id);
        let start = self.0.partition_point(|file| file.entry.tile_id < entry_id);
        let found = self.0[start..]
            .iter()
            .take_while(|file| file.entry.tile_id == entry_id)
            .find(|file| file.filename_hash == hash);
        // A record that stores another id than its name's is only found by looking through all of them.
        found.or_else(|| self.0.iter().find(|file| file.filename_hash == hash)).map(|file| &file.entry)
    }
}

/// The rules shipped with dynamapper, decoded once.
pub(crate) fn embedded_rules() -> Result<&'static WearableRules, &'static str> {
    static RULES: std::sync::OnceLock<Result<WearableRules, String>> = std::sync::OnceLock::new();
    RULES
        .get_or_init(|| WearableRules::decode(WEARABLE_RULES_KDL).map_err(|error| format!("{error:#}")))
        .as_ref()
        .map_err(String::as_str)
}

/// The Derived column of an item row: its layer, and its frame once the frame sets are loaded.
pub(crate) fn item_detail(
    sets: Option<&FrameSets>,
    definitions: Definitions,
    body: u32,
    item_id: u32,
    tiledata_layer: Option<u8>,
) -> String {
    let layer = match item_layer(definitions, item_id, tiledata_layer) {
        Some(layer) => format!("layer={layer} {}", SLOT_NAMES[layer]),
        None => "no layer".to_string(),
    };
    let Some(sets) = sets else { return format!("{layer} (Render to find the frame)") };
    match item_frame(sets, definitions, &Wearer::new(definitions, body), item_id) {
        Ok(frame) => format!(
            "{layer} frame={} {}",
            frame.entry.frame_number,
            if frame.set == MALE_SET { "male" } else { "female" }
        ),
        Err(miss) => format!("{layer}: {}", miss.describe()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::mobile_body::MOBILE_DEFINITION_BASE;
    use uocf::enhanced::paperdoll::slot_layer;
    use uocf::enhanced::tileart::{PropertyKey, TaeProp};

    const LAYERED_ITEM: u32 = 5399;
    const TORSO_LAYER: u32 = 5;
    /// Entries that store another tile id than their name's, as `build/tileart/00042091.bin` stores 41516.
    const RENAMED_ENTRY: u32 = 42091;
    const RENAMED_STORED_ID: u32 = 41516;
    const STORED_ID_LAYER: u32 = 2;

    /// The rules embedded from dynamapper decode; their order is tested in `uocf::enhanced::paperdoll`.
    #[test]
    fn the_embedded_rules_decode() {
        let rules = embedded_rules().unwrap();
        assert!(rules.base_priority(slot_layer("dress").unwrap()) > rules.base_priority(slot_layer("torso").unwrap()));
    }

    /// The editor's loaded records answer the shared lookup, without frame sets for now.
    #[test]
    fn loaded_records_answer_the_lookup() {
        let mut files: Vec<TileArtFileEntry> = [MOBILE_DEFINITION_BASE + 400, LAYERED_ITEM]
            .into_iter()
            .map(|tile_id| TileArtFileEntry {
                filename_hash: tileart_entry_hash(tile_id),
                entry: TileArtEntry {
                    tile_id,
                    prop_vector1: if tile_id == LAYERED_ITEM {
                        vec![TaeProp { id: PropertyKey::Layer as u8, val: TORSO_LAYER }]
                    } else {
                        Vec::new()
                    },
                    ..TileArtEntry::default()
                },
            })
            .collect();
        files.sort_by_key(|file| file.entry.tile_id);
        let definitions = Definitions(&files);
        assert_eq!(definitions.get_named(400).map(|entry| entry.tile_id), Some(MOBILE_DEFINITION_BASE + 400));
        assert_eq!(item_detail(None, definitions, 400, LAYERED_ITEM, None), "layer=5 torso (Render to find the frame)");
        assert_eq!(item_detail(None, definitions, 400, 1, Some(3)), "layer=3 feet (Render to find the frame)");
    }

    /// A record is found by its entry name, not by the tile id it stores.
    #[test]
    fn records_are_found_by_their_entry_name() {
        let record = |entry_id: u32, layer: u32| TileArtFileEntry {
            filename_hash: tileart_entry_hash(entry_id),
            entry: TileArtEntry {
                tile_id: RENAMED_STORED_ID,
                prop_vector1: vec![TaeProp { id: PropertyKey::Layer as u8, val: layer }],
                ..TileArtEntry::default()
            },
        };
        // Sorted by stored tile id, the renamed record first.
        let files = [record(RENAMED_ENTRY, TORSO_LAYER), record(RENAMED_STORED_ID, STORED_ID_LAYER)];
        let definitions = Definitions(&files);
        let layer = |entry_id| definitions.get(entry_id).and_then(|entry| entry.property1(PropertyKey::Layer));
        assert_eq!(layer(RENAMED_ENTRY), Some(TORSO_LAYER));
        assert_eq!(layer(RENAMED_STORED_ID), Some(STORED_ID_LAYER));
        assert!(definitions.get(LAYERED_ITEM).is_none());
    }

    /// The editor's records, sorted for display by stored tile id, find for every id the record `NamedTileArt` finds,
    /// which is how `tilemeta.uddp` keys its item definitions, the default record among them.
    #[test]
    #[ignore = "needs real Enhanced clients: set UO_EC_CLIENT_ROOTS and run with --ignored"]
    fn shipped_records_are_found_as_named_tileart_finds_them() {
        use crate::ui::test_support::{client_roots, EC_CLIENT_ROOTS_ENV};
        use uocf::enhanced::paperdoll::{NamedTileArt, DEFAULT_DEFINITION_ID, ENTRY_ID_SPAN};
        use uocf::enhanced::tileart::TILEART_UOP_NAME;
        use uocf::uop_container::package::UopPackage;
        use uocf::utils::path::find_file_ignoring_case;
        for root in client_roots(EC_CLIENT_ROOTS_ENV) {
            let package = UopPackage::load(find_file_ignoring_case(&root, TILEART_UOP_NAME).unwrap()).unwrap();
            let mut files: Vec<TileArtFileEntry> = package
                .iter_files()
                .filter_map(|file| {
                    let entry = TileArtEntry::parse_raw(file).ok()?;
                    Some(TileArtFileEntry { filename_hash: file.filename_hash(), entry })
                })
                .collect();
            files.sort_by_key(|file| file.entry.tile_id);
            let named = NamedTileArt::read(&package);
            assert!(!named.renamed.is_empty(), "{}", root.display());
            let editor = Definitions(&files);
            let ids = (0..ENTRY_ID_SPAN).chain(MOBILE_DEFINITION_BASE..MOBILE_DEFINITION_BASE + ENTRY_ID_SPAN);
            for id in ids.chain([DEFAULT_DEFINITION_ID]) {
                assert!(editor.get(id) == (&named).get(id), "{}: entry {id}", root.display());
            }
        }
    }
}
