//! An image converted to the format of the entry it replaces ([`UopEntryEditor::replace_image`]), then stored
//! through the entry editor's checks: a `gumpartLegacyMUL.uop` entry becomes a Classic gump encoded as the shipped
//! ones ([`crate::classic::gump_write`]), an `artLegacyMUL.uop` entry a Classic land tile or static, by the art id
//! its name hashes from ([`crate::classic::art_write`]), a TGA entry a TGA written as the entry is
//! ([`crate::enhanced::tga_writer`]: its type, depth, RLE packing and ending), a DDS entry a DDS written as the entry
//! is ([`crate::enhanced::dds_writer`]: its pixel format, header and mip rule).
//!
//! The entry's format comes from its package, its hash and its current payload ([`entry_image_format`]); an image
//! of another size than the entry is refused unless the caller allows it (gump and button layouts assume the
//! size). A land tile is always 44x44. The 0x0 placeholders (gumps, and the TGA headers of the EC Interface gumps
//! and of KR `TileArtLegacy`) take any size.

use image::RgbaImage;

use crate::audio::validate::FormatPolicy;
use crate::classic::art::LAND_DIMENSION;
use crate::classic::art_write::{ArtColors, ArtKind, art_id_of_uop_hash};
use crate::classic::gump_write::{GumpColors, RgbaConversion};
use crate::enhanced::dds_writer::{encode_dds, DdsDescription, DdsHeaderConvention, SizeChange, SHIPPED_MIP_FILTER};
use crate::enhanced::entry_edit::{package_key, EntryEdit, EntryTarget, UopEntryEditor};
use crate::enhanced::image_census::{classify_content, ContentFormat};
use crate::enhanced::tga_writer::{self, TgaConvention};

crate::eyre_imports!();

/// The package whose entries are Classic gumps.
pub const CLASSIC_GUMP_PACKAGE: &str = "gumpartlegacymul";
/// The package whose entries are Classic art (land tiles and statics).
pub const CLASSIC_ART_PACKAGE: &str = "artlegacymul";
const LAND_SIDE: u16 = LAND_DIMENSION as u16;
/// A Classic gump entry with no pixels: a 0x0 header and no rows (34 shipped, the empty `gumpart.mul` slots).
pub const PLACEHOLDER_GUMP: [u8; 8] = [0; 8];

/// What an image entry is stored as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryImageFormat {
    /// A Classic gump of this size; `None` for a placeholder, which takes any size.
    ClassicGump(Option<(u16, u16)>),
    /// Classic art: a land tile (44x44) or a static of this size; `None` for a static entry that does not decode,
    /// which takes any size.
    ClassicArt { art_id: u32, kind: ArtKind, size: Option<(u16, u16)> },
    /// A TGA written as the current entry is.
    Tga(TgaConvention),
    /// A DDS of this size written as the current entry is.
    Dds { convention: DdsHeaderConvention, width: u32, height: u32 },
}

impl EntryImageFormat {
    pub fn label(&self) -> String {
        match self {
            Self::ClassicGump(_) => "Classic gump".to_owned(),
            Self::ClassicArt { art_id, kind: ArtKind::Land, .. } => format!("Classic land tile {art_id:#06x}"),
            Self::ClassicArt { art_id, kind: ArtKind::Static, .. } => format!("Classic static {art_id:#06x}"),
            Self::Tga(convention) => convention.label(),
            Self::Dds { convention, .. } => format!("DDS {}", convention.format().label()),
        }
    }

    /// The entry's size, `None` for a placeholder, which takes any size.
    pub fn size(&self) -> Option<(u32, u32)> {
        match self {
            Self::ClassicGump(size) | Self::ClassicArt { size, .. } => size.map(|(width, height)| (u32::from(width), u32::from(height))),
            Self::Tga(convention) if convention.width() == 0 || convention.height() == 0 => None,
            Self::Tga(convention) => Some((u32::from(convention.width()), u32::from(convention.height()))),
            Self::Dds { width, height, .. } => Some((*width, *height)),
        }
    }
}

