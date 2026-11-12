//! The edit state of the Facets, Seam and Login server sections: the text the user types per facet, for the split or
//! seam and for the login server, the no-encryption toggle, its live validation (the library's validate functions)
//! and the [`ExePatch`] it builds. Every section that edits the executable contributes its edits to [`EditState::build_patch`].

use uocf::client::classic_facets::{classic_map_blocks, validate_classic_size, validate_classic_wrap};
use uocf::client::facet_dimensions::{
    validate_facet_size, ClientExeInspection, ClientFamily, DISABLED_SPLIT_X, SPLIT_FACETS,
};
use uocf::client::patch::{stock_facets, ExeEdit, ExeManifest, ExePatch, SplitEdit};
use uocf::enhanced::facet_sector::FacetGeometry;

use super::encryption::EncryptionEdit;
use super::login::LoginEdit;

/// The lowest split or seam x the library takes.
const MIN_SPLIT_X: u32 = 1;

/// Errors (the patch cannot be built) and warnings (allowed but unusual) of one part of the edit state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Validation {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// One facet of the table: its values in the opened file, its shipped values and the text being edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetRow {
    pub facet: u8,
    pub name: String,
    /// The size and (Classic) wrap of the opened file.
    pub current: FacetGeometry,
    pub current_wrap: Option<FacetGeometry>,
    /// The shipped size and wrap, when known.
    pub stock: Option<FacetGeometry>,
    pub stock_wrap: Option<FacetGeometry>,
    pub width: String,
    pub height: String,
    /// Classic only. While not touched, the wrap follows the size the way the patcher does (a wrap side equal to the
    /// old side follows the new side) and the fields show it.
    pub wrap_width: String,
    pub wrap_height: String,
    pub wrap_touched: bool,
}

/// The validation of one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowCheck {
    /// The parsed size; `None` when a side does not parse.
    pub size: Option<FacetGeometry>,
    /// Classic: the wrap the copy will have (typed, or followed); `None` when it does not parse.
    pub wrap: Option<FacetGeometry>,
    pub validation: Validation,
}

/// What to do with the split (Enhanced Client) or wrap seam (Classic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamChoice {
    Keep,
    Move,
    Disable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeamEdit {
    /// The located x; `None` when the build has no split or seam (Kingdom Reborn) or it was not found.
    pub current: Option<u32>,
    pub choice: SeamChoice,
    /// The x of [`SeamChoice::Move`].
    pub x: String,
}

/// Everything the user edits, for the opened file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditState {
    pub family: ClientFamily,
    pub rows: Vec<FacetRow>,
    pub seam: SeamEdit,
    /// Classic: false when the startup copy of facet 0 was not located (facet 0 cannot be patched).
    pub facet_0_patchable: bool,
    /// Enhanced Client and Kingdom Reborn: the built-in login server, per site.
    pub login: LoginEdit,
    /// Enhanced Client and Kingdom Reborn: patch the network ciphers out.
    pub encryption: EncryptionEdit,
}

/// The wrap a Classic facet gets from a new size without a wrap edit: a wrap side equal to the old side follows the
/// new side (as [`uocf::client::patch::plan_patch`] does).
pub fn followed_wrap(wrap: FacetGeometry, old: FacetGeometry, new: FacetGeometry) -> FacetGeometry {
    let side = |wrap: u32, old: u32, new: u32| if wrap == old { new } else { wrap };
    FacetGeometry::new(side(wrap.width, old.width, new.width), side(wrap.height, old.height, new.height))
}

fn parse_side(what: &str, text: &str) -> Result<u32, String> {
    text.trim().parse::<u32>().map_err(|_| format!("{what} `{}` is not a whole number", text.trim()))
}

fn parse_size(what: [&str; 2], width: &str, height: &str, errors: &mut Vec<String>) -> Option<FacetGeometry> {
    match (parse_side(what[0], width), parse_side(what[1], height)) {
        (Ok(width), Ok(height)) => Some(FacetGeometry::new(width, height)),
        (width, height) => {
            errors.extend([width.err(), height.err()].into_iter().flatten());
            None
        }
    }
}

impl FacetRow {
    fn set_size(&mut self, size: FacetGeometry) {
        self.width = size.width.to_string();
        self.height = size.height.to_string();
    }

    fn set_wrap(&mut self, wrap: FacetGeometry) {
        self.wrap_width = wrap.width.to_string();
        self.wrap_height = wrap.height.to_string();
    }

    /// True when the row differs from the opened file.
    pub fn is_edited(&self) -> bool {
        let size = self.width.trim() != self.current.width.to_string() || self.height.trim() != self.current.height.to_string();
        size || self.wrap_touched
    }
}

impl EditState {
    /// The unedited state of the opened file, without a manifest.
    #[cfg(test)]
    pub fn new(inspection: &ClientExeInspection) -> Self {
        Self::with_manifest(inspection, None)
    }

