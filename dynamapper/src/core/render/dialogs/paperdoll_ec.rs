//! The Enhanced client's 3D paperdoll (`ECPaperdollTexture::Build`, UOSA 0x0050b340), which the Enhanced and KR-like
//! presets draw from paperdoll_ec.uddp: the body's frame, then the frames of the worn items in the paperdoll draw order
//! of `WearableRules.kdl`, blended by alpha, each hued as the client hues it. The lookup is `uocf::enhanced::paperdoll`,
//! which the uocf editor runs on the client files; the item definitions are the `tileart.uop` records of tilemeta.uddp.
//! What a robe covers is then taken off: the decisions paperdoll_ec.uddp stores, under the user's
//! `PaperdollOverrides.kdl` ([`super::paperdoll_overrides`]).
//!
//! Every paperdoll is drawn on the same canvas ([`PaperdollCanvas`]): the client's paperdoll window around the frames'
//! anchor, widened to the frame box of both sets, so its size does not change with what is worn and no frame (the lit
//! lantern's halo is the widest) is cut.

use bevy::prelude::Image;
use color_eyre::eyre;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use udd_assets::paperdoll_ec::PaperdollEcSet;
use udd_assets::tilemeta::TileMetaPackage;
use udd_assets::PaperdollEcPackage;
use uocf::enhanced::paperdoll::{compose_on, PlacedFrame, WornItem, SLOT_NAMES};

use super::paperdoll_overrides::{plan_taking_off, PaperdollOrder};

/// What the Enhanced and KR-like presets draw for a paperdoll. Kept in the user settings as
/// `window.enhanced_paperdoll`; the Classic preset always draws the gump paperdoll.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnhancedPaperdoll {
    /// The client's 3D paperdoll, from paperdoll_ec.uddp. The derived `Default` exists only for the `Default` of
    /// `SectWindow`; the value always comes from the settings files.
    #[default]
    Frames,
    /// The old-style paperdoll of gumps, from gumps_ec.uddp.
    Gumps,
}

/// `PaperdollWindow.xml`: the client's paperdoll window is 440 x 732.
const CLIENT_WINDOW_WIDTH: i32 = 440;
const CLIENT_WINDOW_HEIGHT: i32 = 732;
/// `PaperdollWindow.lua` anchors the paperdoll texture so that the frames' anchor, at the feet, sits this far below the
/// window's centre (`yOffset + 30`). Its horizontal offsets by race and gender (+5, -5, 0, -10) are left out: the
/// canvas is wider than the window, centred on the anchor.
const CLIENT_ANCHOR_BELOW_CENTRE: i32 = 30;
/// `PaperdollWindow.Initialize` scales the window by `customUiScale * 0.70` (`customUiScale` is 1 by default), and the
/// texture is drawn filtered: the canvas is shown at this scale, sampled linearly.
pub const CLIENT_WINDOW_SCALE: f32 = 0.70;

/// The canvas every Enhanced paperdoll is drawn on: its size and where the frames' anchor is on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaperdollCanvas {
    pub width: u32,
    pub height: u32,
    pub anchor_x: i32,
    pub anchor_y: i32,
}

impl PaperdollCanvas {
    /// The client window's rect around the anchor, widened to the frame box of each set's header, which holds every
    /// frame of the set. EC 2024: both boxes are (-256, -306)..(256, 206), so the canvas is 512 x 732 with the anchor at
    /// (256, 396).
    pub fn for_sets(sets: &[PaperdollEcSet]) -> Self {
        let mut min_x = -CLIENT_WINDOW_WIDTH / 2;
        let mut max_x = min_x + CLIENT_WINDOW_WIDTH;
        let mut min_y = -(CLIENT_WINDOW_HEIGHT / 2 + CLIENT_ANCHOR_BELOW_CENTRE);
        let mut max_y = min_y + CLIENT_WINDOW_HEIGHT;
        for set in sets {
            min_x = min_x.min(i32::from(set.bounds.x0));
            min_y = min_y.min(i32::from(set.bounds.y0));
            max_x = max_x.max(i32::from(set.bounds.x1));
            max_y = max_y.max(i32::from(set.bounds.y1));
        }
        Self {
            width: (max_x - min_x) as u32,
            height: (max_y - min_y) as u32,
            anchor_x: -min_x,
            anchor_y: -min_y,
        }
    }

