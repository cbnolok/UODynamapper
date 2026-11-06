//! A body and action of Classic animation frames as a folder of RGBA PNGs plus [`SIDECAR_FILE_NAME`] (strict KDL):
//! an `AnimationFrame*.uop` `animationlegacyframe` entry (one palette, frames by number), or the five `anim*.mul`
//! entries of the stored directions (a palette and frames by index each).
//!
//! With the palettes, an unedited folder imports back byte for byte; without one, a palette is built from the
//! images ([`index_rgba_frames`]). Images are opaque (alpha 255) or transparent (alpha 0) pixels.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use uot_kdl::ast::SpannedNode;
use uot_kdl::span::LineSpan;
use uot_kdl::strict::{
    arguments, check_properties, describe_error, error_at, in_file, property, reject_children, required_property,
    unknown_node, value_string, value_u16, value_u32,
};

use super::mul::ClassicMulEntry;
use super::uop::{ClassicUopEntry, ClassicUopFrame, FIRST_FRAME_NUMBER};
use super::{index_rgba_frames, ClassicFrame, ClassicPalette, ClassicRgbaFrame, ColourReport, PALETTE_COLOURS};
use crate::animation_sequence::STORED_DIRECTION_COUNT;
use crate::enhanced::animationframe::folder::{kdl_string, value_i16, write_new, write_png};

/// File name of the sidecar in an exported folder.
pub const SIDECAR_FILE_NAME: &str = "classicanimation.kdl";

type KdlNode = SpannedNode<LineSpan>;

const ROOT_NODE: &str = "classic-animation";
const DIRECTION_NODE: &str = "direction";
const PALETTE_NODE: &str = "palette";
const COLOUR_NODE: &str = "colour";
const FRAME_NODE: &str = "frame";
const BODY_PROPERTY: &str = "body";
const ACTION_PROPERTY: &str = "action";
const SOURCE_PROPERTY: &str = "source";
const FILE_PROPERTY: &str = "file";
const FRAMES_PER_DIRECTION_PROPERTY: &str = "frames-per-direction";
const FRAMES_PROPERTY: &str = "frames";
const ANCHOR_X_PROPERTY: &str = "anchor-x";
const ANCHOR_Y_PROPERTY: &str = "anchor-y";
const WIDTH_PROPERTY: &str = "width";
const HEIGHT_PROPERTY: &str = "height";
const IMAGE_PROPERTY: &str = "image";
const SOURCE_UOP: &str = "uop";
const SOURCE_MUL: &str = "mul";
const ROOT_PROPERTIES: [&str; 5] =
    [BODY_PROPERTY, ACTION_PROPERTY, SOURCE_PROPERTY, FILE_PROPERTY, FRAMES_PER_DIRECTION_PROPERTY];
const FRAME_PROPERTIES: [&str; 5] = [ANCHOR_X_PROPERTY, ANCHOR_Y_PROPERTY, WIDTH_PROPERTY, HEIGHT_PROPERTY, IMAGE_PROPERTY];
const UOP_TOP_NODES: [&str; 3] = [ROOT_NODE, PALETTE_NODE, FRAME_NODE];
const MUL_TOP_NODES: [&str; 2] = [ROOT_NODE, DIRECTION_NODE];
const GROUP_NODES: [&str; 2] = [PALETTE_NODE, FRAME_NODE];
const KDL_SOURCE_NAME: &str = "classicanimation.kdl";
const INDENT: &str = "    ";
const IMAGE_EXTENSION: &str = "png";
/// Digits of the frame number in UOP image names (`n00001.png`) and of the index in MUL ones (`d0_f000.png`).
const UOP_FRAME_DIGITS: usize = 5;
const MUL_FRAME_DIGITS: usize = 3;
/// The `anim.mul` file number (`file=1`); `file=N` is `animN.mul`.
pub const FIRST_MUL_FILE: u8 = 1;

const KDL_HEADER: &str = "\
// Classic animation frames, uocf::classic::anim_frames::folder.
// classic-animation body= action= source=\"uop\" [frames-per-direction=N]: an AnimationFrame*.uop
//   animationlegacyframe entry: [palette] and frame <number> lines (numbers from 1 over the 5 directions).
// classic-animation body= action= source=\"mul\" file=N: the anim.mul (file=1) or animN.mul entries of the
//   directions: direction <0..4> frames=<count> { [palette] frame <index> ... } (an index not listed: no frame).
// palette { colour 0xWORD }: the 256 RGB555 words in index order (trailing 0 words left out); delete it to build
//   one from the images. frame <n> anchor-x= anchor-y= width= height= [image=\"x.png\"]: alpha 0 or 255 only.
";

