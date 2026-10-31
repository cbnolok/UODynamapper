//! The client generation a file layout belongs to.
//!
//! The Kingdom Reborn client (`UOKR.exe`, 2008) stores several packages in an older form of the Enhanced Client
//! layouts: facet squares with 3 more bytes, tileart records version 3, shifted effect type keys, AnimationSequence
//! entries without the i16 offsets, MultiCollection parts with one flag byte. Parsers and writers that serve both
//! take a [`ClientDialect`]; the KR differences are listed where each format is read. Where a field of the KR form
//! has no confirmed meaning yet it is kept verbatim and named neutrally (`kr_*`).

/// Which client's form of a shared file layout to read or write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ClientDialect {
    /// The Enhanced Client (and the Classic Client where it reads the same packages).
    #[default]
    Enhanced,
    /// The Kingdom Reborn client.
    KingdomReborn,
}

impl ClientDialect {
    pub fn is_kingdom_reborn(self) -> bool {
        self == Self::KingdomReborn
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Enhanced => "Enhanced Client",
            Self::KingdomReborn => "Kingdom Reborn",
        }
    }
}
