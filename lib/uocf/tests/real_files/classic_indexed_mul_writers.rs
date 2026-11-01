//! `multi.mul`, `light.mul` and `texmaps.mul` with their index files on real clients of every era: each pair
//! reads and writes back byte for byte, and an edit that grows one entry changes only that entry, moves the
//! later ones, and reads back through the renderer's readers (`MultiMap`, `LightMap`, `TexMap`).
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. The edited files go to a temporary directory, removed at the end. Run with `--ignored --release`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use uocf::classic::indexed_mul::IndexedMul;
use uocf::classic::land_texture::{LandTextureSize, TexMap};
use uocf::classic::light::LightMap;
use uocf::classic::light_write::{LIGHT_DATA_FILE, LIGHT_INDEX_FILE, LightMulFile};
use uocf::classic::multi::MultiMap;
use uocf::classic::multi_write::{MULTI_DATA_FILE, MULTI_INDEX_FILES, MultiMulFile};
use uocf::classic::texmap_write::{TEXMAP_DATA_FILE, TEXMAP_INDEX_FILE, TexmapMulFile, TexmapRecord};
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// RGBA bytes per pixel of the renderer readers' output.
const RGBA_CHANNELS: usize = 4;
/// A light pixel the edit sets, as `decode_light_from_raw` reads it: intensity 31, white and opaque.
const FULL_INTENSITY: u8 = 0x1F;
const FULL_INTENSITY_RGBA: [u8; 4] = [255, 255, 255, 255];

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn read_pair(root: &Path, index: &str, data: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let index = find_file_ignoring_case(root, index)?;
    let data = find_file_ignoring_case(root, data)?;
    Some((std::fs::read(index).unwrap(), std::fs::read(data).unwrap()))
}

/// Every entry but `edited` keeps its record (lookup aside) and payload.
fn assert_only_entry_changed(before: &IndexedMul, after: &IndexedMul, edited: usize, what: &str) {
    assert_eq!(before.entries.len(), after.entries.len(), "{what}");
    for (id, (a, b)) in before.entries.iter().zip(&after.entries).enumerate() {
        if id == edited {
            assert_ne!(a, b, "{what}: entry {id} should have changed");
        } else {
            assert_eq!(a, b, "{what}: entry {id} changed");
        }
    }
}

fn check_multis(root: &Path, name: &str, out: &Path) -> bool {
    let Some(index_name) = MULTI_INDEX_FILES.iter().find(|n| find_file_ignoring_case(root, n).is_some()) else { return false };
    let Some((index, data)) = read_pair(root, index_name, MULTI_DATA_FILE) else { return false };
    let mut file = MultiMulFile::from_bytes(&index, &data).unwrap();
    assert_eq!(file.to_bytes().unwrap(), (index, data), "{name}: {MULTI_DATA_FILE}");
    let present: Vec<usize> = (0..file.len()).filter(|&id| file.parts(id).unwrap().is_some()).collect();
    println!("{name}: {MULTI_DATA_FILE} ({} of {} multis, {}-byte parts) written back byte for byte", present.len(), file.len(), file.part_bytes);

    // Grow one multi by a part, two from the end so a later multi moves.
    let edited = present[present.len() - 2];
    let mut parts = file.parts(edited).unwrap().unwrap();
    let mut added = parts[0];
    added.part.z += 1;
    parts.push(added);
    let before = file.container.clone();
    file.set_parts(edited, &parts).unwrap();
    assert_only_entry_changed(&before, &file.container, edited, name);
    let (index_path, data_path) = (out.join("multi.idx"), out.join(MULTI_DATA_FILE));
    file.save(&index_path, &data_path).unwrap();
    let reader = MultiMap::load(out).unwrap();
    let read: Vec<_> = reader.get_parts(edited as u32).unwrap();
    assert_eq!(read, parts.iter().map(|r| r.part).collect::<Vec<_>>(), "{name}: MultiMap reads the edit");
    let later = present[present.len() - 1];
    let original = MultiMulFile::from_bytes(&before.to_bytes().unwrap().0, &before.to_bytes().unwrap().1).unwrap();
    let later_parts: Vec<_> = original.parts(later).unwrap().unwrap().iter().map(|r| r.part).collect();
    assert_eq!(reader.get_parts(later as u32).unwrap(), later_parts, "{name}: the moved multi");
    true
}

