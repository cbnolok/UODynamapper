//! The Enhanced Client's paperdoll images: the body and worn-item frames of `Paperdoll.uop`, found and ordered as
//! `ECPaperdollTexture::Build` (UOSA 0x0050b340) does. The lookup is `uocf::enhanced::paperdoll`, shared with
//! dynamapper; this module reads the definitions from the loaded `tileart.uop` and holds the rules shipped with
//! dynamapper.

use uocf::enhanced::paperdoll::{item_frame, item_layer, PaperdollDefinitions, Wearer, MALE_SET, SLOT_NAMES};
pub(crate) use uocf::enhanced::paperdoll::{build, compose, hue_tables, FrameSets, WearableRules, WornItem, PAPERDOLL_UOP_NAME};
use uocf::enhanced::tileart::TileArtEntry;

use crate::app::TileArtFileEntry;

/// `wearables.xml` of `MainMisc.uop`, as dynamapper reads it.
const WEARABLE_RULES_KDL: &str =
    include_str!("../../../../dynamapper/assets/runtime_specs/paperdoll/WearableRules.kdl");

/// `tileart.uop` records by tile id: `UOTileArt::GetObjectDefinition`.
#[derive(Clone, Copy)]
pub(crate) struct Definitions<'a>(pub &'a [TileArtFileEntry]);

impl<'a> PaperdollDefinitions<'a> for Definitions<'a> {
    type Definition = &'a TileArtEntry;

    /// The record of `tile_id`; the entries are sorted by tile id.
    fn get(self, tile_id: u32) -> Option<&'a TileArtEntry> {
        let index = self.0.binary_search_by_key(&tile_id, |file| file.entry.tile_id).ok()?;
        Some(&self.0[index].entry)
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
                filename_hash: 0,
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
}
