//! Checks the Classic readers on real clients of every era, from UO Gold (2004) on: each file parses whole, with
//! no bytes the reader leaves unexplained, and the formats are told apart by file size and record counts.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`). The test prints one row per
//! client and file: the format detected, the record counts and the result.
//!
//! The second test checks an Enhanced Client facet written by `uocf-cli facet from-classic` against the map it was
//! built from (`UO_CLASSIC_FACET_MAP`, `UO_CLASSIC_FACET_PACKAGE`, `UO_CLASSIC_FACET_ID`): the `data.map` size and
//! the sector count come from the map file, every sector validates, and the land of the first and last sector
//! columns equals the map.
//!
//! Run them with `--ignored --release`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use color_eyre::eyre;
use rayon::prelude::*;
use uocf::classic::anim::AnimMap;
use uocf::classic::animdata::AnimData;
use uocf::classic::art::{ArtMap, LAND_DIMENSION, RGBA_BYTES_PER_PIXEL};
use uocf::classic::generic_index::IndexFile;
use uocf::classic::gump::GumpMap;
use uocf::classic::hues::{HuesMul, HUE_GROUP_SIZE};
use uocf::classic::land_texture::TexMap;
use uocf::classic::light::LightMap;
use uocf::classic::map::{MapBlock, MapBlockRelPos, MapPlane, MapSizeCells};
use uocf::classic::map_statics_diff::{MapDiff, StaticDiff};
use uocf::classic::multi::MultiMap;
use uocf::classic::radarcol::load_radarcol;
use uocf::classic::statics::{StaticTile, StaticsReader};
use uocf::classic::tiledata::TileData;
use uocf::classic::verdata::Verdata;
use uocf::enhanced::facet_decoder::decode_facet_bin_raw;
use uocf::enhanced::facet_sector::{data_map_entry_path, sector_entry_path, validate_ec_sector, FacetGeometry, SECTOR_SIDE};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
const FACET_MAP_ENV: &str = "UO_CLASSIC_FACET_MAP";
const FACET_PACKAGE_ENV: &str = "UO_CLASSIC_FACET_PACKAGE";
const FACET_ID_ENV: &str = "UO_CLASSIC_FACET_ID";

/// Classic map indices (Felucca to Ter Mur).
const MAP_INDICES: std::ops::Range<u32> = 0..6;
/// Map file suffixes: the base map and the `x` copies later clients ship.
const MAP_SUFFIXES: [&str; 2] = ["", "x"];
/// Land tile ids are below this; a map square naming a higher one was read at the wrong offset.
const LAND_TILE_COUNT: u32 = 0x4000;
/// `anim.mul` .. `anim6.mul`.
const ANIM_FILE_COUNT: u8 = 6;
const RADARCOL_ENTRY_SIZE: u64 = 2;
/// Least share of textured land tiles whose texture exists in `texidx.mul` (measured: 0.94 in UO Gold, 0.98 in
/// Mondain's Legacy, above 0.99 from 7.0.20 on).
const MIN_RESOLVED_TEXTURE_SHARE: f64 = 0.9;
/// Static art and UOP gump headers: u16 (art, after a u32 flag) or u32 (gump) width and height.
const ART_STATIC_DIMENSIONS_OFFSET: usize = 4;
const GUMP_UOP_NAMES: [&str; 3] = ["gumpartLegacyMUL.uop", "GumpartLegacyMUL.uop", "gumpartlegacymul.uop"];
/// Lights come from the Classic files only, not an Enhanced Client texture package.
const NO_EC_LIGHT_PACKAGE: Option<&Path> = None;
const LAND_TILE_RGBA_SIZE: usize = LAND_DIMENSION * LAND_DIMENSION * RGBA_BYTES_PER_PIXEL;
/// The square index of `(x, y)` in a sector: column-major.
const fn sector_square(local_x: u32, local_y: u32) -> usize {
    (local_x * SECTOR_SIDE + local_y) as usize
}

