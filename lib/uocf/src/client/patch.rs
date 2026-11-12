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
//! - [`ExeEdit::LoginServer`] ([`LoginServerEdit`]: one host per host site, one port per port site). Enhanced
//!   Client: each public login host slot, zero filled (each host within its own slot), and each public port imm16
//!   ([`crate::client::login`]); the internal test address is left alone. Kingdom Reborn: the fixed address's
//!   ip imm32 (dotted IPv4 only: the code holds a number, not a string the client resolves) and port imm16.
//!   Classic is refused: it reads `login.cfg` ([`crate::client::login_servers`]).
//! - [`ExeEdit::NoEncryption`]: the bytes of each network cipher ([`crate::client::encryption`]): NOPs over the
//!   login cipher's XOR (and the Enhanced Client's game cipher XORs), and a `JMP` for the `JZ` that enters the 0xE3
//!   key exchange ciphers' loop. Every cipher of the family must be located exactly once; Classic is refused.
//!   KR also bypasses the key exchange wait, sends the auth-key seed and installs Huffman before sending 0x91.

crate::eyre_imports!();
use std::path::{Path, PathBuf};

use crate::client::classic_facets::{
    classic_split_widths, classic_stock_facet, validate_classic_size, validate_classic_wrap,
    CLASSIC_OLD_FELUCCA_WIDTH, CLASSIC_RECORD_WRAP_HEIGHT_OFFSET, CLASSIC_RECORD_WRAP_WIDTH_OFFSET,
    CLASSIC_STOCK_WIDTHS,
};
use crate::client::facet_dimensions::{
    validate_facet_size, ClientExeInspection, ClientFamily, ExeValues, FacetValues, CLASSIC_GOLD_SHA256, DISABLED_SPLIT_X,
    FACET_RECORD_HEIGHT_OFFSET, FACET_RECORD_WIDTH_OFFSET, SPLIT_FACETS, STOCK_SPLIT_X,
};
use crate::client::encryption::{CipherSiteKind, EncryptionSites, EncryptionState};
use crate::client::login::{LoginHost, LoginServerEdit, LoginSites, LoginValues};
use crate::client::pe::pe_checksum;
use crate::client::version::ExeVersion;
use crate::client::login_servers::{validate_login_port, HostKind, validate_login_host};
use crate::enhanced::facet_sector::{FacetGeometry, CLIENT_FACET_DIMENSIONS};

/// The manifest of a patched copy: `<copy>` plus this suffix.
pub const MANIFEST_SUFFIX: &str = ".facet-dims.json";
pub const MANIFEST_FORMAT: &str = "uocf-facet-dims";
/// Version 2: the family and the steps with their edits. Version 1 (one facet size and split per patch, no steps)
/// is still read.
pub const MANIFEST_VERSION: u32 = 2;
pub const MANIFEST_VERSION_1: u32 = 1;
const PATCH_FIELD_SIZE: usize = size_of::<u32>();

/// What a patch warning is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatchWarningKind {
    /// A facet size the client takes but that is unusual (not a multiple of 64, larger than stock, ...).
    Size,
    /// The Enhanced Client split: not found, or outside a facet.
    Split,
    /// The Classic wrap seam disagrees with a facet's wrap.
    Seam,
    /// The login server: the input's is not the stock one, or a host name the client resolves.
    Login,
    /// The input's stored PE checksum is not the computed one (the copy gets the right one).
    Checksum,
}

/// An allowed but unusual request, and what it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchWarning {
    pub kind: PatchWarningKind,
    pub text: String,
}

impl PatchWarning {
    pub fn new(kind: PatchWarningKind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into() }
    }
}

impl std::fmt::Display for PatchWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExeEdit {
    /// A new size for one facet.
    FacetSize { facet: u8, width: u32, height: u32 },
    Split(SplitEdit),
    /// A new wrap rectangle for one facet (Classic only).
    ClassicWrap { facet: u8, wrap_width: u32, wrap_height: u32 },
    /// A new built-in login server (Enhanced Client, Kingdom Reborn): one host per host site, one port per port site.
    LoginServer(LoginServerEdit),
    /// Plaintext login and game traffic (Enhanced Client, Kingdom Reborn): every network cipher patched out,
    /// and KR's mandatory key exchange replaced with immediate game login.
    NoEncryption,
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
            Self::LoginServer(login) => login.describe(),
            Self::NoEncryption => "no encryption".to_owned(),
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
    pub warnings: Vec<PatchWarning>,
    /// The values of the patched copy.
    pub expected: ExeValues,
    /// The chain: the stock build the copy derives from, its values when known, and the earlier steps (empty for a
    /// stock input).
    pub original_sha256: String,
    pub original_values: Option<ExeValues>,
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
    /// The values of the stock executable the chain started from (`None` in manifests written before they were
    /// recorded): the sizes a "stock" reset goes back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_values: Option<ExeValues>,
    /// The PE checksum written.
    pub checksum: u32,
    /// The file and product versions of the patched copy's version resource (information only; `None` without
    /// one, and in manifests written before it was recorded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe_version: Option<ExeVersion>,
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
    pub warnings: Vec<PatchWarning>,
    pub manifest: ExeManifest,
}

/// A field write of a plan, before comparing it with the input.
struct FieldWrite {
    offset: usize,
    new: Vec<u8>,
    field: String,
    edit: usize,
}