/// Which file the folder came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSource {
    /// An `AnimationFrame*.uop` `animationlegacyframe` entry.
    Uop,
    /// `anim.mul` (`file` [`FIRST_MUL_FILE`]) or `anim<file>.mul`.
    Mul { file: u8 },
}

/// One `frame` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderFrame {
    /// UOP: the frame number; MUL: the frame index in its direction.
    pub number: u16,
    pub center_x: i16,
    pub center_y: i16,
    pub width: u16,
    pub height: u16,
    pub image: Option<String>,
}

/// The frames sharing one palette: the UOP entry (no direction), or one MUL direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameGroup {
    pub direction: Option<u8>,
    /// MUL: the entry's frame count (frames not listed are missing).
    pub frame_count: Option<u16>,
    pub palette: Option<ClassicPalette>,
    pub frames: Vec<FolderFrame>,
}

/// The sidecar contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicFolderSidecar {
    pub body: u32,
    pub action: u16,
    pub source: FrameSource,
    /// UOP: frames per direction, from the caller or inferred.
    pub frames_per_direction: Option<u32>,
    pub groups: Vec<FrameGroup>,
}

/// An imported folder: the sidecar and, per group, its palette, frames and what indexing did.
#[derive(Debug, Clone)]
pub struct ImportedClassicFrames {
    pub sidecar: ClassicFolderSidecar,
    pub groups: Vec<ImportedGroup>,
}

#[derive(Debug, Clone)]
pub struct ImportedGroup {
    pub direction: Option<u8>,
    pub palette: ClassicPalette,
    /// `None` for a missing MUL frame.
    pub frames: Vec<(u16, Option<ClassicFrame>)>,
    pub report: ColourReport,
}

impl ImportedClassicFrames {
    /// The UOP entry: the frames replace those of `base` (the entry being replaced), whose container fields are
    /// kept; without it, a new entry.
    pub fn uop_payload(&self, base: Option<&[u8]>) -> eyre::Result<Vec<u8>> {
        eyre::ensure!(self.sidecar.source == FrameSource::Uop, "the folder holds anim*.mul frames");
        let [group] = self.groups.as_slice() else {
            eyre::bail!("a UOP folder has one palette group, found {}", self.groups.len());
        };
        let mut entry = match base {
            Some(base) => ClassicUopEntry::parse(base).wrap_err("the entry to replace")?,
            None => ClassicUopEntry::new(self.sidecar.body, group.palette),
        };
        entry.palette = group.palette;
        entry.frames = group
            .frames
            .iter()
            .filter_map(|(number, frame)| {
                frame.clone().map(|frame| ClassicUopFrame { action: self.sidecar.action, frame_number: *number, frame })
            })
            .collect();
        entry.to_bytes()
    }

    /// The MUL entries, by direction.
    pub fn mul_payloads(&self) -> eyre::Result<Vec<(u8, Vec<u8>)>> {
        eyre::ensure!(matches!(self.sidecar.source, FrameSource::Mul { .. }), "the folder holds a UOP entry");
        self.groups
            .iter()
            .map(|group| {
                let direction = group.direction.expect("MUL groups have a direction");
                let entry = ClassicMulEntry {
                    palette: group.palette,
                    frames: group.frames.iter().map(|(_, frame)| frame.clone()).collect(),
                };
                Ok((direction, entry.to_bytes().wrap_err_with(|| format!("direction {direction}"))?))
            })
            .collect()
    }
}

fn frame_line(dir: &Path, name: String, number: u16, frame: &ClassicFrame, palette: &ClassicPalette) -> eyre::Result<FolderFrame> {
    let mut line = FolderFrame {
        number,
        center_x: frame.center_x,
        center_y: frame.center_y,
        width: frame.width,
        height: frame.height,
        image: None,
    };
    if !frame.is_empty() {
        let image = format!("{name}.{IMAGE_EXTENSION}");
        let rgba = frame.to_rgba(palette);
        write_png(&dir.join(&image), frame.width, frame.height, &rgba.data, image::ExtendedColorType::Rgba8)?;
        line.image = Some(image);
    }
    Ok(line)
}

