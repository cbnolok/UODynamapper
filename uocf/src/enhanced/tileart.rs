//! Represents tile art data from a UOP file for Enhanced Client "Art Tiles" (statics).

use crate::enhanced::string_dictionary::UoStringDictionary;
use crate::uop::file::UopFile;
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

// region: --- Public API (Convenience & Application Use)

/// Main entry point to load and process Art Tile data from an Enhanced Client UOP file.
/// This function handles the full pipeline: parsing the raw binary structure and
/// resolving string references into a clean, public-facing ArtData structure.
pub fn load(
    uop_file: &UopFile,
    string_dictionary: &UoStringDictionary,
) -> color_eyre::eyre::Result<ArtData> {
    let raw_entry = TileArtEntry::parse_raw(uop_file)?;
    let processed_data = raw_entry.process(string_dictionary);
    Ok(processed_data)
}

/// Inferred tile classification used for high-level logic.
///
/// This is a convenience abstraction. In the raw binary data, this is stored as
/// a simple integer (`type_val` in `TileArtEntry`). We map these values:
/// - 0 => `Static`
/// - 1 => `Solid`
/// - 2 => `Liquid`
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TileType {
    #[default]
    Static,
    Solid,
    Liquid,
}

/// Direct mapping of indices used in the raw `TaeProp` array in the binary file.
///
/// Each enum variant represents a specific property slot (0-11) as defined in the
/// `tileart.uop` format. These values are used byte-per-byte from the source.
#[derive(Debug, Clone, Copy)]
pub enum PropertyKey {
    Weight = 0,
    Quality = 1,
    Quantity = 2,
    Height = 3,
    Value = 4,
    AcVc = 5,
    Slot = 6,
    OffC8 = 7,
    Appearance = 8,
    Race = 9,
    Gender = 10,
    Paperdoll = 11,
}

