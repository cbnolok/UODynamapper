//! `MobilePrescale.kdl`: the bodies whose mobile animation frames the packers upscale to 2x ahead of time
//! ([`super::mobile_prescale`]) when asked to (`--prescale`), beside the size rule of `--prescale-max-side`. The
//! file's own header documents the syntax:
//!
//! ```kdl
//! body 12 package="cc" prescale="always" // Dragon
//! body 400 package="cc" prescale="never"
//! ```
//!
//! `body` takes the in-game (server) body id and follows it to the stored frames the client draws for it, as
//! `MobileFrameFilters.kdl` does ([`super::frame_filter_table`]): every body sharing those frames gets them at 2x.
//! "never" wins over the size rule. The file is read strictly: an unknown node, property or value, a missing
//! property and a body listed twice for one package are errors naming the line; two bodies that share their stored
//! frames and ask for different choices are an error when the packer resolves them.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use knuffel::ast::Node;
use knuffel::span::LineSpan;
use log::{info, warn};
use uocf::utils::bundled_files;
use uot_kdl::strict::{error_at, in_file, required_property, value_string, value_u16, NodeSchema};

use super::frame_filter_table::FramePackage;
use crate::conversion_tables::MOBILE_PRESCALE_KDL;

pub const MOBILE_PRESCALE_FILE_NAME: &str = "MobilePrescale.kdl";
const BODY_NODE: &str = "body";
const PACKAGE_PROPERTY: &str = "package";
const PRESCALE_PROPERTY: &str = "prescale";

const MOBILE_PRESCALE_SCHEMA: &[NodeSchema] = &[NodeSchema::leaf(BODY_NODE, &[PACKAGE_PROPERTY, PRESCALE_PROPERTY])];

/// Whether a body's frames get their 2x frames ahead of time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyPrescale {
    /// Whatever their size.
    Always,
    /// Never, whatever the size rule says.
    Never,
}

impl BodyPrescale {
    pub const ALL: [BodyPrescale; 2] = [BodyPrescale::Always, BodyPrescale::Never];

    pub fn name(self) -> &'static str {
        match self {
            BodyPrescale::Always => "always",
            BodyPrescale::Never => "never",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.name() == name)
    }
}

/// A `body` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyPrescaleEntry {
    /// In-game (server) body id.
    pub body: u16,
    pub package: FramePackage,
    pub prescale: BodyPrescale,
    /// 1-based line of the entry, for messages.
    pub line: usize,
}

/// The file's entries.
#[derive(Debug, Clone, PartialEq)]
pub struct MobilePrescaleTable {
    source_name: String,
    bodies: Vec<BodyPrescaleEntry>,
}

/// The choices of one package's stored frames, keyed by whatever names a body's frames in that package.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPrescale<K> {
    by_frames: BTreeMap<K, BodyPrescaleEntry>,
    /// One line per `body` entry the package has no frames for.
    pub warnings: Vec<String>,
}

impl<K: Ord + Copy> ResolvedPrescale<K> {
    /// The choice of an entry for the frames `key` names, `None` when no entry reached them.
    pub fn choice(&self, key: K) -> Option<BodyPrescale> {
        self.by_frames.get(&key).map(|entry| entry.prescale)
    }

    /// Logs each frames key an entry reached and the warnings.
    pub fn log(&self, package: FramePackage, describe: impl Fn(K) -> String) {
        for (&key, entry) in &self.by_frames {
            info!("Prescale of package \"{}\": body {} (line {}): \"{}\" on {}", package.name(), entry.body,
                entry.line, entry.prescale.name(), describe(key));
        }
        for warning in &self.warnings {
            warn!("{warning}");
        }
    }
}

/// Reads the MobilePrescale.kdl at `given`, else the copy shipped with the tools; gives its path too.
pub fn load_mobile_prescale(given: Option<&Path>) -> eyre::Result<(PathBuf, MobilePrescaleTable)> {
    let path = bundled_files::resolve(given, MOBILE_PRESCALE_KDL)?;
    let table = MobilePrescaleTable::load(&path)?;
    Ok((path, table))
}

