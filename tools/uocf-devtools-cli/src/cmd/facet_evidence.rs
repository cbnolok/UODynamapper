use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{self, Context};
use uocf::classic::map::{MapBlock, MapBlockRelPos, MapPlane, MapSizeCells};
use uocf::enhanced::facet_sector::{square_index, FacetGeometry, SECTOR_SIDE};
use uocf::enhanced::terrain_definition::TerrainDefinitionPackage;
use uocf::uop_container::package::UopPackage;

#[derive(ClapArgs, Debug)]
#[command(about = "Extract facet terrain evidence into KDL.")]
pub struct Args {
    #[arg(long, value_enum)]
    client: ClientKind,
    #[arg(long)]
    facet: Option<PathBuf>,
    #[arg(long)]
    map_index: Option<u8>,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    cc_map: Option<PathBuf>,
    #[arg(long)]
    kr_dictionary_csv: Option<PathBuf>,
    #[arg(long)]
    terrain_definition: Option<PathBuf>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ClientKind {
    Ec,
    Kr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TranscodeKey {
    cc_land_id: u16,
    client_land_id: u32,
    unknown: Option<u8>,
    original_byte_0: Option<u8>,
    original_byte_1: Option<u8>,
}

#[derive(Default)]
struct Evidence {
    transcodes: BTreeMap<TranscodeKey, u64>,
}

pub fn run(args: Args) -> eyre::Result<()> {
    let size = match args.map_index {
        Some(map_index) => Some(map_size(map_index, args.width, args.height)?),
        None => None,
    };
    let mut output = String::new();

    output.push_str("// Generated facet terrain evidence. Do not hand-edit as authoritative data.\n");
    output.push_str(&format!(
        "source client={}",
        kdl_quote(match args.client {
            ClientKind::Ec => "ec",
            ClientKind::Kr => "kr",
        })
    ));
    if let Some(map_index) = args.map_index {
        output.push_str(&format!(" map_index={map_index}"));
    }
    if let Some(facet) = args.facet.as_ref() {
        output.push_str(&format!(" facet={}", kdl_quote_path(facet)));
    }
    if let Some(size) = size {
        output.push_str(&format!(" width={} height={}", size.width, size.height));
    }
    if let Some(cc_map) = args.cc_map.as_ref() {
        output.push_str(&format!(" cc_map={}", kdl_quote_path(cc_map)));
    }
    if let Some(path) = args.kr_dictionary_csv.as_ref() {
        output.push_str(&format!(" kr_dictionary_csv={}", kdl_quote_path(path)));
    }
    if let Some(path) = args.terrain_definition.as_ref() {
        output.push_str(&format!(" terrain_definition={}", kdl_quote_path(path)));
    }
    output.push('\n');

    let mut evidence = Evidence::default();
    if let Some(path) = args.kr_dictionary_csv.as_ref() {
        if !matches!(args.client, ClientKind::Kr) {
            return Err(eyre::eyre!("--kr-dictionary-csv can only be used with --client kr"));
        }
        merge_evidence(&mut evidence, extract_kr_dictionary_csv(path)?);
    }
    if args.facet.is_some() {
        let map_index = args
            .map_index
            .ok_or_else(|| eyre::eyre!("--map-index is required when --facet is used"))?;
        let size = size.expect("size is computed when map_index is present");
        merge_evidence(
            &mut evidence,
            match args.client {
                ClientKind::Ec => extract_ec_evidence(&args, map_index, size)?,
                ClientKind::Kr => extract_kr_evidence(&args, map_index, size)?,
            },
        );
    }
    if evidence.transcodes.is_empty() {
        return Err(eyre::eyre!(
            "no evidence source selected; use --facet and/or --kr-dictionary-csv"
        ));
    }
    write_transcodes(&mut output, args.client, &evidence);

    if let Some(path) = args.terrain_definition.as_ref() {
        match TerrainDefinitionPackage::load(path) {
            Ok(terrain) => write_terrain_definition(&mut output, &terrain),
            Err(err) => {
                log::warn!(
                    "Skipping terrain definition evidence from {}: {err:?}",
                    path.display()
                );
                output.push_str(&format!(
                    "terrain_definition_unparsed path={} reason={}\n",
                    kdl_quote_path(path),
                    kdl_quote(&err.to_string())
                ));
            }
        }
    }

    fs::write(&args.output, output)
        .wrap_err_with(|| format!("write {}", args.output.display()))?;
    log::info!("Wrote facet evidence to {}", args.output.display());

    Ok(())
}

fn extract_ec_evidence(args: &Args, map_index: u8, size: MapSizeCells) -> eyre::Result<Evidence> {
    let cc_map = args
        .cc_map
        .as_ref()
        .ok_or_else(|| eyre::eyre!("--cc-map is required for EC facet transcode extraction"))?;
    let mut cc_plane = load_cc_plane(cc_map, map_index, size)?;
    let facet = args.facet.as_ref().expect("checked by caller");
    let package = UopPackage::load(facet)
        .wrap_err_with(|| format!("load {}", facet.display()))?;
    let mut evidence = Evidence::default();
    // EC sectors are numbered column-major over ceil(w / 64) x ceil(h / 64), with the squares past the map edge
    // padded (Tokuno's 1448 squares end inside a sector).
    let geometry = FacetGeometry::new(size.width, size.height);

    for block_id in 0..geometry.sector_count() {
        let (origin_x, origin_y) = geometry.sector_origin(block_id);
        load_cc_chunk_blocks_at(&mut cc_plane, origin_x, origin_y, size)?;
        let facet = uocf::enhanced::facet_decoder::read_facet_block_raw(
            &package,
            map_index,
            block_id,
        )
        .wrap_err_with(|| format!("decode EC facet block {block_id}"))?;

        for x in 0..SECTOR_SIDE {
            for y in 0..SECTOR_SIDE {
                let global_x = origin_x + x;
                let global_y = origin_y + y;
                if !geometry.contains(global_x, global_y) {
                    continue;
                }
                let cc_land_id = cc_land_id_at(&mut cc_plane, global_x, global_y)?;
                let tile = &facet.tiles[square_index(x, y)];
                let key = TranscodeKey {
                    cc_land_id,
                    client_land_id: tile.land_graphic as u32,
                    unknown: None,
                    original_byte_0: None,
                    original_byte_1: None,
                };
                *evidence.transcodes.entry(key).or_default() += 1;
            }
        }
    }

    Ok(evidence)
}

fn extract_kr_evidence(args: &Args, map_index: u8, size: MapSizeCells) -> eyre::Result<Evidence> {
    let facet = args.facet.as_ref().expect("checked by caller");
    let package = UopPackage::load(facet)
        .wrap_err_with(|| format!("load {}", facet.display()))?;
    let mut cc_plane = args
        .cc_map
        .as_ref()
        .map(|cc_map| load_cc_plane(cc_map, map_index, size))
        .transpose()?;
    let mut evidence = Evidence::default();
    // KR sectors are numbered and padded like EC ones (column-major over the ceil size).
    let geometry = FacetGeometry::new(size.width, size.height);

    for block_id in 0..geometry.sector_count() {
        let (origin_x, origin_y) = geometry.sector_origin(block_id);
        if let Some(plane) = cc_plane.as_mut() {
            load_cc_chunk_blocks_at(plane, origin_x, origin_y, size)?;
        }
        let facet = uocf::kr::facet::facet_decoder::read_facet_block_raw(
            &package,
            map_index,
            block_id,
        )
        .wrap_err_with(|| format!("decode KR facet block {block_id}"))?;

        for x in 0..SECTOR_SIDE {
            for y in 0..SECTOR_SIDE {
                let global_x = origin_x + x;
                let global_y = origin_y + y;
                if !geometry.contains(global_x, global_y) {
                    continue;
                }
                let tile = &facet.tiles[square_index(x, y)];
                let kr = tile.kr.ok_or_else(|| eyre::eyre!("KR facet block {block_id} has an EC square"))?;
                let original_id = match cc_plane.as_mut() {
                    Some(plane) => cc_land_id_at(plane, global_x, global_y)?,
                    None => kr.classic_land,
                };
                // The stored Classic id, a little-endian u16.
                let [original_byte_0, original_byte_1] = kr.classic_land.to_le_bytes();
                let key = TranscodeKey {
                    cc_land_id: original_id,
                    client_land_id: tile.land_graphic as u32,
                    unknown: Some(kr.subtype),
                    original_byte_0: Some(original_byte_0),
                    original_byte_1: Some(original_byte_1),
                };
                *evidence.transcodes.entry(key).or_default() += 1;
            }
        }
    }

    Ok(evidence)
}

fn extract_kr_dictionary_csv(path: &Path) -> eyre::Result<Evidence> {
    let mut reader = csv::Reader::from_path(path)
        .wrap_err_with(|| format!("read {}", path.display()))?;
    let headers = reader.headers()?.clone();
    let cc_index = headers
        .iter()
        .position(|header| header == "CC_ID")
        .ok_or_else(|| eyre::eyre!("{} is missing CC_ID column", path.display()))?;
    let kr_index = headers
        .iter()
        .position(|header| header == "KR_ID" || header == "EC_ID")
        .ok_or_else(|| eyre::eyre!("{} is missing KR_ID or EC_ID column", path.display()))?;
    let mut evidence = Evidence::default();

    for record in reader.records() {
        let record = record?;
        let cc_land_id = record
            .get(cc_index)
            .ok_or_else(|| eyre::eyre!("missing CC_ID value"))?
            .parse::<u16>()?;
        let client_land_id = record
            .get(kr_index)
            .ok_or_else(|| eyre::eyre!("missing KR_ID/EC_ID value"))?
            .parse::<u32>()?;
        let key = TranscodeKey {
            cc_land_id,
            client_land_id,
            unknown: None,
            original_byte_0: None,
            original_byte_1: None,
        };
        *evidence.transcodes.entry(key).or_default() += 1;
    }

    Ok(evidence)
}

fn merge_evidence(target: &mut Evidence, source: Evidence) {
    for (key, count) in source.transcodes {
        *target.transcodes.entry(key).or_default() += count;
    }
}

fn write_transcodes(output: &mut String, client: ClientKind, evidence: &Evidence) {
    let target_name = match client {
        ClientKind::Ec => "ec",
        ClientKind::Kr => "kr",
    };

    output.push_str("facet_transcodes {\n");
    for (key, count) in &evidence.transcodes {
        output.push_str(&format!(
            "    t cc={} {}={} count={}",
            key.cc_land_id,
            target_name,
            key.client_land_id,
            count
        ));
        if let Some(unknown) = key.unknown {
            output.push_str(&format!(" unknown={unknown}"));
        }
        if let (Some(byte_0), Some(byte_1)) = (key.original_byte_0, key.original_byte_1) {
            output.push_str(&format!(" original_byte_0={byte_0} original_byte_1={byte_1}"));
        }
        output.push('\n');
    }
    output.push_str("}\n");
}

fn write_terrain_definition(output: &mut String, terrain: &TerrainDefinitionPackage) {
    output.push_str("terrain_definition {\n");
    for entry in &terrain.entries {
        output.push_str(&format!("    material id={}", entry.id));
        if let Some(name) = entry.name.as_deref() {
            output.push_str(&format!(" name={}", kdl_quote(name)));
        }
        if let Some(primary) = entry.primary_texture_id() {
            output.push_str(&format!(" primary_texture_id={primary}"));
        }
        output.push_str(" {\n");

        for alias in &entry.aliases {
            output.push_str(&format!(
                "        alias id={} count_index={} flags={}\n",
                alias.legacy_land_id, alias.subtype, alias.tile_flags
            ));
        }

        if let Some(texture) = entry.texture.as_ref() {
            if let Some(shader) = texture.shader_name.as_deref() {
                output.push_str(&format!("        shader name={}\n", kdl_quote(shader)));
            }
            for layer in &texture.layers {
                output.push_str("        layer");
                if let Some(texture_id) = layer.texture_id {
                    output.push_str(&format!(" texture_id={texture_id}"));
                }
                if let Some(path) = layer.path.as_deref() {
                    output.push_str(&format!(" path={}", kdl_quote(path)));
                }
                output.push_str(&format!(" repetition={}", kdl_float(layer.texture_repetition)));
                output.push_str(&format!(" type={}", kdl_quote(&format!("{:?}", layer.texture_type))));
                output.push('\n');
            }
        }

        output.push_str("    }\n");
    }
    output.push_str("}\n");
}

fn load_cc_plane(path: &Path, map_index: u8, size: MapSizeCells) -> eyre::Result<MapPlane> {
    if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("uop")) {
        MapPlane::init_uop_with_size(path.to_path_buf(), map_index as u32, Some(size))
            .wrap_err_with(|| format!("load {}", path.display()))
    } else {
        MapPlane::init_with_size(path.to_path_buf(), map_index as u32, Some(size))
            .wrap_err_with(|| format!("load {}", path.display()))
    }
}

/// Loads the Classic blocks of the EC or KR sector at square `(origin_x, origin_y)` that lie on the map.
fn load_cc_chunk_blocks_at(
    plane: &mut MapPlane,
    origin_x: u32,
    origin_y: u32,
    size: MapSizeCells,
) -> eyre::Result<()> {
    let cells_per_block = MapBlock::CELLS_PER_ROW;
    let blocks_per_side = SECTOR_SIDE / cells_per_block;
    let (first_x, first_y) = (origin_x / cells_per_block, origin_y / cells_per_block);
    let end_x = (first_x + blocks_per_side).min(size.width.div_ceil(cells_per_block));
    let end_y = (first_y + blocks_per_side).min(size.height.div_ceil(cells_per_block));
    let mut blocks: Vec<MapBlockRelPos> = (first_x..end_x)
        .flat_map(|x| (first_y..end_y).map(move |y| MapBlockRelPos { x, y }))
        .collect();
    plane.load_blocks(&mut blocks)
}

fn cc_land_id_at(plane: &mut MapPlane, x: u32, y: u32) -> eyre::Result<u16> {
    let block_pos = MapBlockRelPos { x: x / 8, y: y / 8 };
    let cell_x = x % 8;
    let cell_y = y % 8;
    plane
        .block(block_pos)
        .and_then(|block| block.cell(cell_x, cell_y).ok())
        .map(|cell| cell.id)
        .ok_or_else(|| eyre::eyre!("missing CC map cell at {x},{y}"))
}

fn map_size(map_index: u8, width: Option<u32>, height: Option<u32>) -> eyre::Result<MapSizeCells> {
    match (width, height) {
        (Some(width), Some(height)) => Ok(MapSizeCells { width, height }),
        (None, None) => match map_index {
            0 | 1 => Ok(MapSizeCells { width: 7168, height: 4096 }),
            2 => Ok(MapSizeCells { width: 2304, height: 1600 }),
            3 => Ok(MapSizeCells { width: 2560, height: 2048 }),
            4 => Ok(MapSizeCells { width: 1448, height: 1448 }),
            5 => Ok(MapSizeCells { width: 1280, height: 4096 }),
            _ => Err(eyre::eyre!("unknown default size for map index {map_index}")),
        },
        _ => Err(eyre::eyre!("--width and --height must be provided together")),
    }
}

fn kdl_quote_path(path: &Path) -> String {
    kdl_quote(&path.display().to_string())
}

fn kdl_quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

fn kdl_float(value: f32) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        kdl_quote(&value.to_string())
    }
}
