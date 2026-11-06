//! Exporting Classic art and gumps on real clients (`uocf::image_export`), in each container found and each file
//! format: every file imports back (`classic_import_rgba`, then [`ClassicImageFiles::prepare`]) to the stored
//! pixels without a colour rounded, and to the same record bytes as the stored pixels import to.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`). Nothing is written. Run with
//! `--ignored --release`.

use uocf::classic::art_write::{ArtKind, STATIC_ART_ID_BASE};
use uocf::classic::image_files::{ClassicImageFiles, ClassicImageKind};
use uocf::enhanced::dds_writer::SizeChange;
use uocf::image_export::{classic_import_rgba, ImageFileFormat};

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// Records sampled per kind (land, statics, gumps): the first ones present from each start id.
const SAMPLED: usize = 12;
/// Where the sampling of each kind starts.
const ART_STARTS: [u32; 2] = [0, STATIC_ART_ID_BASE];
const GUMP_STARTS: [u32; 1] = [0];
/// Ids searched past a start for present records.
const SEARCHED: u32 = 0x2000;

#[test]
#[ignore]
fn exported_art_and_gumps_import_back_in_every_format() {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    for root in std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()) {
        for (kind, starts) in [(ClassicImageKind::Art, &ART_STARTS[..]), (ClassicImageKind::Gump, &GUMP_STARTS[..])] {
            for files in ClassicImageFiles::find(&root, kind) {
                let label = files.container.label();
                let mut sampled = 0;
                for &start in starts {
                    let mut taken = 0;
                    for id in start..start + SEARCHED {
                        if taken == SAMPLED {
                            break;
                        }
                        let (Ok(Some(stored)), Ok(Some(export))) = (files.read(id), files.read_export(id)) else { continue };
                        let plain = image::RgbaImage::from_raw(stored.width, stored.height, stored.rgba.clone()).unwrap();
                        let plain = files.prepare(id, &plain, SizeChange::Refuse).unwrap_or_else(|error| panic!("{label} {id}: {error:#}"));
                        let land_tile = kind == ClassicImageKind::Art && ArtKind::of_id(id) == ArtKind::Land;
                        for format in ImageFileFormat::ALL {
                            let bytes = export.encode(format).unwrap_or_else(|error| panic!("{label} {id} {format:?}: {error:#}"));
                            let rgba = classic_import_rgba(&bytes, land_tile).unwrap();
                            let prepared = files.prepare(id, &rgba, SizeChange::Refuse).unwrap_or_else(|error| panic!("{label} {id} {format:?}: {error:#}"));
                            assert_eq!(prepared.rgba, stored.rgba, "{label} {id} {format:?}: the pixels differ");
                            assert_eq!(prepared.notes, plain.notes, "{label} {id} {format:?}: the conversion differs");
                            assert_eq!(prepared.uop_record, plain.uop_record, "{label} {id} {format:?}: the record differs");
                        }
                        taken += 1;
                        sampled += 1;
                    }
                }
                assert!(sampled > 0, "{label}: no record");
                println!("{label}: {sampled} records import back from BMP, PNG and WebP to the stored pixels and record");
            }
        }
    }
}
