//! An AMOU entry (one body and action) as a folder of PNGs plus a strict KDL sidecar, for editing in an image
//! editor and importing back ([`export_folder`], [`import_folder`]).
//!
//! ```kdl
//! animation-frame body=400 action=0 frames-per-direction=10
//! palette {
//!     colour 0x000000
//!     colour 0x8C7B6A keep-colour=1
//! }
//! frame 1 anchor-x=15 anchor-y=74 width=30 height=64 image="n00001.png" hue-mask="n00001_hue.png"
//! ```
//!
//! - `animation-frame`: exactly one. `body=` and `action=` name the entry (`build/animationframe/%06d/%02d.bin`).
//!   `animation-id=` (header +0x0c) defaults to the body and `version=` to 1; the writer omits them at their
//!   default. `frames-per-direction=` is the `AnimationSequence` frame count of the action, when known: the
//!   importer compares it with the sequence entry (`super::package`).
//! - `bounds x0 y0 x1 y1`: the header box, only when it is not the union of the frame boxes and their mirrors
//!   (never in EC and KR files).
//! - `palette { colour 0xRRGGBB [keep-colour=N] }`: the palette in order (byte 0 is the high byte;
//!   `keep-colour=` is palette byte 3, 0 = the colour takes the hue). With it, the import maps each pixel to the
//!   entry with its colour and flag, which keeps byte-identical round trips; a colour that is not in it takes the
//!   nearest entry with its flag (reported). Delete the node to build a new palette from the images (exact up to
//!   256 colours, median cut above).
//! - `frame <number> anchor-x= anchor-y= width= height= [image=] [hue-mask=] [action=]`: one per frame entry, any
//!   order (the file sorts them). The anchor is `(-x0, -y0)` of the frame box, the pixel of the image drawn at the
//!   mobile's position; it may be negative or outside the image. `image=` is an RGBA PNG of `width` x `height`,
//!   relative to the folder (absent only when the size is 0). Alpha is coverage: 0..7 transparent, then the 15
//!   edge levels `255 * f >> 4`, 255 solid (other values take the nearest level). `hue-mask=` is a grey PNG of the
//!   same size: 255 where the pixel takes the hue, anything else keeps its colour; without it every pixel takes
//!   the hue. `action=` defaults to the entry's action. Frame numbers are `(direction - 3) * frames + frame + 1`
//!   for the stored directions 3..7.
//! - The reader is strict: unknown nodes and properties, repeated frames, missing images and size mismatches are
//!   errors naming the line and column.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use image::ImageEncoder;
use udd_kdl::ast::{Literal, Node, SpannedNode, Value};
use udd_kdl::span::LineSpan;
use udd_kdl::strict::{
    arguments, check_properties, describe_error, error_at, in_file, property, reject_children, required_property,
    unknown_node, value_string, value_u16, value_u32,
};

use super::encode::{AmouFile, AMOU_VERSION};
use super::quantize::{index_frames, IndexingReport, RgbaFrame};
use super::{
    AnimationBounds, AnimationFrame, HUE_MASK_HUEABLE, HUE_MASK_KEEP_COLOUR, PALETTE_ENTRY_SIZE,
    PALETTE_KEEP_COLOUR_BYTE, RGBA_CHANNELS,
};
use crate::animation_sequence::STORED_DIRECTION_COUNT;

/// File name of the sidecar in an exported folder.
pub const SIDECAR_FILE_NAME: &str = "animationframe.kdl";

type KdlNode = SpannedNode<LineSpan>;

const ROOT_NODE: &str = "animation-frame";
const BOUNDS_NODE: &str = "bounds";
const PALETTE_NODE: &str = "palette";
const COLOUR_NODE: &str = "colour";
const FRAME_NODE: &str = "frame";
const BODY_PROPERTY: &str = "body";
const ACTION_PROPERTY: &str = "action";
const ANIMATION_ID_PROPERTY: &str = "animation-id";
const VERSION_PROPERTY: &str = "version";
const FRAMES_PER_DIRECTION_PROPERTY: &str = "frames-per-direction";
const KEEP_COLOUR_PROPERTY: &str = "keep-colour";
const ANCHOR_X_PROPERTY: &str = "anchor-x";
const ANCHOR_Y_PROPERTY: &str = "anchor-y";
const WIDTH_PROPERTY: &str = "width";
const HEIGHT_PROPERTY: &str = "height";
const IMAGE_PROPERTY: &str = "image";
const HUE_MASK_PROPERTY: &str = "hue-mask";
const ROOT_PROPERTIES: [&str; 5] =
    [BODY_PROPERTY, ACTION_PROPERTY, ANIMATION_ID_PROPERTY, VERSION_PROPERTY, FRAMES_PER_DIRECTION_PROPERTY];