/// Cleaned-up representation of texture coordinates and offsets for a tile.
///
/// Unlike the internal `TaeImgOffset`, this structure includes the resolved `texture_id`
/// and is what the rest of the application uses to render the tile.
#[derive(Debug, Default, Clone)]
pub struct ArtTexture {
    pub texture_id: u32,
    pub start_x: i32,
    pub start_y: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// The primary public-facing structure for Enhanced Client static art data.
///
/// This is a convenience structure that aggregates and simplifies the raw data found in
/// `TileArtEntry`. It is intended for use by the renderer and other high-level systems.
/// It contains resolved textures for both Enhanced (EC) and Classic (CC) visual modes,
/// as well as unified flags and radar colors.
#[derive(Debug, Default, Clone)]
pub struct ArtData {
    pub id: u16,
    pub tile_type: TileType,
    pub flags: TaeFlag,
    pub height: u8,
    pub ec_texture: Option<ArtTexture>,
    pub cc_texture: Option<ArtTexture>,
    pub radar_color: TaeRadarcol,
    pub texture_items: Vec<Vec<TextureItem>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TextureType {
    #[default]
    Undefined,
    WorldArt,
    TileArtLegacy,
    TileArtEnhanced,
}

#[derive(Debug, Clone, Default)]
pub struct TextureItem {
    pub texture_type: TextureType,
    pub id: u32,
    pub path: String,
}

bitflags! {
    /// Direct bit-for-bit mapping of the 64-bit tile flag field found in the binary file.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct TaeFlag: u64 {
        const None = 0x0;
        const Background = 0x1;
        const Weapon = 0x2;
        const Transparent = 0x4;
        const Translucent = 0x8;
        const Wall = 0x10;
        const Damaging = 0x20;
        const Impassable = 0x40;
        const Wet = 0x80;
        const Ignored = 0x100;
        const Surface = 0x200;
        const Bridge = 0x400;
        const Generic = 0x800;
        const Window = 0x1000;
        const NoShoot = 0x2000;
        const ArticleA = 0x4000;
        const ArticleAn = 0x8000;
        const ArticleThe = Self::ArticleA.bits() | Self::ArticleAn.bits();
        const Mongen = 0x10000;
        const Foliage = 0x20000;
        const PartialHue = 0x40000;
        const UseNewArt = 0x80000;
        const Map = 0x100000;
        const Container = 0x200000;
        const Wearable = 0x400000;
        const LightSource = 0x800000;
        const Animation = 0x1000000;
        const HoverOver = 0x2000000;
        const ArtUsed = 0x4000000;
        const Armor = 0x8000000;
        const Roof = 0x10000000;
        const Door = 0x20000000;
        const StairBack = 0x40000000;
        const StairRight = 0x80000000;
        const NoHouse = 0x100000000;
        const NoDraw = 0x200000000;
        const Unused1 = 0x400000000;
        const AlphaBlend = 0x800000000;
        const NoShadow = 0x1000000000;
        const PixelBleed = 0x2000000000;
        const Unused2 = 0x4000000000;
        const PlayAnimOnce = 0x8000000000;
        const MultiMovable = 0x10000000000;
    }
}

/// Direct mapping of the 4-byte radar color data (typically BGRA) found in the file.
#[derive(Debug, Default, Clone)]
pub struct TaeRadarcol {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

// endregion: --- Public API

// region: --- Internal Raw File-Mapping Structs (Binary Format)

impl Default for TaeFlag {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Default, Clone)]
pub struct TaeProp {
    pub id: u8,
    pub val: u32,
}

#[derive(Debug, Default, Clone)]
pub struct TaeStackAlias {
    pub amount: u32,
    pub amount_id: u32,
}

#[derive(Debug, Default, Clone)]
pub struct TaeAnimationAppearanceSub3 {
    pub unk1: u32,
    pub unk2: u32,
}

#[derive(Debug, Default, Clone)]
pub struct TaeAnimationAppearanceSub2 {
    pub sub_count: u32,
    pub sub3_vector: Vec<TaeAnimationAppearanceSub3>,
}

#[derive(Debug, Default, Clone)]
pub struct TaeAnimationAppearanceSub1 {
    pub unk1: u8,
    pub unk2: u32,
}

#[derive(Debug, Default, Clone)]
pub struct TaeAnimationAppearance {
    pub sub_type: u8,
    pub sub1: Option<TaeAnimationAppearanceSub1>,
    pub sub2: Option<TaeAnimationAppearanceSub2>,
}

#[derive(Debug, Default, Clone)]
pub struct TaeSittingAnimation {
    pub unk1: u32,
    pub unk2: u32,
    pub unk3: u32,
    pub unk4: u32,
}

#[derive(Debug, Default, Clone)]
pub struct TaeTextureImage {
    pub name_string_off: u32,
    pub unk4: u8,
    pub texture_stretch: f32,
    pub unk6: u32,
    pub unk7: u32,
}

#[derive(Debug, Default, Clone)]
pub struct TaeTexture {
    pub has_texture: u8,
    pub unk1: u8,
    pub type_string_off: u32,
    pub texture_items_count: u8,
    pub texture_items: Vec<TaeTextureImage>,
    pub unk8_count: u32,
    pub unk8_vector: Vec<i32>,
    pub unk9_count: u32,
    pub unk9_vector: Vec<f32>,
}

#[derive(Debug, Default, Clone)]
pub struct TaeImgOffset {
    pub x_start: i32,
    pub y_start: i32,
    pub x_end: i32,
    pub y_end: i32,
    pub x_off: i32,
    pub y_off: i32,
}

/// Raw, bit-for-bit mapping of an Art Tile entry as stored in the UOP binary.
///
/// This structure contains many "unknown" fields and raw offsets that are specific to the
/// file format. It is used internally for parsing and should not be exposed to the
/// main application. Use `ArtData` instead for a cleaned-up version.
#[derive(Debug, Default, Clone)]
pub struct TileArtEntry {
    pub uop_block: i32,
    pub uop_file: i32,
    pub version: u16,
    pub string_dict_off: u32,
    pub tile_id: u32,
    pub unk_bool1: bool,
    pub unk_bool2: bool,
    pub unk_float3: f32,
    pub unk_float4: f32,
    pub fixed_zero: i32,
    pub old_id: i32,
    pub unk_float5: f32,
    pub type_val: i32,
    pub unk_byte6: u8,
    pub unk_dw7: i32,
    pub unk_dw8: i32,
    pub light1: f32,
    pub light2: f32,
    pub unk_dw9: u32,
    pub flags1: TaeFlag,
    pub flags2: TaeFlag,
    pub facing: i32,
    pub ec_img_offset: TaeImgOffset,
    pub cc_img_offset: TaeImgOffset,
    pub prop_count1: u8,
    pub prop_vector1: Vec<TaeProp>,
    pub prop_count2: u8,
    pub prop_vector2: Vec<TaeProp>,
    pub stack_alias_count_offset: u32,
    pub stack_alias_count: u32,
    pub stack_alias_vector: Vec<TaeStackAlias>,
    pub appearance_count: u32,
    pub appearance_vector: Vec<TaeAnimationAppearance>,
    pub has_sitting: bool,
    pub sitting: Option<TaeSittingAnimation>,
    pub radarcol_offset: u32,
    pub radarcol: TaeRadarcol,
    pub texture_vector: Vec<TaeTexture>,
}

// endregion: --- Internal Raw File-Mapping Structs

impl TileArtEntry {
    pub fn parse_raw(uop_file: &UopFile) -> color_eyre::eyre::Result<Self> {
        let decompressed_data = uop_file.unpack()?;
        let mut reader = Cursor::new(&*decompressed_data);
        let mut entry = TileArtEntry::default();

        macro_rules! read_value {
            ($field:expr, $reader_method:ident, $type:ty) => {
                $field = reader.$reader_method::<$type>()?;
            };
        }

        read_value!(entry.version, read_u16, LittleEndian);
        read_value!(entry.string_dict_off, read_u32, LittleEndian);
        read_value!(entry.tile_id, read_u32, LittleEndian);
        entry.unk_bool1 = reader.read_u8()? != 0;
        entry.unk_bool2 = reader.read_u8()? != 0;
        read_value!(entry.unk_float3, read_f32, LittleEndian);
        read_value!(entry.unk_float4, read_f32, LittleEndian);
        read_value!(entry.fixed_zero, read_i32, LittleEndian);
        read_value!(entry.old_id, read_i32, LittleEndian);
        read_value!(entry.unk_float5, read_f32, LittleEndian);
        read_value!(entry.type_val, read_i32, LittleEndian);
        entry.unk_byte6 = reader.read_u8()?;
        read_value!(entry.unk_dw7, read_i32, LittleEndian);
        read_value!(entry.unk_dw8, read_i32, LittleEndian);
        read_value!(entry.light1, read_f32, LittleEndian);
        read_value!(entry.light2, read_f32, LittleEndian);
        read_value!(entry.unk_dw9, read_u32, LittleEndian);
        entry.flags1 = TaeFlag::from_bits_truncate(reader.read_u64::<LittleEndian>()?);
        entry.flags2 = TaeFlag::from_bits_truncate(reader.read_u64::<LittleEndian>()?);
        read_value!(entry.facing, read_i32, LittleEndian);

        read_value!(entry.ec_img_offset.x_start, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.y_start, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.x_end, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.y_end, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.x_off, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.y_off, read_i32, LittleEndian);

        read_value!(entry.cc_img_offset.x_start, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.y_start, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.x_end, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.y_end, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.x_off, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.y_off, read_i32, LittleEndian);

        entry.prop_count1 = reader.read_u8()?;
        for _ in 0..entry.prop_count1 {
            let mut prop = TaeProp::default();
            prop.id = reader.read_u8()?;
            prop.val = reader.read_u32::<LittleEndian>()?;
            entry.prop_vector1.push(prop);
        }

        entry.prop_count2 = reader.read_u8()?;
        for _ in 0..entry.prop_count2 {
            let mut prop = TaeProp::default();
            prop.id = reader.read_u8()?;
            prop.val = reader.read_u32::<LittleEndian>()?;
            entry.prop_vector2.push(prop);
        }

        entry.stack_alias_count_offset = reader.position() as u32;
        read_value!(entry.stack_alias_count, read_u32, LittleEndian);
        for _ in 0..entry.stack_alias_count {
            let mut alias = TaeStackAlias::default();
            alias.amount = reader.read_u32::<LittleEndian>()?;
            alias.amount_id = reader.read_u32::<LittleEndian>()?;
            entry.stack_alias_vector.push(alias);
        }

        read_value!(entry.appearance_count, read_u32, LittleEndian);
        for _ in 0..entry.appearance_count {
            let mut appearance = TaeAnimationAppearance::default();
            appearance.sub_type = reader.read_u8()?;
            if appearance.sub_type == 1 {
                let mut sub1 = TaeAnimationAppearanceSub1::default();
                sub1.unk1 = reader.read_u8()?;
                sub1.unk2 = reader.read_u32::<LittleEndian>()?;
                appearance.sub1 = Some(sub1);
            } else {
                let mut sub2 = TaeAnimationAppearanceSub2::default();
                sub2.sub_count = reader.read_u32::<LittleEndian>()?;
                for _ in 0..sub2.sub_count {
                    let mut sub3 = TaeAnimationAppearanceSub3::default();
                    sub3.unk1 = reader.read_u32::<LittleEndian>()?;
                    sub3.unk2 = reader.read_u32::<LittleEndian>()?;
                    sub2.sub3_vector.push(sub3);
                }
                appearance.sub2 = Some(sub2);
            }
            entry.appearance_vector.push(appearance);
        }

        entry.has_sitting = reader.read_u8()? != 0;
        if entry.has_sitting {
            let mut sitting = TaeSittingAnimation::default();
            sitting.unk1 = reader.read_u32::<LittleEndian>()?;
            sitting.unk2 = reader.read_u32::<LittleEndian>()?;
            sitting.unk3 = reader.read_u32::<LittleEndian>()?;
            sitting.unk4 = reader.read_u32::<LittleEndian>()?;
            entry.sitting = Some(sitting);
        }

        entry.radarcol_offset = reader.position() as u32;
        entry.radarcol.r = reader.read_u8()?;
        entry.radarcol.g = reader.read_u8()?;
        entry.radarcol.b = reader.read_u8()?;
        entry.radarcol.a = reader.read_u8()?;

        for _ in 0..4 {
            let mut texture = TaeTexture::default();
            texture.has_texture = reader.read_u8()?;
            if texture.has_texture == 1 {
                texture.unk1 = reader.read_u8()?;
                texture.type_string_off = reader.read_u32::<LittleEndian>()?;
                texture.texture_items_count = reader.read_u8()?;
                for _ in 0..texture.texture_items_count {
                    let mut item = TaeTextureImage::default();
                    item.name_string_off = reader.read_u32::<LittleEndian>()?;
                    item.unk4 = reader.read_u8()?;
                    item.texture_stretch = reader.read_f32::<LittleEndian>()?;
                    item.unk6 = reader.read_u32::<LittleEndian>()?;
                    item.unk7 = reader.read_u32::<LittleEndian>()?;
                    texture.texture_items.push(item);
                }

                texture.unk8_count = reader.read_u32::<LittleEndian>()?;
                for _ in 0..texture.unk8_count {
                    texture.unk8_vector.push(reader.read_i32::<LittleEndian>()?);
                }

                texture.unk9_count = reader.read_u32::<LittleEndian>()?;
                for _ in 0..texture.unk9_count {
                    texture.unk9_vector.push(reader.read_f32::<LittleEndian>()?);
                }
            }
            entry.texture_vector.push(texture);
        }

        Ok(entry)
    }

