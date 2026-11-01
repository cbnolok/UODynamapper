use super::*;
use crate::classic::hues::HUE_GROUP_SIZE;
use crate::enhanced::hues::EcHueTable;

/// Groups of the synthetic `hues.mul` (more than one strip of the atlas).
const TEST_GROUPS: usize = 130;
const TEST_TRAILING: &[u8] = b"tail";
const TEST_FILETIME: u64 = 0x01DA_0000_0000_0000;
/// Expanded (8-bit) value of the 5-bit channel maximum.
const MAX_EXPANDED: u8 = 0xf8;
const RED: u16 = 0x7c00;
const GREEN: u16 = 0x03e0;
const BLUE: u16 = 0x001f;
const EA_BITMAP_SIZE: usize = 2102;
/// Offset of the DDS header flags.
const DDS_FLAGS_FIELD: usize = 8;

fn synthetic_hues() -> HuesMul {
    let mut hues = HuesMul::with_group_count(TEST_GROUPS);
    for (index, group) in hues.groups.iter_mut().enumerate() {
        group.header = (index as u32).wrapping_mul(0x9e37_79b9);
    }
    for hue in hues.groups.iter_mut().flat_map(|group| group.entries.iter_mut()) {
        let seed = hue.id;
        for (index, color) in hue.color_table.iter_mut().enumerate() {
            *color = (seed.wrapping_mul(2_654_435_761).rotate_left(index as u32) & 0xffff) as u16;
        }
        hue.table_start = (seed * 3) as u16;
        hue.table_end = (seed * 5) as u16;
        // A name with bytes after its NUL, which only the metadata entry keeps.
        hue.name[..4].copy_from_slice(&(seed as u32).to_le_bytes());
        hue.name[0] = b'H';
        hue.name[HUE_NAME_LEN - 1] = (seed % 251) as u8;
    }
    hues.trailing = TEST_TRAILING.to_vec();
    hues
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("uocf-hues-{}-{unique}-{name}", std::process::id()))
}

fn package_of(entries: &[HuesUopEntry]) -> UopPackage {
    let path = temp_path("Hues.uop");
    write_hues_uop(entries, &path, Some(TEST_FILETIME)).expect("write package");
    let package = UopPackage::load_with_mode(&path, LoadMode::Eager).expect("load package");
    std::fs::remove_file(&path).expect("remove package");
    package
}

#[test]
fn colour_expansion_and_reduction_are_inverse() {
    for color in 0..=COLOR_VALUE_MASK {
        assert_eq!(reduce_hue_color(expand_hue_color(color)), color);
        assert_eq!(expand_hue_color(color | COLOR_FLAG_BIT), expand_hue_color(color));
    }
    assert_eq!(expand_hue_color(RED), [MAX_EXPANDED, 0, 0, OPAQUE]);
    assert_eq!(expand_hue_color(GREEN), [0, MAX_EXPANDED, 0, OPAQUE]);
    assert_eq!(expand_hue_color(BLUE), [0, 0, MAX_EXPANDED, OPAQUE]);
}

#[test]
fn linear_ramp_interpolates_between_anchors_and_holds_the_last_colour() {
    let mut colors = [0u16; HUE_COLOR_COUNT];
    colors[1] = RED;
    colors[HUE_COLOR_COUNT - 1] = BLUE;
    let row = hue_palette_row(&colors, HueRampMode::Linear);
    for (index, &color) in colors.iter().enumerate() {
        assert_eq!(row[index * TEXELS_PER_HUE_COLOR], expand_hue_color(color));
    }
    // Black to red over texels 0..8, then red back to black.
    for weight in 0..TEXELS_PER_HUE_COLOR {
        let red = (usize::from(MAX_EXPANDED) * weight / TEXELS_PER_HUE_COLOR) as u8;
        assert_eq!(row[weight], [red, 0, 0, OPAQUE]);
        assert_eq!(row[TEXELS_PER_HUE_COLOR + weight][0], MAX_EXPANDED - red);
    }
    let last = (HUE_COLOR_COUNT - 1) * TEXELS_PER_HUE_COLOR;
    assert!(row[last..].iter().all(|&texel| texel == expand_hue_color(BLUE)));
}

