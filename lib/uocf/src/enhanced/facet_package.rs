//! Editing Enhanced Client facet packages (`facetN.uop` and the variants `facetNx.uop`).
//!
//! [`EcFacetPackage`] reads sectors on demand, edits squares by map coordinates (land id and z, statics),
//! keeps the shipped delimiters of the edited squares' neighbours in step (also across sectors), and writes the
//! edited sectors back into the package. Saving keeps the source file's layout, so only edited entries change.
//! [`EcFacetPackage::new`] builds a package from scratch with the `data.map` entry the shipped packages have.
//!
//! A variant package (`facet0x.uop`, ...) is opened with its base facet index: it uses the base package's entry
//! names and sector ids. The radar image (`facetNN.dds`) is not in the package; see `facet_radar`.
//!
//! Kingdom Reborn facet packages open with `ClientDialect::KingdomReborn` ([`EcFacetPackage::open_dialect`], or
//! [`detect_facet_dialect`]): the same entries and rules, the KR square layout, and version 4 packages with 100
//! entries per block. Their land is edited with [`EcFacetPackage::set_kr_land`].

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::facet_decoder::{decode_sector, DecodedEcFacet, EcFacetDelimiter, EcFacetTile, KrSquareLand, StaticTile};
use crate::enhanced::facet_sector::{
    data_map_entry_path, delimiter_targets, encode_sector, local_coords, sector_entry_path, square_index,
    validate_sector, FacetGeometry, DELIMITER_OFFSETS, MAX_SQUARE_RECORDS,
};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{NewEntryCompression, UopPackage};

crate::eyre_imports!();

/// UOP version of the shipped facet packages.
pub const FACET_PACKAGE_VERSION: u32 = 5;
/// Entries per block of the shipped facet packages.
pub const FACET_PACKAGE_BLOCK_SIZE: u32 = 1000;
/// Compression of the shipped sector entries.
pub const SECTOR_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;
/// Compression of the shipped `data.map` entry.
pub const DATA_MAP_COMPRESSION: CompressionFlag = CompressionFlag::None;
/// UOP version of the shipped Kingdom Reborn facet packages (sectors zlib-compressed, `data.map` first and
/// uncompressed, 12-byte timestamp data block headers, as in the EC packages).
pub const KR_FACET_PACKAGE_VERSION: u32 = 4;
/// Entries per block of the shipped Kingdom Reborn facet packages.
pub const KR_FACET_PACKAGE_BLOCK_SIZE: u32 = 100;

/// The UOP version and block size of the shipped facet packages of `dialect`.
pub fn facet_package_layout(dialect: ClientDialect) -> (u32, u32) {
    match dialect {
        ClientDialect::Enhanced => (FACET_PACKAGE_VERSION, FACET_PACKAGE_BLOCK_SIZE),
        ClientDialect::KingdomReborn => (KR_FACET_PACKAGE_VERSION, KR_FACET_PACKAGE_BLOCK_SIZE),
    }
}

/// The square layout of a facet package: the dialect that decodes its first sector. Fails when neither does.
pub fn detect_facet_dialect(package: &UopPackage, facet: u8) -> eyre::Result<ClientDialect> {
    let geometry = FacetGeometry::client_facet(facet);
    let sector_count = geometry.map_or(u32::from(u16::MAX) + 1, |geometry| geometry.sector_count());
    for sector in 0..sector_count {
        let path = sector_entry_path(facet, sector);
        let Some(payload) = package.unpack_file_by_hash(hash_file_name_single(&path))? else {
            continue;
        };
        for dialect in [ClientDialect::Enhanced, ClientDialect::KingdomReborn] {
            if decode_sector(&payload, dialect).is_ok() {
                return Ok(dialect);
            }
        }
        eyre::bail!("{path} decodes neither as an Enhanced Client nor as a Kingdom Reborn sector");
    }
    eyre::bail!("the package has no sector of facet {facet}")
}

