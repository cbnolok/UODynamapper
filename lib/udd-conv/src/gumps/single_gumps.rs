//! The single (non-paperdoll) gumps of `gumps_cc` and `gumps_ec`, one entry each: lossless RGBA8888 by default; when
//! packing lossy, BC1 for Classic gumps and BC7 for Enhanced ones (the user, 2026-11-06,
//! `docs/agent_notes/uddp_anim_format_requirements.md` section 8). A lossy `gumps_ec` stores its gumps in the Classic
//! colour space ([`is_in_classic_colour_space`]), the Classic gumps Interface.uop repeats, as BC1 too (the user,
//! 2026-11-06).
//!
//! The payload layout is [`udd_assets::gumps::encode_single_gump_payload`]'s. BC1 keeps Classic's colour 0 as its
//! punch-through transparent black, and turns alpha below one half transparent, so the writer counts the gumps whose
//! alpha isn't binary, which BC1 makes hard-edged. BC7 stores premultiplied colour and keeps soft alpha.
//!
//! The paperdoll's body gumps stay RGBA8888 in a lossy pack: both clients hue them on exact greys only, which a block
//! codec doesn't keep (the user: "we can't lose exact grays").
//!
//! A writer given a `gumps_cc` to share with ([`SingleGumpWriter::with_shared_classic_gumps`]) leaves out each gump that
//! `gumps_cc` draws the same and lists it in the shared Classic gump table instead (the user, 2026-11-06: "i'd keep the
//! ability to both share cc gumps and pack them into ec gumps file").

use color_eyre::eyre::{self, WrapErr};
use rayon::prelude::*;
use std::sync::Arc;
use udd_container::{AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, UddpBuilder};

pub use udd_assets::gumps::GumpPixelFormat;
use udd_assets::gumps::{
    decode_single_gump_payload, encode_shared_classic_gump_table, encode_single_gump_payload, gump_image_hash,
    GumpImage, GumpsPackage, SharedClassicGump, SHARED_CLASSIC_GUMP_TABLE_ID,
};

/// The Zstd level of BC1 single gumps unless the packer is asked for another: the level of the measurements.
pub const BC1_SINGLE_GUMP_ZSTD_LEVEL: i32 = 19;
/// The Zstd level of BC7 single gumps unless the packer is asked for another: the level of the measurements.
pub const BC7_SINGLE_GUMP_ZSTD_LEVEL: i32 = 19;

/// How many single gumps wait to be encoded side by side: enough to keep the cores busy on BC1 or BC7, few enough that
/// only a slice of the decoded pixels is held twice.
const SINGLE_GUMP_ENCODE_BATCH: usize = 512;
/// The shared Classic gump table holds ids and hashes, not pixels, so it never takes an image codec.
const SHARED_CLASSIC_GUMP_TABLE_COMPRESSION: CompressionFlag = CompressionFlag::ZstdNoDict;

/// The paperdoll's body gump of each mobile body, as both clients pick it; they agree. Enhanced:
/// `SelectPaperdollBodyGumpForBody` (UOSA.exe 0x0050d1c0, called by `UOPaperdoll::BuildImage` 0x0050c790). Classic:
/// `DrawPaperdollGumpLayers` (client.exe 7.0.103.0 0x0051a4a0). Body 987 takes 13 for a woman; any other body takes
/// 12 or 13 by gender (Enhanced) or 12/13, 14/15, 666/665 by race (Classic), all listed. Gump 51253 is in the
/// paperdoll equipment range, so it is on an atlas page, lossless already.
///
/// The Enhanced client always draws the body with partial hue (`UOPaperdoll::DrawGumpLayer` 0x00509b30); Classic draws
/// these bodies with partial hue (`BlitGumpRowPartialHue` 0x005d8430) and other bodies, through the race fallback, with
/// full hue. Both hue exact greys only, indexed by the red channel.
pub const PAPERDOLL_BODY_GUMP_BY_BODY: [(u16, u32); 16] = [
    (400, 12),
    (401, 13),
    (402, 12),
    (403, 13),
    (605, 14),
    (606, 15),
    (607, 14),
    (608, 15),
    (666, 666),
    (667, 665),
    (694, 666),
    (695, 665),
    (744, 12),
    (745, 13),
    (987, 12),
    (1253, 51_253),
];

/// Whether the paperdoll draws `gump_id` as a body, with partial hue: see [`PAPERDOLL_BODY_GUMP_BY_BODY`].
pub fn is_paperdoll_body_gump_id(gump_id: u32) -> bool {
    PAPERDOLL_BODY_GUMP_BY_BODY.iter().any(|&(_, body_gump)| body_gump == gump_id)
}

