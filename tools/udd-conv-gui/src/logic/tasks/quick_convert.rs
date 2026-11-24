use std::fs;
use udd_conv::classic_sources::resolve_classic_map_source;
use udd_conv::animation::mobile_strides::keep_staged_strides;
use udd_conv::kr_sections::has_kr_section;
use uocf::classic::facet_sources::CLASSIC_FACET_NAMES;
use uocf::utils::output_file::write_through_temp;

use crate::logic::settings::enhanced_client_has_world_textures;
use crate::models::QuickConvertMode;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PackageJob {
    TileMeta,
    Hues,
    WorldLights,
    ArtCc,
    LandCc,
    MobileCc,
    GumpsCc,
    Map(u32),
    Statics(u32),
    ArtEc,
    LandEc,
    AuxEc,
    MobileEc,
    GumpsEc,
    EffectsEc,
    PaperdollEc,
}

impl PackageJob {
    fn file_name(self) -> String {
        let name = match self {
            Self::TileMeta => "tilemeta.uddp",
            Self::Hues => HUES_OUTPUT,
            Self::WorldLights => WORLD_LIGHTS_OUTPUT,
            Self::ArtCc => "tex_art_cc.uddp",
            Self::LandCc => "tex_land_cc.uddp",
            Self::MobileCc => "mobile_anim_cc.uddp",
            Self::GumpsCc => GUMPS_CC_DEFAULT_OUTPUT,
            Self::Map(id) => return format!("map{id}.uddp"),
            Self::Statics(id) => return format!("statics{id}.uddp"),
            Self::ArtEc => "tex_art_ec.uddp",
            Self::LandEc => "tex_land_ec.uddp",
            Self::AuxEc => "tex_aux_ec.uddp",
            Self::MobileEc => "mobile_anim_ec.uddp",
            Self::GumpsEc => GUMPS_EC_DEFAULT_OUTPUT,
            Self::EffectsEc => EFFECTS_EC_OUTPUT,
            Self::PaperdollEc => PAPERDOLL_EC_OUTPUT,
        };
        name.to_string()
    }

    fn pack(self, settings: &AppSettings, output: &Path, progress: &AssetProgressReporter) -> eyre::Result<()> {
        match self {
            Self::TileMeta => pack_tilemeta(settings, output, progress).map(drop),
            Self::Hues => pack_hues(settings, output).map(drop),
            Self::WorldLights => pack_world_lights(settings, output).map(drop),
            Self::ArtCc => pack_tex_art_cc(settings, output, progress).map(drop),
            Self::LandCc => pack_tex_land_cc(settings, output, progress).map(drop),
            Self::MobileCc => pack_mobile_anim_cc(settings, output, progress).map(drop),
            Self::GumpsCc => pack_gumps_cc(settings, output, progress).map(drop),
            Self::Map(id) => pack_map(settings, output, id).map(drop),
            Self::Statics(id) => pack_statics(settings, output, id).map(drop),
            Self::ArtEc => pack_tex_art_ec(settings, output, progress).map(drop),
            Self::LandEc => pack_tex_land_ec(settings, output, progress).map(drop),
            Self::AuxEc => pack_tex_aux_ec_job(settings, output, &settings.input_uddp_dir.join(Self::TileMeta.file_name()), progress).map(drop),
            Self::MobileEc => pack_mobile_anim_ec(settings, output, progress).map(drop),
            Self::GumpsEc => {
                let cc = settings.output_uddp_dir.join(GUMPS_CC_DEFAULT_OUTPUT);
                let share = ec_gumps_share(settings.gumps_ec_sharing, &cc, cc.is_file());
                pack_gumps_ec(settings, output, &share, progress).map(drop)
            }
            Self::EffectsEc => pack_effects_ec_job(settings, output).map(drop),
            Self::PaperdollEc => pack_paperdoll_ec_job(settings, output).map(drop),
        }
    }

