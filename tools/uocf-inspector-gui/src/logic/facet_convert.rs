//! Whole-facet conversion behind the Maps tab: the tab's choices as a [`uocf::facet_convert`] conversion, the one behind
//! `uocf-cli facet convert`, where a save without changes is the source as it is.

use std::path::{Path, PathBuf};

use color_eyre::eyre;
use uocf::classic::facet_image_render::FacetImageMode;
use uocf::classic::map::MapSizeCells;
use uocf::classic::world::LandFill;
use uocf::facet_convert::{ClassicContainers, ConversionSource, ConversionTarget, ConvertHints, FacetConversion, ParsedValues, SourceDifs};
use uocf::kr::facet::kr_land_map::KrLandMap;
use uocf::map_edition::MapEdition;

const PACKAGE_NAME_PREFIX: &str = "facet";
/// What the messages of the conversion call the tab's controls.
const HINTS: ConvertHints = ConvertHints {
    standard_edition: "choose the standard edition",
    target_is: "the target is",
    size_is: "",
    crop: "tick Crop",
    allow_lossy: "tick Accept losses",
    accepted_losses: "Accepted losses",
    bake_difs: "choose Bake them all",
    choose_container: "choose a Classic target",
    image_choice: "the world map image choice is",
    image_needs_client: "it needs a target client that draws facetNN.mul",
    image_classic_only: "the world map image choice applies to Classic targets only.",
    image_kept: "as chosen",
    image_refused: "as chosen",
    image_regenerate: "choose Render the whole image",
    no_radarcol: "in the source or target client's folder",
};

/// What to do with the dif files of a Classic source.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DifChoice {
    /// Bake the kinds the source client applies, told by its executable; nothing when that is unknown.
    #[default]
    AsTheClientApplies,
    /// Bake every dif file found.
    Bake,
    Ignore,
}

impl DifChoice {
    pub const ALL: [Self; 3] = [Self::AsTheClientApplies, Self::Bake, Self::Ignore];

    pub fn label(self) -> &'static str {
        match self {
            Self::AsTheClientApplies => "As its client applies them",
            Self::Bake => "Bake them all",
            Self::Ignore => "Ignore them",
        }
    }
}

impl From<DifChoice> for SourceDifs {
    fn from(choice: DifChoice) -> Self {
        match choice {
            DifChoice::AsTheClientApplies => Self::AsTheClientApplies,
            DifChoice::Bake => Self::Bake,
            DifChoice::Ignore => Self::Ignore,
        }
    }
}

