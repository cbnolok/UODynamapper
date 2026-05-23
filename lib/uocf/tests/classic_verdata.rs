use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use uocf::classic::anim::AnimMap;
use uocf::classic::animdata::{AnimData, ANIMDATA_CHUNK_SIZE, ANIMDATA_RECORD_SIZE};
use uocf::classic::art::{ArtMap, LAND_DIAMOND_PIXEL_COUNT};
use uocf::classic::gump::GumpMap;
use uocf::classic::land_texture::TexMap;
use uocf::classic::light::LightMap;
use uocf::classic::map::{MapBlockRelPos, MapPlane, MapSizeCells};
use uocf::classic::multi::MultiMap;
use uocf::classic::sound::SoundMap;
use uocf::classic::statics::StaticsReader;
use uocf::classic::tiledata::TileData;
use uocf::classic::verdata::{VerFileId, Verdata};

mod common;

#[test]
fn verdata_static_patch_overrides_base_static_block() {
    let dir = common::temp_dir("classic_verdata");
    let idx_path = dir.join("staidx0.mul");
    let mul_path = dir.join("statics0.mul");
    let verdata_path = dir.join("verdata.mul");

    let mut idx = fs::File::create(&idx_path).unwrap();
    common::write_index_entry(&mut idx, u32::MAX, 0, u32::MAX);
    common::write_index_entry(&mut idx, 0, 7, 0);
    fs::write(&mul_path, common::static_tile(100, 1, 2, 3, 4)).unwrap();

    let patch = common::static_tile(300, 4, 5, 6, 7);
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
    let dir = common::temp_dir("classic_verdata_map");
    let map_path = dir.join("map0.mul");
    let verdata_path = dir.join("verdata.mul");

    fs::write(&map_path, common::map_block(1, 2)).unwrap();
    common::write_verdata(&verdata_path, &[(VerFileId::Map, 0, 0, common::map_block(77, -7))]);

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
    let dir = common::temp_dir("classic_verdata_payloads");
    let verdata_path = dir.join("verdata.mul");

    let mut invalid_idx = fs::File::create(dir.join("artidx.mul")).unwrap();
    common::write_index_entry(&mut invalid_idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("art.mul"), [0u8]).unwrap();

    let mut invalid_gump_idx = fs::File::create(dir.join("gumpidx.mul")).unwrap();
    common::write_index_entry(&mut invalid_gump_idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("gumpart.mul"), [0u8]).unwrap();

    let mut invalid_sound_idx = fs::File::create(dir.join("soundidx.mul")).unwrap();
    common::write_index_entry(&mut invalid_sound_idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("sound.mul"), [0u8]).unwrap();

    let mut invalid_multi_idx = fs::File::create(dir.join("multi.idx")).unwrap();
    common::write_index_entry(&mut invalid_multi_idx, u32::MAX, 0, u32::MAX);
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

    common::write_verdata(
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
    let dir = common::temp_dir("classic_verdata_record_payloads");
    let verdata_path = dir.join("verdata.mul");

    let mut light_idx = fs::File::create(dir.join("lightidx.mul")).unwrap();
    common::write_index_entry(&mut light_idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("light.mul"), [0u8]).unwrap();

    let mut texidx = fs::File::create(dir.join("texidx.mul")).unwrap();
    for _ in 0..0x1388 {
        common::write_index_entry(&mut texidx, u32::MAX, 0, u32::MAX);
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
    common::write_verdata(
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
    let dir = common::temp_dir("classic_verdata_anim");
    let verdata_path = dir.join("verdata.mul");

    let mut idx = fs::File::create(dir.join("anim.idx")).unwrap();
    common::write_index_entry(&mut idx, u32::MAX, 0, u32::MAX);
    fs::write(dir.join("anim.mul"), [0u8]).unwrap();

    let mut anim_payload = vec![0u8; 256 * 2];
    anim_payload.extend_from_slice(&0u32.to_le_bytes());
    common::write_verdata(&verdata_path, &[(VerFileId::Anim, 0, 0, anim_payload)]);
    let verdata = Arc::new(Verdata::load(&verdata_path).unwrap());

    let anims = AnimMap::load(&dir).unwrap().with_verdata(verdata);
    assert!(anims.has_anim(0, 0));
    assert!(anims.decode_animation(0, 0).unwrap().is_empty());

    let _ = fs::remove_dir_all(dir);
}