/// How far the Classic gump decoder shifts each 5-bit colour channel left to make it 8 bits (`c << 3`, so 0, 8, ...,
/// 248), which leaves the low 3 bits clear. uocf names no constant for it: the expansion is `rgba8888_word_from_bgra5551`
/// and `bulk_convert_bgra5551_to_rgba8888` in `lib/uocf/src/utils/color.rs`, the colour table of
/// `lib/uocf/src/classic/ui/gump.rs`.
pub const CLASSIC_COLOUR_CHANNEL_SHIFT: u32 = 3;
/// The low bits of an 8-bit channel that the Classic expansion leaves clear.
const CLASSIC_COLOUR_LOW_BITS: u8 = (1 << CLASSIC_COLOUR_CHANNEL_SHIFT) - 1;

const RGBA_CHANNELS: usize = 4;
const ALPHA_CHANNEL: usize = 3;

/// A gump image to compare: its size, upscale factor and straight RGBA8888 pixels.
#[derive(Debug, Clone, Copy)]
pub struct GumpPixels<'a> {
    pub width: u32,
    pub height: u32,
    pub upscale_factor: u16,
    pub rgba: &'a [u8],
}

impl<'a> GumpPixels<'a> {
    /// The pixels of a single gump, which stores its whole canvas (only atlas gumps are trimmed).
    pub fn of(image: &'a GumpImage) -> Self {
        debug_assert!(!image.is_trimmed(), "a single gump stores its whole canvas");
        Self {
            width: image.physical_width,
            height: image.physical_height,
            upscale_factor: image.upscale_factor,
            rgba: &image.stored_rgba,
        }
    }
}

/// Whether two images of gump `gump_id` draw the same: the same size and upscale factor, and the same RGBA except the
/// red, green and blue under texels whose alpha is 0 in both. The paperdoll body gumps ([`is_paperdoll_body_gump_id`])
/// must match exactly. The Enhanced paperdoll keyed each layer on its first texel's colour whatever its alpha; a
/// `gumps_ec` now bakes that rule into its body gumps' alpha at packing time (`crate::ec_gumps`), leaving their clear
/// texels transparent black as the Classic decoder's, so the exact match still holds for a twin.
pub fn gump_images_draw_the_same(gump_id: u32, a: GumpPixels, b: GumpPixels) -> bool {
    if (a.width, a.height, a.upscale_factor) != (b.width, b.height, b.upscale_factor) || a.rgba.len() != b.rgba.len() {
        return false;
    }
    if is_paperdoll_body_gump_id(gump_id) {
        return a.rgba == b.rgba;
    }
    a.rgba.chunks_exact(RGBA_CHANNELS).zip(b.rgba.chunks_exact(RGBA_CHANNELS)).all(|(a, b)| {
        a[ALPHA_CHANNEL] == b[ALPHA_CHANNEL] && (a[ALPHA_CHANNEL] == u8::MIN || a[..ALPHA_CHANNEL] == b[..ALPHA_CHANNEL])
    })
}

/// Whether straight RGBA8888 pixels are in the Classic gump decoder's colour space: alpha only 0 or 255, and every
/// opaque texel's red, green and blue with the low [`CLASSIC_COLOUR_CHANNEL_SHIFT`] bits clear. The colour under a fully
/// transparent texel doesn't count, and an image with no opaque texel is in it.
///
/// Among the 3403 single gumps of Interface.uop it picks exactly the 2216 that are Classic gumps again (measured
/// 2026-11-06): their opaque pixels match the Classic client's decode, alpha on or off, and 1456 of them differ from it
/// only in the colour under transparent texels. It needs no Classic files, so `gumps_ec` stays self-contained.
pub fn is_in_classic_colour_space(rgba: &[u8]) -> bool {
    rgba.chunks_exact(RGBA_CHANNELS).all(|pixel| match pixel[ALPHA_CHANNEL] {
        u8::MIN => true,
        u8::MAX => pixel[..ALPHA_CHANNEL].iter().all(|channel| channel & CLASSIC_COLOUR_LOW_BITS == 0),
        _ => false,
    })
}

/// The builder codec of single gumps in `format` unless the packer is asked for another.
pub const fn default_single_gump_compression(format: GumpPixelFormat) -> CompressionFlag {
    match format {
        GumpPixelFormat::Rgba8888 => CompressionFlag::ZstdNoDict,
        GumpPixelFormat::Bc1 => CompressionFlag::ZstdNoDictLevel(BC1_SINGLE_GUMP_ZSTD_LEVEL),
        GumpPixelFormat::Bc7 => CompressionFlag::ZstdNoDictLevel(BC7_SINGLE_GUMP_ZSTD_LEVEL),
    }
}

/// Refuses a single gump format that isn't the package's lossy one: BC1 is for Classic gumps (`gumps_cc`), BC7 for
/// Enhanced ones (`gumps_ec`, whose lossy pack still stores its gumps in the Classic colour space as BC1).
pub fn ensure_single_gump_format_for_package(
    format: GumpPixelFormat,
    lossy_format: GumpPixelFormat,
    package: &str,
) -> eyre::Result<()> {
    eyre::ensure!(
        format == GumpPixelFormat::Rgba8888 || format == lossy_format,
        "{package} single gumps are lossless RGBA8888 or {}, not {}",
        lossy_format.label(),
        format.label()
    );
    Ok(())
}

