//! Client executables (`UOSA.exe` builds, `UOKR.exe`): the facet size table and its patcher
//! ([`facet_dimensions`], located by structure in any 32-bit build), the check of the facet data against it
//! ([`facet_data`]), and the PE helpers they use. Executables are only read; patches go to new copies.

pub mod facet_data;
pub mod facet_dimensions;
pub mod pe;
pub mod sha256;
