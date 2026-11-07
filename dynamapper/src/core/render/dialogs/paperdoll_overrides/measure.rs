//! What the decisions paperdoll_ec.uddp stores take away under a robe, as Dynamapper draws them (under
//! `PaperdollOverrides.kdl`): every item the Enhanced paperdoll wears as a dress, drawn over full outfits and over each
//! garment alone, with the client's rules and with Dynamapper's, on the human, elf and gargoyle bodies. What it measures
//! is what is taken away where the dress does not cover: the texels that change outside the dress's own opaque texels.
//! Near the dress those are the edges the decisions are for; far from it, a large area means the dress leaves that
//! garment in plain view, and the decision hides something the player would see. The packer measures each garment
//! beneath each dress the same way (`udd_conv::paperdoll_ec_dress`); these outfits check the result as it is drawn.

use super::*;
use crate::core::render::dialogs::paperdoll_ec::{paperdoll_drawing, PaperdollCanvas, PaperdollItem};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use udd_assets::tilemeta::TileMetaPackage;
use udd_assets::{HuesPackage, PaperdollEcPackage};
use uocf::enhanced::paperdoll::{item_layer, slot_layer, PlacedFrame};

/// How far from the dress, in texels across or down, a texel taken away still counts as an edge showing past it, as
/// the packer's `EDGE_BAND_TEXELS`. At 16 the robes' edges fall inside it: what the reviewed rule took away beyond it,
/// on any outfit here, was at most 911 texels for a robe, and at least 3033 for a dress that leaves an item in plain
/// view.
const EDGE_BAND_TEXELS: usize = 16;
/// Texels that may be taken away beyond the edge band before the dress is flagged, as the packer's
/// `LARGE_REMOVED_AREA_TEXELS`: about a 40 x 40 patch at full size, 28 x 28 points in the window, between the two
/// groups above.
const LARGE_REMOVED_AREA_TEXELS: usize = 1600;
/// An override that keeps everything beneath any robe: the client's rules.
const CLIENT_RULES_OVERRIDES: &str = "cover \"dress\" items=\"any\"\n";
/// The hidden items are dyed so what the overrides take away stands out on the sheets.
const HIDDEN_ITEM_HUE: u16 = 3;
const SKIN_HUE: u16 = 1002;
const RGBA_BYTES: usize = 4;
const ALPHA_BYTE: usize = 3;
const OPAQUE: u8 = u8::MAX;
/// A taken-away texel on the `removed` images.
const MARK: [u8; RGBA_BYTES] = [OPAQUE, 0, OPAQUE, OPAQUE];

/// A body and the outfits drawn on it: the items the robe rule hides, dyed, and the items it keeps.
struct Wearer {
    name: &'static str,
    body: u32,
    outfits: [(&'static str, &'static [u32]); 2],
    kept: &'static [u32],
}

/// Shirt, long pants, skirt and tunic.
const HUMAN_CLOTH: &[u32] = &[0x1517, 0x1539, 0x1516, 0x1FA1];
/// Plate chest, arms and legs, a kilt and a body sash.
const HUMAN_ARMOUR: &[u32] = &[0x1415, 0x1410, 0x1411, 0x1537, 0x1541];
/// Shoes, gloves, ring, bracelet, necklace, earrings, cloak and talisman.
const HUMAN_KEPT: &[u32] = &[0x170F, 0x13C6, 0x108A, 0x1086, 0x1088, 0x1087, 0x1515, 0x2F58];
/// Gargish cloth chest, arms and legs, then leather; their kilts are worn on the hands layer, which the rule keeps.
const GARGOYLE_MALE_CLOTH: &[u32] = &[0x0405, 0x0403, 0x0409];
const GARGOYLE_MALE_LEATHER: &[u32] = &[0x0303, 0x0301, 0x0305];
const GARGOYLE_FEMALE_CLOTH: &[u32] = &[0x0406, 0x0404, 0x040A];
const GARGOYLE_FEMALE_LEATHER: &[u32] = &[0x0304, 0x0302, 0x0306];
/// A kilt, necklace, bracelet, ring and earrings.
const GARGOYLE_MALE_KEPT: &[u32] = &[0x0407, 0x4210, 0x4211, 0x4212, 0x4213];
const GARGOYLE_FEMALE_KEPT: &[u32] = &[0x0408, 0x4210, 0x4211, 0x4212, 0x4213];

