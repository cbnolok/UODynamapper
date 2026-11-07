//! Whether the gump paperdolls show the garments beneath a robe at its edges, as the 3D paperdoll does
//! (`paperdoll_overrides`), and what the decisions measured on the 3D frames take off there: every garment of the
//! slots the 3D paperdoll takes off beneath a robe is drawn under every dress-layer item, on each bundled profile, from
//! the gumps the Classic preset draws and from those the Enhanced presets draw with the old-style paperdoll, and the
//! garment texels left showing are counted. They do: the cloth lines up, a shirt showing at most at a robe's open
//! collar, but armour is drawn wider than a robe and its dark outline shows around the shoulders and sleeves
//! (docs/agent_notes/ec_paperdoll_robe_override.md).

use super::*;
use crate::core::uo_files_loader::ClassicGumpSource;
use std::collections::HashSet;
use std::path::PathBuf;
use udd_assets::tilemeta::TileMetaPackage;
use udd_assets::GumpsPackage;
use uocf::enhanced::paperdoll::slot_layer;

/// The slots the 3D paperdoll takes off beneath a robe.
const CANDIDATE_SLOTS: [&str; 6] = ["torso", "legs", "skirt", "arms", "chest", "abovechest"];
/// How far from the dress, in logical texels across or down (the larger), a garment texel left bare counts as an edge
/// showing past it. The 3D paperdoll's band is 16 of its texels, which it shows at 0.7: about 11 points, and the gump
/// paperdoll shows a logical texel as a point. A gump drawn to line up with another slips by a texel or two.
const GUMP_EDGE_BAND_TEXELS: usize = 11;
/// Bare texels beyond the band that put a garment in plain view: the packer's 1,600 3D texels, shown at 0.7 across
/// and down.
const GUMP_PLAIN_VIEW_TEXELS: usize = 784;
/// An override that keeps everything beneath any robe: the client's rules.
const CLIENT_RULES_OVERRIDES: &str = "cover \"dress\" items=\"any\"\n";
/// The body gump of each bundled profile.
const PROFILE_BODY_GUMPS: [(&str, u32); 6] = [
    ("human_male", 12),
    ("human_female", 13),
    ("elf_male", 14),
    ("elf_female", 15),
    ("gargoyle_male", 666),
    ("gargoyle_female", 667),
];
/// A robe over a shirt and long pants, over plate, and over a chainmail tunic, drawn on the human bodies whatever is
/// measured.
const REFERENCE_ROBE: u32 = 0x1F03;
const REFERENCE_OUTFITS: [&[u32]; 3] = [&[0x1517, 0x1539], &[0x1410, 0x1415, 0x1411], &[0x13BD]];
const REFERENCE_PROFILES: [&str; 2] = ["human_male", "human_female"];
/// The cases with the most bare texels near the dress whose paperdolls are saved, per preset.
const SAVED_CASES: usize = 8;
const OPAQUE: u8 = u8::MAX;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")))
}

/// An item worn in a candidate slot or on the dress layer, as the preset finds its layer.
struct Candidate {
    item_id: u32,
    layer: usize,
}

/// Every item with a paperdoll anim on the dress layer or a candidate slot, as `client` finds its layer
/// (`gump_paperdoll_layers`), one item per layer and anim: those draw the same gump.
fn candidates(tilemeta: &TileMetaPackage, client: GumpClient) -> Vec<Candidate> {
    let mut layers: Vec<usize> = CANDIDATE_SLOTS.iter().map(|slot| slot_layer(slot).unwrap()).collect();
    layers.push(slot_layer("dress").unwrap());
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for tile in tilemeta.item_tiles() {
        if tile.anim_id == 0 {
            continue;
        }
        let tiledata_layer = Some(tile.quality).filter(|&layer| layer != 0);
        let layer = match client {
            GumpClient::Classic => tiledata_layer.map(usize::from).filter(|&layer| layer < LAYER_COUNT),
            GumpClient::Enhanced => item_layer(tilemeta.item_definitions(), tile.tile_id, tiledata_layer),
        };
        let Some(layer) = layer.filter(|layer| layers.contains(layer)) else {
            continue;
        };
        if seen.insert((layer, tile.anim_id)) {
            candidates.push(Candidate { item_id: tile.tile_id, layer });
        }
    }
    candidates
}

