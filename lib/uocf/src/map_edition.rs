//! The two editions of a facet's map data. From Classic 7.0.20 and in the Enhanced Client, some facets ship a second
//! copy of their map and statics, the `x` files (`mapNxLegacyMUL.uop`/`mapNx.mul`, `staidxNx.mul`, `staticsNx.mul`;
//! `facetNx.uop`). The server selects them for the whole session with bit 0x8000 of the 0xA9 character-list flags:
//! the Classic client then reads its second facet table, the Enhanced Client mounts `facetNx.uop` in place of
//! `facetN.uop`. The variant is an alternate source of the facet, never a patch layer over the standard files, and
//! no client falls back from one to the other. See
//! `scripts/dev-helpers/ec-research/ghidra-classic-facets/results/classic_facets.md` and
//! `scripts/dev-helpers/ec-research/ghidra-facets/results/facet_variants_data.md`.

use serde::{Deserialize, Serialize};

/// Which edition of a facet's map data to read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MapEdition {
    /// The files every client reads without the 0xA9 flag 0x8000: `mapN`, `staidxN`, `staticsN`, `facetN.uop`.
    #[default]
    Standard,
    /// The `x` files the server selects with the 0xA9 flag 0x8000.
    Variant,
}

impl MapEdition {
    pub const ALL: [Self; 2] = [Self::Standard, Self::Variant];

    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Variant => "variant",
        }
    }

    pub fn is_variant(self) -> bool {
        self == Self::Variant
    }

    /// The suffix the edition's file names put after the facet index: `""` or `"x"`.
    pub fn file_suffix(self) -> &'static str {
        match self {
            Self::Standard => "",
            Self::Variant => "x",
        }
    }
}

impl std::fmt::Display for MapEdition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.label())
    }
}
