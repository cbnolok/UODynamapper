//! The Classic remap def files of real clients of every era read strictly as the client reads them
//! (`classic::def_files`) and write back byte for byte; an edited `body.def` and `bodyconv.def` change only the
//! edited line and read back through the renderer's readers (`BodyDef`, `BodyConvDef`).
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. The edited files go to a temporary directory, removed at the end. Run with `--ignored`.

use std::path::{Path, PathBuf};

use uocf::classic::body_def::BodyDef;
use uocf::classic::bodyconv_def::BodyConvDef;
use uocf::classic::def_files::{BodyConvDefFile, DefDocument, DefRecord, EquipConvRecord, TargetsDefFile, TargetsRecord};
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
const TARGETS_FILES: [&str; 9] =
    ["anim1.def", "anim2.def", "anim3.def", "anim4.def", "art.def", "body.def", "corpse.def", "gump.def", "texterr.def"];
const BODY_DEF: &str = "body.def";
const BODYCONV_DEF: &str = "bodyconv.def";
const EQUIPCONV_DEF: &str = "equipconv.def";
/// The hue the `body.def` edit sets.
const EDITED_HUE: u16 = 0x0021;
/// The `anim2.mul` body the `bodyconv.def` edit sets.
const EDITED_ANIM2_BODY: i32 = 7;

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Reads `name` strictly and checks it writes back byte for byte; `None` when the client lacks it.
fn read_back<R: DefRecord>(root: &Path, name: &str, client: &str) -> Option<(DefDocument<R>, Vec<u8>)> {
    let path = find_file_ignoring_case(root, name)?;
    let bytes = std::fs::read(&path).unwrap();
    let file = DefDocument::<R>::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} {name}: {error}"));
    assert_eq!(file.to_bytes(), bytes, "{client} {name}");
    println!("{client}: {name} ({} records) written back byte for byte", file.records().count());
    Some((file, bytes))
}

/// The lines of `before` and `after` that differ.
fn changed_lines(before: &[u8], after: &[u8]) -> Vec<usize> {
    let (before, after): (Vec<_>, Vec<_>) = (before.split(|&b| b == b'\n').collect(), after.split(|&b| b == b'\n').collect());
    assert_eq!(before.len(), after.len());
    (0..before.len()).filter(|&i| before[i] != after[i]).collect()
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_def_files_read_strictly_and_write_back_byte_for_byte() {
    let out = std::env::temp_dir().join(format!("uocf_classic_def_files_{}", std::process::id()));
    let mut files = 0;
    for root in client_roots() {
        let client = label(&root);
        let dir = out.join(&client);
        std::fs::create_dir_all(&dir).unwrap();
        for name in TARGETS_FILES {
            files += usize::from(read_back::<TargetsRecord>(&root, name, &client).is_some());
        }
        files += usize::from(read_back::<EquipConvRecord>(&root, EQUIPCONV_DEF, &client).is_some());

        // body.def: a new hue for the first remap, read back by BodyDef.
        let (mut body, bytes): (TargetsDefFile, _) = read_back(&root, BODY_DEF, &client).unwrap();
        let record = body.records_mut().next().unwrap();
        let (id, single) = (record.id, record.targets.len() == 1);
        record.value = i32::from(EDITED_HUE);
        let path = dir.join(BODY_DEF);
        body.save(&path).unwrap();
        assert_eq!(changed_lines(&bytes, &std::fs::read(&path).unwrap()).len(), 1, "{client} {BODY_DEF}");
        let reread = BodyDef::load(&path).unwrap();
        assert_eq!(reread.resolve(id).unwrap().hue, EDITED_HUE, "{client}: BodyDef reads the edit");
        if single {
            assert_eq!(i32::from(reread.resolve(id).unwrap().graphic), body.resolve(id).unwrap().targets[0]);
        }

        // bodyconv.def: a new anim2 body for the last line, read back by BodyConvDef.
        let (mut conv, bytes): (BodyConvDefFile, _) = read_back(&root, BODYCONV_DEF, &client).unwrap();
        let record = conv.records_mut().last().unwrap();
        let conv_body = record.body;
        record.anim_file_bodies.iter_mut().for_each(|body| *body = -1);
        record.anim_file_bodies[0] = EDITED_ANIM2_BODY;
        let path = dir.join(BODYCONV_DEF);
        conv.save(&path).unwrap();
        assert_eq!(changed_lines(&bytes, &std::fs::read(&path).unwrap()).len(), 1, "{client} {BODYCONV_DEF}");
        let reread = BodyConvDef::load(&path).unwrap();
        let entry = reread.resolve(conv_body).unwrap();
        assert_eq!((entry.file_index, i32::from(entry.graphic)), (1, EDITED_ANIM2_BODY), "{client}: BodyConvDef reads the edit");
        files += 2;
    }
    std::fs::remove_dir_all(&out).unwrap();
    assert!(files > 0);
}
