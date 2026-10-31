//! Add missing auxiliary images while preserving every existing logical entry.
use std::{collections::{BTreeMap, BTreeSet}, path::PathBuf};
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use udd_assets::tex_aux_ec::{TexAuxEcPackage, TexAuxEcSlotRecord,
    TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH, texture_entry_path};
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
    TexAuxEcPackage::load(&args.additions)?;
    let manifest_hash = xxh64_virtual_path(TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH);
    let requested: BTreeSet<_> = args.texture.iter().copied().collect();
    let mut slots = BTreeMap::new();
    let mut added = Vec::new();
    for (index, package) in [&base, &additions].into_iter().enumerate() {
        let bytes = package.read_file_by_path_hash(manifest_hash)?;
        let size = std::mem::size_of::<TexAuxEcSlotRecord>();
        ensure!(size >= 4 && bytes.len() % size == 0, "invalid auxiliary manifest");
        for record in bytes.chunks_exact(size) {
            let id = u32::from_le_bytes(record[..4].try_into().unwrap());
            if index == 1 && !requested.contains(&id) { continue; }
            if let std::collections::btree_map::Entry::Vacant(entry) = slots.entry(id) {
                entry.insert(record.to_vec());
                if index == 1 { added.push(id); }
            }
        }
    }
    ensure!(requested.iter().all(|id| slots.contains_key(id)), "requested auxiliary image is missing");
    let mut output = UddpStreamingBuilder::create(LookupMode::VirtualPathHash, &args.output)?;
    output.add_owned_file(AddOwnedFileRequest {
        data_type: DataType::Metadata as u8, compression: CompressionFlag::ZstdNoDictLevel(3),
        width: 0, height: 0, virtual_path: None, path_hash64: Some(manifest_hash), id: None,
        data: slots.values().flatten().copied().collect(),
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
    for id in &added { append(&additions, xxh64_virtual_path(&texture_entry_path(*id)))?; }
    drop(append);
    output.add_owned_files(batch)?;
    output.finish()?;
    println!("{}", serde_json::json!({"base":args.base,"additions":args.additions,
        "output":args.output,"added_texture_ids":added,"total_images":slots.len(),
        "policy":"Existing manifest records and decoded container payloads are retained; only missing image IDs are added."}));
    Ok(())
}
