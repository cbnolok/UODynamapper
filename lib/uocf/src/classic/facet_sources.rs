//! Finds the files a Classic facet is read from: its map (`mapN.mul` or `mapNLegacyMUL.uop`), its statics
//! (`staidxN.mul`, `staticsN.mul`) and its dif patches (`mapdifN.mul`/`mapdiflN.mul`,
//! `stadifN.mul`/`stadiflN.mul`/`stadifiN.mul`), and opens them with the patches applied.
//!
//! Every reader of a Classic facet goes through [`ClassicFacetSources::resolve`], so the facets are found the same
//! way everywhere:
//! - A facet's own dif files are applied only when [`ClassicDifOptions`] asks for them; when asked for and absent,
//!   the facet is read unpatched.
//! - Clients before 7.0 (UO Gold, Mondain's Legacy) ship no `map1.mul`: they read Trammel from `map0.mul`,
//!   `staidx0.mul` and `statics0.mul` with the facet-1 difs applied. When a facet's own map is missing and the facet
//!   has such a base ([`SHARED_BASE_FACETS`]), it is built the same way whatever the options say, since that is the
//!   only way to read it, and the resolver warns.

crate::eyre_imports!();
use std::path::{Path, PathBuf};
use std::sync::Arc;

use log::{info, warn};

use crate::classic::map::{MapPlane, MapSizeCells};
use crate::classic::map_statics_diff::{MapDiff, StaticDiff};
use crate::classic::statics::StaticsReader;
use crate::classic::verdata::Verdata;
use crate::utils::path::find_file_ignoring_case;

/// Felucca's map index.
pub const FELUCCA: u32 = 0;
/// Trammel's map index.
pub const TRAMMEL: u32 = 1;

/// The Classic facet names, by map index.
pub const CLASSIC_FACET_NAMES: [&str; 6] = ["Felucca", "Trammel", "Ilshenar", "Malas", "Tokuno", "Ter Mur"];

/// Facets that clients without the facet's own map file read from another facet's map and statics with the
/// facet's own difs applied: (facet, base facet).
pub const SHARED_BASE_FACETS: [(u32, u32); 1] = [(TRAMMEL, FELUCCA)];

/// The facet whose files `verdata.mul` patches: its map, staidx and statics file ids (0, 1, 2) name `map0.mul`,
/// `staidx0.mul` and `statics0.mul`.
pub const VERDATA_FACET: u32 = FELUCCA;

/// The base facet whose files stand in for `facet` when its own map is missing ([`SHARED_BASE_FACETS`]).
pub fn shared_base_facet(facet: u32) -> Option<u32> {
    SHARED_BASE_FACETS.iter().find(|(shared, _)| *shared == facet).map(|&(_, base)| base)
}

fn facet_label(facet: u32) -> String {
    match CLASSIC_FACET_NAMES.get(facet as usize) {
        Some(name) => format!("{name} (map{facet})"),
        None => format!("map{facet}"),
    }
}

/// The container of a Classic map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicMapFormat {
    /// `mapN.mul`.
    Mul,
    /// `mapNLegacyMUL.uop` (or one of the other names of [`map_file_names`]).
    Uop,
}

impl ClassicMapFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mul => "MUL",
            Self::Uop => "UOP",
        }
    }
}

/// The file names map `map_index` can have in `format`, in the order they are looked for.
pub fn map_file_names(map_index: u32, format: ClassicMapFormat) -> Vec<String> {
    match format {
        ClassicMapFormat::Mul => vec![format!("map{map_index}.mul")],
        ClassicMapFormat::Uop => vec![
            format!("map{map_index}LegacyMUL.uop"),
            format!("map{map_index}.uop"),
            format!("map{map_index}xLegacyMUL.uop"),
            format!("map{map_index}x.uop"),
        ],
    }
}

/// Which of a facet's own dif files to apply. A facet built from a base facet's files always has its difs
/// applied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassicDifOptions {
    /// Apply `mapdifN.mul`/`mapdiflN.mul`.
    pub map_difs: bool,
    /// Apply `stadifN.mul`/`stadiflN.mul`/`stadifiN.mul`.
    pub static_difs: bool,
}

impl ClassicDifOptions {
    pub const NONE: Self = Self { map_difs: false, static_difs: false };
    pub const ALL: Self = Self { map_difs: true, static_difs: true };
}