/// Writes the UOP entry `payload` of `body` / `action` to `dir` (created if needed; existing files are never
/// overwritten). `frames_per_direction` comes from the caller when known, else is the frame count / 5 when the
/// frames are numbered `1..=count`. Returns the sidecar.
pub fn export_uop_folder(
    payload: &[u8],
    body: u32,
    action: u16,
    frames_per_direction: Option<u32>,
    include_palette: bool,
    dir: impl AsRef<Path>,
) -> eyre::Result<ClassicFolderSidecar> {
    let dir = dir.as_ref();
    let entry = ClassicUopEntry::parse(payload)?;
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;
    let mut frames = Vec::with_capacity(entry.frames.len());
    for frame in &entry.frames {
        eyre::ensure!(
            frame.action == action,
            "frame {} has action {}: an entry's frames have its action {action}",
            frame.frame_number,
            frame.action
        );
        let name = format!("n{:0digits$}", frame.frame_number, digits = UOP_FRAME_DIGITS);
        frames.push(frame_line(dir, name, frame.frame_number, &frame.frame, &entry.palette)?);
    }
    let frames_per_direction = frames_per_direction.or_else(|| {
        let count = u32::try_from(frames.len()).ok()?;
        let numbered = frames.iter().zip(u32::from(FIRST_FRAME_NUMBER)..).all(|(frame, number)| u32::from(frame.number) == number);
        (count > 0 && count % STORED_DIRECTION_COUNT == 0 && numbered).then_some(count / STORED_DIRECTION_COUNT)
    });
    let sidecar = ClassicFolderSidecar {
        body,
        action,
        source: FrameSource::Uop,
        frames_per_direction,
        groups: vec![FrameGroup { direction: None, frame_count: None, palette: include_palette.then_some(entry.palette), frames }],
    };
    write_new(&dir.join(SIDECAR_FILE_NAME), sidecar_to_kdl(&sidecar).as_bytes())?;
    Ok(sidecar)
}

/// Writes the MUL entries `(direction, payload)` of `body` / `action` in `anim<file>.mul` to `dir`, as
/// [`export_uop_folder`].
pub fn export_mul_folder(
    entries: &[(u8, Vec<u8>)],
    file: u8,
    body: u32,
    action: u16,
    include_palette: bool,
    dir: impl AsRef<Path>,
) -> eyre::Result<ClassicFolderSidecar> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;
    let mut groups = Vec::with_capacity(entries.len());
    for (direction, payload) in entries {
        let entry = ClassicMulEntry::parse(payload).wrap_err_with(|| format!("direction {direction}"))?;
        let mut frames = Vec::new();
        for (index, frame) in entry.frames.iter().enumerate() {
            let Some(frame) = frame else { continue };
            let index = u16::try_from(index)?;
            let name = format!("d{direction}_f{index:0digits$}", digits = MUL_FRAME_DIGITS);
            frames.push(frame_line(dir, name, index, frame, &entry.palette)?);
        }
        groups.push(FrameGroup {
            direction: Some(*direction),
            frame_count: Some(u16::try_from(entry.frames.len())?),
            palette: include_palette.then_some(entry.palette),
            frames,
        });
    }
    let sidecar = ClassicFolderSidecar { body, action, source: FrameSource::Mul { file }, frames_per_direction: None, groups };
    write_new(&dir.join(SIDECAR_FILE_NAME), sidecar_to_kdl(&sidecar).as_bytes())?;
    Ok(sidecar)
}