    fn write(self, settings: &AppSettings, output: &Path, progress: &AssetProgressReporter) -> eyre::Result<()> {
        write_through_temp(output, |temp| {
            let pack = || {
                self.pack(settings, temp, progress)?;
                progress.panic_if_cancelled();
                // The packer normally reads those records at its own output path. Carry them across staging.
                if matches!(self, Self::MobileCc | Self::MobileEc) {
                    keep_staged_strides(temp, output)?;
                    progress.panic_if_cancelled();
                }
                Ok(())
            };
            match panic::catch_unwind(AssertUnwindSafe(pack)) {
                Ok(result) => result,
                Err(payload) if payload.is::<ConversionCancelled>() => eyre::bail!("conversion cancelled"),
                Err(_) => eyre::bail!("packing panicked"),
            }
        })
    }
}

fn quick_settings(configured: &AppSettings, mode: QuickConvertMode) -> AppSettings {
    let use_enhanced = mode != QuickConvertMode::ClassicTextures;
    AppSettings {
        // Enhanced's complete tile metadata and KR auxiliary textures also need the Classic source.
        cc_dir: configured.cc_dir.as_ref().filter(|path| path.is_dir()).cloned(),
        ec_dir: configured.ec_dir.as_ref().filter(|path| use_enhanced && path.is_dir()).cloned(),
        kr_dir: configured.kr_dir.as_ref().filter(|_| use_enhanced).cloned(),
        input_uddp_dir: configured.output_uddp_dir.clone(),
        output_uddp_dir: configured.output_uddp_dir.clone(),
        opt_tex_art_cc: TextureOptimization::Bc7Zstd,
        opt_tex_art_ec: TextureOptimization::Bc7Zstd,
        opt_tex_land_ec: TextureOptimization::Bc7Zstd,
        opt_tex_aux_ec: TextureOptimization::Bc7Zstd,
        opt_mobile_anim_cc: TextureOptimization::Bc7Zstd,
        opt_mobile_anim_ec: TextureOptimization::Bc7Zstd,
        bc7_rdo_enabled: false,
        bc_encode_device: configured.bc_encode_device,
        bc7_rdo_device: configured.bc7_rdo_device,
        upscale_tex_land_ec_64: UpscaleConfig::default(),
        upscale_tex_land_ec_128: UpscaleConfig::default(),
        upscale_tex_land_ec_256: UpscaleConfig::default(),
        upscale_tex_land_ec_512: UpscaleConfig::default(),
        gumps_cc_bc1: true,
        gumps_ec_bc7: true,
        gumps_ec_sharing: if mode == QuickConvertMode::Everything { GumpsEcSharing::ShareWithGumpsCc } else { GumpsEcSharing::Standalone },
        effects_ec_bc7: true,
        ..AppSettings::default()
    }
}

fn quick_jobs(settings: &AppSettings, mode: QuickConvertMode) -> Vec<PackageJob> {
    use PackageJob::*;
    use QuickConvertMode::*;

    let classic_ready = settings.cc_dir.is_some();
    let enhanced_ready = settings.ec_dir.is_some();
    if !mode.is_available(classic_ready, enhanced_ready) {
        return Vec::new();
    }
    let metadata = matches!(mode, Metadata | Everything);
    let classic = matches!(mode, ClassicTextures | Everything);
    let enhanced = matches!(mode, EnhancedTextures | Everything);
    let mut jobs = Vec::new();

    if metadata {
        if classic_ready {
            jobs.push(TileMeta);
        }
        jobs.push(Hues);
    }
    if classic {
        jobs.extend([WorldLights, ArtCc, LandCc, GumpsCc]);
    }
    if metadata && classic_ready {
        let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
        let patches = classic_patch_options(settings);
        for (id, preference) in (0..CLASSIC_FACET_NAMES.len() as u32).zip(settings.map_preferences) {
            let Ok(source) = resolve_classic_map_source(&sources, id, preference, &patches) else {
                continue;
            };
            jobs.push(Map(id));
            if source.facet.statics.is_some() {
                jobs.push(Statics(id));
            }
        }
    }
    if enhanced {
        jobs.extend([ArtEc, LandEc]);
        if settings.cc_dir.is_some() {
            jobs.push(AuxEc);
        }
        jobs.extend([GumpsEc, EffectsEc, PaperdollEc]);
    }
    // Animation packing takes longest. Make the world and interface packages available first.
    if classic {
        jobs.push(MobileCc);
    }
    if enhanced {
        jobs.push(MobileEc);
    }
    jobs
}

