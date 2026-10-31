//! Patching a copy of a client executable. A patch is a list of edits ([`ExeEdit`]) applied in one step: the whole
//! set is validated against the inspected executable ([`ClientExeInspection`]) first, then planned as exact byte
//! changes ([`PatchPlan`]: offset, old and new bytes and the edit that causes each, which is the dry run), then
//! applied, the PE checksum recomputed and the copy inspected again: the tables, split or seam and Classic startup
//! globals must be found at the same places with the planned values.
//!
//! Safety rules: the copy goes to a new path outside the input's folder; signed images are refused (and 64-bit ones,
//! which [`ClientExeInspection::inspect`] does not take); sizes the client cannot hold are refused
//! ([`validate_facet_size`], [`validate_classic_size`], [`validate_classic_wrap`]). The input must have the stock
//! values, or the values the manifest of the copy it is (`<copy>.facet-dims.json`, [`ExeManifest`]) records. The
//! manifest keeps the chain: the stock build's SHA-256, and per step the source, the result and the edits.
//!
//! What each edit writes:
//!
//! - [`ExeEdit::FacetSize`]: the width and height of the facet's record in every table copy. Classic: a wrap width
//!   or height equal to the old side follows the new side (unless the patch has a [`ExeEdit::ClassicWrap`] for the
//!   facet), and facet 0 also writes the startup globals.
//! - [`ExeEdit::Split`]: the three split imm32 (Enhanced Client) or the two wrap seam imm32 (Classic).
//! - [`ExeEdit::ClassicWrap`]: the facet's wrap in every table copy (and the startup globals for facet 0).

crate::eyre_imports!();
use std::path::{Path, PathBuf};

use crate::client_exe::classic_facets::{
    validate_classic_size, validate_classic_wrap, CLASSIC_RECORD_WRAP_HEIGHT_OFFSET, CLASSIC_RECORD_WRAP_WIDTH_OFFSET,
};
use crate::client_exe::facet_dimensions::{
    validate_facet_size, ClientExeInspection, ClientFamily, ExeValues, FacetValues, DISABLED_SPLIT_X,
    FACET_RECORD_HEIGHT_OFFSET, FACET_RECORD_WIDTH_OFFSET, SPLIT_FACETS, STOCK_SPLIT_X,
};
use crate::client_exe::pe::pe_checksum;
use crate::enhanced::facet_sector::FacetGeometry;

/// The manifest of a patched copy: `<copy>` plus this suffix.
pub const MANIFEST_SUFFIX: &str = ".facet-dims.json";
pub const MANIFEST_FORMAT: &str = "uocf-facet-dims";
/// Version 2: the family and the steps with their edits. Version 1 (one facet size and split per patch, no steps)
/// is still read.
pub const MANIFEST_VERSION: u32 = 2;
pub const MANIFEST_VERSION_1: u32 = 1;
const PATCH_FIELD_SIZE: usize = size_of::<u32>();

/// A new split x.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitEdit {
    /// Moves the Enhanced Client split of facets 0 and 1, or the Classic wrap seam, to this x.
    Set(u32),
    /// Sets it to [`DISABLED_SPLIT_X`].
    Disable,
}

impl SplitEdit {
    pub fn x(self) -> u32 {
        match self {
            Self::Set(x) => x,
            Self::Disable => DISABLED_SPLIT_X,
        }
    }
}

/// One change of a patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExeEdit {
    /// A new size for one facet.
    FacetSize { facet: u8, width: u32, height: u32 },
    Split(SplitEdit),
    /// A new wrap rectangle for one facet (Classic only).
    ClassicWrap { facet: u8, wrap_width: u32, wrap_height: u32 },
    // Planned with the login and encryption research: `LoginAddress`, `LoginPort`, `Encryption`. Each is one more
    // variant whose validation and byte changes go in `plan_patch`; the plan, the change list and the manifest steps
    // keep their shape.
}

impl ExeEdit {
    pub fn describe(&self) -> String {
        match self {
            Self::FacetSize { facet, width, height } => format!("facet {facet} size {width}x{height}"),
            Self::Split(SplitEdit::Set(x)) => format!("split x {x}"),
            Self::Split(SplitEdit::Disable) => format!("split disabled ({DISABLED_SPLIT_X})"),
            Self::ClassicWrap { facet, wrap_width, wrap_height } => {
                format!("facet {facet} wrap {wrap_width}x{wrap_height}")
            }
        }
    }
}

/// The edits written in one step.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExePatch {
    pub edits: Vec<ExeEdit>,
}

/// Why a byte range changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeCause {
    /// The edit at this index of [`ExePatch::edits`].
    Edit(usize),
    /// The recomputed PE checksum.
    Checksum,
}

/// One changed field of the executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteChange {
    /// File offset and virtual address (`None` for the header's checksum, outside any section).
    pub offset: usize,
    pub va: Option<u32>,
    /// What the bytes are, e.g. "facet 0 (Felucca) width, table 2".
    pub field: String,
    pub old: Vec<u8>,
    pub new: Vec<u8>,
    pub cause: ChangeCause,
}

