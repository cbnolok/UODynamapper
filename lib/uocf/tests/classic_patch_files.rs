use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use uocf::classic::body_def::BodyDef;
use uocf::classic::bodyconv_def::BodyConvDef;
use uocf::classic::art::{ArtMap, LAND_DIAMOND_PIXEL_COUNT};
use uocf::classic::gump::GumpMap;
use uocf::classic::anim::AnimMap;
use uocf::classic::animdata::{AnimData, ANIMDATA_CHUNK_SIZE, ANIMDATA_RECORD_SIZE};
use uocf::classic::land_texture::TexMap;
use uocf::classic::light::LightMap;
use uocf::classic::multi::MultiMap;
use uocf::classic::sound::SoundMap;
use uocf::classic::tiledata::TileData;
use uocf::classic::map::{MapBlockRelPos, MapPlane, MapSizeCells};
use uocf::classic::map_statics_diff::{MapDiff, StaticDiff};
use uocf::classic::statics::StaticsReader;
use uocf::classic::verdata::{VerFileId, Verdata};

fn temp_dir(test_name: &str) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("uocf_{test_name}_{timestamp}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn map_block(tile_id: u16, z: i8) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(196);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..64 {
        bytes.extend_from_slice(&tile_id.to_le_bytes());
        bytes.push(z as u8);
    }
    bytes
}

fn static_tile(graphic: u16, x: u8, y: u8, z: i8, hue: u16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(7);
    bytes.extend_from_slice(&graphic.to_le_bytes());
    bytes.push(x);
    bytes.push(y);
    bytes.push(z as u8);
    bytes.extend_from_slice(&hue.to_le_bytes());
    bytes
}

fn write_index_entry(file: &mut fs::File, lookup: u32, size: u32, extra: u32) {
    file.write_all(&lookup.to_le_bytes()).unwrap();
    file.write_all(&size.to_le_bytes()).unwrap();
    file.write_all(&extra.to_le_bytes()).unwrap();
}

fn write_verdata(path: &PathBuf, entries: &[(VerFileId, i32, i32, Vec<u8>)]) {
    let header_len = 4 + entries.len() * 20;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(entries.len() as i32).to_le_bytes());

    let mut lookup = header_len as i32;
    for (file_id, index, extra, payload) in entries {
        bytes.extend_from_slice(&(*file_id as i32).to_le_bytes());
        bytes.extend_from_slice(&index.to_le_bytes());
        bytes.extend_from_slice(&lookup.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as i32).to_le_bytes());
        bytes.extend_from_slice(&extra.to_le_bytes());
        lookup += payload.len() as i32;
    }

    for (_file_id, _index, _extra, payload) in entries {
        bytes.extend_from_slice(payload);
    }

    fs::write(path, bytes).unwrap();
}

