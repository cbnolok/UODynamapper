use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use color_eyre::eyre::{self, Context};
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::facet_package::detect_facet_dialect;
use uocf::enhanced::facet_sector::{data_map_entry_path, facet_package_file_name, FacetGeometry, SECTOR_SIDE};
use uocf::kr::facet::facet_decoder::read_facet_block_raw;
use uocf::kr::facet::kr_land_map::{KrLandCounter, KrLandMap};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

#[derive(ClapArgs, Debug)]
#[command(about = "Generate a verified KR land mapping and evidence from every shipped facet (0-4).")]
pub struct Args {
    #[arg(long)]
    client_dir: PathBuf,
    #[arg(long)]
    output_dir: PathBuf,
    /// Require the generated routes to match the bundled verified binary.
    #[arg(long)]
    verify_shipped: bool,
}

pub fn run(args: Args) -> eyre::Result<()> {
    let mut counter = KrLandCounter::default();
    let mut observations = BTreeMap::<(u16, u16, u8), u64>::new();
    let mut sources = String::from("facet,file,bytes,width,height,sectors,on_map_squares\n");
    for facet in 0..=4 {
        let name = facet_package_file_name(facet, false);
        let path = args.client_dir.join(&name);
        let package = UopPackage::load(&path).wrap_err_with(|| format!("load {}", path.display()))?;
        eyre::ensure!(detect_facet_dialect(&package, facet)? == ClientDialect::KingdomReborn,
            "{} is not a KR facet", path.display());
        let data_map = package.unpack_file_by_hash(hash_file_name_single(&data_map_entry_path(facet)))?
            .ok_or_else(|| eyre::eyre!("{} has no data.map", path.display()))?;
        let geometry = FacetGeometry::from_data_map_payload(&data_map)?;
        let mut squares = 0u64;
        for sector in 0..geometry.sector_count() {
            let decoded = read_facet_block_raw(&package, facet, sector)
                .wrap_err_with(|| format!("facet {facet}, sector {sector}"))?;
            let (ox, oy) = geometry.sector_origin(sector);
            let on_map = |index: usize| geometry.contains(
                ox + index as u32 / SECTOR_SIDE, oy + index as u32 % SECTOR_SIDE);
            counter.add_sector(&decoded, on_map);
            for (index, tile) in decoded.tiles.iter().enumerate() {
                if !on_map(index) {
                    continue;
                }
                let kr = tile.kr.ok_or_else(|| eyre::eyre!("facet {facet}, sector {sector}: missing KR land"))?;
                *observations.entry((kr.classic_land, tile.land_graphic, kr.subtype)).or_default() += 1;
                squares += 1;
            }
        }
        eyre::ensure!(squares == u64::from(geometry.width) * u64::from(geometry.height),
            "facet {facet}: incomplete on-map coverage");
        sources.push_str(&format!("{facet},{name},{},{},{},{},{squares}\n",
            fs::metadata(&path)?.len(), geometry.width, geometry.height, geometry.sector_count()));
        println!("facet {facet}: {squares} on-map squares");
    }

    let map = counter.to_map();
    let rows = map.rows();
    if args.verify_shipped {
        eyre::ensure!(rows == KrLandMap::shipped_verified().rows(), "generated routes differ from the bundled verified binary");
    }
    let binary = map.to_binary();
    let mut evidence = String::from("classic_land,material,subtype,squares\n");
    for ((classic, material, subtype), count) in observations {
        evidence.push_str(&format!("{classic},{material},{subtype},{count}\n"));
    }
    // Decode and validate every source before creating any output.
    fs::create_dir_all(&args.output_dir)?;
    fs::write(args.output_dir.join("mappings.bin"), &binary)?;
    fs::write(args.output_dir.join("observations.csv"), evidence)?;
    fs::write(args.output_dir.join("sources.csv"), sources)?;
    let (squares, material_ok, subtype_ok) = counter.agreement(&map);
    println!("{} routes, {} bytes; material agreement {material_ok}/{squares}, subtype {subtype_ok}/{squares}",
        map.len(), binary.len());
    Ok(())
}