/// A facet package open for reading and editing sectors.
pub struct EcFacetPackage {
    facet: u8,
    dialect: ClientDialect,
    geometry: FacetGeometry,
    package: UopPackage,
    /// Sectors read for editing, by sector id.
    sectors: HashMap<u32, DecodedEcFacet>,
    /// Cached sectors changed since the last [`Self::flush`].
    dirty: BTreeSet<u32>,
}

impl EcFacetPackage {
    /// Opens `path` as facet `facet` (the base index for a variant package). The geometry comes from the
    /// `data.map` entry, else from the client's table.
    pub fn open(path: impl AsRef<Path>, facet: u8) -> eyre::Result<Self> {
        Self::open_dialect(path, facet, ClientDialect::Enhanced)
    }

    /// [`Self::open`] for a package in the square layout of `dialect`.
    pub fn open_dialect(path: impl AsRef<Path>, facet: u8, dialect: ClientDialect) -> eyre::Result<Self> {
        let path = path.as_ref();
        let package = UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
        Self::from_package_dialect(package, facet, None, dialect)
    }

    /// Wraps a loaded package. `geometry` overrides the `data.map` entry and the client table.
    pub fn from_package(package: UopPackage, facet: u8, geometry: Option<FacetGeometry>) -> eyre::Result<Self> {
        Self::from_package_dialect(package, facet, geometry, ClientDialect::Enhanced)
    }

    /// [`Self::from_package`] for a package in the square layout of `dialect`.
    pub fn from_package_dialect(
        package: UopPackage,
        facet: u8,
        geometry: Option<FacetGeometry>,
        dialect: ClientDialect,
    ) -> eyre::Result<Self> {
        let geometry = match geometry {
            Some(geometry) => geometry,
            None => match package.unpack_file_by_hash(hash_file_name_single(&data_map_entry_path(facet)))? {
                Some(payload) => FacetGeometry::from_data_map_payload(&payload)?,
                None => FacetGeometry::client_facet(facet)
                    .ok_or_else(|| eyre!("facet {facet}: no data.map entry and no client size; pass a geometry"))?,
            },
        };
        check_sector_ids_fit(&geometry)?;
        Ok(Self { facet, dialect, geometry, package, sectors: HashMap::new(), dirty: BTreeSet::new() })
    }

    /// An empty package for facet `facet`, holding only the `data.map` entry (u32 height, u32 width,
    /// uncompressed). Add every sector with [`Self::put_sector`]: the client leaves a hole for a missing one.
    pub fn new(facet: u8, geometry: FacetGeometry) -> eyre::Result<Self> {
        Self::new_dialect(facet, geometry, ClientDialect::Enhanced)
    }

    /// [`Self::new`] for `dialect`, with the package version and block size of its shipped packages
    /// ([`facet_package_layout`]).
    pub fn new_dialect(facet: u8, geometry: FacetGeometry, dialect: ClientDialect) -> eyre::Result<Self> {
        check_sector_ids_fit(&geometry)?;
        let (version, block_size) = facet_package_layout(dialect);
        let mut package = UopPackage::new(version, block_size)?;
        package.add_entry_by_name(
            &data_map_entry_path(facet),
            &geometry.data_map_payload(),
            NewEntryCompression::Flag(DATA_MAP_COMPRESSION),
        )?;
        Ok(Self { facet, dialect, geometry, package, sectors: HashMap::new(), dirty: BTreeSet::new() })
    }

    pub fn facet(&self) -> u8 {
        self.facet
    }

    /// The square layout of the package's sectors.
    pub fn dialect(&self) -> ClientDialect {
        self.dialect
    }

    pub fn geometry(&self) -> FacetGeometry {
        self.geometry
    }

    /// The package, as of the last [`Self::flush`].
    pub fn package(&self) -> &UopPackage {
        &self.package
    }