/// A gump drawn at (`x`, `y`) on a `width` x `height` canvas, in logical texels: for each, the most and the least alpha
/// of the physical texels it holds.
fn coverage(gump: &udd_assets::gumps::GumpImage, x: i32, y: i32, width: usize, height: usize) -> (Vec<u8>, Vec<u8>) {
    let scale = usize::from(gump.upscale_factor.max(1));
    let rgba = gump.canvas_rgba();
    let physical_width = gump.physical_width as usize;
    let mut most = vec![0u8; width * height];
    let mut least = vec![0u8; width * height];
    for gump_y in 0..gump.physical_height as usize / scale {
        for gump_x in 0..physical_width / scale {
            let (canvas_x, canvas_y) = (x + gump_x as i32, y + gump_y as i32);
            if !(0..width as i32).contains(&canvas_x) || !(0..height as i32).contains(&canvas_y) {
                continue;
            }
            let (mut high, mut low) = (0, OPAQUE);
            for physical_y in gump_y * scale..(gump_y + 1) * scale {
                for physical_x in gump_x * scale..(gump_x + 1) * scale {
                    let alpha = rgba[(physical_y * physical_width + physical_x) * RGBA_BYTES + ALPHA_BYTE];
                    high = high.max(alpha);
                    low = low.min(alpha);
                }
            }
            let index = canvas_y as usize * width + canvas_x as usize;
            most[index] = high;
            least[index] = low;
        }
    }
    (most, least)
}

/// Whether each texel is within `radius` texels (across or down, the larger) of a touched one.
fn near(touched: &[bool], width: usize, height: usize, radius: usize) -> Vec<bool> {
    let mut across = vec![false; touched.len()];
    let mut sums = vec![0usize; width.max(height) + 1];
    for y in 0..height {
        for x in 0..width {
            sums[x + 1] = sums[x] + usize::from(touched[y * width + x]);
        }
        for x in 0..width {
            across[y * width + x] = sums[(x + radius + 1).min(width)] > sums[x.saturating_sub(radius)];
        }
    }
    let mut near = vec![false; touched.len()];
    for x in 0..width {
        for y in 0..height {
            sums[y + 1] = sums[y] + usize::from(across[y * width + x]);
        }
        for y in 0..height {
            near[y * width + x] = sums[(y + radius + 1).min(height)] > sums[y.saturating_sub(radius)];
        }
    }
    near
}

/// A garment's texels left showing under a dress.
#[derive(Default, Clone, Copy)]
struct Showing {
    /// Under the dress's soft edge, where it is partly opaque.
    soft: usize,
    /// Bare: the dress does not touch them, near it and beyond the band.
    bare_near: usize,
    bare_beyond: usize,
}

/// The garment and dress pairs of a preset: what the client's rules leave showing, and what the stored decisions take
/// off.
#[derive(Default)]
struct Tally {
    measured: usize,
    hidden_by_client: usize,
    drawn_over: usize,
    nothing_shows: usize,
    edges_only: usize,
    beyond_band: usize,
    in_plain_view: usize,
    taken_nothing_showed: usize,
    taken_edges_only: usize,
    taken_beyond_band: usize,
    taken_in_plain_view: usize,
}

impl Tally {
    fn add(&mut self, showing: Showing, taken: bool) {
        self.measured += 1;
        let bare = showing.bare_near + showing.bare_beyond;
        let (all, taken_count) = if bare == 0 {
            (&mut self.nothing_shows, &mut self.taken_nothing_showed)
        } else if showing.bare_beyond == 0 {
            (&mut self.edges_only, &mut self.taken_edges_only)
        } else if showing.bare_beyond < GUMP_PLAIN_VIEW_TEXELS {
            (&mut self.beyond_band, &mut self.taken_beyond_band)
        } else {
            (&mut self.in_plain_view, &mut self.taken_in_plain_view)
        };
        *all += 1;
        *taken_count += usize::from(taken);
    }
}

/// For each preset's gumps (gumps_cc.uddp for Classic, gumps_ec.uddp then gumps_cc.uddp for the old-style Enhanced
/// paperdoll) and each bundled profile, every garment of the candidate slots drawn beneath every dress-layer item by
/// the client's rules: the garment's texels the dress leaves showing, near it and beyond, and whether the decision
/// paperdoll_ec.uddp stores for the dress takes the garment off. Writes the cases, the most bare texels near the dress
/// first, to `$TMPDIR/gump-robe-edges/cases.tsv`, a summary to `summary.txt`, and the paperdolls of the reference robe
/// (with the client's rules and with the decisions) and of the worst cases as PNGs.
#[test]
#[ignore = "needs packages: set UDD_GUMPS_CC, UDD_GUMPS_EC, UDD_PAPERDOLL_EC, UDD_TILEMETA and TMPDIR, and run with \
            --ignored"]
