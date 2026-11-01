//! The `GameData.uop` CSV tables of the Enhanced Client (`UO_EC_CLIENT_DIR`) and Kingdom Reborn
//! (`UO_KR_CLIENT_DIR`): every table reads, writes back byte for byte and keeps its own shape, and the typed tables
//! read every record.

use std::path::PathBuf;

use uocf::enhanced::entry_edit::entry_kind;
use uocf::enhanced::game_data::{keeps_shape, BuffDefinition, GameDataTable, LegacyContainer, MobArt, WeaponAbility};
use uocf::enhanced::image_census::ContentFormat;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

const CLIENT_DIR_ENVS: [&str; 2] = ["UO_EC_CLIENT_DIR", "UO_KR_CLIENT_DIR"];
const GAME_DATA_FILE: &str = "GameData.uop";

/// Text tables per client, as measured.
const SHIPPED_TABLES: [(&str, usize); 2] = [("UO_EC_CLIENT_DIR", 78), ("UO_KR_CLIENT_DIR", 63)];
/// Records of the typed tables per client: legacy containers (EC only), weapon abilities, mob art, buffs.
const SHIPPED_RECORDS: [(&str, Option<usize>, usize, usize, usize); 2] =
    [("UO_EC_CLIENT_DIR", Some(70), 241, 335, 194), ("UO_KR_CLIENT_DIR", None, 159, 335, 49)];

fn table(package: &UopPackage, entry: &str) -> Option<GameDataTable> {
    let payload = package.unpack_file_by_hash(hash_file_name_single(entry)).unwrap()?;
    Some(GameDataTable::parse(entry, &payload).unwrap())
}

#[test]
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR and/or UO_KR_CLIENT_DIR and run with --ignored"]
fn every_game_data_table_reads_and_writes_back() {
    let dirs: Vec<(&str, PathBuf)> = CLIENT_DIR_ENVS.into_iter().filter_map(|env| std::env::var_os(env).map(|dir| (env, PathBuf::from(dir)))).collect();
    assert!(!dirs.is_empty(), "set {}", CLIENT_DIR_ENVS.join(" and/or "));
    for (env, dir) in dirs {
        let package = UopPackage::load(dir.join(GAME_DATA_FILE)).unwrap();
        let mut tables = 0;
        for file in package.iter_files().filter(|file| !file.is_empty_slot()) {
            let payload = package.unpack_file_by_hash(file.filename_hash()).unwrap().unwrap();
            if entry_kind(&payload) != ContentFormat::Text {
                continue;
            }
            let label = format!("{env} {:016X}", file.filename_hash());
            let table = GameDataTable::parse(&label, &payload).unwrap();
            assert!(table.to_bytes().unwrap() == payload, "{label} writes back byte for byte");
            assert!(table.header().is_some(), "{label} has column names");
            keeps_shape(&payload, &payload).unwrap_or_else(|error| panic!("{label}: {error:#}"));
            tables += 1;
        }
        let expected = SHIPPED_TABLES.iter().find(|(name, _)| *name == env).unwrap().1;
        assert_eq!(tables, expected, "{env} tables");

        let (_, containers, abilities, art, buffs) = *SHIPPED_RECORDS.iter().find(|record| record.0 == env).unwrap();
        let read = table(&package, LegacyContainer::ENTRY).map(|table| LegacyContainer::read(&table).unwrap().len());
        assert_eq!(read, containers, "{env} legacy containers");
        assert_eq!(WeaponAbility::read(&table(&package, WeaponAbility::ENTRY).unwrap()).unwrap().len(), abilities, "{env} weapon abilities");
        assert_eq!(MobArt::read(&table(&package, MobArt::ENTRY).unwrap()).unwrap().len(), art, "{env} mob art");
        assert_eq!(BuffDefinition::read(&table(&package, BuffDefinition::ENTRY).unwrap()).unwrap().len(), buffs, "{env} buffs");
        println!("{env}: {tables} tables read and written back");
    }
}