    pub fn process(&self, string_dictionary: &UoStringDictionary) -> ArtData {
        let mut art_data = ArtData::default();

        art_data.id = self.tile_id as u16;
        art_data.flags = self.flags1;
        art_data.radar_color = self.radarcol.clone();
        art_data.height = self.get_property(PropertyKey::Height).unwrap_or(0) as u8;
        art_data.texture_items = self.get_texture_item_vector(string_dictionary);

        if let Some(ec_texture_block) = self.texture_vector.get(0) {
            if ec_texture_block.has_texture == 1 {
                art_data.tile_type = self.get_tile_type(ec_texture_block, string_dictionary);
                if let Some(texture_item) = ec_texture_block.texture_items.get(0) {
                    if let Some(id) = Self::get_texture_id_from_string_offset(
                        texture_item.name_string_off,
                        string_dictionary,
                    ) {
                        art_data.ec_texture = Some(ArtTexture {
                            texture_id: id,
                            start_x: self.ec_img_offset.x_start,
                            start_y: self.ec_img_offset.y_start,
                            end_x: self.ec_img_offset.x_end,
                            end_y: self.ec_img_offset.y_end,
                            offset_x: self.ec_img_offset.x_off,
                            offset_y: self.ec_img_offset.y_off,
                        });
                    }
                }
            }
        }

        if let Some(cc_texture_block) = self.texture_vector.get(1) {
            if cc_texture_block.has_texture == 1 {
                if let Some(texture_item) = cc_texture_block.texture_items.get(0) {
                    if let Some(id) = Self::get_texture_id_from_string_offset(
                        texture_item.name_string_off,
                        string_dictionary,
                    ) {
                        art_data.cc_texture = Some(ArtTexture {
                            texture_id: id,
                            start_x: self.cc_img_offset.x_start,
                            start_y: self.cc_img_offset.y_start,
                            end_x: self.cc_img_offset.x_end,
                            end_y: self.cc_img_offset.y_end,
                            offset_x: self.cc_img_offset.x_off,
                            offset_y: self.cc_img_offset.y_off,
                        });
                    }
                }
            }
        }

        art_data
    }