fn what_the_gump_paperdolls_show_beneath_a_robe() {
    let tilemeta = TileMetaPackage::load(env_path("UDD_TILEMETA")).unwrap();
    let classic = ClassicGumpSource::Package(GumpsPackage::load(env_path("UDD_GUMPS_CC")).unwrap());
    let gump_map = GumpMapRes::new(Some(classic), Some(GumpsPackage::load(env_path("UDD_GUMPS_EC")).unwrap())).unwrap();
    let package = udd_assets::PaperdollEcPackage::load(env_path("UDD_PAPERDOLL_EC")).unwrap();
    let profiles = PaperdollProfilesRes::load_from_str(
        PAPERDOLL_PROFILES_FILE_NAME,
        include_str!("../../../../../assets/runtime_specs/paperdoll/PaperdollProfiles.kdl"),
    )
    .unwrap();
    let rules =
        WearableRules::decode(include_str!("../../../../../assets/runtime_specs/paperdoll/WearableRules.kdl")).unwrap();
    let client_order = PaperdollOrder {
        rules: rules.clone(),
        overrides: PaperdollOverrides::decode(PAPERDOLL_OVERRIDES_FILE_NAME, CLIENT_RULES_OVERRIDES).unwrap(),
    };
    let stored_order = PaperdollOrder { rules, overrides: PaperdollOverrides::default() };
    let out_dir = std::env::temp_dir().join("gump-robe-edges");
    assert!(out_dir.is_absolute(), "{}", out_dir.display());
    std::fs::create_dir_all(&out_dir).unwrap();
    let dress_layer = slot_layer("dress").unwrap();
    let item_tile = |item_id: u32| tilemeta.item_tile(item_id).copied();
    let name = |item_id: u32| tilemeta.item_tile(item_id).map(|tile| tile.name_ascii().to_string()).unwrap_or_default();
    let mut scratch = Vec::new();

    let mut cases: Vec<(usize, String)> = Vec::new();
    let mut summary = format!(
        "bare garment texels within {GUMP_EDGE_BAND_TEXELS} logical texels of the dress, and beyond; in plain view at \
         {GUMP_PLAIN_VIEW_TEXELS} beyond\n"
    );
    for client in [GumpClient::Classic, GumpClient::Enhanced] {
        let preset = format!("{client:?}");
        let candidates = candidates(&tilemeta, client);
        let mut tally = Tally::default();
        let mut worst: Vec<(usize, String, u32, u32)> = Vec::new();
        for (profile_id, body_gump) in PROFILE_BODY_GUMPS {
            let profile = profiles.profile(profile_id).unwrap();
            let (width, height) = (profile.canvas_width as usize, profile.canvas_height as usize);
            let stored = stored_dress_hides(Some(&package), tilemeta.item_definitions(), profile.body);
            let request = |items: &[u32]| PaperdollRequest {
                profile: profile.clone(),
                body_gump: Some(body_gump),
                body_hue: 0,
                items: items.iter().map(|&item_id| PaperdollItem { item_id, hue: 0 }).collect(),
            };
            let layers = |items: &[u32]| {
                gump_paperdoll_layers(&request(items), client, item_tile, tilemeta.item_definitions(), &client_order, |_| {
                    None
                })
            };
            // Each candidate's gump on this profile's canvas: the garments as the texels they touch.
            let mut dresses = Vec::new();
            let mut garments = Vec::new();
            for candidate in &candidates {
                let Ok(drawn) = layers(&[candidate.item_id]) else {
                    continue;
                };
                let Some(layer) = drawn.get(1) else {
                    continue;
                };
                let Some(gump_id) = layer_gump(layer, |gump_id| gump_map.has_gump(client, gump_id)) else {
                    continue;
                };
                let gump = gump_map.decode_gump(client, gump_id, &mut scratch).unwrap();
                let skipped = client == GumpClient::Enhanced
                    && gump.logical_width == ENHANCED_SKIPPED_GUMP_SIZE
                    && gump.logical_height == ENHANCED_SKIPPED_GUMP_SIZE;
                if skipped {
                    continue;
                }
                let (most, least) = coverage(&gump, layer.x, layer.y, width, height);
                if candidate.layer == dress_layer {
                    dresses.push((candidate, most, least));
                } else {
                    let touched: Vec<u32> = (0..most.len() as u32).filter(|&index| most[index as usize] != 0).collect();
                    garments.push((candidate, touched));
                }
            }
            for (dress, most, least) in &dresses {
                let touched: Vec<bool> = most.iter().map(|&alpha| alpha != 0).collect();
                let near = near(&touched, width, height, GUMP_EDGE_BAND_TEXELS);
                let hidden = stored(dress.item_id);
                for (garment, texels) in &garments {
                    let Ok(drawn) = layers(&[garment.item_id, dress.item_id]) else {
                        continue;
                    };
                    let position = |item_id: u32| drawn.iter().position(|layer| layer.item.is_some_and(|(_, id)| id == item_id));
                    let (Some(beneath), Some(over)) = (position(garment.item_id), position(dress.item_id)) else {
                        tally.hidden_by_client += 1;
                        continue;
                    };
                    if beneath > over {
                        tally.drawn_over += 1;
                        continue;
                    }
                    let mut showing = Showing::default();
                    for &index in texels {
                        let index = index as usize;
                        match (most[index], least[index]) {
                            (_, OPAQUE) => {}
                            (0, _) if near[index] => showing.bare_near += 1,
                            (0, _) => showing.bare_beyond += 1,
                            _ => showing.soft += 1,
                        }
                    }
                    let taken = hidden.is_some_and(|hidden| hidden & 1 << garment.layer != 0);
                    tally.add(showing, taken);
                    let row = format!(
                        "{preset}\t{profile_id}\t0x{:04X}\t{}\t0x{:04X}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                        dress.item_id,
                        name(dress.item_id),
                        garment.item_id,
                        SLOT_NAMES[garment.layer],
                        name(garment.item_id),
                        showing.soft,
                        showing.bare_near,
                        showing.bare_beyond,
                        match hidden {
                            None => "no decision",
                            Some(_) if taken => "taken off",
                            Some(_) => "kept",
                        }
                    );
                    cases.push((showing.bare_near, row));
                    worst.push((showing.bare_near, profile_id.to_string(), dress.item_id, garment.item_id));
                }
            }
        }
        summary += &format!(
            "{preset}: {} garment and dress pairs measured ({} hidden by the client's rules, {} drawn over the dress): \
             nothing shows on {}, edges only on {}, {} show beyond the band, {} in plain view. The stored decisions take \
             off {} of those where nothing shows, {} edges only, {} beyond the band and {} in plain view\n",
            tally.measured,
            tally.hidden_by_client,
            tally.drawn_over,
            tally.nothing_shows,
            tally.edges_only,
            tally.beyond_band,
            tally.in_plain_view,
            tally.taken_nothing_showed,
            tally.taken_edges_only,
            tally.taken_beyond_band,
            tally.taken_in_plain_view
        );

        // The paperdolls: the reference robe over its garments and alone, with the client's rules and with the stored
        // decisions, then the worst cases with the client's rules.
        worst.sort_by(|a, b| b.0.cmp(&a.0));
        let mut saved: Vec<(&str, &PaperdollOrder, String, Vec<u32>)> = Vec::new();
        for profile_id in REFERENCE_PROFILES {
            for outfit in REFERENCE_OUTFITS {
                let items = [outfit, &[REFERENCE_ROBE]].concat();
                saved.push(("client", &client_order, profile_id.to_string(), items.clone()));
                saved.push(("stored", &stored_order, profile_id.to_string(), items));
            }
            saved.push(("client", &client_order, profile_id.to_string(), vec![REFERENCE_ROBE]));
        }
        for (_, profile_id, dress, garment) in worst.iter().take(SAVED_CASES) {
            saved.push(("client", &client_order, profile_id.clone(), vec![*garment, *dress]));
            saved.push(("client", &client_order, profile_id.clone(), vec![*dress]));
        }
        for (side, order, profile_id, items) in saved {
            let profile = profiles.profile(&profile_id).unwrap();
            let body_gump = PROFILE_BODY_GUMPS.iter().find(|(id, _)| *id == profile_id).unwrap().1;
            let request = PaperdollRequest {
                profile: profile.clone(),
                body_gump: Some(body_gump),
                body_hue: 0,
                items: items.iter().map(|&item_id| PaperdollItem { item_id, hue: 0 }).collect(),
            };
            let stored = stored_dress_hides(Some(&package), tilemeta.item_definitions(), profile.body);
            let mut layers =
                gump_paperdoll_layers(&request, client, item_tile, tilemeta.item_definitions(), order, stored).unwrap();
            // A body the preset has no gump for is left out: the garments are what is looked at.
            layers.retain(|layer| layer.item.is_some() || gump_map.has_gump(client, layer.gump_id));
            let gump = compose_paperdoll(&gump_map, client, None, profile, &layers).unwrap();
            let stem = items.iter().map(|item| format!("{item:04X}")).collect::<Vec<_>>().join("-");
            let path = out_dir.join(format!("{preset}-{profile_id}-{stem}-{side}.png"));
            crate::util_lib::image::image_from_rgba8(gump.physical_width, gump.physical_height, &gump.canvas_rgba())
                .try_into_dynamic()
                .unwrap()
                .save(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        }
    }
    cases.sort_by(|a, b| b.0.cmp(&a.0));
    let table: String = std::iter::once(
        "preset\tprofile\tdress\tdress name\tgarment\tslot\tgarment name\tsoft\tbare near\tbare beyond\tstored decision\n"
            .to_string(),
    )
    .chain(cases.into_iter().map(|(_, row)| row))
    .collect();
    std::fs::write(out_dir.join("cases.tsv"), &table).unwrap();
    std::fs::write(out_dir.join("summary.txt"), &summary).unwrap();
    log::info!("wrote {}:\n{summary}", out_dir.display());
}