/// Counts of the single gumps written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SingleGumpSummary {
    pub count: u32,
    /// Gumps with a texel whose alpha is neither 0 nor 255: BC1 rounds it to one or the other, BC7 keeps it.
    pub soft_alpha_count: u32,
    /// Paperdoll body gumps kept RGBA8888 in a lossy pack.
    pub lossless_body_count: u32,
    /// Gumps stored in the writer's Classic colour space format, when it has one: BC1 in a lossy `gumps_ec`.
    pub classic_colour_space_count: u32,
    /// Gumps left out because the `gumps_cc` shared with draws them the same; `count` and the counts above leave them out.
    pub shared_classic_count: u32,
    /// Gumps stored although `gumps_cc` holds them too, because its copy is the same gump in a lossier format.
    pub kept_classic_lossier_count: u32,
}

/// How a single gump compares with the `gumps_cc` the writer shares with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClassicShare {
    /// The writer shares with no `gumps_cc`.
    Standalone,
    /// `gumps_cc` draws it the same: left out, with the [`gump_image_hash`] of the `gumps_cc` image.
    Shared(u64),
    /// Stored: `gumps_cc` has no single gump with its id, or draws it differently.
    Kept,
    /// Stored: `gumps_cc`'s copy is this gump through a lossier format.
    KeptClassicLossier,
}

/// How a gump compares with `gumps_cc`'s single gump of the same id, if any. It is shared when `gumps_cc`'s image draws
/// the same ([`gump_images_draw_the_same`]) as the one this package would store, `payload` decoded, or as the exact
/// source: with both packages lossless or both lossy the stored copies are equal, and a lossless `gumps_cc` gives a lossy
/// package the exact image. A lossy `gumps_cc` copy that a lossless package's gump doesn't match is "lossier" when it is
/// the source through `gumps_cc`'s format.
fn compare_with_classic(
    classic: &GumpsPackage,
    gump: &PendingSingleGump,
    format: GumpPixelFormat,
    payload: &[u8],
) -> eyre::Result<ClassicShare> {
    let Some((classic_format, classic_image)) = classic
        .read_single_gump_image(gump.gump_id)
        .wrap_err_with(|| format!("read gump {} from the gumps_cc package shared with", gump.gump_id))?
    else {
        return Ok(ClassicShare::Kept);
    };
    let classic_pixels = GumpPixels::of(&classic_image);
    let stored = decode_single_gump_payload(payload)?;
    let source =
        GumpPixels { width: gump.width, height: gump.height, upscale_factor: stored.upscale_factor, rgba: &gump.rgba };
    if gump_images_draw_the_same(gump.gump_id, classic_pixels, GumpPixels::of(&stored))
        || gump_images_draw_the_same(gump.gump_id, classic_pixels, source)
    {
        return Ok(ClassicShare::Shared(gump_image_hash(&classic_image)));
    }
    if classic_format != GumpPixelFormat::Rgba8888 && classic_format != format {
        let through_classic_format = decode_single_gump_payload(&encode_single_gump_payload(
            gump.width,
            gump.height,
            &gump.rgba,
            classic_format,
        )?)?;
        if gump_images_draw_the_same(gump.gump_id, classic_pixels, GumpPixels::of(&through_classic_format)) {
            return Ok(ClassicShare::KeptClassicLossier);
        }
    }
    Ok(ClassicShare::Kept)
}

struct PendingSingleGump {
    gump_id: u32,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Adds single gumps to a package in `format`, encoding them in batches on the rayon pool, in the order given.
pub struct SingleGumpWriter {
    format: GumpPixelFormat,
    /// The format of the gumps in the Classic colour space, when they don't take `format`.
    classic_colour_space_format: Option<GumpPixelFormat>,
    compression: CompressionFlag,
    pending: Vec<PendingSingleGump>,
    summary: SingleGumpSummary,
    /// The `gumps_cc` whose gumps that draw the same are left out.
    shared_with: Option<Arc<GumpsPackage>>,
    shared: Vec<SharedClassicGump>,
}

impl SingleGumpWriter {
    pub fn new(format: GumpPixelFormat, compression: CompressionFlag) -> Self {
        Self {
            format,
            classic_colour_space_format: None,
            compression,
            pending: Vec::with_capacity(SINGLE_GUMP_ENCODE_BATCH),
            summary: SingleGumpSummary::default(),
            shared_with: None,
            shared: Vec::new(),
        }
    }

    /// Leaves out each gump that `classic`, a `gumps_cc`, draws the same, and lists it in the shared Classic gump table
    /// that [`Self::finish`] writes.
    pub fn with_shared_classic_gumps(mut self, classic: Arc<GumpsPackage>) -> Self {
        self.shared_with = Some(classic);
        self
    }

