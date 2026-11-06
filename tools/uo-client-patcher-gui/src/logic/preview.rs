//! The dry run of the edits (the [`PatchPlan`] and its rows as shown) and the data check against the edited sizes.

use std::path::{Path, PathBuf};

use uocf::client::facet_data::{check_client_facet_data_with_sizes, FacetDataReport, FacetDataStatus};
use uocf::client::facet_dimensions::ClientExeInspection;
use uocf::client::patch::{ByteChange, ChangeCause, ExePatch, PatchPlan, PatchWarning, PatchWarningKind};
use uocf::enhanced::facet_sector::FacetGeometry;

use super::edits::EditState;
use super::opened::OpenedExe;

/// One byte change as the preview shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRow {
    pub offset: String,
    pub va: String,
    pub field: String,
    pub old: String,
    pub new: String,
    pub cause: String,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(" ")
}

/// The rows of `changes`, causes named after the edits of `patch`.
pub fn change_rows(changes: &[ByteChange], patch: &ExePatch) -> Vec<ChangeRow> {
    changes
        .iter()
        .map(|change| ChangeRow {
            offset: format!("{:#010x}", change.offset),
            va: change.va.map(|va| format!("{va:#010x}")).unwrap_or_else(|| "header".to_owned()),
            field: change.field.clone(),
            old: hex(&change.old),
            new: hex(&change.new),
            cause: match change.cause {
                ChangeCause::Edit(index) => format!("edit {}: {}", index + 1, patch.edits[index].describe()),
                ChangeCause::Checksum => "PE checksum".to_owned(),
            },
        })
        .collect()
}

/// The library warnings about the split or seam.
pub fn seam_warnings(warnings: &[PatchWarning]) -> Vec<&PatchWarning> {
    warnings.iter().filter(|warning| matches!(warning.kind, PatchWarningKind::Split | PatchWarningKind::Seam)).collect()
}

/// The library warnings about the login server.
pub fn login_warnings(warnings: &[PatchWarning]) -> Vec<&PatchWarning> {
    warnings.iter().filter(|warning| warning.kind == PatchWarningKind::Login).collect()
}

/// The state of the dry run for the current edits.
#[derive(Debug, Clone)]
pub enum Preview {
    /// Nothing edited.
    Empty,
    /// The edits do not validate.
    Invalid(Vec<String>),
    /// The library refused the patch.
    Refused(String),
    Planned(Box<PatchPlan>),
}

impl Preview {
    pub fn plan(&self) -> Option<&PatchPlan> {
        match self {
            Self::Planned(plan) => Some(plan),
            _ => None,
        }
    }

    pub fn warnings(&self) -> &[PatchWarning] {
        self.plan().map(|plan| plan.warnings.as_slice()).unwrap_or_default()
    }
}

/// The dry run of `edits` on `opened`.
pub fn preview(opened: &OpenedExe, edits: &EditState) -> Preview {
    match edits.build_patch() {
        Err(errors) => Preview::Invalid(errors),
        Ok(patch) if patch.edits.is_empty() => Preview::Empty,
        Ok(patch) => match opened.plan(&patch) {
            Ok(plan) => Preview::Planned(Box::new(plan)),
            Err(error) => Preview::Refused(format!("{error:#}")),
        },
    }
}

/// Recomputes the dry run only when the edits change (the library inspects the whole file each time). One cache
/// serves one opened file.
#[derive(Debug, Default)]
pub struct PreviewCache {
    edits: Option<EditState>,
    preview: Option<Preview>,
}

impl PreviewCache {
    /// The dry run of `edits`, recomputed when they differ from the last call's.
    pub fn get(&mut self, opened: &OpenedExe, edits: &EditState) -> &Preview {
        if self.edits.as_ref() != Some(edits) || self.preview.is_none() {
            self.edits = Some(edits.clone());
            self.preview = Some(preview(opened, edits));
        }
        self.preview.as_ref().expect("computed above")
    }
}

/// Counts of the checked items.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DataCounts {
    pub ok: usize,
    pub mismatch: usize,
    pub absent: usize,
}

/// A data check: the folder and sizes it ran with and its reports.
#[derive(Debug, Clone)]
pub struct DataCheck {
    pub dir: PathBuf,
    pub dimensions: Vec<FacetGeometry>,
    pub reports: Result<Vec<FacetDataReport>, String>,
}

impl DataCheck {
    /// Checks the data in `dir` against `inspection` with the sizes `dimensions`. Only reads.
    pub fn run(inspection: &ClientExeInspection, dimensions: &[FacetGeometry], dir: &Path) -> Self {
        let reports = check_client_facet_data_with_sizes(inspection, dimensions, dir).map_err(|error| format!("{error:#}"));
        Self { dir: dir.to_path_buf(), dimensions: dimensions.to_vec(), reports }
    }

    pub fn counts(&self) -> DataCounts {
        let mut counts = DataCounts::default();
        for item in self.reports.iter().flatten().flat_map(|report| &report.items) {
            match item.status {
                FacetDataStatus::Ok => counts.ok += 1,
                FacetDataStatus::Mismatch => counts.mismatch += 1,
                FacetDataStatus::Absent => counts.absent += 1,
            }
        }
        counts
    }

    /// True when it ran with these sizes and folder.
    pub fn is_for(&self, dimensions: &[FacetGeometry], dir: &Path) -> bool {
        self.dimensions == dimensions && self.dir == dir
    }
}