impl UddConvApp {
    pub(crate) fn quick_convert_outputs(&self, mode: QuickConvertMode) -> Vec<PathBuf> {
        let settings = quick_settings(&self.settings, mode);
        quick_jobs(&settings, mode).into_iter()
            .map(|job| settings.output_uddp_dir.join(job.file_name()))
            .collect()
    }

    pub(crate) fn quick_convert(&self, mode: QuickConvertMode) {
        let settings = quick_settings(&self.settings, mode);
        let jobs = quick_jobs(&settings, mode);
        if jobs.is_empty() {
            self.push_log("Set the client source folders required by this action.", LogLevel::Error);
            return;
        }
        if settings.output_uddp_dir.as_os_str().is_empty() {
            self.push_log("Choose an output folder before converting.", LogLevel::Error);
            return;
        }
        self.spawn_asset_task(AssetPackTask::QuickConvert, "Quick Convert".to_string(), Vec::new(), move |progress| {
            if let Some(ec_dir) = settings.ec_dir.as_deref()
                && (mode != QuickConvertMode::Metadata || settings.cc_dir.is_some())
            {
                eyre::ensure!(
                    enhanced_client_has_world_textures(ec_dir),
                    "Texture.uop is missing from {}. Choose the Enhanced Client installation folder that contains Texture.uop.",
                    ec_dir.display()
                );
            }
            if mode == QuickConvertMode::EnhancedTextures {
                if settings.cc_dir.is_none() {
                    log::warn!("Skipping tex_aux_ec.uddp: it needs a Classic Client source folder.");
                } else {
                    let tilemeta = settings.input_uddp_dir.join(PackageJob::TileMeta.file_name());
                    let rebuild = "Run Create metadata with both clients configured before converting Enhanced textures.";
                    eyre::ensure!(tilemeta.is_file(), "Missing {}. {rebuild}", tilemeta.display());
                    eyre::ensure!(
                        has_kr_section(&tilemeta).wrap_err(rebuild)?,
                        "{} has no Kingdom Reborn tile data. {rebuild}",
                        tilemeta.display()
                    );
                }
            }
            fs::create_dir_all(&settings.output_uddp_dir)?;
            // Pack callbacks check the shared cancel flag. Their per-file percentages are independent of the batch counter.
            let package_progress = AssetProgressReporter {
                task: AssetPackTask::QuickConvert,
                progress: Arc::default(),
                cancel: progress.cancel.clone(),
            };
            let total = jobs.len();
            for (index, job) in jobs.into_iter().enumerate() {
                progress.panic_if_cancelled();
                let name = job.file_name();
                let output = settings.output_uddp_dir.join(&name);
                format_log::start_file(format_args!("Quick Convert {}/{total}: ", index + 1), &output);
                progress.set(AssetPackProgressState::Running, index as f32 / total as f32, format!("{}/{total}: {name}", index + 1));
                package_progress.start();
                job.write(&settings, &output, &package_progress)
                    .map_err(|error| eyre::eyre!("pack {name} after writing {index} of {total} packages: {error:#}"))?;
                format_log::written_package(&output);
            }
            Ok(format!("Wrote {total} packages to {}", settings.output_uddp_dir.display()))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_formats_keep_native_resolution_and_ignore_manual_overrides() {
        use crate::models::ComputeDevice;
        let manual = AppSettings {
            upscale_profile_path: Some(PathBuf::from("custom.kdl")),
            opt_tex_art_cc: TextureOptimization::None,
            measure_strides_mobile_anim_cc: StrideMeasurement::Full,
            include_verdata: true,
            input_uddp_dir: PathBuf::from("separate"),
            bc_encode_device: ComputeDevice::Cpu,
            bc7_rdo_device: ComputeDevice::Gpu,
            ..AppSettings::default()
        };
        let quick = quick_settings(&manual, QuickConvertMode::Everything);
        assert_eq!(quick.opt_tex_art_cc, TextureOptimization::Bc7Zstd);
        assert_eq!(quick.opt_tex_land_cc, TextureOptimization::None);
        assert_eq!(quick.opt_tex_land_ec, TextureOptimization::Bc7Zstd);
        assert_eq!(quick.opt_mobile_anim_cc, TextureOptimization::Bc7Zstd);
        assert_eq!(quick.opt_mobile_anim_ec, TextureOptimization::Bc7Zstd);
        assert_eq!(quick.upscale_tex_land_ec_64, UpscaleConfig::default());
        assert_eq!(quick.upscale_tex_land_ec_128, UpscaleConfig::default());
        assert_eq!(quick.upscale_tex_land_ec_256, UpscaleConfig::default());
        assert_eq!(quick.upscale_tex_land_ec_512, UpscaleConfig::default());
        assert!(quick.gumps_cc_bc1 && quick.gumps_ec_bc7 && quick.effects_ec_bc7);
        assert!(quick.upscale_profile_path.is_none());
        assert!(!quick.include_verdata);
        assert_eq!(quick.measure_strides_mobile_anim_cc, StrideMeasurement::Off);
        assert_eq!(quick.input_uddp_dir, quick.output_uddp_dir);
        // The devices are the user's, as in the manual jobs.
        assert_eq!(quick.bc_encode_device, ComputeDevice::Cpu);
        assert_eq!(quick.bc7_rdo_device, ComputeDevice::Gpu);
    }

    #[test]
    fn quick_modes_partition_the_full_plan_and_order_dependencies() {
        use PackageJob::*;
        use QuickConvertMode::*;

        let mut quick = AppSettings { ec_dir: Some(PathBuf::from("ec")), ..AppSettings::default() };
        assert_eq!(quick_jobs(&quick, EnhancedTextures), [ArtEc, LandEc, GumpsEc, EffectsEc, PaperdollEc, MobileEc]);
        assert_eq!(quick_jobs(&quick, Metadata), [Hues]);
        assert!(quick_jobs(&quick, ClassicTextures).is_empty());
        assert!(quick_jobs(&quick, Everything).is_empty());

        quick.cc_dir = Some(PathBuf::from("cc"));
        assert_eq!(quick_jobs(&quick, ClassicTextures), [WorldLights, ArtCc, LandCc, GumpsCc, MobileCc]);
        assert_eq!(quick_jobs(&quick, EnhancedTextures), [ArtEc, LandEc, AuxEc, GumpsEc, EffectsEc, PaperdollEc, MobileEc]);
        assert_eq!(quick_jobs(&quick, Metadata), [TileMeta, Hues]);
        let both = quick_jobs(&quick, Everything);
        let separate: Vec<_> = [ClassicTextures, EnhancedTextures, Metadata].into_iter()
            .flat_map(|mode| quick_jobs(&quick, mode))
            .collect();
        assert_eq!(both.len(), separate.len());
        assert!(separate.iter().all(|job| both.iter().filter(|candidate| *candidate == job).count() == 1));
        assert!(both.iter().position(|&job| job == GumpsCc).unwrap() < both.iter().position(|&job| job == GumpsEc).unwrap());
        assert!(both.iter().position(|&job| job == TileMeta).unwrap() < both.iter().position(|&job| job == AuxEc).unwrap());
    }
}