const FRAME_PROPERTIES: [&str; 7] = [
    ANCHOR_X_PROPERTY,
    ANCHOR_Y_PROPERTY,
    WIDTH_PROPERTY,
    HEIGHT_PROPERTY,
    IMAGE_PROPERTY,
    HUE_MASK_PROPERTY,
    ACTION_PROPERTY,
];
const TOP_NODES: [&str; 4] = [ROOT_NODE, BOUNDS_NODE, PALETTE_NODE, FRAME_NODE];
const KDL_SOURCE_NAME: &str = "animationframe.kdl";
const INDENT: &str = "    ";
/// Bits of palette byte 0 and byte 1 in the packed `0xRRGGBB` colour.
const BYTE0_SHIFT: u32 = 16;
const BYTE1_SHIFT: u32 = 8;
const BYTE_MASK: u32 = 0xff;
/// Largest packed colour.
const MAX_PACKED_COLOUR: u32 = 0x00ff_ffff;
/// Digits of the frame number in the image file names (`n00001.png`).
const FRAME_NUMBER_DIGITS: usize = 5;
const IMAGE_EXTENSION: &str = "png";
const HUE_MASK_SUFFIX: &str = "_hue";

const KDL_HEADER: &str = "\
// AnimationFrame entry (AnimationFrame*.uop), uocf::enhanced::animationframe::folder.
// animation-frame body= action= [animation-id=body] [version=1] [frames-per-direction=N]
// [bounds x0 y0 x1 y1]  palette { colour 0xRRGGBB [keep-colour=1] }  (delete it to build a new palette)
// frame <number> anchor-x= anchor-y= width= height= [image=\"x.png\"] [hue-mask=\"x_hue.png\"] [action=]
// Alpha is coverage: 0..7 transparent, 255 * f >> 4 edges, 255 solid. Hue mask: 255 takes the hue.
";

/// One `frame` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderFrame {
    pub action: u16,
    pub frame_number: u16,
    pub center_x: i16,
    pub center_y: i16,
    pub width: u16,
    pub height: u16,
    pub image: Option<String>,
    pub hue_mask: Option<String>,
}

/// The sidecar contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameFolderSidecar {
    pub body: u32,
    pub action: u16,
    pub animation_id: u32,
    pub version: u32,
    pub frames_per_direction: Option<u32>,
    pub bounds: Option<AnimationBounds>,
    pub palette: Option<Vec<[u8; PALETTE_ENTRY_SIZE]>>,
    pub frames: Vec<FolderFrame>,
}

/// An imported folder: the entry bytes and what indexing did.
#[derive(Debug, Clone)]
pub struct ImportedFrames {
    pub sidecar: FrameFolderSidecar,
    pub payload: Vec<u8>,
    pub report: IndexingReport,
}

