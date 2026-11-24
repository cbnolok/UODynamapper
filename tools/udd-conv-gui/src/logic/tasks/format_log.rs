use super::*;

pub(super) fn start_file(label: std::fmt::Arguments<'_>, output: &Path) {
    let name = output.file_name().unwrap_or(output.as_os_str()).to_string_lossy();
    log::info!(target: crate::logic::panel_logger::PACKAGE_TASK_TARGET, "{label}{name}");
}

pub(super) fn written_package(output: &Path) {
    let name = output.file_name().unwrap_or(output.as_os_str()).to_string_lossy();
    match udd_conv_cli::package_info::packed_compression_summary(output) {
        Ok(summary) => log::info!("{name} storage by entry: {summary}"),
        Err(error) => log::warn!("Could not inspect {name} storage: {error:#}"),
    }
}

fn upscaling(settings: &AppSettings, configs: &[(&str, UpscaleConfig)], accepts_profile: bool) -> String {
    let mut details = Vec::new();
    for (group, config) in configs {
        if !upscale_filter_active(config.filter) {
            continue;
        }
        let scale = if config.target_size == 0 {
            format!("{}×", config.filter.scale_factor())
        } else {
            format!("target {} px", config.target_size)
        };
        details.push(format!("{group}{:?} ({scale})", config.filter));
    }
    if accepts_profile && let Some(path) = &settings.upscale_profile_path {
        details.push(format!("profile {}", path.display()));
    }
    if details.is_empty() { "off".to_owned() } else { details.join(", ") }
}

fn config(filter: UpscaleFilter) -> UpscaleConfig {
    UpscaleConfig { target_size: 0, filter }
}

pub(super) fn asset_settings(settings: &AppSettings, task: AssetPackTask) {
    use AssetPackTask::*;
    use TextureOptimization::*;

    let (optimization, extra_pixels, rdo, configs, accepts_profile) = match task {
        TexArtCc => (settings.opt_tex_art_cc, "", true, vec![("", config(settings.upscale_tex_art_cc))], true),
        TexArtEc => (settings.opt_tex_art_ec, "", true, vec![("", config(settings.upscale_tex_art_ec))], true),
        TexLandCc => (None, "", false, vec![
            ("64 px: ", settings.upscale_tex_land_cc_64), ("128 px: ", settings.upscale_tex_land_cc_128),
        ], true),
        TexLandEc => (settings.opt_tex_land_ec, "", false, vec![
            ("64 px: ", settings.upscale_tex_land_ec_64), ("128 px: ", settings.upscale_tex_land_ec_128),
            ("256 px: ", settings.upscale_tex_land_ec_256), ("512 px: ", settings.upscale_tex_land_ec_512),
        ], true),
        TexAuxEc => (settings.opt_tex_aux_ec, " with raw RGBA8 fallback textures", true,
            vec![("", config(settings.upscale_tex_aux_ec))], false),
        MobileAnimCc => (settings.opt_mobile_anim_cc, "", true, vec![("", config(settings.upscale_mobile_anim_cc))], true),
        MobileAnimEc => (settings.opt_mobile_anim_ec, "", true, vec![("", config(settings.upscale_mobile_anim_ec))], true),
        GumpsCc | GumpsEc => {
            let (format, paperdoll, single) = match task {
                GumpsCc => (single_gump_pixel_format(settings.gumps_cc_bc1, CC_LOSSY_SINGLE_GUMP_FORMAT),
                    settings.upscale_gumps_cc_paperdoll, settings.upscale_gumps_cc_single),
                _ => (single_gump_pixel_format(settings.gumps_ec_bc7, EC_LOSSY_SINGLE_GUMP_FORMAT),
                    settings.upscale_gumps_ec_paperdoll, settings.upscale_gumps_ec_single),
            };
            let upscale = upscaling(settings, &[("paperdoll: ", config(paperdoll)), ("single: ", config(single))], true);
            let classic_space = if task == GumpsEc && format == GumpPixelFormat::Bc7 {
                " (BC1 for Classic-colour-space singles)"
            } else { "" };
            log::info!("Format: single gumps {}{classic_space}, paperdoll atlas raw RGBA8. RDO: off. Upscaling: {upscale}.", format.label());
            return;
        }
        WorldLights => {
            log::info!("Format: raw signed 8-bit light masks. Upscaling: off.");
            return;
        }
        EffectsEc => {
            let pixels = if settings.effects_ec_bc7 { "BC7 where supported, otherwise raw RGBA8" } else { "raw RGBA8" };
            log::info!("Format: {pixels} for newly encoded effect textures, source BC blocks preserved, binary particle definitions. RDO: off. Upscaling: off.");
            return;
        }
        PaperdollEc => {
            log::info!("Format: raw RGBA8 paperdoll frames and hue masks. Upscaling: off.");
            return;
        }
        TileMeta => {
            log::info!("Format: binary tile metadata and source records.");
            return;
        }
        Hues => {
            log::info!("Format: binary hue palettes.");
            return;
        }
        QuickConvert => return,
    };
    let pixels = match optimization {
        None => "raw RGBA8",
        Bc7 | Bc7Zstd => "BC7",
    };
    let lambda = if rdo { bc7_rdo_lambda(settings, optimization) } else { NO_BC7_RDO_LAMBDA };
    let rdo = if lambda > NO_BC7_RDO_LAMBDA {
        format!("on (λ {lambda}, {} block lookback, {})", bc7_rdo_lookback_blocks(settings), settings.bc7_rdo_device.label())
    } else {
        "off".to_owned()
    };
    let upscale = upscaling(settings, &configs, accepts_profile);
    log::info!("Format: {pixels}{extra_pixels}. RDO: {rdo}. Upscaling: {upscale}.");
}

pub(super) fn world_settings(prefix: &str) {
    log::info!("Format: binary {prefix} chunks.");
}

pub(super) fn radar_settings(settings: &AppSettings) {
    let format = match settings.radar_format {
        RadarFormat::Rgba8 => "DDS, raw RGBA8, uncompressed".to_owned(),
        RadarFormat::Bc7 => "DDS, BC7, no container compression".to_owned(),
        RadarFormat::Bc7Ktx2 => format!("KTX2, BC7, Zstd level {}", settings.radar_zstd),
    };
    log::info!("Format: {format}. RDO: off. Upscaling: off.");
}
