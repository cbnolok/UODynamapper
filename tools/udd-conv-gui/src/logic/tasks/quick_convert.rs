use std::fs;
use udd_conv::classic_sources::resolve_classic_map_source;
use udd_conv::animation::mobile_strides::keep_staged_strides;
use uocf::classic::facet_sources::CLASSIC_FACET_NAMES;
use uocf::utils::output_file::write_through_temp;

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

fn quick_settings(configured: &AppSettings, classic: bool, enhanced: bool) -> AppSettings {
    AppSettings {
        // Enhanced's complete tile metadata and KR auxiliary textures also need the Classic source.
        cc_dir: configured.cc_dir.as_ref().filter(|path| path.is_dir()).cloned(),
        ec_dir: configured.ec_dir.as_ref().filter(|path| enhanced && path.is_dir()).cloned(),
        kr_dir: configured.kr_dir.as_ref().filter(|_| enhanced).cloned(),
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
        gumps_ec_sharing: if classic { GumpsEcSharing::ShareWithGumpsCc } else { GumpsEcSharing::Standalone },
        effects_ec_bc7: true,
        ..AppSettings::default()
    }
}

fn quick_jobs(settings: &AppSettings, classic: bool, enhanced: bool) -> Vec<PackageJob> {
    use PackageJob::*;
    let mut jobs = Vec::new();
    if (classic && settings.cc_dir.is_none()) || (enhanced && settings.ec_dir.is_none()) || !(classic || enhanced) {
        return jobs;
    }
    if settings.cc_dir.is_some() {
        jobs.push(TileMeta);
    }
    jobs.push(Hues);
    if classic {
        jobs.extend([WorldLights, ArtCc, LandCc, GumpsCc]);
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
    pub fn quick_convert_outputs(&self, classic: bool, enhanced: bool) -> Vec<PathBuf> {
        let settings = quick_settings(&self.settings, classic, enhanced);
        quick_jobs(&settings, classic, enhanced).into_iter()
            .map(|job| settings.output_uddp_dir.join(job.file_name()))
            .collect()
    }

    pub fn quick_convert(&self, classic: bool, enhanced: bool) {
        let settings = quick_settings(&self.settings, classic, enhanced);
        let jobs = quick_jobs(&settings, classic, enhanced);
        if jobs.is_empty() {
            self.push_log("Select at least one client with a source folder before converting.", LogLevel::Error);
            return;
        }
        if settings.output_uddp_dir.as_os_str().is_empty() {
            self.push_log("Choose an output folder before converting.", LogLevel::Error);
            return;
        }
        self.spawn_asset_task(AssetPackTask::QuickConvert, "Quick Convert".to_string(), Vec::new(), move |progress| {
            fs::create_dir_all(&settings.output_uddp_dir)?;
            if enhanced && settings.cc_dir.is_none() {
                log::warn!("Skipping tilemeta.uddp and tex_aux_ec.uddp: these need a Classic Client source folder.");
            }
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
                log::info!("Quick Convert {}/{total}: {name}", index + 1);
                progress.set(AssetPackProgressState::Running, index as f32 / total as f32, format!("{}/{total}: {name}", index + 1));
                package_progress.start();
                job.write(&settings, &output, &package_progress)
                    .map_err(|error| eyre::eyre!("pack {name} after writing {index} of {total} packages: {error:#}"))?;
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
        let quick = quick_settings(&manual, true, true);
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
    fn enhanced_plan_includes_shared_dependencies_only_when_classic_is_available() {
        use PackageJob::*;
        let mut quick = AppSettings { ec_dir: Some(PathBuf::from("ec")), ..AppSettings::default() };
        assert_eq!(quick_jobs(&quick, false, true), [Hues, ArtEc, LandEc, GumpsEc, EffectsEc, PaperdollEc, MobileEc]);
        quick.cc_dir = Some(PathBuf::from("cc"));
        assert_eq!(quick_jobs(&quick, false, true), [TileMeta, Hues, ArtEc, LandEc, AuxEc, GumpsEc, EffectsEc, PaperdollEc, MobileEc]);
        assert!(!quick_jobs(&quick, true, false).contains(&ArtEc));
        let both = quick_jobs(&quick, true, true);
        assert!(both.iter().position(|&job| job == GumpsCc).unwrap() < both.iter().position(|&job| job == GumpsEc).unwrap());
    }
}