fn check_lights(root: &Path, name: &str, out: &Path) -> bool {
    let Some((index, data)) = read_pair(root, LIGHT_INDEX_FILE, LIGHT_DATA_FILE) else { return false };
    let mut file = LightMulFile::from_bytes(&index, &data).unwrap();
    assert_eq!(file.to_bytes().unwrap(), (index, data), "{name}: {LIGHT_DATA_FILE}");
    let present: Vec<usize> = (0..file.len()).filter(|&id| file.light(id).unwrap().is_some()).collect();
    println!("{name}: {LIGHT_DATA_FILE} ({} of {} lights) written back byte for byte", present.len(), file.len());

    // One more row, its first pixel at full intensity.
    let edited = present[0];
    let mut light = file.light(edited).unwrap().unwrap();
    let width = usize::from(light.width);
    light.height += 1;
    light.intensities.extend(std::iter::once(FULL_INTENSITY).chain(std::iter::repeat_n(0, width - 1)));
    let before = file.container.clone();
    file.set_light(edited, Some(&light)).unwrap();
    assert_only_entry_changed(&before, &file.container, edited, name);
    file.save(&out.join(LIGHT_INDEX_FILE), &out.join(LIGHT_DATA_FILE)).unwrap();
    let reader = LightMap::load(Some(out), None::<&Path>).unwrap();
    let (w, h, rgba) = reader.decode_light(edited as u32).unwrap();
    assert_eq!((w, h), (light.width, light.height), "{name}: LightMap reads the edit");
    let first_added = (usize::from(h) - 1) * width * RGBA_CHANNELS;
    assert_eq!(rgba[first_added..first_added + RGBA_CHANNELS], FULL_INTENSITY_RGBA, "{name}: the added pixel");
    let later = present[1];
    let original = LightMap::load(Some(root), None::<&Path>).unwrap();
    assert_eq!(reader.decode_light(later as u32).unwrap(), original.decode_light(later as u32).unwrap(), "{name}: the moved light");
    true
}

fn check_textures(root: &Path, name: &str, out: &Path) -> bool {
    let Some((index, data)) = read_pair(root, TEXMAP_INDEX_FILE, TEXMAP_DATA_FILE) else { return false };
    let mut file = TexmapMulFile::from_bytes(&index, &data).unwrap();
    assert_eq!(file.to_bytes().unwrap(), (index, data), "{name}: {TEXMAP_DATA_FILE}");
    let present: Vec<usize> = (0..file.len()).filter(|&id| file.texture(id).unwrap().is_some()).collect();
    println!("{name}: {TEXMAP_DATA_FILE} ({} of {} textures) written back byte for byte", present.len(), file.len());

    // A small texture becomes a big one from an RGBA image, so the later textures move by 24 KB.
    let edited = *present.iter().find(|&&id| file.texture(id).unwrap().unwrap().size == LandTextureSize::Small).unwrap();
    let (big_w, big_h) = LandTextureSize::Big.dimensions();
    let rgba: Vec<u8> = (0..big_w * big_h).flat_map(|i| [(i % 256) as u8, (i / 256) as u8, 0x80, 0xFF]).collect();
    let texture = TexmapRecord::from_rgba(big_w, big_h, &rgba).unwrap();
    let before = file.container.clone();
    file.set_texture(edited, Some(&texture)).unwrap();
    assert_only_entry_changed(&before, &file.container, edited, name);
    let (index_path, data_path) = (out.join(TEXMAP_INDEX_FILE), out.join(TEXMAP_DATA_FILE));
    file.save(&index_path, &data_path).unwrap();
    let reader = TexMap::load(data_path, index_path).unwrap();
    let original = TexMap::load(find_file_ignoring_case(root, TEXMAP_DATA_FILE).unwrap(), find_file_ignoring_case(root, TEXMAP_INDEX_FILE).unwrap()).unwrap();
    let now = Instant::now();
    let pixels = reader.get_pixel_data(edited, now).unwrap();
    assert_eq!(pixels.len(), (big_w * big_h) as usize * RGBA_CHANNELS, "{name}: TexMap reads the edit as 128x128");
    let later = *present.last().unwrap();
    assert_eq!(reader.get_pixel_data(later, now), original.get_pixel_data(later, now), "{name}: the moved texture");
    true
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored --release"]
fn classic_multi_light_and_texmap_files_write_back_byte_for_byte_and_edit() {
    let out = std::env::temp_dir().join(format!("uocf_indexed_mul_writers_{}", std::process::id()));
    let mut checked = 0;
    for root in client_roots() {
        let name = label(&root);
        let dir = out.join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        checked += [check_multis(&root, &name, &dir), check_lights(&root, &name, &dir), check_textures(&root, &name, &dir)]
            .iter()
            .filter(|&&checked| checked)
            .count();
    }
    std::fs::remove_dir_all(&out).unwrap();
    assert!(checked > 0, "no client had a multi, light or texmap file");
}
