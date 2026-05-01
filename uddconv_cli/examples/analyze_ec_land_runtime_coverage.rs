use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use uddconv::ec_land::{
    EcLandPackage, EcLandRuntimeMaterialIdOverride, EcLandTerrainProvenanceRecord,
};
use uocf::enhanced::facet::read_facet_block;
use uocf::uop::package::UopPackage;

const DEFAULT_TOP_N: usize = 25;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut args = env::args_os().skip(1);
    let package_path = PathBuf::from(args.next().ok_or_else(|| {
        eyre::eyre!(
            "usage: analyze_ec_land_runtime_coverage <ec_land.uddp> <ecdir> [map_index] [top_n]"
        )
    })?);
    let ecdir = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing <ecdir> argument"))?,
    );
    let map_index = parse_optional_u32(args.next(), 0, "map_index")?;
    let top_n = parse_optional_usize(args.next(), DEFAULT_TOP_N, "top_n")?;

    let package = EcLandPackage::load(&package_path)
        .wrap_err_with(|| format!("load {}", package_path.display()))?;
    let facet_path = facet_uop_path(&ecdir, map_index);
    let facet_package = UopPackage::load(&facet_path)
        .wrap_err_with(|| format!("load {}", facet_path.display()))?;
    let facet_block_count = facet_block_count(map_index)?;

    let mut total_cells = 0u64;
    let mut resolved_cells = 0u64;
    let mut direct_slot_cells = 0u64;
    let mut unresolved_by_id = HashMap::<u32, u64>::new();
    let mut resolved_by_id = HashMap::<u32, u64>::new();

    for block_id in 0..facet_block_count {
        let decoded = read_facet_block(&facet_package, map_index as u8, block_id).wrap_err_with(|| {
            format!(
                "decode {} block {}",
                facet_path.display(),
                block_id
            )
        })?;
        process_decoded_blocks(
            &package,
            decoded.blocks.iter().map(|decoded| &decoded.block.cells),
            &mut total_cells,
            &mut resolved_cells,
            &mut direct_slot_cells,
            &mut resolved_by_id,
            &mut unresolved_by_id,
        )?;
    }

    let unresolved_cells = total_cells.saturating_sub(resolved_cells);
    println!("package={}", package_path.display());
    println!("facet={}", facet_path.display());
    println!("map_index={}", map_index);
    println!("facet_block_count={}", facet_block_count);
    println!("total_cells={}", total_cells);
    println!("resolved_cells={}", resolved_cells);
    println!("unresolved_cells={}", unresolved_cells);
    println!("direct_slot_cells={}", direct_slot_cells);
    println!("resolved_ratio={:.4}", resolved_cells as f64 / total_cells as f64);

    println!("top_unresolved_ids:");
    for (terrain_id, count) in top_counts(&unresolved_by_id, top_n) {
        print_unresolved_detail(&package, terrain_id, count);
    }

    println!("top_resolved_non_direct_ids:");
    let resolved_non_direct = resolved_by_id
        .into_iter()
        .filter(|(terrain_id, _)| package.present_slot(*terrain_id).is_none())
        .collect::<HashMap<_, _>>();
    for (terrain_id, count) in top_counts(&resolved_non_direct, top_n) {
        let override_record = find_override(package.runtime_material_id_overrides(), terrain_id);
        println!(
            "  terrain_id={} count={} override={:?} material_rows={} alias_rows={}",
            terrain_id,
            count,
            override_record.map(|record| record.normalized_material_id),
            package
                .terrain_provenance()
                .iter()
                .filter(|record| record.material_id == terrain_id)
                .count(),
            package
                .terrain_provenance()
                .iter()
                .filter(|record| record.alias_slot_id == terrain_id)
                .count(),
        );
    }

    Ok(())
}

