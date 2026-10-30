//! Server body to animation sequence and frame body, from the `tileart.uop` creature placeholders.
//!
//! The Enhanced Client draws server body `S` through the tileart record `10000000 + S`
//! (`UOTileArt::GetObjectDefinition`, see
//! `scripts/dev-helpers/ec-research/ghidra-animation/results/uosa_animation.md`, "Body to sequence
//! and frames"):
//!
//! - the `AnimationSequence.uop` entry is the record's body type (+0x20);
//! - the `AnimationFrame*.uop` files are keyed by the record's appearance (props1 key 8, else the
//!   body; minus 10000000 when above it); a non-zero animation appearance (filter subtype 2)
//!   replaces it, and body 0 is used when the frame body has no frames;
//! - ghost bodies are drawn with `GHOST_HUE`.
//!
//! No hardcoded table, gender or race substitution is involved.

use std::collections::BTreeMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};

use crate::enhanced::tileart::{PropertyKey, TaeAppearanceFilter, TileArtEntry};
use crate::uop_container::package::UopPackage;

pub use crate::enhanced::hues::GHOST_HUE;

/// Tileart id of the creature placeholder of server body 0; the record of body `S` is
/// `MOBILE_DEFINITION_BASE + S`.
pub const MOBILE_DEFINITION_BASE: u32 = 10_000_000;
/// Frame body the client retries with when the appearance has no frames.
pub const FALLBACK_FRAMES_BODY: u32 = 0;
/// Bodies the client draws with `GHOST_HUE` (checked on the definition's tile id).
pub const GHOST_BODIES: [u32; 6] = [402, 403, 607, 608, 694, 695];

/// Byte offset of the tile id in a tileart record (after the u16 version and u32 string offset).
const TILEART_TILE_ID_OFFSET: usize = 6;
const TILEART_TILE_ID_SIZE: usize = 4;

/// Animation identity of one server body, read from its creature placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileBodyDefinition {
    /// Server body.
    pub body: u32,
    /// `AnimationSequence.uop` entry: the placeholder's body type.
    pub sequence_id: u32,
    /// Frame body from `GetAppearance`: props1 key 8, else the body, minus
    /// `MOBILE_DEFINITION_BASE` when above it.
    pub appearance: u32,
    /// Non-zero animation appearance (filter subtype 2) matching the placeholder's own body type;
    /// replaces `appearance`.
    pub animation_appearance: Option<u32>,
}

/// Frame body after the client's fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedFramesBody {
    pub body: u32,
    /// The requested frame body had no frames and `FALLBACK_FRAMES_BODY` is used.
    pub fallback: bool,
}

impl MobileBodyDefinition {
    /// Definition of a creature placeholder record; `None` for any other record.
    pub fn from_tileart_entry(entry: &TileArtEntry) -> Option<Self> {
        let body = entry.tile_id.checked_sub(MOBILE_DEFINITION_BASE)?;
        let appearance = strip_definition_base(entry.property1(PropertyKey::Appearance).unwrap_or(entry.tile_id));
        let animation_appearance = entry.appearance_vector.iter().find_map(|filter| match filter {
            TaeAppearanceFilter::AnimationByOwnerBodyType(mappings) => mappings
                .iter()
                .find(|mapping| mapping.body_type == entry.body_type && mapping.appearance != 0)
                .map(|mapping| strip_definition_base(mapping.appearance)),
            _ => None,
        });
        Some(Self {
            body,
            sequence_id: entry.body_type,
            appearance,
            animation_appearance,
        })
    }

    /// Frame body the client asks for first.
    pub fn requested_frames_body(&self) -> u32 {
        self.animation_appearance.unwrap_or(self.appearance)
    }

    /// Frame body with the client's fallback to `FALLBACK_FRAMES_BODY`; `None` when neither has
    /// frames (the client draws no layer).
    pub fn resolve_frames_body(&self, has_frames: impl Fn(u32) -> bool) -> Option<ResolvedFramesBody> {
        let requested = self.requested_frames_body();
        if has_frames(requested) {
            Some(ResolvedFramesBody {
                body: requested,
                fallback: false,
            })
        } else if has_frames(FALLBACK_FRAMES_BODY) {
            Some(ResolvedFramesBody {
                body: FALLBACK_FRAMES_BODY,
                fallback: true,
            })
        } else {
            None
        }
    }

    pub fn is_ghost(&self) -> bool {
        is_ghost_body(self.body)
    }

    /// Hue the client forces on this body (`GHOST_HUE` for ghosts).
    pub fn forced_hue(&self) -> Option<u16> {
        self.is_ghost().then_some(GHOST_HUE)
    }
}

pub fn is_ghost_body(body: u32) -> bool {
    GHOST_BODIES.contains(&body)
}

fn strip_definition_base(id: u32) -> u32 {
    if id >= MOBILE_DEFINITION_BASE {
        id - MOBILE_DEFINITION_BASE
    } else {
        id
    }
}

/// Every creature placeholder of `tileart.uop`, keyed by server body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MobileBodyMap {
    pub definitions: BTreeMap<u32, MobileBodyDefinition>,
}

impl MobileBodyMap {
    pub fn load(tileart_path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UopPackage::load(tileart_path).wrap_err("failed to load tileart.uop")?;
        Self::from_tileart_package(&package)
    }