/// A validated patch: the byte changes it makes, in edit order with the checksum last.
#[derive(Debug, Clone)]
pub struct PatchPlan {
    pub before: ClientExeInspection,
    pub patch: ExePatch,
    pub changes: Vec<ByteChange>,
    /// Allowed but unusual requests.
    pub warnings: Vec<String>,
    /// The values of the patched copy.
    pub expected: ExeValues,
    /// The chain: the stock build the copy derives from and the earlier steps (empty for a stock input).
    pub original_sha256: String,
    pub build: Option<String>,
    pub previous_steps: Vec<ManifestStep>,
}

impl PatchPlan {
    /// Applies the changes to `bytes`, which must be the planned input.
    pub fn apply(&self, bytes: &[u8]) -> eyre::Result<Vec<u8>> {
        eyre::ensure!(bytes.len() == self.before.file_size, "not the planned executable: its size differs");
        let mut patched = bytes.to_vec();
        for change in &self.changes {
            let range = change.offset..change.offset + change.old.len();
            eyre::ensure!(patched[range.clone()] == change.old[..], "not the planned executable: {} differs", change.field);
            patched[range].copy_from_slice(&change.new);
        }
        Ok(patched)
    }
}

/// The record written next to a patched copy (`<copy>.facet-dims.json`): the stock build it derives from, the
/// build it was patched from and the result, the values of the result and every step. A later patch of the copy
/// needs it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExeManifest {
    pub format: String,
    pub version: u32,
    /// `None` in a version 1 manifest.
    #[serde(default)]
    pub family: Option<ClientFamily>,
    /// The known build the chain started from, when it is one.
    pub build: Option<String>,
    /// SHA-256 of the stock executable the chain of patches started from.
    pub original_sha256: String,
    /// SHA-256 of the executable this patch was applied to.
    pub source_sha256: String,
    /// SHA-256 of the patched copy.
    pub patched_sha256: String,
    pub values: ExeValues,
    /// The PE checksum written.
    pub checksum: u32,
    /// Every write of the chain, the last one this copy's. A version 1 manifest has one step without edits.
    #[serde(default)]
    pub steps: Vec<ManifestStep>,
}

/// One write: its input and output and the edits it applied.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ManifestStep {
    pub source_sha256: String,
    pub patched_sha256: String,
    /// Empty for a step read from a version 1 manifest.
    pub edits: Vec<ExeEdit>,
}

/// A patched executable.
#[derive(Debug, Clone)]
pub struct PatchedClientExe {
    pub bytes: Vec<u8>,
    pub before: ClientExeInspection,
    pub after: ClientExeInspection,
    pub changes: Vec<ByteChange>,
    pub warnings: Vec<String>,
    pub manifest: ExeManifest,
}

/// A field write of a plan, before comparing it with the input.
struct FieldWrite {
    offset: usize,
    value: u32,
    field: String,
    edit: usize,
}

/// Which edit set each side of a Classic wrap, when it changes.
#[derive(Clone, Copy, Default)]
struct WrapCause {
    width: Option<usize>,
    height: Option<usize>,
}

/// The facet of an edit, checked against the table.
fn checked_facet(facet: u8, count: usize) -> eyre::Result<usize> {
    let index = usize::from(facet);
    eyre::ensure!(index < count, "facet {facet}: the table has facets 0..{count}");
    Ok(index)
}

