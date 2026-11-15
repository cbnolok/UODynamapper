// Census of the tileart sizing fields and a pixel comparison of TileArtLegacy (LegacyTexture.uop) with
// Classic art (artLegacyMUL.uop), using the repo readers.
// usage: census EC_CLIENT CC_CLIENT [ids...]
use color_eyre::eyre::Result;
use std::collections::BTreeMap;
use uocf::classic::art::{ArtMap, ArtSource, STATIC_TILE_ID_BASE};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::TileArtEntry;
use uocf::enhanced::textures::Textures;
use uocf::uop_container::package::UopPackage;

const USE_NEW_ART: u64 = 0x80000;
const STATIC_NAMESPACE: u32 = 0x10000;
const LEGACY_BLOCK: usize = 1;
const RGBA: usize = 4;
const FIVE_BIT_SHIFT: u8 = 3;
const CLASSIC_HALF_TILE: i32 = 22;
const DDS_FOURCC_OFFSET: usize = 84;
const FOURCC_LEN: usize = 4;
const ALPHA_FEW_DENOMINATOR: usize = 100;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let ec = std::path::Path::new(&a[1]);
    let cc = std::path::Path::new(&a[2]);
    let ids: Vec<u32> = a[3..].iter().map(|s| s.parse().unwrap()).collect();
    let dict = UoStringDictionary::load(&ec.join("string_dictionary.uop"))?;
    let package = UopPackage::load(ec.join("tileart.uop"))?;
    let legacy = Textures::new(&ec.join("LegacyTexture.uop"), None, None)?;
    let cc_art = ArtMap::load(cc)?;
    let mut scale_hist: BTreeMap<String, u32> = BTreeMap::new();
    let mut world_scale_hist: BTreeMap<String, u32> = BTreeMap::new();
    let mut window_vs_image: BTreeMap<String, u32> = BTreeMap::new();
    let mut offset_rule: BTreeMap<String, u32> = BTreeMap::new();
    let mut pixel: BTreeMap<String, u32> = BTreeMap::new();
    let mut paths: BTreeMap<String, u32> = BTreeMap::new();
    let mut examples: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    for file in package.iter_files() {
        let e = TileArtEntry::parse_raw(file)?;
        if e.tile_id >= STATIC_NAMESPACE {
            continue;
        }
        let art = e.process(&dict);
        let mode = if e.flags2.bits() & USE_NEW_ART != 0 { 1 } else if e.uses_enhanced_tileart { 2 } else { 0 };
        *scale_hist.entry(format!("mode{mode} legacy_scale={}", e.legacy_art_scale)).or_default() += 1;
        if mode == 1 {
            *world_scale_hist.entry(format!("world_scale={}", e.world_art_scale)).or_default() += 1;
        }
        let w = &e.cc_img_offset;
        let item0 = art.texture_items.iter().flatten().find(|t| t.block_index as usize == LEGACY_BLOCK && t.item_index == 0);
        if let Some(item) = item0 {
            let p = item.path.to_ascii_lowercase();
            let shape = p.chars().map(|c| if c.is_ascii_digit() { '9' } else { c }).collect::<String>();
            *paths.entry(shape).or_default() += 1;
        }
        let Some(item) = item0 else { continue };
        let ec_img: Result<(u16, u16, Vec<u8>)> = match legacy.get_from_id(item.id)? {
            Some(f) => {
                let fourcc = f.raw_bytes().get(DDS_FOURCC_OFFSET..DDS_FOURCC_OFFSET + FOURCC_LEN)
                    .map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                *paths.entry(format!("container {:?} fourcc {fourcc:?}", f.format)).or_default() += 1;
                f.decode_to_rgba8().map(|i| (i.width() as u16, i.height() as u16, i.into_raw()))
            }
            None => Err(color_eyre::eyre::eyre!("missing")),
        };
        let cc_img = cc_art.decode_static_tile_from_source(e.tile_id + STATIC_TILE_ID_BASE, ArtSource::CcUop, &mut buf);
        let ww = w.x_end - w.x_start;
        let wh = w.y_end - w.y_start;
        if e.legacy_art_scale != 1.5 {
            println!("legacy_scale {} on id {} mode {mode} window {:?}", e.legacy_art_scale, e.tile_id, w);
        }
        *window_vs_image.entry(format!("start=({},{})", w.x_start, w.y_start)).or_default() += 1;
        let rule = CLASSIC_HALF_TILE - ww / 2;
        *offset_rule.entry(format!("x_off-(22-W/2)={} y_off={} W odd={}", w.x_off - rule, w.y_off, ww % 2 != 0)).or_default() += 1;
        if let (Ok((ew, eh, ep)), Ok((cw, ch, cp))) = (&ec_img, &cc_img) {
            let fits = w.x_start >= 0 && w.y_start >= 0 && w.x_end <= *ew as i32 && w.y_end <= *eh as i32;
            let verdict = if ww != *cw as i32 || wh != *ch as i32 {
                "window size != classic size".to_string()
            } else if !fits {
                "window outside legacy image".to_string()
            } else {
                let crop = crop(ep, *ew as usize, w.x_start as usize, w.y_start as usize, ww as usize, wh as usize);
                let outside = outside_alpha(ep, *ew as usize, *eh as usize, ww as usize, wh as usize);
                format!("{}{}", compare(&crop, cp), if outside { " (+alpha outside window)" } else { "" })
            };
            if ids.contains(&e.tile_id) {
                println!("id {} mode {mode} legacy {}x{} classic {}x{} -> {verdict} window {:?} scale {}",
                    e.tile_id, ew, eh, cw, ch, w, e.legacy_art_scale);
            }
            if verdict.starts_with("colour") && examples.len() < 12 {
                examples.push(format!("colour differs on {}", e.tile_id));
            }
            *pixel.entry(verdict).or_default() += 1;
        } else if ec_img.is_err() {
            *pixel.entry("legacy image missing".into()).or_default() += 1;
        } else {
            *pixel.entry("classic missing".into()).or_default() += 1;
        }
    }
    for (name, map) in [("legacy scale", &scale_hist), ("world scale (mode 1)", &world_scale_hist),
        ("window", &window_vs_image), ("paths", &paths), ("pixels", &pixel)] {
        println!("== {name}");
        for (k, v) in map { println!("  {k}: {v}"); }
    }
    println!("== offsets (top 25)");
    let mut v: Vec<_> = offset_rule.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in v.iter().take(25) { println!("  {k}: {n}"); }
    println!("== examples");
    for x in examples { println!("  {x}"); }
    Ok(())
}