    /// Stores the gumps in the Classic colour space ([`is_in_classic_colour_space`]) as `format`, and the others as the
    /// writer's format; the paperdoll body gumps stay RGBA8888 either way. The rule sees the pixels as pushed, after any
    /// upscale pass.
    pub fn with_classic_colour_space_format(mut self, format: GumpPixelFormat) -> Self {
        self.classic_colour_space_format = Some(format);
        self
    }

    pub fn push(
        &mut self,
        builder: &mut UddpBuilder,
        gump_id: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> eyre::Result<()> {
        self.pending.push(PendingSingleGump { gump_id, width, height, rgba });
        if self.pending.len() >= SINGLE_GUMP_ENCODE_BATCH {
            self.flush(builder)?;
        }
        Ok(())
    }

    /// Writes the gumps still waiting and, when gumps were left out, the shared Classic gump table, and logs how many have
    /// soft alpha when they went to BC1, how many body gumps stayed lossless, how many took the Classic colour space
    /// format, and how many were shared.
    pub fn finish(mut self, builder: &mut UddpBuilder) -> eyre::Result<SingleGumpSummary> {
        self.flush(builder)?;
        if self.shared_with.is_some() {
            log::info!(
                "{} single gumps shared with gumps_cc, left out to be drawn from it; {} kept, {} of them because \
                 gumps_cc's copy is lossier",
                self.summary.shared_classic_count,
                self.summary.count,
                self.summary.kept_classic_lossier_count
            );
        }
        if !self.shared.is_empty() {
            self.shared.sort_unstable_by_key(|shared| shared.gump_id);
            let table = encode_shared_classic_gump_table(&self.shared)?;
            builder.add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: SHARED_CLASSIC_GUMP_TABLE_COMPRESSION,
                width: 0,
                height: 0,
                virtual_path: None,
                path_hash64: None,
                id: Some(SHARED_CLASSIC_GUMP_TABLE_ID),
                data: &table,
            })?;
        }
        if let Some(classic_colour_space_format) = self.classic_colour_space_format {
            log::info!(
                "{} single gumps in the Classic colour space as {}, {} as {}, {} paperdoll body gumps RGBA8888: both \
                 clients hue them on exact greys",
                self.summary.classic_colour_space_count,
                classic_colour_space_format.label(),
                self.summary.count - self.summary.classic_colour_space_count - self.summary.lossless_body_count,
                self.format.label(),
                self.summary.lossless_body_count
            );
        } else if self.format != GumpPixelFormat::Rgba8888 {
            log::info!(
                "{} paperdoll body gumps stay RGBA8888 among {} {} single gumps: both clients hue them on exact greys",
                self.summary.lossless_body_count,
                self.summary.count,
                self.format.label()
            );
        }
        if self.format == GumpPixelFormat::Bc1 && self.summary.soft_alpha_count > 0 {
            log::info!(
                "{} of {} single gumps have alpha between 0 and 255, which BC1 makes transparent below half and opaque above",
                self.summary.soft_alpha_count,
                self.summary.count
            );
        }
        Ok(self.summary)
    }

