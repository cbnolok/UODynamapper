pub mod sources;
pub mod assets;
pub mod world;
pub mod tools;
pub mod preview;

/// The tooltip of Validate packages after packing, on the Assets and World tabs.
pub(crate) const VALIDATE_AFTER_PACKING_TOOLTIP: &str = "Once a packing job succeeds, read back every package it just \
wrote, as udd-cli tool validate does: the container, every entry decoded, and the reader of its kind. Each package is \
read whole, so the job takes longer; a package that fails fails the job. Every result goes to the log. The radar \
texture is no package and is not read.";