    /// `frames` drawn in order, premultiplied as bevy_egui blends it: `compose_on` blends each over a transparent
    /// canvas. A texel off the canvas is not drawn.
    pub fn compose(&self, frames: &[PlacedFrame]) -> Vec<u8> {
        compose_on(self.width, self.height, self.anchor_x, self.anchor_y, frames)
    }

    /// The window shows the canvas at the client window's scale, in points.
    pub fn display_size(&self) -> (f32, f32) {
        (self.width as f32 * CLIENT_WINDOW_SCALE, self.height as f32 * CLIENT_WINDOW_SCALE)
    }

    /// The canvas as an image sampled linearly, as the client filters its paperdoll texture.
    pub fn image(&self, rgba: &[u8]) -> Image {
        crate::util_lib::image::image_from_rgba8(self.width, self.height, rgba)
    }
}

/// An item put on the paperdoll and its hue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaperdollItem {
    pub item_id: u32,
    pub hue: u16,
}

/// What the client draws for `body` wearing `items` (`uocf::enhanced::paperdoll::plan` with `order`'s rules), without
/// what a robe covers ([`plan_taking_off`], with the decisions the package stores for the body frame drawn), decoded
/// from paperdoll_ec.uddp: the frames, bottom first, hued with the package's hue tables. A body without a frame takes
/// the male 400, as in the client. A worn item without a frame draws nothing, as in the client, and is logged once per
/// item id: `logged_misses` holds the ids logged so far.
pub fn paperdoll_frames(
    package: &PaperdollEcPackage,
    tilemeta: &TileMetaPackage,
    order: &PaperdollOrder,
    body: u32,
    body_hue: u16,
    items: &[PaperdollItem],
    logged_misses: &mut HashSet<u32>,
) -> eyre::Result<Vec<PlacedFrame>> {
    let worn: Vec<WornItem> = items
        .iter()
        .map(|item| WornItem {
            item_id: item.item_id,
            hue: item.hue,
            // A tiledata layer of 0 is no layer: the item is not worn.
            tiledata_layer: tilemeta.item_tile(item.item_id).map(|tile| tile.quality).filter(|&layer| layer != 0),
        })
        .collect();
    let decisions = package.dress_decisions();
    let stored = |body_frame: u32, covering: u32, (layer, garment): (usize, u32)| {
        decisions.takes_off(covering, body_frame, layer, garment)
    };
    let (plan, _) = plan_taking_off(package, tilemeta.item_definitions(), order, body, body_hue, &worn, stored)?;
    for miss in &plan.misses {
        if logged_misses.insert(miss.item_id) {
            log::warn!(
                "Enhanced paperdoll: item {} on {} draws nothing: {}.",
                miss.item_id,
                SLOT_NAMES[miss.layer],
                miss.miss.describe()
            );
        }
    }
    for note in &plan.notes {
        log::debug!("Enhanced paperdoll of body {body}: {note}");
    }
    plan.frames.iter().map(|planned| package.decode(planned.frame, planned.hue)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::animationframe::AnimationBounds;
    use uocf::enhanced::hues::ShadowPaletteStats;

    /// The box of both `Paperdoll.uop` set headers of EC 2024.
    const SHIPPED_SET_BOUNDS: AnimationBounds = AnimationBounds { x0: -256, y0: -306, x1: 256, y1: 206 };
    const SHIPPED_CANVAS: PaperdollCanvas = PaperdollCanvas { width: 512, height: 732, anchor_x: 256, anchor_y: 396 };

    fn set(bounds: AnimationBounds) -> PaperdollEcSet {
        PaperdollEcSet { bounds, shadow: ShadowPaletteStats::default() }
    }

    /// The shipped frame box is wider than the client window and shorter: the canvas takes the box's width and the
    /// window's height, with the anchor 30 below the window's centre.
    #[test]
    fn the_canvas_holds_the_client_window_and_both_frame_boxes() {
        let canvas = PaperdollCanvas::for_sets(&[set(SHIPPED_SET_BOUNDS), set(SHIPPED_SET_BOUNDS)]);
        assert_eq!(canvas, SHIPPED_CANVAS);
        assert_eq!(canvas.anchor_y, CLIENT_WINDOW_HEIGHT / 2 + CLIENT_ANCHOR_BELOW_CENTRE);
        // A box past the window on every side widens the canvas to it.
        const LOW_RIGHT: AnimationBounds = AnimationBounds { x0: -10, y0: -10, x1: 300, y1: 400 };
        let wider = PaperdollCanvas::for_sets(&[set(SHIPPED_SET_BOUNDS), set(LOW_RIGHT)]);
        assert_eq!((wider.anchor_x, wider.anchor_y), (SHIPPED_CANVAS.anchor_x, SHIPPED_CANVAS.anchor_y));
        assert_eq!((wider.width, wider.height), (556, 796));
        let (width, height) = canvas.display_size();
        assert_eq!((width.round(), height.round()), (358.0, 512.0));
    }

    /// A frame goes at its offset from the anchor; the lantern halo's box, the widest frame, fits whole.
    #[test]
    fn frames_go_at_their_offset_from_the_anchor() {
        const RED: [u8; 4] = [200, 0, 0, 255];
        const HALO_BOX: (i32, i32, u32, u32) = (-119, -145, 375, 306);
        let canvas = SHIPPED_CANVAS;
        let (x, y, width, height) = HALO_BOX;
        let halo = PlacedFrame { x, y, width, height, rgba: RED.repeat((width * height) as usize) };
        let rgba = canvas.compose(&[halo]);
        let texel = |x: i32, y: i32| {
            let index = ((y + canvas.anchor_y) as usize * canvas.width as usize + (x + canvas.anchor_x) as usize) * 4;
            <[u8; 4]>::try_from(&rgba[index..index + 4]).unwrap()
        };
        assert_eq!(texel(x, y), RED);
        assert_eq!(texel(x + width as i32 - 1, y + height as i32 - 1), RED);
        assert_eq!(texel(x - 1, y), [0; 4]);
        let drawn = rgba.chunks_exact(4).filter(|texel| *texel == RED).count();
        assert_eq!(drawn, (width * height) as usize, "the whole halo is on the canvas");
    }

    /// A body and what it wears, each item with its hue.
    struct Outfit {
        name: &'static str,
        body: u32,
        items: &'static [(u32, u16)],
    }

    const SKIN_HUE: u16 = 1002;
    /// The outfits drawn from the packages and from the client files: a shirt, a robe and the lit lantern, whose halo
    /// is the widest frame; plate, a dyed cloak, hair and a katana; elven plate; gargish leather.
    const OUTFITS: [Outfit; 4] = [
        Outfit { name: "human_male_robe_lantern", body: 400, items: &[(0x1517, 3), (0x1F03, 0), (0x0A15, 0)] },
        Outfit {
            name: "human_female_plate_cloak",
            body: 401,
            items: &[(0x1415, 0), (0x1515, 38), (0x203C, 0), (0x13FF, 0)],
        },
        Outfit {
            name: "elf_female_plate",
            body: 606,
            items: &[(0x2B67, 0), (0x2B6B, 0), (0x2B6C, 0), (0x2B6A, 0), (0x2B69, 0), (0x2FCD, 0), (0x2D20, 0)],
        },
        Outfit {
            name: "gargoyle_male_leather",
            body: 666,
            items: &[(0x4258, 0), (0x0303, 0), (0x0301, 0), (0x0305, 0), (0x0310, 0), (0x0908, 0), (0x4210, 0)],
        },
    ];

    /// The client file `name` in `dir`, whatever its case.
    fn client_file(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().eq_ignore_ascii_case(name))
            .unwrap_or_else(|| panic!("no {name} in {}", dir.display()))
            .path()
    }

    fn env_path(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")))
    }

    /// With the client's rules alone (an override that keeps everything beneath any robe), the Enhanced presets draw
    /// what the uocf editor composes from the client files (`build` and `compose` on `Paperdoll.uop`, `tileart.uop`
    /// and `hues.uop`): the same frames at the same places, and the editor's composite lands on the canvas at the
    /// anchor texel for texel, nothing cut. Both sides take the item's tiledata layer from tilemeta.uddp. With the
    /// decisions the package stores, the robe takes off the shirt beneath it. Writes each canvas as a PNG
    /// (premultiplied, as egui gets it) and a summary to the temp dir.
    #[test]
    #[ignore = "needs packages and a real Enhanced Client: set UDD_PAPERDOLL_EC, UDD_TILEMETA, UO_EC_CLIENT_DIR and \
                TMPDIR, and run with --ignored"]
    fn the_packed_paperdoll_draws_what_the_editor_composes() {
        use uocf::enhanced::hues::HUES_UOP_NAME;
        use uocf::enhanced::paperdoll::{build, compose, hue_tables, FrameSets, NamedTileArt, PAPERDOLL_UOP_NAME};
        use uocf::enhanced::tileart::TILEART_UOP_NAME;
        use uocf::uop_container::package::{LoadMode, UopPackage};
        const RGBA_BYTES: usize = 4;

        let package = PaperdollEcPackage::load(env_path("UDD_PAPERDOLL_EC")).unwrap();
        let tilemeta = TileMetaPackage::load(env_path("UDD_TILEMETA")).unwrap();
        // tilemeta carries the default record an item without a record takes.
        assert!(
            uocf::enhanced::paperdoll::PaperdollDefinitions::get(
                tilemeta.item_definitions(),
                uocf::enhanced::paperdoll::DEFAULT_DEFINITION_ID
            )
            .is_some(),
            "tilemeta.uddp has no record {}",
            uocf::enhanced::paperdoll::DEFAULT_DEFINITION_ID
        );
        let client = env_path("UO_EC_CLIENT_DIR");
        let named = NamedTileArt::read(&UopPackage::load(client_file(&client, TILEART_UOP_NAME)).unwrap());
        let sets = FrameSets::load(&client_file(&client, PAPERDOLL_UOP_NAME)).unwrap();
        let hues_package = UopPackage::load_with_mode(client_file(&client, HUES_UOP_NAME), LoadMode::Lazy).unwrap();
        use super::super::paperdoll_overrides::{PaperdollOverrides, PAPERDOLL_OVERRIDES_FILE_NAME};
        use uocf::enhanced::paperdoll::WearableRules;
        /// Keeps everything beneath any robe: the client's rules.
        const CLIENT_RULES_OVERRIDES: &str = "cover \"dress\" items=\"any\"\n";
        const ROBE_OUTFIT: &str = "human_male_robe_lantern";
        const SHIRT: u32 = 0x1517;
        let rules = WearableRules::decode(include_str!("../../../../assets/runtime_specs/paperdoll/WearableRules.kdl"))
            .unwrap();
        let stored_order = PaperdollOrder { rules: rules.clone(), overrides: PaperdollOverrides::default() };
        let client_order = PaperdollOrder {
            rules: rules.clone(),
            overrides: PaperdollOverrides::decode(PAPERDOLL_OVERRIDES_FILE_NAME, CLIENT_RULES_OVERRIDES).unwrap(),
        };
        let canvas = PaperdollCanvas::for_sets(package.sets());
        let out_dir = std::env::temp_dir();
        assert!(out_dir.is_absolute(), "{}", out_dir.display());

        let mut summary = format!(
            "canvas {}x{}, anchor ({}, {}), shown at {:?}\n",
            canvas.width,
            canvas.height,
            canvas.anchor_x,
            canvas.anchor_y,
            canvas.display_size()
        );
        for outfit in &OUTFITS {
            let items: Vec<PaperdollItem> =
                outfit.items.iter().map(|&(item_id, hue)| PaperdollItem { item_id, hue }).collect();
            let mut misses = HashSet::new();
            let ours = paperdoll_frames(&package, &tilemeta, &client_order, outfit.body, SKIN_HUE, &items, &mut misses)
                .unwrap_or_else(|error| panic!("{}: {error:#}", outfit.name));
            let stored = paperdoll_frames(&package, &tilemeta, &stored_order, outfit.body, SKIN_HUE, &items, &mut misses)
                .unwrap_or_else(|error| panic!("{}: {error:#}", outfit.name));
            if outfit.name == ROBE_OUTFIT {
                assert!(outfit.items.iter().any(|&(item_id, _)| item_id == SHIRT));
                assert_eq!(stored.len() + 1, ours.len(), "the robe takes off the shirt");
            }
            let drawn_stored = canvas.compose(&stored);
            let path = out_dir.join(format!("paperdoll_ec-{}-stored.png", outfit.name));
            canvas.image(&drawn_stored).try_into_dynamic().unwrap().save(&path).unwrap();
            let worn: Vec<WornItem> = items
                .iter()
                .map(|item| WornItem {
                    item_id: item.item_id,
                    hue: item.hue,
                    tiledata_layer: tilemeta.item_tile(item.item_id).map(|tile| tile.quality).filter(|&layer| layer != 0),
                })
                .collect();
            let hues = hue_tables(&hues_package, worn.iter().map(|item| item.hue).chain([SKIN_HUE])).unwrap();
            let editor = build(&sets, &named, &rules, &hues, outfit.body, SKIN_HUE, &worn).unwrap();
            assert_eq!(ours.len(), editor.frames.len(), "{}: {:?}", outfit.name, editor.notes);

            let mut frame_texels_differing = 0;
            for (our, their) in ours.iter().zip(&editor.frames) {
                let geometry = |frame: &PlacedFrame| (frame.x, frame.y, frame.width, frame.height);
                assert_eq!(geometry(our), geometry(their), "{}", outfit.name);
                frame_texels_differing += our
                    .rgba
                    .chunks_exact(RGBA_BYTES)
                    .zip(their.rgba.chunks_exact(RGBA_BYTES))
                    .filter(|(our, their)| our != their)
                    .count();
            }

            // The editor's composite, put on the canvas at the anchor.
            let drawn = canvas.compose(&ours);
            let (width, height, composite) = compose(&editor.frames);
            let min_x = editor.frames.iter().map(|frame| frame.x).min().unwrap_or(0);
            let min_y = editor.frames.iter().map(|frame| frame.y).min().unwrap_or(0);
            let mut placed = vec![0u8; drawn.len()];
            let mut cut = 0;
            for y in 0..height as i32 {
                for x in 0..width as i32 {
                    let source = (y as usize * width as usize + x as usize) * RGBA_BYTES;
                    let texel = &composite[source..source + RGBA_BYTES];
                    let (canvas_x, canvas_y) = (x + min_x + canvas.anchor_x, y + min_y + canvas.anchor_y);
                    let inside = (0..canvas.width as i32).contains(&canvas_x) && (0..canvas.height as i32).contains(&canvas_y);
                    if !inside {
                        cut += usize::from(texel[RGBA_BYTES - 1] != 0);
                        continue;
                    }
                    let target = (canvas_y as usize * canvas.width as usize + canvas_x as usize) * RGBA_BYTES;
                    placed[target..target + RGBA_BYTES].copy_from_slice(texel);
                }
            }
            let differing: Vec<u8> = drawn
                .chunks_exact(RGBA_BYTES)
                .zip(placed.chunks_exact(RGBA_BYTES))
                .filter(|(our, their)| our != their)
                .map(|(our, their)| our.iter().zip(their).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0))
                .collect();
            let shown = drawn.chunks_exact(RGBA_BYTES).filter(|texel| texel[RGBA_BYTES - 1] != 0).count();
            let mut missed: Vec<u32> = misses.into_iter().collect();
            missed.sort_unstable();
            summary += &format!(
                "{}: body {} frames {} ({} with the stored decisions) texels drawn {} differing {} (max channel \
                 difference {}) frame texels differing {} cut {} items without a frame {:x?} notes {:?}\n",
                outfit.name,
                outfit.body,
                ours.len(),
                stored.len(),
                shown,
                differing.len(),
                differing.iter().max().copied().unwrap_or(0),
                frame_texels_differing,
                cut,
                missed,
                editor.notes
            );
            for (side, rgba) in [("dynamapper", &drawn), ("editor", &placed)] {
                let path = out_dir.join(format!("paperdoll_ec-{}-{side}.png", outfit.name));
                canvas.image(rgba).try_into_dynamic().unwrap().save(&path).unwrap();
            }
            assert_eq!((differing.len(), frame_texels_differing, cut), (0, 0, 0), "{}", outfit.name);
        }
        let path = out_dir.join("paperdoll_ec-pixels.txt");
        std::fs::write(&path, &summary).unwrap();
        log::info!("wrote {}:\n{summary}", path.display());
    }

    /// The settings name the two choices as written in user_preferences.toml.
    #[test]
    fn the_setting_reads_frames_and_gumps() {
        #[derive(Deserialize)]
        struct Window {
            enhanced_paperdoll: EnhancedPaperdoll,
        }
        let read = |text: &str| toml::from_str::<Window>(text).map(|window| window.enhanced_paperdoll);
        assert_eq!(read("enhanced_paperdoll = \"frames\"").unwrap(), EnhancedPaperdoll::Frames);
        assert_eq!(read("enhanced_paperdoll = \"gumps\"").unwrap(), EnhancedPaperdoll::Gumps);
        assert!(read("enhanced_paperdoll = \"3d\"").is_err());
    }
}
