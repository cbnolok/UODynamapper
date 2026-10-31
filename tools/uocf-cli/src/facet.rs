use std::path::{Path, PathBuf};

use clap::Subcommand;
use color_eyre::eyre::{self, WrapErr};
use uocf::classic::map::{MapPlane, MapSizeCells};
use uocf::classic::statics::StaticsReader;
use uocf::enhanced::facet_definition::{read_facet_definition, write_facet_definition, FacetDefinition, FacetRect};
use uocf::enhanced::facet_encoder::encode_map_plane;
use uocf::enhanced::facet_package::{detect_facet_dialect, EcFacetPackage};
use uocf::enhanced::facet_radar::{radar_index_agreement, FacetRadarDds, FacetRadarSource, PixelRect, RadarPalette};
use uocf::enhanced::facet_sector::FacetGeometry;
use uocf::kr::facet::facet_encoder as kr_facet_encoder;
use uocf::kr::facet::kr_land_map::KrLandMap;
use uocf::uop_container::package::UopPackage;

/// Values in a `--add-static` / `--add-rect` list.
const STATIC_FIELDS: usize = 3;
const SET_STATIC_FIELDS: usize = 4;
const RECT_FIELDS: usize = 4;
const RGB_FIELDS: usize = 3;
const HEX_PREFIX: &str = "0x";
const HEX_RADIX: u32 = 16;

#[derive(Subcommand, Debug)]
pub enum FacetCmd {
    /// Print one square of a facet package: land, delimiters, statics. Kingdom Reborn packages are detected; their
    /// land is the material, followed by the subtype and the stored Classic land id.
    DumpCell {
        /// facetN.uop or facetNx.uop.
        #[arg(long)]
        uop: PathBuf,
        /// Facet index of the package (the base index for facetNx.uop).
        #[arg(long)]
        facet: u8,
        #[arg(long)]
        x: u32,
        #[arg(long)]
        y: u32,
    },
    /// Edit one square of a facet package and save the package; the neighbours' delimiters follow land edits.
    /// Optionally updates a radar image around the square. In a Kingdom Reborn package (detected), --land is the
    /// Classic land id, stored with the material and subtype of the shipped land map unless --material or
    /// --subtype give them.
    PatchCell {
        #[arg(long)]
        uop: PathBuf,
        #[arg(long)]
        facet: u8,
        #[arg(long)]
        x: u32,
        #[arg(long)]
        y: u32,
        /// New land id.
        #[arg(long)]
        land: Option<String>,
        /// Kingdom Reborn only: new material (TerrainDefinition id).
        #[arg(long)]
        material: Option<String>,
        /// Kingdom Reborn only: new subtype (alias of the material).
        #[arg(long)]
        subtype: Option<u8>,
        /// New land z.
        #[arg(long, allow_hyphen_values = true)]
        z: Option<i8>,
        /// Remove static INDEX (before the other static edits; repeat, highest index first is safest).
        #[arg(long, value_name = "INDEX")]
        remove_static: Vec<usize>,
        /// Change a static: INDEX,GRAPHIC,Z,HUE.
        #[arg(long, value_name = "INDEX,GRAPHIC,Z,HUE", allow_hyphen_values = true)]
        set_static: Vec<String>,
        /// Append a static: GRAPHIC,Z,HUE.
        #[arg(long, value_name = "GRAPHIC,Z,HUE", allow_hyphen_values = true)]
        add_static: Vec<String>,
        /// Output package.
        #[arg(short, long)]
        output: PathBuf,
        /// Radar image (facetNN.dds) to update around the square.
        #[arg(long, requires_all = ["radarcol", "radar_output"])]
        radar: Option<PathBuf>,
        /// Classic radarcol.mul, for --radar.
        #[arg(long)]
        radarcol: Option<PathBuf>,
        /// Output radar image, for --radar.
        #[arg(long)]
        radar_output: Option<PathBuf>,
    },
    /// Regenerate the radar image (facetNN.dds, DXT1 with all mips) of a facet package.
    RebuildRadar {
        #[arg(long)]
        uop: PathBuf,
        #[arg(long)]
        facet: u8,
        /// Classic radarcol.mul.
        #[arg(long)]
        radarcol: PathBuf,
        /// Output facetNN.dds.
        #[arg(short, long)]
        output: PathBuf,
        /// A shipped facetNN.dds to report the palette-index agreement against.
        #[arg(long)]
        compare: Option<PathBuf>,
    },
    /// Build facetN.uop and facetNN.dds from a Classic map, staidx and statics (Enhanced Client, or Kingdom Reborn
    /// with --kr).
    FromClassic {
        /// mapN.mul or mapNLegacyMUL.uop.
        #[arg(long)]
        map: PathBuf,
        #[arg(long)]
        staidx: PathBuf,
        #[arg(long)]
        statics: PathBuf,
        #[arg(long)]
        radarcol: PathBuf,
        #[arg(long)]
        facet: u8,
        /// Map size in squares; defaults to the client's size of the facet.
        #[arg(long, requires = "height")]
        width: Option<u32>,
        #[arg(long, requires = "width")]
        height: Option<u32>,
        /// Output directory.
        #[arg(short, long)]
        output_dir: PathBuf,
        /// Build a Kingdom Reborn facet (squares carry material and subtype from the shipped land map).
        #[arg(long)]
        kr: bool,
        /// Kingdom Reborn: the Classic land id whose material and subtype to store for Classic ids the land map
        /// lacks (else they fail the build).
        #[arg(long, requires = "kr")]
        kr_fallback_land: Option<String>,
    },
    /// Print the facet definitions of facets.uop.
    DumpDef {
        #[arg(long)]
        uop: PathBuf,
        /// Only this facet.
        #[arg(long)]
        facet: Option<u32>,
    },
    /// Edit one facet definition of facets.uop and save the package.
    PatchDef {
        #[arg(long)]
        uop: PathBuf,
        #[arg(long)]
        facet: u32,
        /// New facet name cliloc.
        #[arg(long)]
        name_cliloc: Option<u32>,
        /// Region to edit with the --region-* options.
        #[arg(long)]
        region: Option<usize>,
        #[arg(long, requires = "region")]
        region_name_cliloc: Option<u32>,
        /// R,G,B.
        #[arg(long, requires = "region")]
        region_rgb: Option<String>,
        /// Facet whose coordinates the rects use (255: this facet).
        #[arg(long, requires = "region")]
        region_physical_facet: Option<u8>,
        /// Remove rect INDEX of the region (before --add-rect).
        #[arg(long, requires = "region")]
        remove_rect: Vec<usize>,
        /// Append an inclusive rect X1,Y1,X2,Y2 to the region.
        #[arg(long, requires = "region")]
        add_rect: Vec<String>,
        /// Output package.
        #[arg(short, long)]
        output: PathBuf,
    },
}