#[test]
fn body_def_and_bodyconv_def_parse_redirects() {
    let dir = temp_dir("classic_defs");
    let body_path = dir.join("Body.def");
    let bodyconv_path = dir.join("Bodyconv.def");

    fs::write(&body_path, b"0x00C0 { 1 2 0x03E8 } 44 # comment\n").unwrap();
    fs::write(&bodyconv_path, b"0x00C0 -1 400 -1 500\n").unwrap();

    let body = BodyDef::load(&body_path).unwrap();
    let body_entry = body.resolve(0x00C0).unwrap();
    assert_eq!(body_entry.graphic, 1000);
    assert_eq!(body_entry.hue, 44);

    let bodyconv = BodyConvDef::load(&bodyconv_path).unwrap();
    let bodyconv_entry = bodyconv.resolve(0x00C0).unwrap();
    assert_eq!(bodyconv_entry.file_index, 4);
    assert_eq!(bodyconv_entry.graphic, 500);
    assert_eq!(bodyconv_entry.mount_height, 0);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn map_diff_overrides_base_map_block() {
    let dir = temp_dir("classic_map_diff");
    let map_path = dir.join("map0.mul");
    let lookup_path = dir.join("mapdifl0.mul");
    let diff_path = dir.join("mapdif0.mul");

    let mut map_bytes = Vec::new();
    map_bytes.extend_from_slice(&map_block(1, 2));
    map_bytes.extend_from_slice(&map_block(3, 4));
    fs::write(&map_path, map_bytes).unwrap();
    fs::write(&lookup_path, 1u32.to_le_bytes()).unwrap();
    fs::write(&diff_path, map_block(55, -6)).unwrap();

    let diff = MapDiff::load(&lookup_path, &diff_path).unwrap();
    let mut plane = MapPlane::init_with_size_and_diff(
        map_path,
        0,
        Some(MapSizeCells {
            width: 8,
            height: 16,
        }),
        diff,
    )
    .unwrap();

    let mut blocks = [MapBlockRelPos { x: 0, y: 1 }];
    plane.load_blocks(&mut blocks).unwrap();
    let block = plane.block_no_update(MapBlockRelPos { x: 0, y: 1 }).unwrap();
    let cell = block.cell(0, 0).unwrap();
    assert_eq!(cell.id, 55);
    assert_eq!(cell.z, -6);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn static_diff_overrides_base_static_block() {
    let dir = temp_dir("classic_static_diff");
    let idx_path = dir.join("staidx0.mul");
    let mul_path = dir.join("statics0.mul");
    let lookup_path = dir.join("stadifl0.mul");
    let diff_idx_path = dir.join("stadifi0.mul");
    let diff_path = dir.join("stadif0.mul");

    let mut idx = fs::File::create(&idx_path).unwrap();
    write_index_entry(&mut idx, u32::MAX, 0, u32::MAX);
    write_index_entry(&mut idx, 0, 7, 0);
    fs::write(&mul_path, static_tile(100, 1, 2, 3, 4)).unwrap();

    fs::write(&lookup_path, 1u32.to_le_bytes()).unwrap();
    let mut diff_idx = fs::File::create(&diff_idx_path).unwrap();
    write_index_entry(&mut diff_idx, 0, 7, 0);
    fs::write(&diff_path, static_tile(200, 2, 3, -4, 5)).unwrap();

    let diff = StaticDiff::load(&lookup_path, &diff_idx_path, &diff_path).unwrap();
    let reader = StaticsReader::new_with_patches(
        &idx_path,
        &mul_path,
        8,
        16,
        Some(diff),
        None,
    )
    .unwrap();

    let tiles = reader.read_block(0, 1).unwrap();
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0].graphic, 200);
    assert_eq!(tiles[0].z, -4);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn verdata_static_patch_overrides_base_static_block() {
    let dir = temp_dir("classic_verdata");
    let idx_path = dir.join("staidx0.mul");
    let mul_path = dir.join("statics0.mul");
    let verdata_path = dir.join("verdata.mul");

    let mut idx = fs::File::create(&idx_path).unwrap();
    write_index_entry(&mut idx, u32::MAX, 0, u32::MAX);
    write_index_entry(&mut idx, 0, 7, 0);
    fs::write(&mul_path, static_tile(100, 1, 2, 3, 4)).unwrap();

    let patch = static_tile(300, 4, 5, 6, 7);
    let mut verdata_bytes = Vec::new();
    verdata_bytes.extend_from_slice(&1i32.to_le_bytes());
    verdata_bytes.extend_from_slice(&(VerFileId::Statics as i32).to_le_bytes());
    verdata_bytes.extend_from_slice(&1i32.to_le_bytes());
    verdata_bytes.extend_from_slice(&24i32.to_le_bytes());
    verdata_bytes.extend_from_slice(&(patch.len() as i32).to_le_bytes());
    verdata_bytes.extend_from_slice(&0i32.to_le_bytes());
    verdata_bytes.extend_from_slice(&patch);
    fs::write(&verdata_path, verdata_bytes).unwrap();

    let verdata = Arc::new(Verdata::load(&verdata_path).unwrap());
    let reader = StaticsReader::new_with_patches(
        &idx_path,
        &mul_path,
        8,
        16,
        None,
        Some(verdata),
    )
    .unwrap();

    let tiles = reader.read_block(0, 1).unwrap();
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0].graphic, 300);
    assert_eq!(tiles[0].hue, 7);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn verdata_map_patch_overrides_base_map_block() {
    let dir = temp_dir("classic_verdata_map");
    let map_path = dir.join("map0.mul");
    let verdata_path = dir.join("verdata.mul");

    fs::write(&map_path, map_block(1, 2)).unwrap();
    write_verdata(&verdata_path, &[(VerFileId::Map, 0, 0, map_block(77, -7))]);

    let verdata = Arc::new(Verdata::load(&verdata_path).unwrap());
    let mut plane = MapPlane::init_with_size(
        map_path,
        0,
        Some(MapSizeCells {
            width: 8,
            height: 8,
        }),
    )
    .unwrap()
    .with_verdata(verdata);

    let mut blocks = [MapBlockRelPos { x: 0, y: 0 }];
    plane.load_blocks(&mut blocks).unwrap();
    let cell = plane
        .block_no_update(MapBlockRelPos { x: 0, y: 0 })
        .unwrap()
        .cell(0, 0)
        .unwrap();
    assert_eq!(cell.id, 77);
    assert_eq!(cell.z, -7);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn verdata_art_gump_sound_and_multi_payloads_override_base_files() {
    let dir = temp_dir("classic_verdata_payloads");
    let verdata_path = dir.join("verdata.mul");

    let mut invalid_idx = fs::File::create(dir.join("artidx.mul")).unwrap();
    write_index_entry(&mut invalid_idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("art.mul"), [0u8]).unwrap();

    let mut invalid_gump_idx = fs::File::create(dir.join("gumpidx.mul")).unwrap();
    write_index_entry(&mut invalid_gump_idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("gumpart.mul"), [0u8]).unwrap();

    let mut invalid_sound_idx = fs::File::create(dir.join("soundidx.mul")).unwrap();
    write_index_entry(&mut invalid_sound_idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("sound.mul"), [0u8]).unwrap();

    let mut invalid_multi_idx = fs::File::create(dir.join("multi.idx")).unwrap();
    write_index_entry(&mut invalid_multi_idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("multi.mul"), [0u8]).unwrap();

    let art_payload = vec![0u8; LAND_DIAMOND_PIXEL_COUNT * 2];
    let mut gump_payload = Vec::new();
    gump_payload.extend_from_slice(&1u32.to_le_bytes());
    gump_payload.extend_from_slice(&0x7fffu16.to_le_bytes());
    gump_payload.extend_from_slice(&1u16.to_le_bytes());
    let mut sound_payload = vec![0u8; 32];
    sound_payload[..4].copy_from_slice(b"bell");
    sound_payload.extend_from_slice(&[1, 2, 3, 4]);
    let mut multi_payload = Vec::new();
    multi_payload.extend_from_slice(&0x1234u16.to_le_bytes());
    multi_payload.extend_from_slice(&1i16.to_le_bytes());
    multi_payload.extend_from_slice(&2i16.to_le_bytes());
    multi_payload.extend_from_slice(&3i16.to_le_bytes());
    multi_payload.extend_from_slice(&4u32.to_le_bytes());

    write_verdata(
        &verdata_path,
        &[
            (VerFileId::Art, 0, 0, art_payload.clone()),
            (VerFileId::Gumpart, 0, ((1 << 16) | 1), gump_payload.clone()),
            (VerFileId::Sound, 0, 0, sound_payload),
            (VerFileId::Multi, 0, 0, multi_payload),
        ],
    );
    let verdata = Arc::new(Verdata::load(&verdata_path).unwrap());

    let art = ArtMap::load(&dir).unwrap().with_verdata(Arc::clone(&verdata));
    let mut scratch = Vec::new();
    art.get_raw_art_data_from_source(0, uocf::classic::art::ArtSource::Mul, &mut scratch)
        .unwrap();
    assert_eq!(scratch.len(), art_payload.len());

    let gumps = GumpMap::load(&dir).unwrap().with_verdata(Arc::clone(&verdata));
    let (width, height, pixels) = gumps.decode_gump(0, &mut scratch).unwrap();
    assert_eq!((width, height, pixels.len()), (1, 1, 4));

    let sounds = SoundMap::load(&dir).unwrap().with_verdata(Arc::clone(&verdata));
    let sound = sounds.read_slot(0).unwrap().unwrap();
    assert_eq!(sound.name, "bell");
    assert_eq!(sound.pcm_data, [1, 2, 3, 4]);

    let multis = MultiMap::load(&dir).unwrap().with_verdata(verdata);
    let parts = multis.get_parts(0).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].item_id, 0x1234);
    assert_eq!(parts[0].z, 3);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn verdata_light_texmap_tiledata_and_animdata_payloads_override_base_files() {
    let dir = temp_dir("classic_verdata_record_payloads");
    let verdata_path = dir.join("verdata.mul");

    let mut light_idx = fs::File::create(dir.join("lightidx.mul")).unwrap();
    write_index_entry(&mut light_idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("light.mul"), [0u8]).unwrap();

    let mut texidx = fs::File::create(dir.join("texidx.mul")).unwrap();
    for _ in 0..0x1388 {
        write_index_entry(&mut texidx, u32::MAX, 0, u32::MAX);
    }
    fs::write(dir.join("texmaps.mul"), [0u8]).unwrap();

    let tiledata_len = 512 * (4 + 26 * 32) + 512 * (4 + 37 * 32);
    fs::write(dir.join("tiledata.mul"), vec![0u8; tiledata_len]).unwrap();
    let mut land_tile_patch = vec![0u8; 26];
    land_tile_patch[4..6].copy_from_slice(&0x1234u16.to_le_bytes());

    let animdata_base = vec![0u8; ANIMDATA_CHUNK_SIZE];
    let mut animdata_patch = vec![0u8; ANIMDATA_RECORD_SIZE];
    animdata_patch[65] = 1;
    animdata_patch[66] = 2;
    fs::write(dir.join("animdata.mul"), &animdata_base).unwrap();

    let texmap_patch = vec![0u8; 0x2000];
    write_verdata(
        &verdata_path,
        &[
            (VerFileId::Light, 0, 0x00010001, vec![0x1f]),
            (VerFileId::Texmaps, 0, 0, texmap_patch),
            (VerFileId::Tiledata, 0, 0, land_tile_patch),
            (VerFileId::Animdata, 0, 0, animdata_patch),
        ],
    );
    let verdata = Arc::new(Verdata::load(&verdata_path).unwrap());

    let lights = LightMap::load_with_verdata(
        Some(&dir),
        Option::<&PathBuf>::None,
        Arc::clone(&verdata),
    )
    .unwrap();
    let (width, height, pixels) = lights.decode_light(0).unwrap();
    assert_eq!((width, height, pixels[0], pixels[3]), (1, 1, 255, 255));

    let texmaps = TexMap::load_with_verdata(
        dir.join("texmaps.mul"),
        dir.join("texidx.mul"),
        Some(Arc::clone(&verdata)),
    )
    .unwrap();
    assert_eq!(
        texmaps.get_pixel_data(0, std::time::Instant::now()).unwrap().len(),
        64 * 64 * 4
    );

    let tiledata =
        TileData::load_with_verdata(dir.join("tiledata.mul"), Some(Arc::clone(&verdata)))
            .unwrap();
    assert_eq!(tiledata.land_tiles()[0].texture_id, 0x1234);

    let animdata = AnimData::load_with_verdata(dir.join("animdata.mul"), verdata).unwrap();
    assert_eq!(animdata.get(0).unwrap().frame_count, 1);
    assert_eq!(animdata.get(0).unwrap().frame_interval, 2);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn verdata_anim_payload_overrides_base_anim_file() {
    let dir = temp_dir("classic_verdata_anim");
    let verdata_path = dir.join("verdata.mul");

    let mut idx = fs::File::create(dir.join("anim.idx")).unwrap();
    write_index_entry(&mut idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("anim.mul"), [0u8]).unwrap();

    let mut anim_payload = vec![0u8; 256 * 2];
    anim_payload.extend_from_slice(&0u32.to_le_bytes());
    write_verdata(&verdata_path, &[(VerFileId::Anim, 0, 0, anim_payload)]);
    let verdata = Arc::new(Verdata::load(&verdata_path).unwrap());

    let anims = AnimMap::load(&dir).unwrap().with_verdata(verdata);
    assert!(anims.has_anim(0, 0));
    assert!(anims.decode_animation(0, 0).unwrap().is_empty());

    let _ = fs::remove_dir_all(dir);
}