    /// The unedited state of the opened file; `manifest` (of a patched copy) gives the stock sizes of the build it
    /// derives from ([`stock_facets`]).
    pub fn with_manifest(inspection: &ClientExeInspection, manifest: Option<&ExeManifest>) -> Self {
        let stock_sizes = stock_facets(inspection, manifest);
        let rows = inspection
            .table()
            .records
            .iter()
            .enumerate()
            .map(|(facet, record)| {
                let stock = stock_sizes[facet];
                let (stock, stock_wrap) = (stock.map(|stock| stock.size), stock.and_then(|stock| stock.wrap));
                let current_wrap = record.classic.map(|classic| classic.wrap);
                let mut row = FacetRow {
                    facet: facet as u8,
                    name: record.name.clone(),
                    current: record.geometry,
                    current_wrap,
                    stock,
                    stock_wrap,
                    width: String::new(),
                    height: String::new(),
                    wrap_width: String::new(),
                    wrap_height: String::new(),
                    wrap_touched: false,
                };
                row.set_size(record.geometry);
                if let Some(wrap) = current_wrap {
                    row.set_wrap(wrap);
                }
                row
            })
            .collect();
        let current = inspection.split.as_ref().map(|split| split.x);
        let seam = SeamEdit {
            current,
            choice: SeamChoice::Keep,
            x: current.map(|x| x.to_string()).unwrap_or_default(),
        };
        let facet_0_patchable = inspection.family != ClientFamily::Classic
            || inspection.classic.as_ref().is_some_and(|layout| layout.startup.is_some());
        Self {
            family: inspection.family,
            rows,
            seam,
            facet_0_patchable,
            login: LoginEdit::new(inspection),
            encryption: EncryptionEdit::new(inspection),
        }
    }

    pub fn is_classic(&self) -> bool {
        self.family == ClientFamily::Classic
    }

    /// Back to the opened file's values.
    pub fn reset_to_current(&mut self, index: usize) {
        let row = &mut self.rows[index];
        row.set_size(row.current);
        if let Some(wrap) = row.current_wrap {
            row.set_wrap(wrap);
        }
        row.wrap_touched = false;
    }

    /// True when the row has known shipped values that differ from what it shows.
    pub fn can_reset_to_stock(&self, index: usize) -> bool {
        let row = &self.rows[index];
        let Some(stock) = row.stock else {
            return false;
        };
        let shows_stock = row.width.trim() == stock.width.to_string() && row.height.trim() == stock.height.to_string();
        let wrap_stock = row.stock_wrap.is_none_or(|wrap| {
            row.wrap_width.trim() == wrap.width.to_string() && row.wrap_height.trim() == wrap.height.to_string()
        });
        !(shows_stock && wrap_stock)
    }

    /// To the shipped size (and wrap); a no-op when they are not known.
    pub fn reset_to_stock(&mut self, index: usize) {
        let row = &mut self.rows[index];
        let Some(stock) = row.stock else {
            return;
        };
        row.set_size(stock);
        if let (Some(stock_wrap), Some(current_wrap)) = (row.stock_wrap, row.current_wrap) {
            row.set_wrap(stock_wrap);
            row.wrap_touched = stock_wrap != followed_wrap(current_wrap, row.current, stock);
        }
    }

    /// Classic: shows the followed wrap in the wrap fields of every row whose wrap was not typed.
    pub fn sync_followed_wraps(&mut self) {
        for row in &mut self.rows {
            let Some(current_wrap) = row.current_wrap else {
                continue;
            };
            if row.wrap_touched {
                continue;
            }
            if let (Ok(width), Ok(height)) = (parse_side("width", &row.width), parse_side("height", &row.height)) {
                row.set_wrap(followed_wrap(current_wrap, row.current, FacetGeometry::new(width, height)));
            }
        }
    }

    /// Validates row `index` live: parse errors, the library's size and wrap checks, and the notes of the family.
    pub fn check_row(&self, index: usize) -> RowCheck {
        let row = &self.rows[index];
        let mut validation = Validation::default();
        let size = parse_size(["width", "height"], &row.width, &row.height, &mut validation.errors);
        let size_changed = size.is_some_and(|size| size != row.current);
        if let Some(size) = size.filter(|_| size_changed) {
            let checked = if self.is_classic() {
                validate_classic_size(size.width, size.height)
            } else {
                validate_facet_size(size.width, size.height)
            };
            match checked {
                Ok(warnings) => validation.warnings.extend(warnings),
                Err(error) => validation.errors.push(format!("{error:#}")),
            }
        }
        let wrap = row.current_wrap.and_then(|current_wrap| {
            let wrap = if row.wrap_touched {
                parse_size(["wrap width", "wrap height"], &row.wrap_width, &row.wrap_height, &mut validation.errors)?
            } else {
                followed_wrap(current_wrap, row.current, size?)
            };
            if let Some(size) = size
                && (size_changed || wrap != current_wrap)
                && let Err(error) = validate_classic_wrap(wrap, size)
            {
                let advice = if row.wrap_touched { "" } else { ": the new size cuts the wrap; set the wrap" };
                validation.errors.push(format!("{error:#}{advice}"));
            }
            Some(wrap)
        });
        let wrap_changed = matches!((wrap, row.current_wrap), (Some(wrap), Some(current)) if wrap != current);
        if self.is_classic() && row.facet == 0 && !self.facet_0_patchable && (size_changed || wrap_changed) {
            validation.errors.push(
                "the startup copy of facet 0 (the globals SetFacet fills) was not located: facet 0 cannot be patched"
                    .to_owned(),
            );
        }
        if self.family == ClientFamily::KingdomReborn && SPLIT_FACETS.contains(&row.facet) && size_changed {
            validation.warnings.push(format!(
                "Kingdom Reborn has no Felucca/Trammel split to move: facet {} is resized without one",
                row.facet
            ));
        }
        RowCheck { size, wrap, validation }
    }