    fn get_texture_item_vector(
        &self,
        string_dictionary: &UoStringDictionary,
    ) -> Vec<Vec<TextureItem>> {
        let mut texture_ids = Vec::new();
        for texture in &self.texture_vector {
            if texture.has_texture == 1 {
                let mut items = Vec::new();
                for texture_item in &texture.texture_items {
                    if let Some(str) =
                        string_dictionary.get_string((texture_item.name_string_off - 1) as usize)
                    {
                        let mut item = TextureItem::default();
                        item.texture_type = classify_texture_path(&str);
                        if let Some(id) = extract_texture_id_from_path(&str) {
                            item.id = id;
                        }
                        item.path = str.to_string();

                        items.push(item);
                    }
                }
                texture_ids.push(items);
            }
        }
        texture_ids
    }

    fn get_tile_type(
        &self,
        texture_block: &TaeTexture,
        string_dictionary: &UoStringDictionary,
    ) -> TileType {
        if let Some(shader_name) =
            string_dictionary.get_string((texture_block.type_string_off - 1) as usize)
        {
            match shader_name {
                "UOWaterShader" => TileType::Liquid,
                "UOStaticTerrainShader" => TileType::Solid,
                "UOSpriteShader" => {
                    if let Some(item) = texture_block.texture_items.get(0) {
                        if item.texture_stretch != 1.0 {
                            return TileType::Solid;
                        }
                    }
                    TileType::Static
                }
                _ => TileType::Static,
            }
        } else {
            TileType::Static
        }
    }

