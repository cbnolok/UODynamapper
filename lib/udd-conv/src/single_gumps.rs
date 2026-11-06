//! The single (non-paperdoll) gumps of `gumps_cc` and `gumps_ec`, one entry each: lossless RGBA8888 by default; when
//! packing lossy, BC1 for Classic gumps and BC7 for Enhanced ones (the user, 2026-11-06,
//! `docs/agent_notes/uddp_anim_format_requirements.md` section 8).
//!
//! The payload layout is [`udd_assets::gumps::encode_single_gump_payload`]'s. BC1 keeps Classic's colour 0 as its
//! punch-through transparent black, and turns alpha below one half transparent, so the writer counts the gumps whose
//! alpha isn't binary, which BC1 makes hard-edged. BC7 stores premultiplied colour and keeps soft alpha.

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

    /// Writes the gumps still waiting, and logs how many have soft alpha when they went to BC1.
    pub fn finish(mut self, builder: &mut UddpBuilder) -> eyre::Result<SingleGumpSummary> {
        self.flush(builder)?;
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
                encode_single_gump_payload(gump.width, gump.height, &gump.rgba, format)
                    .map(|payload| (gump.gump_id, gump.width, gump.height, soft_alpha, payload))
                    .map_err(|error| error.wrap_err(format!("encode single gump {}", gump.gump_id)))
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        for (gump_id, width, height, soft_alpha, payload) in encoded {
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

    /// More gumps than one batch, in every format: each reads back at its id, in its format, with soft alpha counted;
    /// BC7 keeps the soft texel within a few levels.
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

        for format in [GumpPixelFormat::Rgba8888, GumpPixelFormat::Bc1, GumpPixelFormat::Bc7] {
            let mut builder = UddpBuilder::new(LookupMode::SparseId);
            let mut writer = SingleGumpWriter::new(format, default_single_gump_compression(format));
            for gump_id in 0..GUMP_COUNT {
                writer.push(&mut builder, gump_id, WIDTH, HEIGHT, rgba_of(gump_id)).unwrap();
            }
            let summary = writer.finish(&mut builder).unwrap();
            assert_eq!(summary, SingleGumpSummary { count: GUMP_COUNT, soft_alpha_count: 1 });
            crate::gump_atlas::add_gump_atlas_files(&mut builder, Vec::new(), &Default::default()).unwrap();

            let package = GumpsPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap()).unwrap();
            for gump_id in 0..GUMP_COUNT {
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