/// A Classic map file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicMapFile {
    pub format: ClassicMapFormat,
    pub path: PathBuf,
    /// The map index the file was written for (0 for the `map0.mul` that stands in for Trammel).
    pub map_index: u32,
}

/// A facet's `staidxN.mul` and `staticsN.mul`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticsFiles {
    pub index: PathBuf,
    pub statics: PathBuf,
}

/// A facet's `mapdiflN.mul` (patched block ids) and `mapdifN.mul` (their blocks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapDifFiles {
    pub lookup: PathBuf,
    pub blocks: PathBuf,
}

impl MapDifFiles {
    pub fn file_names(facet: u32) -> [String; 2] {
        [format!("mapdifl{facet}.mul"), format!("mapdif{facet}.mul")]
    }

    /// The facet's map dif files in `dirs`, or the names of those missing.
    pub fn find(dirs: &[PathBuf], facet: u32) -> Result<Self, Vec<String>> {
        let [lookup, blocks] = find_all(dirs, Self::file_names(facet))?;
        Ok(Self { lookup, blocks })
    }

    pub fn load(&self) -> eyre::Result<MapDiff> {
        MapDiff::load(&self.lookup, &self.blocks)
            .wrap_err_with(|| format!("load {} and {}", self.lookup.display(), self.blocks.display()))
    }

    fn names(&self) -> String {
        file_names_label(&[&self.blocks, &self.lookup])
    }
}

/// A facet's `stadiflN.mul` (patched block ids), `stadifiN.mul` (their index) and `stadifN.mul` (their statics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticDifFiles {
    pub lookup: PathBuf,
    pub index: PathBuf,
    pub statics: PathBuf,
}

impl StaticDifFiles {
    pub fn file_names(facet: u32) -> [String; 3] {
        [format!("stadifl{facet}.mul"), format!("stadifi{facet}.mul"), format!("stadif{facet}.mul")]
    }

    /// The facet's static dif files in `dirs`, or the names of those missing.
    pub fn find(dirs: &[PathBuf], facet: u32) -> Result<Self, Vec<String>> {
        let [lookup, index, statics] = find_all(dirs, Self::file_names(facet))?;
        Ok(Self { lookup, index, statics })
    }

    pub fn load(&self) -> eyre::Result<StaticDiff> {
        StaticDiff::load(&self.lookup, &self.index, &self.statics).wrap_err_with(|| {
            format!("load {}, {} and {}", self.lookup.display(), self.index.display(), self.statics.display())
        })
    }

    fn names(&self) -> String {
        file_names_label(&[&self.statics, &self.lookup, &self.index])
    }
}

/// The files a Classic facet is read from, found by [`ClassicFacetSources::resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicFacetSources {
    /// The facet (map index) read.
    pub facet: u32,
    pub map: ClassicMapFile,
    /// `None` when the statics files are not there; readers of the map alone do not need them.
    pub statics: Option<StaticsFiles>,
    /// The map difs to apply.
    pub map_difs: Option<MapDifFiles>,
    /// The static difs to apply.
    pub static_difs: Option<StaticDifFiles>,
    /// The facet whose map and statics stand in for this facet's missing ones ([`SHARED_BASE_FACETS`]).
    pub base_facet: Option<u32>,
}

impl ClassicFacetSources {
    /// Finds facet `facet` in `dirs` (file names match ignoring ASCII case), trying the map containers in
    /// `formats` order. The facet's own difs are included as `difs` asks; if the facet's map is missing and it has a
    /// base facet whose map is there along with all the facet's dif files, the facet is built from the base facet's
    /// files and its difs, with a warning. Fails naming the files missing.
    pub fn resolve(
        dirs: &[PathBuf],
        facet: u32,
        formats: &[ClassicMapFormat],
        difs: ClassicDifOptions,
    ) -> eyre::Result<Self> {
        if let Some(map) = find_map_file(dirs, facet, formats) {
            return Ok(Self::with_own_difs(facet, map, find_statics_files(dirs, facet), dirs, difs));
        }

        let own_names = formats.iter().flat_map(|&format| map_file_names(facet, format)).collect::<Vec<_>>();
        let Some(base) = shared_base_facet(facet) else {
            eyre::bail!(
                "Missing map data for map{facet}: none of {} in {}",
                own_names.join(", "),
                dirs_label(dirs)
            );
        };

        let base_map = find_map_file(dirs, base, formats);
        let map_difs = MapDifFiles::find(dirs, facet);
        let static_difs = StaticDifFiles::find(dirs, facet);
        match (base_map, map_difs, static_difs) {
            (Some(map), Ok(map_difs), Ok(static_difs)) => {
                let sources = Self {
                    facet,
                    map,
                    statics: find_statics_files(dirs, base),
                    map_difs: Some(map_difs),
                    static_difs: Some(static_difs),
                    base_facet: Some(base),
                };
                if let Some(warning) = sources.base_facet_warning() {
                    warn!("{warning}");
                }
                Ok(sources)
            }
            (base_map, map_difs, static_difs) => {
                let mut missing = Vec::new();
                if base_map.is_none() {
                    missing.extend(formats.iter().filter_map(|&format| map_file_names(base, format).into_iter().next()));
                }
                missing.extend(map_difs.err().unwrap_or_default());
                missing.extend(static_difs.err().unwrap_or_default());
                eyre::bail!(
                    "Missing map data for map{facet}: none of {} in {}. Clients before 7.0 build {} from the map{base} \
                     files and the facet-{facet} difs, but {} missing as well",
                    own_names.join(", "),
                    dirs_label(dirs),
                    facet_label(facet),
                    plural_missing(&missing)
                );
            }
        }
    }

