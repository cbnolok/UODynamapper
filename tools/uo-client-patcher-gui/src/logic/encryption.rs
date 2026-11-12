//! The encryption part of the Login server section (Enhanced Client, Kingdom Reborn): whether the opened
//! executable's network ciphers are on, where each is, and the toggle that patches them out
//! ([`ExeEdit::NoEncryption`], one more edit of the patch).

use uocf::client::encryption::{EncryptionSites, EncryptionState};
use uocf::client::facet_dimensions::ClientExeInspection;
use uocf::client::patch::ExeEdit;

use super::edits::Validation;

/// What the server must do once the client talks plaintext, for the section's note.
pub const PLAINTEXT_SERVER_NOTE: &str = "The server must take the login packets (0xEF seed, 0x80) and, after the game \
     seed, the 0x91 unencrypted, and send unencrypted (still Huffman-compressed) game packets. Kingdom Reborn sends \
     the relay auth key as its game seed and sends 0x91 immediately, skipping the 0xE3/0xE4 key exchange.";

/// Whether the opened executable encrypts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncryptionStatus {
    /// Classic: not located here.
    NotApplicable,
    /// Some cipher's pattern did not match exactly once: these, in words.
    NotLocated(String),
    Located(EncryptionState),
}

/// One located cipher, as the section lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CipherRow {
    pub what: String,
    pub offset: String,
    pub va: String,
    pub bytes: String,
    pub state: String,
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(" ")
}

/// The toggle and the ciphers it patches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionEdit {
    pub sites: Option<EncryptionSites>,
    /// Patch every cipher out.
    pub remove: bool,
}

impl EncryptionEdit {
    pub fn new(inspection: &ClientExeInspection) -> Self {
        Self { sites: inspection.encryption.clone(), remove: false }
    }

    pub fn status(&self) -> EncryptionStatus {
        match &self.sites {
            None => EncryptionStatus::NotApplicable,
            Some(sites) => match sites.values() {
                Some(state) => EncryptionStatus::Located(state),
                None => EncryptionStatus::NotLocated(sites.describe_unlocated()),
            },
        }
    }

    /// True when the toggle patches something: every cipher located, and one at least still on.
    pub fn can_remove(&self) -> bool {
        matches!(self.status(), EncryptionStatus::Located(EncryptionState::Encrypted | EncryptionState::Partial))
    }

    /// The edit of the toggle; an error when it is on but cannot apply.
    pub fn check(&self) -> (Option<ExeEdit>, Validation) {
        let mut validation = Validation::default();
        if !self.remove {
            return (None, validation);
        }
        match self.status() {
            EncryptionStatus::Located(EncryptionState::Removed) => {}
            EncryptionStatus::Located(_) => return (Some(ExeEdit::NoEncryption), validation),
            EncryptionStatus::NotLocated(unlocated) => {
                validation.errors.push(format!("the ciphers were not all located ({unlocated}): nothing is patched"))
            }
            EncryptionStatus::NotApplicable => {
                validation.errors.push("only the Enhanced Client and Kingdom Reborn ciphers are located".to_owned())
            }
        }
        (None, validation)
    }

    /// The located ciphers with their bytes now.
    pub fn rows(&self, inspection: &ClientExeInspection) -> Vec<CipherRow> {
        let Some(sites) = &self.sites else {
            return Vec::new();
        };
        let va = |offset: usize| inspection.pe.offset_to_va(offset).map(|va| format!("{va:#010x}")).unwrap_or_default();
        sites
            .sites
            .iter()
            .map(|site| CipherRow {
                what: site.kind.label().to_owned(),
                offset: format!("{:#010x}", site.offset),
                va: va(site.offset),
                bytes: hex_bytes(if site.removed { site.patched() } else { site.stock() }),
                state: if site.removed {
                    "patched out".to_owned()
                } else {
                    format!("on (patch: {})", hex_bytes(site.patched()))
                },
            })
            .collect()
    }
}
