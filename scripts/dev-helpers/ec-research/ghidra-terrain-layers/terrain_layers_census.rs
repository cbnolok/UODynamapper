// Census of TerrainDefinition.uop layers against the client's binding (vector 0 = generated coverage,
// texture item i = vector i + 1) and against the repo's name/repetition heuristics.
// usage: terrain_layers EC_CLIENT
use color_eyre::eyre::Result;
use std::collections::BTreeMap;
use uocf::enhanced::terrain_definition::TerrainDefinitionPackage;

// Default/bump layers: item 0 base A (where mask alpha = 1), item 1 base B (TEMP), item 2 mask (alpha).
const ITEM_BASE_A: usize = 0;
const ITEM_BASE_B: usize = 1;
const ITEM_MASK: usize = 2;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let ec = std::path::Path::new(&a[1]);
    let pkg = TerrainDefinitionPackage::load_with_dictionary(&ec.join("TerrainDefinition.uop"), &ec.join("string_dictionary.uop"))?;
    let mut tally: BTreeMap<String, u32> = BTreeMap::new();
    let mut entries: Vec<_> = pkg.entries.iter().collect();
    entries.sort_by_key(|e| e.id);
    for e in entries {
        let Some(t) = e.texture.as_ref() else { println!("{} no texture", e.id); continue };
        let shader = t.shader_name.clone().unwrap_or_default();
        let items: Vec<String> = t.layers.iter().map(|l| format!("{}@{}", l.path.clone().unwrap_or_default(), l.texture_repetition)).collect();
        let heur = e.primary_texture_layer_with_reason().map(|(l, r)| {
            let pos = t.layers.iter().position(|x| std::ptr::eq(x, l)).unwrap();
            (pos, r.as_str())
        });
        println!("{:3} {:20} {:22} exp={:<5} tech={:?} tweaks={:?} items={:?} heuristic_primary={:?}",
            e.id, e.name.clone().unwrap_or_default(), shader, e.coverage_exponent, t.techniques, t.tweaks, items, heur);
        *tally.entry(format!("{shader} items={}", t.layers.len())).or_default() += 1;
        if let Some((pos, _)) = heur {
            *tally.entry(format!("{shader} heuristic primary = item {pos}")).or_default() += 1;
        }
        for (i, l) in t.layers.iter().enumerate() {
            let p = l.path.clone().unwrap_or_default().to_ascii_lowercase();
            let support = l.is_support_layer_by_current_name_heuristic();
            let clue = l.has_support_like_name_clue();
            *tally.entry(format!("{shader} item {i}: support_name={support} clue={clue}")).or_default() += 1;
            *tally.entry(format!("{shader} item {i}: repetition {}", l.texture_repetition)).or_default() += 1;
            if i == ITEM_MASK && !support && shader != "UOWaterTerrainLayer" {
                *tally.entry(format!("{shader} mask item without support name: {p}")).or_default() += 1;
            }
            if (i == ITEM_BASE_A || i == ITEM_BASE_B) && support {
                *tally.entry(format!("{shader} base item {i} with support name: {p}")).or_default() += 1;
            }
        }
    }
    println!("== tally");
    for (k, v) in tally { println!("  {k}: {v}"); }
    Ok(())
}
