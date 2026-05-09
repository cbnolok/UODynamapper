use std::env;
use std::path::PathBuf;

use color_eyre::eyre::{self, ContextCompat, WrapErr};
use image::RgbaImage;
use uddconv::bc7::{decode_bc7_to_rgba8888, ImageExtent};
use uddconv::cc_art::PagePixelFormat;
use uddconv::ec_land::EcLandPackage;

fn decode_page_rgba(package: &EcLandPackage, page_index: u32) -> eyre::Result<Vec<u8>> {
    let page = package
        .pages()
        .get(page_index as usize)
        .ok_or_else(|| eyre::eyre!("missing ec_land page metadata for {page_index}"))?;
    let page_bytes = package.read_page_bytes(page_index)?;
    let used_rgba = match page.pixel_format {
        PagePixelFormat::Rgba8888 => page_bytes,
        PagePixelFormat::Bc7 => decode_bc7_to_rgba8888(
            &page_bytes,
            ImageExtent::new(page.used_width, page.used_height)
                .map_err(|error| eyre::eyre!("invalid extent: {error}"))?,
        )
        .map_err(|error| eyre::eyre!("decode BC7 page {page_index}: {error}"))?,
    };

    let atlas_width = package.atlas_width();
    let atlas_height = package.atlas_height();
    let mut atlas_rgba = vec![0u8; (atlas_width * atlas_height * 4) as usize];
    let row_bytes = page.used_width as usize * 4;
    for row in 0..page.used_height as usize {
        let src_start = row * row_bytes;
        let src_end = src_start + row_bytes;
        let dst_start = row * atlas_width as usize * 4;
        let dst_end = dst_start + row_bytes;
        atlas_rgba[dst_start..dst_end].copy_from_slice(&used_rgba[src_start..src_end]);
    }

    Ok(atlas_rgba)
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut args = env::args_os().skip(1);
    let package_path = PathBuf::from(
        args.next().ok_or_else(|| {
            eyre::eyre!(
                "usage: export_ec_land_package_tile <ec_land.uddp> <art_id> <output.png>"
            )
        })?,
    );
    let art_id: u32 = args
        .next()
        .ok_or_else(|| eyre::eyre!("missing art_id"))?
        .to_string_lossy()
        .parse()
        .wrap_err("parse art_id")?;
    let output = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing output path"))?,
    );

    let package = EcLandPackage::load(&package_path)
        .wrap_err_with(|| format!("load {}", package_path.display()))?;
    let slot = package
        .present_slot(art_id)
        .copied()
        .with_context(|| format!("art_id {art_id} is not a present ec_land slot"))?;

    let page_rgba = decode_page_rgba(&package, slot.page_index)?;
    let atlas = RgbaImage::from_raw(package.atlas_width(), package.atlas_height(), page_rgba)
        .ok_or_else(|| eyre::eyre!("failed to create page image"))?;
    let tile = image::imageops::crop_imm(
        &atlas,
        slot.x as u32,
        slot.y as u32,
        slot.width as u32,
        slot.height as u32,
    )
    .to_image();
    tile.save(&output)
        .wrap_err_with(|| format!("save {}", output.display()))?;

    println!(
        "exported ec_land art_id {} from page {} pos=({}, {}) size={}x{} -> {}",
        art_id,
        slot.page_index,
        slot.x,
        slot.y,
        slot.width,
        slot.height,
        output.display()
    );

    Ok(())
}