    /// Reads sector `sector` from the package, ignoring edits not yet flushed. `None` when the package has no
    /// such entry (the client leaves a hole). Fails on a sector the client would not use.
    pub fn read_sector_from_package(&self, sector: u32) -> eyre::Result<Option<DecodedEcFacet>> {
        let path = sector_entry_path(self.facet, sector);
        let Some(payload) = self.package.unpack_file_by_hash(hash_file_name_single(&path))? else {
            return Ok(None);
        };
        let decoded = decode_sector(&payload, self.dialect).wrap_err_with(|| format!("decode {path}"))?;
        if let Some(issue) = validate_sector(&decoded, Some((self.facet, sector)), self.dialect).first() {
            eyre::bail!("{path}: {issue}");
        }
        Ok(Some(decoded))
    }

    /// Calls `f` for every sector in id order, with the edited version when there is one. Missing sectors are
    /// skipped. Sectors not being edited are decoded one at a time and not kept.
    pub fn for_each_sector(&self, mut f: impl FnMut(u32, &DecodedEcFacet) -> eyre::Result<()>) -> eyre::Result<()> {
        for sector in 0..self.geometry.sector_count() {
            if let Some(cached) = self.sectors.get(&sector) {
                f(sector, cached)?;
            } else if let Some(decoded) = self.read_sector_from_package(sector)? {
                f(sector, &decoded)?;
            }
        }
        Ok(())
    }

    /// Sector `sector` for reading, kept for later edits.
    pub fn sector(&mut self, sector: u32) -> eyre::Result<&DecodedEcFacet> {
        self.load_sector(sector)?;
        Ok(&self.sectors[&sector])
    }

    /// Sector `sector` for editing; it is written back by [`Self::flush`]. Edits through this reference do not
    /// refresh delimiters: call [`Self::refresh_delimiters_around`] for edited land.
    pub fn sector_mut(&mut self, sector: u32) -> eyre::Result<&mut DecodedEcFacet> {
        self.load_sector(sector)?;
        self.dirty.insert(sector);
        Ok(self.sectors.get_mut(&sector).expect("the sector was just loaded"))
    }

    /// Replaces (or adds) the sector whose header names it, to be written by [`Self::flush`].
    pub fn set_sector(&mut self, sector: DecodedEcFacet) -> eyre::Result<()> {
        let id = u32::from(sector.file_id);
        self.check_sector(&sector, id)?;
        self.sectors.insert(id, sector);
        self.dirty.insert(id);
        Ok(())
    }

    /// Encodes `sector` straight into the package (replacing or adding its entry) without keeping it, for
    /// building a whole facet without holding every sector in memory.
    pub fn put_sector(&mut self, sector: &DecodedEcFacet) -> eyre::Result<()> {
        let id = u32::from(sector.file_id);
        self.check_sector(sector, id)?;
        self.write_entry(id, &encode_sector(sector, self.dialect)?)?;
        self.sectors.remove(&id);
        self.dirty.remove(&id);
        Ok(())
    }

    /// The square at map `(x, y)`.
    pub fn square(&mut self, x: u32, y: u32) -> eyre::Result<&EcFacetTile> {
        let (sector, index) = self.locate(x, y)?;
        Ok(&self.sector(sector)?.tiles[index])
    }

    /// Sets the land id and z of the square at map `(x, y)` and refreshes the delimiters that point at it from
    /// the neighbouring sectors. Fails on a Kingdom Reborn package, whose squares also store the Classic id:
    /// use [`Self::set_kr_land`].
    pub fn set_land(&mut self, x: u32, y: u32, land_id: u16, z: i8) -> eyre::Result<()> {
        if self.dialect.is_kingdom_reborn() {
            eyre::bail!("({x}, {y}): a Kingdom Reborn square needs its material, subtype and Classic id (set_kr_land)");
        }
        let tile = self.square_mut(x, y)?;
        tile.land_graphic = land_id;
        tile.z = z;
        self.refresh_delimiters_around(x, y)?;
        Ok(())
    }

    /// Sets the land of the Kingdom Reborn square at map `(x, y)`: the material, the `subtype` and stored Classic
    /// id, and z; refreshes the delimiters that point at it (they hold the material). Fails on an
    /// Enhanced Client package.
    pub fn set_kr_land(&mut self, x: u32, y: u32, material: u16, kr: KrSquareLand, z: i8) -> eyre::Result<()> {
        if !self.dialect.is_kingdom_reborn() {
            eyre::bail!("({x}, {y}): an Enhanced Client square has no Kingdom Reborn bytes (set_land)");
        }
        let tile = self.square_mut(x, y)?;
        tile.land_graphic = material;
        tile.kr = Some(kr);
        tile.z = z;
        self.refresh_delimiters_around(x, y)?;
        Ok(())
    }