/// An image encoded for an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryImage {
    pub format: EntryImageFormat,
    pub bytes: Vec<u8>,
    /// What the conversion changed (rounded colors, a new size).
    pub notes: Vec<String>,
}

/// The image format of the entry `current`, hashed `hash`, of the package `package_key` ([`package_key`]), when an
/// image can be converted to it.
pub fn entry_image_format(package_key: &str, hash: u64, current: &[u8]) -> Option<EntryImageFormat> {
    if package_key == CLASSIC_ART_PACKAGE {
        let art_id = art_id_of_uop_hash(hash)?;
        let kind = ArtKind::of_id(art_id);
        let size = match (kind, ArtColors::decode(kind, current)) {
            (ArtKind::Land, _) => Some((LAND_SIDE, LAND_SIDE)),
            (ArtKind::Static, Ok(ArtColors::Static { width, height, .. })) => Some((width, height)),
            (ArtKind::Static, _) => None,
        };
        return Some(EntryImageFormat::ClassicArt { art_id, kind, size });
    }
    if package_key == CLASSIC_GUMP_PACKAGE {
        if current == PLACEHOLDER_GUMP {
            return Some(EntryImageFormat::ClassicGump(None));
        }
        return GumpColors::decode_uop(current).ok().map(|gump| EntryImageFormat::ClassicGump(Some((gump.width, gump.height))));
    }
    if let Ok(description) = DdsDescription::parse(current) {
        let convention = DdsHeaderConvention::of_header(&description).ok()?;
        let (width, height) = (description.width(), description.height());
        return convention.format().is_supported().then_some(EntryImageFormat::Dds { convention, width, height });
    }
    let convention = TgaConvention::of(current).ok()?;
    let is_tga = matches!(classify_content(current).format, ContentFormat::Tga | ContentFormat::TgaTrailing);
    // The 0x0 placeholders, a header alone, classify as binary.
    let placeholder = current.len() == tga_writer::HEADER_BYTES && (convention.width() == 0 || convention.height() == 0);
    (is_tga || placeholder).then_some(EntryImageFormat::Tga(convention))
}

/// Encodes `image` as `format`.
pub fn encode_entry_image(format: EntryImageFormat, image: &RgbaImage, size_change: SizeChange) -> eyre::Result<EntryImage> {
    let mut notes = Vec::new();
    // A land tile has one size: the encoder refuses any other.
    let fixed_size = matches!(format, EntryImageFormat::ClassicArt { kind: ArtKind::Land, .. });
    if let Some((width, height)) = format.size().filter(|&size| !fixed_size && size != image.dimensions()) {
        let change = format!("the image is {}x{}, the entry {width}x{height}", image.width(), image.height());
        match size_change {
            SizeChange::Refuse => eyre::bail!("{change}; allow a size change to store it (layouts may assume the size)"),
            SizeChange::Allow => notes.push(format!("{change}: layouts may assume the old size")),
        }
    }
    let bytes = match &format {
        EntryImageFormat::Tga(convention) => {
            if !convention.reproduces() {
                notes.push(format!("the entry's RLE packing is of no known style: {}", convention.label()));
            }
            convention.encode_rgba(image)?
        }
        EntryImageFormat::Dds { convention, .. } => encode_dds(convention, image, SHIPPED_MIP_FILTER)?,
        EntryImageFormat::ClassicGump(_) => {
            let (gump, conversion) = GumpColors::from_rgba(image.width(), image.height(), image.as_raw())?;
            push_conversion_notes(&mut notes, conversion);
            gump.encode_uop()?
        }
        EntryImageFormat::ClassicArt { kind, .. } => {
            let (art, conversion) = ArtColors::from_rgba(*kind, image.width(), image.height(), image.as_raw())?;
            push_conversion_notes(&mut notes, conversion);
            art.encode()?
        }
    };
    Ok(EntryImage { format, bytes, notes })
}

