//! The encryption part of the Login server section (Enhanced Client, Kingdom Reborn): whether the opened
//! executable's network ciphers are on, where each is, and independent login, game and KR handshake edits.

use uocf::client::encryption::{EncryptionPart, EncryptionSites, EncryptionState};
use uocf::client::facet_dimensions::ClientExeInspection;
use uocf::client::patch::ExeEdit;

use super::edits::Validation;

/// What the server must do once the client talks plaintext, for the section's note.
pub const PLAINTEXT_SERVER_NOTE: &str = "Login and game encryption are independent. Removing KR game encryption \
     keeps its 0xE3/0xE4 handshake. Bypassing the KR handshake sends the relay auth-key seed and plaintext 0x91 \
     immediately, without installing game ciphers, even if their bytes remain stock. Plaintext game traffic \
     still requires Huffman-compressed server replies.";

pub const ENCRYPTION_PARTS: [EncryptionPart; 3] = [EncryptionPart::Login, EncryptionPart::Game, EncryptionPart::KrHandshake];

pub fn part_label(part: EncryptionPart) -> &'static str {
    match part {
        EncryptionPart::Login => "login encryption",
        EncryptionPart::Game => "game encryption",
        EncryptionPart::KrHandshake => "KR handshake",
    }
}

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

/// Independent selections; each validates only the sites it patches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionEdit {
    pub sites: Option<EncryptionSites>,
    pub remove_login: bool,
    pub remove_game: bool,
    pub bypass_kr_handshake: bool,
}

impl EncryptionEdit {
    pub fn new(inspection: &ClientExeInspection) -> Self {
        Self { sites: inspection.encryption.clone(), remove_login: false, remove_game: false, bypass_kr_handshake: false }
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

    pub fn part_status(&self, part: EncryptionPart) -> EncryptionStatus {
        let Some(sites) = &self.sites else {
            return EncryptionStatus::NotApplicable;
        };
        let sites = sites.for_part(part);
        if sites.sites.is_empty() && sites.unlocated.is_empty() {
            return EncryptionStatus::NotApplicable;
        }
        match sites.values() {
            Some(state) => EncryptionStatus::Located(state),
            None => EncryptionStatus::NotLocated(sites.describe_unlocated()),
        }
    }

    /// True when every site of this part is located and at least one still needs patching.
    pub fn can_remove(&self, part: EncryptionPart) -> bool {
        matches!(self.part_status(part), EncryptionStatus::Located(EncryptionState::Encrypted | EncryptionState::Partial))
    }

    pub fn selection_mut(&mut self, part: EncryptionPart) -> &mut bool {
        match part {
            EncryptionPart::Login => &mut self.remove_login,
            EncryptionPart::Game => &mut self.remove_game,
            EncryptionPart::KrHandshake => &mut self.bypass_kr_handshake,
        }
    }

    /// Selected edits; no edits are returned if any selected part cannot apply.
    pub fn check(&self) -> (Vec<ExeEdit>, Validation) {
        let mut edits = Vec::new();
        let mut validation = Validation::default();
        for (part, selected, edit) in [
            (EncryptionPart::Login, self.remove_login, ExeEdit::NoLoginEncryption),
            (EncryptionPart::Game, self.remove_game, ExeEdit::NoGameEncryption),
            (EncryptionPart::KrHandshake, self.bypass_kr_handshake, ExeEdit::BypassKrHandshake),
        ] {
            if !selected {
                continue;
            }
            match self.part_status(part) {
                EncryptionStatus::Located(EncryptionState::Removed) => {}
                EncryptionStatus::Located(_) => edits.push(edit),
                EncryptionStatus::NotLocated(unlocated) => {
                    validation.errors.push(format!("{}: sites not all located ({unlocated}): nothing is patched", part_label(part)))
                }
                EncryptionStatus::NotApplicable => {
                    validation.errors.push(format!("{} is not applicable to this client", part_label(part)))
                }
            }
        }
        if !validation.is_ok() {
            edits.clear();
        }
        (edits, validation)
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