    /// Parses only the placeholder records (tile id >= `MOBILE_DEFINITION_BASE`).
    pub fn from_tileart_package(package: &UopPackage) -> eyre::Result<Self> {
        let mut definitions = BTreeMap::new();
        for file in package.iter_files() {
            if !file.has_size() {
                continue;
            }
            let data = file.unpack_arc().wrap_err("failed to unpack a tileart record")?;
            let Some(tile_id) = data
                .get(TILEART_TILE_ID_OFFSET..TILEART_TILE_ID_OFFSET + TILEART_TILE_ID_SIZE)
                .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            else {
                continue;
            };
            if tile_id < MOBILE_DEFINITION_BASE {
                continue;
            }
            let entry = TileArtEntry::parse_bytes(&data)
                .wrap_err_with(|| format!("failed to parse tileart record {tile_id}"))?;
            if let Some(definition) = MobileBodyDefinition::from_tileart_entry(&entry) {
                definitions.insert(definition.body, definition);
            }
        }
        Ok(Self { definitions })
    }

    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a TileArtEntry>) -> Self {
        let definitions = entries
            .into_iter()
            .filter_map(MobileBodyDefinition::from_tileart_entry)
            .map(|definition| (definition.body, definition))
            .collect();
        Self { definitions }
    }

    pub fn definition(&self, body: u32) -> Option<&MobileBodyDefinition> {
        self.definitions.get(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::tileart::{TaeBodyAppearance, TaeProp};

    const HUMAN_MALE: u32 = 400;
    const HUMAN_FEMALE: u32 = 401;

    fn placeholder(body: u32, body_type: u32, appearance: Option<u32>) -> TileArtEntry {
        TileArtEntry {
            tile_id: MOBILE_DEFINITION_BASE + body,
            body_type,
            prop_vector1: appearance
                .map(|val| TaeProp {
                    id: PropertyKey::Appearance as u8,
                    val,
                })
                .into_iter()
                .collect(),
            ..TileArtEntry::default()
        }
    }

    #[test]
    fn placeholder_maps_body_to_sequence_and_frames() {
        let female = MobileBodyDefinition::from_tileart_entry(&placeholder(HUMAN_FEMALE, HUMAN_MALE, None)).unwrap();
        assert_eq!(female.sequence_id, HUMAN_MALE);
        assert_eq!(female.appearance, HUMAN_FEMALE);
        assert_eq!(female.requested_frames_body(), HUMAN_FEMALE);
        assert!(!female.is_ghost());

        let ghost = MobileBodyDefinition::from_tileart_entry(&placeholder(
            402,
            HUMAN_MALE,
            Some(MOBILE_DEFINITION_BASE + HUMAN_MALE),
        ))
        .unwrap();
        assert_eq!(ghost.requested_frames_body(), HUMAN_MALE);
        assert_eq!(ghost.forced_hue(), Some(GHOST_HUE));

        let plain_appearance =
            MobileBodyDefinition::from_tileart_entry(&placeholder(146, 124, Some(124))).unwrap();
        assert_eq!(plain_appearance.requested_frames_body(), 124);

        assert!(MobileBodyDefinition::from_tileart_entry(&TileArtEntry {
            tile_id: 0x0eed,
            ..TileArtEntry::default()
        })
        .is_none());
    }

    #[test]
    fn props2_appearance_is_ignored() {
        let mut entry = placeholder(HUMAN_FEMALE, HUMAN_MALE, None);
        entry.prop_vector2.push(TaeProp {
            id: PropertyKey::Appearance as u8,
            val: HUMAN_MALE,
        });
        let definition = MobileBodyDefinition::from_tileart_entry(&entry).unwrap();
        assert_eq!(definition.appearance, HUMAN_FEMALE);
    }

    #[test]
    fn animation_appearance_replaces_the_appearance() {
        let mut entry = placeholder(HUMAN_FEMALE, HUMAN_MALE, None);
        entry.appearance_vector.push(TaeAppearanceFilter::AnimationByOwnerBodyType(vec![
            TaeBodyAppearance {
                body_type: HUMAN_MALE,
                appearance: 0,
            },
            TaeBodyAppearance {
                body_type: HUMAN_MALE,
                appearance: MOBILE_DEFINITION_BASE + 605,
            },
        ]));
        let definition = MobileBodyDefinition::from_tileart_entry(&entry).unwrap();
        assert_eq!(definition.animation_appearance, Some(605));
        assert_eq!(definition.requested_frames_body(), 605);
    }

    #[test]
    fn frames_body_falls_back_to_body_zero() {
        let definition = MobileBodyDefinition::from_tileart_entry(&placeholder(433, HUMAN_MALE, None)).unwrap();

        assert_eq!(
            definition.resolve_frames_body(|body| body == 433),
            Some(ResolvedFramesBody {
                body: 433,
                fallback: false
            })
        );
        assert_eq!(
            definition.resolve_frames_body(|body| body == FALLBACK_FRAMES_BODY),
            Some(ResolvedFramesBody {
                body: FALLBACK_FRAMES_BODY,
                fallback: true
            })
        );
        assert_eq!(definition.resolve_frames_body(|_| false), None);
    }

    #[test]
    fn map_collects_only_placeholders() {
        let entries = [
            placeholder(HUMAN_FEMALE, HUMAN_MALE, None),
            TileArtEntry {
                tile_id: 0x0eed,
                ..TileArtEntry::default()
            },
        ];
        let map = MobileBodyMap::from_entries(&entries);
        assert_eq!(map.definitions.len(), 1);
        assert_eq!(map.definition(HUMAN_FEMALE).unwrap().sequence_id, HUMAN_MALE);
    }
}
