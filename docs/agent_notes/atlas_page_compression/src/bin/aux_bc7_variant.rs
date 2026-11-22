//! A copy of `tex_aux_ec.uddp` with some RGBA textures re-encoded BC7, for the rendered check of aux_maps: `reserved`
//! the atmosphere maps and light textures, `array` the textures of the renderer's auxiliary array (all but those, as
//! the BC7 packer stores them), `all` both. The packers' encoder (`AnalyticalWide`, RDO off); everything else is
//! copied as decoded. The Kingdom Reborn section gets the same treatment and is stored uncompressed, as the packer
//! nests it.
//! Usage: aux_bc7_variant <tex_aux_ec.uddp> <output> reserved|array|all.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::tex_aux_ec::{
    decode_tex_aux_ec_slots, encode_tex_aux_ec_slots, is_tex_aux_ec_reserved_key, texture_entry_path,
    TexAuxEcSlotRecord, TEX_AUX_EC_MISSING_TEXTURE_ID, TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH,
};
use udd_assets::common::KR_SECTION_ENTRY_PATH;
use udd_container::{AddOwnedFileRequest, CompressionFlag, FileKey, LookupMode, UddpReader, UddpStreamingBuilder,
    xxh64_virtual_path};
use uot_texture_codecs::bc7::{encode_for_vram, Bc7EncoderBackend, ImageExtent, RawImageFormat, VramTextureEncoding};

const ZSTD_LEVEL: i32 = 19;
const SECTION_SUFFIX: &str = ".kr-section";

/// An atmosphere map or a light texture: a reserved key other than the missing texture's.
fn map_or_light(texture_id: u32) -> bool {
    is_tex_aux_ec_reserved_key(texture_id) && texture_id != TEX_AUX_EC_MISSING_TEXTURE_ID
}

/// Writes `package` to `output` with the selected RGBA textures BC7, and returns its slots.
fn rewrite(package: &UddpReader, output: &Path, reserved: bool, array: bool, base_missing_format: Option<u8>)
    -> Vec<TexAuxEcSlotRecord> {
    let manifest_hash = xxh64_virtual_path(TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH);
    let kr_hash = xxh64_virtual_path(KR_SECTION_ENTRY_PATH);
    let mut slots = decode_tex_aux_ec_slots(&package.read_file_by_path_hash(manifest_hash).unwrap()).unwrap();
    let mut replaced = HashMap::new();
    for slot in slots.iter_mut().filter(|slot| !slot.is_missing_source() && !slot.is_bc7()
        && if map_or_light(slot.texture_id) { reserved } else { array })
    {
        let hash = xxh64_virtual_path(&texture_entry_path(slot.texture_id));
        let rgba = package.read_file_by_path_hash(hash).unwrap();
        let extent = ImageExtent::new(u32::from(slot.width), u32::from(slot.height)).unwrap();
        let vram = encode_for_vram(&rgba, extent, RawImageFormat::Rgba8888,
            VramTextureEncoding::Bc7(Bc7EncoderBackend::AnalyticalWide)).unwrap();
        slot.pixel_format = PagePixelFormat::Bc7 as u8;
        replaced.insert(hash, vram.to_container_bytes());
    }
    // A missing-source alias takes the missing-texture image's format, as the packer writes it; an overlay that
    // leaves that image to its base takes the base's (`base_missing_format`), the format the packer gives both.
    let missing_format = slots.iter().find(|slot| slot.texture_id == TEX_AUX_EC_MISSING_TEXTURE_ID)
        .map_or(base_missing_format, |missing| Some(missing.pixel_format));
    for slot in slots.iter_mut().filter(|slot| slot.is_missing_source()) {
        slot.pixel_format = missing_format.expect("a missing-texture image for the aliases");
    }
    if package.find_by_path_hash(kr_hash).is_some() {
        let section = package.read_file_by_path_hash(kr_hash).unwrap();
        let mut section_path = output.as_os_str().to_owned();
        section_path.push(SECTION_SUFFIX);
        let section_path = PathBuf::from(section_path);
        let section_slots = rewrite(&UddpReader::open(section).unwrap(), &section_path, reserved, array,
            missing_format);
        report("Kingdom Reborn section", &section_slots);
        replaced.insert(kr_hash, std::fs::read(&section_path).unwrap());
        std::fs::remove_file(&section_path).unwrap();
    }
    let mut out = UddpStreamingBuilder::create(LookupMode::VirtualPathHash, output).unwrap();
    out.add_owned_file(AddOwnedFileRequest {
        data_type: package.find_by_path_hash(manifest_hash).unwrap().data_type,
        compression: CompressionFlag::ZstdNoDictLevel(ZSTD_LEVEL), width: 0, height: 0, virtual_path: None,
        path_hash64: Some(manifest_hash), id: None, data: encode_tex_aux_ec_slots(&slots),
    }).unwrap();
    for record in package.records() {
        let FileKey::PathHash(hash) = record.key else { panic!("tex_aux_ec uses path hashes") };
        if hash == manifest_hash { continue; }
        let compression = if hash == kr_hash { CompressionFlag::None } else { CompressionFlag::ZstdNoDictLevel(ZSTD_LEVEL) };
        let data = replaced.remove(&hash).unwrap_or_else(|| package.read_file_by_path_hash(hash).unwrap());
        out.add_owned_file(AddOwnedFileRequest {
            data_type: package.find_by_path_hash(hash).unwrap().data_type, compression, width: 0, height: 0,
            virtual_path: None, path_hash64: Some(hash), id: None, data,
        }).unwrap();
    }
    assert!(replaced.is_empty());
    out.finish().unwrap();
    slots
}

fn report(name: &str, slots: &[TexAuxEcSlotRecord]) {
    println!("{name}: BC7 atmosphere maps and light textures {} of {}, other textures {} of {}",
        slots.iter().filter(|slot| map_or_light(slot.texture_id) && slot.is_bc7()).count(),
        slots.iter().filter(|slot| map_or_light(slot.texture_id)).count(),
        slots.iter().filter(|slot| !map_or_light(slot.texture_id) && slot.is_bc7()).count(),
        slots.iter().filter(|slot| !map_or_light(slot.texture_id)).count());
}

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [input, output, mode] = &args[..] else {
        panic!("usage: aux_bc7_variant <tex_aux_ec.uddp> <output> reserved|array|all")
    };
    let (reserved, array) = match mode.to_str() {
        Some("reserved") => (true, false),
        Some("array") => (false, true),
        Some("all") => (true, true),
        _ => panic!("mode: reserved, array or all"),
    };
    assert!(!output.exists(), "{} exists", output.display());
    let slots = rewrite(&UddpReader::load(input).unwrap(), output, reserved, array, None);
    report(&output.display().to_string(), &slots);
}