    fn flush(&mut self, builder: &mut UddpBuilder) -> eyre::Result<()> {
        let format = self.format;
        let classic_colour_space_format = self.classic_colour_space_format;
        let shared_with = self.shared_with.as_deref();
        let encoded = std::mem::take(&mut self.pending)
            .into_par_iter()
            .map(|gump| {
                let soft_alpha = has_soft_alpha(&gump.rgba);
                let lossless_body = format != GumpPixelFormat::Rgba8888 && is_paperdoll_body_gump_id(gump.gump_id);
                let classic_format = classic_colour_space_format
                    .filter(|_| !lossless_body && is_in_classic_colour_space(&gump.rgba));
                let gump_format = match (lossless_body, classic_format) {
                    (true, _) => GumpPixelFormat::Rgba8888,
                    (false, Some(classic_format)) => classic_format,
                    (false, None) => format,
                };
                let classic_colour_space = classic_format.is_some();
                let payload = encode_single_gump_payload(gump.width, gump.height, &gump.rgba, gump_format)
                    .map_err(|error| error.wrap_err(format!("encode single gump {}", gump.gump_id)))?;
                let share = match shared_with {
                    Some(classic) => compare_with_classic(classic, &gump, gump_format, &payload)
                        .wrap_err_with(|| format!("compare single gump {} with gumps_cc", gump.gump_id))?,
                    None => ClassicShare::Standalone,
                };
                Ok((gump.gump_id, gump.width, gump.height, soft_alpha, lossless_body, classic_colour_space, payload, share))
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        for (gump_id, width, height, soft_alpha, lossless_body, classic_colour_space, payload, share) in encoded {
            match share {
                ClassicShare::Shared(image_hash) => {
                    self.shared.push(SharedClassicGump { gump_id, image_hash });
                    self.summary.shared_classic_count += 1;
                    continue;
                }
                ClassicShare::KeptClassicLossier => self.summary.kept_classic_lossier_count += 1,
                ClassicShare::Standalone | ClassicShare::Kept => {}
            }
            builder.add_owned_file(AddOwnedFileRequest {
                data_type: DataType::Gump as u8,
                compression: self.compression,
                width,
                height,
                virtual_path: None,
                path_hash64: None,
                id: Some(gump_id),
                data: payload,
            })?;
            self.summary.count += 1;
            self.summary.soft_alpha_count += u32::from(soft_alpha);
            self.summary.lossless_body_count += u32::from(lossless_body);
            self.summary.classic_colour_space_count += u32::from(classic_colour_space);
        }
        Ok(())
    }
}

fn has_soft_alpha(rgba: &[u8]) -> bool {
    rgba.chunks_exact(RGBA_CHANNELS)
        .any(|pixel| pixel[ALPHA_CHANNEL] != u8::MIN && pixel[ALPHA_CHANNEL] != u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::gumps::GumpsPackage;
    use udd_container::{LookupMode, UddpReader};

    /// Each package takes lossless gumps or its own lossy format: BC1 for Classic, BC7 for Enhanced.
    #[test]
    fn each_package_refuses_the_other_lossy_format() {
        use crate::cc_gumps::CC_LOSSY_SINGLE_GUMP_FORMAT;
        use crate::ec_gumps::EC_LOSSY_SINGLE_GUMP_FORMAT;
        for (lossy, other) in [
            (CC_LOSSY_SINGLE_GUMP_FORMAT, EC_LOSSY_SINGLE_GUMP_FORMAT),
            (EC_LOSSY_SINGLE_GUMP_FORMAT, CC_LOSSY_SINGLE_GUMP_FORMAT),
        ] {
            assert!(ensure_single_gump_format_for_package(GumpPixelFormat::Rgba8888, lossy, "package").is_ok());
            assert!(ensure_single_gump_format_for_package(lossy, lossy, "package").is_ok());
            let error = ensure_single_gump_format_for_package(other, lossy, "package").unwrap_err();
            assert!(error.to_string().contains(other.label()), "{error}");
        }
        assert_eq!(CC_LOSSY_SINGLE_GUMP_FORMAT, GumpPixelFormat::Bc1);
        assert_eq!(EC_LOSSY_SINGLE_GUMP_FORMAT, GumpPixelFormat::Bc7);
    }

    /// The clients' body gumps: six single gumps, and one in the paperdoll equipment range, on an atlas page.
    #[test]
    fn paperdoll_body_gumps_are_six_single_gumps_and_one_atlas_gump() {
        const SINGLE_BODY_GUMPS: [u32; 6] = [12, 13, 14, 15, 665, 666];
        let mut gumps = PAPERDOLL_BODY_GUMP_BY_BODY.map(|(_, gump)| gump).to_vec();
        gumps.sort_unstable();
        gumps.dedup();
        let (atlas, single): (Vec<u32>, Vec<u32>) =
            gumps.into_iter().partition(|&gump| crate::gump_atlas::is_paperdoll_equipment_gump_id(gump));
        assert_eq!(single, SINGLE_BODY_GUMPS);
        assert_eq!(atlas.len(), 1);
    }

    /// More gumps than one batch, in every format: each reads back at its id, in its format, with soft alpha counted;
    /// BC7 keeps the soft texel within a few levels, and the paperdoll body gumps among them stay RGBA8888.
    #[test]
    fn single_gumps_read_back_in_their_format() {
        const GUMP_COUNT: u32 = SINGLE_GUMP_ENCODE_BATCH as u32 + 3;
        const WIDTH: u32 = 6;
        const HEIGHT: u32 = 5;
        const OPAQUE_WHITE: [u8; 4] = [255, 255, 255, 255];
        const SOFT_WHITE: [u8; 4] = [255, 255, 255, 200];
        const SOFT_GUMP_ID: u32 = 1;
        /// What BC7 may move a channel of these gumps by.
        const BC7_TOLERANCE: u8 = 2;
        let rgba_of = |gump_id: u32| {
            let mut rgba = OPAQUE_WHITE.repeat((WIDTH * HEIGHT) as usize);
            if gump_id == SOFT_GUMP_ID {
                rgba[..SOFT_WHITE.len()].copy_from_slice(&SOFT_WHITE);
            }
            rgba
        };
        let body_count = (0..GUMP_COUNT).filter(|&gump_id| is_paperdoll_body_gump_id(gump_id)).count() as u32;
        assert!(body_count > 0);

        for format in [GumpPixelFormat::Rgba8888, GumpPixelFormat::Bc1, GumpPixelFormat::Bc7] {
            let mut builder = UddpBuilder::new(LookupMode::SparseId);
            let mut writer = SingleGumpWriter::new(format, default_single_gump_compression(format));
            for gump_id in 0..GUMP_COUNT {
                writer.push(&mut builder, gump_id, WIDTH, HEIGHT, rgba_of(gump_id)).unwrap();
            }
            let summary = writer.finish(&mut builder).unwrap();
            let lossless_body_count = if format == GumpPixelFormat::Rgba8888 { 0 } else { body_count };
            assert_eq!(
                summary,
                SingleGumpSummary {
                    count: GUMP_COUNT,
                    soft_alpha_count: 1,
                    lossless_body_count,
                    classic_colour_space_count: 0,
                    ..SingleGumpSummary::default()
                }
            );
            crate::gump_atlas::add_gump_atlas_files(&mut builder, Vec::new(), &Default::default()).unwrap();

            let package = GumpsPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap()).unwrap();
            for gump_id in 0..GUMP_COUNT {
                let format = if is_paperdoll_body_gump_id(gump_id) { GumpPixelFormat::Rgba8888 } else { format };
                assert_eq!(package.single_gump_pixel_format(gump_id).unwrap(), Some(format));
                let (width, height, rgba) = package.read_gump_rgba(gump_id).unwrap();
                assert_eq!((width, height), (WIDTH, HEIGHT));
                let expected = match (format, gump_id) {
                    (GumpPixelFormat::Bc1, SOFT_GUMP_ID) => rgba_of(SOFT_GUMP_ID)
                        .chunks_exact(4)
                        .flat_map(|pixel| if pixel[3] == u8::MAX { pixel.to_vec() } else { OPAQUE_WHITE.to_vec() })
                        .collect(),
                    _ => rgba_of(gump_id),
                };
                if format == GumpPixelFormat::Bc7 {
                    let worst = rgba.iter().zip(&expected).map(|(read, written)| read.abs_diff(*written)).max();
                    assert!(worst <= Some(BC7_TOLERANCE), "{format:?} gump {gump_id}: off by {worst:?}");
                } else {
                    assert_eq!(rgba, expected, "{format:?} gump {gump_id}");
                }
            }
        }
    }

    /// The Classic colour space: alpha 0 or 255, and opaque channels on the decoder's steps of 8 (any of the three low
    /// bits set leaves it); the colour under a clear texel doesn't count, so an image with no opaque texel is in it.
    #[test]
    fn classic_colour_space_is_binary_alpha_and_channels_on_classic_steps() {
        const CLASSIC_OPAQUE: [u8; 4] = [248, 128, 8, 255];
        const CLASSIC_BLACK: [u8; 4] = [0, 0, 0, 255];
        const COLOURED_CLEAR: [u8; 4] = [255, 7, 93, 0];
        const SOFT_ALPHAS: [u8; 3] = [1, 128, 254];
        const CLEAR_TEXELS: usize = 4;
        let classic = [CLASSIC_OPAQUE, CLASSIC_BLACK, COLOURED_CLEAR].concat();
        assert!(is_in_classic_colour_space(&classic));

        for channel in 0..ALPHA_CHANNEL {
            for bit in 0..CLASSIC_COLOUR_CHANNEL_SHIFT {
                let mut off_step = classic.clone();
                off_step[channel] |= 1 << bit;
                assert!(!is_in_classic_colour_space(&off_step), "channel {channel}, bit {bit}");
            }
        }
        for soft_alpha in SOFT_ALPHAS {
            let mut soft = classic.clone();
            soft[ALPHA_CHANNEL] = soft_alpha;
            assert!(!is_in_classic_colour_space(&soft), "alpha {soft_alpha}");
        }
        assert!(is_in_classic_colour_space(&COLOURED_CLEAR.repeat(CLEAR_TEXELS)));
        assert!(is_in_classic_colour_space(&[]));
    }

    /// A writer with a Classic colour space format, as a lossy `gumps_ec`'s: the gumps in the Classic colour space take
    /// BC1, encoded as a `gumps_cc` BC1 gump is, the others BC7, and a body gump RGBA8888 even in the Classic colour
    /// space; the summary counts each.
    #[test]
    fn classic_colour_space_gumps_take_the_classic_colour_space_format() {
        const WIDTH: u32 = 5;
        const HEIGHT: u32 = 3;
        const CLASSIC_GUMP_ID: u32 = 1000;
        const ENHANCED_GUMP_ID: u32 = 1001;
        const SOFT_GUMP_ID: u32 = 1002;
        const BODY_GUMP_ID: u32 = 12;
        const CLASSIC_OPAQUE: [u8; 4] = [248, 128, 8, 255];
        const ENHANCED_OPAQUE: [u8; 4] = [250, 129, 9, 255];
        const CLASSIC_SOFT: [u8; 4] = [248, 128, 8, 200];
        const COLOURED_CLEAR: [u8; 4] = [255, 7, 93, 0];
        let with_clear_texel = |pixel: [u8; 4]| {
            let mut rgba = pixel.repeat((WIDTH * HEIGHT) as usize);
            rgba[..COLOURED_CLEAR.len()].copy_from_slice(&COLOURED_CLEAR);
            rgba
        };
        let gumps = [
            (CLASSIC_GUMP_ID, with_clear_texel(CLASSIC_OPAQUE), GumpPixelFormat::Bc1),
            (ENHANCED_GUMP_ID, with_clear_texel(ENHANCED_OPAQUE), GumpPixelFormat::Bc7),
            (SOFT_GUMP_ID, with_clear_texel(CLASSIC_SOFT), GumpPixelFormat::Bc7),
            (BODY_GUMP_ID, with_clear_texel(CLASSIC_OPAQUE), GumpPixelFormat::Rgba8888),
        ];
        assert!(is_paperdoll_body_gump_id(BODY_GUMP_ID));

        let mut builder = UddpBuilder::new(LookupMode::SparseId);
        let mut writer = SingleGumpWriter::new(GumpPixelFormat::Bc7, default_single_gump_compression(GumpPixelFormat::Bc7))
            .with_classic_colour_space_format(GumpPixelFormat::Bc1);
        for (gump_id, rgba, _) in &gumps {
            writer.push(&mut builder, *gump_id, WIDTH, HEIGHT, rgba.clone()).unwrap();
        }
        let summary = writer.finish(&mut builder).unwrap();
        assert_eq!(
            summary,
            SingleGumpSummary {
                count: gumps.len() as u32,
                soft_alpha_count: 1,
                lossless_body_count: 1,
                classic_colour_space_count: 1,
                ..SingleGumpSummary::default()
            }
        );
        crate::gump_atlas::add_gump_atlas_files(&mut builder, Vec::new(), &Default::default()).unwrap();

        let package = GumpsPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap()).unwrap();
        for (gump_id, rgba, format) in &gumps {
            assert_eq!(package.single_gump_pixel_format(*gump_id).unwrap(), Some(*format), "gump {gump_id}");
            let stored = package.package().read_file_by_sparse_id(*gump_id).unwrap();
            assert_eq!(stored, encode_single_gump_payload(WIDTH, HEIGHT, rgba, *format).unwrap(), "gump {gump_id}");
        }
    }

    /// Equality for sharing ignores the colour under texels clear in both, but not alpha, size or upscale factor, and a
    /// paperdoll body gump must match exactly.
    #[test]
    fn gumps_draw_the_same_whatever_colour_their_clear_texels_hold() {
        const GUMP_ID: u32 = 1000;
        const BODY_GUMP_ID: u32 = 12;
        const OPAQUE: [u8; 4] = [248, 128, 8, 255];
        const CLEAR_BLACK: [u8; 4] = [0, 0, 0, 0];
        const COLOURED_CLEAR: [u8; 4] = [255, 7, 93, 0];
        const ALMOST_CLEAR: [u8; 4] = [0, 0, 0, 1];
        const TEXELS: usize = 2;
        fn pixels(rgba: &[u8]) -> GumpPixels<'_> {
            GumpPixels { width: TEXELS as u32, height: 1, upscale_factor: 1, rgba }
        }
        let plain = [OPAQUE, CLEAR_BLACK].concat();
        let coloured = [OPAQUE, COLOURED_CLEAR].concat();
        let almost = [OPAQUE, ALMOST_CLEAR].concat();
        assert!(is_paperdoll_body_gump_id(BODY_GUMP_ID));

        assert!(gump_images_draw_the_same(GUMP_ID, pixels(&plain), pixels(&coloured)));
        assert!(!gump_images_draw_the_same(GUMP_ID, pixels(&plain), pixels(&almost)));
        assert!(!gump_images_draw_the_same(BODY_GUMP_ID, pixels(&plain), pixels(&coloured)));
        assert!(gump_images_draw_the_same(BODY_GUMP_ID, pixels(&plain), pixels(&plain)));
        let upscaled = GumpPixels { upscale_factor: 2, ..pixels(&plain) };
        let transposed = GumpPixels { width: 1, height: TEXELS as u32, ..pixels(&plain) };
        for other in [upscaled, transposed] {
            assert!(!gump_images_draw_the_same(GUMP_ID, pixels(&plain), other), "{other:?}");
        }
    }