#[test]
fn flat_ramp_repeats_each_colour_over_its_block() {
    let colors: [u16; HUE_COLOR_COUNT] = std::array::from_fn(|index| (index as u16) << RED_SHIFT);
    let row = hue_palette_row(&colors, HueRampMode::Flat);
    for (x, texel) in row.iter().enumerate() {
        assert_eq!(*texel, expand_hue_color(colors[x / TEXELS_PER_HUE_COLOR]));
    }
    assert_eq!(HueRampMode::default(), HueRampMode::Linear);
}

#[test]
fn swatch_is_the_larger_of_the_first_and_last_colour() {
    let mut colors = [0u16; HUE_COLOR_COUNT];
    colors[0] = BLUE;
    colors[HUE_COLOR_COUNT - 1] = GREEN;
    assert_eq!(hue_swatch_color(&colors), expand_hue_color(GREEN));
    colors[0] = RED;
    assert_eq!(hue_swatch_color(&colors), expand_hue_color(RED));
    // Bit 15 does not take part.
    colors[0] = BLUE | COLOR_FLAG_BIT;
    assert_eq!(hue_swatch_color(&colors), expand_hue_color(GREEN));
}

#[test]
fn hue_bitmap_has_ea_header_and_decodes_to_table_and_swatch() {
    let colors: [u16; HUE_COLOR_COUNT] = std::array::from_fn(|index| (index as u16) * 0x0421);
    let table = hue_palette_row(&colors, HueRampMode::Linear);
    let swatch = hue_swatch_color(&colors);
    let bitmap = encode_hue_bitmap(&table, swatch);
    assert_eq!(bitmap.len(), EA_BITMAP_SIZE);
    assert_eq!(&bitmap[..2], b"BM");
    assert_eq!(u32::from_le_bytes(bitmap[2..6].try_into().unwrap()), EA_BMP_FILE_SIZE_FIELD);

    let (width, height, rgba) = decode_hue_image_to_rgba(&bitmap).expect("decode bitmap");
    assert_eq!((width, height), (HUE_BITMAP_WIDTH, HUE_BITMAP_HEIGHT));
    let (top, bottom) = rgba.split_at(HUE_TABLE_ENTRIES * RGBA_CHANNELS);
    assert_eq!(top, table.concat());
    assert!(bottom.chunks_exact(RGBA_CHANNELS).all(|texel| texel == swatch));
    let client_table = EcHueTable::from_bitmap(&bitmap).expect("client table");
    assert_eq!(client_table.entries, table);
}

#[test]
fn dds_round_trips_with_eas_header() {
    let texels: Vec<Rgba> = (0..SWATCH_GRID_SIDE * SWATCH_GRID_SIDE)
        .map(|index| [index as u8, (index >> 8) as u8, 7, (index * 3) as u8])
        .collect();
    let dds = encode_argb32_dds(SWATCH_GRID_SIDE, SWATCH_GRID_SIDE, &texels);
    assert_eq!(dds.len(), DDS_FILE_HEADER_SIZE + texels.len() * RGBA_CHANNELS);
    assert_eq!(u32::from_le_bytes(dds[DDS_FLAGS_FIELD..DDS_FLAGS_FIELD + 4].try_into().unwrap()), DDS_FLAGS);
    assert_eq!(decode_argb32_dds(&dds).expect("decode"), (SWATCH_GRID_SIDE, SWATCH_GRID_SIDE, texels));
    assert!(decode_argb32_dds(b"DDS ").is_err());
}