#[derive(Clone, Debug)]
pub enum FacetSource {
    Classic { dir: PathBuf, edition: MapEdition, difs: DifChoice },
    /// An Enhanced Client or Kingdom Reborn `facetN.uop`, told apart by its contents.
    Package(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FacetTarget {
    ClassicMul,
    #[default]
    ClassicUop,
    ClassicBoth,
    Enhanced,
    KingdomReborn,
}

impl FacetTarget {
    pub const ALL: [Self; 5] = [Self::ClassicMul, Self::ClassicUop, Self::ClassicBoth, Self::Enhanced, Self::KingdomReborn];

    pub fn label(self) -> &'static str {
        match self {
            Self::ClassicMul => "Classic mapN.mul",
            Self::ClassicUop => "Classic mapNLegacyMUL.uop",
            Self::ClassicBoth => "Classic MUL and UOP",
            Self::Enhanced => "Enhanced Client facetN.uop",
            Self::KingdomReborn => "Kingdom Reborn facetN.uop",
        }
    }

    pub fn is_classic(self) -> bool {
        matches!(self, Self::ClassicMul | Self::ClassicUop | Self::ClassicBoth)
    }

    fn conversion_target(self) -> ConversionTarget {
        match self {
            Self::ClassicMul => ConversionTarget::Classic(ClassicContainers::Mul),
            Self::ClassicUop => ConversionTarget::Classic(ClassicContainers::Uop),
            Self::ClassicBoth => ConversionTarget::Classic(ClassicContainers::Both),
            Self::Enhanced => ConversionTarget::Enhanced,
            Self::KingdomReborn => ConversionTarget::KingdomReborn,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConvertRequest {
    pub source: FacetSource,
    pub facet: u8,
    pub target: FacetTarget,
    pub target_facet: u8,
    pub target_edition: MapEdition,
    /// The size written; `None` for the target client's size table, else the source's size.
    pub size: Option<MapSizeCells>,
    /// A client folder or executable written for: its size table, its tiledata for the id check, and the world map
    /// image of a client that draws one.
    pub target_client: Option<PathBuf>,
    /// Lets a smaller size drop the content outside it.
    pub crop: bool,
    /// What a larger size adds.
    pub fill: LandFill,
    /// Accepts what the Classic files cannot hold of a facet package.
    pub allow_lossy: bool,
    /// Kingdom Reborn: the Classic land id whose material and subtype to store for the ids its land map lacks, such as
    /// the land tiles added after its release. `None` refuses those ids.
    pub kr_fallback_land: Option<u16>,
    pub facet_image: FacetImageMode,
    pub output_dir: PathBuf,
}

/// The facet and edition a package's file name tells.
pub fn package_name_facet(path: &Path) -> Option<(u8, MapEdition)> {
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let rest = stem.strip_prefix(PACKAGE_NAME_PREFIX)?;
    let suffix = MapEdition::Variant.file_suffix();
    match rest.strip_suffix(suffix) {
        Some(digits) => Some((digits.parse().ok()?, MapEdition::Variant)),
        None => Some((rest.parse().ok()?, MapEdition::Standard)),
    }
}

/// Converts and saves the facet; returns what was done, line by line.
pub fn convert_facet(request: &ConvertRequest) -> eyre::Result<Vec<String>> {
    let kr_land = kr_land_map(request.kr_fallback_land)?;
    let (source, difs) = match &request.source {
        FacetSource::Classic { dir, edition, difs } => (ConversionSource::ClassicFolder { dir: dir.clone(), edition: *edition }, (*difs).into()),
        FacetSource::Package(path) => (ConversionSource::Package(path.clone()), SourceDifs::default()),
    };
    let conversion = FacetConversion {
        source,
        facet: request.facet,
        difs,
        target: request.target.conversion_target(),
        target_facet: request.target_facet,
        target_edition: Some(request.target_edition),
        target_client: request.target_client.clone(),
        crop: request.crop,
        uop_template: None,
        target_tiledata: None,
        allow_lossy: request.allow_lossy,
        radarcol: None,
        kr_land,
        // The combo shows only with a target client, so a choice left at its start gets no word about a target that draws no image.
        facet_image: (request.target.is_classic() && request.facet_image != FacetImageMode::default()).then_some(request.facet_image),
        keep_unchanged: true,
        output_dir: request.output_dir.clone(),
    };
    let values = ParsedValues { source_size: Ok(None), size: Ok(request.size), fill: Ok(request.fill) };
    let mut notes = Vec::new();
    uocf::facet_convert::convert_facet(&conversion, values, &HINTS, &mut |line| notes.push(line))?;
    Ok(notes)
}

fn kr_land_map(fallback: Option<u16>) -> eyre::Result<KrLandMap> {
    let land_map = KrLandMap::shipped();
    let Some(fallback) = fallback else {
        return Ok(land_map);
    };
    let land = land_map.get(fallback).ok_or_else(|| eyre::eyre!("land {fallback:#06X} is not in the Kingdom Reborn land map"))?;
    Ok(land_map.with_fallback(land))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::{client_roots, output_path, CLASSIC_CLIENT_ROOTS_ENV, EC_CLIENT_ROOTS_ENV};
    use uocf::classic::facet_sources::{statics_file_names, ClassicDifOptions, ClassicFacetSources};
    use uocf::classic::map_write::ClassicMapBlocks;
    use uocf::classic::statics_write::ClassicStaticsBlocks;
    use uocf::classic::world::ClassicWorld;
    use uocf::enhanced::facet_package::{detect_facet_dialect, EcFacetPackage};
    use uocf::enhanced::facet_sector::facet_package_file_name;
    use uocf::facet_convert::SOURCE_MAP_FORMATS;
    use uocf::uop_container::package::UopPackage;
    use uocf::utils::path::find_file_ignoring_case;

    /// The smallest shipped facet, Tokuno, whose height the map readers know; a narrow copy of it is quick to convert.
    const SMALL_FACET: u8 = 4;
    const SMALL_HEIGHT: u32 = 1448;
    const SMALL_WIDTH: u32 = 8;
    const GROWN_WIDTH: u32 = 16;
    const UNALIGNED_WIDTH: u32 = 12;
    const LAND: u16 = 3;
    const LAND_Z: i8 = 1;
    const FILL: LandFill = LandFill { land: 5, z: 2 };
    const FELUCCA: u8 = 0;
    const OLD_FELUCCA: MapSizeCells = MapSizeCells { width: 6144, height: 4096 };
    /// A cell of the ground a client ships, for the checks after a resize.
    const PROBE: (u32, u32) = (1000, 1000);
    const CURRENT_FELUCCA: MapSizeCells = MapSizeCells { width: 7168, height: 4096 };
    /// A land id of the Kingdom Reborn land map, for the land it has no material for.
    const KR_FALLBACK_LAND: u16 = 3;
    const NOT_KR_LAND: u16 = u16::MAX;

    fn request(source: FacetSource, target: FacetTarget, output_dir: PathBuf) -> ConvertRequest {
        ConvertRequest {
            source,
            facet: SMALL_FACET,
            target,
            target_facet: SMALL_FACET,
            target_edition: MapEdition::Standard,
            size: None,
            target_client: None,
            crop: false,
            fill: LandFill::VOID,
            allow_lossy: false,
            kr_fallback_land: None,
            facet_image: FacetImageMode::default(),
            output_dir,
        }
    }

    fn classic(dir: &Path) -> FacetSource {
        FacetSource::Classic { dir: dir.to_path_buf(), edition: MapEdition::Standard, difs: DifChoice::Ignore }
    }

    /// A folder holding a narrow Classic facet, and the folder to write to.
    fn small_client(name: &str) -> (PathBuf, PathBuf) {
        let dir = output_path(&format!("{name}-source"));
        let output = output_path(&format!("{name}-output"));
        std::fs::create_dir_all(&dir).unwrap();
        let world = ClassicWorld::filled(MapSizeCells { width: SMALL_WIDTH, height: SMALL_HEIGHT }, LandFill { land: LAND, z: LAND_Z }).unwrap();
        world.map.write_mul(&dir.join(format!("map{SMALL_FACET}.mul"))).unwrap();
        let [index, data] = statics_file_names(u32::from(SMALL_FACET), MapEdition::Standard);
        world.statics.write_mul(&dir.join(index), &dir.join(data)).unwrap();
        (dir, output)
    }

    fn open_package(path: &Path, facet: u8) -> EcFacetPackage {
        let package = UopPackage::load(path).unwrap();
        let dialect = detect_facet_dialect(&package, facet).unwrap();
        EcFacetPackage::from_package_dialect(package, facet, None, dialect).unwrap()
    }

    fn clean(dirs: &[&Path]) {
        for dir in dirs {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_package_name_tells_its_facet_and_edition() {
        assert_eq!(package_name_facet(Path::new("/a/facet3.uop")), Some((3, MapEdition::Standard)));
        assert_eq!(package_name_facet(Path::new("/a/FACET1X.uop")), Some((1, MapEdition::Variant)));
        assert_eq!(package_name_facet(Path::new("/a/map1.uop")), None);
    }

    #[test]
    fn a_folder_holding_the_source_is_refused_as_the_output() {
        let (dir, output) = small_client("facet-refused");
        let target = request(classic(&dir), FacetTarget::ClassicMul, dir.clone());
        let before = std::fs::read(dir.join(format!("map{SMALL_FACET}.mul"))).unwrap();
        assert!(convert_facet(&target).is_err());
        assert!(std::fs::read(dir.join(format!("map{SMALL_FACET}.mul"))).unwrap() == before);
        clean(&[&dir, &output]);
    }

    #[test]
    fn a_larger_size_fills_with_the_chosen_land_and_comes_back() {
        let (dir, output) = small_client("facet-grow");
        let size = MapSizeCells { width: GROWN_WIDTH, height: SMALL_HEIGHT };
        let target = ConvertRequest { size: Some(size), fill: FILL, ..request(classic(&dir), FacetTarget::ClassicMul, output.clone()) };
        convert_facet(&target).unwrap();
        let map = ClassicMapBlocks::read_mul(&output.join(format!("map{SMALL_FACET}.mul")), u32::from(SMALL_FACET), None).unwrap();
        assert_eq!(map.size_cells(), size);
        assert_eq!(map.cell(0, 0).unwrap(), (LAND, LAND_Z));
        assert_eq!(map.cell(SMALL_WIDTH, 0).unwrap(), (FILL.land, FILL.z));
        clean(&[&dir, &output]);
    }

    #[test]
    fn a_smaller_size_needs_the_crop_and_a_side_must_be_whole_blocks() {
        let (dir, output) = small_client("facet-grown-source");
        let grown = output_path("facet-grown");
        convert_facet(&ConvertRequest {
            size: Some(MapSizeCells { width: GROWN_WIDTH, height: SMALL_HEIGHT }),
            fill: FILL,
            ..request(classic(&dir), FacetTarget::ClassicMul, grown.clone())
        })
        .unwrap();
        let small = MapSizeCells { width: SMALL_WIDTH, height: SMALL_HEIGHT };
        let shrink = ConvertRequest { size: Some(small), ..request(classic(&grown), FacetTarget::ClassicMul, output.clone()) };
        assert!(convert_facet(&shrink).is_err(), "the crop is refused unless asked for");
        convert_facet(&ConvertRequest { crop: true, ..shrink.clone() }).unwrap();
        let map = ClassicMapBlocks::read_mul(&output.join(format!("map{SMALL_FACET}.mul")), u32::from(SMALL_FACET), None).unwrap();
        assert_eq!(map.size_cells(), small);
        let unaligned = ConvertRequest { size: Some(MapSizeCells { width: UNALIGNED_WIDTH, height: SMALL_HEIGHT }), ..shrink };
        assert!(convert_facet(&unaligned).is_err());
        clean(&[&dir, &output, &grown]);
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_classic_facet_saves_the_same_bytes_and_converts_and_comes_back() {
        for root in client_roots(CLASSIC_CLIENT_ROOTS_ENV) {
            let name = root.file_name().unwrap().to_string_lossy().into_owned();
            let source = classic(&root);
            let uop_source = find_file_ignoring_case(&root, &format!("map{SMALL_FACET}LegacyMUL.uop"));
            let map_source = uop_source.clone().or_else(|| find_file_ignoring_case(&root, &format!("map{SMALL_FACET}.mul"))).unwrap();
            let output = output_path(&format!("facet-classic-{}", name.replace(' ', "-")));
            let target = if uop_source.is_some() { FacetTarget::ClassicUop } else { FacetTarget::ClassicMul };
            for note in convert_facet(&request(source.clone(), target, output.clone())).unwrap() {
                println!("{name}: {note}");
            }
            let [index, data] = statics_file_names(u32::from(SMALL_FACET), MapEdition::Standard);
            let map_name = map_source.file_name().unwrap().to_string_lossy().into_owned();
            for file in [&map_name, &index, &data] {
                let source_file = find_file_ignoring_case(&root, file).unwrap();
                assert!(std::fs::read(output.join(file)).unwrap() == std::fs::read(source_file).unwrap(), "{name}: {file} differs after a save without changes");
            }

            let original = ClassicWorld::load(
                &ClassicFacetSources::resolve(&[root.clone()], u32::from(SMALL_FACET), &SOURCE_MAP_FORMATS, ClassicDifOptions::NONE, MapEdition::Standard).unwrap(),
                None,
            )
            .unwrap()
            .0;
            let package_dir = output_path(&format!("facet-package-{}", name.replace(' ', "-")));
            let back_dir = output_path(&format!("facet-back-{}", name.replace(' ', "-")));
            convert_facet(&request(source.clone(), FacetTarget::Enhanced, package_dir.clone())).unwrap();
            let package = package_dir.join(facet_package_file_name(SMALL_FACET, false));
            convert_facet(&ConvertRequest { allow_lossy: true, ..request(FacetSource::Package(package), FacetTarget::ClassicMul, back_dir.clone()) }).unwrap();
            let back = ClassicMapBlocks::read_mul(&back_dir.join(format!("map{SMALL_FACET}.mul")), u32::from(SMALL_FACET), None).unwrap();
            assert!(back == original.map, "{name}: the land differs after Classic to Enhanced Client to Classic");
            let statics = ClassicStaticsBlocks::read_mul(&back_dir.join(&index), &back_dir.join(&data), back.size_cells()).unwrap();
            assert_eq!(statics.static_count(), original.statics.static_count(), "{name}: the statics after Classic to Enhanced Client to Classic");
            println!("{name}: same bytes; Classic to Enhanced Client to Classic keeps the land and the {} statics", statics.static_count());
            clean(&[&output, &package_dir, &back_dir]);
        }
    }

    #[test]
    #[ignore = "needs real clients: set UO_EC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_facet_package_saves_the_same_bytes_and_converts() {
        for root in client_roots(EC_CLIENT_ROOTS_ENV) {
            let name = root.file_name().unwrap().to_string_lossy().into_owned();
            let path = find_file_ignoring_case(&root, &facet_package_file_name(SMALL_FACET, false)).unwrap();
            let package = open_package(&path, SMALL_FACET);
            let target = if package.dialect().is_kingdom_reborn() { FacetTarget::KingdomReborn } else { FacetTarget::Enhanced };
            let output = output_path(&format!("facet-same-{}", name.replace(' ', "-")));
            let notes = convert_facet(&request(FacetSource::Package(path.clone()), target, output.clone())).unwrap();
            assert!(notes.last().unwrap().contains("as it is"));
            assert!(std::fs::read(output.join(path.file_name().unwrap())).unwrap() == std::fs::read(&path).unwrap(), "{name}: differs after a save without changes");

            let classic_dir = output_path(&format!("facet-classic-out-{}", name.replace(' ', "-")));
            for note in convert_facet(&ConvertRequest { allow_lossy: true, ..request(FacetSource::Package(path), FacetTarget::ClassicMul, classic_dir.clone()) }).unwrap() {
                println!("{name}: {note}");
            }
            let map = ClassicMapBlocks::read_mul(&classic_dir.join(format!("map{SMALL_FACET}.mul")), u32::from(SMALL_FACET), None).unwrap();
            let geometry = package.geometry();
            assert_eq!(map.size_cells(), MapSizeCells { width: geometry.width, height: geometry.height }, "{name}");
            println!("{name}: same bytes; converts to Classic at {}x{}", geometry.width, geometry.height);
            clean(&[&output, &classic_dir]);
        }
    }

    #[test]
    #[ignore = "needs real clients and minutes: set UO_CLASSIC_CLIENT_ROOTS, UO_EC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn felucca_goes_to_the_old_client_sizes_as_classic_and_kingdom_reborn() {
        let classic_roots = client_roots(CLASSIC_CLIENT_ROOTS_ENV);
        let root = classic_roots.first().unwrap();
        let base = ConvertRequest { facet: FELUCCA, target_facet: FELUCCA, size: Some(OLD_FELUCCA), ..request(classic(root), FacetTarget::ClassicMul, PathBuf::new()) };
        let refused = output_path("facet-old-refused");
        assert!(convert_facet(&ConvertRequest { output_dir: refused.clone(), ..base.clone() }).is_err(), "the crop is refused unless asked for");

        let output = output_path("facet-old-classic");
        convert_facet(&ConvertRequest { crop: true, output_dir: output.clone(), ..base.clone() }).unwrap();
        let map = ClassicMapBlocks::read_mul(&output.join(format!("map{FELUCCA}.mul")), u32::from(FELUCCA), Some(OLD_FELUCCA)).unwrap();
        assert_eq!(map.size_cells(), OLD_FELUCCA);
        let original = ClassicMapBlocks::read_uop(&find_file_ignoring_case(root, &format!("map{FELUCCA}LegacyMUL.uop")).unwrap(), u32::from(FELUCCA), None).unwrap();
        assert_eq!(map.cell(PROBE.0, PROBE.1).unwrap(), original.cell(PROBE.0, PROBE.1).unwrap());
        println!("{}: Felucca cropped to {}x{} as Classic MUL", root.display(), OLD_FELUCCA.width, OLD_FELUCCA.height);

        let current = ConvertRequest { crop: true, target: FacetTarget::KingdomReborn, output_dir: output_path("facet-new-kr"), ..base.clone() };
        let refusal = convert_facet(&current).unwrap_err();
        println!("{}: without a fallback land: {refusal:#}", root.display());
        let unknown = ConvertRequest { kr_fallback_land: Some(NOT_KR_LAND), ..current.clone() };
        assert!(convert_facet(&unknown).is_err(), "a fallback the land map lacks");
        convert_facet(&ConvertRequest { kr_fallback_land: Some(KR_FALLBACK_LAND), ..current.clone() }).unwrap();
        let package = open_package(&current.output_dir.join(facet_package_file_name(FELUCCA, false)), FELUCCA);
        assert!(package.dialect().is_kingdom_reborn());
        assert_eq!((package.geometry().width, package.geometry().height), (OLD_FELUCCA.width, OLD_FELUCCA.height));
        println!("{}: Felucca written as a Kingdom Reborn package of {}x{} with a fallback land", root.display(), OLD_FELUCCA.width, OLD_FELUCCA.height);

        let mut folders = vec![refused, output, current.output_dir];
        for root in classic_roots.iter().skip(1) {
            let kr = output_path("facet-gold-kr");
            let request = ConvertRequest { facet: FELUCCA, target_facet: FELUCCA, kr_fallback_land: Some(KR_FALLBACK_LAND), ..request(classic(root), FacetTarget::KingdomReborn, kr.clone()) };
            for note in convert_facet(&request).unwrap() {
                println!("{note}");
            }
            let package = open_package(&kr.join(facet_package_file_name(FELUCCA, false)), FELUCCA);
            assert!(package.dialect().is_kingdom_reborn());
            println!("{}: Felucca written as a Kingdom Reborn package of {}x{}", root.display(), package.geometry().width, package.geometry().height);
            folders.push(kr);
        }

        for root in client_roots(EC_CLIENT_ROOTS_ENV) {
            let path = find_file_ignoring_case(&root, &facet_package_file_name(FELUCCA, false)).unwrap();
            let package = open_package(&path, FELUCCA);
            if !package.dialect().is_kingdom_reborn() {
                continue;
            }
            let size = if package.geometry().width == OLD_FELUCCA.width { CURRENT_FELUCCA } else { OLD_FELUCCA };
            let kr = output_path("facet-kr-resized");
            let source = FacetSource::Package(path);
            let request = ConvertRequest { facet: FELUCCA, target_facet: FELUCCA, size: Some(size), crop: true, allow_lossy: true, ..request(source, FacetTarget::KingdomReborn, kr.clone()) };
            for note in convert_facet(&request).unwrap() {
                println!("{note}");
            }
            let package = open_package(&kr.join(facet_package_file_name(FELUCCA, false)), FELUCCA);
            assert!(package.dialect().is_kingdom_reborn());
            assert_eq!((package.geometry().width, package.geometry().height), (size.width, size.height));
            println!("{}: Felucca written as a Kingdom Reborn package of {}x{}", root.display(), size.width, size.height);
            folders.push(kr);
        }
        for folder in &folders {
            clean(&[folder]);
        }
    }
}