struct Row {
    client: String,
    file: String,
    format: String,
    counts: String,
    result: Result<(), String>,
}

/// What a check reports: the format detected and the record counts.
type Report = eyre::Result<(String, String)>;

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn existing(root: &Path, name: &str) -> Option<PathBuf> {
    uocf::utils::path::find_file_ignoring_case(root, name)
}

fn file_len(path: &Path) -> eyre::Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

/// Every non-empty entry of `index` lies inside a file of `data_len` bytes and is a whole number of `record`-byte
/// records. Returns the entries holding data.
fn check_index_extents(index: &IndexFile, data_len: u64, record: u32) -> eyre::Result<usize> {
    let mut used = 0;
    for element in 0..index.element_count() {
        let entry = index.element(element)?;
        let (Some(lookup), Some(len)) = (entry.lookup(), entry.len()) else {
            continue;
        };
        if len == 0 {
            continue;
        }
        eyre::ensure!(len % record == 0, "entry {element}: {len} bytes, not a whole number of {record}-byte records");
        let end = u64::from(lookup) + u64::from(len);
        eyre::ensure!(end <= data_len, "entry {element} ends at {end}, past the {data_len}-byte data file");
        used += 1;
    }
    Ok(used)
}

fn check_tiledata(path: &Path, root: &Path) -> Report {
    let tiledata = TileData::load(path.to_path_buf())?;
    let (land_size, item_size) = tiledata.record_sizes();
    let flag_bits = if tiledata.has_64_bit_flags() { u64::BITS } else { u32::BITS };
    let lands = tiledata.land_tiles();
    let textured: Vec<_> = lands.iter().filter(|land| land.texture_id != 0).collect();
    // The texture field is where the layout puts it when the textures the land tiles name exist: a wrong record
    // size would read names or flags as texture ids. Every client has stray ids (UO Gold names 96 textures that
    // its 4096-entry texidx.mul lacks), so most, not all, must resolve.
    let resolved = match existing(root, "texidx.mul") {
        Some(texidx) => {
            let index = IndexFile::load(texidx)?;
            let holds_data = |texture: u16| {
                index.element(usize::from(texture)).ok().and_then(|entry| entry.len()).is_some_and(|len| len > 0)
            };
            let resolved = textured.iter().filter(|land| holds_data(land.texture_id)).count();
            let share = resolved as f64 / textured.len().max(1) as f64;
            eyre::ensure!(
                share >= MIN_RESOLVED_TEXTURE_SHARE,
                "only {resolved} of {} land textures exist in texidx.mul",
                textured.len()
            );
            format!(", {resolved} resolve")
        }
        None => String::new(),
    };
    Ok((
        format!("{flag_bits}-bit flags, land {land_size} B, item {item_size} B"),
        format!("{} land ({} textured{resolved}), {} items", lands.len(), textured.len(), tiledata.item_tiles().len()),
    ))
}

/// Loads the first and last block columns of `plane` and checks their land ids.
fn check_map_edges(plane: &mut MapPlane) -> eyre::Result<()> {
    let (width, height) = (plane.size_blocks.width, plane.size_blocks.height);
    let mut edges: Vec<_> = [0, width - 1]
        .into_iter()
        .flat_map(|x| (0..height).map(move |y| MapBlockRelPos { x, y }))
        .collect();
    plane.load_blocks(&mut edges)?;
    for pos in edges {
        let block = plane.block_no_update(pos).ok_or_else(|| eyre::eyre!("block ({}, {}) did not load", pos.x, pos.y))?;
        if let Some(cell) = block.cells.iter().find(|cell| u32::from(cell.id) >= LAND_TILE_COUNT) {
            eyre::bail!("block ({}, {}) names land id {:#x}", pos.x, pos.y, cell.id);
        }
    }
    Ok(())
}

