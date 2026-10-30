//! The `effects_ec.uddp` viewer's logic: view models of the package ([`model`]), texture decoding
//! ([`texture`]), the effect preview built from the package alone ([`preview`]) and the background thread that
//! simulates and renders it ([`worker`]). No egui here.

pub mod hues;
pub mod model;
pub mod preview;
pub mod texture;
pub mod worker;

#[cfg(test)]
mod real_package_tests;
#[cfg(test)]
pub mod test_package;
