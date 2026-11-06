//! The single (non-paperdoll) gumps of `gumps_cc` and `gumps_ec`, one entry each: lossless RGBA8888 by default; when
//! packing lossy, BC1 for Classic gumps and BC7 for Enhanced ones (the user, 2026-11-06,
//! `docs/agent_notes/uddp_anim_format_requirements.md` section 8).
//!
//! The payload layout is [`udd_assets::gumps::encode_single_gump_payload`]'s. BC1 keeps Classic's colour 0 as its
//! punch-through transparent black, and turns alpha below one half transparent, so the writer counts the gumps whose
//! alpha isn't binary, which BC1 makes hard-edged. BC7 stores premultiplied colour and keeps soft alpha.
//!
//! The paperdoll's body gumps stay RGBA8888 in a lossy pack: both clients hue them on exact greys only, which a block
//! codec doesn't keep (the user: "we can't lose exact grays").

use color_eyre::eyre;
use rayon::prelude::*;
use udd_container::{AddOwnedFileRequest, CompressionFlag, DataType, UddpBuilder};

pub use udd_assets::gumps::GumpPixelFormat;
use udd_assets::gumps::encode_single_gump_payload;

/// The Zstd level of BC1 single gumps unless the packer is asked for another: the level of the measurements.
pub const BC1_SINGLE_GUMP_ZSTD_LEVEL: i32 = 19;
/// The Zstd level of BC7 single gumps unless the packer is asked for another: the level of the measurements.
pub const BC7_SINGLE_GUMP_ZSTD_LEVEL: i32 = 19;

/// How many single gumps wait to be encoded side by side: enough to keep the cores busy on BC1 or BC7, few enough that
/// only a slice of the decoded pixels is held twice.
const SINGLE_GUMP_ENCODE_BATCH: usize = 512;

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

/// The builder codec of single gumps in `format` unless the packer is asked for another.
pub const fn default_single_gump_compression(format: GumpPixelFormat) -> CompressionFlag {
    match format {
        GumpPixelFormat::Rgba8888 => CompressionFlag::ZstdNoDict,
        GumpPixelFormat::Bc1 => CompressionFlag::ZstdNoDictLevel(BC1_SINGLE_GUMP_ZSTD_LEVEL),
        GumpPixelFormat::Bc7 => CompressionFlag::ZstdNoDictLevel(BC7_SINGLE_GUMP_ZSTD_LEVEL),
    }
}

/// Refuses a single gump format that isn't the package's lossy one: BC1 is for Classic gumps (`gumps_cc`), BC7 for
/// Enhanced ones (`gumps_ec`).
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
    compression: CompressionFlag,
    pending: Vec<PendingSingleGump>,
    summary: SingleGumpSummary,
}

impl SingleGumpWriter {
    pub fn new(format: GumpPixelFormat, compression: CompressionFlag) -> Self {
        Self {
            format,
            compression,
            pending: Vec::with_capacity(SINGLE_GUMP_ENCODE_BATCH),
            summary: SingleGumpSummary::default(),
        }
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

    /// Writes the gumps still waiting, and logs how many have soft alpha when they went to BC1, and how many body gumps
    /// stayed lossless.
    pub fn finish(mut self, builder: &mut UddpBuilder) -> eyre::Result<SingleGumpSummary> {
        self.flush(builder)?;
        if self.format != GumpPixelFormat::Rgba8888 {
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
        let encoded = std::mem::take(&mut self.pending)
            .into_par_iter()
            .map(|gump| {
                let soft_alpha = has_soft_alpha(&gump.rgba);
                let lossless_body = format != GumpPixelFormat::Rgba8888 && is_paperdoll_body_gump_id(gump.gump_id);
                let gump_format = if lossless_body { GumpPixelFormat::Rgba8888 } else { format };
                encode_single_gump_payload(gump.width, gump.height, &gump.rgba, gump_format)
                    .map(|payload| (gump.gump_id, gump.width, gump.height, soft_alpha, lossless_body, payload))
                    .map_err(|error| error.wrap_err(format!("encode single gump {}", gump.gump_id)))
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        for (gump_id, width, height, soft_alpha, lossless_body, payload) in encoded {
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
        }
        Ok(())
    }
}

fn has_soft_alpha(rgba: &[u8]) -> bool {
    const ALPHA_CHANNEL: usize = 3;
    const RGBA_CHANNELS: usize = 4;
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
            assert_eq!(summary, SingleGumpSummary { count: GUMP_COUNT, soft_alpha_count: 1, lossless_body_count });
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
}