    /// The facet read from files given by name: `map` and `statics`, with the facet's own difs looked for in
    /// `dif_dirs` as `difs` asks.
    pub fn from_files(
        facet: u32,
        map: ClassicMapFile,
        statics: Option<StaticsFiles>,
        dif_dirs: &[PathBuf],
        difs: ClassicDifOptions,
    ) -> Self {
        Self::with_own_difs(facet, map, statics, dif_dirs, difs)
    }

    fn with_own_difs(
        facet: u32,
        map: ClassicMapFile,
        statics: Option<StaticsFiles>,
        dirs: &[PathBuf],
        difs: ClassicDifOptions,
    ) -> Self {
        let map_difs = difs
            .map_difs
            .then(|| requested_difs(MapDifFiles::find(dirs, facet), facet, "map", &MapDifFiles::file_names(facet)))
            .flatten();
        let static_difs = difs
            .static_difs
            .then(|| {
                requested_difs(StaticDifFiles::find(dirs, facet), facet, "static", &StaticDifFiles::file_names(facet))
            })
            .flatten();
        if let Some(map_difs) = &map_difs {
            info!("Applying the map difs of map{facet}: {}", map_difs.names());
        }
        if let Some(static_difs) = &static_difs {
            info!("Applying the static difs of map{facet}: {}", static_difs.names());
        }
        Self { facet, map, statics, map_difs, static_difs, base_facet: None }
    }

    /// The warning given when the facet is built from its base facet's files, naming them.
    pub fn base_facet_warning(&self) -> Option<String> {
        let base = self.base_facet?;
        let statics = match &self.statics {
            Some(statics) => format!(", {}", file_names_label(&[&statics.index, &statics.statics])),
            None => String::new(),
        };
        let mut difs = Vec::new();
        difs.extend(self.map_difs.as_ref().map(MapDifFiles::names));
        difs.extend(self.static_difs.as_ref().map(StaticDifFiles::names));
        Some(format!(
            "map{facet}.mul not found: building {} from {}{statics} + {}, as pre-7.0 clients do (map{base} files \
             with the facet-{facet} difs applied)",
            facet_label(self.facet),
            file_name(&self.map.path),
            difs.join(" + "),
            facet = self.facet
        ))
    }

    /// The statics files, or an error naming the missing ones.
    pub fn statics_files(&self) -> eyre::Result<&StaticsFiles> {
        let base = self.base_facet.unwrap_or(self.facet);
        self.statics.as_ref().ok_or_else(|| {
            eyre!("Missing statics for map{}: staidx{base}.mul and statics{base}.mul are needed", self.facet)
        })
    }

