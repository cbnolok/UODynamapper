//! `EffectTexture.uop` for the Effects panel: the loaded package with the kind and script references of every
//! file, the pending file changes of the document laid over it, the browse filter, the Chaos script checks,
//! texture decoding, imports from disk and saving to a new package. No egui here.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use gamebryo_nif::{Block, Link, NifFile};
use particle_sim::chaos::compile::compile_chaos_source;
use particle_sim::chaos::ChaosIssueSeverity as CompileSeverity;
use particle_sim::nif::compile::decode_pixel_level;
use uocf::enhanced::chaos_script::{ChaosIssueSeverity as ParseSeverity, ChaosScript, ChaosValidationContext};
use uocf::enhanced::effect_texture::{effect_texture_hash, EffectFileKind, EffectTexturePackage};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem};

use super::effects_editor::{refuse_protected, FileOverride};

/// File name extensions that can be imported from disk into the package (lowercase).
pub const IMPORTABLE_EXTENSIONS: [&str; 4] = ["ems", "tga", "dds", "nif"];
const EMS_EXTENSION: &str = "ems";
const TGA_EXTENSION: &str = "tga";
const NIF_EXTENSION: &str = "nif";
const DDS_MAGIC: &[u8] = b"DDS ";
/// Prefix of the uocf Chaos parser error messages: `line N, column M: ...`.
const PARSE_ERROR_LINE_PREFIX: &str = "line ";
const PARSE_ERROR_COLUMN_SEPARATOR: &str = ", column ";
const PARSE_ERROR_MESSAGE_SEPARATOR: &str = ": ";

/// What an effect file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    Nif,
    Chaos,
    Tga,
    Dds,
    Psd,
    Other,
    /// A placeholder entry without payload (the client package has some).
    Empty,
}

impl FileKind {
    pub const ALL: [Self; 7] = [Self::Nif, Self::Chaos, Self::Tga, Self::Dds, Self::Psd, Self::Other, Self::Empty];

    /// The kind of a payload; `name` tells TGA (no magic) from other files.
    pub fn detect(payload: &[u8], name: Option<&str>) -> Self {
        if payload.is_empty() {
            return Self::Empty;
        }
        match EffectFileKind::detect(payload) {
            EffectFileKind::Nif => Self::Nif,
            EffectFileKind::ChaosScript => Self::Chaos,
            EffectFileKind::Dds => Self::Dds,
            EffectFileKind::Psd => Self::Psd,
            EffectFileKind::Other if name.is_some_and(|name| has_extension(name, TGA_EXTENSION)) => Self::Tga,
            EffectFileKind::Other => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Nif => "NIF",
            Self::Chaos => "Chaos .ems",
            Self::Tga => "TGA",
            Self::Dds => "DDS",
            Self::Psd => "PSD",
            Self::Other => "other",
            Self::Empty => "empty",
        }
    }
}

/// Whether `name` ends with `.extension` (case insensitive).
pub fn has_extension(name: &str, extension: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, found)| found.eq_ignore_ascii_case(extension))
}

/// Whether a particle name is a NIF scene by its extension (the client plays every name without `.ems` as one).
pub fn is_nif_name(name: &str) -> bool {
    has_extension(name, NIF_EXTENSION)
}

/// How a file differs from the loaded package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileStatus {
    Original,
    Modified,
    Added,
    Removed,
}

impl FileStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "",
            Self::Modified => "modified",
            Self::Added => "added",
            Self::Removed => "removed",
        }
    }
}

/// One file of the package, with the pending change applied.
#[derive(Debug, Clone, PartialEq)]
pub struct FileRow {
    pub hash: u64,
    /// The base name (`fizzle.ems`), when resolved.
    pub name: Option<String>,
    pub kind: FileKind,
    /// Payload size in bytes.
    pub size: usize,
    pub status: FileStatus,
    /// Files a Chaos script names as texture or mesh.
    pub references: Vec<String>,
}

