// Census of TerrainDefinition.uop layers against the client's binding (vector 0 = generated coverage,
// texture item i = vector i + 1): per layer class (from the shader name), the role of each item position,
// its repetitions, and the primary item. The repo's former name/repetition heuristics are gone; roles come
// from `TerrainLayerClass::role` (lib/uocf terrain_definition.rs).
// usage: terrain_layers EC_CLIENT
use color_eyre::eyre::Result;
use std::collections::BTreeMap;
use uocf::enhanced::terrain_definition::TerrainDefinitionPackage;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let ec = std::path::Path::new(&a[1]);
    let pkg = TerrainDefinitionPackage::load_with_dictionary(&ec.join("TerrainDefinition.uop"), &ec.join("string_dictionary.uop"))?;
    let mut tally: BTreeMap<String, u32> = BTreeMap::new();
    let mut entries: Vec<_> = pkg.entries.iter().collect();
    entries.sort_by_key(|e| e.id);
    for e in entries {
        let Some(t) = e.texture.as_ref() else { println!("{} no texture", e.id); continue };
        let class = t.layer_class();
        let shader = t.shader_name.clone().unwrap_or_default();
        let items: Vec<String> = t.layers.iter().enumerate()
            .map(|(i, l)| format!("{}:{}@{}", t.layer_role(i).as_str(), l.path.clone().unwrap_or_default(), l.texture_repetition))
            .collect();
        let primary = e.primary_texture_layer().map(|(pos, _)| pos);
        println!("{:3} {:20} {:22} class={:7} exp={:<5} tech={:?} tweaks={:?} items={:?} primary={:?}",
            e.id, e.name.clone().unwrap_or_default(), shader, class.as_str(), e.coverage_exponent(),
            t.techniques(), t.tweaks(), items, primary);
        let class = class.as_str();
        *tally.entry(format!("{class} ({shader}) items={}", t.layers.len())).or_default() += 1;
        if let Some(pos) = primary {
            *tally.entry(format!("{class} primary = item {pos}")).or_default() += 1;
        }
        if t.tweaks().iter().any(|tweak| *tweak != 0.0) {
            *tally.entry(format!("{class} with non-zero tweaks")).or_default() += 1;
        }
        for (i, l) in t.layers.iter().enumerate() {
            let role = t.layer_role(i).as_str();
            *tally.entry(format!("{class} item {i} ({role}): repetition {}", l.texture_repetition)).or_default() += 1;
        }
    }
    println!("== tally");
    for (k, v) in tally { println!("  {k}: {v}"); }
    Ok(())
}