fn check_map(path: &Path, map_index: u32, uop: bool) -> eyre::Result<(String, String, MapSizeCells)> {
    let mut plane = if uop {
        MapPlane::init_uop(path.to_path_buf(), map_index)?
    } else {
        MapPlane::init(path.to_path_buf(), map_index)?
    };
    check_map_edges(&mut plane)?;
    let size = plane.size_cells();
    let blocks = plane.size_blocks.width * plane.size_blocks.height;
    let format = if uop {
        let entries = UopPackage::load(path)?.file_count();
        format!("UOP, {entries} entries")
    } else {
        "MUL".to_owned()
    };
    Ok((format, format!("{}x{}, {blocks} blocks", size.width, size.height), size))
}

fn check_statics(index_path: &Path, mul_path: &Path, size: MapSizeCells) -> Report {
    let reader = StaticsReader::new(index_path, mul_path, size.width, size.height)?;
    let store = reader.load_all()?;
    let used = check_index_extents(&reader.index_file, reader.mul_mmap.len() as u64, StaticTile::RAW_SIZE as u32)?;
    let blocks = (size.width / MapBlock::CELLS_PER_ROW) * (size.height / MapBlock::CELLS_PER_COLUMN);
    Ok((
        format!("{} index entries for {blocks} blocks", reader.index_file.element_count()),
        format!("{} statics in {used} blocks", store.tiles.len()),
    ))
}

fn check_multi(root: &Path) -> Report {
    let multis = MultiMap::load(root)?;
    let record_size = multis.record_size().ok_or_else(|| eyre::eyre!("entry lengths fit neither part size"))?;
    let parts = multis.load_all_parts()?;
    let used = parts.iter().filter(|parts| !parts.is_empty()).count();
    let total: usize = parts.iter().map(Vec::len).sum();
    Ok((format!("{record_size}-byte parts"), format!("{used} multis of {}, {total} parts", multis.max_id())))
}

fn check_hues(path: &Path) -> Report {
    let hues = HuesMul::load(path)?;
    eyre::ensure!(hues.trailing.is_empty(), "{} bytes after the last {HUE_GROUP_SIZE}-byte group", hues.trailing.len());
    Ok((format!("{HUE_GROUP_SIZE}-byte groups"), format!("{} groups, {} hues", hues.groups.len(), hues.hue_count())))
}

fn check_radarcol(path: &Path) -> Report {
    let colors = load_radarcol(path)?;
    let len = file_len(path)?;
    eyre::ensure!(colors.len() as u64 * RADARCOL_ENTRY_SIZE == len, "{len} bytes is not a whole number of colours");
    Ok(("u16 colours".to_owned(), format!("{} entries", colors.len())))
}

fn check_art(root: &Path) -> Report {
    let art = ArtMap::load(root)?;
    let format = if existing(root, "artidx.mul").is_some() { "MUL" } else { "UOP" };
    let max_id = art.max_id();
    let (lands, items, empty) = (0..max_id)
        .into_par_iter()
        .filter(|&id| art.has_id(id))
        .map_init(Vec::new, |scratch, id| -> eyre::Result<(u32, u32, u32)> {
            if id < LAND_TILE_COUNT {
                let mut pixels = Box::new([0u8; LAND_TILE_RGBA_SIZE]);
                art.decode_land_tile(id, scratch, &mut pixels).map_err(|error| eyre::eyre!("art {id:#x}: {error:#}"))?;
                return Ok((1, 0, 0));
            }
            match art.decode_static_tile(id, scratch) {
                Ok(_) => Ok((0, 1, 0)),
                // UO Gold and Mondain's Legacy hold a 0x0 placeholder at 0x7fff.
                Err(_) if art_static_is_empty(&art, id, scratch) => Ok((0, 0, 1)),
                Err(error) => Err(eyre::eyre!("art {id:#x}: {error:#}")),
            }
        })
        .try_reduce(|| (0, 0, 0), |a, b| Ok((a.0 + b.0, a.1 + b.1, a.2 + b.2)))?;
    Ok((format.to_owned(), format!("{lands} land, {items} items decoded of {max_id} ids; {empty} 0x0 placeholders")))
}

