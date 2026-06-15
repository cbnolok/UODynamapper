use std::path::{Path, PathBuf};
use bytemuck::{pod_read_unaligned, Pod};
use udd_container::Codec;
use crate::models::AtlasPixelFormat;

pub fn open_package_dialog(initial_dir: Option<&Path>) -> Option<PathBuf> {
    udd_tool_gui::open_filtered_file_dialog(initial_dir, "UDDP Packages", &["uddp", "uddpi"])
}

pub fn save_file_dialog(default_name: &str) -> Option<PathBuf> {
    udd_tool_gui::save_file_dialog(default_name)
}

pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f32 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f32 / (1024.0 * 1024.0))
    }
}

pub fn data_type_to_str(t: u8) -> &'static str {
    // TODO: do not use magic numbers here. use const or enums
    match t {
        1 => "Art",
        3 => "Map",
        9 => "Texture",
        11 => "Metadata",
        14 => "Static",
        _ => "Other",
    }
}

pub fn compression_to_str(codec: Codec) -> &'static str {
    match codec {
        Codec::None => "none",
        Codec::ZstdNoDict => "zstd",
        Codec::ZstdTypeDict => "zstd type dict",
        Codec::JpegXl => "jpegxl",
    }
}

pub fn atlas_pixel_format_to_str(format: AtlasPixelFormat) -> &'static str {
    match format {
        AtlasPixelFormat::Rgba8888 => "rgba8888",
        AtlasPixelFormat::Bc7 => "bc7",
    }
}

pub fn read_pod_records<T: Pod>(data: &[u8]) -> Option<Vec<T>> {
    let record_size = std::mem::size_of::<T>();
    if record_size == 0 || data.len() % record_size != 0 {
        return None;
    }

    Some(
        data.chunks_exact(record_size)
            .map(pod_read_unaligned::<T>)
            .collect(),
    )
}

pub fn crop_rgba(rgba: &[u8], page_width: usize, rect: [usize; 4]) -> Vec<u8> {
    let [rx, ry, rw, rh] = rect;
    let mut cropped = Vec::with_capacity(rw * rh * 4);
    for y in 0..rh {
        let start = ((ry + y) * page_width + rx) * 4;
        let end = start + rw * 4;
        if end <= rgba.len() {
            cropped.extend_from_slice(&rgba[start..end]);
        }
    }
    cropped
}

pub fn atlas_page_paths(page_index: u32) -> Vec<String> {
    let mut paths = Vec::with_capacity(6);
    for ext in ["bc7", "rgba8888", "bin"] {
        paths.push(format!("pages/{page_index:05}.{ext}"));
    }
    for ext in ["bc7", "rgba8888", "bin"] {
        paths.push(format!("pages/{page_index}.{ext}"));
    }
    paths
}

pub fn format_tile_flags(flags: u64) -> String {
    let mut names = Vec::new();
    let f = flags as u32;
    if f & 0x01 != 0 { names.push("Background"); }
    if f & 0x02 != 0 { names.push("Weapon"); }
    if f & 0x04 != 0 { names.push("Transparent"); }
    if f & 0x08 != 0 { names.push("Translucent"); }
    if f & 0x10 != 0 { names.push("Wall"); }
    if f & 0x20 != 0 { names.push("Damaging"); }
    if f & 0x40 != 0 { names.push("Impassable"); }
    if f & 0x80 != 0 { names.push("Wet"); }
    if f & 0x200 != 0 { names.push("Surface"); }
    if f & 0x400 != 0 { names.push("Bridge"); }
    if f & 0x800 != 0 { names.push("Stackable"); }
    if f & 0x1000 != 0 { names.push("Window"); }
    if f & 0x2000 != 0 { names.push("NoShoot"); }
    if f & 0x4000 != 0 { names.push("PrefixA"); }
    if f & 0x8000 != 0 { names.push("PrefixAn"); }
    if f & 0x10000 != 0 { names.push("Internal"); }
    if f & 0x20000 != 0 { names.push("Foliage"); }
    if f & 0x40000 != 0 { names.push("PartialHue"); }
    if f & 0x100000 != 0 { names.push("Map"); }
    if f & 0x200000 != 0 { names.push("Container"); }
    if f & 0x400000 != 0 { names.push("Wearable"); }
    if f & 0x800000 != 0 { names.push("LightSource"); }
    if f & 0x1000000 != 0 { names.push("Animated"); }
    if f & 0x2000000 != 0 { names.push("NoDiagonal"); }
    if f & 0x8000000 != 0 { names.push("Armor"); }
    if f & 0x10000000 != 0 { names.push("Roof"); }
    if f & 0x20000000 != 0 { names.push("Door"); }
    if f & 0x40000000 != 0 { names.push("StairBack"); }
    if f & 0x80000000 != 0 { names.push("StairRight"); }

    format!("0x{:08X} ({})", flags, names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_page_paths_try_zero_padded_names_first() {
        let paths = atlas_page_paths(7);

        assert_eq!(paths[0], "pages/00007.bc7");
        assert_eq!(paths[1], "pages/00007.rgba8888");
        assert_eq!(paths[2], "pages/00007.bin");
        assert_eq!(paths[3], "pages/7.bc7");
    }

    #[test]
    fn read_pod_records_reads_packed_map_cells() {
        use crate::models::PackedMapTexel;
        let bytes = bytemuck::bytes_of(&PackedMapTexel {
            tile_id: 123,
            packed_meta: 0x8001,
        });
        let records = read_pod_records::<PackedMapTexel>(bytes).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tile_id, 123);
        assert_eq!(records[0].packed_meta, 0x8001);
    }
}
