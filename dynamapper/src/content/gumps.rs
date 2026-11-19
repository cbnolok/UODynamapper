//! The gump provider: which loaded source (gumps_ec.uddp, the Classic gump package or the Classic client files) each
//! gump is read from, following the "Gumps and paperdoll" preference, and the check that the gumps both sources share
//! are the same image.
use crate::settings::GumpClient;
use crate::prelude::*;
use bevy::prelude::*;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub(super) const GUMPS_CC_FILE_NAME: &str = "gumps_cc.uddp";
pub(super) const GUMPS_EC_FILE_NAME: &str = "gumps_ec.uddp";
const CLASSIC_CLIENT_GUMPS_LABEL: &str = "the Classic client gump files";

/// Which loaded source `pick_gump_source` read a gump from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GumpSourceKind {
    Enhanced,
    Classic,
}

/// What `pick_gump_source` gives for one gump.
#[derive(Debug)]
enum GumpPick<T> {
    Found(GumpSourceKind, T),
    /// The source the rule picked has the gump but cannot decode it. No other source is tried.
    Failed(GumpSourceKind, color_eyre::eyre::Report),
    /// Classic gumps with no Classic source loaded: none show.
    NoClassicSource,
    /// Enhanced gumps, gumps_ec.uddp lacks the gump and no Classic source is loaded.
    Missing,
}

/// The source of one gump of `client`'s gumps (the user, 2026-11-06). Classic reads the Classic source only, never
/// gumps_ec.uddp. Enhanced reads gumps_ec.uddp when it has the gump, else the Classic source; a gumps_ec.uddp entry
/// that fails to decode is an error, not a reason to fall back.
///
/// `enhanced` is set when gumps_ec.uddp is loaded and looks the gump up there: `None` when the package lacks it, else
/// its decode. `classic` is set when a Classic source is loaded and decodes the gump from it.
fn pick_gump_source<T>(
    client: GumpClient,
    enhanced: Option<impl FnOnce() -> Option<color_eyre::eyre::Result<T>>>,
    classic: Option<impl FnOnce() -> color_eyre::eyre::Result<T>>,
) -> GumpPick<T> {
    if client == GumpClient::Enhanced {
        if let Some(decoded) = enhanced.and_then(|lookup| lookup()) {
            return match decoded {
                Ok(gump) => GumpPick::Found(GumpSourceKind::Enhanced, gump),
                Err(error) => GumpPick::Failed(GumpSourceKind::Enhanced, error),
            };
        }
    }

    match (classic, client) {
        (Some(decode), _) => match decode() {
            Ok(gump) => GumpPick::Found(GumpSourceKind::Classic, gump),
            Err(error) => GumpPick::Failed(GumpSourceKind::Classic, error),
        },
        (None, GumpClient::Classic) => GumpPick::NoClassicSource,
        (None, GumpClient::Enhanced) => GumpPick::Missing,
    }
}

/// The startup line naming the source each "Gumps and paperdoll" choice draws its gumps from.
fn gump_sources_summary(classic_label: Option<&str>, enhanced_loaded: bool) -> String {
    let classic_gumps = classic_label.unwrap_or("none, so none show");
    let enhanced_gumps = match (enhanced_loaded, classic_label) {
        (true, Some(classic_label)) => format!("{GUMPS_EC_FILE_NAME}, then {classic_label} for ids it lacks"),
        (true, None) => GUMPS_EC_FILE_NAME.to_string(),
        (false, Some(classic_label)) => format!("{classic_label} ({GUMPS_EC_FILE_NAME} is not loaded)"),
        (false, None) => "none".to_string(),
    };
    format!("gumps: Classic {classic_gumps}; Enhanced {enhanced_gumps}")
}

/// The startup warning for a gumps_ec.uddp that leaves `shared_count` gumps for gumps_cc.uddp to draw, when no Classic
/// source is loaded to draw them.
fn shared_gumps_without_classic_warning(shared_count: usize, classic_loaded: bool) -> Option<String> {
    (shared_count > 0 && !classic_loaded).then(|| {
        format!(
            "{GUMPS_EC_FILE_NAME} shares {shared_count} gumps with {GUMPS_CC_FILE_NAME}, and neither that package nor \
             {CLASSIC_CLIENT_GUMPS_LABEL} are loaded: those gumps won't show as Enhanced gumps."
        )
    })
}

/// What [`SharedGumpCheck::check`] found for one gump the Classic source served as an Enhanced gump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SharedGumpVerdict {
    /// gumps_ec.uddp doesn't share the gump: it lacks it.
    NotShared,
    /// The gump was checked before.
    AlreadyChecked,
    Matches,
    /// The first shared gump the Classic source draws differently.
    FirstMismatch,
    /// A later one, `count` with the first.
    Mismatch { count: u32 },
}