#[test]
fn atlas_and_swatch_grid_follow_the_client_layout() {
    let hues = synthetic_hues();
    let atlas = build_hues_atlas(&hues, HueRampMode::Linear);
    let width = HUES_ATLAS_WIDTH as usize;
    assert!(atlas[..HUE_TABLE_ENTRIES].iter().all(|&texel| texel == TRANSPARENT));
    for id in [1u32, 1023, 1024, 1040] {
        let coord = atlas_coord_for_hue(id as u16).unwrap();
        let start = coord.y as usize * width + coord.x as usize;
        let expected = hue_palette_row(&hues.hue(id).unwrap().color_table, HueRampMode::Linear);
        assert_eq!(atlas[start..start + HUE_TABLE_ENTRIES], expected);
    }

    let grid = build_swatch_grid(&hues);
    let side = SWATCH_GRID_SIDE as usize;
    assert_eq!(grid[0], TRANSPARENT);
    for id in [1u32, 63, 64, 1040] {
        let (x, y) = (id as usize / side, id as usize % side);
        assert_eq!(grid[y * side + x], hue_swatch_color(&hues.hue(id).unwrap().color_table));
    }
}

#[test]
fn huenames_lines_stop_names_at_their_nul() {
    let mut hues = HuesMul::with_group_count(1);
    hues.hue_mut(1).unwrap().name[..4].copy_from_slice(b"Blue");
    hues.hue_mut(1).unwrap().name[6] = b'x';
    let csv = encode_huenames_csv(&hues);
    let text = String::from_utf8(csv).unwrap();
    assert!(text.starts_with("1,Blue\r\n2,\r\n"));
    assert_eq!(text.lines().count(), HUES_PER_GROUP);
}

#[test]
fn metadata_round_trips_and_rejects_bad_input() {
    let hues = synthetic_hues();
    let metadata = HuesMulMetadata::from_hues(&hues);
    let bytes = metadata.encode();
    assert_eq!(bytes.len(), METADATA_HEADER_SIZE + TEST_GROUPS * METADATA_GROUP_SIZE + TEST_TRAILING.len());
    assert_eq!(HuesMulMetadata::decode(&bytes).expect("decode"), metadata);

    let mut bad_magic = bytes.clone();
    bad_magic[0] ^= 1;
    assert!(HuesMulMetadata::decode(&bad_magic).is_err());
    let mut bad_version = bytes.clone();
    bad_version[4] = bad_version[4].wrapping_add(1);
    assert!(HuesMulMetadata::decode(&bad_version).is_err());
    assert!(HuesMulMetadata::decode(&bytes[..bytes.len() - 1]).is_err());
    assert!(HuesMulMetadata::decode(&bytes[..METADATA_HEADER_SIZE - 1]).is_err());
}

#[test]
fn own_packages_convert_back_to_the_identical_hues_mul_in_both_modes() {
    let hues = synthetic_hues();
    let source = hues.to_bytes();
    assert_eq!(source.len(), TEST_GROUPS * HUE_GROUP_SIZE + TEST_TRAILING.len());
    for mode in [HueRampMode::Linear, HueRampMode::Flat] {
        let entries = build_hues_uop_entries(&hues, mode);
        assert_eq!(entries.len(), hues.hue_count() + 4);
        assert_eq!(entries[0].hash, FIXED_PALETTE_HASH);
        let package = package_of(&entries);
        for entry in &entries {
            let file = package.get_file_by_hash(entry.hash).expect("entry");
            assert_eq!(file.compression(), CompressionFlag::Zlib);
            assert!(file.parse_data_block_header().is_some(), "every entry has a timestamp header");
            assert_eq!(file.unpack().unwrap(), entry.data);
        }
        let recovered = hues_mul_from_uop_package(&package).expect("recover");
        assert!(recovered.had_metadata);
        assert!(recovered.colors_from_atlas.is_empty() && recovered.missing_colors.is_empty());
        assert_eq!(recovered.hues.to_bytes(), source, "{mode:?}");
    }
}