fn push_conversion_notes(notes: &mut Vec<String>, conversion: RgbaConversion) {
    if conversion.rounded > 0 {
        notes.push(format!("{} pixels rounded to 5 bits per channel", conversion.rounded));
    }
    if conversion.black > 0 {
        notes.push(format!("{} opaque black pixels stored as 0x0001 (0 is transparent)", conversion.black));
    }
}

impl UopEntryEditor {
    /// The image format of the existing entry `target`, when an image can be converted to it.
    pub fn image_format(&self, target: &EntryTarget) -> eyre::Result<Option<EntryImageFormat>> {
        let current = self.package().unpack_file_by_hash(target.hash())?.ok_or_else(|| eyre!("no entry {}", target.label()))?;
        Ok(entry_image_format(&package_key(self.source()), target.hash(), &current))
    }

    /// Replaces the image entry `target` with `image` converted to its format, then checked as [`Self::replace`]
    /// checks a file.
    pub fn replace_image(&mut self, target: &EntryTarget, image: &RgbaImage, size_change: SizeChange) -> eyre::Result<(EntryImage, EntryEdit)> {
        let format = self.image_format(target)?.ok_or_else(|| {
            eyre!("{}: the entry {} is no image format uocf converts to (Classic gumps and art, TGA, DDS)", self.source().display(), target.label())
        })?;
        let label = format.label();
        let converted = encode_entry_image(format, image, size_change).wrap_err_with(|| format!("{}: {label}", target.label()))?;
        let edit = self.replace(target, &converted.bytes, FormatPolicy::VerifiedOnly)?;
        Ok((converted, edit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::dds_writer::{decode_dds_levels, DdsHeaderStyle, DdsTextureFormat};
    use crate::enhanced::entry_edit::EntryCheck;
    use crate::uop_container::file::CompressionFlag;
    use crate::uop_container::package::{NewEntryCompression, UopPackage};

    #[test]
    fn replaces_a_gump_with_an_image() {
        const NAME: &str = "build/gumpartlegacymul/00000001.tga";
        const EMPTY: &str = "build/gumpartlegacymul/00000029.tga";
        let dir = std::env::temp_dir().join(format!("uocf_entry_image_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let current = GumpColors { width: 2, height: 1, colors: vec![0x7C00, 0] }.encode_uop().unwrap();
        let mut package = UopPackage::new_default();
        package.add_entry_by_name(NAME, &current, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
        package.add_entry_by_name(EMPTY, &PLACEHOLDER_GUMP, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
        let path = dir.join("gumpartLegacyMUL.uop");
        package.finalize_and_save(&path).unwrap();

        let mut editor = UopEntryEditor::load(&path).unwrap();
        let target = EntryTarget::Name(NAME.to_owned());
        assert_eq!(editor.image_format(&target).unwrap(), Some(EntryImageFormat::ClassicGump(Some((2, 1)))));
        let image = RgbaImage::from_raw(2, 1, vec![0, 0, 0, 255, 9, 9, 9, 255]).unwrap();
        let (converted, edit) = editor.replace_image(&target, &image, SizeChange::Refuse).unwrap();
        assert_eq!(GumpColors::decode_uop(&converted.bytes).unwrap().colors, [0x0001, 0x0421]);
        assert_eq!(converted.notes.len(), 2, "{:?}", converted.notes);
        assert_eq!(edit.check, EntryCheck::Parsed { reader: "Classic gump", dialect: None });
        let larger = RgbaImage::new(3, 1);
        assert!(editor.replace_image(&target, &larger, SizeChange::Refuse).is_err());
        assert_eq!(editor.replace_image(&target, &larger, SizeChange::Allow).unwrap().0.notes.len(), 1);
        let (_, edit) = editor.replace_image(&EntryTarget::Name(EMPTY.to_owned()), &larger, SizeChange::Refuse).unwrap();
        assert_eq!(edit.check, EntryCheck::Kind(crate::enhanced::image_census::ContentFormat::Binary), "a placeholder has no gump to parse");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replaces_classic_art_with_an_image_by_its_id() {
        use crate::classic::art::LAND_DIAMOND_PIXEL_COUNT;
        use crate::classic::art_write::{STATIC_ART_ID_BASE, uop_entry_name};
        const LAND_ID: u32 = 3;
        const STATIC_ID: u32 = STATIC_ART_ID_BASE + 1;
        let dir = std::env::temp_dir().join(format!("uocf_entry_art_image_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let land = ArtColors::Land(vec![0x7C00; LAND_DIAMOND_PIXEL_COUNT]);
        let item = ArtColors::Static { width: 2, height: 1, colors: vec![0x7C00, 0] };
        let mut package = UopPackage::new_default();
        for (id, art) in [(LAND_ID, &land), (STATIC_ID, &item)] {
            package.add_entry_by_name(&uop_entry_name(id), &art.encode().unwrap(), NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
        }
        let path = dir.join("artLegacyMUL.uop");
        package.finalize_and_save(&path).unwrap();

        let mut editor = UopEntryEditor::load(&path).unwrap();
        let land_target = EntryTarget::Hash(EntryTarget::Name(uop_entry_name(LAND_ID)).hash());
        let side = LAND_DIMENSION as u16;
        assert_eq!(editor.image_format(&land_target).unwrap(), Some(EntryImageFormat::ClassicArt { art_id: LAND_ID, kind: ArtKind::Land, size: Some((side, side)) }));
        let (_, _, mut rgba) = ArtColors::Land(vec![0x001F; LAND_DIAMOND_PIXEL_COUNT]).to_rgba();
        let image = RgbaImage::from_raw(side.into(), side.into(), rgba.clone()).unwrap();
        let (converted, edit) = editor.replace_image(&land_target, &image, SizeChange::Refuse).unwrap();
        assert_eq!(ArtColors::decode(ArtKind::Land, &converted.bytes).unwrap(), ArtColors::Land(vec![0x001F; LAND_DIAMOND_PIXEL_COUNT]));
        assert_eq!(edit.check, EntryCheck::Parsed { reader: "Classic art", dialect: None });
        rgba[3] = u8::MAX;
        let corner = RgbaImage::from_raw(side.into(), side.into(), rgba).unwrap();
        assert!(editor.replace_image(&land_target, &corner, SizeChange::Allow).is_err(), "a land corner pixel is not transparent");
        assert!(editor.replace_image(&land_target, &RgbaImage::new(2, 1), SizeChange::Allow).is_err(), "a land tile is 44x44");

        let static_target = EntryTarget::Name(uop_entry_name(STATIC_ID));
        let image = RgbaImage::from_raw(2, 1, vec![0, 0, 0, 255, 0, 0, 0, 0]).unwrap();
        let (converted, _) = editor.replace_image(&static_target, &image, SizeChange::Refuse).unwrap();
        assert_eq!(converted.format.label(), "Classic static 0x4001");
        assert_eq!(ArtColors::decode(ArtKind::Static, &converted.bytes).unwrap(), ArtColors::Static { width: 2, height: 1, colors: vec![0x0001, 0] });
        assert_eq!(converted.notes.len(), 1, "{:?}", converted.notes);
        let larger = RgbaImage::new(3, 1);
        assert!(editor.replace_image(&static_target, &larger, SizeChange::Refuse).is_err());
        assert!(editor.replace_image(&static_target, &larger, SizeChange::Allow).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replaces_a_tga_with_an_image_in_its_convention() {
        const NAME: &str = "data/interface/default/textures/gumpart/00000001.tga";
        const EMPTY: &str = "data/interface/default/textures/gumpart/00000029.tga";
        const TGA_HEADER_BYTES: usize = 18;
        let tga_header = |image_type: u8, width: u8| {
            let mut header = vec![0; TGA_HEADER_BYTES];
            (header[2], header[12], header[14], header[16], header[17]) = (image_type, width, width.min(1), 32, 8);
            header
        };
        // Type 10, two pixels as single-pixel raw packets; the placeholder a 0x0 type 10 header.
        let current = [tga_header(10, 2), vec![0, 1, 2, 3, 255, 0, 4, 5, 6, 255]].concat();
        let dir = std::env::temp_dir().join(format!("uocf_entry_image_tga_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut package = UopPackage::new_default();
        package.add_entry_by_name(NAME, &current, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
        package.add_entry_by_name(EMPTY, &tga_header(10, 0), NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
        let path = dir.join("Interface.uop");
        package.finalize_and_save(&path).unwrap();

        let mut editor = UopEntryEditor::load(&path).unwrap();
        let target = EntryTarget::Name(NAME.to_owned());
        let image = RgbaImage::from_raw(2, 1, vec![9, 8, 7, 255, 9, 8, 7, 255]).unwrap();
        let (converted, edit) = editor.replace_image(&target, &image, SizeChange::Refuse).unwrap();
        assert_eq!(converted.bytes, [tga_header(10, 2), vec![0x81, 7, 8, 9, 255]].concat(), "one run");
        assert!(matches!(edit.check, EntryCheck::Parsed { .. }), "{:?}", edit.check);
        assert!(editor.replace_image(&target, &RgbaImage::new(3, 1), SizeChange::Refuse).is_err());
        let (converted, _) = editor.replace_image(&EntryTarget::Name(EMPTY.to_owned()), &image, SizeChange::Refuse).unwrap();
        assert_eq!(crate::enhanced::textures::decode_tga_rgba8(&converted.bytes).unwrap(), image);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replaces_a_dds_with_an_image_in_its_format_and_mip_rule() {
        const NAME: &str = "data/interface/default/icons/actions/a.dds";
        const SIDE: u32 = 8;
        const FULL_CHAIN_LEVELS: u32 = 4;
        let image = RgbaImage::from_pixel(SIDE, SIDE, image::Rgba([40, 80, 120, 255]));
        let convention = DdsHeaderStyle::Mipmapped.convention(DdsTextureFormat::A8R8G8B8).unwrap();
        let current = encode_dds(&convention, &image, SHIPPED_MIP_FILTER).unwrap();
        let dir = std::env::temp_dir().join(format!("uocf_entry_image_dds_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut package = UopPackage::new_default();
        package.add_entry_by_name(NAME, &current, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
        let path = dir.join("Interface.uop");
        package.finalize_and_save(&path).unwrap();

        let mut editor = UopEntryEditor::load(&path).unwrap();
        let target = EntryTarget::Name(NAME.to_owned());
        let format = editor.image_format(&target).unwrap().unwrap();
        assert_eq!(format.size(), Some((SIDE, SIDE)));
        let (unchanged, _) = editor.replace_image(&target, &image, SizeChange::Refuse).unwrap();
        assert_eq!(unchanged.bytes, current, "the same image encodes to the same bytes");

        let other = RgbaImage::from_pixel(SIDE, SIDE, image::Rgba([200, 10, 30, 255]));
        let (converted, _) = editor.replace_image(&target, &other, SizeChange::Refuse).unwrap();
        let (description, levels) = decode_dds_levels(&converted.bytes).unwrap();
        assert_eq!((description.format, description.mip_count), (DdsTextureFormat::A8R8G8B8, FULL_CHAIN_LEVELS));
        assert_eq!(levels[0], other);
        assert!(editor.replace_image(&target, &RgbaImage::new(SIDE * 2, SIDE), SizeChange::Refuse).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