impl FileRow {
    fn from_payload(hash: u64, name: Option<String>, payload: &[u8], status: FileStatus) -> Self {
        let kind = FileKind::detect(payload, name.as_deref());
        let references = match kind {
            FileKind::Chaos => script_references(payload),
            _ => Vec::new(),
        };
        Self { hash, name, kind, size: payload.len(), status, references }
    }

    /// The name, or the hash in hex.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("{:016x}", self.hash))
    }
}

/// The files a script references (texture and mesh names); empty when it does not parse.
pub fn script_references(payload: &[u8]) -> Vec<String> {
    ChaosScript::parse_bytes(payload)
        .map(|script| script.referenced_files().into_iter().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The loaded `EffectTexture.uop`.
pub struct EffectFiles {
    package: EffectTexturePackage,
    rows: Vec<FileRow>,
    index: HashMap<u64, usize>,
    /// Payloads of the Chaos scripts, kept for the editor and the checks (small text files).
    scripts: HashMap<u64, Arc<[u8]>>,
}

impl EffectFiles {
    /// Opens a package and reads every file once to learn its kind; names are resolved from the string
    /// dictionary and from the payloads.
    pub fn open(path: &Path, strings: Option<&UoStringDictionary>) -> Result<Self, String> {
        let mut package = EffectTexturePackage::load(path).map_err(|error| format!("{error:#}"))?;
        if let Some(strings) = strings {
            package.resolve_names_from_string_dictionary(strings);
        }
        package.harvest_names_from_payloads().map_err(|error| format!("{error:#}"))?;
        let mut rows = Vec::new();
        let mut scripts = HashMap::new();
        for entry in package.entries() {
            let payload = if entry.has_payload() {
                package.read_by_hash(entry.hash).map_err(|error| format!("{error:#}"))?.unwrap_or_default()
            } else {
                Vec::new()
            };
            let name = entry.file_name().map(str::to_owned);
            let row = FileRow::from_payload(entry.hash, name, &payload, FileStatus::Original);
            if row.kind == FileKind::Chaos {
                scripts.insert(entry.hash, Arc::from(payload));
            }
            rows.push(row);
        }
        let index = rows.iter().enumerate().map(|(position, row)| (row.hash, position)).collect();
        Ok(Self { package, rows, index, scripts })
    }

    pub fn source_path(&self) -> &Path {
        self.package.source_path()
    }

    /// The resolved base name of a file of the package or of a pending change.
    pub fn name(&self, overrides: &BTreeMap<u64, FileOverride>, hash: u64) -> Option<String> {
        overrides
            .get(&hash)
            .map(|change| change.name.clone())
            .or_else(|| self.index.get(&hash).and_then(|&position| self.rows[position].name.clone()))
    }

    /// Every file with the pending changes applied: package order, then the added files.
    pub fn rows(&self, overrides: &BTreeMap<u64, FileOverride>) -> Vec<FileRow> {
        let mut rows: Vec<FileRow> = self
            .rows
            .iter()
            .map(|row| match overrides.get(&row.hash) {
                None => row.clone(),
                Some(FileOverride { payload: None, .. }) => FileRow { status: FileStatus::Removed, ..row.clone() },
                Some(FileOverride { name, payload: Some(payload) }) => {
                    FileRow::from_payload(row.hash, Some(name.clone()), payload, FileStatus::Modified)
                }
            })
            .collect();
        for (&hash, change) in overrides {
            if self.index.contains_key(&hash) {
                continue;
            }
            if let Some(payload) = &change.payload {
                rows.push(FileRow::from_payload(hash, Some(change.name.clone()), payload, FileStatus::Added));
            }
        }
        rows
    }

    /// The payload of a file, with the pending changes applied; `None` when it does not exist or was removed.
    pub fn payload(&self, overrides: &BTreeMap<u64, FileOverride>, hash: u64) -> Option<Arc<[u8]>> {
        match overrides.get(&hash) {
            Some(change) => change.payload.clone(),
            None => match self.scripts.get(&hash) {
                Some(script) => Some(Arc::clone(script)),
                None => self.package.read_by_hash(hash).ok().flatten().map(Arc::from),
            },
        }
    }

    /// Whether a file named as in a particle definition or a script exists (with the pending changes).
    pub fn exists(&self, overrides: &BTreeMap<u64, FileOverride>, name: &str) -> bool {
        let hash = effect_texture_hash(name);
        match overrides.get(&hash) {
            Some(change) => change.payload.is_some(),
            None => self.index.contains_key(&hash),
        }
    }

    /// The scripts with their references, with the pending changes applied, for the "used by" links.
    pub fn script_references(&self, overrides: &BTreeMap<u64, FileOverride>) -> Vec<(u64, Vec<String>)> {
        self.rows(overrides)
            .into_iter()
            .filter(|row| row.status != FileStatus::Removed && !row.references.is_empty())
            .map(|row| (row.hash, row.references))
            .collect()
    }

    /// Writes a copy of the loaded package with the pending changes to `output`. Refuses to write over the
    /// loaded package or any `protected` file. Replaced entries keep their old data block header.
    pub fn save_as(
        &self,
        overrides: &BTreeMap<u64, FileOverride>,
        output: &Path,
        protected: &[&Path],
    ) -> Result<FileSaveReport, String> {
        let source = self.source_path();
        refuse_protected(output, std::iter::once(source).chain(protected.iter().copied()))?;
        let mut package = EffectTexturePackage::load(source).map_err(|error| format!("{error:#}"))?;
        let mut report = FileSaveReport::default();
        for change in overrides.values() {
            match &change.payload {
                Some(payload) => match package.put(&change.name, payload).map_err(|error| format!("{error:#}"))? {
                    uocf::enhanced::effect_texture::EntryPut::Replaced => report.replaced += 1,
                    uocf::enhanced::effect_texture::EntryPut::Added => report.added += 1,
                },
                None => {
                    if package.remove(&change.name) {
                        report.removed += 1;
                    }
                }
            }
        }
        package.save_as(output).map_err(|error| format!("{error:#}"))?;
        Ok(report)
    }
}

/// What [`EffectFiles::save_as`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileSaveReport {
    pub replaced: usize,
    pub added: usize,
    pub removed: usize,
}

/// Filter of the file list. Empty fields do not filter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileFilter {
    /// Matches the name or the hex hash (case insensitive).
    pub text: String,
    pub kind: Option<FileKind>,
    /// Only files without a resolved name.
    pub unnamed_only: bool,
    /// Only files with pending changes.
    pub changed_only: bool,
}