/// Checks each gump gumps_ec.uddp shares with gumps_cc.uddp the first time the Classic source serves it as an
/// Enhanced gump: the image hash gumps_ec.uddp lists against the served image's. Each gump is hashed once.
#[derive(Default)]
struct SharedGumpCheck {
    checked: Mutex<HashSet<u32>>,
    mismatch_count: AtomicU32,
}

impl SharedGumpCheck {
    /// `listed_hash` is gumps_ec.uddp's hash of the gump, `None` when it doesn't share it.
    fn check(
        &self,
        gump_id: u32,
        listed_hash: Option<u64>,
        served: &udd_assets::gumps::GumpImage,
    ) -> SharedGumpVerdict {
        let Some(listed_hash) = listed_hash else {
            return SharedGumpVerdict::NotShared;
        };
        if !self.checked.lock().insert(gump_id) {
            return SharedGumpVerdict::AlreadyChecked;
        }
        if udd_assets::gumps::gump_image_hash(served) == listed_hash {
            return SharedGumpVerdict::Matches;
        }
        match self.mismatch_count.fetch_add(1, Ordering::Relaxed) + 1 {
            1 => SharedGumpVerdict::FirstMismatch,
            count => SharedGumpVerdict::Mismatch { count },
        }
    }
}

/// The Classic gump source: gumps_cc.uddp, or the Classic client's gump files when that package is absent or fails to
/// load.
pub enum ClassicGumpSource {
    Package(udd_assets::GumpsPackage),
    ClientFiles(uocf::classic::gump::GumpMap),
}

impl ClassicGumpSource {
    fn label(&self) -> &'static str {
        match self {
            Self::Package(_) => GUMPS_CC_FILE_NAME,
            Self::ClientFiles(_) => CLASSIC_CLIENT_GUMPS_LABEL,
        }
    }

    fn has_gump(&self, gump_id: u32) -> bool {
        match self {
            Self::Package(package) => package.has_gump(gump_id),
            Self::ClientFiles(classic) => classic.has_id(gump_id),
        }
    }

    fn decode(
        &self,
        gump_id: u32,
        scratch: &mut Vec<u8>,
    ) -> color_eyre::eyre::Result<udd_assets::gumps::GumpImage> {
        let classic = match self {
            Self::Package(package) => return package.read_gump_image(gump_id),
            Self::ClientFiles(classic) => classic,
        };
        let (width, height, pixels) = classic.decode_gump(gump_id, scratch)?;
        Ok(udd_assets::gumps::GumpImage::whole(u32::from(width), u32::from(height), 1, pixels))
    }
}

/// gumps_ec.uddp with its gump ids, sorted, so a lookup knows when Enhanced gumps fall back.
struct EnhancedGumps {
    package: udd_assets::GumpsPackage,
    gump_ids: Vec<u32>,
}

impl EnhancedGumps {
    fn new(package: udd_assets::GumpsPackage) -> Self {
        let gump_ids = package.available_gump_ids();
        Self { package, gump_ids }
    }

    fn has_gump(&self, gump_id: u32) -> bool {
        self.gump_ids.binary_search(&gump_id).is_ok()
    }

    fn lookup(&self, gump_id: u32) -> Option<color_eyre::eyre::Result<udd_assets::gumps::GumpImage>> {
        self.has_gump(gump_id).then(|| self.package.read_gump_image(gump_id))
    }
}

/// Optional gump art source. UDDP packages are preferred, legacy Classic sources are fallback.
/// The "Gumps and paperdoll" preference picks which source serves a gump (`GumpClient`, `pick_gump_source`).
#[derive(Resource)]
pub struct GumpMapRes {
    classic: Option<Arc<ClassicGumpSource>>,
    enhanced: Option<Arc<EnhancedGumps>>,
    /// Set once Classic gumps were asked for with no Classic source loaded, which is logged once.
    classic_missing_logged: AtomicBool,
    shared_check: SharedGumpCheck,
}

impl GumpMapRes {
    /// `None` when neither source is loaded.
    pub fn new(classic: Option<ClassicGumpSource>, enhanced: Option<udd_assets::GumpsPackage>) -> Option<Self> {
        if classic.is_none() && enhanced.is_none() {
            return None;
        }
        Some(Self {
            classic: classic.map(Arc::new),
            enhanced: enhanced.map(|package| Arc::new(EnhancedGumps::new(package))),
            classic_missing_logged: AtomicBool::new(false),
            shared_check: SharedGumpCheck::default(),
        })
    }