    /// Appends a static to the square at map `(x, y)`; returns its index. Fails when the square already holds
    /// 255 statics (u8 count).
    pub fn add_static(&mut self, x: u32, y: u32, graphic: u32, z: i8, hue: u32) -> eyre::Result<usize> {
        let (local_x, local_y) = local_coords(x, y);
        let tile = self.square_mut(x, y)?;
        if tile.statics.len() >= MAX_SQUARE_RECORDS {
            eyre::bail!("square ({x}, {y}) already holds {MAX_SQUARE_RECORDS} statics");
        }
        tile.statics.push(StaticTile { graphic_id: graphic, x: local_x as u8, y: local_y as u8, z, hue });
        Ok(tile.statics.len() - 1)
    }

    /// Removes static `index` of the square at map `(x, y)` and returns it.
    pub fn remove_static(&mut self, x: u32, y: u32, index: usize) -> eyre::Result<StaticTile> {
        let tile = self.square_mut(x, y)?;
        if index >= tile.statics.len() {
            eyre::bail!("square ({x}, {y}) has {} statics, no static {index}", tile.statics.len());
        }
        Ok(tile.statics.remove(index))
    }

    /// Changes static `index` of the square at map `(x, y)`.
    pub fn set_static(&mut self, x: u32, y: u32, index: usize, graphic: u32, z: i8, hue: u32) -> eyre::Result<()> {
        let tile = self.square_mut(x, y)?;
        let count = tile.statics.len();
        let item = tile
            .statics
            .get_mut(index)
            .ok_or_else(|| eyre!("square ({x}, {y}) has {count} statics, no static {index}"))?;
        item.graphic_id = graphic;
        item.z = z;
        item.hue = hue;
        Ok(())
    }

    /// After the land of map `(x, y)` changed: in every square around it that points at it, sets that delimiter
    /// to the new land z and id (adding it if the square lacks it), following the shipped rule. Other
    /// delimiters of those squares are left as they are. Returns how many squares changed.
    pub fn refresh_delimiters_around(&mut self, x: u32, y: u32) -> eyre::Result<usize> {
        let (z, land) = {
            let tile = self.square(x, y)?;
            (tile.z, tile.land_graphic)
        };
        let mut changed = 0;
        for &(dx, dy) in &DELIMITER_OFFSETS {
            let Some((nx, ny)) = self.geometry.offset(x, y, dx, dy) else {
                continue;
            };
            let Some((direction, _, _)) =
                delimiter_targets(&self.geometry, nx, ny).into_iter().find(|&(_, tx, ty)| (tx, ty) == (x, y))
            else {
                continue;
            };
            let expected = EcFacetDelimiter { direction, z, graphic_id: u32::from(land) };
            let (sector, index) = self.locate(nx, ny)?;
            if self.sector(sector)?.tiles[index].delimiters.contains(&expected) {
                continue;
            }
            let tile = &mut self.sector_mut(sector)?.tiles[index];
            match tile.delimiters.iter_mut().find(|delimiter| delimiter.direction == direction) {
                Some(delimiter) => *delimiter = expected,
                None => {
                    let at = tile.delimiters.partition_point(|delimiter| delimiter.direction < direction);
                    tile.delimiters.insert(at, expected);
                }
            }
            changed += 1;
        }
        Ok(changed)
    }

    /// Replaces the delimiters of the square at map `(x, y)` with the shipped set computed from its neighbours.
    /// Returns whether they changed.
    pub fn recompute_square_delimiters(&mut self, x: u32, y: u32) -> eyre::Result<bool> {
        let mut expected = Vec::new();
        for (direction, nx, ny) in delimiter_targets(&self.geometry, x, y) {
            let neighbour = self.square(nx, ny)?;
            expected.push(EcFacetDelimiter {
                direction,
                z: neighbour.z,
                graphic_id: u32::from(neighbour.land_graphic),
            });
        }
        if self.square(x, y)?.delimiters == expected {
            return Ok(false);
        }
        self.square_mut(x, y)?.delimiters = expected;
        Ok(true)
    }