impl MobilePrescaleTable {
    pub fn load(path: &Path) -> eyre::Result<Self> {
        let text = std::fs::read_to_string(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::parse(&path.display().to_string(), &text)
    }

    /// Reads the file strictly (see the module documentation).
    pub fn parse(source_name: &str, text: &str) -> eyre::Result<Self> {
        Self::parse_entries(source_name, text).map_err(|error| in_file(source_name, error))
    }

    fn parse_entries(source_name: &str, text: &str) -> eyre::Result<Self> {
        let document = uot_kdl::parse_ast_line_span(source_name, text)
            .map_err(|error| uot_kdl::strict::describe_error(text, &error))?;
        uot_kdl::strict::check_document(&document, MOBILE_PRESCALE_SCHEMA)?;
        let mut bodies = Vec::<BodyPrescaleEntry>::new();
        for node in &document.nodes {
            let (package, prescale) = package_and_choice(node)?;
            let [body] = uot_kdl::strict::arguments::<1>(node)?;
            let body = value_u16(body, "body id")?;
            if let Some(first) = bodies.iter().find(|entry| entry.body == body && entry.package == package) {
                return Err(error_at(node.node_name.span(), format!(
                    "`{BODY_NODE}`: body {body} of package \"{}\" is already listed on line {}",
                    package.name(), first.line)));
            }
            bodies.push(BodyPrescaleEntry { body, package, prescale, line: line_of(node) });
        }
        Ok(Self { source_name: source_name.to_string(), bodies })
    }

    pub fn bodies(&self, package: FramePackage) -> impl Iterator<Item = &BodyPrescaleEntry> {
        self.bodies.iter().filter(move |entry| entry.package == package)
    }

    /// The choices of `package`'s stored frames. `frames_of` gives the key of the frames the client draws for a
    /// server body (`None`: the package has none for it), `describe` names a key in messages. Two entries reaching
    /// the same frames with different choices are an error.
    pub fn resolve<K: Ord + Copy + Debug>(
        &self,
        package: FramePackage,
        frames_of: impl Fn(u16) -> Option<K>,
        describe: impl Fn(K) -> String,
    ) -> eyre::Result<ResolvedPrescale<K>> {
        let mut resolved = ResolvedPrescale { by_frames: BTreeMap::new(), warnings: Vec::new() };
        for entry in self.bodies(package) {
            let Some(key) = frames_of(entry.body) else {
                resolved.warnings.push(format!(
                    "{}: line {}: package \"{}\" has no frames for body {}; the entry does nothing",
                    self.source_name, entry.line, package.name(), entry.body));
                continue;
            };
            match resolved.by_frames.get(&key) {
                Some(first) if first.prescale != entry.prescale => eyre::bail!(
                    "{}: line {}: body {} asks for prescale \"{}\", but it shares its frames ({}) with body {} of \
                     line {}, which asks for \"{}\"; give both the same choice",
                    self.source_name, entry.line, entry.body, entry.prescale.name(), describe(key), first.body,
                    first.line, first.prescale.name()),
                Some(_) => {}
                None => {
                    resolved.by_frames.insert(key, *entry);
                }
            }
        }
        Ok(resolved)
    }
}

fn line_of(node: &Node<LineSpan>) -> usize {
    node.node_name.span().0.line + 1
}

fn package_and_choice(node: &Node<LineSpan>) -> eyre::Result<(FramePackage, BodyPrescale)> {
    let package_value = required_property(node, PACKAGE_PROPERTY)?;
    let package_name = value_string(package_value, PACKAGE_PROPERTY)?;
    let package = FramePackage::ALL.into_iter().find(|package| package.name() == package_name).ok_or_else(|| {
        error_at(package_value.literal.span(), format!(
            "unknown package \"{package_name}\": expected {}",
            FramePackage::ALL.map(|package| format!("\"{}\"", package.name())).join(" or ")))
    })?;
    let choice_value = required_property(node, PRESCALE_PROPERTY)?;
    let choice_name = value_string(choice_value, PRESCALE_PROPERTY)?;
    let choice = BodyPrescale::from_name(&choice_name).ok_or_else(|| {
        error_at(choice_value.literal.span(), format!(
            "unknown prescale \"{choice_name}\": expected {}",
            BodyPrescale::ALL.map(|choice| format!("\"{}\"", choice.name())).join(" or ")))
    })?;
    Ok((package, choice))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "MobilePrescale.kdl";

    fn parse(text: &str) -> eyre::Result<MobilePrescaleTable> {
        MobilePrescaleTable::parse(FILE, text)
    }

    fn error(text: &str) -> String {
        parse(text).expect_err("error").to_string()
    }

    #[test]
    fn reads_bodies() {
        let table = parse("body 12 package=\"cc\" prescale=\"always\" // Dragon\n").unwrap();
        assert_eq!(table.bodies(FramePackage::Cc).copied().collect::<Vec<_>>(), [BodyPrescaleEntry {
            body: 12,
            package: FramePackage::Cc,
            prescale: BodyPrescale::Always,
            line: 1,
        }]);
        assert_eq!(table.bodies(FramePackage::Ec).count(), 0);
        assert!(parse("").unwrap().bodies(FramePackage::Cc).next().is_none());
    }

    #[test]
    fn unknown_names_and_values_are_errors_with_their_line() {
        assert_eq!(
            error("\nbodies 12 package=\"cc\" prescale=\"always\"\n"),
            "MobilePrescale.kdl: line 2, column 1: unknown node `bodies` at the top level: expected `body`"
        );
        assert_eq!(
            error("body 12 package=\"cc\" prescale=\"sometimes\"\n"),
            "MobilePrescale.kdl: line 1, column 31: unknown prescale \"sometimes\": expected \"always\" or \"never\""
        );
        assert_eq!(
            error("body 12 package=\"kr\" prescale=\"always\"\n"),
            "MobilePrescale.kdl: line 1, column 17: unknown package \"kr\": expected \"cc\" or \"ec\""
        );
        assert_eq!(
            error("body 12 prescale=\"always\"\n"),
            "MobilePrescale.kdl: line 1, column 1: `body`: missing package="
        );
        assert_eq!(
            error("body 12 package=\"cc\" prescale=\"always\"\nbody 12 package=\"cc\" prescale=\"never\"\n"),
            "MobilePrescale.kdl: line 2, column 1: `body`: body 12 of package \"cc\" is already listed on line 1"
        );
    }

    #[test]
    fn resolves_bodies_to_their_frames_and_rejects_conflicts() {
        // Server bodies 12 and 59 both show frames body 59; 400 has no frames.
        let frames_of = |body: u16| match body {
            12 | 59 => Some(59u32),
            400 => None,
            other => Some(u32::from(other)),
        };
        let describe = |key: u32| format!("frames body {key}");
        let table = parse("body 12 package=\"cc\" prescale=\"always\"\nbody 59 package=\"cc\" prescale=\"always\"\n\
            body 400 package=\"cc\" prescale=\"never\"\nbody 286 package=\"cc\" prescale=\"never\"\n").unwrap();
        let resolved = table.resolve(FramePackage::Cc, frames_of, describe).unwrap();
        assert_eq!(resolved.choice(59), Some(BodyPrescale::Always));
        assert_eq!(resolved.choice(286), Some(BodyPrescale::Never));
        assert_eq!(resolved.choice(1), None);
        assert_eq!(resolved.warnings, [
            "MobilePrescale.kdl: line 3: package \"cc\" has no frames for body 400; the entry does nothing"
        ]);
        let conflicting = parse("body 12 package=\"cc\" prescale=\"always\"\nbody 59 package=\"cc\" prescale=\"never\"\n")
            .unwrap();
        assert_eq!(
            conflicting.resolve(FramePackage::Cc, frames_of, describe).unwrap_err().to_string(),
            "MobilePrescale.kdl: line 2: body 59 asks for prescale \"never\", but it shares its frames (frames body \
             59) with body 12 of line 1, which asks for \"always\"; give both the same choice"
        );
    }

    #[test]
    fn the_shipped_table_reads() {
        let (path, _) = load_mobile_prescale(None).unwrap();
        assert_eq!(path, MOBILE_PRESCALE_KDL.checkout_dir().join(MOBILE_PRESCALE_KDL.relative_path()));
    }
}
