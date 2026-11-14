//! Client configuration and executables (`UOSA.exe` builds, `UOKR.exe`, Classic `client.exe`): the facet size tables and the values
//! derived from them ([`facet_dimensions`], [`classic_facets`], located by structure in any 32-bit build), the
//! Classic login files ([`login_servers`]), the built-in login server address of the Enhanced Client and Kingdom Reborn ([`login`]), their network
//! ciphers ([`encryption`]), the patcher of copies
//! ([`patch`]), the check of the facet data against them ([`facet_data`]), the version resource ([`version`],
//! information only), and the PE helpers they use. Executables are only read; patches go to new copies.

pub mod classic_difs;
pub mod classic_facets;
pub mod crypt_keys;
pub mod encryption;
pub mod facet_data;
pub mod facet_dimensions;
mod kr_game_login;
mod kr_game_login_legacy;
mod kr_legacy_login;
pub mod login;
pub mod login_servers;
pub mod patch;
pub mod pe;
pub mod sha256;
pub mod version;
