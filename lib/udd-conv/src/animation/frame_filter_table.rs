//! `MobileFrameFilters.kdl`: the cleanup filter ([`FrameFilter`]) each body's mobile animation frames get at pack
//! time, per package. The file's own header documents the syntax:
//!
//! ```kdl
//! default package="ec" filter="highlight"
//! default package="cc" filter="none"
//! body 12 package="ec" filter="median" // Dragon
//! ```
//!
//! `body` takes the in-game (server) body id. The packer resolves it to the stored frames the client draws for it,
//! through the same body mapping that builds the package's body records, and filters those frames; every body that
//! shares them shows them filtered. Frames no `body` entry reaches take their package's `default`.
//!
//! The file is read strictly: an unknown node, property or value, a missing property, a package without its
//! `default` or with two, and a body listed twice for one package are errors naming the line. Two bodies that share
//! their stored frames and ask for different filters are an error when the packer resolves them.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use knuffel::ast::Node;
use knuffel::span::LineSpan;
use log::{info, warn};
use uocf::utils::bundled_files;
use uot_kdl::strict::{error_at, in_file, required_property, value_string, value_u16, NodeSchema};

use super::frame_filters::FrameFilter;
use crate::conversion_tables::MOBILE_FRAME_FILTERS_KDL;

pub const MOBILE_FRAME_FILTERS_FILE_NAME: &str = "MobileFrameFilters.kdl";
const DEFAULT_NODE: &str = "default";
const BODY_NODE: &str = "body";
const PACKAGE_PROPERTY: &str = "package";
const FILTER_PROPERTY: &str = "filter";

const MOBILE_FRAME_FILTERS_SCHEMA: &[NodeSchema] = &[
    NodeSchema::leaf(DEFAULT_NODE, &[PACKAGE_PROPERTY, FILTER_PROPERTY]),
    NodeSchema::leaf(BODY_NODE, &[PACKAGE_PROPERTY, FILTER_PROPERTY]),
];

/// The mobile animation package whose frames an entry filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FramePackage {
    /// `mobile_anim_cc.uddp`: the Classic frames.
    Cc,
    /// `mobile_anim_ec.uddp`: the Enhanced frames.
    Ec,
}

impl FramePackage {
    pub const ALL: [FramePackage; 2] = [FramePackage::Cc, FramePackage::Ec];

    pub fn name(self) -> &'static str {
        match self {
            FramePackage::Cc => "cc",
            FramePackage::Ec => "ec",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|package| package.name() == name)
    }
}

/// A `body` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyFrameFilter {
    /// In-game (server) body id.
    pub body: u16,
    pub package: FramePackage,
    pub filter: FrameFilter,
    /// 1-based line of the entry, for messages.
    pub line: usize,
}

/// The file's entries.
#[derive(Debug, Clone, PartialEq)]
pub struct MobileFrameFilters {
    source_name: String,
    cc_default: FrameFilter,
    ec_default: FrameFilter,
    bodies: Vec<BodyFrameFilter>,
}

/// The filters of one package's stored frames, keyed by whatever names a body's frames in that package.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFrameFilters<K> {
    default: FrameFilter,
    by_frames: BTreeMap<K, BodyFrameFilter>,
    /// One line per `body` entry the package has no frames for.
    pub warnings: Vec<String>,
}

impl<K: Ord + Copy> ResolvedFrameFilters<K> {
    /// The filter of the frames `key` names.
    pub fn filter_for(&self, key: K) -> FrameFilter {
        self.by_frames.get(&key).map_or(self.default, |entry| entry.filter)
    }

    /// Every frames key a `body` entry reached, with the entry.
    pub fn entries(&self) -> impl Iterator<Item = (&K, &BodyFrameFilter)> {
        self.by_frames.iter()
    }

    pub fn default_filter(&self) -> FrameFilter {
        self.default
    }

    /// The default for every body.
    pub fn uniform(filter: FrameFilter) -> Self {
        Self { default: filter, by_frames: BTreeMap::new(), warnings: Vec::new() }
    }