/// Validates `patch` against the executable `bytes` and plans its byte changes, without writing anything (the dry
/// run). `previous` is the manifest of the input when it is an earlier patched copy.
pub fn plan_patch(bytes: &[u8], patch: &ExePatch, previous: Option<&ExeManifest>) -> eyre::Result<PatchPlan> {
    let before = ClientExeInspection::inspect(bytes)?;
    eyre::ensure!(!before.pe.is_signed(), "the executable is Authenticode signed; a patch would break the signature");
    eyre::ensure!(!patch.edits.is_empty(), "the patch has no edits");
    let previous = previous.filter(|manifest| manifest.patched_sha256 == before.sha256);
    let (original_sha256, build, previous_steps) = match previous {
        Some(manifest) => {
            eyre::ensure!(
                manifest.values == before.values(),
                "the executable's facet values differ from the ones its manifest records"
            );
            (manifest.original_sha256.clone(), manifest.build.clone(), manifest.steps.clone())
        }
        None => {
            eyre::ensure!(
                before.is_stock(),
                "the executable's facet values are not the stock ones and no manifest of an earlier patch records them"
            );
            (before.sha256.clone(), before.known.map(|build| build.name.to_owned()), Vec::new())
        }
    };
    let count = before.facet_count();
    let classic = before.family == ClientFamily::Classic;
    let startup = before.classic.as_ref().and_then(|layout| layout.startup);
    let mut warnings = Vec::new();
    let mut expected = before.values();
    let mut sized = vec![None; count];
    let mut wrapped = vec![None; count];
    let mut split_edit = None;
    for (index, edit) in patch.edits.iter().enumerate() {
        match *edit {
            ExeEdit::FacetSize { facet, width, height } => {
                let facet_index = checked_facet(facet, count)?;
                eyre::ensure!(sized[facet_index].is_none(), "two size edits for facet {facet}");
                sized[facet_index] = Some(index);
                let size_warnings =
                    if classic { validate_classic_size(width, height)? } else { validate_facet_size(width, height)? };
                warnings.extend(size_warnings);
            }
            ExeEdit::Split(split) => {
                eyre::ensure!(split_edit.is_none(), "two split edits");
                let x = split.x();
                eyre::ensure!((1..=DISABLED_SPLIT_X).contains(&x), "split x {x} is outside 1..={DISABLED_SPLIT_X}");
                eyre::ensure!(
                    before.split.is_some(),
                    "{}",
                    match before.family {
                        ClientFamily::Classic => "no wrap seam located in this build".to_owned(),
                        _ => format!("no x = {STOCK_SPLIT_X} split sequence found in this build"),
                    }
                );
                split_edit = Some((index, x));
            }
            ExeEdit::ClassicWrap { facet, .. } => {
                eyre::ensure!(classic, "a wrap edit needs a Classic client, not {}", before.family.label());
                let facet_index = checked_facet(facet, count)?;
                eyre::ensure!(wrapped[facet_index].is_none(), "two wrap edits for facet {facet}");
                wrapped[facet_index] = Some(index);
            }
        }
        let facet = match *edit {
            ExeEdit::FacetSize { facet, .. } | ExeEdit::ClassicWrap { facet, .. } => Some(facet),
            ExeEdit::Split(_) => None,
        };
        if classic && facet == Some(0) {
            eyre::ensure!(
                startup.is_some(),
                "the startup copy of facet 0 (the wrap and size globals SetFacet fills) was not located: facet 0 cannot \
                 be patched"
            );
        }
    }

    // The values the copy will have, and which edit sets each Classic wrap side.
    let mut wrap_causes = vec![WrapCause::default(); count];
    for (facet, values) in expected.facets.iter_mut().enumerate() {
        let (old_width, old_height) = (values.width, values.height);
        if let Some(index) = sized[facet] {
            let ExeEdit::FacetSize { width, height, .. } = patch.edits[index] else { unreachable!() };
            (values.width, values.height) = (width, height);
            if classic && wrapped[facet].is_none() {
                if values.wrap_width == Some(old_width) && width != old_width {
                    (values.wrap_width, wrap_causes[facet].width) = (Some(width), Some(index));
                }
                if values.wrap_height == Some(old_height) && height != old_height {
                    (values.wrap_height, wrap_causes[facet].height) = (Some(height), Some(index));
                }
            }
        }
        if let Some(index) = wrapped[facet] {
            let ExeEdit::ClassicWrap { wrap_width, wrap_height, .. } = patch.edits[index] else { unreachable!() };
            (values.wrap_width, values.wrap_height) = (Some(wrap_width), Some(wrap_height));
            wrap_causes[facet] = WrapCause { width: Some(index), height: Some(index) };
        }
        if let (Some(wrap_width), Some(wrap_height)) = (values.wrap_width, values.wrap_height)
            && (sized[facet].is_some() || wrapped[facet].is_some())
        {
            let size = FacetGeometry::new(values.width, values.height);
            validate_classic_wrap(FacetGeometry::new(wrap_width, wrap_height), size)
                .wrap_err_with(|| match wrapped[facet] {
                    Some(_) => format!("facet {facet}"),
                    None => format!("facet {facet}: the new size cuts its wrap; add a wrap edit"),
                })?;
        }
    }
    if let Some((_, x)) = split_edit {
        expected.split_x = Some(x);
    }
    warnings.extend(split_warnings(&before, &expected, &sized));

    let mut writes = Vec::new();
    for (facet, values) in expected.facets.iter().enumerate() {
        let label = |what: &str, table: usize| {
            let copy = if before.tables.len() > 1 { format!(", table {}", table + 1) } else { String::new() };
            format!("facet {facet} ({}) {what}{copy}", values.name)
        };
        let causes = wrap_causes[facet];
        let record_fields = [
            (sized[facet], FACET_RECORD_WIDTH_OFFSET, Some(values.width), "width"),
            (sized[facet], FACET_RECORD_HEIGHT_OFFSET, Some(values.height), "height"),
            (causes.width, CLASSIC_RECORD_WRAP_WIDTH_OFFSET, values.wrap_width, "wrap width"),
            (causes.height, CLASSIC_RECORD_WRAP_HEIGHT_OFFSET, values.wrap_height, "wrap height"),
        ];
        for (table_index, table) in before.tables.iter().enumerate() {
            let record = table.record_offset(facet);
            for (cause, field_offset, value, what) in record_fields {
                if let (Some(edit), Some(value)) = (cause, value) {
                    let field = label(what, table_index);
                    writes.push(FieldWrite { offset: record + field_offset, value, field, edit });
                }
            }
        }
        if facet == 0
            && let Some(startup) = startup
        {
            let [wrap_width, wrap_height, width, height] = startup.field_offsets();
            let startup_fields = [
                (causes.width, wrap_width, values.wrap_width, "startup wrap width"),
                (causes.height, wrap_height, values.wrap_height, "startup wrap height"),
                (sized[facet], width, Some(values.width), "startup width"),
                (sized[facet], height, Some(values.height), "startup height"),
            ];
            for (cause, offset, value, what) in startup_fields {
                if let (Some(edit), Some(value)) = (cause, value) {
                    writes.push(FieldWrite { offset, value, field: format!("facet 0 ({}) {what}", values.name), edit });
                }
            }
        }
    }
    if let (Some((edit, x)), Some(split)) = (split_edit, &before.split) {
        let what = if classic { "wrap seam" } else { "split" };
        for (number, offset) in split.offsets.iter().enumerate() {
            let field = format!("{what} x, compare {}", number + 1);
            writes.push(FieldWrite { offset: *offset, value: x, field, edit });
        }
    }

    let mut patched = bytes.to_vec();
    let mut changes = Vec::new();
    for write in writes {
        let range = write.offset..write.offset + PATCH_FIELD_SIZE;
        let new = write.value.to_le_bytes();
        if bytes[range.clone()] == new {
            continue;
        }
        patched[range.clone()].copy_from_slice(&new);
        changes.push(ByteChange {
            offset: write.offset,
            va: before.pe.offset_to_va(write.offset),
            field: write.field,
            old: bytes[range].to_vec(),
            new: new.to_vec(),
            cause: ChangeCause::Edit(write.edit),
        });
    }
    eyre::ensure!(!changes.is_empty(), "the patch changes nothing: the executable already has these values");
    let checksum_offset = before.pe.checksum_offset;
    let checksum = pe_checksum(&patched, checksum_offset).to_le_bytes();
    let range = checksum_offset..checksum_offset + PATCH_FIELD_SIZE;
    if bytes[range.clone()] != checksum {
        changes.push(ByteChange {
            offset: checksum_offset,
            va: None,
            field: "PE checksum".to_owned(),
            old: bytes[range].to_vec(),
            new: checksum.to_vec(),
            cause: ChangeCause::Checksum,
        });
    }
    Ok(PatchPlan { before, patch: patch.clone(), changes, warnings, expected, original_sha256, build, previous_steps })
}