const WEARERS: [Wearer; 6] = [
    Wearer { name: "human_male", body: 400, outfits: [("cloth", HUMAN_CLOTH), ("armour", HUMAN_ARMOUR)], kept: HUMAN_KEPT },
    Wearer { name: "human_female", body: 401, outfits: [("cloth", HUMAN_CLOTH), ("armour", HUMAN_ARMOUR)], kept: HUMAN_KEPT },
    Wearer { name: "elf_male", body: 605, outfits: [("cloth", HUMAN_CLOTH), ("armour", HUMAN_ARMOUR)], kept: HUMAN_KEPT },
    Wearer { name: "elf_female", body: 606, outfits: [("cloth", HUMAN_CLOTH), ("armour", HUMAN_ARMOUR)], kept: HUMAN_KEPT },
    Wearer {
        name: "gargoyle_male",
        body: 666,
        outfits: [("cloth", GARGOYLE_MALE_CLOTH), ("leather", GARGOYLE_MALE_LEATHER)],
        kept: GARGOYLE_MALE_KEPT,
    },
    Wearer {
        name: "gargoyle_female",
        body: 667,
        outfits: [("cloth", GARGOYLE_FEMALE_CLOTH), ("leather", GARGOYLE_FEMALE_LEATHER)],
        kept: GARGOYLE_FEMALE_KEPT,
    },
];

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")))
}

struct Sources {
    package: PaperdollEcPackage,
    tilemeta: TileMetaPackage,
    hues: HuesPackage,
    canvas: PaperdollCanvas,
    client: PaperdollOrder,
    dynamapper: PaperdollOrder,
}

impl Sources {
    /// The canvas `order` draws for `body` wearing `items`, and its frames.
    fn draw(&self, order: &PaperdollOrder, body: u32, items: &[PaperdollItem]) -> (Vec<u8>, Vec<PlacedFrame>) {
        let frames =
            paperdoll_drawing(&self.package, &self.tilemeta, Some(&self.hues), order, body, SKIN_HUE, items, &mut HashSet::new())
                .unwrap_or_else(|error| panic!("body {body} wearing {items:x?}: {error:#}"))
                .frames;
        (self.canvas.compose(&frames), frames)
    }
}

/// What the overrides take away where the dress does not cover.
#[derive(Clone, Copy, Default)]
struct Removed {
    /// Texels that change where the dress is not opaque.
    outside: usize,
    /// Of those, texels the dress does not touch at all.
    bare: usize,
    /// Of those, texels beyond the edge band.
    beyond_edge: usize,
}