    /// One `gumps_ec` writer against one `gumps_cc` per pairing of lossless and lossy: which gumps it leaves out, which
    /// it keeps as lossier in `gumps_cc`, and the table it writes, with each shared gump's `gumps_cc` image hash.
    #[test]
    fn a_shared_writer_leaves_out_what_gumps_cc_draws_the_same() {
        const WIDTH: u32 = 5;
        const HEIGHT: u32 = 3;
        const IDENTICAL: u32 = 1000;
        const CLEAR_COLOUR: u32 = 1001;
        const DIFFERENT: u32 = 1002;
        const RESIZED: u32 = 1003;
        const ENHANCED_ONLY: u32 = 1004;
        const BODY_IDENTICAL: u32 = 13;
        const BODY_CLEAR_COLOUR: u32 = 12;
        const CLASSIC_OPAQUE: [u8; 4] = [248, 128, 8, 255];
        const OTHER_OPAQUE: [u8; 4] = [8, 248, 128, 255];
        const CLEAR_BLACK: [u8; 4] = [0, 0, 0, 0];
        const COLOURED_CLEAR: [u8; 4] = [255, 7, 93, 0];
        let with_clear = |pixel: [u8; 4], clear: [u8; 4]| {
            let mut rgba = pixel.repeat((WIDTH * HEIGHT) as usize);
            rgba[..clear.len()].copy_from_slice(&clear);
            (WIDTH, HEIGHT, rgba)
        };
        let classic_gumps = [
            (IDENTICAL, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK)),
            (CLEAR_COLOUR, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK)),
            (DIFFERENT, with_clear(OTHER_OPAQUE, CLEAR_BLACK)),
            (RESIZED, (HEIGHT, WIDTH, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK).2)),
            (BODY_IDENTICAL, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK)),
            (BODY_CLEAR_COLOUR, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK)),
        ];
        let enhanced_gumps = [
            (IDENTICAL, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK)),
            (CLEAR_COLOUR, with_clear(CLASSIC_OPAQUE, COLOURED_CLEAR)),
            (DIFFERENT, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK)),
            (RESIZED, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK)),
            (ENHANCED_ONLY, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK)),
            (BODY_IDENTICAL, with_clear(CLASSIC_OPAQUE, CLEAR_BLACK)),
            (BODY_CLEAR_COLOUR, with_clear(CLASSIC_OPAQUE, COLOURED_CLEAR)),
        ];
        let writer = |format| SingleGumpWriter::new(format, default_single_gump_compression(format));
        let lossless = || writer(GumpPixelFormat::Rgba8888);
        let classic_lossy = || writer(GumpPixelFormat::Bc1);
        let enhanced_lossy = || writer(GumpPixelFormat::Bc7).with_classic_colour_space_format(GumpPixelFormat::Bc1);
        let write = |writer: SingleGumpWriter, gumps: &[(u32, (u32, u32, Vec<u8>))]| {
            let mut builder = UddpBuilder::new(LookupMode::SparseId);
            let mut writer = writer;
            for (gump_id, (width, height, rgba)) in gumps {
                writer.push(&mut builder, *gump_id, *width, *height, rgba.clone()).unwrap();
            }
            let summary = writer.finish(&mut builder).unwrap();
            crate::gump_atlas::add_gump_atlas_files(&mut builder, Vec::new(), &Default::default()).unwrap();
            (summary, GumpsPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap()).unwrap())
        };

        // In ascending id order, as the table holds them.
        let both_lossless_shared = vec![BODY_IDENTICAL, IDENTICAL, CLEAR_COLOUR];
        let cases = [
            ("both lossless", lossless(), lossless(), both_lossless_shared.clone(), Vec::new()),
            ("both lossy", classic_lossy(), enhanced_lossy(), both_lossless_shared.clone(), Vec::new()),
            ("lossy gumps_ec, lossless gumps_cc", lossless(), enhanced_lossy(), both_lossless_shared, Vec::new()),
            (
                "lossless gumps_ec, lossy gumps_cc",
                classic_lossy(),
                lossless(),
                vec![BODY_IDENTICAL],
                vec![IDENTICAL, CLEAR_COLOUR],
            ),
        ];
        for (case, classic_writer, enhanced_writer, expected_shared, expected_lossier) in cases {
            let (_, classic) = write(classic_writer, &classic_gumps);
            let classic = Arc::new(classic);
            let (summary, enhanced) = write(enhanced_writer.with_shared_classic_gumps(classic.clone()), &enhanced_gumps);

            let expected_table = expected_shared
                .iter()
                .map(|&gump_id| SharedClassicGump {
                    gump_id,
                    image_hash: gump_image_hash(&classic.read_gump_image(gump_id).unwrap()),
                })
                .collect::<Vec<_>>();
            assert_eq!(enhanced.shared_classic_gumps().collect::<Vec<_>>(), expected_table, "{case}");
            assert_eq!(summary.shared_classic_count, expected_shared.len() as u32, "{case}");
            assert_eq!(summary.kept_classic_lossier_count, expected_lossier.len() as u32, "{case}");
            assert_eq!(summary.count as usize, enhanced_gumps.len() - expected_shared.len(), "{case}");
            for (gump_id, _) in &enhanced_gumps {
                let stored = enhanced.read_single_gump_image(*gump_id).unwrap().is_some();
                assert_eq!(stored, !expected_shared.contains(gump_id), "{case}: gump {gump_id}");
            }
        }

        let (summary, standalone) = write(lossless(), &enhanced_gumps);
        assert_eq!((summary.shared_classic_count, standalone.shared_classic_gump_count()), (0, 0));
        assert!(standalone.package().find_by_sparse_id(SHARED_CLASSIC_GUMP_TABLE_ID).is_none());
    }
}