impl FieldWrite {
    fn u32(offset: usize, value: u32, field: String, edit: usize) -> Self {
        Self { offset, new: value.to_le_bytes().to_vec(), field, edit }
    }
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

/// True when the `recorded` values of a manifest are the `current` values of the executable. A manifest written
/// before the login values or the encryption state were recorded has none, and then they do not compare.
pub fn recorded_values_match(recorded: &ExeValues, current: &ExeValues) -> bool {
    recorded.facets == current.facets
        && recorded.split_x == current.split_x
        && (recorded.login.is_none() || recorded.login == current.login)
        && (recorded.encryption.is_none() || recorded.encryption == current.encryption)
}

/// Older KR plaintext patches removed both ciphers but left the game handshake stock. Their manifests call
/// that state `Removed`; accept it only when the newly added handshake is the sole site still stock.
fn recorded_inspection_values_match(recorded: &ExeValues, inspection: &ClientExeInspection) -> bool {
    let mut current = inspection.values();
    if inspection.family == ClientFamily::KingdomReborn
        && recorded.encryption == Some(EncryptionState::Removed)
        && current.encryption == Some(EncryptionState::Partial)
        && inspection.encryption.as_ref().is_some_and(|sites| {
            sites.is_complete()
                && sites.sites.iter().filter(|site| !site.removed).map(|site| site.kind)
                    .eq([CipherSiteKind::GameLoginHandshake])
        })
    {
        current.encryption = Some(EncryptionState::Removed);
    }
    recorded_values_match(recorded, &current)
}

/// The ciphers a no-encryption edit patches: every one of the family, each located exactly once.
fn checked_encryption_sites(before: &ClientExeInspection) -> eyre::Result<&EncryptionSites> {
    let Some(sites) = &before.encryption else {
        eyre::bail!(
            "removing the encryption is supported for the Enhanced Client and Kingdom Reborn only: the {} client's \
             ciphers are not located",
            before.family.label()
        );
    };
    eyre::ensure!(
        sites.is_complete(),
        "the encryption code was not located in this build: {}: each cipher's instruction pattern must match exactly \
         once (nothing is patched at a guessed offset)",
        sites.describe_unlocated()
    );
    Ok(sites)
}

/// Why a login server edit is refused for `family`.
fn login_refusal(family: ClientFamily) -> String {
    match family {
        ClientFamily::Classic => "the Classic client reads its login server from login.cfg (or the LoginServerFile that \
                                  uo.cfg names), not from the executable: edit that file (`uocf-cli login set`)"
            .to_owned(),
        ClientFamily::KingdomReborn => "the login address code was not located in this build: no single fixed \
                                        address store (MOV imm32 ip, MOV imm16 port, JMP to the LEA of the pair)"
            .to_owned(),
        ClientFamily::Enhanced => "the login address code was not located in this build: no single fixed address store \
                                   followed by the public host pushes and port stores"
            .to_owned(),
    }
}

/// `count` sites, in words.
fn site_count_text(count: usize, what: &str) -> String {
    format!("{count} {what} site{}", if count == 1 { "" } else { "s" })
}

/// The login sites `login` writes, checked: one host and port per site, the ports, and each host against its site
/// (Enhanced Client: its slot's capacity; Kingdom Reborn: dotted IPv4).
fn checked_login_sites<'a>(before: &'a ClientExeInspection, login: &LoginServerEdit) -> eyre::Result<&'a LoginSites> {
    let sites = match (&before.login, before.family) {
        (Some(sites), ClientFamily::Enhanced) if !sites.hosts.is_empty() && !sites.ports.is_empty() => sites,
        (Some(sites), ClientFamily::KingdomReborn) => sites,
        _ => eyre::bail!("{}", login_refusal(before.family)),
    };
    let (host_sites, port_sites) = (sites.host_site_count(), sites.port_site_count());
    eyre::ensure!(
        login.hosts.len() == host_sites && login.ports.len() == port_sites,
        "{} and {} given: this build has {} and {}: give one per site (the same host or port for each to use one \
         address)",
        site_count_text(login.hosts.len(), "host"),
        site_count_text(login.ports.len(), "port"),
        site_count_text(host_sites, "host"),
        site_count_text(port_sites, "port")
    );
    for port in &login.ports {
        validate_login_port(*port)?;
    }
    if sites.uses_fixed_address() {
        let host = &login.hosts[0];
        eyre::ensure!(
            validate_login_host(host.as_str())? == HostKind::Ipv4,
            "host `{host}`: Kingdom Reborn needs a dotted IPv4 address: UOKR.exe stores its login address as a 32-bit \
             number in an instruction (MOV imm32), not as a string it resolves, so a host name cannot be written"
        );
        return Ok(sites);
    }
    for (number, (slot, host)) in sites.hosts.iter().zip(&login.hosts).enumerate() {
        eyre::ensure!(
            host.as_str().len() < slot.capacity,
            "host `{host}` has {} characters: host slot {} (VA {:#x}, `{}`) takes at most {} (a {}-byte slot): use a \
             shorter host (a shorter IPv4 address or name)",
            host.as_str().len(),
            number + 1,
            slot.va,
            slot.host,
            slot.capacity - 1,
            slot.capacity
        );
        eyre::ensure!(
            slot.references == 1,
            "host slot {} (VA {:#x}) is referenced {} times in the image: writing it could change other code",
            number + 1,
            slot.va,
            slot.references
        );
    }
    Ok(sites)
}

/// Validates `patch` against the executable `bytes` and plans its byte changes, without writing anything (the dry
/// run). `previous` is the manifest of the input when it is an earlier patched copy.
pub fn plan_patch(bytes: &[u8], patch: &ExePatch, previous: Option<&ExeManifest>) -> eyre::Result<PatchPlan> {
    let before = ClientExeInspection::inspect(bytes)?;
    plan_patch_inspected(bytes, &before, patch, previous)
}

