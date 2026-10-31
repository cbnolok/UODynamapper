//! Client executables (`UOSA.exe` builds, `UOKR.exe`, Classic `client.exe`): the facet size tables and the values
//! derived from them ([`facet_dimensions`], [`classic_facets`], located by structure in any 32-bit build), the
//! patcher of copies ([`patch`]), the check of the facet data against them ([`facet_data`]), and the PE helpers they
//! use. Executables are only read; patches go to new copies.

pub mod classic_facets;
pub mod facet_data;
pub mod facet_dimensions;
pub mod patch;
pub mod pe;
pub mod sha256;