    /// Sector ids edited since the last [`Self::flush`].
    pub fn dirty_sectors(&self) -> impl Iterator<Item = u32> + '_ {
        self.dirty.iter().copied()
    }

    /// Encodes the edited sectors into the package; returns how many were written. Sectors that encode to their
    /// current payload keep their entry untouched.
    pub fn flush(&mut self) -> eyre::Result<usize> {
        let dirty: Vec<u32> = std::mem::take(&mut self.dirty).into_iter().collect();
        for &sector in &dirty {
            let payload = encode_sector(&self.sectors[&sector], self.dialect)?;
            self.write_entry(sector, &payload)?;
        }
        Ok(dirty.len())
    }

    /// Drops the cached sectors that have no unflushed edits.
    pub fn evict_clean_sectors(&mut self) {
        let dirty = &self.dirty;
        self.sectors.retain(|sector, _| dirty.contains(sector));
    }

    /// Flushes the edits and saves the package to `path`, keeping the layout of the file it was loaded from.
    pub fn save(&mut self, path: impl AsRef<Path>) -> eyre::Result<()> {
        let path = path.as_ref();
        self.flush()?;
        self.package.finalize_and_save(path).wrap_err_with(|| format!("save {}", path.display()))?;
        Ok(())
    }

    fn check_sector(&self, sector: &DecodedEcFacet, id: u32) -> eyre::Result<()> {
        if id >= self.geometry.sector_count() {
            eyre::bail!("sector {id} is outside the facet's {} sectors", self.geometry.sector_count());
        }
        if let Some(issue) = validate_sector(sector, Some((self.facet, id)), self.dialect).first() {
            eyre::bail!("facet {} sector {id}: {issue}", self.facet);
        }
        Ok(())
    }

    fn write_entry(&mut self, sector: u32, payload: &[u8]) -> eyre::Result<()> {
        let hash = hash_file_name_single(&sector_entry_path(self.facet, sector));
        if self.package.get_file_by_hash(hash).is_some() {
            self.package.replace_file(hash, payload, SECTOR_COMPRESSION)?;
        } else {
            self.package.add_entry(hash, payload, NewEntryCompression::Flag(SECTOR_COMPRESSION))?;
        }
        Ok(())
    }

    fn load_sector(&mut self, sector: u32) -> eyre::Result<()> {
        if !self.sectors.contains_key(&sector) {
            let decoded = self
                .read_sector_from_package(sector)?
                .ok_or_else(|| eyre!("the package has no {}", sector_entry_path(self.facet, sector)))?;
            self.sectors.insert(sector, decoded);
        }
        Ok(())
    }

    /// The sector and square index of map `(x, y)`; fails off the map.
    fn locate(&self, x: u32, y: u32) -> eyre::Result<(u32, usize)> {
        if !self.geometry.contains(x, y) {
            eyre::bail!("({x}, {y}) is outside the {}x{} facet", self.geometry.width, self.geometry.height);
        }
        let (local_x, local_y) = local_coords(x, y);
        Ok((self.geometry.sector_of(x, y), square_index(local_x, local_y)))
    }

    fn square_mut(&mut self, x: u32, y: u32) -> eyre::Result<&mut EcFacetTile> {
        let (sector, index) = self.locate(x, y)?;
        Ok(&mut self.sector_mut(sector)?.tiles[index])
    }
}