/// [`plan_patch`] with the inspection of `bytes` already made (a GUI plans each edit without inspecting the file
/// again). `before` must be [`ClientExeInspection::inspect`] of these `bytes`.
pub fn plan_patch_inspected(
    bytes: &[u8],
    before: &ClientExeInspection,
    patch: &ExePatch,
    previous: Option<&ExeManifest>,
) -> eyre::Result<PatchPlan> {
    eyre::ensure!(bytes.len() == before.file_size, "the inspection is not of this executable: its size differs");
    let before = before.clone();
    eyre::ensure!(!before.pe.is_signed(), "the executable is Authenticode signed; a patch would break the signature");
    eyre::ensure!(!patch.edits.is_empty(), "the patch has no edits");
    let previous = previous.filter(|manifest| manifest.patched_sha256 == before.sha256);
    let (original_sha256, original_values, build, previous_steps) = match previous {
        Some(manifest) => {
            eyre::ensure!(
                recorded_inspection_values_match(&manifest.values, &before),
                "the executable's facet, login or encryption values differ from the ones its manifest records"
            );
            let build = manifest.build.clone();
            (manifest.original_sha256.clone(), manifest.original_values.clone(), build, manifest.steps.clone())
        }
        None => {
            eyre::ensure!(
                before.is_stock(),
                "the executable's facet values are not the stock ones and no manifest of an earlier patch records them"
            );
            let build = before.known.map(|build| build.name.to_owned());
            (before.sha256.clone(), Some(before.values()), build, Vec::new())
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
    let mut login_edit = None;
    let mut encryption_edit = None;
    if !before.checksum_valid() {
        warnings.push(PatchWarning::new(
            PatchWarningKind::Checksum,
            format!(
                "the input's stored PE checksum {:#010x} is not the computed {:#010x}; the copy gets a correct one",
                before.pe.stored_checksum, before.computed_checksum
            ),
        ));
    }
    for (index, edit) in patch.edits.iter().enumerate() {
        match *edit {
            ExeEdit::FacetSize { facet, width, height } => {
                let facet_index = checked_facet(facet, count)?;
                eyre::ensure!(sized[facet_index].is_none(), "two size edits for facet {facet}");
                sized[facet_index] = Some(index);
                let size_warnings =
                    if classic { validate_classic_size(width, height)? } else { validate_facet_size(width, height)? };
                warnings.extend(size_warnings.into_iter().map(|text| PatchWarning::new(PatchWarningKind::Size, text)));
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
            ExeEdit::LoginServer(ref login) => {
                eyre::ensure!(login_edit.is_none(), "two login server edits");
                let sites = checked_login_sites(&before, login)?;
                if !sites.is_stock() && previous.is_none() {
                    let values = sites.values();
                    let ports: Vec<String> = values.ports.iter().map(u16::to_string).collect();
                    warnings.push(PatchWarning::new(
                        PatchWarningKind::Login,
                        format!(
                            "the built-in login server is already not the stock one (hosts {}; ports {})",
                            values.hosts.join(", "),
                            ports.join(", ")
                        ),
                    ));
                }
                let mut names: Vec<&LoginHost> = Vec::new();
                for host in &login.hosts {
                    if validate_login_host(host.as_str())? == HostKind::Name && !names.contains(&host) {
                        names.push(host);
                    }
                }
                for host in names {
                    warnings.push(PatchWarning::new(
                        PatchWarningKind::Login,
                        format!("`{host}` is a host name: the client resolves it (gethostbyname) at each login"),
                    ));
                }
                login_edit = Some((index, login));
            }
            ExeEdit::NoEncryption => {
                eyre::ensure!(encryption_edit.is_none(), "two no-encryption edits");
                encryption_edit = Some((index, checked_encryption_sites(&before)?));
            }
        }
        let facet = match *edit {
            ExeEdit::FacetSize { facet, .. } | ExeEdit::ClassicWrap { facet, .. } => Some(facet),
            ExeEdit::Split(_) | ExeEdit::LoginServer(_) | ExeEdit::NoEncryption => None,
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
    if let Some((_, login)) = login_edit {
        expected.login = Some(LoginValues {
            hosts: login.hosts.iter().map(|host| host.as_str().to_owned()).collect(),
            ports: login.ports.clone(),
        });
    }
    if encryption_edit.is_some() {
        expected.encryption = Some(EncryptionState::Removed);
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
                    writes.push(FieldWrite::u32(record + field_offset, value, field, edit));
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
                    writes.push(FieldWrite::u32(offset, value, format!("facet 0 ({}) {what}", values.name), edit));
                }
            }
        }
    }
    if let (Some((edit, x)), Some(split)) = (split_edit, &before.split) {
        let what = if classic { "wrap seam" } else { "split" };
        for (number, offset) in split.offsets.iter().enumerate() {
            let field = format!("{what} x, compare {}", number + 1);
            writes.push(FieldWrite::u32(*offset, x, field, edit));
        }
    }
    if let (Some((edit, login)), Some(sites)) = (login_edit, &before.login) {
        if sites.uses_fixed_address() {
            let ip: std::net::Ipv4Addr = login.hosts[0].as_str().parse().wrap_err("the checked IPv4 host")?;
            let fixed = &sites.fixed;
            writes.push(FieldWrite::u32(fixed.ip_offset, u32::from(ip), "login address (ip imm32)".to_owned(), edit));
            let new = login.ports[0].to_le_bytes().to_vec();
            writes.push(FieldWrite { offset: fixed.port_offset, new, field: "login port (imm16)".to_owned(), edit });
        }
        for (number, (slot, host)) in sites.hosts.iter().zip(&login.hosts).enumerate() {
            let mut new = host.as_str().as_bytes().to_vec();
            new.resize(slot.capacity, 0);
            let field = format!("login host {} ({}-byte slot)", number + 1, slot.capacity);
            writes.push(FieldWrite { offset: slot.offset, new, field, edit });
        }
        for (number, (site, port)) in sites.ports.iter().zip(&login.ports).enumerate() {
            let field = format!("login port {} (imm16)", number + 1);
            writes.push(FieldWrite { offset: site.offset, new: port.to_le_bytes().to_vec(), field, edit });
        }
    }
    if let Some((edit, sites)) = encryption_edit {
        for site in &sites.sites {
            let field = site.kind.patch_label().to_owned();
            writes.push(FieldWrite { offset: site.offset, new: site.patched().to_vec(), field, edit });
        }
    }

    let mut patched = bytes.to_vec();
    let mut changes = Vec::new();
    for write in writes {
        let range = write.offset..write.offset + write.new.len();
        let new = write.new;
        if bytes[range.clone()] == new[..] {
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
    Ok(PatchPlan {
        before,
        patch: patch.clone(),
        changes,
        warnings,
        expected,
        original_sha256,
        original_values,
        build,
        previous_steps,
    })
}

/// The warnings about the split or seam of the patched values.
fn split_warnings(before: &ClientExeInspection, expected: &ExeValues, sized: &[Option<usize>]) -> Vec<PatchWarning> {
    let split_facets = || expected.facets.iter().filter(|facet| SPLIT_FACETS.contains(&facet.facet));
    let mut warnings = Vec::new();
    match (before.family, expected.split_x) {
        (ClientFamily::Enhanced, None) if split_facets().any(|facet| sized[usize::from(facet.facet)].is_some()) => {
            warnings.push(PatchWarning::new(PatchWarningKind::Split, format!(
                "no x = {STOCK_SPLIT_X} split sequence found: in an Enhanced Client build, the split code differs \
                 from the known sequence"
            )));
        }
        (ClientFamily::Classic, Some(x)) => {
            for facet in split_facets() {
                let FacetValues { width, wrap_width, .. } = facet;
                let wrap = wrap_width.unwrap_or(*width);
                if wrap.min(*width) != x.min(*width) {
                    warnings.push(PatchWarning::new(PatchWarningKind::Seam, format!(
                        "the wrap seam x {x} is not facet {} ({})'s wrap width {wrap}: the radar and the walk test \
                         assume the facet stops wrapping at the seam",
                        facet.facet, facet.name
                    )));
                }
            }
        }
        (_, Some(x)) if x != DISABLED_SPLIT_X => {
            for facet in split_facets().filter(|facet| x >= facet.width) {
                warnings.push(PatchWarning::new(PatchWarningKind::Split, format!(
                    "split x {x} is not inside facet {} ({})'s width {}: nothing is split",
                    facet.facet, facet.name, facet.width
                )));
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
    let login_offsets = |inspection: &ClientExeInspection| inspection.login.as_ref().map(LoginSites::offsets);
    let cipher_offsets = |inspection: &ClientExeInspection| inspection.encryption.as_ref().map(EncryptionSites::offsets);
    eyre::ensure!(
        after.family == before.family
            && offsets(&after) == offsets(&before)
            && split_offsets(&after) == split_offsets(&before)
            && startup(&after) == startup(&before)
            && login_offsets(&after) == login_offsets(&before)
            && cipher_offsets(&after) == cipher_offsets(&before),
        "internal error: the patched copy's tables, split, startup values, login sites or ciphers are located elsewhere"
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
        original_values: plan.original_values,
        checksum: after.pe.stored_checksum,
        exe_version: after.version,
        steps,
    };
    Ok(PatchedClientExe { bytes: patched, before, after, changes: plan.changes, warnings: plan.warnings, manifest })
}

/// The stock size of a facet, and its stock wrap (Classic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockFacet {
    pub size: FacetGeometry,
    pub wrap: Option<FacetGeometry>,
}

/// The stock size (and Classic wrap) of each facet of the build `inspection` is, or derives from (`manifest`: its
/// manifest, when it is a patched copy); `None` for a facet whose stock size is not known.
///
/// - A stock file: its own table (every Classic generation: UO Gold's 6144-wide Felucca and Trammel, Mondain's
///   Legacy on 7168).
/// - A copy whose manifest records the original values: those.
/// - Otherwise Enhanced Client and Kingdom Reborn: the shipped table ([`CLIENT_FACET_DIMENSIONS`]). Classic: the
///   generation's values; Felucca and Trammel are 7168 wide where Trammel reads map1 (6.0.14.3 on), 6144 when the
///   manifest's original build is UO Gold, and unknown in an older build of unknown origin (UO Gold to 5.x share
///   the layout).
pub fn stock_facets(inspection: &ClientExeInspection, manifest: Option<&ExeManifest>) -> Vec<Option<StockFacet>> {
    let manifest = manifest.filter(|manifest| manifest.patched_sha256 == inspection.sha256);
    let from_values = |values: &ExeValues| -> Vec<Option<StockFacet>> {
        values
            .facets
            .iter()
            .map(|facet| {
                let wrap = facet.wrap_width.zip(facet.wrap_height).map(|(width, height)| FacetGeometry::new(width, height));
                Some(StockFacet { size: FacetGeometry::new(facet.width, facet.height), wrap })
            })
            .collect()
    };
    if inspection.is_stock() {
        return from_values(&inspection.values());
    }
    if let Some(values) = manifest.and_then(|manifest| manifest.original_values.as_ref()) {
        return from_values(values);
    }
    let count = inspection.facet_count();
    match inspection.family {
        ClientFamily::Enhanced | ClientFamily::KingdomReborn => (0..count)
            .map(|facet| CLIENT_FACET_DIMENSIONS.get(facet).map(|size| StockFacet { size: *size, wrap: None }))
            .collect(),
        ClientFamily::Classic => {
            let widths = classic_split_widths(inspection.table());
            let gold = manifest.is_some_and(|manifest| manifest.original_sha256 == CLASSIC_GOLD_SHA256);
            let split_width = match widths {
                [only] => Some(*only),
                _ if gold => Some(CLASSIC_OLD_FELUCCA_WIDTH),
                _ => None,
            };
            (0..count)
                .map(|facet| {
                    // The width argument is read for Felucca and Trammel only.
                    let width = if SPLIT_FACETS.contains(&(facet as u8)) { split_width? } else { CLASSIC_STOCK_WIDTHS[0] };
                    classic_stock_facet(facet, width).map(|(size, wrap)| StockFacet { size, wrap: Some(wrap) })
                })
                .collect()
        }
    }
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
    use crate::client::classic_facets::tests::{seam_imm_offsets, stock_size, synthetic_classic};
    use crate::client::facet_dimensions::tests::{synthetic_client, synthetic_kr_client, NEW_SPLIT};
    use crate::client::facet_dimensions::{FACET_SIDE_ALIGNMENT, MAX_FACET_SIDE, STOCK_FACET_COUNT};
    use crate::client::encryption::tests::{synthetic_enhanced_encrypted_client, synthetic_kr_encrypted_client};
    use crate::client::login::tests::{synthetic_kr_login_client, synthetic_login_client};
    use crate::client::login::LOGIN_HOST_SLOT_SIZE;
    use crate::client::classic_facets::CLASSIC_STOCK_WRAP;
    use crate::client::sha256::sha256_hex;

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
            vec![good.clone(), size(3, NEW_WIDTH, NEW_WIDTH)],
            vec![ExeEdit::Split(SplitEdit::Set(DISABLED_SPLIT_X + 1))],
            vec![ExeEdit::Split(SplitEdit::Set(NEW_SPLIT)), ExeEdit::Split(SplitEdit::Disable)],
            vec![ExeEdit::ClassicWrap { facet: 3, wrap_width: 2048, wrap_height: 2048 }],
            vec![size(3, CLIENT_FACET_DIMENSIONS[3].width, CLIENT_FACET_DIMENSIONS[3].height)],
        ] {
            assert!(plan_patch(&image, &ExePatch { edits: bad.clone() }, None).is_err(), "{bad:?}");
        }
        assert!(plan_patch(&image, &ExePatch { edits: vec![good.clone()] }, None).is_ok());
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

    /// A login edit of the given hosts and ports, one per site.
    fn per_site(hosts: &[&str], ports: &[u16]) -> ExeEdit {
        let hosts = hosts.iter().map(|host| LoginHost::new(host).unwrap()).collect();
        ExeEdit::LoginServer(LoginServerEdit { hosts, ports: ports.to_vec() })
    }

    /// `host:port` on every login site of `image`.
    fn login(image: &[u8], host: &str, port: u16) -> ExeEdit {
        let sites = ClientExeInspection::inspect(image).unwrap().login.unwrap();
        ExeEdit::LoginServer(LoginServerEdit::uniform(&sites, LoginHost::new(host).unwrap(), port))
    }

    const LOCAL_HOST: &str = "127.0.0.1";
    const SHARD_HOST: &str = "10.0.0.7";
    const SHARD_PORT: u16 = 2593;
    const SECOND_SHARD_PORT: u16 = 2594;

    #[test]
    fn a_login_patch_writes_every_public_host_and_port() {
        let image = synthetic_login_client(true, false);
        let before = ClientExeInspection::inspect(&image).unwrap();
        let sites = before.login.clone().unwrap();
        let patch = ExePatch { edits: vec![login(&image, LOCAL_HOST, SHARD_PORT), size(5, 1280, 4160)] };
        let plan = plan_patch(&image, &patch, None).unwrap();
        assert_eq!(plan_patch_inspected(&image, &before, &patch, None).unwrap().changes, plan.changes);
        let patched = patch_client_exe(&image, &patch, None).unwrap();
        assert_plan_is_the_write(&image, &plan, &patched.bytes);
        assert_eq!(plan.changes, patched.changes, "the dry run is the write");
        // Two 16-byte host slots, two ports, the facet width, the checksum.
        let login_changes: Vec<(usize, usize)> = plan
            .changes
            .iter()
            .filter(|change| change.cause == ChangeCause::Edit(0))
            .map(|change| (change.offset, change.new.len()))
            .collect();
        let mut expected: Vec<(usize, usize)> = sites.hosts.iter().map(|slot| (slot.offset, LOGIN_HOST_SLOT_SIZE)).collect();
        expected.extend(sites.ports.iter().map(|site| (site.offset, size_of::<u16>())));
        assert_eq!(login_changes, expected);
        let after = patched.after.login.as_ref().unwrap();
        assert_eq!(after.public_servers(), [(LOCAL_HOST.to_owned(), SHARD_PORT), (LOCAL_HOST.to_owned(), SHARD_PORT)]);
        assert_eq!(after.fixed, sites.fixed, "the internal test address is kept");
        assert!(!after.is_stock());
        assert_eq!(patched.manifest.values.login, plan.expected.login);
        assert_eq!(patched.manifest.steps[0].edits[0].describe(), format!("login server {LOCAL_HOST}:{SHARD_PORT}"));
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);

        // The copy patches again with its manifest; a host name warns; the same address changes nothing.
        let renamed = patch_client_exe(&patched.bytes, &ExePatch { edits: vec![login(&image, "shard.example", SHARD_PORT)] }, Some(&patched.manifest)).unwrap();
        assert_eq!(renamed.warnings.iter().map(|warning| warning.kind).collect::<Vec<_>>(), [PatchWarningKind::Login]);
        assert_eq!(renamed.manifest.steps.len(), 2);
        let same = ExePatch { edits: vec![login(&image, LOCAL_HOST, SHARD_PORT)] };
        assert!(plan_patch(&patched.bytes, &same, Some(&patched.manifest)).is_err());

        // A copy patched elsewhere only at its login sites (as UOSA_eclo.exe) has stock facets: it patches again
        // without a manifest, with a warning that its login server is not stock.
        let mut foreign = image.clone();
        for slot in &sites.hosts {
            foreign[slot.offset..slot.offset + slot.capacity].fill(0);
            foreign[slot.offset..slot.offset + LOCAL_HOST.len()].copy_from_slice(LOCAL_HOST.as_bytes());
        }
        let foreign_plan = plan_patch(&foreign, &ExePatch { edits: vec![login(&image, SHARD_HOST, SHARD_PORT)] }, None).unwrap();
        assert_eq!(foreign_plan.warnings.iter().map(|warning| warning.kind).collect::<Vec<_>>(), [PatchWarningKind::Checksum, PatchWarningKind::Login]);

        // A manifest written before the login values were recorded still matches.
        let mut old = patched.manifest.clone();
        old.values.login = None;
        assert!(plan_patch(&patched.bytes, &ExePatch { edits: vec![size(6, 512, 512)] }, Some(&old)).is_ok());
        old.values.login = Some(LoginValues { hosts: vec![], ports: vec![] });
        assert!(plan_patch(&patched.bytes, &ExePatch { edits: vec![size(6, 512, 512)] }, Some(&old)).is_err());

        // The one-host build before Time of Legends.
        let old_build = synthetic_login_client(false, false);
        let patched = patch_client_exe(&old_build, &ExePatch { edits: vec![login(&old_build, LOCAL_HOST, SHARD_PORT)] }, None).unwrap();
        assert_eq!(patched.after.login.unwrap().public_servers(), [(LOCAL_HOST.to_owned(), SHARD_PORT)]);
    }

    #[test]
    fn each_login_site_takes_its_own_host_and_port() {
        let image = synthetic_login_client(true, false);
        let edit = per_site(&[LOCAL_HOST, SHARD_HOST], &[SHARD_PORT, SECOND_SHARD_PORT]);
        let patched = patch_client_exe(&image, &ExePatch { edits: vec![edit.clone()] }, None).unwrap();
        let after = patched.after.login.as_ref().unwrap();
        assert_eq!(after.public_servers(), [(LOCAL_HOST.to_owned(), SHARD_PORT), (SHARD_HOST.to_owned(), SECOND_SHARD_PORT)]);
        assert_eq!(
            patched.manifest.values.login,
            Some(LoginValues { hosts: vec![LOCAL_HOST.to_owned(), SHARD_HOST.to_owned()], ports: vec![SHARD_PORT, SECOND_SHARD_PORT] })
        );
        assert_eq!(edit.describe(), format!("login hosts 1 {LOCAL_HOST}, 2 {SHARD_HOST}; ports 1 {SHARD_PORT}, 2 {SECOND_SHARD_PORT}"));
        // Only the second host changes: one host write and nothing at the ports.
        let second = per_site(&[LOCAL_HOST, LOCAL_HOST], &[SHARD_PORT, SECOND_SHARD_PORT]);
        let plan = plan_patch(&patched.bytes, &ExePatch { edits: vec![second] }, Some(&patched.manifest)).unwrap();
        let fields: Vec<String> = plan.changes.iter().filter(|change| change.cause == ChangeCause::Edit(0)).map(|change| change.field.clone()).collect();
        assert_eq!(fields, [format!("login host 2 ({LOGIN_HOST_SLOT_SIZE}-byte slot)")]);

        // Each host is checked against its own slot: 13 characters fit the 16-byte first slot, not the 12-byte second.
        let short = synthetic_login_client(true, true);
        const LONG_HOST: &str = "192.168.100.1";
        const FITTING_HOST: &str = "192.168.1.1";
        assert!(plan_patch(&short, &ExePatch { edits: vec![per_site(&[LONG_HOST, FITTING_HOST], &[SHARD_PORT; 2])] }, None).is_ok());
        let error = plan_patch(&short, &ExePatch { edits: vec![per_site(&[FITTING_HOST, LONG_HOST], &[SHARD_PORT; 2])] }, None).unwrap_err().to_string();
        assert!(error.contains("slot 2"), "{error}");

        // Counts that differ from the sites are refused.
        for wrong in [
            per_site(&[LOCAL_HOST], &[SHARD_PORT; 2]),
            per_site(&[LOCAL_HOST; 3], &[SHARD_PORT; 2]),
            per_site(&[LOCAL_HOST; 2], &[SHARD_PORT]),
            per_site(&[], &[]),
        ] {
            let error = plan_patch(&image, &ExePatch { edits: vec![wrong] }, None).unwrap_err().to_string();
            assert!(error.contains("this build has 2 host sites and 2 port sites"), "{error}");
        }
        let one_host = synthetic_login_client(false, false);
        let error = plan_patch(&one_host, &ExePatch { edits: vec![per_site(&[LOCAL_HOST; 2], &[SHARD_PORT; 2])] }, None).unwrap_err().to_string();
        assert!(error.contains("1 host site and 1 port site"), "{error}");
    }

    #[test]
    fn kingdom_reborn_patches_its_fixed_ipv4_address() {
        let image = synthetic_kr_login_client();
        let before = ClientExeInspection::inspect(&image).unwrap();
        let sites = before.login.clone().unwrap();
        assert!(sites.uses_fixed_address() && sites.is_stock());
        assert_eq!(before.values().login, Some(LoginValues { hosts: vec![sites.fixed.ip.to_string()], ports: vec![sites.fixed.port] }));
        let patch = ExePatch { edits: vec![login(&image, SHARD_HOST, SHARD_PORT)] };
        let plan = plan_patch(&image, &patch, None).unwrap();
        let patched = patch_client_exe(&image, &patch, None).unwrap();
        assert_plan_is_the_write(&image, &plan, &patched.bytes);
        let fields: Vec<(&str, usize)> = plan
            .changes
            .iter()
            .filter(|change| change.cause == ChangeCause::Edit(0))
            .map(|change| (change.field.as_str(), change.offset))
            .collect();
        assert_eq!(fields, [("login address (ip imm32)", sites.fixed.ip_offset), ("login port (imm16)", sites.fixed.port_offset)]);
        // The address is a host-order u32, little endian.
        let ip: std::net::Ipv4Addr = SHARD_HOST.parse().unwrap();
        assert_eq!(patched.bytes[sites.fixed.ip_offset..][..4], u32::from(ip).to_le_bytes());
        let after = patched.after.login.as_ref().unwrap();
        assert_eq!((after.fixed.ip, after.fixed.port), (ip, SHARD_PORT));
        assert!(!after.is_stock());
        assert_eq!(patched.manifest.values.login, Some(LoginValues { hosts: vec![SHARD_HOST.to_owned()], ports: vec![SHARD_PORT] }));
        // Patches again with its manifest; a host name cannot be written.
        assert!(plan_patch(&patched.bytes, &ExePatch { edits: vec![login(&image, LOCAL_HOST, SHARD_PORT)] }, Some(&patched.manifest)).is_ok());
        let error = plan_patch(&image, &ExePatch { edits: vec![login(&image, "shard.example", SHARD_PORT)] }, None).unwrap_err().to_string();
        assert!(error.contains("dotted IPv4") && error.contains("imm32"), "{error}");
        let error = plan_patch(&image, &ExePatch { edits: vec![per_site(&[SHARD_HOST; 2], &[SHARD_PORT; 2])] }, None).unwrap_err().to_string();
        assert!(error.contains("1 host site and 1 port site"), "{error}");
    }

    #[test]
    fn login_patches_are_refused_where_they_cannot_apply() {
        let image = synthetic_login_client(true, false);
        let edit = ExePatch { edits: vec![per_site(&[LOCAL_HOST], &[SHARD_PORT])] };
        let error = |image: &[u8], patch: &ExePatch| plan_patch(image, patch, None).unwrap_err().to_string();
        assert!(error(&synthetic_classic(6, 2), &edit).contains("login.cfg"));
        assert!(error(&synthetic_client(Some(STOCK_SPLIT_X), false), &edit).contains("not located"));
        assert!(plan_patch(&image, &ExePatch { edits: vec![login(&image, LOCAL_HOST, 0)] }, None).is_err());
        let twice = ExePatch { edits: vec![login(&image, LOCAL_HOST, 1), login(&image, LOCAL_HOST, 2)] };
        assert!(plan_patch(&image, &twice, None).is_err());
        let uniform = ExePatch { edits: vec![login(&image, LOCAL_HOST, SHARD_PORT)] };
        let serialized = serde_json::to_string(&uniform.edits).unwrap();
        assert_eq!(serialized, r#"[{"login_server":{"hosts":["127.0.0.1","127.0.0.1"],"ports":[2593,2593]}}]"#);
        assert_eq!(serde_json::from_str::<Vec<ExeEdit>>(&serialized).unwrap(), uniform.edits);
        // A manifest step written before per-site edits reads as one host and one port.
        let single = r#"[{"login_server":{"host":"127.0.0.1","port":2593}}]"#;
        assert_eq!(serde_json::from_str::<Vec<ExeEdit>>(single).unwrap(), edit.edits);
    }

    #[test]
    fn no_encryption_patches_every_cipher_of_the_family() {
        let patch = ExePatch { edits: vec![ExeEdit::NoEncryption] };
        for image in [synthetic_kr_encrypted_client(), synthetic_enhanced_encrypted_client()] {
            let before = ClientExeInspection::inspect(&image).unwrap();
            let sites = before.encryption.clone().unwrap();
            assert_eq!(before.values().encryption, Some(EncryptionState::Encrypted));
            let plan = plan_patch(&image, &patch, None).unwrap();
            let patched = patch_client_exe(&image, &patch, None).unwrap();
            assert_plan_is_the_write(&image, &plan, &patched.bytes);
            let fields: Vec<(&str, usize, &[u8])> = plan
                .changes
                .iter()
                .filter(|change| change.cause == ChangeCause::Edit(0))
                .map(|change| (change.field.as_str(), change.offset, change.new.as_slice()))
                .collect();
            let planned: Vec<(&str, usize, &[u8])> =
                sites.sites.iter().map(|site| (site.kind.patch_label(), site.offset, site.patched())).collect();
            assert_eq!(fields, planned);
            assert_eq!(plan.changes.last().unwrap().cause, ChangeCause::Checksum);
            assert_eq!(patched.after.encryption.as_ref().unwrap().state(), Some(EncryptionState::Removed));
            assert_eq!(patched.manifest.values.encryption, Some(EncryptionState::Removed));
            assert_eq!(patched.manifest.original_values.as_ref().unwrap().encryption, Some(EncryptionState::Encrypted));
            // Once removed, the edit changes nothing; the copy takes other edits with its manifest.
            let error = plan_patch(&patched.bytes, &patch, Some(&patched.manifest)).unwrap_err().to_string();
            assert!(error.contains("changes nothing"), "{error}");
            let resize = ExePatch { edits: vec![size(6, 512, 512)] };
            let chained = patch_client_exe(&patched.bytes, &resize, Some(&patched.manifest)).unwrap();
            assert_eq!(chained.manifest.values.encryption, Some(EncryptionState::Removed));
        }
        let serialized = serde_json::to_string(&patch.edits).unwrap();
        assert_eq!(serialized, r#"["no_encryption"]"#);
        assert_eq!(serde_json::from_str::<Vec<ExeEdit>>(&serialized).unwrap(), patch.edits);
    }

    #[test]
    fn no_encryption_upgrades_the_legacy_kr_manifest_only_for_the_stock_handshake() {
        let image = synthetic_kr_encrypted_client();
        let patch = ExePatch { edits: vec![ExeEdit::NoEncryption] };
        let patched = patch_client_exe(&image, &patch, None).unwrap();
        let sites = &patched.after.encryption.as_ref().unwrap().sites;
        let handshake = sites.iter().find(|site| site.kind == CipherSiteKind::GameLoginHandshake).unwrap();
        // Reconstruct the earlier two-site patch and its hash-matching manifest.
        let mut legacy = patched.bytes.clone();
        legacy[handshake.offset..][..handshake.stock().len()].copy_from_slice(handshake.stock());
        let checksum_offset = patched.after.pe.checksum_offset;
        let checksum = pe_checksum(&legacy, checksum_offset);
        legacy[checksum_offset..][..PATCH_FIELD_SIZE].copy_from_slice(&checksum.to_le_bytes());
        let mut manifest = patched.manifest.clone();
        manifest.patched_sha256 = sha256_hex(&legacy);
        manifest.steps.last_mut().unwrap().patched_sha256 = manifest.patched_sha256.clone();
        manifest.checksum = checksum;
        let inspection = ClientExeInspection::inspect(&legacy).unwrap();
        assert_eq!(inspection.values().encryption, Some(EncryptionState::Partial));
        assert!(!recorded_values_match(&manifest.values, &inspection.values()));

        let upgraded = patch_client_exe(&legacy, &patch, Some(&manifest)).unwrap();
        assert_eq!(upgraded.bytes, patched.bytes);
        assert_eq!(upgraded.manifest.values.encryption, Some(EncryptionState::Removed));
        assert_eq!(upgraded.manifest.original_sha256, manifest.original_sha256);
        assert_eq!(upgraded.manifest.steps[..manifest.steps.len()], manifest.steps);
        let writes: Vec<_> = upgraded.changes.iter().filter(|change| change.cause == ChangeCause::Edit(0)).collect();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].offset, handshake.offset);

        // The compatibility exception never forgives other recorded value mismatches.
        let mut wrong_values = manifest.clone();
        wrong_values.values.facets[0].width = NEW_WIDTH;
        assert!(plan_patch(&legacy, &patch, Some(&wrong_values)).unwrap_err().to_string().contains("values differ"));
        wrong_values = manifest.clone();
        wrong_values.values.login = ClientExeInspection::inspect(&synthetic_kr_login_client()).unwrap().values().login;
        assert!(plan_patch(&legacy, &patch, Some(&wrong_values)).unwrap_err().to_string().contains("values differ"));
        wrong_values = manifest.clone();
        wrong_values.values.encryption = Some(EncryptionState::Encrypted);
        assert!(plan_patch(&legacy, &patch, Some(&wrong_values)).unwrap_err().to_string().contains("values differ"));

        // Another cipher still on, an incomplete inspection or a different family is not the legacy state.
        let mut other = inspection.clone();
        other.encryption.as_mut().unwrap().sites[0].removed = false;
        assert!(!recorded_inspection_values_match(&manifest.values, &other));
        other = inspection.clone();
        let ciphers = other.encryption.as_mut().unwrap();
        ciphers.sites.retain(|site| site.kind != CipherSiteKind::GameLoginHandshake);
        ciphers.unlocated.push(crate::client::encryption::UnlocatedCipherSite {
            kind: CipherSiteKind::GameLoginHandshake,
            matches: 0,
        });
        assert!(!recorded_inspection_values_match(&manifest.values, &other));
        other = inspection.clone();
        other.family = ClientFamily::Enhanced;
        assert!(!recorded_inspection_values_match(&manifest.values, &other));
    }

    #[test]
    fn no_encryption_is_refused_without_every_cipher() {
        let patch = ExePatch { edits: vec![ExeEdit::NoEncryption] };
        let error = |image: &[u8], patch: &ExePatch| plan_patch(image, patch, None).unwrap_err().to_string();
        assert!(error(&synthetic_classic(6, 2), &patch).contains("Enhanced Client and Kingdom Reborn only"));
        let missing = error(&synthetic_kr_client(), &patch);
        assert!(missing.contains("login cipher (0 matches)") && missing.contains("exactly once"), "{missing}");
        let twice = ExePatch { edits: vec![ExeEdit::NoEncryption, ExeEdit::NoEncryption] };
        assert!(error(&synthetic_kr_encrypted_client(), &twice).contains("two no-encryption edits"));
    }

    #[test]
    fn stock_sizes_come_from_the_file_its_manifest_or_its_generation() {
        let image = synthetic_classic(6, 2);
        let stock = stock_facets(&ClientExeInspection::inspect(&image).unwrap(), None);
        assert_eq!(stock[0], Some(StockFacet { size: stock_size(0), wrap: Some(CLASSIC_STOCK_WRAP) }));
        let patch = ExePatch { edits: vec![size(0, CLASSIC_NEW_WIDTH, stock_size(0).height), size(3, 4096, 4096)] };
        let patched = patch_client_exe(&image, &patch, None).unwrap();
        assert_eq!(stock_facets(&patched.after, Some(&patched.manifest)), stock);
        // Without the manifest: the generation (this table's Trammel reads map1, so 7168).
        assert_eq!(stock_facets(&patched.after, None), stock);
        let enhanced = patch_client_exe(&synthetic_client(None, false), &ExePatch { edits: vec![size(0, NEW_WIDTH, NEW_HEIGHT)] }, None).unwrap();
        let sizes: Vec<FacetGeometry> = stock_facets(&enhanced.after, None).into_iter().map(|facet| facet.unwrap().size).collect();
        assert_eq!(sizes, CLIENT_FACET_DIMENSIONS);
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