    /// Logs the default, each frames key an entry reached and the warnings.
    pub fn log(&self, package: FramePackage, describe: impl Fn(K) -> String) {
        info!("Mobile frame filters of package \"{}\": default \"{}\"", package.name(), self.default.name());
        for (&key, entry) in &self.by_frames {
            info!("  body {} (line {}): \"{}\" on {}", entry.body, entry.line, entry.filter.name(), describe(key));
        }
        for warning in &self.warnings {
            warn!("{warning}");
        }
    }
}

/// Reads the MobileFrameFilters.kdl at `given`, else the copy shipped with the tools; gives its path too.
pub fn load_mobile_frame_filters(given: Option<&Path>) -> eyre::Result<(PathBuf, MobileFrameFilters)> {
    let path = bundled_files::resolve(given, MOBILE_FRAME_FILTERS_KDL)?;
    let filters = MobileFrameFilters::load(&path)?;
    Ok((path, filters))
}

impl MobileFrameFilters {
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
        uot_kdl::strict::check_document(&document, MOBILE_FRAME_FILTERS_SCHEMA)?;
        let mut defaults = BTreeMap::<&str, (FrameFilter, usize)>::new();
        let mut bodies = Vec::<BodyFrameFilter>::new();
        for node in &document.nodes {
            let (package, filter) = package_and_filter(node)?;
            let line = line_of(node);
            let name: &str = &node.node_name;
            if name == DEFAULT_NODE {
                uot_kdl::strict::arguments::<0>(node)?;
                if let Some((_, first)) = defaults.insert(package.name(), (filter, line)) {
                    return Err(error_at(node.node_name.span(), format!(
                        "`{DEFAULT_NODE}`: package \"{}\" already has its default on line {first}", package.name())));
                }
            } else {
                let [body] = uot_kdl::strict::arguments::<1>(node)?;
                let body = value_u16(body, "body id")?;
                if let Some(first) = bodies.iter().find(|entry| entry.body == body && entry.package == package) {
                    return Err(error_at(node.node_name.span(), format!(
                        "`{BODY_NODE}`: body {body} of package \"{}\" is already listed on line {}",
                        package.name(), first.line)));
                }
                bodies.push(BodyFrameFilter { body, package, filter, line });
            }
        }
        let default = |package: FramePackage| {
            defaults.get(package.name()).map(|&(filter, _)| filter).ok_or_else(|| {
                eyre::eyre!("missing `{DEFAULT_NODE} {PACKAGE_PROPERTY}=\"{}\" {FILTER_PROPERTY}=...`", package.name())
            })
        };
        Ok(Self {
            source_name: source_name.to_string(),
            cc_default: default(FramePackage::Cc)?,
            ec_default: default(FramePackage::Ec)?,
            bodies,
        })
    }

    pub fn default_filter(&self, package: FramePackage) -> FrameFilter {
        match package {
            FramePackage::Cc => self.cc_default,
            FramePackage::Ec => self.ec_default,
        }
    }

    pub fn bodies(&self, package: FramePackage) -> impl Iterator<Item = &BodyFrameFilter> {
        self.bodies.iter().filter(move |entry| entry.package == package)
    }

    /// The filters of `package`'s stored frames. `frames_of` gives the key of the frames the client draws for a
    /// server body (`None`: the package has none for it), `describe` names a key in messages. Two entries reaching
    /// the same frames with different filters are an error.
    pub fn resolve<K: Ord + Copy + Debug>(
        &self,
        package: FramePackage,
        frames_of: impl Fn(u16) -> Option<K>,
        describe: impl Fn(K) -> String,
    ) -> eyre::Result<ResolvedFrameFilters<K>> {
        let mut resolved = ResolvedFrameFilters::uniform(self.default_filter(package));
        for entry in self.bodies(package) {
            let Some(key) = frames_of(entry.body) else {
                resolved.warnings.push(format!(
                    "{}: line {}: package \"{}\" has no frames for body {}; the entry does nothing",
                    self.source_name, entry.line, package.name(), entry.body));
                continue;
            };
            match resolved.by_frames.get(&key) {
                Some(first) if first.filter != entry.filter => eyre::bail!(
                    "{}: line {}: body {} asks for filter \"{}\", but it shares its frames ({}) with body {} of line {}, \
                     which asks for \"{}\"; give both the same filter",
                    self.source_name, entry.line, entry.body, entry.filter.name(), describe(key), first.body,
                    first.line, first.filter.name()),
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

fn package_and_filter(node: &Node<LineSpan>) -> eyre::Result<(FramePackage, FrameFilter)> {
    let package_value = required_property(node, PACKAGE_PROPERTY)?;
    let package_name = value_string(package_value, PACKAGE_PROPERTY)?;
    let package = FramePackage::from_name(&package_name).ok_or_else(|| {
        error_at(package_value.literal.span(), format!(
            "unknown package \"{package_name}\": expected {}",
            FramePackage::ALL.map(|package| format!("\"{}\"", package.name())).join(" or ")))
    })?;
    let filter_value = required_property(node, FILTER_PROPERTY)?;
    let filter_name = value_string(filter_value, FILTER_PROPERTY)?;
    let filter = FrameFilter::from_name(&filter_name).ok_or_else(|| {
        error_at(filter_value.literal.span(), format!(
            "unknown filter \"{filter_name}\": expected one of {}",
            FrameFilter::ALL.map(|filter| format!("\"{}\"", filter.name())).join(", ")))
    })?;
    Ok((package, filter))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "MobileFrameFilters.kdl";
    const DEFAULTS: &str = "default package=\"ec\" filter=\"highlight\"\ndefault package=\"cc\" filter=\"none\"\n";

    fn parse(text: &str) -> eyre::Result<MobileFrameFilters> {
        MobileFrameFilters::parse(FILE, text)
    }

    fn error(text: &str) -> String {
        parse(text).expect_err("error").to_string()
    }

    #[test]
    fn reads_defaults_and_bodies() {
        let filters = parse(&format!("{DEFAULTS}body 12 package=\"ec\" filter=\"median\" // Dragon\n")).unwrap();
        assert_eq!(filters.default_filter(FramePackage::Ec), FrameFilter::Highlight);
        assert_eq!(filters.default_filter(FramePackage::Cc), FrameFilter::None);
        assert_eq!(filters.bodies(FramePackage::Ec).copied().collect::<Vec<_>>(), [BodyFrameFilter {
            body: 12,
            package: FramePackage::Ec,
            filter: FrameFilter::Median,
            line: 3,
        }]);
        assert_eq!(filters.bodies(FramePackage::Cc).count(), 0);
    }

    #[test]
    fn unknown_names_and_values_are_errors_with_their_line() {
        assert_eq!(
            error(&format!("{DEFAULTS}bodies 12 package=\"ec\" filter=\"median\"\n")),
            "MobileFrameFilters.kdl: line 3, column 1: unknown node `bodies` at the top level: expected `default` or `body`"
        );
        assert_eq!(
            error(&format!("{DEFAULTS}body 12 package=\"ec\" filter=\"median\" strength=2\n")),
            "MobileFrameFilters.kdl: line 3, column 38: unknown property `strength` of `body`: expected `package` or `filter`"
        );
        assert_eq!(
            error(&format!("{DEFAULTS}body 12 package=\"ec\" filter=\"blur\"\n")),
            "MobileFrameFilters.kdl: line 3, column 29: unknown filter \"blur\": expected one of \"none\", \"highlight\", \"median\""
        );
        assert_eq!(
            error(&format!("{DEFAULTS}body 12 package=\"kr\" filter=\"median\"\n")),
            "MobileFrameFilters.kdl: line 3, column 17: unknown package \"kr\": expected \"cc\" or \"ec\""
        );
        assert_eq!(
            error(&format!("{DEFAULTS}body 12 filter=\"median\"\n")),
            "MobileFrameFilters.kdl: line 3, column 1: `body`: missing package="
        );
        assert_eq!(
            error(&format!("{DEFAULTS}body 70000 package=\"ec\" filter=\"median\"\n")),
            "MobileFrameFilters.kdl: line 3, column 6: body id: not a u16 (number too large to fit in target type)"
        );
        assert_eq!(
            error(&format!("{DEFAULTS}body package=\"ec\" filter=\"median\"\n")),
            "MobileFrameFilters.kdl: line 3, column 1: `body` takes 1 arguments, found 0"
        );
    }

    #[test]
    fn defaults_are_required_once_and_bodies_once_per_package() {
        assert_eq!(
            error("default package=\"ec\" filter=\"highlight\"\n"),
            "MobileFrameFilters.kdl: missing `default package=\"cc\" filter=...`"
        );
        assert_eq!(
            error(&format!("{DEFAULTS}default package=\"ec\" filter=\"none\"\n")),
            "MobileFrameFilters.kdl: line 3, column 1: `default`: package \"ec\" already has its default on line 1"
        );
        assert_eq!(
            error(&format!("{DEFAULTS}body 12 package=\"ec\" filter=\"median\"\nbody 12 package=\"ec\" filter=\"none\"\n")),
            "MobileFrameFilters.kdl: line 4, column 1: `body`: body 12 of package \"ec\" is already listed on line 3"
        );
        assert!(parse(&format!("{DEFAULTS}body 12 package=\"ec\" filter=\"median\"\nbody 12 package=\"cc\" filter=\"none\"\n")).is_ok());
    }

    #[test]
    fn resolves_bodies_to_their_frames_and_rejects_conflicts() {
        // Server bodies 12 and 59 both show frames body 59; 400 has no frames.
        let frames_of = |body: u16| match body {
            12 | 59 => Some(59u32),
            400 => None,
            other => Some(u32::from(other)),
        };
        let describe = |key: u32| format!("AnimationFrame body {key}");
        let filters = parse(&format!(
            "{DEFAULTS}body 12 package=\"ec\" filter=\"median\"\nbody 59 package=\"ec\" filter=\"median\"\n\
             body 400 package=\"ec\" filter=\"none\"\nbody 286 package=\"ec\" filter=\"none\"\n"
        ))
        .unwrap();
        let resolved = filters.resolve(FramePackage::Ec, frames_of, describe).unwrap();
        assert_eq!(resolved.filter_for(59), FrameFilter::Median);
        assert_eq!(resolved.filter_for(286), FrameFilter::None);
        assert_eq!(resolved.filter_for(1), FrameFilter::Highlight);
        assert_eq!(resolved.warnings, [
            "MobileFrameFilters.kdl: line 5: package \"ec\" has no frames for body 400; the entry does nothing"
        ]);
        let cc = filters.resolve(FramePackage::Cc, frames_of, describe).unwrap();
        assert_eq!(cc.filter_for(59), FrameFilter::None);

        let conflicting = parse(&format!(
            "{DEFAULTS}body 12 package=\"ec\" filter=\"median\"\nbody 59 package=\"ec\" filter=\"highlight\"\n"
        ))
        .unwrap();
        assert_eq!(
            conflicting.resolve(FramePackage::Ec, frames_of, describe).unwrap_err().to_string(),
            "MobileFrameFilters.kdl: line 4: body 59 asks for filter \"highlight\", but it shares its frames \
             (AnimationFrame body 59) with body 12 of line 3, which asks for \"median\"; give both the same filter"
        );
    }

    #[test]
    fn the_shipped_table_reads() {
        let (path, filters) = load_mobile_frame_filters(None).unwrap();
        assert_eq!(path, MOBILE_FRAME_FILTERS_KDL.checkout_dir().join(MOBILE_FRAME_FILTERS_KDL.relative_path()));
        assert_eq!(filters.default_filter(FramePackage::Ec), FrameFilter::Highlight);
        assert_eq!(filters.default_filter(FramePackage::Cc), FrameFilter::None);
    }
}