    /// The edits of row `index` (none while it has errors).
    fn row_edits(&self, index: usize, check: &RowCheck) -> Vec<ExeEdit> {
        let row = &self.rows[index];
        let Some(size) = check.size.filter(|_| check.validation.is_ok()) else {
            return Vec::new();
        };
        let mut edits = Vec::new();
        if size != row.current {
            edits.push(ExeEdit::FacetSize { facet: row.facet, width: size.width, height: size.height });
        }
        if let (Some(wrap), Some(current_wrap)) = (check.wrap, row.current_wrap)
            && row.wrap_touched
            && wrap != followed_wrap(current_wrap, row.current, size)
        {
            edits.push(ExeEdit::ClassicWrap { facet: row.facet, wrap_width: wrap.width, wrap_height: wrap.height });
        }
        edits
    }

    /// Validates the split or seam choice; returns its edit, if any.
    pub fn check_seam(&self) -> (Option<SplitEdit>, Validation) {
        let mut validation = Validation::default();
        let seam = &self.seam;
        let edit = match seam.choice {
            SeamChoice::Keep => None,
            SeamChoice::Move => match parse_side("x", &seam.x) {
                Err(error) => {
                    validation.errors.push(error);
                    None
                }
                Ok(x) if !(MIN_SPLIT_X..=DISABLED_SPLIT_X).contains(&x) => {
                    validation.errors.push(format!("x {x} is outside {MIN_SPLIT_X}..={DISABLED_SPLIT_X}"));
                    None
                }
                Ok(x) => (seam.current != Some(x)).then_some(SplitEdit::Set(x)),
            },
            SeamChoice::Disable => (seam.current != Some(DISABLED_SPLIT_X)).then_some(SplitEdit::Disable),
        };
        if seam.current.is_none() && seam.choice != SeamChoice::Keep {
            validation.errors.push("this build has no split or seam located: keep it".to_owned());
        }
        (edit.filter(|_| validation.is_ok()), validation)
    }

    /// Classic: the wrap width facet 0 will have, for "seam at Felucca's wrap".
    pub fn felucca_wrap_width(&self) -> Option<u32> {
        if !self.is_classic() || self.rows.is_empty() {
            return None;
        }
        self.check_row(0).wrap.map(|wrap| wrap.width)
    }

    /// The patch of every section, in facet order, then the seam, then the login server, then the encryption; the
    /// errors of every row, the seam, the login server and the encryption when any part does not validate. An empty
    /// patch means nothing is edited.
    pub fn build_patch(&self) -> Result<ExePatch, Vec<String>> {
        let mut errors = Vec::new();
        let mut edits = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            let check = self.check_row(index);
            errors.extend(check.validation.errors.iter().map(|error| format!("facet {} ({}): {error}", row.facet, row.name)));
            edits.extend(self.row_edits(index, &check));
        }
        let (seam_edit, seam_validation) = self.check_seam();
        errors.extend(seam_validation.errors.iter().map(|error| format!("seam: {error}")));
        edits.extend(seam_edit.map(ExeEdit::Split));
        let (login_edit, login_validation) = self.login.check();
        errors.extend(login_validation.errors.iter().map(|error| format!("login server: {error}")));
        edits.extend(login_edit);
        let (encryption_edit, encryption_validation) = self.encryption.check();
        errors.extend(encryption_validation.errors.iter().map(|error| format!("encryption: {error}")));
        edits.extend(encryption_edit);
        if errors.is_empty() { Ok(ExePatch { edits }) } else { Err(errors) }
    }

    /// The sizes the rows show, for the data check; the first parse error otherwise.
    pub fn edited_dimensions(&self) -> Result<Vec<FacetGeometry>, String> {
        self.rows
            .iter()
            .map(|row| {
                let mut errors = Vec::new();
                parse_size(["width", "height"], &row.width, &row.height, &mut errors)
                    .ok_or_else(|| format!("facet {} ({}): {}", row.facet, row.name, errors.join("; ")))
            })
            .collect()
    }
}

/// What a size holds: sectors (Enhanced Client, Kingdom Reborn) or map blocks (Classic).
pub fn size_units(family: ClientFamily, size: FacetGeometry) -> String {
    match family {
        ClientFamily::Classic => format!("{} blocks", classic_map_blocks(size)),
        ClientFamily::Enhanced | ClientFamily::KingdomReborn => {
            format!("{} x {} = {} sectors", size.sector_columns(), size.sector_rows(), size.sector_count())
        }
    }
}