    pub(super) fn sources_summary(&self) -> String {
        gump_sources_summary(self.classic.as_deref().map(ClassicGumpSource::label), self.enhanced.is_some())
    }

    fn is_shared_gump(&self, gump_id: u32) -> bool {
        self.enhanced.as_ref().is_some_and(|enhanced| enhanced.package.shared_classic_gump_hash(gump_id).is_some())
    }

    pub(super) fn shared_gumps_warning(&self) -> Option<String> {
        let shared_count = self.enhanced.as_ref().map_or(0, |enhanced| enhanced.package.shared_classic_gump_count());
        shared_gumps_without_classic_warning(shared_count, self.classic.is_some())
    }

    /// Checks a gump the Classic source served as an Enhanced gump against the hash gumps_ec.uddp lists when it
    /// shares the gump. A mismatch still draws the gump.
    fn check_shared_gump(&self, gump_id: u32, served: &udd_assets::gumps::GumpImage) {
        let (Some(enhanced), Some(classic)) = (self.enhanced.as_deref(), self.classic.as_deref()) else {
            return;
        };
        let listed_hash = enhanced.package.shared_classic_gump_hash(gump_id);
        match self.shared_check.check(gump_id, listed_hash, served) {
            SharedGumpVerdict::NotShared | SharedGumpVerdict::AlreadyChecked | SharedGumpVerdict::Matches => {}
            SharedGumpVerdict::FirstMismatch => console_logger::one(
                console_logger::LogSev::Warn,
                console_logger::LogAbout::UoFiles,
                &format!(
                    "Gump {gump_id}, which {GUMPS_EC_FILE_NAME} shares, differs in {}: it is not the \
                     {GUMPS_CC_FILE_NAME} that {GUMPS_EC_FILE_NAME} was packed against. The gump is drawn anyway; \
                     repack {GUMPS_EC_FILE_NAME} against this {GUMPS_CC_FILE_NAME}, or standalone.",
                    classic.label()
                ),
            ),
            SharedGumpVerdict::Mismatch { count } => debug!(
                "Gump {gump_id}, which {GUMPS_EC_FILE_NAME} shares, differs in {}: {count} shared gumps differ so far.",
                classic.label()
            ),
        }
    }

    /// Whether `client`'s gumps find gump `gump_id` in a loaded source (`pick_gump_source`): Classic in the Classic
    /// source, Enhanced in gumps_ec.uddp or the Classic source.
    pub fn has_gump(&self, client: GumpClient, gump_id: u32) -> bool {
        let in_enhanced = || self.enhanced.as_deref().is_some_and(|enhanced| enhanced.has_gump(gump_id));
        let in_classic = || self.classic.as_deref().is_some_and(|classic| classic.has_gump(gump_id));
        match client {
            GumpClient::Classic => in_classic(),
            GumpClient::Enhanced => in_enhanced() || in_classic(),
        }
    }