/// Reads a folder written by [`export_uop_folder`] or [`export_mul_folder`] (or by hand) and indexes its images.
pub fn import_folder(dir: impl AsRef<Path>) -> eyre::Result<ImportedClassicFrames> {
    let dir = dir.as_ref();
    let sidecar_path = dir.join(SIDECAR_FILE_NAME);
    let text = std::fs::read_to_string(&sidecar_path)
        .wrap_err_with(|| format!("failed to read {}", sidecar_path.display()))?;
    let sidecar = sidecar_from_kdl(&text).map_err(|error| in_file(&sidecar_path.display().to_string(), error))?;
    let mut groups = Vec::with_capacity(sidecar.groups.len());
    for group in &sidecar.groups {
        let context = || group.direction.map_or(String::from("the entry"), |direction| format!("direction {direction}"));
        let mut rgba = Vec::with_capacity(group.frames.len());
        for frame in &group.frames {
            let pixels = match &frame.image {
                Some(name) => read_png(&dir.join(name), frame)?,
                None => Vec::new(),
            };
            rgba.push(ClassicRgbaFrame {
                center_x: frame.center_x,
                center_y: frame.center_y,
                width: frame.width,
                height: frame.height,
                rgba: pixels,
            });
        }
        let (palette, indexed, report) = index_rgba_frames(&rgba, group.palette.as_ref()).wrap_err_with(context)?;
        let mut by_number: BTreeMap<u16, ClassicFrame> =
            group.frames.iter().zip(indexed).map(|(line, frame)| (line.number, frame)).collect();
        let frames = match group.frame_count {
            Some(count) => (0..count).map(|index| (index, by_number.remove(&index))).collect(),
            None => by_number.into_iter().map(|(number, frame)| (number, Some(frame))).collect(),
        };
        groups.push(ImportedGroup { direction: group.direction, palette, frames, report });
    }
    Ok(ImportedClassicFrames { sidecar, groups })
}