/// Whether static art `id` is a header with a zero width or height and no pixels.
fn art_static_is_empty(art: &ArtMap, id: u32, scratch: &mut Vec<u8>) -> bool {
    art.get_raw_art_data(id, scratch).is_ok()
        && scratch.get(ART_STATIC_DIMENSIONS_OFFSET..ART_STATIC_DIMENSIONS_OFFSET + 2 * std::mem::size_of::<u16>()).is_some_and(
            |dimensions| dimensions[..2] == [0, 0] || dimensions[2..] == [0, 0],
        )
}

/// Whether UOP gump `id` has a zero width or height.
fn uop_gump_is_empty(package: &UopPackage, id: u32) -> bool {
    let names = [format!("build/gumpartlegacymul/{id:08}.tga"), format!("build/gumpartlegacymul/{id:07}.tga")];
    let dimension = std::mem::size_of::<u32>();
    names.iter().any(|name| {
        package.unpack_file_by_hash(hash_file_name_single(name)).ok().flatten().is_some_and(|payload| {
            payload.get(..2 * dimension).is_some_and(|header| header[..dimension].iter().all(|&byte| byte == 0) || header[dimension..].iter().all(|&byte| byte == 0))
        })
    })
}

fn check_gumps(root: &Path) -> Report {
    let gumps = GumpMap::load(root)?;
    let format = if existing(root, "gumpidx.mul").is_some() { "MUL" } else { "UOP" };
    let uop = GUMP_UOP_NAMES.iter().find_map(|name| existing(root, name)).map(|path| UopPackage::load(&path)).transpose()?;
    let ids = gumps.available_gump_ids();
    let empty = ids
        .par_iter()
        .map_init(Vec::new, |scratch, &id| match gumps.decode_gump(id, scratch) {
            Ok(_) => Ok(0),
            // The UOP lists entries whose header is 0x0 (Classic: gump 34 and others).
            Err(_) if uop.as_ref().is_some_and(|package| uop_gump_is_empty(package, id)) => Ok(1),
            Err(error) => Err(eyre::eyre!("gump {id}: {error:#}")),
        })
        .try_reduce(|| 0, |a, b| Ok(a + b))?;
    Ok((format.to_owned(), format!("{} gumps decoded; {empty} 0x0 placeholders", ids.len() - empty)))
}

fn check_texmaps(root: &Path) -> Report {
    let (Some(mul), Some(idx)) = (existing(root, "texmaps.mul"), existing(root, "texidx.mul")) else {
        eyre::bail!("texidx.mul without texmaps.mul");
    };
    let textures = TexMap::load(mul, idx)?;
    let now = Instant::now();
    let mut decoded = 0;
    for id in 0..textures.len() {
        if textures.element(id).is_some() {
            textures.get_pixel_data(id, now).ok_or_else(|| eyre::eyre!("texture {id} did not decode"))?;
            decoded += 1;
        }
    }
    Ok(("MUL".to_owned(), format!("{decoded} textures decoded of {} slots", textures.len())))
}

fn check_anims(root: &Path) -> Report {
    let anims = AnimMap::load(root)?;
    let mut files = Vec::new();
    for file in 0..ANIM_FILE_COUNT {
        let Some(count) = anims.source_index_count(file) else {
            continue;
        };
        let decoded = (0..count as u32)
            .into_par_iter()
            .filter(|&id| anims.has_anim(file, id))
            .map(|id| {
                anims.decode_animation_metadata(file, id).map(|_| 1).map_err(|error| eyre::eyre!("anim{} {id}: {error:#}", file + 1))
            })
            .try_reduce(|| 0, |a, b| Ok(a + b))?;
        files.push(format!("anim{}: {decoded}/{count}", file + 1));
    }
    eyre::ensure!(!files.is_empty(), "no anim*.idx/anim*.mul pair");
    Ok(("MUL".to_owned(), files.join(", ")))
}