impl FileFilter {
    pub fn matches(&self, row: &FileRow) -> bool {
        if self.kind.is_some_and(|kind| kind != row.kind)
            || (self.unnamed_only && row.name.is_some())
            || (self.changed_only && row.status == FileStatus::Original)
        {
            return false;
        }
        let query = self.text.trim().to_lowercase();
        query.is_empty()
            || row.name.as_deref().is_some_and(|name| name.to_lowercase().contains(&query))
            || format!("{:016x}", row.hash).contains(query.trim_start_matches("0x"))
    }

    /// The indices of the rows that pass, sorted by name (unnamed files last, by hash).
    pub fn visible(&self, rows: &[FileRow]) -> Vec<usize> {
        let mut visible: Vec<usize> = (0..rows.len()).filter(|&index| self.matches(&rows[index])).collect();
        visible.sort_by(|&left, &right| {
            let key = |row: &FileRow| (row.name.is_none(), row.name.as_deref().map(str::to_lowercase), row.hash);
            key(&rows[left]).cmp(&key(&rows[right]))
        });
        visible
    }
}

/// Severity of a script finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScriptSeverity {
    Note,
    Warning,
    Error,
}

/// One finding of the script checks, at a 1-based line and column when known.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScriptIssue {
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub severity: ScriptSeverity,
    pub message: String,
}

