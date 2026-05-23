use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, ValueEnum};
use color_eyre::eyre::{self, Context};
use uocf::classic::map::{MapBlockRelPos, MapPlane, MapSizeCells};
use uocf::enhanced::terrain_definition::TerrainDefinitionPackage;
use uocf::uop_container::package::UopPackage;

#[derive(Parser, Debug)]
#[command(author, version, about = "Extract facet terrain evidence into KDL.")]
struct Cli {
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

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    let cli = Cli::parse();
    let size = match cli.map_index {
        Some(map_index) => Some(map_size(map_index, cli.width, cli.height)?),
        None => None,
    };
    let mut output = String::new();

    output.push_str("// Generated facet terrain evidence. Do not hand-edit as authoritative data.\n");
    output.push_str(&format!(
        "source client={}",
        kdl_quote(match cli.client {
            ClientKind::Ec => "ec",
            ClientKind::Kr => "kr",
        })
    ));
    if let Some(map_index) = cli.map_index {
        output.push_str(&format!(" map_index={map_index}"));
    }
    if let Some(facet) = cli.facet.as_ref() {
        output.push_str(&format!(" facet={}", kdl_quote_path(facet)));
    }
    if let Some(size) = size {
        output.push_str(&format!(" width={} height={}", size.width, size.height));
    }
    if let Some(cc_map) = cli.cc_map.as_ref() {
        output.push_str(&format!(" cc_map={}", kdl_quote_path(cc_map)));
    }
    if let Some(path) = cli.kr_dictionary_csv.as_ref() {
        output.push_str(&format!(" kr_dictionary_csv={}", kdl_quote_path(path)));
    }
    if let Some(path) = cli.terrain_definition.as_ref() {
        output.push_str(&format!(" terrain_definition={}", kdl_quote_path(path)));
    }
    output.push('\n');

    let mut evidence = Evidence::default();
    if let Some(path) = cli.kr_dictionary_csv.as_ref() {
        if !matches!(cli.client, ClientKind::Kr) {
            return Err(eyre::eyre!("--kr-dictionary-csv can only be used with --client kr"));
        }
        merge_evidence(&mut evidence, extract_kr_dictionary_csv(path)?);
    }
    if cli.facet.is_some() {
        let map_index = cli
            .map_index
            .ok_or_else(|| eyre::eyre!("--map-index is required when --facet is used"))?;
        let size = size.expect("size is computed when map_index is present");
        merge_evidence(
            &mut evidence,
            match cli.client {
                ClientKind::Ec => extract_ec_evidence(&cli, map_index, size)?,
                ClientKind::Kr => extract_kr_evidence(&cli, map_index, size)?,
            },
        );
    }
    if evidence.transcodes.is_empty() {
        return Err(eyre::eyre!(
            "no evidence source selected; use --facet and/or --kr-dictionary-csv"
        ));
    }
    write_transcodes(&mut output, cli.client, &evidence);

    if let Some(path) = cli.terrain_definition.as_ref() {
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

    fs::write(&cli.output, output)
        .wrap_err_with(|| format!("write {}", cli.output.display()))?;
    println!("Wrote facet evidence to {}", cli.output.display());

    Ok(())
}

fn extract_ec_evidence(cli: &Cli, map_index: u8, size: MapSizeCells) -> eyre::Result<Evidence> {
    let cc_map = cli
        .cc_map
        .as_ref()
        .ok_or_else(|| eyre::eyre!("--cc-map is required for EC facet transcode extraction"))?;
    let mut cc_plane = load_cc_plane(cc_map, map_index, size)?;
    let facet = cli.facet.as_ref().expect("checked by caller");
    let package = UopPackage::load(facet)
        .wrap_err_with(|| format!("load {}", facet.display()))?;
    let mut evidence = Evidence::default();

    for block_id in 0..facet_block_count(size) {
        load_cc_chunk_blocks(&mut cc_plane, block_id, size)?;
        let facet = uocf::enhanced::facet_decoder::read_facet_block_raw(
            &package,
            map_index,
            block_id,
        )
        .wrap_err_with(|| format!("decode EC facet block {block_id}"))?;
        let (chunk_x, chunk_y) = facet_chunk_coords(block_id, size);

        for x in 0..64_u32 {
            for y in 0..64_u32 {
                let global_x = chunk_x * 64 + x;
                let global_y = chunk_y * 64 + y;
                let cc_land_id = cc_land_id_at(&mut cc_plane, global_x, global_y)?;
                let tile = &facet.tiles[(x * 64 + y) as usize];
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

fn extract_kr_evidence(cli: &Cli, map_index: u8, size: MapSizeCells) -> eyre::Result<Evidence> {
    let facet = cli.facet.as_ref().expect("checked by caller");
    let package = UopPackage::load(facet)
        .wrap_err_with(|| format!("load {}", facet.display()))?;
    let mut cc_plane = cli
        .cc_map
        .as_ref()
        .map(|cc_map| load_cc_plane(cc_map, map_index, size))
        .transpose()?;
    let mut evidence = Evidence::default();

    for block_id in 0..facet_block_count(size) {
        if let Some(plane) = cc_plane.as_mut() {
            load_cc_chunk_blocks(plane, block_id, size)?;
        }
        let facet = uocf::kr::facet::facet_decoder::read_facet_block_raw(
            &package,
            map_index,
            block_id,
        )
        .wrap_err_with(|| format!("decode KR facet block {block_id}"))?;

        let (chunk_x, chunk_y) = facet_chunk_coords(block_id, size);
        for (x, column) in facet.tiles.iter().enumerate() {
            for (y, tile) in column.iter().enumerate() {
                let original_id = if let Some(plane) = cc_plane.as_mut() {
                    cc_land_id_at(
                        plane,
                        chunk_x * 64 + x as u32,
                        chunk_y * 64 + y as u32,
                    )?
                } else {
                    u16::from_be_bytes([
                        tile.original_id_low,
                        tile.original_id_high,
                    ])
                };
                let key = TranscodeKey {
                    cc_land_id: original_id,
                    client_land_id: tile.land_graphic as u32,
                    unknown: Some(tile.unknown_byte),
                    original_byte_0: Some(tile.original_id_low),
                    original_byte_1: Some(tile.original_id_high),
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
            "    transcode cc={} {}={} count={}",
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
                alias.alias, alias.count_index, alias.tile_flags
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

fn load_cc_chunk_blocks(
    plane: &mut MapPlane,
    block_id: u32,
    size: MapSizeCells,
) -> eyre::Result<()> {
    let (chunk_x, chunk_y) = facet_chunk_coords(block_id, size);
    let mut blocks = Vec::with_capacity(64);
    for x in 0..8 {
        for y in 0..8 {
            blocks.push(MapBlockRelPos {
                x: chunk_x * 8 + x,
                y: chunk_y * 8 + y,
            });
        }
    }
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

fn facet_chunk_coords(block_id: u32, size: MapSizeCells) -> (u32, u32) {
    let chunks_y = size.height / 64;
    (block_id / chunks_y, block_id % chunks_y)
}

fn facet_block_count(size: MapSizeCells) -> u32 {
    (size.width / 64) * (size.height / 64)
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