/// Writes `payload` (the AMOU entry of `body` / `action`) to `dir` as PNGs and [`SIDECAR_FILE_NAME`]. `dir` is
/// created if needed; existing files are never overwritten. `frames_per_direction` comes from the
/// `AnimationSequence` entry when the caller has it; without it, it is the frame count / 5 when the frames are
/// exactly numbered `1..=count`. Without `include_palette` the sidecar has no palette (the import builds one).
/// Returns the sidecar.
pub fn export_folder(
    payload: &[u8],
    body: u32,
    action: u16,
    frames_per_direction: Option<u32>,
    include_palette: bool,
    dir: impl AsRef<Path>,
) -> eyre::Result<FrameFolderSidecar> {
    let dir = dir.as_ref();
    let animation = AnimationFrame::load(payload)?;
    let (file, _) = AmouFile::parse(payload)?;
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;

    let mut frames = Vec::with_capacity(animation.frames.len());
    for entry in &animation.frames {
        let decoded = animation
            .decode_frame(entry)
            .wrap_err_with(|| format!("action {} frame {}", entry.action, entry.frame_number))?;
        let (width, height) = entry.size()?;
        let stem = format!("n{:0digits$}", entry.frame_number, digits = FRAME_NUMBER_DIGITS);
        let stem = if entry.action == action { stem } else { format!("a{}_{stem}", entry.action) };
        let mut frame = FolderFrame {
            action: entry.action,
            frame_number: entry.frame_number,
            center_x: decoded.center_x,
            center_y: decoded.center_y,
            width,
            height,
            image: None,
            hue_mask: None,
        };
        if width > 0 && height > 0 {
            let image_name = format!("{stem}.{IMAGE_EXTENSION}");
            write_png(&dir.join(&image_name), width, height, &decoded.data, image::ExtendedColorType::Rgba8)?;
            frame.image = Some(image_name);
            let indexed = animation.decode_frame_indexed(entry)?;
            let keeps_colour = indexed
                .coverage
                .iter()
                .zip(&decoded.hue_mask)
                .any(|(&coverage, &mask)| coverage != super::COVERAGE_TRANSPARENT && mask != HUE_MASK_HUEABLE);
            if keeps_colour {
                let mask_name = format!("{stem}{HUE_MASK_SUFFIX}.{IMAGE_EXTENSION}");
                write_png(&dir.join(&mask_name), width, height, &decoded.hue_mask, image::ExtendedColorType::L8)?;
                frame.hue_mask = Some(mask_name);
            }
        }
        frames.push(frame);
    }

    let frames_per_direction = frames_per_direction.or_else(|| {
        let count = u32::try_from(frames.len()).ok()?;
        let numbered = frames.iter().zip(1u32..).all(|(frame, number)| u32::from(frame.frame_number) == number);
        (count > 0 && count % STORED_DIRECTION_COUNT == 0 && numbered).then_some(count / STORED_DIRECTION_COUNT)
    });
    let sidecar = FrameFolderSidecar {
        body,
        action,
        animation_id: file.animation_id,
        version: file.version,
        frames_per_direction,
        bounds: (file.bounds != file.frame_union_bounds()).then_some(file.bounds),
        palette: include_palette.then(|| file.colours.clone()),
        frames,
    };
    write_new(&dir.join(SIDECAR_FILE_NAME), sidecar_to_kdl(&sidecar).as_bytes())?;
    Ok(sidecar)
}

/// Reads a folder written by [`export_folder`] (or by hand) and encodes its entry.
pub fn import_folder(dir: impl AsRef<Path>) -> eyre::Result<ImportedFrames> {
    let dir = dir.as_ref();
    let sidecar_path = dir.join(SIDECAR_FILE_NAME);
    let text = std::fs::read_to_string(&sidecar_path)
        .wrap_err_with(|| format!("failed to read {}", sidecar_path.display()))?;
    let sidecar = sidecar_from_kdl(&text).map_err(|error| in_file(&sidecar_path.display().to_string(), error))?;

    let mut rgba_frames = Vec::with_capacity(sidecar.frames.len());
    for frame in &sidecar.frames {
        let rgba = match &frame.image {
            Some(name) => read_png(&dir.join(name), frame, false)?,
            None => Vec::new(),
        };
        let hue_mask = frame.hue_mask.as_ref().map(|name| read_png(&dir.join(name), frame, true)).transpose()?;
        rgba_frames.push(RgbaFrame {
            width: frame.width,
            height: frame.height,
            center_x: frame.center_x,
            center_y: frame.center_y,
            rgba,
            hue_mask,
        });
    }
    let (palette, indexed, report) = index_frames(&rgba_frames, sidecar.palette.as_deref())?;
    let keyed: Vec<_> = sidecar
        .frames
        .iter()
        .zip(indexed)
        .map(|(frame, indexed)| (frame.action, frame.frame_number, indexed))
        .collect();
    let mut file = AmouFile::from_indexed_frames(sidecar.animation_id, palette, &keyed)?;
    file.version = sidecar.version;
    if let Some(bounds) = sidecar.bounds {
        file.bounds = bounds;
    }
    let payload = file.to_bytes()?;
    Ok(ImportedFrames { sidecar, payload, report })
}