pub fn run(cmd: FacetCmd) -> eyre::Result<()> {
    match cmd {
        FacetCmd::DumpCell { uop, facet, x, y } => {
            let mut package = open_facet(&uop, facet)?;
            print_cell(&mut package, x, y)?;
        }
        FacetCmd::PatchCell {
            uop,
            facet,
            x,
            y,
            land,
            material,
            subtype,
            z,
            remove_static,
            set_static,
            add_static,
            output,
            radar,
            radarcol,
            radar_output,
        } => {
            let mut package = open_facet(&uop, facet)?;
            let land = land.as_deref().map(parse_number::<u16>).transpose()?;
            let material = material.as_deref().map(parse_number::<u16>).transpose()?;
            if package.dialect().is_kingdom_reborn() {
                if land.is_some() || material.is_some() || subtype.is_some() || z.is_some() {
                    let current = package.square(x, y)?;
                    let current_kr =
                        current.kr.ok_or_else(|| eyre::eyre!("({x}, {y}) has no Kingdom Reborn bytes"))?;
                    let (mut new_material, mut kr, current_z) = (current.land_graphic, current_kr, current.z);
                    if let Some(classic_land) = land {
                        let (mapped_material, mapped) = KrLandMap::shipped().square_land(classic_land)?;
                        new_material = mapped_material;
                        kr = mapped.expect("the land map gives every Classic id its Kingdom Reborn bytes");
                    }
                    new_material = material.unwrap_or(new_material);
                    kr.subtype = subtype.unwrap_or(kr.subtype);
                    package.set_kr_land(x, y, new_material, kr, z.unwrap_or(current_z))?;
                }
            } else {
                if material.is_some() || subtype.is_some() {
                    eyre::bail!("--material and --subtype need a Kingdom Reborn package");
                }
                if land.is_some() || z.is_some() {
                    let current = package.square(x, y)?;
                    let (current_land, current_z) = (current.land_graphic, current.z);
                    package.set_land(x, y, land.unwrap_or(current_land), z.unwrap_or(current_z))?;
                }
            }
            for index in remove_static {
                package.remove_static(x, y, index)?;
            }
            for item in &set_static {
                let [index, graphic, z, hue] = parse_list::<SET_STATIC_FIELDS>(item)?;
                package.set_static(x, y, index as usize, graphic as u32, to_z(z)?, hue as u32)?;
            }
            for item in &add_static {
                let [graphic, z, hue] = parse_list::<STATIC_FIELDS>(item)?;
                package.add_static(x, y, graphic as u32, to_z(z)?, hue as u32)?;
            }
            let sectors: Vec<u32> = package.dirty_sectors().collect();
            if let (Some(radar), Some(radarcol), Some(radar_output)) = (radar, radarcol, radar_output) {
                let palette = RadarPalette::load_radarcol(&radarcol)?;
                let mut image = FacetRadarDds::load(&radar)?;
                let rewritten = image.update_region(&mut package, &palette, PixelRect::square(x, y))?;
                image.save(&radar_output)?;
                println!(
                    "Wrote {}: pixels {}..{} x {}..{} re-encoded at mip 0, and the pixels over them at every mip.",
                    radar_output.display(),
                    rewritten.x0,
                    rewritten.x1,
                    rewritten.y0,
                    rewritten.y1
                );
            }
            package.save(&output)?;
            print_cell(&mut package, x, y)?;
            println!("Wrote {} (sectors {sectors:?} changed).", output.display());
        }
        FacetCmd::RebuildRadar { uop, facet, radarcol, output, compare } => {
            let package = open_facet(&uop, facet)?;
            let palette = RadarPalette::load_radarcol(&radarcol)?;
            let geometry = package.geometry();
            let rgb = FacetRadarSource::from_facet(&package, &palette)?.rgb8();
            let image = FacetRadarDds::encode_rgb8(geometry.width, geometry.height, &rgb)?;
            image.save(&output)?;
            println!("Wrote {} ({}x{}, {} mips).", output.display(), geometry.width, geometry.height, image.mip_count());
            if let Some(compare) = compare {
                let shipped = FacetRadarDds::load(&compare)?;
                println!(
                    "Palette-index agreement with {}: {:.4} (recipe), {:.4} (our DXT1).",
                    compare.display(),
                    radar_index_agreement(&shipped, &rgb)?,
                    radar_index_agreement(&shipped, &image.decode_level_rgb8(0))?
                );
            }
        }
        FacetCmd::FromClassic {
            map,
            staidx,
            statics,
            radarcol,
            facet,
            width,
            height,
            output_dir,
            kr,
            kr_fallback_land,
        } => {
            let geometry = match (width, height) {
                (Some(width), Some(height)) => FacetGeometry::new(width, height),
                _ => FacetGeometry::client_facet(facet)
                    .ok_or_else(|| eyre::eyre!("facet {facet} has no client size; pass --width and --height"))?,
            };
            let size = MapSizeCells { width: geometry.width, height: geometry.height };
            let is_uop = map.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("uop"));
            let mut plane = if is_uop {
                MapPlane::init_uop_with_size(map.clone(), u32::from(facet), Some(size))?
            } else {
                MapPlane::init_with_size(map.clone(), u32::from(facet), Some(size))?
            };
            let statics = StaticsReader::new(&staidx, &statics, geometry.width, geometry.height)?;
            std::fs::create_dir_all(&output_dir).wrap_err_with(|| format!("create {}", output_dir.display()))?;
            if kr {
                let mut land_map = KrLandMap::shipped();
                if let Some(fallback) = kr_fallback_land.as_deref().map(parse_number::<u16>).transpose()? {
                    let land = land_map
                        .get(fallback)
                        .ok_or_else(|| eyre::eyre!("--kr-fallback-land {fallback} is not in the land map"))?;
                    land_map = land_map.with_fallback(land);
                }
                kr_facet_encoder::encode_map_plane(&mut plane, &statics, &radarcol, &output_dir, facet, &land_map)?;
            } else {
                encode_map_plane(&mut plane, &statics, &radarcol, &output_dir, facet)?;
            }
            let client = if kr { "Kingdom Reborn" } else { "Enhanced Client" };
            println!(
                "Wrote {client} facet {facet} ({}x{}) to {}.",
                geometry.width,
                geometry.height,
                output_dir.display()
            );
        }
        FacetCmd::DumpDef { uop, facet } => {
            let package = load_package(&uop)?;
            let ids: Vec<u32> = match facet {
                Some(facet) => vec![facet],
                None => uocf::enhanced::facet_definition::CLIENT_FACET_DEFINITION_IDS.collect(),
            };
            for id in ids {
                match read_facet_definition(&package, id)? {
                    Some(definition) => print_definition(&definition),
                    None => println!("facet {id}: no entry"),
                }
            }
        }
        FacetCmd::PatchDef {
            uop,
            facet,
            name_cliloc,
            region,
            region_name_cliloc,
            region_rgb,
            region_physical_facet,
            remove_rect,
            add_rect,
            output,
        } => {
            let mut package = load_package(&uop)?;
            let mut definition = read_facet_definition(&package, facet)?
                .ok_or_else(|| eyre::eyre!("{} has no definition of facet {facet}", uop.display()))?;
            if let Some(cliloc) = name_cliloc {
                definition.name_cliloc = cliloc;
            }
            if let Some(region) = region {
                let count = definition.regions.len();
                let target = definition
                    .regions
                    .get_mut(region)
                    .ok_or_else(|| eyre::eyre!("facet {facet} has {count} regions, no region {region}"))?;
                if let Some(cliloc) = region_name_cliloc {
                    target.name_cliloc = cliloc;
                }
                if let Some(rgb) = region_rgb {
                    target.rgb = parse_list::<RGB_FIELDS>(&rgb)?.map(|channel| channel as u8);
                }
                if let Some(physical) = region_physical_facet {
                    target.physical_facet = physical;
                }
                for index in remove_rect {
                    if index >= target.rects.len() {
                        eyre::bail!("region {region} has {} rects, no rect {index}", target.rects.len());
                    }
                    target.rects.remove(index);
                }
                for rect in &add_rect {
                    let [x1, y1, x2, y2] = parse_list::<RECT_FIELDS>(rect)?.map(|value| value as u32);
                    target.rects.push(FacetRect { x1, y1, x2, y2 });
                }
            }
            write_facet_definition(&mut package, &definition)?;
            package.finalize_and_save(&output).wrap_err_with(|| format!("save {}", output.display()))?;
            print_definition(&definition);
            println!("Wrote {}.", output.display());
        }
    }
    Ok(())
}