impl std::fmt::Display for ScriptIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "{line}:{column}: {}", self.message),
            (Some(line), None) => write!(f, "{line}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

/// Checks a script with the uocf parser and validator (syntax, values, missing files) and with the client
/// compiler rules of particle-sim (what the client rejects or ignores). Sorted by position; duplicates dropped.
pub fn check_script(payload: &[u8], file_exists: Option<&dyn Fn(&str) -> bool>) -> Vec<ScriptIssue> {
    let mut issues = Vec::new();
    match ChaosScript::parse_bytes(payload) {
        Ok(script) => {
            for issue in script.validate(&ChaosValidationContext { file_exists }) {
                let severity = match issue.severity() {
                    ParseSeverity::Warning => ScriptSeverity::Warning,
                    ParseSeverity::Error => ScriptSeverity::Error,
                };
                let location = issue.location;
                let message = uocf::enhanced::chaos_script::ChaosIssue { location: None, ..issue }.to_string();
                issues.push(ScriptIssue {
                    line: location.map(|location| location.line),
                    column: location.map(|location| location.column),
                    severity,
                    message,
                });
            }
        }
        Err(error) => issues.push(parse_error_issue(&format!("{error:#}"))),
    }
    for issue in compile_chaos_source(payload).issues {
        if matches!(issue.kind, particle_sim::chaos::ChaosIssueKind::SyntaxError { .. }) {
            continue; // reported by the parser above
        }
        let severity = match issue.severity() {
            CompileSeverity::Note => ScriptSeverity::Note,
            CompileSeverity::Warning => ScriptSeverity::Warning,
            CompileSeverity::Error => ScriptSeverity::Error,
        };
        let location = issue.location;
        let message = format!("client: {}", particle_sim::chaos::ChaosIssue { location: None, ..issue });
        issues.push(ScriptIssue {
            line: location.map(|location| location.line),
            column: location.map(|location| location.column),
            severity,
            message,
        });
    }
    issues.sort();
    issues.dedup();
    issues
}

/// A parse error as an issue, its position taken from the `line N, column M: ` prefix.
fn parse_error_issue(message: &str) -> ScriptIssue {
    let located = message.strip_prefix(PARSE_ERROR_LINE_PREFIX).and_then(|rest| {
        let (line, rest) = rest.split_once(PARSE_ERROR_COLUMN_SEPARATOR)?;
        let (column, text) = rest.split_once(PARSE_ERROR_MESSAGE_SEPARATOR)?;
        Some((line.parse().ok()?, column.parse().ok()?, text))
    });
    match located {
        Some((line, column, text)) => {
            ScriptIssue { line: Some(line), column: Some(column), severity: ScriptSeverity::Error, message: text.to_owned() }
        }
        None => ScriptIssue { line: None, column: None, severity: ScriptSeverity::Error, message: message.to_owned() },
    }
}

/// The script text in its canonical layout.
pub fn canonical_script(text: &str) -> Result<String, String> {
    ChaosScript::parse(text).map(|script| script.to_canonical_string()).map_err(|error| format!("{error:#}"))
}

/// Script bytes as text: the client reads them as Latin-1.
pub fn latin1_decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// Script text as Latin-1 bytes; fails on characters outside Latin-1.
pub fn latin1_encode(text: &str) -> Result<Vec<u8>, String> {
    text.chars()
        .map(|character| {
            u8::try_from(u32::from(character))
                .map_err(|_| format!("character {character:?} is outside Latin-1 and cannot be stored"))
        })
        .collect()
}

/// Decodes a DDS or TGA payload to RGBA.
pub fn decode_texture(payload: &[u8]) -> Result<image::RgbaImage, String> {
    let format = if payload.starts_with(DDS_MAGIC) { ECImageFormat::DDS } else { ECImageFormat::TGA };
    let file = TextureFile {
        metadata: TextureItem::absent(),
        is_ec: true,
        format,
        props: None,
        raw_data: Arc::from(payload),
        image_data_offset: 0,
    };
    file.decode_to_rgba8().map_err(|error| format!("{error:#}"))
}

/// A NIF image, as the client's `NiNIFImageReader::ReadFile` accepts it: a NIF file whose only top object is an
/// `NiPixelData`. Returns its first stored level.
pub fn decode_nif_image(payload: &[u8]) -> Result<image::RgbaImage, String> {
    let nif = NifFile::read(payload).map_err(|error| error.to_string())?;
    let [root] = nif.roots.as_slice() else {
        return Err(format!("{} top objects, a NIF image has one", nif.roots.len()));
    };
    let pixels = match root.index().and_then(|index| nif.blocks.get(index)) {
        Some(Block::NiPixelData(pixels)) => pixels,
        _ => return Err("the top object is not an NiPixelData".to_owned()),
    };
    let (width, height, rgba) = decode_pixel_level(pixels, 0)
        .ok_or_else(|| format!("NiPixelData format {} is not decoded", pixels.pixel_format.format.0))?;
    image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| "NiPixelData level 0 is truncated".to_owned())
}