fn write_new(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .wrap_err_with(|| format!("failed to create {} (existing files are never overwritten)", path.display()))?;
    file.write_all(bytes).wrap_err_with(|| format!("failed to write {}", path.display()))
}

fn write_png(path: &Path, width: u16, height: u16, pixels: &[u8], colour: image::ExtendedColorType) -> eyre::Result<()> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(pixels, u32::from(width), u32::from(height), colour)
        .wrap_err_with(|| format!("failed to encode {}", path.display()))?;
    write_new(path, &bytes)
}

/// The image as RGBA8, or as one grey byte per pixel for a hue mask (`HUE_MASK_HUEABLE` stays, the rest becomes
/// `HUE_MASK_KEEP_COLOUR`).
fn read_png(path: &Path, frame: &FolderFrame, grey: bool) -> eyre::Result<Vec<u8>> {
    let image = image::open(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    eyre::ensure!(
        (image.width(), image.height()) == (u32::from(frame.width), u32::from(frame.height)),
        "{} is {}x{}; frame {} says {}x{}",
        path.display(),
        image.width(),
        image.height(),
        frame.frame_number,
        frame.width,
        frame.height
    );
    if !grey {
        return Ok(image.into_rgba8().into_raw());
    }
    Ok(image
        .into_luma8()
        .into_raw()
        .into_iter()
        .map(|value| if value == HUE_MASK_HUEABLE { HUE_MASK_HUEABLE } else { HUE_MASK_KEEP_COLOUR })
        .collect())
}

/// The sidecar as KDL text.
pub fn sidecar_to_kdl(sidecar: &FrameFolderSidecar) -> String {
    let mut text = String::from(KDL_HEADER);
    write!(text, "{ROOT_NODE} {BODY_PROPERTY}={} {ACTION_PROPERTY}={}", sidecar.body, sidecar.action).unwrap();
    if sidecar.animation_id != sidecar.body {
        write!(text, " {ANIMATION_ID_PROPERTY}={}", sidecar.animation_id).unwrap();
    }
    if sidecar.version != AMOU_VERSION {
        write!(text, " {VERSION_PROPERTY}={}", sidecar.version).unwrap();
    }
    if let Some(frames) = sidecar.frames_per_direction {
        write!(text, " {FRAMES_PER_DIRECTION_PROPERTY}={frames}").unwrap();
    }
    text.push('\n');
    if let Some(bounds) = sidecar.bounds {
        writeln!(text, "{BOUNDS_NODE} {} {} {} {}", bounds.x0, bounds.y0, bounds.x1, bounds.y1).unwrap();
    }
    if let Some(palette) = &sidecar.palette {
        writeln!(text, "{PALETTE_NODE} {{").unwrap();
        for colour in palette {
            let packed = (u32::from(colour[0]) << BYTE0_SHIFT) | (u32::from(colour[1]) << BYTE1_SHIFT) | u32::from(colour[2]);
            write!(text, "{INDENT}{COLOUR_NODE} 0x{packed:06X}").unwrap();
            if colour[PALETTE_KEEP_COLOUR_BYTE] != 0 {
                write!(text, " {KEEP_COLOUR_PROPERTY}={}", colour[PALETTE_KEEP_COLOUR_BYTE]).unwrap();
            }
            text.push('\n');
        }
        writeln!(text, "}}").unwrap();
    }
    for frame in &sidecar.frames {
        write!(
            text,
            "{FRAME_NODE} {} {ANCHOR_X_PROPERTY}={} {ANCHOR_Y_PROPERTY}={} {WIDTH_PROPERTY}={} {HEIGHT_PROPERTY}={}",
            frame.frame_number, frame.center_x, frame.center_y, frame.width, frame.height
        )
        .unwrap();
        if let Some(image) = &frame.image {
            write!(text, " {IMAGE_PROPERTY}={}", kdl_string(image)).unwrap();
        }
        if let Some(mask) = &frame.hue_mask {
            write!(text, " {HUE_MASK_PROPERTY}={}", kdl_string(mask)).unwrap();
        }
        if frame.action != sidecar.action {
            write!(text, " {ACTION_PROPERTY}={}", frame.action).unwrap();
        }
        text.push('\n');
    }
    text
}

fn kdl_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Parses a sidecar (strict).
pub fn sidecar_from_kdl(text: &str) -> eyre::Result<FrameFolderSidecar> {
    let document =
        udd_kdl::parse_ast_line_span(KDL_SOURCE_NAME, text).map_err(|error| describe_error(text, &error))?;
    let mut root: Option<&KdlNode> = None;
    let mut bounds = None;
    let mut palette = None;
    let mut frame_nodes = Vec::new();
    for node in &document.nodes {
        let name: &str = &node.node_name;
        let once = |seen: bool| {
            if seen {
                Err(error_at(node.node_name.span(), format!("`{name}` repeats")))
            } else {
                Ok(())
            }
        };
        match name {
            ROOT_NODE => {
                once(root.is_some())?;
                root = Some(node);
            }
            BOUNDS_NODE => {
                once(bounds.is_some())?;
                bounds = Some(read_bounds(node)?);
            }
            PALETTE_NODE => {
                once(palette.is_some())?;
                palette = Some(read_palette(node)?);
            }
            FRAME_NODE => frame_nodes.push(node),
            _ => return Err(unknown_node(node, None, &TOP_NODES)),
        }
    }
    let root = root.ok_or_else(|| eyre::eyre!("missing the `{ROOT_NODE}` node"))?;
    check_properties(root, &ROOT_PROPERTIES)?;
    reject_children(root)?;
    arguments::<0>(root)?;
    let body = value_u32(required_property(root, BODY_PROPERTY)?, BODY_PROPERTY)?;
    let action = value_u16(required_property(root, ACTION_PROPERTY)?, ACTION_PROPERTY)?;
    let animation_id =
        property(root, ANIMATION_ID_PROPERTY).map(|value| value_u32(value, ANIMATION_ID_PROPERTY)).transpose()?;
    let version = property(root, VERSION_PROPERTY).map(|value| value_u32(value, VERSION_PROPERTY)).transpose()?;
    let frames_per_direction = property(root, FRAMES_PER_DIRECTION_PROPERTY)
        .map(|value| value_u32(value, FRAMES_PER_DIRECTION_PROPERTY))
        .transpose()?;

    let mut seen = BTreeMap::new();
    let mut frames = Vec::with_capacity(frame_nodes.len());
    for node in frame_nodes {
        let frame = read_frame(node, action)?;
        if let Some(line) = seen.insert((frame.action, frame.frame_number), node.node_name.span().0.line + 1) {
            return Err(error_at(
                node.node_name.span(),
                format!("action {} frame {} repeats (first on line {line})", frame.action, frame.frame_number),
            ));
        }
        frames.push(frame);
    }
    Ok(FrameFolderSidecar {
        body,
        action,
        animation_id: animation_id.unwrap_or(body),
        version: version.unwrap_or(AMOU_VERSION),
        frames_per_direction,
        bounds,
        palette,
        frames,
    })
}

fn read_bounds(node: &Node<LineSpan>) -> eyre::Result<AnimationBounds> {
    check_properties(node, &[])?;
    reject_children(node)?;
    let [x0, y0, x1, y1] = arguments::<4>(node)?;
    Ok(AnimationBounds {
        x0: value_i16(x0, "x0")?,
        y0: value_i16(y0, "y0")?,
        x1: value_i16(x1, "x1")?,
        y1: value_i16(y1, "y1")?,
    })
}

fn read_palette(node: &KdlNode) -> eyre::Result<Vec<[u8; PALETTE_ENTRY_SIZE]>> {
    check_properties(node, &[])?;
    arguments::<0>(node)?;
    let children = node.children.as_ref().map(|children| children.as_slice()).unwrap_or_default();
    let mut palette = Vec::with_capacity(children.len());
    for child in children {
        if &**child.node_name != COLOUR_NODE {
            return Err(unknown_node(child, Some(PALETTE_NODE), &[COLOUR_NODE]));
        }
        check_properties(child, &[KEEP_COLOUR_PROPERTY])?;
        reject_children(child)?;
        let [packed] = arguments::<1>(child)?;
        let span = packed.literal.span();
        let packed = value_u32(packed, COLOUR_NODE)?;
        if packed > MAX_PACKED_COLOUR {
            return Err(error_at(span, format!("colour 0x{packed:X} is above 0x{MAX_PACKED_COLOUR:06X}")));
        }
        let keep = property(child, KEEP_COLOUR_PROPERTY)
            .map(|value| {
                let span = value.literal.span();
                let keep = value_u32(value, KEEP_COLOUR_PROPERTY)?;
                u8::try_from(keep).map_err(|_| error_at(span, format!("{KEEP_COLOUR_PROPERTY}: {keep} is not a byte")))
            })
            .transpose()?
            .unwrap_or(0);
        palette.push([
            ((packed >> BYTE0_SHIFT) & BYTE_MASK) as u8,
            ((packed >> BYTE1_SHIFT) & BYTE_MASK) as u8,
            (packed & BYTE_MASK) as u8,
            keep,
        ]);
    }
    if palette.is_empty() {
        return Err(error_at(node.node_name.span(), "`palette` has no colours (delete it to build one)"));
    }
    Ok(palette)
}

fn read_frame(node: &KdlNode, default_action: u16) -> eyre::Result<FolderFrame> {
    check_properties(node, &FRAME_PROPERTIES)?;
    reject_children(node)?;
    let [number] = arguments::<1>(node)?;
    let frame = FolderFrame {
        action: property(node, ACTION_PROPERTY)
            .map(|value| value_u16(value, ACTION_PROPERTY))
            .transpose()?
            .unwrap_or(default_action),
        frame_number: value_u16(number, "frame number")?,
        center_x: value_i16(required_property(node, ANCHOR_X_PROPERTY)?, ANCHOR_X_PROPERTY)?,
        center_y: value_i16(required_property(node, ANCHOR_Y_PROPERTY)?, ANCHOR_Y_PROPERTY)?,
        width: value_u16(required_property(node, WIDTH_PROPERTY)?, WIDTH_PROPERTY)?,
        height: value_u16(required_property(node, HEIGHT_PROPERTY)?, HEIGHT_PROPERTY)?,
        image: property(node, IMAGE_PROPERTY).map(|value| value_string(value, IMAGE_PROPERTY)).transpose()?,
        hue_mask: property(node, HUE_MASK_PROPERTY).map(|value| value_string(value, HUE_MASK_PROPERTY)).transpose()?,
    };
    let span = node.node_name.span();
    let empty = frame.width == 0 || frame.height == 0;
    if empty && (frame.image.is_some() || frame.hue_mask.is_some()) {
        return Err(error_at(span, format!("frame {}: an empty frame has no image", frame.frame_number)));
    }
    if !empty && frame.image.is_none() {
        return Err(error_at(span, format!("frame {}: missing {IMAGE_PROPERTY}=", frame.frame_number)));
    }
    for name in frame.image.iter().chain(&frame.hue_mask) {
        let path = Path::new(name);
        if path.is_absolute() || path.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            return Err(error_at(span, format!("{name}: images are named relative to the folder, inside it")));
        }
    }
    Ok(frame)
}