#[test]
fn packages_without_metadata_keep_colours_and_default_the_rest() {
    let mut hues = synthetic_hues();
    for hue in hues.groups.iter_mut().flat_map(|group| group.entries.iter_mut()) {
        for color in &mut hue.color_table {
            *color &= COLOR_VALUE_MASK;
        }
        apply_default_metadata(hue, Some(b"Hue"));
    }
    let mut entries = build_hues_uop_entries(&hues, HueRampMode::Linear);
    entries.retain(|entry| entry.path.as_deref() != Some(HUES_MUL_METADATA_PATH));
    // A missing bitmap falls back to the atlas.
    let dropped = hue_bitmap_hash(2);
    entries.retain(|entry| entry.hash != dropped);

    let recovered = hues_mul_from_uop_package(&package_of(&entries)).expect("recover");
    assert!(!recovered.had_metadata);
    assert_eq!(recovered.colors_from_atlas, vec![2]);
    // Without metadata the hue count is that of the last bitmap, in whole groups.
    assert_eq!(recovered.hues.hue_count(), hues.hue_count());
    for (expected, actual) in hues.hues().zip(recovered.hues.hues()) {
        assert_eq!(expected, actual);
    }
    assert!(recovered.hues.trailing.is_empty());
    assert!(recovered.hues.groups.iter().all(|group| group.header == 0));
}

/// Low bits an authored EC gradient carries below the 5-bit colours.
const AUTHORED_LOW_BITS: u8 = 0b101;
/// `bfSize` and `biSizeImage` of the KR's other bitmap header, and the bytes it has after the pixels.
const KR_HEADER_FILE_SIZE_FIELD: usize = 2;
const KR_HEADER_IMAGE_SIZE_FIELD: usize = 34;
const KR_HEADER_IMAGE_SIZE: u32 = 2050;
const KR_HEADER_TRAILING: [u8; 2] = [0; 2];
/// `biBitCount` field, and a depth the client refuses.
const BIT_COUNT_FIELD: usize = 28;
const REFUSED_BIT_COUNT: u16 = 24;

fn entry_data<'a>(entries: &'a [HuesUopEntry], hash: u64) -> &'a [u8] {
    &entries.iter().find(|entry| entry.hash == hash).expect("entry").data
}

fn set_entry(entries: &mut [HuesUopEntry], hash: u64, data: Vec<u8>) {
    entries.iter_mut().find(|entry| entry.hash == hash).expect("entry").data = data;
}

