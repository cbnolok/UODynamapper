//! The client generation a file layout belongs to.
//!
//! The Kingdom Reborn client (`UOKR.exe`, 2008) stores several packages in an older form of the Enhanced Client
//! layouts: facet squares with 3 more bytes, tileart records version 3, shifted effect type keys, AnimationSequence
//! entries without the i16 offsets, MultiCollection parts with one flag byte. Parsers and writers that serve both
//! take a [`ClientDialect`]; the KR differences are listed where each format is read. Where a field of the KR form
//! has no confirmed meaning yet it is kept verbatim and named neutrally (`kr_*`).

use std::path::Path;

use crate::client::login_servers::KINGDOM_REBORN_EXE;
use crate::utils::path::find_file_ignoring_case;

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

    /// The dialect of the client folder `dir`: Kingdom Reborn when it holds `UOKR.exe` (any case), else Enhanced
    /// Client. For packages whose data reads in both forms (no field tells them apart), so the folder decides
    /// what an edit writes.
    pub fn of_client_dir(dir: &Path) -> Self {
        if find_file_ignoring_case(dir, KINGDOM_REBORN_EXE).is_some() {
            Self::KingdomReborn
        } else {
            Self::Enhanced
        }
    }

    /// [`Self::of_client_dir`] of the folder holding the package `path`.
    pub fn of_package_path(path: &Path) -> Self {
        path.parent().map_or(Self::Enhanced, Self::of_client_dir)
    }
}