fn value_i16(value: &Value<LineSpan>, field: &str) -> eyre::Result<i16> {
    let span = value.literal.span();
    if let Some(type_name) = &value.type_name {
        return Err(error_at(type_name.span(), "type annotations are not supported"));
    }
    match &*value.literal {
        Literal::Int(integer) => {
            i16::try_from(integer).map_err(|error| error_at(span, format!("{field}: not an i16 ({error})")))
        }
        _ => Err(error_at(span, format!("{field}: expected an integer"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::animationframe::encode::{IndexedFrame, COVERAGE_SOLID, COVERAGE_TRANSPARENT};

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_amou_folder_{}_{name}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        dir
    }

    fn sample_payload() -> Vec<u8> {
        let colours = vec![[0, 0, 0, 0], [200, 10, 10, 1], [90, 90, 90, 0]];
        let frame = |center_x: i16| IndexedFrame {
            width: 3,
            height: 2,
            center_x,
            center_y: 4,
            indices: vec![1, 2, 0, 0, 2, 1],
            coverage: vec![8, COVERAGE_SOLID, COVERAGE_TRANSPARENT, COVERAGE_TRANSPARENT, COVERAGE_SOLID, 3],
        };
        let frames: Vec<_> = (1..=5).map(|number| (2u16, number, frame(number as i16 - 3))).collect();
        AmouFile::from_indexed_frames(77, colours, &frames).unwrap().to_bytes().unwrap()
    }

    #[test]
    fn export_then_import_is_byte_identical_and_without_palette_still_decodes_the_same() {
        let payload = sample_payload();
        let dir = scratch_dir("round_trip");
        let sidecar = export_folder(&payload, 77, 2, None, true, &dir).unwrap();
        assert_eq!(sidecar.frames_per_direction, Some(1));
        assert!(sidecar.frames.iter().all(|frame| frame.hue_mask.is_some()));
        let imported = import_folder(&dir).unwrap();
        assert_eq!(imported.payload, payload);
        assert_eq!(imported.report.approximated, 0);

        let dir_without = scratch_dir("no_palette");
        export_folder(&payload, 77, 2, None, false, &dir_without).unwrap();
        let rebuilt = import_folder(&dir_without).unwrap();
        let (original, rebuilt) = (AnimationFrame::load(&payload).unwrap(), AnimationFrame::load(&rebuilt.payload).unwrap());
        for (a, b) in original.frames.iter().zip(&rebuilt.frames) {
            let (a, b) = (original.decode_frame(a).unwrap(), rebuilt.decode_frame(b).unwrap());
            assert_eq!((a.data, a.hue_mask), (b.data, b.hue_mask));
        }
        // Existing files are never overwritten.
        assert!(export_folder(&payload, 77, 2, None, true, &dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&dir_without).unwrap();
    }

    #[test]
    fn sidecar_text_round_trips() {
        let sidecar = FrameFolderSidecar {
            body: 400,
            action: 3,
            animation_id: 401,
            version: 2,
            frames_per_direction: Some(10),
            bounds: Some(AnimationBounds { x0: -5, y0: -6, x1: 7, y1: 8 }),
            palette: Some(vec![[1, 2, 3, 0], [0xff, 0x80, 0, 1]]),
            frames: vec![FolderFrame {
                action: 4,
                frame_number: 2,
                center_x: -3,
                center_y: 9,
                width: 2,
                height: 1,
                image: Some("x.png".into()),
                hue_mask: None,
            }],
        };
        assert_eq!(sidecar_from_kdl(&sidecar_to_kdl(&sidecar)).unwrap(), sidecar);
    }

    #[test]
    fn sidecar_reader_is_strict() {
        let base = "animation-frame body=1 action=0\n";
        let error = |text: &str| sidecar_from_kdl(text).unwrap_err().to_string();
        assert!(error("frame 1 anchor-x=0 anchor-y=0 width=1 height=1 image=\"a.png\"").contains("missing"));
        assert!(error(&format!("{base}frames 1")).contains("frames"));
        assert!(error(&format!("{base}frame 1 anchor-x=0 anchor-y=0 width=1 height=1")).contains("image="));
        assert!(error(&format!("{base}frame 1 anchor-x=0 anchor-y=0 width=1 height=1 image=\"a.png\" size=2"))
            .contains("size"));
        let repeated = format!("{base}frame 1 anchor-x=0 anchor-y=0 width=0 height=0\nframe 1 anchor-x=0 anchor-y=0 width=0 height=0");
        assert!(error(&repeated).contains("repeats"));
        assert!(error(&format!("{base}frame 1 anchor-x=0 anchor-y=0 width=1 height=1 image=\"../a.png\""))
            .contains("inside"));
        assert!(error(&format!("{base}palette {{\n colour 0x1000000\n}}")).contains("above"));
        assert!(error(&format!("{base}{base}")).contains("repeats"));
    }
}
