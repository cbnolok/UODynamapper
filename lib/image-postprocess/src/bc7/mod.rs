//! BC7 encoding and RDO postprocessing.
//!
//! The public re-exports in `crate::lib` keep the older module paths stable,
//! while this directory keeps the encoder, wide front-end, tables, and RDO pass
//! together.

pub mod analytical;
pub mod analytical_wide;
pub mod rdo;
mod tables;