    /// Opens the map plane with the map difs applied, sized as the file says unless `size` overrides it.
    /// `verdata` is applied to `map0.mul` only ([`VERDATA_FACET`]), and never to a UOP map.
    pub fn load_map_plane(
        &self,
        size: Option<MapSizeCells>,
        verdata: Option<Arc<Verdata>>,
    ) -> eyre::Result<MapPlane> {
        let path = self.map.path.clone();
        let mut plane = match self.map.format {
            ClassicMapFormat::Mul => MapPlane::init_with_size(path, self.map.map_index, size)?,
            ClassicMapFormat::Uop => MapPlane::init_uop_with_size(path, self.map.map_index, size)?,
        }
        .for_facet(self.facet);
        if let Some(map_difs) = &self.map_difs {
            plane = plane.with_map_diff(map_difs.load()?);
        }
        if let Some(verdata) = verdata {
            if self.map.format == ClassicMapFormat::Uop {
                warn!("verdata.mul is not applied to the UOP map {}.", self.map.path.display());
            } else if self.map.map_index != VERDATA_FACET {
                info!("verdata.mul patches map{VERDATA_FACET}.mul only; map{} is read without it.", self.map.map_index);
            } else {
                plane = plane.with_verdata(verdata);
            }
        }
        Ok(plane)
    }

    /// Opens the statics of a `width` x `height` map with the static difs applied. `verdata` is applied to
    /// `staidx0.mul`/`statics0.mul` only ([`VERDATA_FACET`]).
    pub fn load_statics_reader(
        &self,
        width: u32,
        height: u32,
        verdata: Option<Arc<Verdata>>,
    ) -> eyre::Result<StaticsReader> {
        let files = self.statics_files()?;
        let static_diff = self.static_difs.as_ref().map(StaticDifFiles::load).transpose()?;
        let statics_map_index = self.base_facet.unwrap_or(self.facet);
        let verdata = verdata.filter(|_| {
            let applies = statics_map_index == VERDATA_FACET;
            if !applies {
                info!(
                    "verdata.mul patches staidx{VERDATA_FACET}.mul/statics{VERDATA_FACET}.mul only; \
                     statics{statics_map_index} are read without it."
                );
            }
            applies
        });
        StaticsReader::new_with_patches(&files.index, &files.statics, width, height, static_diff, verdata)
    }
}

fn find_in_dirs(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| find_file_ignoring_case(dir, name))
}

/// All of `names` in `dirs`, or the names missing.
fn find_all<const N: usize>(dirs: &[PathBuf], names: [String; N]) -> Result<[PathBuf; N], Vec<String>> {
    let found = names.each_ref().map(|name| find_in_dirs(dirs, name));
    let missing: Vec<String> =
        names.iter().zip(&found).filter(|(_, path)| path.is_none()).map(|(name, _)| name.clone()).collect();
    if missing.is_empty() { Ok(found.map(|path| path.expect("every file was found"))) } else { Err(missing) }
}

fn find_map_file(dirs: &[PathBuf], map_index: u32, formats: &[ClassicMapFormat]) -> Option<ClassicMapFile> {
    let (position, format, path) = formats.iter().enumerate().find_map(|(position, &format)| {
        map_file_names(map_index, format).iter().find_map(|name| find_in_dirs(dirs, name)).map(|path| (position, format, path))
    })?;
    if position > 0 {
        warn!(
            "map{map_index} {} source not found, falling back to {}",
            formats[0].label(),
            format.label()
        );
    }
    Some(ClassicMapFile { format, path, map_index })
}

fn find_statics_files(dirs: &[PathBuf], map_index: u32) -> Option<StaticsFiles> {
    let [index, statics] = find_all(dirs, [format!("staidx{map_index}.mul"), format!("statics{map_index}.mul")]).ok()?;
    Some(StaticsFiles { index, statics })
}

/// The dif files of a facet whose difs were asked for: a missing set is no error, and an incomplete one is not
/// applied.
fn requested_difs<T>(found: Result<T, Vec<String>>, facet: u32, kind: &str, names: &[String]) -> Option<T> {
    match found {
        Ok(files) => Some(files),
        Err(missing) if missing.len() == names.len() => {
            info!("{kind} difs asked for, but map{facet} has none ({}): reading it unpatched.", names.join(", "));
            None
        }
        Err(missing) => {
            warn!(
                "{kind} difs asked for, but map{facet} lacks {} of its {kind} dif files: reading it without them.",
                missing.join(", ")
            );
            None
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn file_names_label(paths: &[&PathBuf]) -> String {
    paths.iter().map(|path| file_name(path)).collect::<Vec<_>>().join("/")
}

fn dirs_label(dirs: &[PathBuf]) -> String {
    dirs.iter().map(|dir| format!("'{}'", dir.display())).collect::<Vec<_>>().join(", ")
}

fn plural_missing(names: &[String]) -> String {
    match names {
        [name] => format!("{name} is"),
        _ => format!("{} are", names.join(", ")),
    }
}
