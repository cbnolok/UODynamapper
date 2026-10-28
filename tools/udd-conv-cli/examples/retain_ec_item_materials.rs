//! Carry source material metadata into an existing package after checking sampler identities.

use std::{io::Write, path::PathBuf};
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use udd_assets::tilemeta::{TileMetaItemTextureRef, TileMetaPackage, TILEMETA_ITEM_MATERIAL_ENTRY_PATH};
use udd_container::{AddFileRequest, CompressionFlag, DataType, FileKey, LookupMode, UddpBuilder, UddpReader, xxh64_virtual_path};

#[derive(Parser)]
struct Args {
    /// Existing package whose logical files must be preserved.
    #[arg(long)]
    package: PathBuf,
    /// Freshly rebuilt tilemeta package from the matching original CC/EC bundle.
    #[arg(long)]
    source: PathBuf,
    /// New output file; existing files are refused.
    #[arg(long)]
    output: PathBuf,
}

fn source_ref(reference: &TileMetaItemTextureRef) -> (u32, u8, u8, u32, u8, u32, u32, u8, u8) {
    (reference.texture_id, reference.block_index, reference.item_index,
        reference.texture_stretch.to_bits(), reference.source_kind, reference.sampler_index, reference.uv_set,
        reference.texture_type, reference.physical_package)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let original = UddpReader::load(&args.package)?;
    let rebuilt = UddpReader::load(&args.source)?;
    let source = TileMetaPackage::from_uddp_package(rebuilt.clone())?;
    ensure!(original.lookup_mode() == LookupMode::VirtualPathHash, "expected a path-addressed tilemeta package");
    let material_hash = xxh64_virtual_path(TILEMETA_ITEM_MATERIAL_ENTRY_PATH);
    let material_bytes = rebuilt.read_file_by_path_hash(material_hash)?;
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    for record in original.records() {
        let FileKey::PathHash(hash) = record.key else { unreachable!() };
        if hash == material_hash { continue; }
        let bytes = original.read_file_by_path_hash(hash)?;
        builder.add_file(AddFileRequest {
            data_type: udd_container::unpack_type(record.locator.meta32),
            compression: CompressionFlag::ZstdNoDict, width: 0, height: 0,
            virtual_path: None, path_hash64: Some(hash), id: None, data: &bytes,
        })?;
    }
    builder.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8, compression: CompressionFlag::ZstdNoDict,
        width: 0, height: 0, virtual_path: Some(TILEMETA_ITEM_MATERIAL_ENTRY_PATH),
        path_hash64: None, id: None, data: &material_bytes,
    })?;
    let bytes = builder.build()?;
    let candidate = TileMetaPackage::from_uddp_package(UddpReader::open(bytes.clone())?)?;
    let mut mismatches = Vec::new();
    let mut checked = 0usize;
    for item in source.item_tiles() {
        if source.item_materials(item.tile_id).is_empty() { continue; }
        checked += 1;
        if !source.item_texture_refs(item.tile_id).iter().map(source_ref)
            .eq(candidate.item_texture_refs(item.tile_id).iter().map(source_ref)) {
            mismatches.push(item.tile_id);
        }
    }
    let details = mismatches.iter().take(3).map(|id| format!(
        "item {id}: source={:?}, existing={:?}",
        source.item_texture_refs(*id).iter().map(source_ref).collect::<Vec<_>>(),
        candidate.item_texture_refs(*id).iter().map(source_ref).collect::<Vec<_>>(),
    )).collect::<Vec<_>>().join("\n");
    ensure!(mismatches.is_empty(), "source sampler refs disagree for {} items; first IDs: {:?}\n{}",
        mismatches.len(), &mismatches[..mismatches.len().min(20)], details);
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&args.output)?;
    output.write_all(&bytes)?;
    println!("Validated source refs for {checked} items; preserved existing payloads and wrote {}", args.output.display());
    Ok(())
}