#[test]
fn writing_on_a_base_keeps_unedited_hues_byte_for_byte() {
    let (authored, refused, edited, renamed, unnamed, truncated) = (5u32, 6u32, 7u32, 8u32, 3u32, 4u32);
    let mut hues = synthetic_hues();
    hues.hue_mut(unnamed).unwrap().name = [0; HUE_NAME_LEN];
    let truncated_name = b"ABCDEFGHIJKLMNOPQRS";
    let truncated_hue = hues.hue_mut(truncated).unwrap();
    truncated_hue.name = [0; HUE_NAME_LEN];
    truncated_hue.name[..truncated_name.len()].copy_from_slice(truncated_name);

    // The base, as EA ships it: no metadata, one authored bitmap with the KR header, names of an older file.
    let mut base = build_hues_uop_entries(&hues, HueRampMode::Linear);
    base.retain(|entry| entry.path.as_deref() != Some(HUES_MUL_METADATA_PATH));
    let mut table = hue_palette_row(&hues.hue(authored).unwrap().color_table, HueRampMode::Linear);
    for texel in &mut table {
        for channel in &mut texel[..RGBA_CHANNELS - 1] {
            *channel |= AUTHORED_LOW_BITS;
        }
    }
    let swatch = [1, 2, 3, OPAQUE];
    let mut authored_bitmap = encode_hue_bitmap(&table, swatch);
    authored_bitmap.extend_from_slice(&KR_HEADER_TRAILING);
    let file_size = authored_bitmap.len() as u32;
    authored_bitmap[KR_HEADER_FILE_SIZE_FIELD..KR_HEADER_FILE_SIZE_FIELD + 4].copy_from_slice(&file_size.to_le_bytes());
    authored_bitmap[KR_HEADER_IMAGE_SIZE_FIELD..KR_HEADER_IMAGE_SIZE_FIELD + 4]
        .copy_from_slice(&KR_HEADER_IMAGE_SIZE.to_le_bytes());
    set_entry(&mut base, hue_bitmap_hash(authored as u16), authored_bitmap.clone());
    let mut refused_bitmap = entry_data(&base, hue_bitmap_hash(refused as u16)).to_vec();
    refused_bitmap[BIT_COUNT_FIELD..BIT_COUNT_FIELD + 2].copy_from_slice(&REFUSED_BIT_COUNT.to_le_bytes());
    set_entry(&mut base, hue_bitmap_hash(refused as u16), refused_bitmap);
    let mut csv = Vec::new();
    for hue in hues.hues() {
        let line = match hue.id {
            id if id == unnamed => format!("{id}").into_bytes(),
            id if id == truncated => format!("{id},{}T", std::str::from_utf8(truncated_name).unwrap()).into_bytes(),
            id if id == renamed => format!("{id},Old").into_bytes(),
            id => [format!("{id:04},").into_bytes(), hue.name_bytes().to_vec()].concat(),
        };
        csv.extend_from_slice(&line);
        csv.extend_from_slice(CSV_LINE_END);
    }
    set_entry(&mut base, hash_file_name_single(HUENAMES_PATH), csv.clone());
    let base_package = package_of(&base);

    hues.hue_mut(edited).unwrap().color_table[0] ^= RED;
    let (entries, reuse) = build_hues_uop_entries_on_base(&hues, &base_package).expect("build on base");
    assert_eq!(reuse.bitmaps_rebuilt, vec![refused, edited]);
    assert_eq!(reuse.bitmaps_kept, hues.hue_count() - 2);
    assert_eq!(reuse.names_rebuilt, vec![renamed]);
    assert_eq!(reuse.names_kept, hues.hue_count() - 1);

    assert_eq!(entry_data(&entries, hue_bitmap_hash(authored as u16)), authored_bitmap);
    for id in [refused, edited] {
        let hue = hues.hue(id).unwrap();
        let rule = encode_hue_bitmap(&hue_palette_row(&hue.color_table, HueRampMode::Linear), hue_swatch_color(&hue.color_table));
        assert_eq!(entry_data(&entries, hue_bitmap_hash(id as u16)), rule);
    }
    let (_, _, atlas) = decode_argb32_dds(entry_data(&entries, hash_file_name_single(HUES_ATLAS_PATH))).unwrap();
    let coord = atlas_coord_for_hue(authored as u16).unwrap();
    let start = coord.y as usize * HUES_ATLAS_WIDTH as usize + coord.x as usize;
    assert_eq!(atlas[start..start + HUE_TABLE_ENTRIES], table);
    let (_, _, grid) = decode_argb32_dds(entry_data(&entries, FIXED_PALETTE_HASH)).unwrap();
    let side = SWATCH_GRID_SIDE as usize;
    assert_eq!(grid[(authored as usize % side) * side + authored as usize / side], swatch);
    // Every name line but the renamed hue's is the base's, in the base's spelling.
    let names = String::from_utf8_lossy(entry_data(&entries, hash_file_name_single(HUENAMES_PATH))).to_string();
    let base_names = String::from_utf8_lossy(&csv).to_string();
    let differing: Vec<_> = names.split("\r\n").zip(base_names.split("\r\n")).filter(|(a, b)| a != b).collect();
    let renamed_line = format!("{renamed},{}", String::from_utf8_lossy(hues.hue(renamed).unwrap().name_bytes()));
    assert_eq!(differing, vec![(renamed_line.as_str(), format!("{renamed},Old").as_str())]);
    assert!(names.starts_with("0001,"));

    let recovered = hues_mul_from_uop_package(&package_of(&entries)).expect("recover");
    assert!(recovered.had_metadata);
    assert_eq!(recovered.hues.to_bytes(), hues.to_bytes());
}