fn read_png(path: &Path, frame: &FolderFrame) -> eyre::Result<Vec<u8>> {
    let image = image::open(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    eyre::ensure!(
        (image.width(), image.height()) == (u32::from(frame.width), u32::from(frame.height)),
        "{} is {}x{}; frame {} says {}x{}",
        path.display(),
        image.width(),
        image.height(),
        frame.number,
        frame.width,
        frame.height
    );
    Ok(image.into_rgba8().into_raw())
}

/// The sidecar as KDL text.
pub fn sidecar_to_kdl(sidecar: &ClassicFolderSidecar) -> String {
    let mut text = String::from(KDL_HEADER);
    write!(text, "{ROOT_NODE} {BODY_PROPERTY}={} {ACTION_PROPERTY}={}", sidecar.body, sidecar.action).unwrap();
    match sidecar.source {
        FrameSource::Uop => write!(text, " {SOURCE_PROPERTY}={}", kdl_string(SOURCE_UOP)).unwrap(),
        FrameSource::Mul { file } => write!(text, " {SOURCE_PROPERTY}={} {FILE_PROPERTY}={file}", kdl_string(SOURCE_MUL)).unwrap(),
    }
    if let Some(frames) = sidecar.frames_per_direction {
        write!(text, " {FRAMES_PER_DIRECTION_PROPERTY}={frames}").unwrap();
    }
    text.push('\n');
    for group in &sidecar.groups {
        let indent = match group.direction {
            Some(direction) => {
                write!(text, "{DIRECTION_NODE} {direction}").unwrap();
                if let Some(count) = group.frame_count {
                    write!(text, " {FRAMES_PROPERTY}={count}").unwrap();
                }
                text.push_str(" {\n");
                INDENT
            }
            None => "",
        };
        if let Some(palette) = &group.palette {
            writeln!(text, "{indent}{PALETTE_NODE} {{").unwrap();
            let used = palette.iter().rposition(|&word| word != 0).map_or(0, |last| last + 1);
            for word in &palette[..used] {
                writeln!(text, "{indent}{INDENT}{COLOUR_NODE} 0x{word:04X}").unwrap();
            }
            writeln!(text, "{indent}}}").unwrap();
        }
        for frame in &group.frames {
            write!(
                text,
                "{indent}{FRAME_NODE} {} {ANCHOR_X_PROPERTY}={} {ANCHOR_Y_PROPERTY}={} {WIDTH_PROPERTY}={} {HEIGHT_PROPERTY}={}",
                frame.number, frame.center_x, frame.center_y, frame.width, frame.height
            )
            .unwrap();
            if let Some(image) = &frame.image {
                write!(text, " {IMAGE_PROPERTY}={}", kdl_string(image)).unwrap();
            }
            text.push('\n');
        }
        if group.direction.is_some() {
            text.push_str("}\n");
        }
    }
    text
}

/// Parses a sidecar (strict).
pub fn sidecar_from_kdl(text: &str) -> eyre::Result<ClassicFolderSidecar> {
    let document =
        uot_kdl::parse_ast_line_span(KDL_SOURCE_NAME, text).map_err(|error| describe_error(text, &error))?;
    let root = document
        .nodes
        .iter()
        .find(|node| &**node.node_name == ROOT_NODE)
        .ok_or_else(|| eyre::eyre!("missing the `{ROOT_NODE}` node"))?;
    check_properties(root, &ROOT_PROPERTIES)?;
    reject_children(root)?;
    arguments::<0>(root)?;
    let body = value_u32(required_property(root, BODY_PROPERTY)?, BODY_PROPERTY)?;
    let action = value_u16(required_property(root, ACTION_PROPERTY)?, ACTION_PROPERTY)?;
    let source_value = required_property(root, SOURCE_PROPERTY)?;
    let file = property(root, FILE_PROPERTY).map(|value| value_u32(value, FILE_PROPERTY).map(|file| (file, value))).transpose()?;
    let frames_per_direction = property(root, FRAMES_PER_DIRECTION_PROPERTY)
        .map(|value| value_u32(value, FRAMES_PER_DIRECTION_PROPERTY))
        .transpose()?;
    let source = match value_string(source_value, SOURCE_PROPERTY)?.as_str() {
        SOURCE_UOP => {
            if let Some((_, value)) = file {
                return Err(error_at(value.literal.span(), format!("{FILE_PROPERTY}= is for source=\"{SOURCE_MUL}\"")));
            }
            FrameSource::Uop
        }
        SOURCE_MUL => {
            let (file, value) = file.ok_or_else(|| error_at(root.node_name.span(), format!("source=\"{SOURCE_MUL}\" needs {FILE_PROPERTY}=")))?;
            let file = u8::try_from(file)
                .ok()
                .filter(|&file| file >= FIRST_MUL_FILE)
                .ok_or_else(|| error_at(value.literal.span(), format!("{FILE_PROPERTY}: {file} is not an anim file number")))?;
            if frames_per_direction.is_some() {
                return Err(error_at(root.node_name.span(), format!("{FRAMES_PER_DIRECTION_PROPERTY}= is for source=\"{SOURCE_UOP}\"")));
            }
            FrameSource::Mul { file }
        }
        other => {
            return Err(error_at(
                source_value.literal.span(),
                format!("{SOURCE_PROPERTY}: \"{other}\" is not \"{SOURCE_UOP}\" or \"{SOURCE_MUL}\""),
            ))
        }
    };

    let mut groups = Vec::new();
    let mut roots = 0;
    match source {
        FrameSource::Uop => {
            let nodes: Vec<&KdlNode> = document.nodes.iter().collect();
            let mut group = FrameGroup { direction: None, frame_count: None, palette: None, frames: Vec::new() };
            for node in nodes {
                match &**node.node_name {
                    ROOT_NODE => roots += 1,
                    PALETTE_NODE | FRAME_NODE => read_group_node(node, &mut group)?,
                    _ => return Err(unknown_node(node, None, &UOP_TOP_NODES)),
                }
            }
            groups.push(group);
        }
        FrameSource::Mul { .. } => {
            for node in &document.nodes {
                match &**node.node_name {
                    ROOT_NODE => roots += 1,
                    DIRECTION_NODE => {
                        let group = read_direction(node)?;
                        if groups.iter().any(|other: &FrameGroup| other.direction == group.direction) {
                            return Err(error_at(node.node_name.span(), format!("direction {} repeats", group.direction.unwrap_or_default())));
                        }
                        groups.push(group);
                    }
                    _ => return Err(unknown_node(node, None, &MUL_TOP_NODES)),
                }
            }
        }
    }
    if roots > 1 {
        return Err(error_at(root.node_name.span(), format!("`{ROOT_NODE}` repeats")));
    }
    Ok(ClassicFolderSidecar { body, action, source, frames_per_direction, groups })
}

fn read_direction(node: &KdlNode) -> eyre::Result<FrameGroup> {
    check_properties(node, &[FRAMES_PROPERTY])?;
    let [direction] = arguments::<1>(node)?;
    let span = direction.literal.span();
    let direction = value_u32(direction, DIRECTION_NODE)?;
    let direction = u8::try_from(direction)
        .ok()
        .filter(|&direction| u32::from(direction) < STORED_DIRECTION_COUNT)
        .ok_or_else(|| error_at(span, format!("direction {direction} is not 0..{STORED_DIRECTION_COUNT}")))?;
    let frame_count = value_u16(required_property(node, FRAMES_PROPERTY)?, FRAMES_PROPERTY)?;
    let mut group = FrameGroup { direction: Some(direction), frame_count: Some(frame_count), palette: None, frames: Vec::new() };
    for child in node.children.as_ref().map(|children| children.as_slice()).unwrap_or_default() {
        match &**child.node_name {
            PALETTE_NODE | FRAME_NODE => read_group_node(child, &mut group)?,
            _ => return Err(unknown_node(child, Some(DIRECTION_NODE), &GROUP_NODES)),
        }
    }
    if let Some(frame) = group.frames.iter().find(|frame| frame.number >= frame_count) {
        return Err(error_at(node.node_name.span(), format!("direction {direction}: frame {} is not below {FRAMES_PROPERTY}={frame_count}", frame.number)));
    }
    Ok(group)
}

fn read_group_node(node: &KdlNode, group: &mut FrameGroup) -> eyre::Result<()> {
    if &**node.node_name == PALETTE_NODE {
        if group.palette.is_some() {
            return Err(error_at(node.node_name.span(), format!("`{PALETTE_NODE}` repeats")));
        }
        group.palette = Some(read_palette(node)?);
        return Ok(());
    }
    let frame = read_frame(node)?;
    if group.frames.iter().any(|other| other.number == frame.number) {
        return Err(error_at(node.node_name.span(), format!("frame {} repeats", frame.number)));
    }
    group.frames.push(frame);
    Ok(())
}

fn read_palette(node: &KdlNode) -> eyre::Result<ClassicPalette> {
    check_properties(node, &[])?;
    arguments::<0>(node)?;
    let children = node.children.as_ref().map(|children| children.as_slice()).unwrap_or_default();
    if children.is_empty() {
        return Err(error_at(node.node_name.span(), "`palette` has no colours (delete it to build one)"));
    }
    if children.len() > PALETTE_COLOURS {
        return Err(error_at(node.node_name.span(), format!("{} colours: a palette holds {PALETTE_COLOURS}", children.len())));
    }
    let mut palette = [0u16; PALETTE_COLOURS];
    for (slot, child) in palette.iter_mut().zip(children) {
        if &**child.node_name != COLOUR_NODE {
            return Err(unknown_node(child, Some(PALETTE_NODE), &[COLOUR_NODE]));
        }
        check_properties(child, &[])?;
        reject_children(child)?;
        let [word] = arguments::<1>(child)?;
        *slot = value_u16(word, COLOUR_NODE)?;
    }
    Ok(palette)
}

fn read_frame(node: &KdlNode) -> eyre::Result<FolderFrame> {
    check_properties(node, &FRAME_PROPERTIES)?;
    reject_children(node)?;
    let [number] = arguments::<1>(node)?;
    let frame = FolderFrame {
        number: value_u16(number, "frame number")?,
        center_x: value_i16(required_property(node, ANCHOR_X_PROPERTY)?, ANCHOR_X_PROPERTY)?,
        center_y: value_i16(required_property(node, ANCHOR_Y_PROPERTY)?, ANCHOR_Y_PROPERTY)?,
        width: value_u16(required_property(node, WIDTH_PROPERTY)?, WIDTH_PROPERTY)?,
        height: value_u16(required_property(node, HEIGHT_PROPERTY)?, HEIGHT_PROPERTY)?,
        image: property(node, IMAGE_PROPERTY).map(|value| value_string(value, IMAGE_PROPERTY)).transpose()?,
    };
    let span = node.node_name.span();
    let empty = frame.width == 0 || frame.height == 0;
    if empty && frame.image.is_some() {
        return Err(error_at(span, format!("frame {}: an empty frame has no image", frame.number)));
    }
    if !empty && frame.image.is_none() {
        return Err(error_at(span, format!("frame {}: missing {IMAGE_PROPERTY}=", frame.number)));
    }
    if let Some(name) = &frame.image {
        let path = Path::new(name);
        if path.is_absolute() || path.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            return Err(error_at(span, format!("{name}: images are named relative to the folder, inside it")));
        }
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_classic_folder_{}_{name}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        dir
    }

    fn palette() -> ClassicPalette {
        let mut palette = [0u16; PALETTE_COLOURS];
        palette[0] = 0xdc18;
        palette[1] = 0x7C00;
        palette[2] = 0x83E0; // bit 15 kept
        palette[3] = 0x0001;
        palette
    }

    fn frame(center_x: i16) -> ClassicFrame {
        ClassicFrame { center_x, center_y: -4, width: 3, height: 2, pixels: vec![Some(1), Some(2), None, None, Some(3), Some(1)] }
    }

    #[test]
    fn uop_folders_round_trip() {
        let mut entry = ClassicUopEntry::new(400, palette());
        for number in 1..=5u16 {
            entry.frames.push(ClassicUopFrame { action: 2, frame_number: number, frame: frame(number as i16) });
        }
        entry.frames[2].frame = ClassicFrame::empty(0, 0);
        let payload = entry.to_bytes().unwrap();
        let dir = scratch_dir("uop");
        let sidecar = export_uop_folder(&payload, 400, 2, None, true, &dir).unwrap();
        assert_eq!(sidecar.frames_per_direction, Some(1));
        let imported = import_folder(&dir).unwrap();
        assert_eq!(imported.uop_payload(Some(&payload)).unwrap(), payload);
        assert_eq!(imported.uop_payload(None).unwrap(), payload, "a new entry is laid out the same");
        assert!(export_uop_folder(&payload, 400, 2, None, true, &dir).is_err(), "never overwrites");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mul_folders_round_trip_and_rebuild_palettes() {
        let entries: Vec<(u8, Vec<u8>)> = (0..STORED_DIRECTION_COUNT as u8)
            .map(|direction| {
                let entry = ClassicMulEntry { palette: palette(), frames: vec![None, Some(frame(i16::from(direction))), Some(frame(1))] };
                (direction, entry.to_bytes().unwrap())
            })
            .collect();
        let dir = scratch_dir("mul");
        export_mul_folder(&entries, 2, 200, 4, true, &dir).unwrap();
        assert_eq!(import_folder(&dir).unwrap().mul_payloads().unwrap(), entries);

        let dir_without = scratch_dir("mul_without");
        export_mul_folder(&entries, 2, 200, 4, false, &dir_without).unwrap();
        for ((_, original), (_, rebuilt)) in entries.iter().zip(import_folder(&dir_without).unwrap().mul_payloads().unwrap()) {
            let (original, rebuilt) = (ClassicMulEntry::parse(original).unwrap(), ClassicMulEntry::parse(&rebuilt).unwrap());
            for (a, b) in original.frames.iter().zip(&rebuilt.frames) {
                let rgba = |frame: &Option<ClassicFrame>, palette| frame.as_ref().map(|frame| frame.to_rgba(palette).data);
                assert_eq!(rgba(a, &original.palette), rgba(b, &rebuilt.palette));
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&dir_without).unwrap();
    }

    #[test]
    fn sidecar_reader_is_strict() {
        let error = |text: &str| sidecar_from_kdl(text).unwrap_err().to_string();
        let uop = "classic-animation body=1 action=0 source=\"uop\"\n";
        let mul = "classic-animation body=1 action=0 source=\"mul\" file=1\n";
        assert!(error("frame 1 anchor-x=0 anchor-y=0 width=0 height=0").contains("missing"));
        assert!(error("classic-animation body=1 action=0 source=\"zip\"").contains("zip"));
        assert!(error("classic-animation body=1 action=0 source=\"mul\"").contains("file="));
        assert!(error(&format!("{uop}direction 0 frames=1")).contains("direction"));
        assert!(error(&format!("{mul}frame 0 anchor-x=0 anchor-y=0 width=0 height=0")).contains("frame"));
        assert!(error(&format!("{mul}direction 5 frames=1")).contains("not 0..5"));
        assert!(error(&format!("{mul}direction 0 frames=1 {{\n frame 1 anchor-x=0 anchor-y=0 width=0 height=0\n}}")).contains("below"));
        assert!(error(&format!("{uop}frame 1 anchor-x=0 anchor-y=0 width=1 height=1")).contains("image="));
        assert!(error(&format!("{uop}palette {{\n colour 0x10000\n}}")).contains("colour"));
        assert!(error(&format!("{uop}{uop}")).contains("repeats"));
        assert!(error(&format!("{uop}frame 1 anchor-x=0 anchor-y=0 width=1 height=1 image=\"../a.png\"")).contains("inside"));
    }
}