/// The warnings about the split or seam of the patched values.
fn split_warnings(before: &ClientExeInspection, expected: &ExeValues, sized: &[Option<usize>]) -> Vec<String> {
    let split_facets = || expected.facets.iter().filter(|facet| SPLIT_FACETS.contains(&facet.facet));
    let mut warnings = Vec::new();
    match (before.family, expected.split_x) {
        (ClientFamily::Enhanced, None) if split_facets().any(|facet| sized[usize::from(facet.facet)].is_some()) => {
            warnings.push(format!(
                "no x = {STOCK_SPLIT_X} split sequence found: in an Enhanced Client build, the split code differs \
                 from the known sequence"
            ));
        }
        (ClientFamily::Classic, Some(x)) => {
            for facet in split_facets() {
                let FacetValues { width, wrap_width, .. } = facet;
                let wrap = wrap_width.unwrap_or(*width);
                if wrap.min(*width) != x.min(*width) {
                    warnings.push(format!(
                        "the wrap seam x {x} is not facet {} ({})'s wrap width {wrap}: the radar and the walk test \
                         assume the facet stops wrapping at the seam",
                        facet.facet, facet.name
                    ));
                }
            }
        }
        (_, Some(x)) if x != DISABLED_SPLIT_X => {
            for facet in split_facets().filter(|facet| x >= facet.width) {
                warnings.push(format!(
                    "split x {x} is not inside facet {} ({})'s width {}: nothing is split",
                    facet.facet, facet.name, facet.width
                ));
            }
        }
        _ => {}
    }
    warnings
}

