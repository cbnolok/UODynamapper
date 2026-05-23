use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use color_eyre::eyre::{self, Context};
use uocf::uop_container::package::UopPackage;

#[derive(Parser, Debug)]
#[command(author, version, about = "Compare KR terrain routing against EC facet and TerrainDefinition evidence.")]
struct Cli {
    #[arg(long)]
    kr_generated: PathBuf,
    #[arg(long)]
    ec_generated: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    krdir: Option<PathBuf>,
    #[arg(long)]
    ecdir: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct EcMaterial {
    primary_texture_id: Option<u32>,
    aliases: Vec<u32>,
    layers: Vec<(u32, f32, String)>,
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    let cli = Cli::parse();
    let kr = parse_route_map(&cli.kr_generated, "kr")?;
    let ec_facet = parse_route_map(&cli.ec_generated, "ec")?;
    let ec_materials = parse_ec_materials(&cli.ec_generated)?;
    let ec_alias_to_material = ec_materials
        .iter()
        .flat_map(|(material_id, material)| {
            material
                .aliases
                .iter()
                .copied()
                .map(|alias| (alias, *material_id))
                .collect::<Vec<_>>()
        })
        .collect::<BTreeMap<_, _>>();

    let mut cc_ids = BTreeSet::new();
    cc_ids.extend(kr.keys().copied());
    cc_ids.extend(ec_facet.keys().copied());

    let mut csv = String::from(
        "cc_id,kr_material_id,ec_facet_id,ec_material_id,kr_matches_ec_material,ec_primary_texture_id,ec_layer_texture_ids,ec_layer_repetitions,notes\n",
    );
    let mut same_material = 0usize;
    let mut comparable = 0usize;
    let mut missing_ec_material = 0usize;

    for cc_id in cc_ids {
        let kr_material = kr.get(&cc_id).copied();
        let ec_facet_id = ec_facet.get(&cc_id).copied();
        let ec_material_id = ec_facet_id.and_then(|id| {
            ec_alias_to_material
                .get(&id)
                .copied()
                .or_else(|| ec_materials.contains_key(&id).then_some(id))
        });
        let matches = kr_material.zip(ec_material_id).map(|(kr, ec)| kr == ec);
        if matches.is_some() {
            comparable += 1;
        }
        if matches == Some(true) {
            same_material += 1;
        }
        if ec_facet_id.is_some() && ec_material_id.is_none() {
            missing_ec_material += 1;
        }

        let material = ec_material_id.and_then(|id| ec_materials.get(&id));
        let primary = material
            .and_then(|material| material.primary_texture_id)
            .map(|value| value.to_string())
            .unwrap_or_default();
        let layer_ids = material
            .map(|material| {
                material
                    .layers
                    .iter()
                    .map(|(texture_id, _, _)| texture_id.to_string())
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .unwrap_or_default();
        let repetitions = material
            .map(|material| {
                material
                    .layers
                    .iter()
                    .map(|(_, repetition, _)| format!("{repetition:.6}"))
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .unwrap_or_default();
        let notes = if ec_facet_id.is_some() && ec_material_id.is_none() {
            "ec_facet_id_has_no_terrain_material"
        } else if matches == Some(false) {
            "different_material_route"
        } else {
            ""
        };

        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            cc_id,
            optional_u32(kr_material),
            optional_u32(ec_facet_id),
            optional_u32(ec_material_id),
            matches.map(|value| value.to_string()).unwrap_or_default(),
            primary,
            csv_escape(&layer_ids),
            csv_escape(&repetitions),
            notes
        ));
    }

    fs::write(&cli.output, csv).wrap_err_with(|| format!("write {}", cli.output.display()))?;
    println!("wrote {}", cli.output.display());
    println!(
        "routes: cc_ids={} comparable={} same_material={} different_material={} ec_facet_without_material={}",
        kr.len().max(ec_facet.len()),
        comparable,
        same_material,
        comparable.saturating_sub(same_material),
        missing_ec_material
    );

    if let (Some(krdir), Some(ecdir)) = (cli.krdir.as_ref(), cli.ecdir.as_ref()) {
        report_package_containment(krdir, ecdir)?;
    }

    Ok(())
}

fn parse_route_map(path: &Path, target: &str) -> eyre::Result<BTreeMap<u32, u32>> {
    let text = fs::read_to_string(path).wrap_err_with(|| format!("read {}", path.display()))?;
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("t ") {
            continue;
        }
        let Some(cc) = property_u32(line, "cc") else {
            continue;
        };
        let Some(target_id) = property_u32(line, target) else {
            continue;
        };
        map.insert(cc, target_id);
    }
    Ok(map)
}

fn parse_ec_materials(path: &Path) -> eyre::Result<BTreeMap<u32, EcMaterial>> {
    let text = fs::read_to_string(path).wrap_err_with(|| format!("read {}", path.display()))?;
    let mut materials = BTreeMap::new();
    let mut current_id = None;

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("material ") {
            let Some(material_id) = property_u32(rest, "id") else {
                continue;
            };
            let material = EcMaterial {
                primary_texture_id: property_u32(rest, "primary_texture_id"),
                ..EcMaterial::default()
            };
            if line.ends_with('}') {
                current_id = None;
            } else {
                current_id = Some(material_id);
            }
            materials.insert(material_id, material);
            if line.contains(" {") {
                current_id = Some(material_id);
            }
            continue;
        }

        let Some(material_id) = current_id else {
            continue;
        };
        if line == "}" {
            current_id = None;
            continue;
        }
        let material = materials
            .get_mut(&material_id)
            .expect("current material was inserted");
        if let Some(rest) = line.strip_prefix("alias ") {
            if let Some(alias) = property_u32(rest, "id") {
                if alias != 0 {
                    material.aliases.push(alias);
                }
            }
        } else if let Some(rest) = line.strip_prefix("layer ") {
            if let Some(texture_id) = property_u32(rest, "texture_id") {
                let repetition = property_f32(rest, "repetition").unwrap_or_default();
                let path = property_string(rest, "path").unwrap_or_default();
                material.layers.push((texture_id, repetition, path));
            }
        }
    }

    Ok(materials)
}

fn report_package_containment(krdir: &Path, ecdir: &Path) -> eyre::Result<()> {
    for package_name in ["Texture.uop", "LegacyTexture.uop", "TerrainTexture.uop"] {
        let kr_path = krdir.join(package_name);
        let ec_path = ecdir.join(package_name);
        if !kr_path.exists() || !ec_path.exists() {
            println!(
                "package {package_name}: skipped kr_exists={} ec_exists={}",
                kr_path.exists(),
                ec_path.exists()
            );
            continue;
        }
        let kr_hashes = package_hashes(&kr_path)?;
        let ec_hashes = package_hashes(&ec_path)?;
        let kr_not_ec = kr_hashes.difference(&ec_hashes).count();
        let ec_not_kr = ec_hashes.difference(&kr_hashes).count();
        println!(
            "package {package_name}: kr_files={} ec_files={} kr_not_in_ec={} ec_not_in_kr={}",
            kr_hashes.len(),
            ec_hashes.len(),
            kr_not_ec,
            ec_not_kr
        );
        if kr_not_ec > 0 {
            let first_missing = kr_hashes
                .difference(&ec_hashes)
                .take(16)
                .map(|hash| format!("0x{hash:016x}"))
                .collect::<Vec<_>>()
                .join(",");
            println!("package {package_name}: first_kr_not_in_ec={first_missing}");
        }
    }
    Ok(())
}

fn package_hashes(path: &Path) -> eyre::Result<BTreeSet<u64>> {
    let package = UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
    Ok(package
        .iter_files()
        .filter(|file| file.has_size())
        .map(|file| file.filename_hash())
        .collect())
}

fn property_u32(line: &str, key: &str) -> Option<u32> {
    property_value(line, key)?.parse().ok()
}

fn property_f32(line: &str, key: &str) -> Option<f32> {
    property_value(line, key)?.parse().ok()
}

fn property_string(line: &str, key: &str) -> Option<String> {
    let value = property_value(line, key)?;
    Some(value.trim_matches('"').to_string())
}

fn property_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(&prefix))
}

fn optional_u32(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
