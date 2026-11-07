//! `hues.uddp` holds the 256-entry table of every hue the client draws with: built from `Hues.uop`, the top row
//! of each `hue%04d.bmp` as it is (what `EcHueTables` reads); built from `hues.mul`, the 32 colours spread by the
//! package's ramp (`hue_palette_row`). The paperdoll and the gumps hue from these tables.

use super::*;
use udd_assets::hues::HuesPackage;
use uocf::enhanced::hues::EcHueTables;

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const CC_CLIENT_DIR_ENV: &str = "UO_CC_CLIENT_DIR";
/// An installed `hues.uddp` to compare with `Hues.uop` too (reported, not asserted: it may come from `hues.mul`).
const INSTALLED_HUES_ENV: &str = "UDD_HUES";
/// Folder under the temporary directory (set `TMPDIR`) the packages are written to.
const OUTPUT_DIR_NAME: &str = "udd-hues-client-tables";

fn client_file(env: &str, names: &[&str]) -> PathBuf {
    let dir = PathBuf::from(std::env::var_os(env).unwrap_or_else(|| panic!("set {env}")));
    find_first_existing_file(&[dir], names).unwrap_or_else(|| panic!("no {names:?} in {env}"))
}

fn output_path(file_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(OUTPUT_DIR_NAME);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(file_name)
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR (and TMPDIR; UDD_HUES to compare) and run with --ignored"]
fn a_hues_package_from_hues_uop_holds_its_tables() {
    let hues_uop = client_file(EC_CLIENT_DIR_ENV, &[HUES_UOP_NAME]);
    let output = output_path("hues_from_uop.uddp");
    convert_hues_uop_to_hues_uddp(&hues_uop, &output, &HuesOptions::default()).unwrap();
    let packed = HuesPackage::load(&output).unwrap().ec_hue_tables();
    let client = EcHueTables::load(&hues_uop).unwrap();

    assert_eq!(packed.tables.keys().collect::<Vec<_>>(), client.tables.keys().collect::<Vec<_>>());
    let differing: Vec<u16> =
        client.tables.iter().filter(|(hue, table)| packed.tables[*hue] != **table).map(|(hue, _)| *hue).collect();
    assert!(differing.is_empty(), "{} tables differ: {differing:?}", differing.len());
    println!("hues.uddp from {}: {} tables, each identical to its bitmap", hues_uop.display(), client.tables.len());

    if let Some(installed) = std::env::var_os(INSTALLED_HUES_ENV) {
        let installed_tables = HuesPackage::load(&installed).unwrap().ec_hue_tables();
        let same = client.tables.iter().filter(|(hue, table)| installed_tables.tables.get(*hue) == Some(*table)).count();
        println!("  {}: {same} of {} tables identical", PathBuf::from(installed).display(), client.tables.len());
    }
}

#[test]
#[ignore = "needs a real Classic Client: set UO_CC_CLIENT_DIR (and TMPDIR; UO_EC_CLIENT_DIR to compare) and run with --ignored"]
fn a_hues_package_from_hues_mul_holds_its_ramps() {
    let hues_mul = client_file(CC_CLIENT_DIR_ENV, &["hues.mul"]);
    let hues = load_hues(&hues_mul).unwrap();
    let in_range: Vec<&HueEntry> = hues.iter().filter(|hue| (1..=u32::from(MAX_HUE_ID)).contains(&hue.id)).collect();
    let client = std::env::var_os(EC_CLIENT_DIR_ENV)
        .map(|_| EcHueTables::load(client_file(EC_CLIENT_DIR_ENV, &[HUES_UOP_NAME])).unwrap());

    for ramp in [HueRampMode::Linear, HueRampMode::Flat] {
        let output = output_path(&format!("hues_from_mul_{ramp:?}.uddp").to_lowercase());
        convert_hues_mul_to_hues_uddp(&hues_mul, &output, &HuesOptions { ramp, ..Default::default() }).unwrap();
        let packed = HuesPackage::load(&output).unwrap().ec_hue_tables();

        assert_eq!(packed.tables.len(), in_range.len(), "{ramp:?}: hues held");
        let differing: Vec<u32> = in_range
            .iter()
            .filter(|hue| packed.tables[&(hue.id as u16)].entries != hue_palette_row(&hue.color_table, ramp))
            .map(|hue| hue.id)
            .collect();
        assert!(differing.is_empty(), "{ramp:?}: {} tables differ: {differing:?}", differing.len());
        println!("hues.uddp from {} ({ramp:?}): {} tables, each the ramp of its colours", hues_mul.display(), in_range.len());

        if let Some(client) = &client {
            let same = client.tables.iter().filter(|(hue, table)| packed.tables.get(*hue) == Some(*table)).count();
            println!("  {same} of {} Hues.uop tables are identical", client.tables.len());
        }
    }
}