// Exact RGBA equality, else equality on the visible pixels at 5-bit precision.
fn compare(a: &[u8], b: &[u8]) -> String {
    if a == b {
        return "identical rgba".into();
    }
    let mut alpha_diff = 0usize;
    let mut colour_diff = 0usize;
    for (pa, pb) in a.chunks(RGBA).zip(b.chunks(RGBA)) {
        let va = pa[3] != 0;
        let vb = pb[3] != 0;
        if va != vb {
            alpha_diff += 1;
        } else if va && (0..3).any(|c| pa[c] >> FIVE_BIT_SHIFT != pb[c] >> FIVE_BIT_SHIFT) {
            colour_diff += 1;
        }
    }
    match (alpha_diff, colour_diff) {
        (0, 0) => "same visible pixels (5-bit)".into(),
        (0, _) => "colour differs".into(),
        (n, _) if n * ALPHA_FEW_DENOMINATOR <= a.len() / RGBA => "alpha mask differs on <=1% of pixels".into(),
        _ => "alpha mask differs on >1% of pixels".into(),
    }
}

fn crop(src: &[u8], src_w: usize, x0: usize, y0: usize, w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * RGBA);
    for y in y0..y0 + h {
        let row = (y * src_w + x0) * RGBA;
        out.extend_from_slice(&src[row..row + w * RGBA]);
    }
    out
}

// Any visible pixel of the legacy image outside the top-left window?
fn outside_alpha(src: &[u8], src_w: usize, src_h: usize, w: usize, h: usize) -> bool {
    (0..src_h).any(|y| (0..src_w).any(|x| (x >= w || y >= h) && src[(y * src_w + x) * RGBA + 3] != 0))
}