/// Patches a copy of the executable `bytes` with every edit of `patch` and recomputes the PE checksum (see
/// [`plan_patch`]); only the planned bytes change, and the copy is inspected again.
pub fn patch_client_exe(bytes: &[u8], patch: &ExePatch, previous: Option<&ExeManifest>) -> eyre::Result<PatchedClientExe> {
    let plan = plan_patch(bytes, patch, previous)?;
    let patched = plan.apply(bytes)?;
    let changed = |offset: usize| {
        plan.changes.iter().any(|change| (change.offset..change.offset + change.new.len()).contains(&offset))
    };
    let stray = bytes.iter().zip(&patched).enumerate().find(|(offset, (old, new))| old != new && !changed(*offset));
    eyre::ensure!(stray.is_none(), "internal error: byte {:?} changed outside the patched fields", stray.map(|(offset, _)| offset));
    let before = plan.before;
    let after = ClientExeInspection::inspect(&patched)?;
    eyre::ensure!(after.checksum_valid(), "internal error: the written checksum does not verify");
    let offsets = |inspection: &ClientExeInspection| inspection.tables.iter().map(|table| table.offset).collect::<Vec<_>>();
    let split_offsets = |inspection: &ClientExeInspection| inspection.split.as_ref().map(|split| split.offsets.clone());
    let startup = |inspection: &ClientExeInspection| {
        inspection.classic.as_ref().and_then(|layout| layout.startup).map(|startup| startup.offset)
    };
    eyre::ensure!(
        after.family == before.family
            && offsets(&after) == offsets(&before)
            && split_offsets(&after) == split_offsets(&before)
            && startup(&after) == startup(&before),
        "internal error: the patched copy's tables, split or startup values are located elsewhere"
    );
    eyre::ensure!(after.values() == plan.expected, "internal error: the patched copy does not have the planned values");
    let facet_0_edited = plan.patch.edits.iter().any(|edit| {
        matches!(edit, ExeEdit::FacetSize { facet: 0, .. } | ExeEdit::ClassicWrap { facet: 0, .. })
    });
    if let Some(startup) = after.classic.as_ref().and_then(|layout| layout.startup)
        && facet_0_edited
    {
        let felucca = &plan.expected.facets[0];
        eyre::ensure!(
            startup.size == FacetGeometry::new(felucca.width, felucca.height)
                && Some(startup.wrap.width) == felucca.wrap_width
                && Some(startup.wrap.height) == felucca.wrap_height,
            "internal error: the patched copy's startup values are not facet 0's"
        );
    }
    let mut steps = plan.previous_steps;
    steps.push(ManifestStep {
        source_sha256: before.sha256.clone(),
        patched_sha256: after.sha256.clone(),
        edits: plan.patch.edits.clone(),
    });
    let manifest = ExeManifest {
        format: MANIFEST_FORMAT.to_owned(),
        version: MANIFEST_VERSION,
        family: Some(after.family),
        build: plan.build,
        original_sha256: plan.original_sha256,
        source_sha256: before.sha256.clone(),
        patched_sha256: after.sha256.clone(),
        values: after.values(),
        checksum: after.pe.stored_checksum,
        steps,
    };
    Ok(PatchedClientExe { bytes: patched, before, after, changes: plan.changes, warnings: plan.warnings, manifest })
}

/// The manifest path of the patched copy `exe`: its file name plus [`MANIFEST_SUFFIX`].
pub fn manifest_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(MANIFEST_SUFFIX);
    exe.with_file_name(name)
}

/// Parses a manifest of version 1 or [`MANIFEST_VERSION`]. A version 1 manifest gets its one step, without edits.
pub fn parse_manifest(text: &str) -> eyre::Result<ExeManifest> {
    let mut manifest: ExeManifest = serde_json::from_str(text)?;
    eyre::ensure!(
        manifest.format == MANIFEST_FORMAT && [MANIFEST_VERSION_1, MANIFEST_VERSION].contains(&manifest.version),
        "not a {MANIFEST_FORMAT} version {MANIFEST_VERSION_1} or {MANIFEST_VERSION} manifest"
    );
    if manifest.version == MANIFEST_VERSION_1 && manifest.steps.is_empty() {
        manifest.steps.push(ManifestStep {
            source_sha256: manifest.source_sha256.clone(),
            patched_sha256: manifest.patched_sha256.clone(),
            edits: Vec::new(),
        });
    }
    Ok(manifest)
}

/// The manifest of `exe`, when it has one.
pub fn read_manifest(exe: &Path) -> eyre::Result<Option<ExeManifest>> {
    let path = manifest_path(exe);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).wrap_err_with(|| format!("read {}", path.display()))?;
    parse_manifest(&text).wrap_err_with(|| format!("parse {}", path.display())).map(Some)
}

/// Fails unless `output` is a new file outside the folder of `input` (the client folder), and its manifest path is
/// free too.
fn check_output_path(input: &Path, output: &Path) -> eyre::Result<()> {
    let input = std::fs::canonicalize(input).wrap_err_with(|| format!("resolve {}", input.display()))?;
    let client_dir = input.parent().ok_or_else(|| eyre!("{} has no folder", input.display()))?;
    for path in [output.to_path_buf(), manifest_path(output)] {
        eyre::ensure!(path.symlink_metadata().is_err(), "{} exists: write to a new path", path.display());
    }
    let output_dir = match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let output_dir =
        std::fs::canonicalize(output_dir).wrap_err_with(|| format!("resolve the folder of {}", output.display()))?;
    eyre::ensure!(
        !output_dir.starts_with(client_dir),
        "refusing to write inside the folder of the input executable ({}): write the copy elsewhere",
        client_dir.display()
    );
    Ok(())
}

