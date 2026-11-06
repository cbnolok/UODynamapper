//! What a facet conversion needs to know about the client it writes for ([`TargetClient`]): the facet sizes from the
//! executable's size table (located by structure, `client_exe`; never a per-version table), whether Trammel reads
//! Felucca's files and whether the difs are applied (Classic), the map container its folder ships, the
//! `tiledata.mul` sizes, and which world map images (`facetNN.mul`) it draws.

use std::path::{Path, PathBuf};

use crate::classic::facet_image::{facet_image_file_name, FACET_IMAGE_COUNT};
use crate::classic::map::MapSizeCells;
use crate::classic::tiledata::TileData;
use crate::client::login_servers::{ENHANCED_EXE, KINGDOM_REBORN_EXE};
use crate::client::classic_difs::ClassicDifUsage;
use crate::client::classic_facets::classic_trammel_reads_map0;
use crate::client::facet_dimensions::{ClientExeInspection, ClientFamily};
use crate::utils::path::find_file_ignoring_case;

crate::eyre_imports!();

/// The executable of each client family, looked for in this order in a client folder.
pub const CLASSIC_EXE: &str = "client.exe";
pub const CLIENT_EXES: [&str; 3] = [CLASSIC_EXE, ENHANCED_EXE, KINGDOM_REBORN_EXE];
const TILEDATA_NAME: &str = "tiledata.mul";

/// The Classic map container a client folder ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicContainer {
    /// `mapN.mul` (clients before 7.0.24).
    Mul,
    /// `mapNLegacyMUL.uop`.
    Uop,
}

/// The `tiledata.mul` sizes of a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileDataCounts {
    pub land: usize,
    pub items: usize,
}

/// A client a facet conversion writes for (see the module documentation).
#[derive(Debug, Clone)]
pub struct TargetClient {
    pub exe: PathBuf,
    pub inspection: ClientExeInspection,
    /// The folder of the executable.
    pub dir: PathBuf,
    /// Classic: the facets whose `facetNN.mul` the executable names (7.0.20 and 2024 name all six; UO Gold and
    /// Mondain's Legacy none). The 2024 client draws them in the map item gump, reading tile (x, y) at column x
    /// of row y without bounds checks (`ghidra-classic-map-image`).
    pub facet_images: Vec<u32>,
}

impl TargetClient {
    /// Opens a client from its folder (the first of [`CLIENT_EXES`] found) or its executable.
    pub fn open(path: &Path) -> eyre::Result<Self> {
        let exe = if path.is_dir() {
            CLIENT_EXES.iter().find_map(|name| find_file_ignoring_case(path, name)).ok_or_else(|| {
                eyre!("{} holds none of {}", path.display(), CLIENT_EXES.join(", "))
            })?
        } else {
            path.to_path_buf()
        };
        let bytes = std::fs::read(&exe).wrap_err_with(|| format!("read {}", exe.display()))?;
        let inspection = ClientExeInspection::inspect(&bytes).wrap_err_with(|| format!("inspect {}", exe.display()))?;
        let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
        let facet_images = if inspection.family == ClientFamily::Classic {
            (0..FACET_IMAGE_COUNT)
                .filter(|&facet| {
                    let name = facet_image_file_name(facet);
                    bytes.windows(name.len()).any(|window| window.eq_ignore_ascii_case(name.as_bytes()))
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self { exe, inspection, dir, facet_images })
    }

    pub fn family(&self) -> ClientFamily {
        self.inspection.family
    }

    /// The size of `facet` in the size table (the first copy: the standard edition), or an error naming the
    /// facets the table has.
    pub fn facet_size(&self, facet: u32) -> eyre::Result<MapSizeCells> {
        let table = self.inspection.tables.first().ok_or_else(|| eyre!("{}: no size table", self.exe.display()))?;
        let record = table.records.get(facet as usize).ok_or_else(|| {
            eyre!("{}: the size table has facets 0..{} only, not {facet}", self.exe.display(), table.records.len())
        })?;
        Ok(MapSizeCells { width: record.geometry.width, height: record.geometry.height })
    }

    /// Classic: whether Trammel (facet 1) reads Felucca's map and statics (UO Gold to 5.x, with the facet-1 difs).
    pub fn trammel_reads_map0(&self) -> bool {
        self.family() == ClientFamily::Classic && self.inspection.tables.first().is_some_and(classic_trammel_reads_map0)
    }

    /// Classic: what the client does with the dif files.
    pub fn difs(&self) -> Option<&ClassicDifUsage> {
        self.inspection.difs.as_ref()
    }

    /// Classic: whether the client draws `facetNN.mul` for `facet` ([`Self::facet_images`]).
    pub fn reads_facet_image(&self, facet: u32) -> bool {
        self.facet_images.contains(&facet)
    }

    /// The Classic map container the client folder holds for `facet`, if any.
    pub fn classic_container(&self, facet: u32) -> Option<ClassicContainer> {
        if find_file_ignoring_case(&self.dir, &format!("map{facet}LegacyMUL.uop")).is_some() {
            Some(ClassicContainer::Uop)
        } else if find_file_ignoring_case(&self.dir, &format!("map{facet}.mul")).is_some() {
            Some(ClassicContainer::Mul)
        } else {
            None
        }
    }

    /// The `tiledata.mul` sizes of the client folder, when it has one.
    pub fn tiledata_counts(&self) -> eyre::Result<Option<TileDataCounts>> {
        let Some(path) = find_file_ignoring_case(&self.dir, TILEDATA_NAME) else {
            return Ok(None);
        };
        tiledata_counts(&path).map(Some)
    }
}

/// The land and item counts of a `tiledata.mul`.
pub fn tiledata_counts(path: &Path) -> eyre::Result<TileDataCounts> {
    let tiledata = TileData::load(path.to_path_buf()).wrap_err_with(|| format!("load {}", path.display()))?;
    Ok(TileDataCounts { land: tiledata.land_tiles().len(), items: tiledata.item_tiles().len() })
}