/// Whether each texel is within `radius` texels (across or down, the larger) of a texel the dress touches.
fn near_dress(dress: &[u8], width: usize, height: usize, radius: usize) -> Vec<bool> {
    let touched: Vec<bool> = dress.chunks_exact(RGBA_BYTES).map(|texel| texel[ALPHA_BYTE] != 0).collect();
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

fn removed(before: &[u8], after: &[u8], dress: &[u8], near: &[bool], mut mark: impl FnMut(usize)) -> Removed {
    let mut removed = Removed::default();
    for (index, ((before, after), dress)) in
        before.chunks_exact(RGBA_BYTES).zip(after.chunks_exact(RGBA_BYTES)).zip(dress.chunks_exact(RGBA_BYTES)).enumerate()
    {
        if before == after || dress[ALPHA_BYTE] == OPAQUE {
            continue;
        }
        removed.outside += 1;
        removed.bare += usize::from(dress[ALPHA_BYTE] == 0);
        removed.beyond_edge += usize::from(!near[index]);
        mark(index);
    }
    removed
}

fn save(canvas: &PaperdollCanvas, rgba: &[u8], path: &Path) {
    canvas.image(rgba).try_into_dynamic().unwrap().save(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn items(hidden: &[u32], kept: &[u32], dress: u32) -> Vec<PaperdollItem> {
    hidden
        .iter()
        .map(|&item_id| PaperdollItem { item_id, hue: HIDDEN_ITEM_HUE })
        .chain(kept.iter().chain([&dress]).map(|&item_id| PaperdollItem { item_id, hue: 0 }))
        .collect()
}

/// The items the Enhanced paperdoll wears on the dress layer: their `tileart.uop` layer, else their tiledata one.
fn dresses(tilemeta: &TileMetaPackage) -> Vec<u32> {
    let dress = slot_layer("dress").unwrap();
    (0..tilemeta.item_tiles().len() as u32)
        .filter(|&item_id| {
            let tiledata_layer = tilemeta.item_tile(item_id).map(|tile| tile.quality).filter(|&layer| layer != 0);
            item_layer(tilemeta.item_definitions(), item_id, tiledata_layer) == Some(dress)
        })
        .collect()
}

/// For each wearer, every dress with a frame for its body, grouped by that frame; each group is drawn once over each
/// outfit and over each hidden item alone, with the client's order and with Dynamapper's: the stored decisions under
/// the bundled overrides, or under the overrides file `UDD_PAPERDOLL_OVERRIDES` names. Writes `ranked.tsv` (every
/// case, the most taken away beyond the edge band first), `sheet.tsv` (the outfit cases, for the contact sheets) and,
/// for each outfit case, the canvases before and after and the texels taken away outside the dress, to
/// `$TMPDIR/robe-override`; then fails if any case is flagged. Two dresses of one frame share a group, so an override
/// that sets them apart is measured on the group's first item.
#[test]
#[ignore = "needs packages: set UDD_PAPERDOLL_EC, UDD_TILEMETA, UDD_HUES and TMPDIR, and run with --ignored"]
fn the_stored_decisions_take_away_little_outside_the_dress() {
    let package = PaperdollEcPackage::load(env_path("UDD_PAPERDOLL_EC")).unwrap();
    let tilemeta = TileMetaPackage::load(env_path("UDD_TILEMETA")).unwrap();
    let hues = HuesPackage::load(env_path("UDD_HUES")).unwrap();
    let rules = WearableRules::decode(include_str!("../../../../../assets/runtime_specs/paperdoll/WearableRules.kdl"))
        .unwrap();
    let overrides_text = match std::env::var_os("UDD_PAPERDOLL_OVERRIDES") {
        Some(path) => std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display())),
        None => include_str!("../../../../../assets/runtime_specs/paperdoll/PaperdollOverrides.kdl").to_string(),
    };
    let overrides = PaperdollOverrides::decode(PAPERDOLL_OVERRIDES_FILE_NAME, &overrides_text).unwrap();
    let canvas = PaperdollCanvas::for_sets(package.sets());
    let sources = Sources {
        package,
        tilemeta,
        hues,
        canvas,
        client: PaperdollOrder {
            rules: rules.clone(),
            overrides: PaperdollOverrides::decode(PAPERDOLL_OVERRIDES_FILE_NAME, CLIENT_RULES_OVERRIDES).unwrap(),
        },
        dynamapper: PaperdollOrder { rules, overrides },
    };
    let out_dir = std::env::temp_dir().join("robe-override");
    assert!(out_dir.is_absolute(), "{}", out_dir.display());
    std::fs::create_dir_all(&out_dir).unwrap();
    let dresses = dresses(&sources.tilemeta);
    let (width, height) = (sources.canvas.width as usize, sources.canvas.height as usize);

    let mut ranked = Vec::new();
    let mut flagged_cases = Vec::new();
    let mut sheet = String::from(
        "wearer\toutfit\tdress items\tname\tremoved outside\tbare\tbeyond edge\tflagged\tbefore\tafter\tremoved\n",
    );
    let mut without_frame: HashMap<&str, Vec<u32>> = HashMap::new();
    for wearer in &WEARERS {
        // The dresses with a frame for this body, grouped by that frame.
        let mut groups: Vec<(u64, Vec<u32>, Vec<u8>)> = Vec::new();
        for &dress in &dresses {
            let (_, frames) = sources.draw(&sources.client, wearer.body, &[PaperdollItem { item_id: dress, hue: 0 }]);
            let [_, dress_frame] = frames.as_slice() else {
                without_frame.entry(wearer.name).or_default().push(dress);
                continue;
            };
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (dress_frame.x, dress_frame.y, dress_frame.width, dress_frame.height, &dress_frame.rgba).hash(&mut hasher);
            let key = hasher.finish();
            match groups.iter_mut().find(|(group, ..)| *group == key) {
                Some((_, items, _)) => items.push(dress),
                None => groups.push((key, vec![dress], sources.canvas.compose(std::slice::from_ref(dress_frame)))),
            }
        }
        for (_, group, dress_alpha) in &groups {
            let dress = group[0];
            let near = near_dress(dress_alpha, width, height, EDGE_BAND_TEXELS);
            let name = sources.tilemeta.item_tile(dress).map(|tile| tile.name_ascii().to_string()).unwrap_or_default();
            let ids = group.iter().map(|item| format!("0x{item:04X}")).collect::<Vec<_>>().join(",");
            let mut cases: Vec<(String, Vec<u32>)> =
                wearer.outfits.iter().map(|&(outfit, hidden)| (outfit.to_string(), hidden.to_vec())).collect();
            let singles: Vec<u32> = wearer.outfits.iter().flat_map(|(_, hidden)| hidden.iter().copied()).collect();
            cases.extend(singles.into_iter().map(|item| (format!("0x{item:04X} alone"), vec![item])));
            for (case, hidden) in &cases {
                let worn = items(hidden, wearer.kept, dress);
                let (before, _) = sources.draw(&sources.client, wearer.body, &worn);
                let (after, _) = sources.draw(&sources.dynamapper, wearer.body, &worn);
                let mut mask = vec![0u8; before.len()];
                let taken = removed(&before, &after, dress_alpha, &near, |texel| {
                    mask[texel * RGBA_BYTES..(texel + 1) * RGBA_BYTES].copy_from_slice(&MARK);
                });
                let flagged = taken.beyond_edge >= LARGE_REMOVED_AREA_TEXELS;
                let row = format!(
                    "{}\t{case}\t{ids}\t{name}\t{}\t{}\t{}\t{}",
                    wearer.name,
                    taken.outside,
                    taken.bare,
                    taken.beyond_edge,
                    if flagged { "flagged" } else { "" }
                );
                if flagged {
                    flagged_cases.push(row.clone());
                }
                ranked.push((taken.beyond_edge, row));
                if hidden.len() > 1 {
                    let stem = format!("{}-{case}-0x{dress:04X}", wearer.name);
                    let paths = ["before", "after", "removed"].map(|side| out_dir.join(format!("{stem}-{side}.png")));
                    for (path, rgba) in paths.iter().zip([&before, &after, &mask]) {
                        save(&sources.canvas, rgba, path);
                    }
                    sheet += &format!(
                        "{}\t{case}\t{ids}\t{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                        wearer.name,
                        taken.outside,
                        taken.bare,
                        taken.beyond_edge,
                        u8::from(flagged),
                        paths[0].display(),
                        paths[1].display(),
                        paths[2].display()
                    );
                }
            }
        }
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    let mut table = format!(
        "# flagged at {LARGE_REMOVED_AREA_TEXELS} texels or more taken away beyond {EDGE_BAND_TEXELS} texels from the dress\n\
         wearer\tcase\tdress items\tname\tremoved outside\tbare\tbeyond edge\tflagged\n"
    );
    for (_, row) in &ranked {
        table += row;
        table.push('\n');
    }
    for wearer in &WEARERS {
        let missing = without_frame.get(wearer.name).map(Vec::as_slice).unwrap_or_default();
        table += &format!("# {}: {} dresses without a frame: {missing:x?}\n", wearer.name, missing.len());
    }
    std::fs::write(out_dir.join("ranked.tsv"), &table).unwrap();
    std::fs::write(out_dir.join("sheet.tsv"), &sheet).unwrap();
    assert!(flagged_cases.is_empty(), "{} flagged cases, in {}:\n{}", flagged_cases.len(), out_dir.display(), flagged_cases.join("\n"));
}
