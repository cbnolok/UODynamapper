use crate::models::{PackedMapTexel, TileMetaItemInfo, TileMetaLandInfo};
use crate::utils::format_tile_flags;

pub const PACKAGE_MAP_CHUNK_TILE_DIM: usize = 32;

pub fn render_tilemeta_land_preview(id: u32, info: &TileMetaLandInfo) -> String {
    format!(
        "TileMeta Land {}\nname: {}\ntexture_id: {}\ntile_type: {}\nflags: {}\nradar_color: rgba({}, {}, {}, {})",
        id,
        if info.name.is_empty() { "<unnamed>" } else { &info.name },
        info.texture_id,
        info.tile_type,
        format_tile_flags(info.flags),
        info.radar_color[0],
        info.radar_color[1],
        info.radar_color[2],
        info.radar_color[3]
    )
}

pub fn render_tilemeta_item_preview(id: u32, info: &TileMetaItemInfo) -> String {
    format!(
        "TileMeta Item {}\nname: {}\nflags: {}\nweight: {}\nquality: {}\nquantity: {}\nhue_extra: {}\nanim_id: {}\nstacking_offset: {}\nvalue: {}\nheight: {}\nradar_color: rgba({}, {}, {}, {})\nec_texture_id: {}\nec_start: {}, {}\nec_offset: {}, {}\ncc_texture_id: {}\ncc_start: {}, {}\ncc_offset: {}, {}",
        id,
        if info.name.is_empty() { "<unnamed>" } else { &info.name },
        format_tile_flags(info.flags),
        info.weight,
        info.quality,
        info.quantity,
        info.hue_extra,
        info.anim_id,
        info.stacking_offset,
        info.value,
        info.height,
        info.radar_color[0],
        info.radar_color[1],
        info.radar_color[2],
        info.radar_color[3],
        info.ec_texture_id,
        info.ec_start_x,
        info.ec_start_y,
        info.ec_offset_x,
        info.ec_offset_y,
        info.cc_texture_id,
        info.cc_start_x,
        info.cc_start_y,
        info.cc_offset_x,
        info.cc_offset_y
    )
}

pub fn render_map_block_preview(block_id: u32, data: &[u8]) -> String {
    let Some(cells) = crate::utils::read_pod_records::<PackedMapTexel>(data) else {
        return format!(
            "Map chunk {} has invalid size: expected {} bytes, got {}.",
            block_id,
            PACKAGE_MAP_CHUNK_TILE_DIM
                * PACKAGE_MAP_CHUNK_TILE_DIM
                * std::mem::size_of::<PackedMapTexel>(),
            data.len()
        );
    };

    if cells.len() != PACKAGE_MAP_CHUNK_TILE_DIM * PACKAGE_MAP_CHUNK_TILE_DIM {
        return format!(
            "Map chunk {} contains {} cells, expected {}.",
            block_id,
            cells.len(),
            PACKAGE_MAP_CHUNK_TILE_DIM * PACKAGE_MAP_CHUNK_TILE_DIM
        );
    }

    let mut text = format!("Map Chunk {}\n", block_id);
    for (index, cell) in cells.iter().enumerate() {
        let x = index % PACKAGE_MAP_CHUNK_TILE_DIM;
        let y = index / PACKAGE_MAP_CHUNK_TILE_DIM;
        let z = ((cell.packed_meta & 0x00FF) as i16 - 128) as i8;
        let mode = (cell.packed_meta >> 8) as u8;
        text.push_str(&format!(
            "({x},{y}) tile={} z={} mode={}\n",
            cell.tile_id, z, mode
        ));
    }
    text
}

pub fn render_static_block_preview(block_id: u32, data: &[u8]) -> String {
    use uocf::classic::statics::StaticTile;
    let record_size = std::mem::size_of::<StaticTile>(); // Should be 8
    if data.len() % record_size != 0 {
        return format!(
            "Static block {} has invalid size: {} bytes is not a multiple of {}.",
            block_id,
            data.len(),
            record_size
        );
    }

    let count = data.len() / record_size;
    let mut text = format!("Static Chunk {}\nentries: {}\n", block_id, count);
    for i in 0..count {
        let base = i * record_size;
        let graphic = u16::from_le_bytes([data[base], data[base + 1]]);
        let x_offset = data[base + 2];
        let y_offset = data[base + 3];
        let z = data[base + 4] as i8;
        // Padding byte at data[base + 5]
        let hue = u16::from_le_bytes([data[base + 6], data[base + 7]]);
        text.push_str(&format!(
            "#{i}: graphic={graphic} offset=({x_offset}, {y_offset}) z={z} hue={hue}\n",
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_map_block_preview_decodes_height_bias() {
        let cells = vec![
            PackedMapTexel {
                tile_id: 5,
                packed_meta: 128
            };
            PACKAGE_MAP_CHUNK_TILE_DIM * PACKAGE_MAP_CHUNK_TILE_DIM
        ];
        let preview = render_map_block_preview(9, bytemuck::cast_slice(&cells));

        assert!(preview.contains("Map Chunk 9"));
        assert!(preview.contains("(0,0) tile=5 z=0 mode=0"));
    }
}
