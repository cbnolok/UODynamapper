//! An image converted to the format of the entry it replaces ([`UopEntryEditor::replace_image`]), then stored
//! through the entry editor's checks: a `gumpartLegacyMUL.uop` entry becomes a Classic gump encoded as the shipped
//! ones ([`crate::classic::gump_write`]), a TGA entry a TGA written as the entry is
//! ([`crate::enhanced::tga_writer`]: its type, depth, RLE packing and ending).
//!
//! The entry's format comes from its package and its current payload ([`entry_image_format`]); an image of another
//! size than the entry is refused unless the caller allows it (gump and button layouts assume the size). The
//! 0x0 placeholders (gumps, and the TGA headers of the EC Interface gumps and of KR `TileArtLegacy`) take any size.

use image::RgbaImage;

use crate::audio::validate::FormatPolicy;
use crate::classic::gump_write::GumpColors;
use crate::enhanced::dds_writer::SizeChange;
use crate::enhanced::entry_edit::{package_key, EntryEdit, EntryTarget, UopEntryEditor};
use crate::enhanced::image_census::{classify_content, ContentFormat};
use crate::enhanced::tga_writer::{self, TgaConvention};

crate::eyre_imports!();

/// The package whose entries are Classic gumps.
pub const CLASSIC_GUMP_PACKAGE: &str = "gumpartlegacymul";
/// A Classic gump entry with no pixels: a 0x0 header and no rows (34 shipped, the empty `gumpart.mul` slots).
pub const PLACEHOLDER_GUMP: [u8; 8] = [0; 8];

/// What an image entry is stored as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryImageFormat {
    /// A Classic gump of this size; `None` for a placeholder, which takes any size.
    ClassicGump(Option<(u16, u16)>),
    /// A TGA written as the current entry is.
    Tga(TgaConvention),
}

impl EntryImageFormat {
    pub fn label(&self) -> String {
        match self {
            Self::ClassicGump(_) => "Classic gump".to_owned(),
            Self::Tga(convention) => convention.label(),
        }
    }

    /// The entry's size, `None` for a placeholder, which takes any size.
    pub fn size(&self) -> Option<(u32, u32)> {
        match self {
            Self::ClassicGump(size) => size.map(|(width, height)| (u32::from(width), u32::from(height))),
            Self::Tga(convention) if convention.width() == 0 || convention.height() == 0 => None,
            Self::Tga(convention) => Some((u32::from(convention.width()), u32::from(convention.height()))),
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

/// The image format of the entry `current` of the package `package_key` ([`package_key`]), when an image can be
/// converted to it.
pub fn entry_image_format(package_key: &str, current: &[u8]) -> Option<EntryImageFormat> {
    if package_key == CLASSIC_GUMP_PACKAGE {
        if current == PLACEHOLDER_GUMP {
            return Some(EntryImageFormat::ClassicGump(None));
        }
        return GumpColors::decode_uop(current).ok().map(|gump| EntryImageFormat::ClassicGump(Some((gump.width, gump.height))));
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
    if let Some((width, height)) = format.size().filter(|&size| size != image.dimensions()) {
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
        EntryImageFormat::ClassicGump(_) => {
            let (gump, conversion) = GumpColors::from_rgba(image.width(), image.height(), image.as_raw())?;
            if conversion.rounded > 0 {
                notes.push(format!("{} pixels rounded to 5 bits per channel", conversion.rounded));
            }
            if conversion.black > 0 {
                notes.push(format!("{} opaque black pixels stored as 0x0001 (0 is transparent)", conversion.black));
            }
            gump.encode_uop()?
        }
    };
    Ok(EntryImage { format, bytes, notes })
}

impl UopEntryEditor {
    /// The image format of the existing entry `target`, when an image can be converted to it.
    pub fn image_format(&self, target: &EntryTarget) -> eyre::Result<Option<EntryImageFormat>> {
        let current = self.package().unpack_file_by_hash(target.hash())?.ok_or_else(|| eyre!("no entry {}", target.label()))?;
        Ok(entry_image_format(&package_key(self.source()), &current))
    }

    /// Replaces the image entry `target` with `image` converted to its format, then checked as [`Self::replace`]
    /// checks a file.
    pub fn replace_image(&mut self, target: &EntryTarget, image: &RgbaImage, size_change: SizeChange) -> eyre::Result<(EntryImage, EntryEdit)> {
        let format = self.image_format(target)?.ok_or_else(|| {
            eyre!("{}: the entry {} is no image format uocf converts to (Classic gumps, TGA)", self.source().display(), target.label())
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
}