fn check_animdata(path: &Path) -> Report {
    let animdata = AnimData::from_bytes(&std::fs::read(path)?)?;
    let trailing = animdata.trailing_bytes.len();
    eyre::ensure!(animdata.trailing_bytes.iter().all(|&byte| byte == 0), "{trailing} trailing bytes, not all zero");
    Ok((
        format!("{} chunks, {trailing} zero trailing bytes", animdata.chunk_headers.len()),
        format!("{} active of {}", animdata.active_count(), animdata.entries.len()),
    ))
}

fn check_lights(root: &Path) -> Report {
    let lights = LightMap::load(Some(root), NO_EC_LIGHT_PACKAGE)?;
    let mut decoded = 0;
    for id in 0..lights.max_id() {
        if lights.has_id(id) {
            lights.decode_light(id)?;
            decoded += 1;
        }
    }
    Ok(("MUL".to_owned(), format!("{decoded} lights decoded of {}", lights.element_count())))
}

fn lookup_ids(path: &Path) -> eyre::Result<Vec<u32>> {
    let bytes = std::fs::read(path)?;
    let mut chunks = bytes.chunks_exact(std::mem::size_of::<u32>());
    let ids = chunks.by_ref().map(|chunk| u32::from_le_bytes(chunk.try_into().expect("u32"))).collect();
    eyre::ensure!(chunks.remainder().is_empty(), "{} bytes after the last block id", chunks.remainder().len());
    Ok(ids)
}

fn check_map_diff(lookup: &Path, diff: &Path) -> Report {
    let ids = lookup_ids(lookup)?;
    let diffs = MapDiff::load(lookup, diff)?;
    let block_size = MapBlock::PACKED_SIZE as u64;
    eyre::ensure!(file_len(diff)? == ids.len() as u64 * block_size, "mapdif holds other than {} blocks", ids.len());
    for &id in &ids {
        diffs.raw_block(id)?.ok_or_else(|| eyre::eyre!("block {id} missing"))?;
    }
    Ok(("MUL".to_owned(), format!("{} patched blocks", ids.len())))
}

fn check_static_diff(lookup: &Path, index: &Path, diff: &Path) -> Report {
    let ids = lookup_ids(lookup)?;
    let index_file = IndexFile::load(index.to_path_buf())?;
    eyre::ensure!(index_file.element_count() == ids.len(), "{} index entries for {} blocks", index_file.element_count(), ids.len());
    let used = check_index_extents(&index_file, file_len(diff)?, StaticTile::RAW_SIZE as u32)?;
    let diffs = StaticDiff::load(lookup, index, diff)?;
    for &id in &ids {
        diffs.raw_block(id)?;
    }
    Ok(("MUL".to_owned(), format!("{} patched blocks, {used} with statics", ids.len())))
}

fn check_verdata(path: &Path) -> Report {
    let verdata = Verdata::load(path)?;
    Ok(("MUL".to_owned(), format!("{} patches", verdata.len())))
}

