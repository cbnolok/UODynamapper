//! Section 1: what the inspection found.

use eframe::egui;
use uocf::client::patch::manifest_path;

use super::{error_label, info_grid, muted_label, warning_label, ITEM_SPACING, SUCCESS_COLOR};
use crate::app::Session;
use crate::logic::opened::{
    checksum_status, dif_lines, manifest_steps, seam_status, table_locations, version_label, ManifestState,
};

pub fn ui_executable(ui: &mut egui::Ui, session: &Session) {
    let opened = &session.opened;
    let inspection = &opened.inspection;
    let mut rows = vec![
        ("Family", format!("{} (located by structure)", inspection.family.label())),
        ("Known build", inspection.known.map_or_else(|| "not a traced build".to_owned(), |build| build.name.to_owned())),
        ("SHA-256", inspection.sha256.clone()),
        ("Size", format!("{} bytes", inspection.file_size)),
        ("Version", version_label(inspection.version.as_ref())),
        ("Values", if inspection.is_stock() { "stock".to_owned() } else { "not stock".to_owned() }),
        ("PE checksum", checksum_status(inspection)),
        ("Signature", if inspection.pe.is_signed() { "Authenticode signed".to_owned() } else { "not signed".to_owned() }),
    ];
    for (index, location) in table_locations(inspection).into_iter().enumerate() {
        rows.push((if index == 0 { "Facet table" } else { "" }, location));
    }
    if let Some(layout) = &inspection.classic {
        let names = match &layout.file_names {
            Some(names) => format!("{} entries of {} bytes at file {:#x}", names.names.len(), names.stride, names.offset),
            None => "not located: file ids are shown raw".to_owned(),
        };
        rows.push(("File names", names));
        let startup = match layout.startup {
            Some(startup) => format!(
                "facet 0 globals at file {:#x}: wrap {} x {}, size {} x {}",
                startup.offset, startup.wrap.width, startup.wrap.height, startup.size.width, startup.size.height
            ),
            None => "not located: facet 0 cannot be patched".to_owned(),
        };
        rows.push(("Startup copy", startup));
    }
    for (index, line) in dif_lines(inspection).into_iter().flatten().enumerate() {
        rows.push((if index == 0 { "Dif patches" } else { "" }, line));
    }
    rows.push(("Split / seam", seam_status(inspection)));
    info_grid(ui, "executable_grid", &rows);
    if let Some(difs) = &inspection.difs {
        egui::CollapsingHeader::new("Dif patch evidence").id_salt("dif_evidence").show(ui, |ui| {
            for line in &difs.evidence {
                muted_label(ui, line.as_str());
            }
        });
    }

    ui.add_space(ITEM_SPACING);
    let blockers = opened.blockers();
    if blockers.is_empty() {
        ui.colored_label(SUCCESS_COLOR, "Can be patched.");
    }
    for blocker in blockers {
        error_label(ui, format!("Cannot be patched: {blocker}"));
    }

    ui.add_space(ITEM_SPACING);
    let manifest_file = manifest_path(&opened.path);
    match &opened.manifest {
        ManifestState::None => muted_label(ui, "No manifest: not a copy written by the patcher."),
        ManifestState::Unreadable(error) => error_label(ui, format!("Manifest {}: {error}", manifest_file.display())),
        ManifestState::Stale(manifest) => warning_label(
            ui,
            format!(
                "Manifest {} records another file (SHA-256 {}; the copy was replaced): ignored.",
                manifest_file.display(),
                manifest.patched_sha256
            ),
        ),
        ManifestState::Current(manifest) => {
            ui.label(egui::RichText::new(format!("Manifest {}", manifest_file.display())).strong());
            let mut rows = vec![("Original", manifest.original_sha256.clone())];
            if let Some(build) = &manifest.build {
                rows.push(("Started from", build.clone()));
            }
            if let Some(version) = &manifest.exe_version {
                rows.push(("Version", version_label(Some(version))));
            }
            let steps = manifest_steps(manifest);
            for (index, step) in steps.into_iter().enumerate() {
                rows.push((if index == 0 { "Steps" } else { "" }, step));
            }
            info_grid(ui, "manifest_grid", &rows);
        }
    }
}