/// Opens facet package `path` in the square layout its sectors have (Enhanced Client or Kingdom Reborn).
fn open_facet(path: &Path, facet: u8) -> eyre::Result<EcFacetPackage> {
    let package = load_package(path)?;
    let dialect = detect_facet_dialect(&package, facet).wrap_err_with(|| format!("read {}", path.display()))?;
    EcFacetPackage::from_package_dialect(package, facet, None, dialect)
}

fn load_package(path: &Path) -> eyre::Result<UopPackage> {
    UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))
}

fn print_cell(package: &mut EcFacetPackage, x: u32, y: u32) -> eyre::Result<()> {
    let sector = package.geometry().sector_of(x, y);
    let tile = package.square(x, y)?;
    match tile.kr {
        Some(kr) => println!(
            "({x}, {y}) sector {sector}: material 0x{:04x} subtype {} classic land 0x{:04x} z {}",
            tile.land_graphic, kr.subtype, kr.classic_land, tile.z
        ),
        None => println!("({x}, {y}) sector {sector}: land 0x{:04x} z {}", tile.land_graphic, tile.z),
    }
    for delimiter in &tile.delimiters {
        println!("  delimiter dir {} z {} land 0x{:04x}", delimiter.direction, delimiter.z, delimiter.graphic_id);
    }
    for (index, item) in tile.statics.iter().enumerate() {
        println!("  static {index}: graphic 0x{:04x} z {} hue {}", item.graphic_id, item.z, item.hue);
    }
    Ok(())
}