/// Plans `patch` on `input` (and its manifest, when it is an earlier patched copy) without writing anything.
pub fn dry_run_client_exe(input: &Path, patch: &ExePatch) -> eyre::Result<PatchPlan> {
    let bytes = std::fs::read(input).wrap_err_with(|| format!("read {}", input.display()))?;
    let previous = read_manifest(input)?;
    plan_patch(&bytes, patch, previous.as_ref())
}

/// Patches a copy of `input` to the new file `output` (outside the input's folder) and writes its manifest
/// `<output>.facet-dims.json`. The input is only read; when it is an earlier patched copy, its manifest is read too.
pub fn write_patched_client_exe(input: &Path, output: &Path, patch: &ExePatch) -> eyre::Result<PatchedClientExe> {
    check_output_path(input, output)?;
    let bytes = std::fs::read(input).wrap_err_with(|| format!("read {}", input.display()))?;
    let previous = read_manifest(input)?;
    let patched = patch_client_exe(&bytes, patch, previous.as_ref())?;
    let manifest = serde_json::to_string_pretty(&patched.manifest)?;
    std::fs::write(output, &patched.bytes).wrap_err_with(|| format!("write {}", output.display()))?;
    let manifest_file = manifest_path(output);
    std::fs::write(&manifest_file, manifest).wrap_err_with(|| format!("write {}", manifest_file.display()))?;
    Ok(patched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_exe::classic_facets::tests::{seam_imm_offsets, stock_size, synthetic_classic};
    use crate::client_exe::facet_dimensions::tests::{synthetic_client, synthetic_kr_client, NEW_SPLIT};
    use crate::client_exe::facet_dimensions::{FACET_SIDE_ALIGNMENT, MAX_FACET_SIDE, STOCK_FACET_COUNT};
    use crate::client_exe::sha256::sha256_hex;
    use crate::enhanced::facet_sector::CLIENT_FACET_DIMENSIONS;

    const NEW_WIDTH: u32 = 8192;
    const NEW_HEIGHT: u32 = 4160;
    /// Not a multiple of 64: one warning.
    const PADDED_HEIGHT: u32 = 1608;
    const CLASSIC_NEW_WIDTH: u32 = 8192;
    const CLASSIC_ILSHENAR_WIDTH: u32 = 2400;
    const CLASSIC_TRAMMEL_WRAP: u32 = 6144;

    fn size(facet: u8, width: u32, height: u32) -> ExeEdit {
        ExeEdit::FacetSize { facet, width, height }
    }

    /// Every byte that differs between `bytes` and `patched` lies in a change of `plan`, and the plan applied gives
    /// `patched`.
    fn assert_plan_is_the_write(bytes: &[u8], plan: &PatchPlan, patched: &[u8]) {
        assert_eq!(plan.apply(bytes).unwrap(), patched);
        for (offset, (old, new)) in bytes.iter().zip(patched).enumerate() {
            let planned = plan.changes.iter().any(|change| (change.offset..change.offset + change.new.len()).contains(&offset));
            assert!(old == new || planned, "byte {offset:#x} changed outside the plan");
        }
        assert!(plan.changes.iter().all(|change| change.old != change.new));
    }

    #[test]
    fn a_multi_edit_patch_writes_exactly_its_plan() {
        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let patch = ExePatch { edits: vec![size(0, NEW_WIDTH, NEW_HEIGHT), size(2, 2304, PADDED_HEIGHT), ExeEdit::Split(SplitEdit::Set(NEW_SPLIT))] };
        let plan = plan_patch(&image, &patch, None).unwrap();
        let patched = patch_client_exe(&image, &patch, None).unwrap();
        assert_plan_is_the_write(&image, &plan, &patched.bytes);
        assert_eq!(plan.changes, patched.changes);
        assert_eq!(patched.after.dimensions()[0], FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
        assert_eq!(patched.after.dimensions()[2], FacetGeometry::new(2304, PADDED_HEIGHT));
        assert_eq!(patched.after.dimensions()[3..], CLIENT_FACET_DIMENSIONS[3..]);
        assert_eq!(patched.after.split.as_ref().unwrap().x, NEW_SPLIT);
        assert!(patched.after.checksum_valid() && !patched.after.is_stock());
        assert_eq!(patched.warnings.len(), 1, "1608 is not a multiple of 64");
        // Width of 0, height of 2 (its width is unchanged), three split imm32, checksum.
        let causes: Vec<ChangeCause> = patched.changes.iter().map(|change| change.cause).collect();
        let edit = ChangeCause::Edit;
        assert_eq!(causes, [edit(0), edit(0), edit(1), edit(2), edit(2), edit(2), ChangeCause::Checksum]);
        assert_eq!(patched.manifest.original_sha256, sha256_hex(&image));
        assert_eq!(patched.manifest.patched_sha256, sha256_hex(&patched.bytes));
        assert_eq!(patched.manifest.steps.len(), 1);
        assert_eq!(patched.manifest.steps[0].edits, patch.edits);

        // A patched copy needs its manifest; with it, it patches again and keeps the chain.
        let again = ExePatch { edits: vec![size(6, 512, 512), ExeEdit::Split(SplitEdit::Disable)] };
        assert!(plan_patch(&patched.bytes, &again, None).is_err());
        let chained = patch_client_exe(&patched.bytes, &again, Some(&patched.manifest)).unwrap();
        assert_eq!(chained.manifest.original_sha256, patched.manifest.original_sha256);
        assert_eq!(chained.manifest.source_sha256, patched.manifest.patched_sha256);
        assert_eq!(chained.manifest.steps.len(), 2);
        assert_eq!(chained.after.split.unwrap().x, DISABLED_SPLIT_X);

        // Without split code, resizing facet 0 of an Enhanced Client warns that the split was not found; Kingdom
        // Reborn has none.
        let resize = ExePatch { edits: vec![size(0, NEW_WIDTH, NEW_HEIGHT)] };
        assert_eq!(patch_client_exe(&synthetic_client(None, false), &resize, None).unwrap().warnings.len(), 1);
        assert!(patch_client_exe(&synthetic_kr_client(), &resize, None).unwrap().warnings.is_empty());
    }

    #[test]
    fn bad_requests_and_builds_are_refused() {
        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let good = size(3, NEW_WIDTH, NEW_HEIGHT);
        for bad in [
            vec![],
            vec![size(STOCK_FACET_COUNT as u8, NEW_WIDTH, NEW_HEIGHT)],
            vec![size(3, MAX_FACET_SIDE + FACET_SIDE_ALIGNMENT, NEW_HEIGHT)],
            vec![size(3, NEW_WIDTH + FACET_SIDE_ALIGNMENT / 2, NEW_HEIGHT)],
            vec![size(3, 0, NEW_HEIGHT)],
            vec![size(3, MAX_FACET_SIDE, MAX_FACET_SIDE)],
            vec![good, size(3, NEW_WIDTH, NEW_WIDTH)],
            vec![ExeEdit::Split(SplitEdit::Set(DISABLED_SPLIT_X + 1))],
            vec![ExeEdit::Split(SplitEdit::Set(NEW_SPLIT)), ExeEdit::Split(SplitEdit::Disable)],
            vec![ExeEdit::ClassicWrap { facet: 3, wrap_width: 2048, wrap_height: 2048 }],
            vec![size(3, CLIENT_FACET_DIMENSIONS[3].width, CLIENT_FACET_DIMENSIONS[3].height)],
        ] {
            assert!(plan_patch(&image, &ExePatch { edits: bad.clone() }, None).is_err(), "{bad:?}");
        }
        assert!(plan_patch(&image, &ExePatch { edits: vec![good] }, None).is_ok());
        let split = ExePatch { edits: vec![ExeEdit::Split(SplitEdit::Disable)] };
        assert!(plan_patch(&synthetic_kr_client(), &split, None).is_err(), "no split to patch");
        let signed = synthetic_client(Some(STOCK_SPLIT_X), true);
        assert!(plan_patch(&signed, &ExePatch { edits: vec![good] }, None).is_err(), "signed");
    }

    #[test]
    fn a_classic_patch_writes_both_copies_the_startup_globals_and_the_seam() {
        let image = synthetic_classic(6, 2);
        let before = ClientExeInspection::inspect(&image).unwrap();
        let patch = ExePatch {
            edits: vec![
                size(0, CLASSIC_NEW_WIDTH, stock_size(0).height),
                size(2, CLASSIC_ILSHENAR_WIDTH, stock_size(2).height),
                ExeEdit::ClassicWrap { facet: 1, wrap_width: CLASSIC_TRAMMEL_WRAP, wrap_height: stock_size(1).height },
                ExeEdit::Split(SplitEdit::Set(CLASSIC_TRAMMEL_WRAP)),
            ],
        };
        let plan = plan_patch(&image, &patch, None).unwrap();
        let patched = patch_client_exe(&image, &patch, None).unwrap();
        assert_plan_is_the_write(&image, &plan, &patched.bytes);
        // Facet 0: width in 2 copies and the startup width; facet 2: width and the wrap width that followed it, in 2
        // copies; facet 1: wrap width in 2 copies; 2 seam imm32; checksum.
        let counts: Vec<usize> =
            (0..patch.edits.len()).map(|edit| plan.changes.iter().filter(|change| change.cause == ChangeCause::Edit(edit)).count()).collect();
        assert_eq!(counts, [3, 4, 2, 2]);
        let startup = before.classic.as_ref().unwrap().startup.unwrap();
        assert!(plan.changes.iter().any(|change| change.offset == startup.field_offsets()[2]));
        let seam: Vec<usize> = plan.changes.iter().filter(|change| change.cause == ChangeCause::Edit(3)).map(|change| change.offset).collect();
        assert_eq!(seam, seam_imm_offsets(&before));

        let after = &patched.after;
        for table in &after.tables {
            let ilshenar = &table.records[2];
            assert_eq!(ilshenar.geometry.width, CLASSIC_ILSHENAR_WIDTH);
            assert_eq!(ilshenar.classic.unwrap().wrap, ilshenar.geometry, "the wrap followed the size");
            assert_eq!(table.records[1].classic.unwrap().wrap.width, CLASSIC_TRAMMEL_WRAP);
            assert_eq!(table.records[0].classic.unwrap().wrap.width, STOCK_SPLIT_X, "Felucca's wrap is kept");
        }
        assert_eq!(after.classic.as_ref().unwrap().startup.unwrap().size.width, CLASSIC_NEW_WIDTH);
        assert_eq!(after.split.as_ref().unwrap().x, CLASSIC_TRAMMEL_WRAP);
        assert_eq!(patched.warnings.len(), 2, "Felucca's wrap 5120 is not the seam; Felucca is larger than stock");

        for bad in [
            vec![ExeEdit::ClassicWrap { facet: 3, wrap_width: stock_size(3).width + FACET_SIDE_ALIGNMENT, wrap_height: 8 }],
            vec![size(1, STOCK_SPLIT_X - FACET_SIDE_ALIGNMENT, stock_size(1).height)],
            vec![size(4, stock_size(4).width, stock_size(4).height + 4)],
        ] {
            assert!(plan_patch(&image, &ExePatch { edits: bad.clone() }, None).is_err(), "{bad:?}");
        }
        // A smaller Trammel with its wrap.
        let smaller = ExePatch {
            edits: vec![
                size(1, STOCK_SPLIT_X - FACET_SIDE_ALIGNMENT, stock_size(1).height),
                ExeEdit::ClassicWrap { facet: 1, wrap_width: STOCK_SPLIT_X - FACET_SIDE_ALIGNMENT, wrap_height: stock_size(1).height },
            ],
        };
        assert!(plan_patch(&image, &smaller, None).is_ok());
    }

    #[test]
    fn version_1_manifests_are_read() {
        let v1 = r#"{
            "format": "uocf-facet-dims", "version": 1, "build": "Enhanced Client UOSA.exe",
            "original_sha256": "aa", "source_sha256": "aa", "patched_sha256": "bb",
            "values": {"facets": [{"facet": 0, "name": "Felucca", "width": 8192, "height": 4160}], "split_x": 6144},
            "checksum": 1234
        }"#;
        let manifest = parse_manifest(v1).unwrap();
        assert_eq!(manifest.version, MANIFEST_VERSION_1);
        assert_eq!(manifest.family, None);
        assert_eq!(manifest.values.facets[0].wrap_width, None);
        assert_eq!(manifest.steps, [ManifestStep { source_sha256: "aa".into(), patched_sha256: "bb".into(), edits: vec![] }]);
        assert!(parse_manifest(&v1.replace("\"version\": 1", "\"version\": 3")).is_err());

        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let patched = patch_client_exe(&image, &ExePatch { edits: vec![size(0, NEW_WIDTH, NEW_HEIGHT)] }, None).unwrap();
        let text = serde_json::to_string_pretty(&patched.manifest).unwrap();
        assert_eq!(parse_manifest(&text).unwrap(), patched.manifest);
    }

    #[test]
    fn the_copy_goes_to_a_new_path_outside_the_client_folder() {
        let root = std::env::temp_dir().join(format!("uocf_client_exe_patch_{}", std::process::id()));
        let (client, copies) = (root.join("client"), root.join("copies"));
        std::fs::create_dir_all(&client).unwrap();
        std::fs::create_dir_all(&copies).unwrap();
        let exe = client.join("UOSA.exe");
        std::fs::write(&exe, synthetic_client(Some(STOCK_SPLIT_X), false)).unwrap();
        let patch = ExePatch { edits: vec![size(5, 1280, 4160)] };
        assert!(write_patched_client_exe(&exe, &exe, &patch).is_err());
        assert!(write_patched_client_exe(&exe, &client.join("copy.exe"), &patch).is_err());
        let output = copies.join("UOSA.exe");
        let plan = dry_run_client_exe(&exe, &patch).unwrap();
        assert!(!output.exists(), "a dry run writes nothing");
        let patched = write_patched_client_exe(&exe, &output, &patch).unwrap();
        assert_eq!(plan.changes, patched.changes);
        assert_eq!(std::fs::read(&output).unwrap(), patched.bytes);
        assert_eq!(read_manifest(&output).unwrap().unwrap(), patched.manifest);
        assert!(write_patched_client_exe(&exe, &output, &patch).is_err(), "the copy exists");
        let second = root.join("second.exe");
        let again = write_patched_client_exe(&output, &second, &ExePatch { edits: vec![size(6, 512, 512)] });
        assert_eq!(again.unwrap().manifest.original_sha256, patched.manifest.original_sha256);
        let _ = std::fs::remove_dir_all(root);
    }
}