/// Sector ids are u16 in the sector header.
fn check_sector_ids_fit(geometry: &FacetGeometry) -> eyre::Result<()> {
    if geometry.width == 0 || geometry.height == 0 {
        eyre::bail!("empty facet ({}x{})", geometry.width, geometry.height);
    }
    if geometry.sector_count() > u32::from(u16::MAX) + 1 {
        eyre::bail!(
            "a {}x{} facet has {} sectors; sector ids are u16",
            geometry.width,
            geometry.height,
            geometry.sector_count()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::facet_sector::empty_ec_sector;

    const FACET: u8 = 3;
    const WIDTH: u32 = 128;
    const HEIGHT: u32 = 100;

    /// A 128x100 facet (2x2 sectors, the bottom row partial) with land id `x + y` and z `x - y` on the map and
    /// the shipped delimiters.
    fn small_facet() -> EcFacetPackage {
        let geometry = FacetGeometry::new(WIDTH, HEIGHT);
        let land = |x: u32, y: u32| ((x as i32 - y as i32) as i8, (x + y) as u16);
        let mut facet = EcFacetPackage::new(FACET, geometry).unwrap();
        for sector in 0..geometry.sector_count() {
            let (ox, oy) = geometry.sector_origin(sector);
            let mut decoded = empty_ec_sector(FACET, sector).unwrap();
            for (index, tile) in decoded.tiles.iter_mut().enumerate() {
                let (x, y) = (ox + index as u32 / 64, oy + index as u32 % 64);
                if geometry.contains(x, y) {
                    (tile.z, tile.land_graphic) = land(x, y);
                    tile.delimiters = crate::enhanced::facet_sector::expected_delimiters(&geometry, x, y, land);
                }
            }
            facet.put_sector(&decoded).unwrap();
        }
        facet
    }

    #[test]
    fn set_land_refreshes_delimiters_in_neighbouring_sectors() {
        let mut facet = small_facet();
        // (63, 63) is the bottom-right corner of sector 0: sector 1 (below), 2 (right) and 3 (diagonal) point at it.
        facet.set_land(63, 63, 999, -7).unwrap();
        let expected = |direction| EcFacetDelimiter { direction, z: -7, graphic_id: 999 };
        assert!(facet.square(64, 63).unwrap().delimiters.contains(&expected(0)));
        assert!(facet.square(63, 64).unwrap().delimiters.contains(&expected(2)));
        assert!(facet.square(64, 64).unwrap().delimiters.contains(&expected(1)));
        // Squares of sector 0 hold no delimiter pointing at it.
        assert!(facet.square(62, 62).unwrap().delimiters.is_empty());
        assert_eq!(facet.square(62, 63).unwrap().delimiters.len(), 1);
        assert_eq!(facet.dirty_sectors().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        for (x, y) in [(64, 63), (63, 64), (64, 64), (63, 63)] {
            assert!(!facet.recompute_square_delimiters(x, y).unwrap());
        }
    }

    #[test]
    fn statics_are_capped_and_edits_survive_a_save() {
        let mut facet = small_facet();
        for _ in 0..MAX_SQUARE_RECORDS {
            facet.add_static(5, 6, 0x0EED, 3, 0).unwrap();
        }
        assert!(facet.add_static(5, 6, 0x0EED, 3, 0).is_err());
        facet.remove_static(5, 6, 0).unwrap();
        facet.set_static(5, 6, 1, 0x1F2A, 9, 1153).unwrap();
        facet.set_land(127, 99, 3, 4).unwrap();

        let dir = std::env::temp_dir().join(format!("uocf_facet_package_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("facet3.uop");
        facet.save(&path).unwrap();
        let mut reopened = EcFacetPackage::open(&path, FACET).unwrap();
        assert_eq!(reopened.geometry(), FacetGeometry::new(WIDTH, HEIGHT));
        let square = reopened.square(5, 6).unwrap();
        assert_eq!(square.statics.len(), MAX_SQUARE_RECORDS - 1);
        assert_eq!((square.statics[1].graphic_id, square.statics[1].z, square.statics[1].hue), (0x1F2A, 9, 1153));
        assert_eq!(reopened.square(127, 99).unwrap().land_graphic, 3);
        // The last row of squares in sector 3 is padding.
        assert_eq!(reopened.sector(3).unwrap().tiles[square_index(63, 63)].land_graphic, 0);
        assert!(reopened.square(0, HEIGHT).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