fn print_definition(definition: &FacetDefinition) {
    println!(
        "facet {}: name cliloc {}, {}x{}, unknown {:02x?}, {} regions",
        definition.facet_id,
        definition.name_cliloc,
        definition.width,
        definition.height,
        definition.unknown,
        definition.regions.len()
    );
    for (position, region) in definition.regions.iter().enumerate() {
        let rects: Vec<String> =
            region.rects.iter().map(|rect| format!("{},{}..{},{}", rect.x1, rect.y1, rect.x2, rect.y2)).collect();
        println!(
            "  region {position} (index {}): cliloc {}, rgb {:?}, physical facet {}, rects [{}]",
            region.index,
            region.name_cliloc,
            region.rgb,
            region.physical_facet,
            rects.join(" ")
        );
    }
    for issue in definition.validate() {
        println!("  issue: {issue:?}");
    }
}

/// Parses a decimal or `0x` hexadecimal number.
fn parse_number<T: TryFrom<i64>>(text: &str) -> eyre::Result<T> {
    let text = text.trim();
    let value = match text.strip_prefix(HEX_PREFIX) {
        Some(hex) => i64::from_str_radix(hex, HEX_RADIX),
        None => text.parse::<i64>(),
    }
    .wrap_err_with(|| format!("bad number {text:?}"))?;
    T::try_from(value).map_err(|_| eyre::eyre!("{text} is out of range"))
}

/// Parses `N` comma-separated numbers.
fn parse_list<const N: usize>(text: &str) -> eyre::Result<[i64; N]> {
    let values = text.split(',').map(parse_number::<i64>).collect::<eyre::Result<Vec<_>>>()?;
    values.try_into().map_err(|values: Vec<i64>| eyre::eyre!("{text:?}: expected {N} values, got {}", values.len()))
}

fn to_z(value: i64) -> eyre::Result<i8> {
    i8::try_from(value).map_err(|_| eyre::eyre!("z {value} is outside -128..127"))
}