/// Reads a file to import into the package: its lowercase base name and payload. Scripts may have issues
/// (shown afterwards); textures must decode; NIF files must parse.
pub fn read_import(path: &Path) -> Result<(String, Arc<[u8]>), String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .ok_or_else(|| format!("{}: not a file", path.display()))?;
    if !IMPORTABLE_EXTENSIONS.iter().any(|extension| has_extension(&name, extension)) {
        return Err(format!("{name}: only {} files can be imported", IMPORTABLE_EXTENSIONS.join(", ")));
    }
    let payload = std::fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    if has_extension(&name, EMS_EXTENSION) {
        if FileKind::detect(&payload, Some(&name)) != FileKind::Chaos {
            return Err(format!("{name}: not a Chaos script (it must start with particlesystem)"));
        }
    } else if is_nif_name(&name) {
        gamebryo_nif::NifFile::read(&payload).map_err(|error| format!("{name}: {error}"))?;
    } else {
        decode_texture(&payload).map_err(|error| format!("{name}: {error}"))?;
    }
    Ok((name, Arc::from(payload)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "particlesystem \"spark\" 1\n{\neventsequence \"a\"\n{\ntype = point\ntexture = \"Spark.TGA\"\n}\n}\n";

    fn row(name: Option<&str>, kind: FileKind, status: FileStatus) -> FileRow {
        FileRow { hash: name.map_or(0xABCD, effect_texture_hash), name: name.map(str::to_owned), kind, size: 1, status, references: Vec::new() }
    }

    #[test]
    fn kinds_follow_the_payload_and_the_name() {
        assert_eq!(FileKind::detect(SCRIPT.as_bytes(), None), FileKind::Chaos);
        assert_eq!(FileKind::detect(b"\0\0\x02", Some("glow.TGA")), FileKind::Tga);
        assert_eq!(FileKind::detect(b"\0\0\x02", None), FileKind::Other);
        assert_eq!(FileKind::detect(b"DDS \0", Some("x.tga")), FileKind::Dds);
        assert_eq!(FileKind::detect(b"", None), FileKind::Empty);
        assert!(is_nif_name("smoke.NIF") && !is_nif_name("smoke.ems"));
    }

    #[test]
    fn scripts_list_their_files() {
        assert_eq!(script_references(SCRIPT.as_bytes()), ["Spark.TGA"]);
        assert!(script_references(b"not a script").is_empty());
    }

    #[test]
    fn file_filter_matches_name_hash_kind_and_status() {
        let rows = vec![
            row(Some("zeta.ems"), FileKind::Chaos, FileStatus::Original),
            row(None, FileKind::Nif, FileStatus::Original),
            row(Some("Alpha.tga"), FileKind::Tga, FileStatus::Modified),
        ];
        assert_eq!(FileFilter::default().visible(&rows), [2, 0, 1], "by name, unnamed last");
        let filter = |filter: FileFilter| filter.visible(&rows);
        assert_eq!(filter(FileFilter { text: "ALPHA".into(), ..Default::default() }), [2]);
        assert_eq!(filter(FileFilter { text: "0xabcd".into(), ..Default::default() }), [1]);
        assert_eq!(filter(FileFilter { kind: Some(FileKind::Chaos), ..Default::default() }), [0]);
        assert_eq!(filter(FileFilter { unnamed_only: true, ..Default::default() }), [1]);
        assert_eq!(filter(FileFilter { changed_only: true, ..Default::default() }), [2]);
    }

    #[test]
    fn script_checks_locate_parse_errors_and_missing_files() {
        let missing = |_: &str| false;
        let issues = check_script(SCRIPT.as_bytes(), Some(&missing));
        let missing_texture = issues.iter().find(|issue| issue.message.contains("Spark.TGA")).expect("missing file");
        assert_eq!((missing_texture.line, missing_texture.severity), (Some(6), ScriptSeverity::Error));
        let present = |_: &str| true;
        assert!(check_script(SCRIPT.as_bytes(), Some(&present)).iter().all(|issue| issue.severity != ScriptSeverity::Error));

        let broken = "particlesystem \"x\" 1\n{\neventsequence \"a\"\n{\ntexture = XYZ(\n";
        let issues = check_script(broken.as_bytes(), None);
        let error = issues.iter().find(|issue| issue.severity == ScriptSeverity::Error).expect("parse error");
        assert!(error.line.is_some() && error.column.is_some(), "{error}");
    }

    #[test]
    fn parse_error_prefix_gives_the_position() {
        let issue = parse_error_issue("line 3, column 7: unexpected end");
        assert_eq!((issue.line, issue.column, issue.message.as_str()), (Some(3), Some(7), "unexpected end"));
        assert_eq!(parse_error_issue("something else").line, None);
    }

    #[test]
    fn latin1_round_trips_and_rejects_other_characters() {
        let bytes = b"caf\xe9";
        assert_eq!(latin1_encode(&latin1_decode(bytes)).unwrap(), bytes);
        assert!(latin1_encode("\u{2603}").is_err());
    }

    #[test]
    fn canonical_format_parses_the_text() {
        let canonical = canonical_script(SCRIPT).unwrap();
        assert!(canonical.contains("Spark.TGA"));
        assert!(canonical_script("particlesystem \"x\" 1 {").is_err());
    }

    #[test]
    fn imports_check_extension_and_content() {
        let dir = super::super::effects_editor::tests::scratch_dir("import");
        let script = dir.join("Spark.EMS");
        std::fs::write(&script, SCRIPT).unwrap();
        let (name, payload) = read_import(&script).unwrap();
        assert_eq!((name.as_str(), &*payload), ("spark.ems", SCRIPT.as_bytes()));
        let text = dir.join("notes.txt");
        std::fs::write(&text, "x").unwrap();
        assert!(read_import(&text).is_err());
        let bad_nif = dir.join("bad.nif");
        std::fs::write(&bad_nif, "Gamebryo File Format, Version 20.0.0.4\n").unwrap();
        assert!(read_import(&bad_nif).is_err());
        let bad_texture = dir.join("bad.dds");
        std::fs::write(&bad_texture, "DDS nope").unwrap();
        assert!(read_import(&bad_texture).is_err());
        let mut tga = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 4]))
            .write_to(&mut std::io::Cursor::new(&mut tga), image::ImageFormat::Tga)
            .unwrap();
        let texture = dir.join("ok.tga");
        std::fs::write(&texture, &tga).unwrap();
        let (_, payload) = read_import(&texture).unwrap();
        assert_eq!(decode_texture(&payload).unwrap().dimensions(), (2, 2));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