fn process_decoded_blocks<'a>(
    package: &EcLandPackage,
    blocks: impl Iterator<Item = &'a [uocf::classic::map::MapCell; 64]>,
    total_cells: &mut u64,
    resolved_cells: &mut u64,
    direct_slot_cells: &mut u64,
    resolved_by_id: &mut HashMap<u32, u64>,
    unresolved_by_id: &mut HashMap<u32, u64>,
) -> eyre::Result<()> {
    for block_cells in blocks {
        for cell in block_cells {
            let terrain_id = cell.id as u32;
            *total_cells += 1;
            if package.resolve_runtime_slot_id(terrain_id).is_some() {
                *resolved_cells += 1;
                *resolved_by_id.entry(terrain_id).or_default() += 1;
                if package.present_slot(terrain_id).is_some() {
                    *direct_slot_cells += 1;
                }
            } else {
                *unresolved_by_id.entry(terrain_id).or_default() += 1;
            }
        }
    }
    Ok(())
}

fn print_unresolved_detail(package: &EcLandPackage, terrain_id: u32, count: u64) {
    let override_record = find_override(package.runtime_material_id_overrides(), terrain_id);
    let material_rows = package
        .terrain_provenance()
        .iter()
        .filter(|record| record.material_id == terrain_id)
        .collect::<Vec<_>>();
    let alias_rows = package
        .terrain_provenance()
        .iter()
        .filter(|record| record.alias_slot_id == terrain_id)
        .collect::<Vec<_>>();

    println!(
        "  terrain_id={} count={} override={:?} material_rows={} alias_rows={}",
        terrain_id,
        count,
        override_record.map(|record| record.normalized_material_id),
        material_rows.len(),
        alias_rows.len(),
    );

    for record in material_rows.iter().take(3) {
        print_provenance("material", record);
    }
    for record in alias_rows.iter().take(3) {
        print_provenance("alias", record);
    }
}

fn print_provenance(kind: &str, record: &EcLandTerrainProvenanceRecord) {
    println!(
        "    {} material_id={} alias_index={} alias_slot={} selected_texture={} canonical_slot={} tile_flags={}",
        kind,
        record.material_id,
        record.alias_count_index,
        record.alias_slot_id,
        record.selected_texture_id,
        record.canonical_slot_id,
        record.alias_tile_flags,
    );
}

fn top_counts(counts: &HashMap<u32, u64>, top_n: usize) -> Vec<(u32, u64)> {
    let mut pairs = counts.iter().map(|(id, count)| (*id, *count)).collect::<Vec<_>>();
    pairs.sort_unstable_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    pairs.truncate(top_n);
    pairs
}

fn find_override(
    overrides: &[EcLandRuntimeMaterialIdOverride],
    terrain_id: u32,
) -> Option<&EcLandRuntimeMaterialIdOverride> {
    overrides
        .binary_search_by_key(&terrain_id, |record| record.terrain_id)
        .ok()
        .and_then(|index| overrides.get(index))
}

fn parse_optional_u32(
    value: Option<std::ffi::OsString>,
    default: u32,
    label: &str,
) -> eyre::Result<u32> {
    value
        .map(|value| {
            value
                .to_string_lossy()
                .parse::<u32>()
                .wrap_err_with(|| format!("parse {}", label))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn parse_optional_usize(
    value: Option<std::ffi::OsString>,
    default: usize,
    label: &str,
) -> eyre::Result<usize> {
    value
        .map(|value| {
            value
                .to_string_lossy()
                .parse::<usize>()
                .wrap_err_with(|| format!("parse {}", label))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn facet_uop_path(ecdir: &Path, map_index: u32) -> PathBuf {
    ecdir.join(format!("facet{map_index}.uop"))
}

fn facet_block_count(map_index: u32) -> eyre::Result<u32> {
    let (width, height) = match map_index {
        0 | 1 => (7168, 4096),
        2 => (2304, 1600),
        3 => (2560, 2048),
        4 => (1448, 1448),
        5 => (1280, 4096),
        _ => eyre::bail!("unsupported map_index {}", map_index),
    };

    Ok((width / 64) * (height / 64))
}
