//! Importing images into Classic art and gumps on real clients ([`ClassicImageFiles`]), in each container found:
//! records exported as images import back to the same pixels (and, but for the padding of old memory, the same
//! bytes, so a copy is unchanged), and a replaced record changes only itself in the copy.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`). The copies go to the temporary
//! folder and are removed. Run with `--ignored --release`.

use std::path::PathBuf;

use uocf::classic::art_write::STATIC_ART_ID_BASE;
use uocf::classic::image_files::{ClassicImageContainer, ClassicImageFiles, ClassicImageKind, ImageWrite};
use uocf::enhanced::dds_writer::SizeChange;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// Records sampled per kind (land, statics, gumps): the first ones present from each start id.
const SAMPLED: usize = 40;
/// Where the sampling of each kind starts.
const ART_STARTS: [u32; 2] = [0, STATIC_ART_ID_BASE];
const GUMP_STARTS: [u32; 1] = [0];
/// Ids searched past a start for present records.
const SEARCHED: u32 = 0x2000;
const OPAQUE: u8 = u8::MAX;

#[test]
#[ignore]
fn exported_art_and_gumps_import_back_and_a_copy_changes_one_record() {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let out = std::env::temp_dir().join(format!("uocf_classic_image_import_{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    for root in std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()) {
        for (kind, starts) in [(ClassicImageKind::Art, &ART_STARTS[..]), (ClassicImageKind::Gump, &GUMP_STARTS[..])] {
            for files in ClassicImageFiles::find(&root, kind) {
                let (mut sampled, mut same_bytes) = (0, 0);
                let mut edited = None;
                for &start in starts {
                    let mut taken = 0;
                    for id in start..start + SEARCHED {
                        if taken == SAMPLED {
                            break;
                        }
                        let Ok(Some(stored)) = files.read(id) else { continue };
                        let image = image::RgbaImage::from_raw(stored.width, stored.height, stored.rgba.clone()).unwrap();
                        let prepared = files.prepare(id, &image, SizeChange::Refuse).unwrap_or_else(|error| panic!("{} {id}: {error:#}", files.container.label()));
                        assert_eq!(prepared.rgba, stored.rgba, "{} {id}: the pixels differ", files.container.label());
                        assert!(prepared.replaces);
                        taken += 1;
                        sampled += 1;
                        let probe = files.container.in_dir(&out);
                        if edited.is_none() {
                            // Darken the first opaque pixel of the first record and write a copy.
                            let mut changed = image.clone();
                            if let Some(pixel) = changed.pixels_mut().find(|pixel| pixel[3] == OPAQUE) {
                                pixel[0] ^= 0x80;
                            }
                            let edit = files.prepare(id, &changed, SizeChange::Refuse).unwrap();
                            assert_eq!(files.write_copy(&edit, &probe).unwrap(), ImageWrite::Replaced);
                            edited = Some((id, edit.rgba, probe));
                        } else {
                            same_bytes += usize::from(stored_bytes_match(&files, id, &prepared));
                        }
                    }
                }
                let (id, rgba, probe) = edited.unwrap_or_else(|| panic!("{}: no record", files.container.label()));
                let copy = ClassicImageFiles { kind, container: probe.clone() };
                assert_eq!(copy.read(id).unwrap().unwrap().rgba, rgba);
                for other in [id + 1, id + 2] {
                    assert_eq!(copy.read(other).unwrap(), files.read(other).unwrap(), "{other} changed in the copy");
                }
                println!(
                    "{}: {sampled} records import back to the same pixels, {same_bytes} of the {} others to the same bytes; record {id} replaced in a copy",
                    files.container.label(),
                    sampled - 1
                );
                for path in container_paths(&probe) {
                    std::fs::remove_file(path).unwrap();
                }
            }
        }
    }
    std::fs::remove_dir_all(out).unwrap();
}

/// Whether writing `prepared` would leave the record as it is (the encoder reproduced its bytes).
fn stored_bytes_match(files: &ClassicImageFiles, id: u32, prepared: &uocf::classic::image_files::PreparedImage) -> bool {
    files.stored_record(id).unwrap().is_some_and(|record| record == prepared.uop_record)
}

fn container_paths(container: &ClassicImageContainer) -> Vec<PathBuf> {
    match container {
        ClassicImageContainer::Mul { index, data } => vec![index.clone(), data.clone()],
        ClassicImageContainer::Uop(path) => vec![path.clone()],
    }
}
