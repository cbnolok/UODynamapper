//! An opened client executable: its bytes, its inspection and the manifest next to it, and the descriptions the
//! Executable section shows.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use uocf::client_exe::classic_facets::{ClassicFileRole, CLASSIC_FACET_RECORD_SIZE};
use uocf::client_exe::facet_dimensions::{
    ClientExeInspection, ClientFamily, FacetTable, DISABLED_SPLIT_X, FACET_RECORD_SIZE, STOCK_SPLIT_X,
};
use uocf::client_exe::patch::{
    plan_patch_inspected, read_manifest, write_patched_client_exe, ExeEdit, ExeManifest, ExePatch, PatchPlan, PatchedClientExe,
};

/// The manifest found next to the opened file.
#[derive(Debug, Clone)]
pub enum ManifestState {
    None,
    /// It records this file: a later patch continues its chain.
    Current(ExeManifest),
    /// It records another file (the copy was replaced): it is ignored.
    Stale(ExeManifest),
    Unreadable(String),
}

/// A client executable read and inspected. The file is only read.
#[derive(Debug, Clone)]
pub struct OpenedExe {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub inspection: ClientExeInspection,
    pub manifest: ManifestState,
}

impl OpenedExe {
    /// Reads and inspects `path` and its manifest. Fails when the file is not a client the library takes (a 64-bit
    /// image, no facet table, ...).
    pub fn open(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        let inspection = ClientExeInspection::inspect(&bytes).wrap_err_with(|| format!("inspect {}", path.display()))?;
        let manifest = match read_manifest(path) {
            Ok(None) => ManifestState::None,
            Ok(Some(manifest)) if manifest.patched_sha256 == inspection.sha256 => ManifestState::Current(manifest),
            Ok(Some(manifest)) => ManifestState::Stale(manifest),
            Err(error) => ManifestState::Unreadable(format!("{error:#}")),
        };
        Ok(Self { path: path.to_path_buf(), bytes, inspection, manifest })
    }

    /// The manifest that continues the chain, when the file is a patched copy with one.
    pub fn current_manifest(&self) -> Option<&ExeManifest> {
        match &self.manifest {
            ManifestState::Current(manifest) => Some(manifest),
            _ => None,
        }
    }

    /// Why any patch of this file is refused (empty when it can be patched).
    pub fn blockers(&self) -> Vec<String> {
        let mut blockers = Vec::new();
        if self.inspection.pe.is_signed() {
            blockers.push("Authenticode signed: a patch would break the signature".to_owned());
        }
        if !self.inspection.is_stock() && self.current_manifest().is_none() {
            blockers.push(
                "the facet values are not the stock ones and no manifest of an earlier patch records them".to_owned(),
            );
        }
        blockers
    }

    /// The dry run of `patch` on the bytes read.
    pub fn plan(&self, patch: &ExePatch) -> eyre::Result<PatchPlan> {
        plan_patch_inspected(&self.bytes, &self.inspection, patch, self.current_manifest())
    }

    /// Writes the patched copy `output` and its manifest (see [`write_patched_client_exe`]: a new file outside the
    /// folder of this one).
    pub fn write_copy(&self, output: &Path, patch: &ExePatch) -> eyre::Result<PatchedClientExe> {
        write_patched_client_exe(&self.path, output, patch)
    }

    /// The folder of the file: the default client data folder.
    pub fn folder(&self) -> PathBuf {
        self.path.parent().map(Path::to_path_buf).unwrap_or_default()
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().unwrap_or_default().to_string_lossy().into_owned()
    }
}

/// "split" for the Enhanced Client, "wrap seam" for Classic.
pub fn seam_word(family: ClientFamily) -> &'static str {
    match family {
        ClientFamily::Classic => "wrap seam",
        ClientFamily::Enhanced | ClientFamily::KingdomReborn => "split",
    }
}