    pub fn decode_gump(
        &self,
        client: GumpClient,
        gump_id: u32,
        scratch: &mut Vec<u8>,
    ) -> color_eyre::eyre::Result<udd_assets::gumps::GumpImage> {
        let enhanced = self.enhanced.as_deref().map(|enhanced| move || enhanced.lookup(gump_id));
        let classic = self.classic.as_deref().map(|classic| move || classic.decode(gump_id, scratch));
        match pick_gump_source(client, enhanced, classic) {
            GumpPick::Found(GumpSourceKind::Classic, gump) if client == GumpClient::Enhanced => {
                self.check_shared_gump(gump_id, &gump);
                Ok(gump)
            }
            GumpPick::Found(_, gump) => Ok(gump),
            GumpPick::Failed(GumpSourceKind::Enhanced, error) => {
                let error = error.wrap_err(format!("{GUMPS_EC_FILE_NAME} gump {gump_id} does not decode"));
                console_logger::one(
                    console_logger::LogSev::Error,
                    console_logger::LogAbout::UoFiles,
                    &format!("{error:#}"),
                );
                Err(error)
            }
            GumpPick::Failed(GumpSourceKind::Classic, error) => Err(error),
            GumpPick::NoClassicSource => {
                if !self.classic_missing_logged.swap(true, Ordering::Relaxed) {
                    console_logger::one(
                        console_logger::LogSev::Warn,
                        console_logger::LogAbout::UoFiles,
                        &format!(
                            "Classic gumps show none: neither {GUMPS_CC_FILE_NAME} nor {CLASSIC_CLIENT_GUMPS_LABEL} are loaded."
                        ),
                    );
                }
                Err(color_eyre::eyre::eyre!("Classic gumps have no source loaded"))
            }
            GumpPick::Missing if self.is_shared_gump(gump_id) => Err(color_eyre::eyre::eyre!(
                "gump {gump_id} is one {GUMPS_EC_FILE_NAME} shares with {GUMPS_CC_FILE_NAME}, and no Classic gump \
                 source is loaded"
            )),
            GumpPick::Missing => Err(color_eyre::eyre::eyre!(
                "gump {gump_id} is not in {GUMPS_EC_FILE_NAME} and no Classic gump source is loaded"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        gump_sources_summary, pick_gump_source, shared_gumps_without_classic_warning, GumpClient, GumpPick,
        GumpSourceKind, SharedGumpCheck, SharedGumpVerdict, GUMPS_CC_FILE_NAME,
    };

    const ENHANCED_IMAGE: &str = "gumps_ec image";
    const CLASSIC_IMAGE: &str = "Classic image";

    /// gumps_ec.uddp's state for the looked-up gump.
    #[derive(Clone, Copy, Debug)]
    enum Enhanced {
        NotLoaded,
        LacksGump,
        HasGump,
        DecodeError,
    }

    /// What `pick_gump_source` should give, without the error text.
    #[derive(Debug, PartialEq, Eq)]
    enum Expected {
        Found(GumpSourceKind, &'static str),
        Failed(GumpSourceKind),
        NoClassicSource,
        Missing,
    }

    /// What `pick` saw: the result, and whether each loaded source was asked.
    struct Picked {
        result: Expected,
        enhanced_asked: bool,
        classic_asked: bool,
    }

    /// Runs the rule with stand-in sources.
    fn pick(client: GumpClient, enhanced: Enhanced, classic_loaded: bool, classic_decodes: bool) -> Picked {
        let mut enhanced_asked = false;
        let mut classic_asked = false;
        let enhanced_flag = &mut enhanced_asked;
        let enhanced_lookup = match enhanced {
            Enhanced::NotLoaded => None,
            loaded => Some(move || {
                *enhanced_flag = true;
                match loaded {
                    Enhanced::HasGump => Some(Ok(ENHANCED_IMAGE)),
                    Enhanced::DecodeError => Some(Err(color_eyre::eyre::eyre!("bad gumps_ec entry"))),
                    _ => None,
                }
            }),
        };
        let classic = classic_loaded.then_some(|| {
            classic_asked = true;
            if classic_decodes {
                Ok(CLASSIC_IMAGE)
            } else {
                Err(color_eyre::eyre::eyre!("bad Classic entry"))
            }
        });
        let result = match pick_gump_source(client, enhanced_lookup, classic) {
            GumpPick::Found(kind, image) => Expected::Found(kind, image),
            GumpPick::Failed(kind, _) => Expected::Failed(kind),
            GumpPick::NoClassicSource => Expected::NoClassicSource,
            GumpPick::Missing => Expected::Missing,
        };
        Picked { result, enhanced_asked, classic_asked }
    }

    /// Each "Gumps and paperdoll" choice against which packages are loaded, whether gumps_ec.uddp has the gump, and its
    /// decode error.
    #[test]
    fn gump_source_follows_the_gumps_and_paperdoll_preference() {
        use Enhanced::{DecodeError, HasGump, LacksGump, NotLoaded};
        use Expected::{Failed, Found, Missing, NoClassicSource};
        use GumpSourceKind::{Classic, Enhanced as Ec};
        let classic_cases = [
            (NotLoaded, true, Found(Classic, CLASSIC_IMAGE)),
            (LacksGump, true, Found(Classic, CLASSIC_IMAGE)),
            (HasGump, true, Found(Classic, CLASSIC_IMAGE)),
            (DecodeError, true, Found(Classic, CLASSIC_IMAGE)),
            (NotLoaded, false, NoClassicSource),
            (LacksGump, false, NoClassicSource),
            (HasGump, false, NoClassicSource),
            (DecodeError, false, NoClassicSource),
        ];
        for (enhanced, classic_loaded, expected) in classic_cases {
            let picked = pick(GumpClient::Classic, enhanced, classic_loaded, true);
            let case = format!("Classic gumps, gumps_ec {enhanced:?}, Classic source {classic_loaded}");
            assert_eq!(picked.result, expected, "{case}");
            assert!(!picked.enhanced_asked, "{case}: Classic gumps never read gumps_ec");
        }

        let enhanced_cases = [
            (NotLoaded, true, Found(Classic, CLASSIC_IMAGE)),
            (LacksGump, true, Found(Classic, CLASSIC_IMAGE)),
            (HasGump, true, Found(Ec, ENHANCED_IMAGE)),
            (DecodeError, true, Failed(Ec)),
            (NotLoaded, false, Missing),
            (LacksGump, false, Missing),
            (HasGump, false, Found(Ec, ENHANCED_IMAGE)),
            (DecodeError, false, Failed(Ec)),
        ];
        for (enhanced, classic_loaded, expected) in enhanced_cases {
            let picked = pick(GumpClient::Enhanced, enhanced, classic_loaded, true);
            let case = format!("Enhanced gumps, gumps_ec {enhanced:?}, Classic source {classic_loaded}");
            assert_eq!(picked.result, expected, "{case}");
            // The Classic source is asked only when gumps_ec lacks the gump, never after a decode error.
            let falls_back = matches!(picked.result, Found(Classic, _));
            assert_eq!(picked.classic_asked, falls_back, "{case}");
        }
    }

    #[test]
    fn classic_gump_decode_error_is_reported_from_the_classic_source() {
        for client in GumpClient::ALL {
            let picked = pick(client, Enhanced::LacksGump, true, false);
            assert_eq!(picked.result, Expected::Failed(GumpSourceKind::Classic), "{client:?} gumps");
        }
    }

    #[test]
    fn gump_startup_summary_names_each_choice_source() {
        assert_eq!(
            gump_sources_summary(Some(GUMPS_CC_FILE_NAME), true),
            "gumps: Classic gumps_cc.uddp; Enhanced gumps_ec.uddp, then gumps_cc.uddp for ids it lacks"
        );
        assert_eq!(
            gump_sources_summary(None, true),
            "gumps: Classic none, so none show; Enhanced gumps_ec.uddp"
        );
        assert_eq!(
            gump_sources_summary(Some(GUMPS_CC_FILE_NAME), false),
            "gumps: Classic gumps_cc.uddp; Enhanced gumps_cc.uddp (gumps_ec.uddp is not loaded)"
        );
    }

    /// The startup warning comes only for a shared gumps_ec.uddp without a Classic source to draw what it shares.
    #[test]
    fn shared_gumps_without_a_classic_source_are_warned_of_once_at_startup() {
        const SHARED_COUNT: usize = 2216;
        assert_eq!(shared_gumps_without_classic_warning(0, false), None);
        assert_eq!(shared_gumps_without_classic_warning(SHARED_COUNT, true), None);
        let warning = shared_gumps_without_classic_warning(SHARED_COUNT, false).expect("a warning");
        assert!(warning.contains("2216 gumps with gumps_cc.uddp"), "{warning}");
        assert!(warning.contains("won't show as Enhanced gumps"), "{warning}");
    }

    /// A shared gump is hashed the first time it is served and never again; the first mismatch is told apart from the
    /// later ones, which carry the count; a gump gumps_ec.uddp doesn't share is never checked.
    #[test]
    fn shared_gumps_are_hashed_once_and_mismatches_counted() {
        const MATCHING: u32 = 100;
        const FIRST_DIFFERING: u32 = 101;
        const SECOND_DIFFERING: u32 = 102;
        const NOT_SHARED: u32 = 103;
        const SIDE: u32 = 2;
        const RGBA_CHANNELS: usize = 4;
        const OPAQUE_GREY: u8 = 128;
        let served =
            udd_assets::gumps::GumpImage::whole(SIDE, SIDE, 1, vec![OPAQUE_GREY; (SIDE * SIDE) as usize * RGBA_CHANNELS]);
        let served_hash = udd_assets::gumps::gump_image_hash(&served);
        let other_hash = served_hash.wrapping_add(1);
        let check = SharedGumpCheck::default();

        assert_eq!(check.check(NOT_SHARED, None, &served), SharedGumpVerdict::NotShared);
        assert_eq!(check.check(MATCHING, Some(served_hash), &served), SharedGumpVerdict::Matches);
        assert_eq!(check.check(MATCHING, Some(served_hash), &served), SharedGumpVerdict::AlreadyChecked);
        assert_eq!(check.check(FIRST_DIFFERING, Some(other_hash), &served), SharedGumpVerdict::FirstMismatch);
        assert_eq!(check.check(FIRST_DIFFERING, Some(other_hash), &served), SharedGumpVerdict::AlreadyChecked);
        assert_eq!(
            check.check(SECOND_DIFFERING, Some(other_hash), &served),
            SharedGumpVerdict::Mismatch { count: 2 }
        );
        assert_eq!(check.check(NOT_SHARED, None, &served), SharedGumpVerdict::NotShared);
    }
}
