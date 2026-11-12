//! Add missing auxiliary images while preserving every existing logical entry.
use std::{collections::{BTreeMap, BTreeSet}, path::PathBuf};
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use udd_assets::tex_aux_ec::{decode_tex_aux_ec_slots, encode_tex_aux_ec_slots, TexAuxEcPackage, TexAuxEcSlotRecord,
    TEX_AUX_EC_MISSING_TEXTURE_ID, TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH, texture_entry_path};
use udd_container::{AddOwnedFileRequest, CompressionFlag, DataType, FileKey,
    LookupMode, UddpReader, UddpStreamingBuilder, xxh64_virtual_path};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    base: PathBuf,
    #[arg(long)]
    additions: PathBuf,
    /// Explicit source-backed image IDs to add; unrelated rebuilt entries are excluded.
    #[arg(long, required = true, num_args = 1..)]
    texture: Vec<u32>,
    /// New package; existing assets are never overwritten.
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "output already exists: {:?}", args.output);
    let base = UddpReader::load(&args.base)?;
    let additions = UddpReader::load(&args.additions)?;
    // Validate both manifests through the current runtime format reader.
    TexAuxEcPackage::load(&args.base)?;
    let additions_package = TexAuxEcPackage::load(&args.additions)?;
    let manifest_hash = xxh64_virtual_path(TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH);
    let requested: BTreeSet<_> = args.texture.iter().copied().collect();
    let mut slots = BTreeMap::new();
    let mut added = Vec::new();
    for (index, package) in [&base, &additions].into_iter().enumerate() {
        let bytes = package.read_file_by_path_hash(manifest_hash)?;
        for record in decode_tex_aux_ec_slots(&bytes)? {
            let id = record.texture_id;
            if index == 1 && !requested.contains(&id) { continue; }
            if let std::collections::btree_map::Entry::Vacant(entry) = slots.entry(id) {
                entry.insert(record);
                if index == 1 { added.push(id); }
            }
        }
    }
    ensure!(requested.iter().all(|id| slots.contains_key(id)), "requested auxiliary image is missing");
    let mut output = UddpStreamingBuilder::create(LookupMode::VirtualPathHash, &args.output)?;
    output.add_owned_file(AddOwnedFileRequest {
        data_type: DataType::Metadata as u8, compression: CompressionFlag::ZstdNoDictLevel(3),
        width: 0, height: 0, virtual_path: None, path_hash64: Some(manifest_hash), id: None,
        data: encode_tex_aux_ec_slots(&slots.values().copied().collect::<Vec<TexAuxEcSlotRecord>>()),
    })?;
    let mut batch = Vec::new();
    let mut append = |package: &UddpReader, hash| -> Result<()> {
        let file = package.find_by_path_hash(hash)
            .ok_or_else(|| color_eyre::eyre::eyre!("missing auxiliary payload {hash:016x}"))?;
        batch.push(AddOwnedFileRequest {
            data_type: file.data_type, compression: CompressionFlag::ZstdNoDictLevel(3),
            width: 0, height: 0, virtual_path: None, path_hash64: Some(hash), id: None,
            data: package.read_file_by_path_hash(hash)?,
        });
        if batch.len() >= 128 { output.add_owned_files(std::mem::take(&mut batch))?; }
        Ok(())
    };
    for record in base.records() {
        let FileKey::PathHash(hash) = record.key else {
            color_eyre::eyre::bail!("auxiliary package must use path hashes");
        };
        if hash != manifest_hash { append(&base, hash)?; }
    }
    for id in &added {
        // A missing-source alias has no payload: it reads the base package's missing-texture image.
        if additions_package.slot_for_texture_id(*id).is_some_and(|slot| slot.is_missing_source()) {
            ensure!(slots.contains_key(&TEX_AUX_EC_MISSING_TEXTURE_ID), "added alias {id} needs the missing-texture slot");
            continue;
        }
        append(&additions, xxh64_virtual_path(&texture_entry_path(*id)))?;
    }
    drop(append);
    output.add_owned_files(batch)?;
    output.finish()?;
    println!("{}", serde_json::json!({"base":args.base,"additions":args.additions,
        "output":args.output,"added_texture_ids":added,"total_images":slots.len(),
        "policy":"Existing manifest records and decoded container payloads are retained; only missing image IDs are added."}));
    Ok(())
}
