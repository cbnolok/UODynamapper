//! `facet00.mul`-`facet05.mul` (the Classic world map images) of real clients read strictly, written back byte for
//! byte, exported as PNG and imported back to the same bytes; an edited image reads back and undoing it restores
//! the file.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. Run with `--ignored`.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{ImageFormat, Rgba, RgbaImage};
use uocf::classic::facet_image::{FACET_IMAGE_COUNT, FacetImageFile, UNDRAWN_BIT, facet_image_file_name};
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// The side of the square the edit paints.
const EDIT_SIDE: u32 = 16;
/// The colour the edit paints (RGB555 1, 2, 3), as RGBA.
const EDIT_RGBA: Rgba<u8> = Rgba([8, 16, 24, u8::MAX]);
const EDIT_COLOUR: u16 = 1 << 10 | 2 << 5 | 3;

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn png_round_trip(image: &RgbaImage) -> RgbaImage {
    let mut png = Vec::new();
    image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
    image::load_from_memory_with_format(&png, ImageFormat::Png).unwrap().into_rgba8()
}

#[test]
#[ignore = "needs real clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn facet_images_write_back_byte_for_byte_and_through_png() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        for facet in 0..FACET_IMAGE_COUNT {
            let name = facet_image_file_name(facet);
            let Some(path) = find_file_ignoring_case(&root, &name) else { continue };
            let bytes = std::fs::read(&path).unwrap();
            let mut file = FacetImageFile::load_for_facet(&path, facet).unwrap_or_else(|error| panic!("{client} {name}: {error:#}"));
            assert_eq!(file.to_bytes(), bytes, "{client} {name}: written back byte for byte");
            let undrawn = file.pixels().iter().filter(|&&colour| colour & UNDRAWN_BIT != 0).count();

            let image = png_round_trip(&file.to_rgba());
            assert_eq!(file.replace_image(&image).unwrap(), 0, "{client} {name}: the PNG imports unchanged");
            assert_eq!(file.to_bytes(), bytes, "{client} {name}: the PNG imports to the same bytes");

            let mut edited = image.clone();
            let (x0, y0) = (image.width() / 2, image.height() / 2);
            for y in y0..y0 + EDIT_SIDE {
                for x in x0..x0 + EDIT_SIDE {
                    edited.put_pixel(x, y, EDIT_RGBA);
                }
            }
            assert_eq!(file.replace_image(&png_round_trip(&edited)).unwrap(), EDIT_SIDE as usize, "{client} {name}");
            let back = FacetImageFile::from_bytes(&file.to_bytes()).unwrap();
            let width = usize::from(back.width());
            let pixels = back.pixels();
            assert_eq!(pixels[y0 as usize * width + x0 as usize], EDIT_COLOUR, "{client} {name}: the edit reads back");
            assert_eq!(back.to_rgba(), edited, "{client} {name}: only the edited pixels changed");
            file.replace_image(&image).unwrap();
            assert_eq!(file.to_bytes(), bytes, "{client} {name}: undoing the edit restores the file");
            println!(
                "{client}: {name} ({} x {}, {} runs, {undrawn} pixels with bit 15) written back byte for byte",
                back.width(),
                back.height(),
                file.rows().iter().map(Vec::len).sum::<usize>()
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no client ships a facet image");
}