    fn get_property(&self, key: PropertyKey) -> Option<u32> {
        let key_id = key as u8;
        for prop in &self.prop_vector1 {
            if prop.id == key_id {
                return Some(prop.val);
            }
        }
        for prop in &self.prop_vector2 {
            if prop.id == key_id {
                return Some(prop.val);
            }
        }
        None
    }

    fn get_texture_id_from_string_offset(
        offset: u32,
        string_dictionary: &UoStringDictionary,
    ) -> Option<u32> {
        string_dictionary
            .get_string((offset - 1) as usize)
            .and_then(extract_texture_id_from_path)
    }
}

fn classify_texture_path(path: &str) -> TextureType {
    let normalized = normalize_dictionary_path(path);
    if normalized.contains("data\\worldart\\") {
        TextureType::WorldArt
    } else if normalized.contains("data\\tileartlegacy\\") {
        TextureType::TileArtLegacy
    } else if normalized.contains("data\\tileartenhanced\\") {
        TextureType::TileArtEnhanced
    } else {
        TextureType::Undefined
    }
}

fn extract_texture_id_from_path(path: &str) -> Option<u32> {
    let file_name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file_name.split('.').next().unwrap_or(file_name);

    stem.split('_')
        .next()
        .and_then(extract_first_digit_run)
        .or_else(|| extract_first_digit_run(stem))
}

fn normalize_dictionary_path(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

fn extract_first_digit_run(segment: &str) -> Option<u32> {
    let start = segment.find(|c: char| c.is_ascii_digit())?;
    let digits = segment[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then_some(digits)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tileart_extract_texture_id_normalizes_dictionary_name_patterns() {
        assert_eq!(
            extract_texture_id_from_path("Data\\WorldArt\\00000002_ankh.tga"),
            Some(2)
        );
        assert_eq!(
            extract_texture_id_from_path("Data\\TileArtLegacy\\3.tga"),
            Some(3)
        );
        assert_eq!(
            extract_texture_id_from_path("Data/TileArtEnhanced/02000540_Sand_Cliff_EW_A.tga"),
            Some(2000540)
        );
    }

    #[test]
    fn tileart_classify_texture_path_normalizes_case_and_slashes() {
        assert_eq!(
            classify_texture_path("data/worldart/00000002_ankh.tga"),
            TextureType::WorldArt
        );
        assert_eq!(
            classify_texture_path("DATA\\TILEARTLEGACY\\3.tga"),
            TextureType::TileArtLegacy
        );
        assert_eq!(
            classify_texture_path("Data/TileArtEnhanced/00000004_tree.tga"),
            TextureType::TileArtEnhanced
        );
    }
}
