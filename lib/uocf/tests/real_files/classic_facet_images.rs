//! `facet00.mul`-`facet05.mul` (the Classic world map images) of real clients read strictly, written back byte for
//! byte, exported as PNG and imported back to the same bytes; an edited image reads back and undoing it restores
//! the file.
//!
//! The slope rule renders Tokuno's map close to the shipped image, and a resized map gets an image of its own size
//! that the reader accepts.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. Run with `--ignored`.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{ImageFormat, Rgba, RgbaImage};
use uocf::classic::facet_image::{FACET_IMAGE_COUNT, FacetImageFile, UNDRAWN_BIT, facet_image_file_name};
use uocf::classic::facet_image_render::FacetImageRenderer;
use uocf::classic::facet_sources::{ClassicDifOptions, ClassicFacetSources, ClassicMapFormat};
use uocf::classic::map::{MapBlock, MapSizeCells};
use uocf::classic::world::{ClassicWorld, LandFill};
use uocf::map_edition::MapEdition;
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// The side of the square the edit paints.
const EDIT_SIDE: u32 = 16;
/// The colour the edit paints (RGB555 1, 2, 3), as RGBA.
const EDIT_RGBA: Rgba<u8> = Rgba([8, 16, 24, u8::MAX]);
const EDIT_COLOUR: u16 = 1 << 10 | 2 << 5 | 3;
/// Tokuno, the smallest facet with an image.
const RENDERED_FACET: u32 = 4;
/// The least share of Tokuno's pixels the rule gives exactly (2024: 94.4%, 7.0.20 and fp alike or better).
const MIN_EXACT_SHARE: f64 = 0.94;
const MAP_FORMATS: [ClassicMapFormat; 2] = [ClassicMapFormat::Uop, ClassicMapFormat::Mul];

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

#[test]
#[ignore = "needs real clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn the_slope_rule_renders_close_to_the_shipped_image_and_a_resized_map_gets_its_size() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        let name = facet_image_file_name(RENDERED_FACET);
        let Some(path) = find_file_ignoring_case(&root, &name) else { continue };
        let shipped = FacetImageFile::load_for_facet(&path, RENDERED_FACET).unwrap();
        let sources =
            ClassicFacetSources::resolve(&[root.clone()], RENDERED_FACET, &MAP_FORMATS, ClassicDifOptions::NONE, MapEdition::Standard)
                .unwrap();
        let size = MapSizeCells { width: u32::from(shipped.width()), height: u32::from(shipped.height()) };
        let (world, _, _) = ClassicWorld::load(&sources, Some(size)).unwrap();
        let renderer = FacetImageRenderer::load(&root).unwrap();
        let rendered = renderer.render(&world).unwrap();
        let (want, got) = (shipped.pixels(), rendered.pixels());
        let exact = want.iter().zip(&got).filter(|(want, got)| want == got).count() as f64 / want.len() as f64;
        println!("{client}: {name} rendered by the slope rule: {:.2}% of pixels exact", exact * 100.0);
        assert!(exact >= MIN_EXACT_SHARE, "{client}: {exact}");

        let grown = MapSizeCells { width: size.width + MapBlock::CELLS_PER_ROW, height: size.height };
        let (resized, _) = world.resized(grown, LandFill::VOID, false).unwrap();
        let image = FacetImageFile::from_bytes(&renderer.render(&resized).unwrap().to_bytes()).unwrap();
        image.check_size(grown.width, grown.height).unwrap();
        assert!(image.check_facet_size(RENDERED_FACET).is_err(), "the stock size no longer fits");
        checked += 1;
    }
    assert!(checked > 0, "no client ships {}", facet_image_file_name(RENDERED_FACET));
}