/// What a split or seam x means.
pub fn split_x_label(x: u32) -> &'static str {
    match x {
        STOCK_SPLIT_X => "stock",
        DISABLED_SPLIT_X => "disabled",
        _ => "moved",
    }
}

/// The split or seam of `inspection`, in words.
pub fn seam_status(inspection: &ClientExeInspection) -> String {
    match (&inspection.split, inspection.family) {
        (_, ClientFamily::KingdomReborn) => "none: Kingdom Reborn has no Felucca/Trammel split".to_owned(),
        (None, ClientFamily::Enhanced) => format!(
            "not found: no x = {STOCK_SPLIT_X} split sequence (the split code has another shape: check it by hand)"
        ),
        (None, ClientFamily::Classic) => format!("not found: no x = {STOCK_SPLIT_X} wrap seam compares"),
        (Some(split), family) => {
            let offsets = split.offsets.iter().map(|offset| format!("{offset:#x}")).collect::<Vec<_>>().join(", ");
            format!(
                "{} at x = {} ({}), {} imm32 at file {offsets}",
                seam_word(family),
                split.x,
                split_x_label(split.x),
                split.offsets.len()
            )
        }
    }
}

/// Where each table copy lies.
pub fn table_locations(inspection: &ClientExeInspection) -> Vec<String> {
    let copies = inspection.tables.len();
    inspection
        .tables
        .iter()
        .enumerate()
        .map(|(index, table)| {
            let copy = match (copies, index) {
                (1, _) => String::new(),
                (_, 0) => "copy 1 (default): ".to_owned(),
                _ => format!("copy {} (0xA9 flag 0x8000): ", index + 1),
            };
            format!("{copy}{}", describe_table(table))
        })
        .collect()
}

fn describe_table(table: &FacetTable) -> String {
    let layout = if table.record_size == CLASSIC_FACET_RECORD_SIZE {
        "{name, width, height, wrap, 6 file ids, felucca_rules}"
    } else {
        debug_assert_eq!(table.record_size, FACET_RECORD_SIZE);
        "{name pointer, width, height}"
    };
    format!(
        "{} records of {} bytes {layout} in {}, file {:#x}, VA {:#x}",
        table.records.len(),
        table.record_size,
        table.section,
        table.offset,
        table.va
    )
}

/// The PE checksum, in words.
pub fn checksum_status(inspection: &ClientExeInspection) -> String {
    let state = if inspection.checksum_valid() { "valid" } else { "invalid (the client does not verify it)" };
    format!(
        "{:#010x} at {:#x}, computed {:#010x}: {state}",
        inspection.pe.stored_checksum, inspection.pe.checksum_offset, inspection.computed_checksum
    )
}

/// One line per manifest step.
pub fn manifest_steps(manifest: &ExeManifest) -> Vec<String> {
    manifest
        .steps
        .iter()
        .enumerate()
        .map(|(number, step)| {
            let edits = step.edits.iter().map(ExeEdit::describe).collect::<Vec<_>>().join("; ");
            let edits = if edits.is_empty() { "(edits not recorded)".to_owned() } else { edits };
            format!("step {}: {edits}", number + 1)
        })
        .collect()
}

/// Classic: the decoded file ids of facet `facet`, one line per table copy (raw ids when the file name table was not
/// located), and its Felucca-rules flag. `None` in the other families.
pub fn classic_files(inspection: &ClientExeInspection, facet: usize) -> Option<(Vec<String>, u32)> {
    let names = inspection.classic.as_ref()?.file_names.as_ref();
    let mut flag = None;
    let lines = inspection
        .tables
        .iter()
        .filter_map(|table| table.records.get(facet)?.classic)
        .map(|record| {
            flag = Some(record.felucca_rules);
            ClassicFileRole::ALL
                .iter()
                .zip(record.file_ids)
                .map(|(role, id)| match names.and_then(|names| names.name(id)) {
                    Some(file) => format!("{} {file}", role.label()),
                    None => format!("{} #{id}", role.label()),
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect();
    Some((lines, flag?))
}
