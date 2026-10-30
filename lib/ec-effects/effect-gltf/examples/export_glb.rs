//! Exports NIF effects of an Enhanced Client's `EffectTexture.uop` to `.glb` files.
//!
//! Usage: `cargo run -p effect-gltf --example export_glb -- <client dir> <output dir> [--bake] [--clip <seconds>]
//! <name.nif>...` (the output directory must be an absolute path; it is created when missing).

use std::path::PathBuf;

use effect_gltf::validate::validate_glb;
use effect_gltf::{BakeOptions, ExportOptions, export_nif_file};
use gamebryo_nif::NifFile;
use uocf::enhanced::effect_texture::EffectTexturePackage;

const USAGE: &str = "usage: export_glb <client dir> <absolute output dir> [--bake] [--clip <seconds>] <name.nif>...";
const BAKE_FLAG: &str = "--bake";
const CLIP_FLAG: &str = "--clip";
const NIF_EXTENSION: &str = ".nif";
const GLB_EXTENSION: &str = "glb";

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let client_dir = PathBuf::from(args.next().ok_or(USAGE)?);
    let out_dir = PathBuf::from(args.next().ok_or(USAGE)?);
    if !out_dir.is_absolute() {
        return Err(format!("{}: the output directory must be absolute\n{USAGE}", out_dir.display()));
    }
    let mut options = ExportOptions::default();
    let mut names = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            BAKE_FLAG => options.bake = Some(BakeOptions::default()),
            CLIP_FLAG => {
                let seconds = args.next().ok_or(USAGE)?;
                options.clip_length = Some(seconds.parse().map_err(|_| format!("{seconds}: not a number"))?);
            }
            _ => names.push(arg),
        }
    }
    if names.is_empty() {
        return Err(USAGE.to_owned());
    }
    let package = EffectTexturePackage::load_with_names(&client_dir).map_err(|error| error.to_string())?;
    let loader = |name: &str| package.read(name).ok().flatten();
    std::fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;
    for name in names {
        let Some(payload) = package.read(&name).map_err(|error| error.to_string())? else {
            eprintln!("{name}: not in the package");
            continue;
        };
        let nif = NifFile::read(&payload).map_err(|error| format!("{name}: {error}"))?;
        let export = export_nif_file(&nif, &name, &loader, &options);
        for warning in &export.warnings {
            eprintln!("warning: {warning}");
        }
        let summary = validate_glb(&export.bytes).map_err(|errors| format!("{name}: {errors:#?}"))?;
        let stem = name.strip_suffix(NIF_EXTENSION).unwrap_or(&name);
        let path = out_dir.join(format!("{stem}.{GLB_EXTENSION}"));
        std::fs::write(&path, &export.bytes).map_err(|error| format!("{}: {error}", path.display()))?;
        println!("{}: {} bytes, {} nodes, {} meshes, {} channels", path.display(), summary.bytes, summary.nodes, summary.meshes, summary.channels);
    }
    Ok(())
}