fn check_client(root: &Path) -> Vec<Row> {
    let client = root.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let mut rows = Vec::new();
    let mut push = |file: &str, report: Report| {
        let (format, counts, result) = match report {
            Ok((format, counts)) => (format, counts, Ok(())),
            Err(error) => (String::new(), String::new(), Err(format!("{error:#}"))),
        };
        rows.push(Row { client: client.clone(), file: file.to_owned(), format, counts, result });
    };

    if let Some(path) = existing(root, "tiledata.mul") {
        push("tiledata.mul", check_tiledata(&path, root));
    }
    for map_index in MAP_INDICES {
        for suffix in MAP_SUFFIXES {
            let mul = format!("map{map_index}{suffix}.mul");
            let uop = format!("map{map_index}{suffix}LegacyMUL.uop");
            let mut size = None;
            for (name, is_uop) in [(&mul, false), (&uop, true)] {
                if let Some(path) = existing(root, name) {
                    match check_map(&path, map_index, is_uop) {
                        Ok((format, counts, map_size)) => {
                            size.get_or_insert(map_size);
                            push(name, Ok((format, counts)));
                        }
                        Err(error) => push(name, Err(error)),
                    }
                }
            }
            let index_name = format!("staidx{map_index}{suffix}.mul");
            let mul_name = format!("statics{map_index}{suffix}.mul");
            if let (Some(index), Some(statics)) = (existing(root, &index_name), existing(root, &mul_name)) {
                let report = match size {
                    Some(size) => check_statics(&index, &statics, size),
                    None => Err(eyre::eyre!("no map{map_index}{suffix} to size it")),
                };
                push(&index_name, report);
            }
            if suffix.is_empty() {
                if let (Some(lookup), Some(diff)) =
                    (existing(root, &format!("mapdifl{map_index}.mul")), existing(root, &format!("mapdif{map_index}.mul")))
                {
                    push(&format!("mapdif{map_index}.mul"), check_map_diff(&lookup, &diff));
                }
                if let (Some(lookup), Some(index), Some(diff)) = (
                    existing(root, &format!("stadifl{map_index}.mul")),
                    existing(root, &format!("stadifi{map_index}.mul")),
                    existing(root, &format!("stadif{map_index}.mul")),
                ) {
                    push(&format!("stadif{map_index}.mul"), check_static_diff(&lookup, &index, &diff));
                }
            }
        }
    }
    if existing(root, "multi.mul").is_some() {
        push("multi.mul", check_multi(root));
    }
    if let Some(path) = existing(root, "hues.mul") {
        push("hues.mul", check_hues(&path));
    }
    if let Some(path) = existing(root, "radarcol.mul") {
        push("radarcol.mul", check_radarcol(&path));
    }
    push("art", check_art(root));
    push("gumps", check_gumps(root));
    if existing(root, "texidx.mul").is_some() {
        push("texmaps.mul", check_texmaps(root));
    }
    push("anim*.mul", check_anims(root));
    if let Some(path) = existing(root, "animdata.mul") {
        push("animdata.mul", check_animdata(&path));
    }
    if existing(root, "lightidx.mul").is_some() {
        push("light.mul", check_lights(root));
    }
    match existing(root, "verdata.mul") {
        Some(path) => push("verdata.mul", check_verdata(&path)),
        None => push("verdata.mul", Ok(("absent".to_owned(), String::new()))),
    }
    rows
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored --release"]
fn classic_client_files_parse_fully() {
    let rows: Vec<Row> = client_roots().iter().flat_map(|root| check_client(root)).collect();
    let width = |column: fn(&Row) -> &str| rows.iter().map(|row| column(row).len()).max().unwrap_or(0);
    let (client_width, file_width, format_width) =
        (width(|row| &row.client), width(|row| &row.file), width(|row| &row.format));
    for row in &rows {
        let result = match &row.result {
            Ok(()) => "ok".to_owned(),
            Err(error) => format!("FAILED: {error}"),
        };
        println!(
            "{:client_width$} | {:file_width$} | {:format_width$} | {} | {result}",
            row.client, row.file, row.format, row.counts
        );
    }
    let failures = rows.iter().filter(|row| row.result.is_err()).count();
    assert_eq!(failures, 0, "{failures} files failed");
}

#[test]
#[ignore = "needs a facet built by `uocf-cli facet from-classic`: set UO_CLASSIC_FACET_MAP, UO_CLASSIC_FACET_PACKAGE and UO_CLASSIC_FACET_ID and run with --ignored --release"]
fn facet_built_from_classic_matches_its_map() {
    let var = |name: &str| std::env::var_os(name).unwrap_or_else(|| panic!("set {name}"));
    let map_path = PathBuf::from(var(FACET_MAP_ENV));
    let package_path = PathBuf::from(var(FACET_PACKAGE_ENV));
    let facet: u8 = var(FACET_ID_ENV).to_string_lossy().parse().expect("facet id");

    let is_uop = map_path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("uop"));
    let mut plane = if is_uop {
        MapPlane::init_uop(map_path.clone(), u32::from(facet))
    } else {
        MapPlane::init(map_path.clone(), u32::from(facet))
    }
    .expect("open map");
    let size = plane.size_cells();

    let package = UopPackage::load(&package_path).expect("load facet package");
    let data_map = package
        .unpack_file_by_hash(hash_file_name_single(&data_map_entry_path(facet)))
        .expect("unpack data.map")
        .expect("data.map entry");
    let geometry = FacetGeometry::from_data_map_payload(&data_map).expect("data.map");
    assert_eq!((geometry.width, geometry.height), (size.width, size.height), "data.map size");
    let sector_count = size.width.div_ceil(SECTOR_SIDE) * size.height.div_ceil(SECTOR_SIDE);
    assert_eq!(geometry.sector_count(), sector_count);
    let beyond = sector_entry_path(facet, sector_count);
    assert!(package.get_file_by_name(&beyond).is_none(), "{beyond} past the last sector");

    let decode = |sector: u32| {
        let path = sector_entry_path(facet, sector);
        let payload = package.unpack_file_by_hash(hash_file_name_single(&path)).expect("unpack").expect(&path);
        decode_facet_bin_raw(&payload).unwrap_or_else(|error| panic!("{path}: {error:#}"))
    };
    let statics: usize = (0..sector_count)
        .into_par_iter()
        .map(|sector| {
            let decoded = decode(sector);
            let issues = validate_ec_sector(&decoded, Some((facet, sector)));
            assert!(issues.is_empty(), "sector {sector}: {issues:?}");
            decoded.tiles.iter().map(|tile| tile.statics.len()).sum::<usize>()
        })
        .sum();

    // The land of the first and last sector columns against the map, which checks the far edge.
    check_map_edges(&mut plane).expect("map edges");
    let block_side = MapBlock::CELLS_PER_ROW;
    let mut compared = 0u64;
    for sector_x in [0, geometry.sector_columns() - 1] {
        for sector_y in 0..geometry.sector_rows() {
            let sector = sector_x * geometry.sector_rows() + sector_y;
            let (origin_x, origin_y) = geometry.sector_origin(sector);
            assert_eq!((origin_x, origin_y), (sector_x * SECTOR_SIDE, sector_y * SECTOR_SIDE), "sector order");
            let decoded = decode(sector);
            let mut blocks: Vec<_> = (origin_x / block_side..(origin_x + SECTOR_SIDE).min(size.width) / block_side)
                .flat_map(|x| {
                    (origin_y / block_side..(origin_y + SECTOR_SIDE).min(size.height) / block_side)
                        .map(move |y| MapBlockRelPos { x, y })
                })
                .collect();
            plane.load_blocks(&mut blocks).expect("load blocks");
            for pos in blocks {
                let block = plane.block_no_update(pos).expect("loaded block");
                for (index, cell) in block.cells.iter().enumerate() {
                    let index = index as u32;
                    let (x, y) = (pos.x * block_side + index % block_side, pos.y * block_side + index / block_side);
                    let tile = &decoded.tiles[sector_square(x - origin_x, y - origin_y)];
                    assert_eq!((tile.land_graphic, tile.z), (cell.id, cell.z), "square ({x}, {y})");
                    compared += 1;
                }
            }
        }
    }
    println!(
        "{}: facet {facet}, {}x{}, {sector_count} sectors valid, {statics} statics; {compared} squares of the edge \
         sector columns equal {}",
        package_path.display(),
        geometry.width,
        geometry.height,
        map_path.display()
    );
}
